package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"
)

// Synthetic all-retained snapshot, imported into a separate new Core55 target.
func TestConnectionHistoryFunnelLive(t *testing.T) {
	socket, dsn := os.Getenv("LMM_TEST_CONNECTION_HISTORY_SOCKET"), os.Getenv("ACQUISITION_CONNECTION_HISTORY_DATABASE_URL")
	if socket == "" || dsn == "" {
		t.Skip("requires Core55 synthetic-history fixture")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if _, err = db.Exec(Schema); err != nil {
		t.Fatal(err)
	}
	defer db.Exec(`DROP SCHEMA lmm_acquisition CASCADE`)
	client, err := community.NewCoreRead(socket, []byte("synthetic-community-purpose-token-20261104"))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	facts, err := client.ConnectionMilestones(context.Background(), []int64{11, 12, 13, 14, 15, 16})
	if err != nil || len(facts.Users) != 6 {
		t.Fatalf("facts: %+v %v", facts, err)
	}
	if facts.Users[0].HistorySource != "retained_snapshot_and_native" || facts.Users[1].APIKeyCreatedAt != nil || facts.Users[1].OAuthAuthorizedAt == nil {
		t.Fatalf("historical facts unavailable or managed token misclassified: %+v", facts)
	}
	service, err := New(context.Background(), db, "https://synthetic.example", true)
	if err != nil {
		t.Fatal(err)
	}
	service.UserRead = client
	now := time.Now().Unix()
	service.now = func() time.Time { return time.Unix(now, 0) }
	mux := http.NewServeMux()
	service.Register(mux, listVerifier{5})
	for _, tt := range []struct {
		method                     string
		count, key, oauth, unknown int
	}{
		{"api_key", 1, 1, 0, 0}, {"oauth", 1, 0, 1, 0}, {"both", 1, 1, 1, 0}, {"unknown", 3, 0, 0, 2},
	} {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/funnel?from=%d&to=%d&observation_days=1&connection_method=%s", now-3*daySeconds, now, tt.method), nil))
		var body struct {
			Success bool
			Data    funnelReport
		}
		if err = json.Unmarshal(w.Body.Bytes(), &body); err != nil || w.Code != 200 || !body.Success {
			t.Fatalf("%s: %d %s", tt.method, w.Code, w.Body.String())
		}
		registrations := 0
		for _, ch := range body.Data.Channels {
			registrations += ch.Registrations
			stages := map[string]funnelStage{}
			for _, s := range ch.Stages {
				stages[s.ID] = s
			}
			for _, v := range []struct {
				id       string
				observed int
			}{{"api_key_created", tt.key}, {"oauth_authorized", tt.oauth}, {"credential_ready", max(tt.key, tt.oauth)}} {
				s := stages[v.id]
				if s.Observed != v.observed || s.Unknown != tt.unknown || s.MatureKnown != tt.count-tt.unknown || s.TimingAccounts != v.observed {
					t.Fatalf("%s %s: %+v", tt.method, v.id, s)
				}
			}
			if tt.method == "api_key" {
				for id, want := range map[string]float64{"application_submitted": 10, "access_approved": 20} {
					s := stages[id]
					if s.Observed != 1 || s.Unknown != 0 || s.TimingAccounts != 1 || s.MeanSeconds == nil || *s.MeanSeconds != want {
						t.Fatalf("%s: %+v", id, s)
					}
				}
			}
			if tt.method == "unknown" && stages["application_submitted"].Unknown != 2 {
				t.Fatalf("missing application evidence hidden: %+v", stages["application_submitted"])
			}
		}
		if registrations != tt.count {
			t.Fatalf("%s registrations=%d", tt.method, registrations)
		}
	}
}
