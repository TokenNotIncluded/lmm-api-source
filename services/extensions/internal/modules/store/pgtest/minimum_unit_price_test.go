package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcprelay"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// The ordinary administrator can review products. Only the root may write site prices.
type priceAuthority struct{ legacyAuthority }

func (a priceAuthority) Check(ctx context.Context, token string, account store.Account, p store.Permission) (int64, error) {
	if token == "root" && account == (store.Account{Kind: "personal", ID: 9}) {
		return 9, nil
	}
	return a.legacyAuthority.Check(ctx, token, account, p)
}
func (a priceAuthority) CheckAdmin(ctx context.Context, token string, root bool) (int64, error) {
	if token == "root" {
		return 9, nil
	}
	if root {
		return 0, store.ErrForbidden
	}
	return a.legacyAuthority.CheckAdmin(ctx, token, root)
}

func TestPostgresConfigurableMinimumUnitPrice(t *testing.T) {
	db := legacyDatabase(t)
	repo, err := store.NewPostgres(db)
	must(t, err)
	ctx := context.Background()
	ledger := &recoveryPort{receipts: map[string]store.Receipt{}, checkDelivery: func(c store.MoneyCommand) {
		if c.Total.Amount != 50 || c.Total.Unit != "credits" {
			t.Fatalf("charge did not preserve original total: %+v", c.Total)
		}
	}}
	svc, err := store.New(repo, priceAuthority{}, ledger)
	must(t, err)
	call := func(token, action, id string, payload any) (any, error) {
		t.Helper()
		var body json.RawMessage
		if payload != nil {
			var err error
			body, err = json.Marshal(payload)
			must(t, err)
		}
		account := store.Account{Kind: "personal", ID: map[string]int64{"seller": 1, "buyer": 2, "admin": 4, "root": 9}[token]}
		return svc.Legacy(ctx, token, account, store.LegacyRequest{Action: action, ID: id, Body: body})
	}
	require := func(token, action, id string, payload any) map[string]any {
		t.Helper()
		v, e := call(token, action, id, payload)
		if e != nil {
			t.Fatalf("%s %s: %v", token, action, e)
		}
		if v == nil {
			return nil
		}
		return v.(map[string]any)
	}
	reject := func(token, action, id string, payload any, want error) {
		t.Helper()
		_, e := call(token, action, id, payload)
		if !errors.Is(e, want) {
			t.Fatalf("%s %s: got %v, want %v", token, action, e, want)
		}
	}
	policy := func(floor int64) map[string]any {
		return map[string]any{"minimum_unit_price_quota": floor, "fee_bps": 0, "promotion_quota": 0}
	}
	assertFloor := func(want int64) {
		t.Helper()
		config, e := svc.PublicConfig(ctx)
		must(t, e)
		if config["minimum_unit_price_quota"] != want || config["credits_per_usd"] != int64(500000) {
			t.Fatalf("advertised minimum %v, currency conversion %v", config["minimum_unit_price_quota"], config["credits_per_usd"])
		}
	}
	product := func(price int64) map[string]any {
		return map[string]any{"title": "Price boundary", "description": "Synthetic listing", "price_quota": price, "template": "fixed-content", "fixed_content": "Synthetic delivery", "payment_methods": []string{"balance"}, "pickup_login_required": true}
	}
	assertFloor(500000)
	reject("seller", "set_minimum_price", "", policy(0), store.ErrForbidden)
	reject("admin", "set_minimum_price", "", policy(0), store.ErrForbidden)
	reject("root", "set_minimum_price", "", policy(-1), store.ErrInvalid)
	reject("root", "set_minimum_price", "", policy(9007199254740992), store.ErrInvalid)
	reject("seller", "save_product", "", product(499999), store.ErrInvalid)
	require("root", "set_minimum_price", "", policy(0))
	// Reading through a newly constructed service exercises durable state.
	svc, err = store.New(repo, priceAuthority{}, ledger)
	must(t, err)
	assertFloor(0)
	reject("seller", "save_product", "", product(0), store.ErrInvalid)
	require("seller", "save_product", "", product(1))
	terms := require("seller", "save_terms", "", map[string]any{"content": "Synthetic price terms", "expected_version": ""})
	id := require("seller", "save_product", "", product(25))["id"].(string)
	require("root", "set_minimum_price", "", policy(26))
	reject("seller", "submit_product", id, nil, store.ErrInvalid)
	reject("seller", "create_variant", id, map[string]any{"name": "Under floor", "price_quota": 25, "enabled": true, "template": "fixed-content", "fixed_content": "synthetic"}, store.ErrInvalid)
	require("root", "set_minimum_price", "", policy(25))
	require("seller", "submit_product", id, nil)
	require("admin", "review_product", id, map[string]any{"approve": true})
	require("buyer", "accept_disclaimer", "", map[string]any{"version": "merchant-store-v1", "accepted": true})
	order := map[string]any{"product_id": id, "quantity": 2, "payment_method": "balance", "request_key": "synthetic-floor-order", "disclaimer_version": "merchant-store-v1", "seller_terms_version": terms["version"], "accept_seller_terms": true}
	first := require("buyer", "create_order", "", order)["order"].(map[string]any)
	if first["price_quota"] != int64(50) {
		t.Fatalf("unexpected original total: %v", first["price_quota"])
	}
	row, e := repo.FindProduct(ctx, id)
	must(t, e)
	// Apply the new root policy immediately before the checkout transaction begins.
	// An earlier availability check still sees an eligible product.
	window := &priceWindowRepository{Postgres: repo, shop: row.ShopID, reprice: func() { require("root", "set_minimum_price", "", policy(26)) }}
	svc, err = store.New(window, priceAuthority{}, ledger)
	must(t, err)
	order["request_key"] = "synthetic-floor-new"
	reject("buyer", "create_order", "", order, store.ErrInvalid)
	assertFloor(26)
	pub, e := svc.PublicProduct(ctx, id)
	must(t, e)
	if pub["trading_paused"] != true || pub["price_quota"] != int64(25) {
		t.Fatalf("raised floor did not pause existing price: %#v", pub)
	}
	for stock, n := range map[string]int{"in_stock": 0, "out_of_stock": 1} {
		rows, e := svc.PublicCatalogueQuery(ctx, store.CatalogueQuery{Stock: stock, Limit: 24})
		must(t, e)
		if len(rows) != n {
			t.Fatalf("%s returned %d rows, want %d", stock, len(rows), n)
		}
	}
	order["request_key"] = "synthetic-floor-order"
	replay := require("buyer", "create_order", "", order)
	original := replay["order"].(map[string]any)
	if replay["created"] != false || original["id"] != first["id"] || original["price_quota"] != int64(50) {
		t.Fatal("price change altered the existing order")
	}
	paid := require("buyer", "pay_order", first["id"].(string), map[string]any{})
	if paid["status"] != "paid" || ledger.payEffects != 1 || ledger.captureEffects != 1 {
		t.Fatal("original order failed to pay and deliver exactly once")
	}

	require("root", "set_minimum_price", "", policy(25))
	pub, e = svc.PublicProduct(ctx, id)
	must(t, e)
	if pub["trading_paused"] != false {
		t.Fatal("lowering floor did not restore unchanged listing")
	}
}

