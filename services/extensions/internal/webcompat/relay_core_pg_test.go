// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat_test

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	_ "github.com/lib/pq"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcprelay"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store/rustauth"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store/rustfunds"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/webcompat"
)

// Only the acknowledgement is dropped. Rust has committed the real command
// before this adapter injects a transport failure.
type loseRelayAck struct {
	*rustfunds.Adapter
	losePay, loseRefund bool
	refunds             int
}

func (a *loseRelayAck) Execute(ctx context.Context, token string, c store.MoneyCommand) (store.Receipt, error) {
	if c.Kind == "refund" {
		a.refunds++
	}
	r, e := a.Adapter.Execute(ctx, token, c)
	if e == nil && r.Outcome == "succeeded" {
		if c.Kind == "pay" && a.losePay {
			a.losePay = false
			return store.Receipt{}, store.ErrPending
		}
		if c.Kind == "refund" && a.loseRefund {
			a.loseRefund = false
			return store.Receipt{}, store.ErrPending
		}
	}
	return r, e
}

// Invoked by scripts/test_relay_expired_refund_pg.py against the real Core binary.
// The independent processes share only the private test databases and Core UDS.
func TestRelayExpiredRefundCoreProcess(t *testing.T) {
	phase := os.Getenv("LMM_REFUND_TEST_PHASE")
	if phase == "" {
		t.Skip("driven by scripts/test_relay_expired_refund_pg.py")
	}
	if phase != "lost" && phase != "restart" && phase != "revoked" {
		t.Fatal("invalid test phase")
	}
	ctx := context.Background()
	db, e := sql.Open("postgres", os.Getenv("LMM_STORE_TEST_DSN"))
	refundMust(t, e)
	defer db.Close()
	var name string
	refundMust(t, db.QueryRow("SELECT current_database()").Scan(&name))
	if name != "relay_refund_test" {
		t.Fatal("requires dedicated relay_refund_test")
	}
	if phase == "lost" {
		_, e = db.Exec(store.Schema)
		refundMust(t, e)
	}
	repo, e := store.NewPostgres(db)
	refundMust(t, e)
	socket := os.Getenv("LMM_REFUND_TEST_SOCKET")
	idclient, e := coreclient.New(socket, []byte("synthetic-identity-relay-refund-service-credential"))
	refundMust(t, e)
	defer idclient.Close()
	cashclient, e := coreclient.New(socket, []byte("synthetic-commerce-relay-refund-service-credential"))
	refundMust(t, e)
	defer cashclient.Close()
	caps, e := idclient.Capabilities(ctx)
	refundMust(t, e)
	live, e := rustfunds.New(cashclient, caps.GetFeatures())
	refundMust(t, e)
	fail := &loseRelayAck{Adapter: live, losePay: phase == "lost", loseRefund: phase == "lost"}
	var deadline int64
	oldMonth := "2021-01"
	const buyerID = int64(7)
	orderID := store.ID("ssmcp-relay-order-v1", buyerID, "expired-relay")
	grantChecks := 0
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.Header.Get("Authorization") != "Bearer "+strings.Repeat("ce", 32) {
			t.Error("unauthenticated accounting request")
			w.WriteHeader(401)
			return
		}
		switch r.URL.Path {
		case "/internal/v1/ssmcp/quota":
			var in map[string]string
			if e := json.NewDecoder(r.Body).Decode(&in); e != nil || in["user_id"] != "7" {
				t.Error("wrong quota subject")
				w.WriteHeader(400)
				return
			}
			month := oldMonth
			reset := deadline
			if phase != "lost" || time.Now().Unix() >= deadline {
				month = "2021-02"
				reset = time.Now().Add(time.Hour).Unix()
			}
			fmt.Fprintf(w, `{"user_id":"7","username":"fixture","period":%q,"reset_at":%d,"free_bytes":"0","paid_bytes":"0","used_bytes":"0","uploaded_bytes":"0","downloaded_bytes":"0","legacy_used_bytes":"0","limit_bytes":"0","remaining_bytes":"0"}`, month, reset)
		case "/internal/v1/ssmcp/entitlements":
			var in ssmcprelay.Entitlement
			if err := json.NewDecoder(r.Body).Decode(&in); err != nil || in.UserID != "7" || in.OrderID != orderID || in.Period != oldMonth || in.Bytes != "3221225472" || time.Now().Unix() < deadline {
				t.Error("wrong entitlement probe")
				w.WriteHeader(400)
				return
			}
			grantChecks++
			w.WriteHeader(409)
			fmt.Fprint(w, `{"error":"period_mismatch"}`)
		default:
			t.Error("unknown accounting endpoint")
			w.WriteHeader(404)
		}
	}))
	defer upstream.Close()
	relay, e := ssmcprelay.NewClient(upstream.URL, strings.Repeat("ce", 32))
	refundMust(t, e)
	merchant := store.Account{Kind: "personal", ID: 11}
	svc, e := ssmcprelay.New(relay, repo, fail, merchant)
	refundMust(t, e)
	auth, e := rustauth.New(idclient)
	refundMust(t, e)
	refundMust(t, svc.WithRefundAuthority(auth))
	browser, e := webcompat.NewAuthenticator(idclient, []byte(strings.Repeat("h", 40)))
	refundMust(t, e)
	mux := http.NewServeMux()
	webcompat.RegisterSSMCPRelay(mux, browser, svc)
	client := httptest.NewServer(mux)
	defer client.Close()
	request := func(token string) (int, map[string]json.RawMessage) {
		req, e := http.NewRequest("POST", client.URL+"/api/ssmcpd/admin/orders/"+orderID+"/refund", strings.NewReader(`{"user_id":7,"order_id":"`+orderID+`"}`))
		refundMust(t, e)
		req.Header.Set("Authorization", "Bearer "+token)
		req.Header.Set("Content-Type", "application/json")
		response, e := client.Client().Do(req)
		refundMust(t, e)
		defer response.Body.Close()
		var envelope map[string]json.RawMessage
		refundMust(t, json.NewDecoder(response.Body).Decode(&envelope))
		return response.StatusCode, envelope
	}
	buyer, operator, other := os.Getenv("LMM_REFUND_FIXTURE_BUYER"), os.Getenv("LMM_REFUND_FIXTURE_MERCHANT"), os.Getenv("LMM_REFUND_FIXTURE_OTHER")
	denyOutsiders := func() {
		for _, denied := range []string{buyer, other} {
			code, env := request(denied)
			if code != 403 || env["data"] != nil {
				t.Fatalf("scope check: HTTP %d fields %v", code, len(env))
			}
		}
	}
	if phase == "revoked" {
		denyOutsiders()
		code, env := request(operator)
		if code != 401 || env["data"] != nil || fail.refunds != 0 {
			t.Fatalf("revoked replay %d", code)
		}
		return
	}
	if phase == "lost" {
		deadline = time.Now().Add(3 * time.Second).Unix()
		// Exact, frozen credits with a non-USD-cent unit price.
		priceSvc, e := store.New(repo, auth, live)
		refundMust(t, e)
		_, e = priceSvc.Legacy(ctx, operator, merchant, store.LegacyRequest{Action: "set_minimum_price", Body: json.RawMessage(`{"relay_price_quota_per_gib":3}`)})
		refundMust(t, e)
		o, e := svc.Preview(ctx, buyerID, "expired-relay", 3)
		refundMust(t, e)
		if o.ID != orderID || o.TotalPriceQuota != 9 {
			t.Fatal(o)
		}
		_, e = svc.Confirm(ctx, buyerID, buyer, o.ID)
		if !errors.Is(e, store.ErrPending) || fail.losePay {
			t.Fatal("original real pay was not committed", e)
		}
		denyOutsiders()
		time.Sleep(time.Until(time.Unix(o.ResetAt, 0)) + 10*time.Millisecond)
		code, env := request(operator)
		var seen ssmcprelay.Order
		refundMust(t, json.Unmarshal(env["data"], &seen))
		if code != 202 || string(env["success"]) != "false" || seen.State != "refund_pending" || seen.PaymentRef == "" || seen.GrantAbsentInPeriod != "2021-02" || seen.RefundRef != "" || fail.loseRefund || fail.refunds != 1 || grantChecks != 1 {
			t.Fatalf("lost acknowledgement, HTTP %d: %+v submits=%d proofs=%d", code, seen, fail.refunds, grantChecks)
		}
		return
	}
	denyOutsiders()
	for i := 0; i < 2; i++ {
		code, env := request(operator)
		var seen ssmcprelay.Order
		refundMust(t, json.Unmarshal(env["data"], &seen))
		if code != 200 || seen.State != "refunded" || seen.RefundRef == "" || fail.refunds != 0 || grantChecks != 0 {
			t.Fatalf("restart replay %d HTTP %d: %+v", i, code, seen)
		}
	}
}

func refundMust(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}
