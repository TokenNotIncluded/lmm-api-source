package payments

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	_ "github.com/lib/pq"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

type stripeActualCoreFixture struct {
	Socket, RestrictedSocket, Token, User, PaymentsURL, CoreAuditURL, Merchant, WebhookSecret string
	PlanID, ChannelID                                                                         int64
}
type stripeLostACKCore struct {
	StripeSubscriptionCore
	lost bool
}

func (c *stripeLostACKCore) AcceptStripeSubscriptionEvent(ctx context.Context, r *pb.AcceptEPaySubscriptionProofRequest) (*pb.StripeSubscriptionEvent, error) {
	result, err := c.StripeSubscriptionCore.AcceptStripeSubscriptionEvent(ctx, r)
	if err == nil && !c.lost {
		c.lost = true
		return nil, status.Error(codes.Unavailable, "synthetic lost ACK after real Core commit")
	}
	return result, err
}

// Requires actual Core62 processes (normal and permission-disabled) and two
// isolated databases. The provider is numeric-loopback synthetic HTTP; HMAC,
// proof storage, payment namespace and grants are verified by the real Core.
// Nothing in this fixture can fabricate a successful grant response.
func TestStripeSubscriptionActualCoreTCPRecovery(t *testing.T) {
	path := os.Getenv("SUB01_STRIPE_CORE_FIXTURE")
	if path == "" {
		t.Skip("dedicated Core62 + PostgreSQL fixture required")
	}
	info, err := os.Stat(path)
	if err != nil || info.Mode().Perm() != 0600 {
		t.Fatal("private fixture file required")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var f stripeActualCoreFixture
	if json.Unmarshal(data, &f) != nil || f.PlanID <= 0 || f.ChannelID <= 0 || f.WebhookSecret == "" || f.RestrictedSocket == "" {
		t.Fatal("incomplete fixture")
	}
	paymentURL, e1 := url.Parse(f.PaymentsURL)
	coreURL, e2 := url.Parse(f.CoreAuditURL)
	if e1 != nil || e2 != nil || paymentURL.Hostname() != "127.0.0.1" || coreURL.Hostname() != "127.0.0.1" || !strings.HasSuffix(paymentURL.Path, "_test") || !strings.HasSuffix(coreURL.Path, "_test") || paymentURL.Path == coreURL.Path {
		t.Fatal("separate private loopback test databases required")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	db, err := sql.Open("postgres", f.PaymentsURL)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(2)
	audit, err := sql.Open("postgres", f.CoreAuditURL)
	if err != nil {
		t.Fatal(err)
	}
	defer audit.Close()
	var isolated bool
	if err = db.QueryRowContext(ctx, `SELECT current_database() LIKE 'lmm_payments%_test' AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\') AND to_regclass('public.payment_orders') IS NULL`).Scan(&isolated); err != nil || !isolated {
		t.Fatal("empty isolated payments test database required", err)
	}
	// Explicit fixture setup, never application startup DDL.
	if _, err = db.ExecContext(ctx, Schema+SubscriptionEPaySchema+SubscriptionStripeSchema); err != nil {
		t.Fatal(err)
	}
	var walletBefore int
	if err = audit.QueryRowContext(ctx, `SELECT count(*) FROM core_ledger.journals`).Scan(&walletBefore); err != nil {
		t.Fatal(err)
	}
	core, err := coreclient.New(f.Socket, []byte(f.Token))
	if err != nil {
		t.Fatal(err)
	}
	defer core.Close()
	restricted, err := coreclient.New(f.RestrictedSocket, []byte(f.Token))
	if err != nil {
		t.Fatal(err)
	}
	defer restricted.Close()
	cfg, _, mux := isolatedStripeFixture(t)
	cfg.Stripe.Channel.ID = f.ChannelID
	cfg.Stripe.Channel.Merchant = f.Merchant
	cfg.Stripe.Channel.Enabled = false
	isolatedBinding(t, mux, cfg, nil)
	var providerMu sync.Mutex
	var order *pb.StripeSubscriptionOrder
	var checkout []byte
	posts := 0
	mux.HandleFunc("/v1/", func(w http.ResponseWriter, r *http.Request) {
		providerMu.Lock()
		defer providerMu.Unlock()
		if r.Header.Get("Stripe-Version") != StripeSubscriptionAPIVersion {
			t.Error("subscription version drift")
		}
		switch r.URL.Path {
		case "/v1/account":
			json.NewEncoder(w).Encode(map[string]any{"id": f.Merchant})
		case "/v1/prices/price_frozen_daily":
			if order == nil {
				t.Error("price queried before Core freeze")
				w.WriteHeader(500)
				return
			}
			amount, _ := strconv.ParseInt(order.Price.AmountMinor, 10, 64)
			json.NewEncoder(w).Encode(map[string]any{"id": order.Price.ProviderPriceId, "object": "price", "type": "recurring", "currency": "usd", "active": true, "livemode": false, "unit_amount": amount, "recurring": map[string]any{"interval": order.Price.Interval, "interval_count": order.Price.IntervalCount}})
		case "/v1/checkout/sessions":
			if r.Method == "GET" {
				if checkout == nil {
					io.WriteString(w, `{"data":[],"has_more":false}`)
				} else {
					fmt.Fprintf(w, `{"data":[%s],"has_more":false}`, checkout)
				}
				return
			}
			posts++
			if err := r.ParseForm(); err != nil {
				t.Error(err)
			}
			if r.Header.Get("Idempotency-Key") != stableKey("stripe-subscription", order.Order.OrderId) || r.Form.Get("mode") != "subscription" || r.Form.Get("line_items[0][price]") != order.Price.ProviderPriceId || r.Form.Get("success_url") != order.Price.SuccessUrl || r.Form.Get("payment_method_types[0]") != "card" {
				t.Error("original checkout identity changed")
			}
			for k, v := range stripeMetadata(order) {
				if r.Form.Get("metadata["+k+"]") != v || r.Form.Get("subscription_data[metadata]["+k+"]") != v {
					t.Error("frozen metadata omitted")
				}
			}
			amount, _ := strconv.ParseInt(order.Price.AmountMinor, 10, 64)
			checkout, _ = json.Marshal(map[string]any{"object": "checkout.session", "id": "cs_original_subscription", "mode": "subscription", "status": "open", "client_reference_id": order.Order.OrderId, "livemode": false, "currency": "usd", "amount_total": amount, "metadata": stripeMetadata(order), "url": cfg.CheckoutOrigin + "/checkout/cs_original_subscription", "expires_at": time.Now().Add(time.Hour).Unix()})
			// Provider committed, but the caller did not receive a checkout response.
			w.WriteHeader(http.StatusServiceUnavailable)
		default:
			w.WriteHeader(404)
		}
	})
	adapter, err := NewStripeIsolated(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	wrapped := &stripeLostACKCore{StripeSubscriptionCore: core}
	service, err := NewStripeSubscriptions(wrapped, db, []*Stripe{adapter}, map[int64]bool{f.ChannelID: true})
	if err != nil {
		t.Fatal(err)
	}
	if adapter.cfg.APIVersion != cfg.Stripe.APIVersion || service.adapters[f.ChannelID].cfg.APIVersion != StripeSubscriptionAPIVersion {
		t.Fatal("wallet adapter was mutated")
	}
	prepared, err := service.Prepare(ctx, f.User, "stripe-original-checkout-0001", f.PlanID, f.ChannelID)
	providerMu.Lock()
	order = prepared
	providerMu.Unlock()
	if err != nil {
		t.Fatal(err)
	}
	if order.Price.ProviderPriceId != "price_frozen_daily" || order.Price.Interval != "day" || order.Price.IntervalCount != 1 {
		t.Fatal("fixture requires fixed one-day price_frozen_daily")
	}
	again, err := service.Prepare(ctx, f.User, "stripe-original-checkout-0001", f.PlanID, f.ChannelID)
	if err != nil || again.Order.OrderId != order.Order.OrderId {
		t.Fatal("original command identity lost", err)
	}
	if _, err = service.Confirm(ctx, f.User, order.Order.OrderId); err == nil {
		t.Fatal("lost provider response acknowledged")
	}
	// Reopen with SUB admission off and wallet disabled; the original key remains
	// recoverable without a user Session in the worker and without another POST.
	recovered, err := NewStripeSubscriptions(core, db, []*Stripe{adapter}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if err = recovered.Recover(ctx, 1); err != nil {
		t.Fatal(err)
	}
	view, err := recovered.Read(ctx, f.User, order.Order.OrderId)
	providerMu.Lock()
	postCount := posts
	providerMu.Unlock()
	if err != nil || view.Checkout == nil || view.Checkout.ID != "cs_original_subscription" || postCount != 1 {
		t.Fatal("original checkout not recovered", err, postCount)
	}
	if _, err = recovered.Prepare(ctx, f.User, "new-purchase-while-paused", f.PlanID, f.ChannelID); err == nil {
		t.Fatal("paused subscriptions admitted a new order")
	}
	current := time.Now().Unix()
	amount, _ := strconv.ParseInt(order.Price.AmountMinor, 10, 64)
	invoice := func(o *pb.StripeSubscriptionOrder, event, invoice, payment string, start, end int64) []byte {
		reason := "subscription_cycle"
		if invoice != "in_next_invoice" {
			reason = "subscription_create"
		}
		raw, err := json.Marshal(map[string]any{"id": event, "type": "invoice.paid", "created": current, "api_version": StripeSubscriptionAPIVersion, "livemode": false, "data": map[string]any{"object": map[string]any{"object": "invoice", "id": invoice, "subscription": "sub_" + o.Order.OrderId, "customer": "cus_" + o.Order.OrderId, "currency": "usd", "total": amount, "amount_due": amount, "amount_paid": amount, "amount_remaining": 0, "paid": true, "paid_out_of_band": false, "status": "paid", "payment_intent": payment, "billing_reason": reason, "subscription_details": map[string]any{"metadata": stripeMetadata(o)}, "lines": map[string]any{"has_more": false, "data": []any{map[string]any{"price": map[string]any{"id": o.Price.ProviderPriceId}, "quantity": 1, "currency": "usd", "amount": amount, "proration": false, "period": map[string]int64{"start": start, "end": end}}}}}}})
		if err != nil {
			t.Fatal(err)
		}
		return raw
	}
	signed := func(raw []byte) string {
		m := hmac.New(sha256.New, []byte(f.WebhookSecret))
		fmt.Fprintf(m, "%d.%s", current, raw)
		return fmt.Sprintf("t=%d,v1=%s", current, hex.EncodeToString(m.Sum(nil)))
	}
	send := func(s *StripeSubscriptions, raw []byte, sig string) int {
		server := httptest.NewServer(s.Handler())
		defer server.Close()
		req, err := http.NewRequestWithContext(ctx, "POST", server.URL+"/callbacks/"+strconv.FormatInt(f.ChannelID, 10), strings.NewReader(string(raw)))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Stripe-Signature", sig)
		req.Header.Set("Content-Type", "application/json")
		res, err := server.Client().Do(req)
		if err != nil {
			t.Fatal(err)
		}
		io.Copy(io.Discard, res.Body)
		res.Body.Close()
		return res.StatusCode
	}
	raw := invoice(order, "evt_original_invoice", "in_original_invoice", "pi_original_invoice", current-30, current+86370)
	if send(service, raw, fmt.Sprintf("t=%d,v1=%s", current, strings.Repeat("0", 64))) == 200 {
		t.Fatal("forged callback acknowledged")
	}
	if send(service, raw, signed(raw)) == 200 || !wrapped.lost {
		t.Fatal("real Core lost ACK was hidden")
	}
	if _, err = db.ExecContext(ctx, `UPDATE payment_stripe_subscription_inbox SET next_attempt=clock_timestamp()-interval '10 seconds' WHERE state='pending'`); err != nil {
		t.Fatal(err)
	}
	if err = recovered.Recover(ctx, 1); err != nil {
		t.Fatal(err)
	}
	if send(recovered, raw, signed(raw)) != 200 {
		t.Fatal("original signed replay failed")
	}
	var count int
	if err = audit.QueryRowContext(ctx, `SELECT count(*) FROM core_subscriptions.stripe_cycles WHERE order_id=$1`, order.Order.OrderId).Scan(&count); err != nil || count != 1 {
		t.Fatal("first invoice granted more than once", count, err)
	}
	// A real permission-disabled Core must not permanently reject the original.
	second, err := service.Prepare(ctx, f.User, "stripe-original-second-0001", f.PlanID, f.ChannelID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = core.ConfirmStripePurchase(ctx, f.User, second.Order.OrderId); err != nil {
		t.Fatal(err)
	}
	denied, err := NewStripeSubscriptions(restricted, db, []*Stripe{adapter}, nil)
	if err != nil {
		t.Fatal(err)
	}
	secondRaw := invoice(second, "evt_permission_restore", "in_permission_restore", "pi_permission_restore", current-30, current+86370)
	if send(denied, secondRaw, signed(secondRaw)) == 200 {
		t.Fatal("disabled real Core acknowledged payment")
	}
	var pending string
	if err = db.QueryRowContext(ctx, `SELECT state FROM payment_stripe_subscription_inbox WHERE order_id=$1`, second.Order.OrderId).Scan(&pending); err != nil || pending != "pending" {
		t.Fatal("permission denial lost original proof", pending, err)
	}
	unknown := proto.Clone(second).(*pb.StripeSubscriptionOrder)
	unknown.Order.OrderId = "sub_" + strings.Repeat("f", 64)
	unknownRaw := invoice(unknown, "evt_unknown_original", "in_unknown_original", "pi_unknown_original", current-30, current+86370)
	if send(recovered, unknownRaw, signed(unknownRaw)) == 200 {
		t.Fatal("unknown original order acknowledged")
	}
	if _, err = db.ExecContext(ctx, `UPDATE payment_stripe_subscription_inbox SET next_attempt=clock_timestamp()-interval '20 seconds' WHERE order_id=$1`, unknown.Order.OrderId); err != nil {
		t.Fatal(err)
	}
	if _, err = db.ExecContext(ctx, `UPDATE payment_stripe_subscription_inbox SET next_attempt=clock_timestamp()-interval '10 seconds' WHERE order_id=$1`, second.Order.OrderId); err != nil {
		t.Fatal(err)
	}
	if err = recovered.Recover(ctx, 1); err == nil {
		t.Fatal("unknown original became success")
	}
	// The unknown candidate is fairly deferred; limit1 must now advance the
	// already-persisted legitimate paid order without another provider delivery.
	if err = recovered.Recover(ctx, 1); err != nil {
		t.Fatal(err)
	}
	if err = audit.QueryRowContext(ctx, `SELECT count(*) FROM core_subscriptions.stripe_cycles WHERE order_id=$1`, second.Order.OrderId).Scan(&count); err != nil || count != 1 {
		t.Fatal("permission restoration did not uniquely grant original proof", err)
	}
	next := invoice(order, "evt_next_invoice", "in_next_invoice", "pi_next_invoice", current+86370, current+172770)
	if send(recovered, next, signed(next)) != 200 {
		t.Fatal("recurring signed invoice failed")
	}
	if err = audit.QueryRowContext(ctx, `SELECT count(*) FROM core_subscriptions.stripe_cycles WHERE order_id=$1`, order.Order.OrderId).Scan(&count); err != nil || count != 2 {
		t.Fatal("recurring cycle not unique", count, err)
	}
	var walletAfter int
	if err = audit.QueryRowContext(ctx, `SELECT count(*) FROM core_ledger.journals`).Scan(&walletAfter); err != nil || walletAfter != walletBefore {
		t.Fatal("subscription altered wallet journals", err)
	}
	t.Log("real Core RPC/HMAC + isolated PG + TCP provider: independent admission, original command/key recovery after lost checkout response, fake-before-real, Core commit/lost ACK, permission restore without redelivery, recurring unique grants; wallet journals unchanged")
}
