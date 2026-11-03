// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
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

func testCoreFunnelLive(t *testing.T, db *sql.DB, service *Service, client *community.CoreRead, mux *http.ServeMux) {
	t.Helper()
	dsn := os.Getenv("ACQUISITION_FUNNEL_COMMUNITY_DATABASE_URL")
	if dsn == "" {
		t.Skip("requires a separate community rehearsal database")
	}
	cdb, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer cdb.Close()
	var exists bool
	if err = cdb.QueryRow(`SELECT to_regnamespace('lmm_community') IS NOT NULL`).Scan(&exists); err != nil || exists {
		t.Fatalf("empty community database required: %v", err)
	}
	if _, err = cdb.Exec(community.Schema); err != nil {
		t.Fatal(err)
	}
	defer cdb.Exec(`DROP SCHEMA lmm_community CASCADE`)
	svc, err := community.New(context.Background(), cdb, "https://synthetic.example")
	if err != nil {
		t.Fatal(err)
	}
	service.ClientRead = svc
	facts, err := client.FirstCashPayments(context.Background(), []int64{101})
	if err != nil {
		t.Fatal(err)
	}
	at := *facts.Users[0].FirstPaidAt
	now := time.Now().Unix() + 2
	service.now = func() time.Time { return time.Unix(now, 0) }
	if _, err = cdb.Exec(`INSERT INTO lmm_community.onboarding_todos(user_id,key_proven_at,client_installed_at,client_configured_at,created_at,updated_at) VALUES(101,$1,$1,$1,$1,$1)`, at); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`UPDATE lmm_acquisition.accounts SET registration_source='campaign-a',registration_evidence='promotion_link',registration_campaign='spring',registration_content='copy' WHERE user_id=101`); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.activities(user_id,day,first_at,last_at) VALUES(101,($1::bigint/86400)*86400,$1,$1)`, at); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.first_payments(user_id,first_paid_at,source,evidence,rule,lookback_days,inferred,snapshotted_at) VALUES(101,$1,'payment-source','promotion_link','last_touch',30,true,$2)`, at, now); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`UPDATE lmm_acquisition.config SET payment_snapshot_status='ready',payment_snapshot_updated_at=$1`, now); err != nil {
		t.Fatal(err)
	}
	path := fmt.Sprintf("/api/admin/acquisition/funnel?from=%d&to=%d&observation_days=1", now-1000, now)
	call := func(path string) (int, funnelReport) {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest("GET", path, nil))
		var v struct {
			Success bool
			Data    funnelReport
		}
		if err = json.Unmarshal(w.Body.Bytes(), &v); err != nil {
			t.Fatal(err)
		}
		return w.Code, v.Data
	}
	code, result := call(path)
	if code != 200 || len(result.Channels) != 2 || result.PaymentStatus != "ready" || len(result.Sources) != 1 || result.Sources[0].Source != "payment-source" || result.Sources[0].Accounts != 1 {
		t.Fatalf("real Core funnel: %d %#v", code, result)
	}
	first := result.Channels[0]
	if first.Source != "campaign-a" || first.Registrations != 1 || first.Observing != 1 || first.Mature != 0 || len(first.Stages) != 9 {
		t.Fatalf("cohort: %#v", first)
	}
	stages := map[string]funnelStage{}
	for _, s := range first.Stages {
		stages[s.ID] = s
	}
	for _, id := range []string{"first_payment", "repeat_payment"} {
		s := stages[id]
		if s.Observed != 1 || s.Unknown != 0 || s.TimingAccounts != 1 || s.ConversionRate != nil {
			t.Fatalf("payment stage %s: %#v", id, s)
		}
	}
	for _, id := range []string{"client_configured", "first_successful_request"} {
		s := stages[id]
		if s.Observed != 1 || s.TimingAccounts != 0 || s.MeanSeconds != nil {
			t.Fatalf("sparse positive %s: %#v", id, s)
		}
	}
	if s := stages["access_approved"]; s.Unknown != 1 || s.NotObserved != 0 || s.ConversionRate != nil {
		t.Fatalf("approval time inferred from current permission: %#v", s)
	}
	for _, s := range result.Channels[1].Stages {
		if s.ID == "first_payment" && (s.Observed != 0 || s.NotObserved != 1 || s.Unknown != 0) {
			t.Fatalf("test-mode credit counted as cash: %#v", s)
		}
	}
	for _, suffix := range []string{"&source=campaign-a", "&campaign=spring", "&content=copy"} {
		code, v := call(path + suffix)
		if code != 200 || len(v.Channels) != 1 || v.Channels[0].Source != "campaign-a" {
			t.Fatalf("filter %s: %d %#v", suffix, code, v)
		}
	}
	if code, v := call(path + "&campaign=absent"); code != 200 || len(v.Channels) != 0 || len(v.Sources) != 0 {
		t.Fatalf("empty cohort: %d %#v", code, v)
	}
	if code, _ := call(path + "&connection_method=oauth"); code != 503 {
		t.Fatalf("unsupported filter silently ignored: %d", code)
	}
	if code, _ := call(path + "&connection_method=invalid"); code != 400 {
		t.Fatalf("invalid filter: %d", code)
	}
	if _, err = db.Exec(`UPDATE lmm_acquisition.first_payments SET history_conflict_at=$1 WHERE user_id=101`, now); err != nil {
		t.Fatal(err)
	}
	if code, v := call(path); code != 200 || v.PaymentStatus != "incomplete" || len(v.Sources) != 0 {
		t.Fatalf("conflicting attribution: %d %#v", code, v)
	}
	limited := http.NewServeMux()
	service.Register(limited, listVerifier{0})
	w := httptest.NewRecorder()
	limited.ServeHTTP(w, httptest.NewRequest("GET", path, nil))
	if w.Code != 403 {
		t.Fatalf("nonadmin funnel: %d", w.Code)
	}
	service.UserRead = nil
	if code, _ := call(path); code != 503 {
		t.Fatalf("Core missing should fail closed: %d", code)
	}
}
