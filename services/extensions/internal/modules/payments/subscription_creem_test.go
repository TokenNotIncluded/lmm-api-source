package payments

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"errors"
	"net/http"
	"os"
	"strings"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	_ "github.com/lib/pq"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

func creemSubscriptionOrder(currency string) *pb.CreemSubscriptionOrder {
	return &pb.CreemSubscriptionOrder{Order: &pb.EPaySubscriptionOrder{OrderId: "sub_" + strings.Repeat("c", 64), AccountId: 1, PlanId: 1, PlanRevision: 1, PaymentChannelId: 2, Currency: currency, AmountMinor: "1000", PlanJson: []byte(`{"limit_units":"10000"}`)}, Price: &pb.CreemSubscriptionPrice{PlanId: 1, PlanRevision: 1, ChannelId: 2, Currency: currency, AmountMinor: "1000", ProviderPriceId: "prod_recurring", Interval: "month", IntervalCount: 1, SuccessUrl: "https://example.test/subscriptions", CancelUrl: "https://example.test/subscriptions"}, Merchant: "merchant_fixture", Environment: "test"}
}
func creemProduct(currency string, price int) string {
	raw, _ := json.Marshal(map[string]any{"id": "prod_recurring", "object": "product", "mode": "test", "status": "active", "price": price, "currency": currency, "billing_type": "recurring", "billing_period": "every-month", "tax_mode": "inclusive"})
	return string(raw)
}
func creemCheckoutResponse(o *pb.CreemSubscriptionOrder) string {
	raw, _ := json.Marshal(map[string]any{"id": "ch_original", "object": "checkout", "mode": "test", "request_id": o.Order.OrderId, "product": "prod_recurring", "metadata": creemMetadata(o), "checkout_url": "https://checkout.creem.io/ch_original"})
	return string(raw)
}
func TestCreemSubscriptionProductCurrencyAndTaxPreflight(t *testing.T) {
	for _, currency := range []string{"USD", "CNY", "EUR"} {
		t.Run(currency, func(t *testing.T) {
			cfg := creemFixture(t).cfg
			cfg.Channel.Currency = currency
			cfg.ProductID = ""
			cfg.ProductAmountMinor = 0
			a, err := NewCreemSubscriptionAdapter(cfg)
			if err != nil {
				t.Fatal(err)
			}
			o := creemSubscriptionOrder(currency)
			if !validCreemOrder(o) {
				t.Fatal("valid frozen currency rejected")
			}
			raw := creemProduct(currency, 1000)
			a.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
				if r.Method != http.MethodGet || r.URL.Query().Get("product_id") != "prod_recurring" {
					t.Fatal("wrong product request")
				}
				return response(raw), nil
			})}
			if err = a.validateSubscriptionPrice(context.Background(), o); err != nil {
				t.Fatal(err)
			}
			for _, bad := range []string{creemProduct(currency, 999), strings.Replace(raw, `"inclusive"`, `"exclusive"`, 1), strings.Replace(raw, `"recurring"`, `"onetime"`, 1), strings.Replace(raw, `"every-month"`, `"every-year"`, 1)} {
				saved := raw
				raw = bad
				if err = a.validateSubscriptionPrice(context.Background(), o); !errors.Is(err, ErrConflict) {
					t.Fatal("changed product accepted", err)
				}
				raw = saved
			}
			unsupported := proto.Clone(o).(*pb.CreemSubscriptionOrder)
			unsupported.Price.Currency = "JPY"
			unsupported.Order.Currency = "JPY"
			if validCreemOrder(unsupported) {
				t.Fatal("unsupported currency accepted")
			}
		})
	}
}

// The fake Core certifies Go queue behavior only. Native evidence/grants are
// tested independently by the real Core PostgreSQL case in creem_tests.rs.
type creemQueueCore struct {
	order    *pb.CreemSubscriptionOrder
	receipts map[string]*pb.CreemSubscriptionEvent
	accepts  int
	lostACK  bool
}

