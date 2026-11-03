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
	"strings"
	"testing"
	"time"
)

// Actual Core supplies identities and settled payment facts. Only the external
// administrator session verifier is synthetic; this is developer integration.
func TestCoreCostLive(t *testing.T) {
	socket := os.Getenv("LMM_TEST_CORE_CASH_SOCKET")
	dsn := os.Getenv("ACQUISITION_TEST_DATABASE_URL")
	if socket == "" || dsn == "" {
		t.Skip("requires Rust-owned Core and separate acquisition database")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var exists bool
	if err = db.QueryRow(`SELECT to_regnamespace('lmm_acquisition') IS NOT NULL`).Scan(&exists); err != nil || exists {
		t.Fatalf("empty acquisition database required: %v", err)
	}
	if _, err = db.Exec(Schema); err != nil {
		t.Fatal(err)
	}
	defer db.Exec(`DROP SCHEMA lmm_acquisition CASCADE`)
	client, err := community.NewCoreRead(socket, []byte("synthetic-community-purpose-token-20261104"))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	if err = client.ReadyFirstCashPayments(context.Background()); err != nil {
		t.Fatal(err)
	}
	service, err := New(context.Background(), db, "https://synthetic.example", true)
	if err != nil {
		t.Fatal(err)
	}
	service.UserRead = client
	const link = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
	now := time.Now().Unix()
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.links(id,name,created_at) VALUES($1,'Core-backed cost',1)`, link); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.accounts(user_id,registration_link_id,created_at) VALUES(101,$1,$2),(102,$1,$2)`, link, now); err != nil {
		t.Fatal(err)
	}
	mux := http.NewServeMux()
	service.Register(mux, listVerifier{5})
	body := fmt.Sprintf(`{"link_id":%q,"from":%d,"to":%d,"observation_days":1,"currency":"USD","amount_micros":1000001}`, link, now-1000, now)
	w := httptest.NewRecorder()
	mux.ServeHTTP(w, httptest.NewRequest("PUT", "/api/admin/acquisition/cost", strings.NewReader(body)))
	if w.Code != 200 {
		t.Fatalf("write: %d %s", w.Code, w.Body.String())
	}
	path := fmt.Sprintf("/api/admin/acquisition/cost?link_id=%s&from=%d&to=%d&observation_days=1&currency=USD", link, now-1000, now)
	w = httptest.NewRecorder()
	mux.ServeHTTP(w, httptest.NewRequest("GET", path, nil))
	var result struct {
		Success bool
		Data    costReport
	}
	if err = json.Unmarshal(w.Body.Bytes(), &result); err != nil || w.Code != 200 || !result.Success {
		t.Fatalf("Core-backed cost: %d %s", w.Code, w.Body.String())
	}
	if result.Data.Registrations != 2 || result.Data.FirstPayingAccounts != 1 || result.Data.PerRegistration == nil || *result.Data.PerRegistration != 500001 || !result.Data.Observing || result.Data.PerPayer != nil || result.Data.Unclassified != 0 {
		t.Fatalf("Core-backed metrics: %#v", result.Data)
	}
	t.Run("funnel", func(t *testing.T) { testCoreFunnelLive(t, db, service, client, mux) })

}
