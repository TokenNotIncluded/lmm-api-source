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

func TestConnectionMethodsAndStageStatistics(t *testing.T) {
	ptr := func(v int64) *int64 { return &v }
	u := funnelAccount{ID: 1, Registered: 100}
	for _, tt := range []struct {
		name                      string
		key, oauth                *int64
		complete                  bool
		want                      string
		observed, unknown, timing int
	}{
		{"key", ptr(110), nil, true, "api_key", 1, 0, 1},
		{"oauth", nil, ptr(120), true, "oauth", 0, 0, 0},
		{"both", ptr(110), ptr(120), true, "both", 1, 0, 1},
		{"none", nil, nil, true, "unknown", 0, 0, 0},
		{"legacy absence", nil, nil, false, "unknown", 0, 1, 0},
		{"legacy positive", ptr(110), nil, false, "unknown", 1, 0, 0},
		{"legacy both", ptr(110), ptr(120), false, "both", 1, 0, 0},
		{"outside window", ptr(86500), nil, true, "unknown", 0, 0, 0},
	} {
		t.Run(tt.name, func(t *testing.T) {
			key, _, method, err := connectionObservations(u, community.ConnectionMilestone{UserID: 1, APIKeyCreatedAt: tt.key, OAuthAuthorizedAt: tt.oauth, HistoryComplete: tt.complete}, 100000, 1)
			if err != nil || method != tt.want {
				t.Fatalf("method=%s err=%v", method, err)
			}
			s, err := summarizeFunnelStage("api_key_created", 100000, 1, []funnelObservation{key})
			if err != nil || s.Observed != tt.observed || s.Unknown != tt.unknown || s.TimingAccounts != tt.timing {
				t.Fatalf("stage=%+v err=%v", s, err)
			}
		})
	}
}

func TestConnectionFunnelLive(t *testing.T) {
	socket := os.Getenv("LMM_TEST_CONNECTION_SOCKET")
	dsn := os.Getenv("ACQUISITION_CONNECTION_DATABASE_URL")
	if socket == "" || dsn == "" {
		t.Skip("requires private Core and own acquisition DB")
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
		method, stage           string
		count, observed, timing int
	}{
		{"api_key", "api_key_created", 1, 1, 1}, {"oauth", "oauth_authorized", 1, 1, 1},
		{"both", "credential_ready", 1, 1, 1}, {"unknown", "credential_ready", 1, 0, 0},
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
		count := 0
		for _, ch := range body.Data.Channels {
			count += ch.Registrations
			for _, stage := range ch.Stages {
				if stage.ID == tt.stage && (stage.Observed != tt.observed || stage.TimingAccounts != tt.timing || stage.Unknown != 0 || stage.MatureKnown != tt.count) {
					t.Fatalf("%s stage: %+v", tt.method, stage)
				}
			}
		}
		if count != tt.count {
			t.Fatalf("%s registrations: %d", tt.method, count)
		}
	}
}