// Coordinate two actual PostgreSQL sessions; observe the conflicting advisory
// lock before releasing the checkout transaction, without assuming scheduling.
func TestPostgresMinimumUnitPriceSerializesConcurrentConfig(t *testing.T) {
	db := legacyDatabase(t)
	repo, err := store.NewPostgres(db)
	must(t, err)
	ctx := context.Background()
	svc, err := store.New(repo, priceAuthority{}, &funds{seen: map[string]store.Receipt{}})
	must(t, err)
	done := make(chan error, 1)
	err = repo.Within(ctx, "synthetic-price-reader", func(tx store.Tx) error {
		initial, e := tx.MinimumUnitPrice()
		if e != nil {
			return e
		}
		if initial != 500000 {
			return fmt.Errorf("unexpected initial floor %d", initial)
		}
		go func() {
			_, e := svc.Legacy(ctx, "root", store.Account{Kind: "personal", ID: 9}, store.LegacyRequest{Action: "set_minimum_price", Body: json.RawMessage(`{"minimum_unit_price_quota":26}`)})
			done <- e
		}()
		deadline := time.Now().Add(2 * time.Second)
		for {
			var blocked bool
			e = db.QueryRowContext(ctx, `SELECT EXISTS(SELECT FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database()))`).Scan(&blocked)
			if e != nil {
				return e
			}
			if blocked {
				break
			}
			if time.Now().After(deadline) {
				return errors.New("configuration writer did not wait for checkout")
			}
			time.Sleep(5 * time.Millisecond)
		}
		current, e := tx.MinimumUnitPrice()
		if e != nil {
			return e
		}
		if current != initial {
			return fmt.Errorf("checkout price changed mid-transaction: %d", current)
		}
		return nil
	})
	must(t, err)
	must(t, <-done)
	cfg, err := svc.PublicConfig(ctx)
	must(t, err)
	if cfg["minimum_unit_price_quota"] != int64(26) {
		t.Fatal("new checkout did not see committed configuration")
	}
}