func (f *creemQueueCore) PrepareCreemPurchase(context.Context, string, *pb.PrepareEPayPurchaseRequest) (*pb.CreemSubscriptionOrder, error) {
	return proto.Clone(f.order).(*pb.CreemSubscriptionOrder), nil
}
func (f *creemQueueCore) ReadCreemPurchase(_ context.Context, user, id string) (*pb.CreemSubscriptionOrder, error) {
	if user != "owner" || id != f.order.Order.OrderId {
		return nil, status.Error(codes.NotFound, "order")
	}
	return proto.Clone(f.order).(*pb.CreemSubscriptionOrder), nil
}
func (f *creemQueueCore) ConfirmCreemPurchase(ctx context.Context, user, id string) (*pb.CreemSubscriptionOrder, error) {
	o, e := f.ReadCreemPurchase(ctx, user, id)
	if e != nil {
		return nil, e
	}
	n := time.Now().Unix()
	o.Order.ConfirmedAtUnix = &n
	f.order = o
	return o, nil
}
func (f *creemQueueCore) ResolveCreemSubscriptionOrder(context.Context, *pb.CreemSubscriptionLookup) (*pb.EPaySubscriptionOrderRequest, error) {
	return &pb.EPaySubscriptionOrderRequest{OrderId: f.order.Order.OrderId}, nil
}
func (f *creemQueueCore) GetCreemSubscriptionEvent(_ context.Context, r *pb.GetCreemSubscriptionEventRequest) (*pb.CreemSubscriptionEvent, error) {
	v := f.receipts[r.ProviderEventId]
	if v == nil {
		return nil, status.Error(codes.NotFound, "receipt")
	}
	return v, nil
}
func (f *creemQueueCore) AcceptCreemSubscriptionEvent(_ context.Context, r *pb.AcceptCreemSubscriptionEventRequest) (*pb.CreemSubscriptionEvent, error) {
	f.accepts++
	var e struct{ ID string }
	if jsonObject(r.SignedPayload, &e) != nil {
		return nil, status.Error(codes.InvalidArgument, "evidence")
	}
	h := sha256.Sum256(r.SignedPayload)
	v := &pb.CreemSubscriptionEvent{OrderId: r.OrderId, EventId: e.ID, PayloadSha256: h[:], Kind: "checkout", State: "pending"}
	f.receipts[e.ID] = v
	if f.lostACK {
		f.lostACK = false
		return nil, status.Error(codes.DeadlineExceeded, "committed; ACK lost")
	}
	return v, nil
}
func TestCreemSubscriptionQueueCheckoutRecoveryAndCatalogChangesPostgres(t *testing.T) {
	dsn := os.Getenv("LMM_CREEM_GO_TEST_DATABASE_URL")
	if dsn == "" {
		t.Skip("requires exclusive lmm_creem_go_test")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var own bool
	if err = db.QueryRow(`SELECT current_database()='lmm_creem_go_test'`).Scan(&own); err != nil || !own {
		t.Fatal("exclusive database required", err)
	}
	var exists bool
	if err = db.QueryRow(`SELECT to_regclass('public.payment_creem_subscription_inbox') IS NOT NULL`).Scan(&exists); err != nil {
		t.Fatal(err)
	}
	if !exists {
		if _, err = db.Exec(SubscriptionCreemSchema); err != nil {
			t.Fatal(err)
		}
	}
	for _, lostCheckout := range []bool{false, true} {
		t.Run(map[bool]string{false: "saved_checkout", true: "unknown_checkout"}[lostCheckout], func(t *testing.T) {
			a := creemFixture(t)
			o := creemSubscriptionOrder("USD")
			o.Order.OrderId = "sub_" + eventHash([]byte(time.Now().String()))
			core := &creemQueueCore{order: o, receipts: map[string]*pb.CreemSubscriptionEvent{}, lostACK: true}
			posts, products := 0, 0
			price := 1000
			a.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
				if r.URL.Path == "/v1/products" {
					products++
					return response(creemProduct("USD", price)), nil
				}
				if r.Method == http.MethodPost {
					posts++
					if r.URL.Path != "/v1/checkouts" {
						t.Fatal("wrong dispatch")
					}
					var body map[string]any
					if json.NewDecoder(r.Body).Decode(&body) != nil || body["request_id"] != o.Order.OrderId || body["product_id"] != "prod_recurring" || body["units"] != float64(1) {
						t.Fatal("native checkout lost frozen fields", body)
					}
					if _, ok := body["amount"]; ok {
						t.Fatal("arbitrary price override")
					}
					if lostCheckout {
						return nil, ErrUnavailable
					}
					return response(creemCheckoutResponse(o)), nil
				}
				if r.Method != http.MethodGet || r.URL.Query().Get("checkout_id") != "ch_original" {
					t.Fatal("invented recovery query", r.URL)
				}
				return response(creemCheckoutResponse(o)), nil
			})}
			service, err := NewCreemSubscriptions(core, db, []*Creem{a}, map[int64]bool{2: true})
			if err != nil {
				t.Fatal(err)
			}
			if _, err = service.Confirm(context.Background(), "other", o.Order.OrderId); err == nil || posts != 0 {
				t.Fatal("cross-user dispatched", err)
			}
			view, err := service.Confirm(context.Background(), "owner", o.Order.OrderId)
			if lostCheckout {
				if !errors.Is(err, ErrUnavailable) {
					t.Fatal(err)
				}
			} else if err != nil || view.Checkout == nil {
				t.Fatal(view, err)
			}
			if posts != 1 || products != 1 {
				t.Fatal("first confirm repeated product fetch or checkout", products, posts)
			}
			price = 999 // Current product drift cannot invalidate an already frozen dispatch.
			restarted, err := NewCreemSubscriptions(core, db, []*Creem{a}, map[int64]bool{})
			if err != nil {
				t.Fatal(err)
			}
			view, err = restarted.Confirm(context.Background(), "owner", o.Order.OrderId)
			if lostCheckout {
				if !errors.Is(err, ErrUnavailable) {
					t.Fatal(err)
				}
			} else if err != nil || view.Checkout == nil {
				t.Fatal("saved checkout stranded by price change", err)
			}
			if posts != 1 || products != 1 {
				t.Fatal("retry consulted current product or reposted", products, posts)
			}
			raw, _ := json.Marshal(map[string]any{"id": "evt_checkout_recovery", "eventType": "checkout.completed", "object": map[string]any{"id": "ch_original", "subscription": "sub_original", "metadata": creemMetadata(o)}})
			if _, err = restarted.Accept(context.Background(), 2, gatewayCallback(string(raw), "Creem-Signature")); status.Code(err) != codes.DeadlineExceeded {
				t.Fatal("lost Core ACK must stay pending", err)
			}
			if _, err = db.Exec(`UPDATE payment_creem_subscription_inbox SET next_attempt=clock_timestamp() WHERE order_id=$1`, o.Order.OrderId); err != nil {
				t.Fatal(err)
			}
			if err = restarted.Recover(context.Background(), 100); err != nil {
				t.Fatal(err)
			}
			view, err = restarted.Read(context.Background(), "owner", o.Order.OrderId)
			if err != nil || view.Checkout == nil || view.Checkout.ID != "ch_original" || posts != 1 || products != 1 || core.accepts != 1 {
				t.Fatal("original receipt/checkout recovery failed", view, err, products, posts, core.accepts)
			}
			var accepted int
			if err = db.QueryRow(`SELECT count(*) FROM payment_creem_subscription_inbox WHERE order_id=$1 AND state='accepted'`, o.Order.OrderId).Scan(&accepted); err != nil || accepted != 1 {
				t.Fatal(accepted, err)
			}
		})
	}
}
