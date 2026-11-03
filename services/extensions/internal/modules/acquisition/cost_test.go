// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"
)

type costFacts struct {
	testUserRead
	now   int64
	facts map[int64]community.FirstCashPayment
	fail  bool
}

func (c *costFacts) UserPage(context.Context, int64, int) (community.RegistrationPage, error) {
	users := []community.RegistrationUser{}
	for _, id := range []int64{77, 78, 79} {
		at := c.now - (102-id)*daySeconds
		role := "user"
		if id == 79 {
			role = "admin"
		}
		users = append(users, community.RegistrationUser{UserID: id, RegisteredAt: &at, PlatformRole: role, Active: true})
	}
	return community.RegistrationPage{Users: users, NextID: 79}, nil
}
func (c *costFacts) FirstCashPayments(ctx context.Context, ids []int64) (community.FirstCashPayments, error) {
	if c.fail {
		return community.FirstCashPayments{}, errors.New("synthetic Core read unavailable")
	}
	result := community.FirstCashPayments{}
	for _, id := range ids {
		if fact, ok := c.facts[id]; ok {
			result.Users = append(result.Users, fact)
		}
	}
	return result, nil
}

func TestCostPostgresHTTP(t *testing.T) {
	dsn := os.Getenv("ACQUISITION_TEST_DATABASE_URL")
	if dsn == "" {
		t.Skip("requires isolated PostgreSQL database")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var exists bool
	if err = db.QueryRow(`SELECT to_regnamespace('lmm_acquisition') IS NOT NULL`).Scan(&exists); err != nil || exists {
		t.Fatalf("requires empty isolated database: %v", err)
	}
	if _, err = db.Exec(Schema); err != nil {
		t.Fatal(err)
	}
	defer db.Exec(`DROP SCHEMA lmm_acquisition CASCADE`)
	const now int64 = 1800000000
	const link = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.links(id,name,created_at) VALUES($1,'synthetic',1);`, link); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.accounts(user_id,registration_link_id,created_at) SELECT n,$1,$2 FROM generate_series(77,79) n`, link, now-25*daySeconds); err != nil {
		t.Fatal(err)
	}
	service, err := New(context.Background(), db, "https://synthetic.example", true)
	if err != nil {
		t.Fatal(err)
	}
	service.now = func() time.Time { return time.Unix(now, 0) }
	paid := now - 24*daySeconds
	reader := &costFacts{testUserRead: testUserRead{role: "user"}, now: now, facts: map[int64]community.FirstCashPayment{
		77: {UserID: 77, FirstPaidAt: &paid, HistoryComplete: true}, 78: {UserID: 78, HistoryComplete: true}}}
	service.UserRead = reader
	mux := http.NewServeMux()
	service.Register(mux, listVerifier{5})
	request := func(method, path, body string) *httptest.ResponseRecorder {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest(method, path, strings.NewReader(body)))
		return w
	}
	path := fmt.Sprintf("/api/admin/acquisition/cost?link_id=%s&from=%d&to=%d&observation_days=7&currency=USD", link, now-30*daySeconds, now-20*daySeconds)
	body := fmt.Sprintf(`{"link_id":%q,"from":%d,"to":%d,"observation_days":7,"currency":"USD","amount_micros":1000001}`, link, now-30*daySeconds, now-20*daySeconds)
	if w := request("PUT", "/api/admin/acquisition/cost", body); w.Code != 200 {
		t.Fatalf("write: %d %s", w.Code, w.Body.String())
	}
	read := func() costReport {
		w := request("GET", path, "")
		var envelope struct {
			Success bool
			Data    costReport
		}
		if err = json.Unmarshal(w.Body.Bytes(), &envelope); err != nil || w.Code != 200 || !envelope.Success {
			t.Fatalf("read: %d %s", w.Code, w.Body.String())
		}
		return envelope.Data
	}
	result := read()
	if result.Registrations != 2 || result.FirstPayingAccounts != 1 || result.PerRegistration == nil || *result.PerRegistration != 500001 || result.PerPayer == nil || *result.PerPayer != 1000001 || result.Observing {
		t.Fatalf("cost report: %#v", result)
	}
	if w := request("GET", strings.Replace(path, "currency=USD", "currency=EUR", 1), ""); w.Code != 200 || !strings.Contains(w.Body.String(), `"spend":null`) {
		t.Fatalf("currency scopes mixed: %d %s", w.Code, w.Body.String())
	}
	unknown := reader.facts[77]
	unknown.HistoryComplete = false
	unknown.FirstPaidAt = nil
	unknown.UnclassifiedOrders = 1
	reader.facts[77] = unknown
	result = read()
	if result.Unclassified != 1 || result.PerPayer != nil || result.PerRegistration == nil {
		t.Fatalf("unknown money fabricated a final cost: %#v", result)
	}
	unknown.UnclassifiedOrders = 0
	reader.facts[77] = unknown
	if w := request("GET", path, ""); w.Code != 503 {
		t.Fatalf("missing historical coverage became zero: %d", w.Code)
	}
	reader.facts[77] = community.FirstCashPayment{UserID: 77, FirstPaidAt: &paid, HistoryComplete: true}
	reader.fail = true
	if w := request("GET", path, ""); w.Code != 503 {
		t.Fatalf("Core outage became zero: %d", w.Code)
	}
	reader.fail = false
	service.now = func() time.Time { return time.Unix(now-15*daySeconds, 0) }
	result = read()
	if !result.Observing || result.PerPayer != nil {
		t.Fatalf("observation closed early: %#v", result)
	}
	service.now = func() time.Time { return time.Unix(now, 0) }
	if w := request("GET", strings.Replace(path, "observation_days=7", "observation_days=0", 1), ""); w.Code != 400 {
		t.Fatalf("invalid observation accepted: %d", w.Code)
	}
	if w := request("GET", strings.Replace(path, link, strings.Repeat("b", 32), 1), ""); w.Code != 404 {
		t.Fatalf("missing link became zero: %d", w.Code)
	}
	denied := http.NewServeMux()
	service.Register(denied, listVerifier{0})
	w := httptest.NewRecorder()
	denied.ServeHTTP(w, httptest.NewRequest("GET", path, nil))
	if w.Code != 403 {
		t.Fatalf("nonadmin cost read: %d", w.Code)
	}
}
