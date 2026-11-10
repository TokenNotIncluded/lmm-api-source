package pgtest

import (
	"context"
	"database/sql"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments/rustbridge"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type subscriptionCoreFixture struct{ Socket, RestrictedSocket, ReassignedSocket, Token, DatabaseURL, DatabaseName, FirstOrder, SecondOrder, ThirdOrder, FourthOrder, RawFirst, RawSecond, RawThird, RawFourth string }
type interruptedSubscriptionCore struct {
	p.SubscriptionCore
	mu        sync.Mutex
	first     string
	lost      bool
	transient string
}

func (c *interruptedSubscriptionCore) GetEPaySubscriptionProof(ctx context.Context, r *pb.GetEPaySubscriptionProofRequest) (*pb.EPaySubscriptionProof, error) {
	if r.OrderId == c.transient {
		return nil, status.Error(codes.Unavailable, "synthetic ongoing network outage")
	}
	return c.SubscriptionCore.GetEPaySubscriptionProof(ctx, r)
}
func (c *interruptedSubscriptionCore) ApplyEPaySubscriptionGrant(ctx context.Context, r *pb.ApplyEPaySubscriptionGrantRequest) (*pb.EPaySubscriptionGrant, error) {
	g, err := c.SubscriptionCore.ApplyEPaySubscriptionGrant(ctx, r)
	c.mu.Lock()
	defer c.mu.Unlock()
	if err == nil && r.OrderId == c.first && !c.lost {
		c.lost = true
		return nil, status.Error(codes.Unavailable, "synthetic lost ACK after actual Core COMMIT")
	}
	return g, err
}

// Launched by the Rust PG test with an actual Rust Core RPC service. No fake
// signature verifier or entitlement writer exists in this test.
func TestSubscriptionEPayCoreTCPPoisoningRecovery(t *testing.T) {
	path := os.Getenv("SUB01_CORE_FIXTURE")
	if path == "" {
		t.Skip("Rust Core/PG fixture required")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var f subscriptionCoreFixture
	if err = json.Unmarshal(data, &f); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	dsn, _ := url.Parse(f.DatabaseURL)
	// SQLx-only connection options must not become lib/pq startup GUCs.
	query := url.Values{}
	query.Set("sslmode", "disable")
	dsn.RawQuery = query.Encode()
	db, err := sql.Open("postgres", dsn.String())
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(2)
	if _, err = db.ExecContext(ctx, p.Schema+"\n"+p.SubscriptionEPaySchema); err != nil {
		t.Fatal(err)
	}
	store, err := p.NewPostgres(ctx, db, f.DatabaseName)
	if err != nil {
		t.Fatal(err)
	}
	core, err := coreclient.New(f.Socket, []byte(f.Token))
	if err != nil {
		t.Fatal(err)
	}
	defer core.Close()
	signer, err := rustbridge.New(core)
	if err != nil {
		t.Fatal(err)
	}
	adapter, err := p.NewCoreEPay(p.EPayConfig{Channel: p.Channel{ID: 7, Provider: "epay", Merchant: "1001", Environment: "test", Currency: "CNY", Enabled: true}, PaymentType: "alipay"}, signer)
	if err != nil {
		t.Fatal(err)
	}
	wrapped := &interruptedSubscriptionCore{SubscriptionCore: core, first: f.FirstOrder, transient: "sub_" + strings.Repeat("e", 64)}
	service, err := p.NewEPaySubscriptions(wrapped, store, []*p.EPay{adapter})
	if err != nil {
		t.Fatal(err)
	}
	server := httptest.NewServer(service.Handler())
	defer server.Close()
	sendTo := func(target *httptest.Server, raw string) int {
		r, err := http.NewRequestWithContext(ctx, http.MethodPost, target.URL+"/callbacks/7", strings.NewReader(raw))
		if err != nil {
			t.Fatal(err)
		}
		r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		resp, err := target.Client().Do(r)
		if err != nil {
			t.Fatal(err)
		}
		_, _ = io.Copy(io.Discard, resp.Body)
		_ = resp.Body.Close()
		return resp.StatusCode
	}
	send := func(raw string) int { return sendTo(server, raw) }
	forged := func(order, transaction string) string {
		return url.Values{"pid": {"1001"}, "type": {"alipay"}, "out_trade_no": {order}, "trade_no": {transaction}, "money": {"1.25"}, "trade_status": {"TRADE_SUCCESS"}, "sign_type": {"MD5"}, "sign": {strings.Repeat("00", 16)}}.Encode()
	}
	if send(forged("sub_"+strings.Repeat("f", 64), "fake-unknown")) == 200 {
		t.Fatal("unknown forged order acknowledged")
	}
	firstParams, _ := url.ParseQuery(f.RawFirst)
	if send(forged(f.FirstOrder, firstParams.Get("trade_no"))) == 200 {
		t.Fatal("fake signature acknowledged")
	}
	if send(f.RawFirst) == 200 {
		t.Fatal("lost grant ACK incorrectly reported success")
	}
	var rejected, pending int
	if err = db.QueryRowContext(ctx, `SELECT count(*) FILTER(WHERE state='rejected'),count(*) FILTER(WHERE state='pending') FROM payment_subscription_inbox`).Scan(&rejected, &pending); err != nil || rejected != 1 || pending != 2 {
		t.Fatalf("poison evidence state: rejected=%d pending=%d err=%v", rejected, pending, err)
	}
	// Durable outage candidate remains pending without starving the next valid order.
	queue := func(raw string) {
		req := httptest.NewRequest(http.MethodPost, "https://local.test", strings.NewReader(raw))
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		e, err := adapter.Verify(req, time.Now())
		if err != nil {
			t.Fatal(err)
		}
		if _, err = store.PutSubscriptionEvidence(ctx, e); err != nil {
			t.Fatal(err)
		}
	}
	queue(forged(wrapped.transient, "transient-outage"))
	queue(f.RawSecond)
	if _, err = db.ExecContext(ctx, `UPDATE payment_subscription_inbox SET next_attempt=clock_timestamp()-interval '2 seconds' WHERE order_id=$1`, wrapped.transient); err != nil {
		t.Fatal(err)
	}
	if err = service.Recover(ctx, 1); err == nil || status.Code(err) != codes.Unavailable {
		t.Fatal("transient outage became success")
	}
	// Recreate the Go service on the same DB. Other due candidates must advance
	// with limit=1 even while the outage candidate remains unresolved.
	store, err = p.NewPostgres(ctx, db, f.DatabaseName)
	if err != nil {
		t.Fatal(err)
	}
	service, err = p.NewEPaySubscriptions(wrapped, store, []*p.EPay{adapter})
	if err != nil {
		t.Fatal(err)
	}
	if err = service.Recover(ctx, 1); err != nil {
		t.Fatal(err)
	}
	if _, err = db.ExecContext(ctx, `UPDATE payment_subscription_inbox SET next_attempt=clock_timestamp()-interval '1 second' WHERE order_id=$1 AND state='pending'`, f.FirstOrder); err != nil {
		t.Fatal(err)
	}
	if err = service.Recover(ctx, 1); err != nil {
		t.Fatal(err)
	}
	var granted, transient int
	if err = db.QueryRowContext(ctx, `SELECT count(*) FILTER(WHERE state='granted'),count(*) FILTER(WHERE state='pending') FROM payment_subscription_inbox`).Scan(&granted, &transient); err != nil || granted != 2 || transient != 2 {
		t.Fatalf("starved valid order or lost UNKNOWN: granted=%d pending=%d err=%v", granted, transient, err)
	}
	deniedCore, err := coreclient.New(f.RestrictedSocket, []byte(f.Token))
	if err != nil {
		t.Fatal(err)
	}
	defer deniedCore.Close()
	deniedService, err := p.NewEPaySubscriptions(deniedCore, store, []*p.EPay{adapter})
	if err != nil {
		t.Fatal(err)
	}
	deniedHTTP := httptest.NewServer(deniedService.Handler())
	defer deniedHTTP.Close()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, deniedHTTP.URL+"/callbacks/7", strings.NewReader(f.RawThird))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	response, err := deniedHTTP.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = io.Copy(io.Discard, response.Body)
	_ = response.Body.Close()
	if response.StatusCode == 200 {
		t.Fatal("disabled real Core permissions acknowledged payment")
	}
	var thirdState string
	if err = db.QueryRowContext(ctx, `SELECT state FROM payment_subscription_inbox WHERE order_id=$1`, f.ThirdOrder).Scan(&thirdState); err != nil || thirdState != "pending" {
		t.Fatalf("permission denial lost the paid callback: %q %v", thirdState, err)
	}
	// Restore the configured Core capability using the same durable queue, with
	// no second provider delivery. Core authenticates and grants exactly once.
	if _, err = db.ExecContext(ctx, `UPDATE payment_subscription_inbox SET next_attempt=clock_timestamp()-interval '10 seconds' WHERE order_id=$1`, f.ThirdOrder); err != nil {
		t.Fatal(err)
	}
	if err = service.Recover(ctx, 1); err != nil {
		t.Fatal(err)
	}
	if err = db.QueryRowContext(ctx, `SELECT state FROM payment_subscription_inbox WHERE order_id=$1`, f.ThirdOrder).Scan(&thirdState); err != nil || thirdState != "granted" {
		t.Fatalf("restored permission failed recovery: %q %v", thirdState, err)
	}
	// The same configured channel ID is temporarily a valid Stripe channel in
	// actual Core. This is not evidence that the original EPay callback is bad.
	reassignedCore, err := coreclient.New(f.ReassignedSocket, []byte(f.Token))
	if err != nil {
		t.Fatal(err)
	}
	defer reassignedCore.Close()
	reassignedService, err := p.NewEPaySubscriptions(reassignedCore, store, []*p.EPay{adapter})
	if err != nil {
		t.Fatal(err)
	}
	reassignedHTTP := httptest.NewServer(reassignedService.Handler())
	defer reassignedHTTP.Close()
	if sendTo(reassignedHTTP, f.RawFourth) == http.StatusOK {
		t.Fatal("temporarily reassigned provider acknowledged payment")
	}
	var fourthState string
	if err = db.QueryRowContext(ctx, `SELECT state FROM payment_subscription_inbox WHERE order_id=$1`, f.FourthOrder).Scan(&fourthState); err != nil || fourthState != "pending" {
		t.Fatalf("provider configuration drift rejected original proof: %q %v", fourthState, err)
	}
	if _, err = db.ExecContext(ctx, `UPDATE payment_subscription_inbox SET next_attempt=clock_timestamp()-interval '10 seconds' WHERE order_id=$1`, f.FourthOrder); err != nil {
		t.Fatal(err)
	}
	// Restore the EPay-configured Core over the SAME durable inbox. No second
	// callback is sent: the original persisted evidence must grant exactly once.
	if err = service.Recover(ctx, 1); err != nil {
		t.Fatal(err)
	}
	if err = db.QueryRowContext(ctx, `SELECT state FROM payment_subscription_inbox WHERE order_id=$1`, f.FourthOrder).Scan(&fourthState); err != nil || fourthState != "granted" {
		t.Fatalf("restored EPay configuration failed recovery: %q %v", fourthState, err)
	}
	if send(f.RawFirst) != 200 {
		t.Fatal("actual original replay failed")
	}
	t.Log("actual TCP callbacks + Rust Core signature/RPC + isolated PG: forged unknown and same-event inputs rejected, evidence retained, limit1 fairness, COMMIT/lost-ACK/reopen recovery; permission and provider configuration restored without provider redelivery; final grants checked by Rust")
}
