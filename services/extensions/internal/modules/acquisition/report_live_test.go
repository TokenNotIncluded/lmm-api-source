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
	"testing"
	"time"
)

func testCoreReportLive(t *testing.T, db *sql.DB, service *Service, client *community.CoreRead, mux *http.ServeMux) {
	t.Helper()
	service.UserRead = client
	now := service.now().Unix()
	for _, q := range []string{
		`UPDATE lmm_acquisition.config SET started_at=$1-3600`,
		`UPDATE lmm_acquisition.accounts SET consent_version=2,lookback_days=30`,
		`INSERT INTO lmm_acquisition.consents(user_id,allowed,version,updated_at) VALUES(101,true,2,$1),(102,true,2,$1)`,
		`INSERT INTO lmm_acquisition.activity_state(singleton,started_at,scanned_through,updated_at,status,incomplete) VALUES(true,$1-3600,$1,$1,'ready',false)`,
	} {
		var err error
		if q == `UPDATE lmm_acquisition.accounts SET consent_version=2,lookback_days=30` {
			_, err = db.Exec(q)
		} else {
			_, err = db.Exec(q, now)
		}
		if err != nil {
			t.Fatal(err)
		}
	}
	path := fmt.Sprintf("/api/admin/acquisition/report?from=%d&to=%d", now-1000, now)
	call := func(path string) (int, acquisitionReport) {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest("GET", path, nil))
		var v struct {
			Success bool
			Data    acquisitionReport
		}
		if err := json.Unmarshal(w.Body.Bytes(), &v); err != nil {
			t.Fatal(err)
		}
		return w.Code, v.Data
	}
	code, v := call(path)
	if code != 200 || len(v.Channels) != 2 || len(v.Payments) != 1 || len(v.Applied) != 1 || v.Applied[0] != 30 || v.State == nil {
		t.Fatalf("real Core report: %d %#v", code, v)
	}
	p := v.Payments[0]
	if p.Source != "unknown" || p.Currency != "USD" || p.Paid != 2740000 || p.Refund != 170000 || p.Net != 2570000 || p.Payers != 1 {
		t.Fatalf("cash source/refund/replay metrics: %#v", p)
	}
	if len(v.Activity) != 2 || v.Activity[0].Source != "campaign-a" || v.Activity[0].Eligible != 1 || v.Activity[0].Successful != 1 || v.Activity[0].Observing != 1 || v.Activity[0].Mature != 0 || v.Activity[0].Rate != nil {
		t.Fatalf("young cohort retention: %#v", v.Activity)
	}
	if _, err := db.Exec(`UPDATE lmm_acquisition.first_payments SET history_conflict_at=0 WHERE user_id=101`); err != nil {
		t.Fatal(err)
	}
	if code, v := call(path); code != 200 || v.Payments[0].Source != "payment-source" {
		t.Fatalf("saved first-payment source: %d %#v", code, v)
	}
	if _, err := db.Exec(`INSERT INTO lmm_acquisition.activity_gaps(from_at,through_at) VALUES($1-1,$1)`, now); err != nil {
		t.Fatal(err)
	}
	if code, v := call(path); code != 200 || v.Activity[0].Incomplete != 1 || v.Activity[1].Incomplete != 1 || v.Activity[0].Rate != nil {
		t.Fatalf("coverage gap hidden: %d %#v", code, v.Activity)
	}

	if _, err := db.Exec(`DELETE FROM lmm_acquisition.activity_gaps`); err != nil {
		t.Fatal(err)
	}
	first, err := client.FirstCashPayments(context.Background(), []int64{101})
	if err != nil {
		t.Fatal(err)
	}
	seventh := (*first.Users[0].FirstPaidAt/daySeconds)*daySeconds + 7*daySeconds
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.activities(user_id,day,first_at,last_at) VALUES(101,$1::bigint,$1::bigint+1,$1::bigint+1)`, seventh); err != nil {
		t.Fatal(err)
	}
	asof := now + 8*daySeconds
	service.now = func() time.Time { return time.Unix(asof, 0) }
	if _, err = db.Exec(`UPDATE lmm_acquisition.activity_state SET scanned_through=$1,updated_at=$1`, asof); err != nil {
		t.Fatal(err)
	}
	maturePath := fmt.Sprintf("/api/admin/acquisition/report?from=%d&to=%d", now-1000, asof)
	if code, v := call(maturePath); code != 200 || v.Activity[0].Mature != 1 || v.Activity[0].Retained != 1 || v.Activity[0].Rate == nil || *v.Activity[0].Rate != 1 {
		t.Fatalf("processed UTC day-seven retention: %d %#v", code, v.Activity)
	}
	if _, err = db.Exec(`DELETE FROM lmm_acquisition.activities WHERE day=$1`, seventh); err != nil {
		t.Fatal(err)
	}
	if code, v := call(maturePath); code != 200 || v.Activity[0].Mature != 1 || v.Activity[0].Retained != 0 || v.Activity[0].Rate == nil || *v.Activity[0].Rate != 0 {
		t.Fatalf("known non-retention: %d %#v", code, v.Activity)
	}
	if code, _ := call(path + "0"); code != 400 {
		t.Fatalf("invalid range: %d", code)
	}
	limited := http.NewServeMux()
	service.Register(limited, listVerifier{0})
	w := httptest.NewRecorder()
	limited.ServeHTTP(w, httptest.NewRequest("GET", path, nil))
	if w.Code != 403 {
		t.Fatalf("nonadmin report: %d", w.Code)
	}
	facts, err := client.CashPaymentTotals(context.Background(), []int64{101, 102}, now-1000, now)
	if err != nil || len(facts.Users) != 2 {
		t.Fatal("real cash source unavailable")
	}
	service.UserRead = nil
	if code, _ := call(path); code != 503 {
		t.Fatalf("missing Core became an empty report: %d", code)
	}
}