func TestPostgresRootCanSetIndependentRelayAndListingPrices(t *testing.T) {
	db := legacyDatabase(t)
	repo, e := store.NewPostgres(db)
	must(t, e)
	svc, e := store.New(repo, priceAuthority{}, &funds{seen: map[string]store.Receipt{}})
	must(t, e)
	ctx := context.Background()
	write := func(token string, id int64, payload string) error {
		_, e := svc.Legacy(ctx, token, store.Account{Kind: "personal", ID: id}, store.LegacyRequest{Action: "set_minimum_price", Body: json.RawMessage(payload)})
		return e
	}
	check := func(floor, price int64) {
		t.Helper()
		cfg, e := svc.PublicConfig(ctx)
		must(t, e)
		if cfg["minimum_unit_price_quota"] != floor || cfg["relay_price_quota_per_gib"] != price {
			t.Fatalf("wrong prices: %v %v", cfg["minimum_unit_price_quota"], cfg["relay_price_quota_per_gib"])
		}
		must(t, repo.Within(ctx, "synthetic-new-order", func(tx store.Tx) error {
			v, e := tx.RelayPricePerGiB()
			if e != nil {
				return e
			}
			if v != price {
				t.Fatalf("checkout price %d, expected %d", v, price)
			}
			return nil
		}))
	}
	check(500000, 500000)
	for _, tc := range []struct {
		user string
		id   int64
	}{{"seller", 1}, {"admin", 4}} {
		if e := write(tc.user, tc.id, `{"relay_price_quota_per_gib":25}`); !errors.Is(e, store.ErrForbidden) {
			t.Fatal("unauthorized price update", e)
		}
	}
	for _, bad := range []string{`{"minimum_unit_price_quota":0,"relay_price_quota_per_gib":0}`, `{"relay_price_quota_per_gib":-1}`, `{"relay_price_quota_per_gib":9007199254740992}`, `{"relay_price_quota_per_gib":1.5}`, `{"relay_price_quota_per_gib":null}`} {
		if e := write("root", 9, bad); !errors.Is(e, store.ErrInvalid) {
			t.Fatal(bad, e)
		}
	}
	check(500000, 500000)
	must(t, write("root", 9, `{"minimum_unit_price_quota":0,"relay_price_quota_per_gib":750001}`))
	check(0, 750001)
	must(t, write("root", 9, `{"minimum_unit_price_quota":10}`))
	check(10, 750001)
	must(t, write("root", 9, `{"relay_price_quota_per_gib":1}`))
	svc, e = store.New(repo, priceAuthority{}, &funds{seen: map[string]store.Receipt{}})
	must(t, e)
	check(10, 1)
}

