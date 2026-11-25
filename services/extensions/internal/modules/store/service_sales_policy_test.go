package store

import (
	"context"
	"errors"
	"testing"
)

func TestMonthlyServiceCapacityUsesOriginalBytesAndShopLock(t *testing.T) {
	ctx := context.Background()
	f := setup(t, 20, 10, -1, false, true)
	f.s.funds = &a06StateFunds{}
	price := Price{Unit: "credits", Amount: 23}
	f.catalog(t, CatalogChange{Action: "create_variant", ProductID: f.product.ID, Title: "Relay", Price: price, Quota: -1, Key: "relay-sku"}, &f.variant)
	terms := ServiceTerms{Service: "ssmcpd.monthly-traffic.v1", Audience: ID("local"), Period: "2026-11", UnitsPerItem: 4, UnitPrice: price, NonRefundable: true}
	buyer := Account{Kind: "personal", ID: 2}
	buy := func(key string, q int64, terms ServiceTerms) (Order, error) {
		return f.s.PurchaseServiceEntitlement(ctx, buyerToken, Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: buyer, Quantity: q, Key: key}, terms)
	}
	first, err := buy("before-limit", 1, terms)
	must(t, err)
	cap := int64(8)
	_, err = f.s.ServiceSalesPolicy(ctx, f.shop.ID, f.variant.ID, terms.Service, &ServiceSalesPolicy{MonthlyLimitBytes: &cap, UnitCredits: 26})
	must(t, err)
	policy, err := f.s.ServiceSalesPolicy(ctx, f.shop.ID, f.variant.ID, terms.Service, nil)
	if err != nil || policy.MonthlyLimitBytes == nil || *policy.MonthlyLimitBytes != cap || policy.UnitCredits != 26 {
		t.Fatalf("read policy %#v: %v", policy, err)
	}
	if _, err = buy("stale-price", 1, terms); !errors.Is(err, ErrConflict) {
		t.Fatalf("stale price: %v", err)
	}
	original, err := buy("before-limit", 1, terms)
	if err != nil || original.ID != first.ID {
		t.Fatalf("original order: %v", err)
	}
	terms.UnitPrice.Amount = 26
	type result struct {
		o   Order
		err error
	}
	ch := make(chan result, 2)
	for _, key := range []string{"left", "right"} {
		go func(key string) { o, e := buy(key, 1, terms); ch <- result{o, e} }(key)
	}
	a, b := <-ch, <-ch
	if a.err != nil {
		a, b = b, a
	}
	if a.err != nil || !errors.Is(b.err, ErrSoldOut) {
		t.Fatalf("concurrent purchases: %v, %v", a.err, b.err)
	}
	_, err = f.change(buyerToken, a.o, "cancel", "")
	must(t, err)
	paid, err := buy("released", 1, terms)
	must(t, err)
	_, err = f.change(buyerToken, paid, "pay", "")
	must(t, err)
	if _, err = f.change(buyerToken, paid, "request_refund", "unused"); !errors.Is(err, ErrConflict) {
		t.Fatalf("refund requested: %v", err)
	}
	if _, err = f.change(sellerToken, paid, "refund", ""); !errors.Is(err, ErrConflict) {
		t.Fatalf("refund executed: %v", err)
	}
	terms.Period = "2026-12"
	_, err = buy("new-month", 2, terms)
	must(t, err)
	cap = 0
	_, err = f.s.ServiceSalesPolicy(ctx, f.shop.ID, f.variant.ID, terms.Service, &ServiceSalesPolicy{MonthlyLimitBytes: &cap, UnitCredits: 26})
	must(t, err)
	terms.Period = "2027-01"
	if _, err = buy("paused", 1, terms); !errors.Is(err, ErrSoldOut) {
		t.Fatalf("zero limit: %v", err)
	}
}