type relayAccountingFixture struct {
	grants map[string]ssmcprelay.Entitlement
}

func (*relayAccountingFixture) Quota(context.Context, int64) (ssmcprelay.Quota, error) {
	now := time.Now().UTC()
	return ssmcprelay.Quota{Period: now.Format("2006-01"), ResetAt: now.Add(time.Hour).Unix()}, nil
}
func (f *relayAccountingFixture) Grant(_ context.Context, e ssmcprelay.Entitlement) error {
	if old, ok := f.grants[e.OrderID]; ok && old != e {
		return store.ErrConflict
	}
	f.grants[e.OrderID] = e
	return nil
}
func TestPostgresRelayChargeAndGrantUseFrozenQuoteAfterRestart(t *testing.T) {
	db := legacyDatabase(t)
	repo, e := store.NewPostgres(db)
	must(t, e)
	ctx := context.Background()
	ledger := &recoveryPort{receipts: map[string]store.Receipt{}, losePay: true, checkDelivery: func(c store.MoneyCommand) {
		if c.Total != (store.Price{Amount: 2250003, Unit: "credits"}) || c.OrderSnapshot.UnitPrice != (store.Price{Amount: 750001, Unit: "credits"}) || c.OrderSnapshot.Quantity != 3 {
			t.Fatal("capture changed frozen quote")
		}
	}}
	policy, e := store.New(repo, priceAuthority{}, ledger)
	must(t, e)
	price := func(n int64) {
		t.Helper()
		_, e := policy.Legacy(ctx, "root", store.Account{Kind: "personal", ID: 9}, store.LegacyRequest{Action: "set_minimum_price", Body: json.RawMessage(fmt.Sprintf(`{"relay_price_quota_per_gib":%d}`, n))})
		must(t, e)
	}
	relay := &relayAccountingFixture{grants: map[string]ssmcprelay.Entitlement{}}
	seller := store.Account{Kind: "team", ID: 7}
	svc, e := ssmcprelay.New(relay, repo, ledger, seller)
	must(t, e)
	price(750001)
	order, e := svc.Preview(ctx, 2, "pg-frozen-quote", 3)
	must(t, e)
	if order.USDCents != 0 || order.TotalPriceQuota != 2250003 || order.Bytes != "3221225472" {
		t.Fatal(order)
	}
	if _, e = svc.Confirm(ctx, 2, "synthetic-user", order.ID); !errors.Is(e, store.ErrPending) {
		t.Fatal("missing receipt must stay pending", e)
	}
	price(1)
	svc, e = ssmcprelay.New(relay, repo, ledger, seller)
	must(t, e)
	replay, e := svc.Preview(ctx, 2, "pg-frozen-quote", 3)
	must(t, e)
	if replay.TotalPriceQuota != 2250003 || replay.State != "payment_pending" || replay.ID != order.ID {
		t.Fatal(replay)
	}
	for i := 0; i < 2; i++ {
		paid, e := svc.Confirm(ctx, 2, "synthetic-user", order.ID)
		must(t, e)
		if paid.State != "completed" {
			t.Fatal(paid)
		}
	}
	if ledger.payEffects != 1 || ledger.captureEffects != 1 || len(relay.grants) != 1 || relay.grants[order.ID].Bytes != "3221225472" {
		t.Fatal("unexpected grant/debit effects")
	}
	newOrder, e := svc.Preview(ctx, 2, "pg-new-quote", 3)
	must(t, e)
	if newOrder.TotalPriceQuota != 3 || newOrder.Bytes != order.Bytes {
		t.Fatal(newOrder)
	}
}
