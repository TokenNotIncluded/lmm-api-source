// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

// Synthetic persistence/authority cover domain transitions, separately from
// the normal HTTP/Core/PostgreSQL acceptance fixture.
type stockCatalogueIndex struct {
	*memoryRepo
	shop string
}

func (r *stockCatalogueIndex) FindProduct(ctx context.Context, id string) (LocatedProduct, error) {
	row := LocatedProduct{ShopID: r.shop, Owner: Account{"personal", 1}}
	err := r.Within(ctx, r.shop, func(tx Tx) error {
		if err := tx.Get("products", id, &row.Product); err != nil {
			return err
		}
		var err error
		row.Variants, err = variantsFor(tx, id)
		return err
	})
	return row, err
}
func (r *stockCatalogueIndex) FindOrder(context.Context, string) (LocatedOrder, error) {
	return LocatedOrder{}, ErrNotFound
}
func (r *stockCatalogueIndex) ListPublished(context.Context, int64, string, int, int) ([]LocatedProduct, error) {
	return nil, ErrUnavailable
}

func TestLegacyStockCatalogueInventoryAndReplay(t *testing.T) {
	ctx := context.Background()
	actor := Account{"personal", 1}
	repo := &stockCatalogueIndex{memoryRepo: newRepo(), shop: ID("legacy-web-shop-v1", actor)}
	s, err := New(repo, &authority{}, nil)
	must(t, err)
	must(t, s.WithStockKey(make([]byte, 32)))
	body := func(v any) json.RawMessage { b, e := json.Marshal(v); must(t, e); return b }
	call := func(action, id, child string, in any) (any, error) {
		return s.Legacy(ctx, sellerToken, actor, LegacyRequest{Action: action, ID: id, ChildID: child, Body: body(in), Limit: 20})
	}
	input := LegacyProductInput{Title: "Synthetic inventory", PriceQuota: 500000, Template: "card-key", PaymentMethods: []string{"balance"}, PickupLoginRequired: true}
	value, err := call("save_product", "", "", input)
	must(t, err)
	product := value.(map[string]any)
	id := product["id"].(string)
	variant := product["default_variant_id"].(string)
	row, err := repo.FindProduct(ctx, id)
	must(t, err)
	if !row.Variants[0].TrackStock || row.Variants[0].Stock != 0 || row.Variants[0].Legacy.FixedContent != "" {
		t.Fatal("finite stock constructor changed")
	}
	in := map[string]any{"items": []string{"SYNTHETIC-KEY", "SYNTHETIC-KEY"}, "request_key": "same-batch"}
	_, err = call("stock_add", id, "", in)
	must(t, err)
	_, err = call("stock_add", id, variant, in)
	must(t, err)
	in["items"] = []string{"OTHER-KEY"}
	_, err = call("stock_add", id, variant, in)
	wantError(t, err, ErrConflict)
	value, err = call("stock_list", id, variant, nil)
	must(t, err)
	items := value.(map[string]any)["items"].([]map[string]any)
	if len(items) != 2 {
		t.Fatal("replay added stock or deduplicated legitimate contents")
	}
	if encoded := string(body(items)); strings.Contains(encoded, "SYNTHETIC-KEY") || strings.Contains(encoded, "ciphertext") {
		t.Fatal("inventory leaked private content")
	}
	_, err = s.Legacy(ctx, otherToken, Account{"personal", 3}, LegacyRequest{Action: "stock_list", ID: id, Limit: 20})
	wantError(t, err, ErrForbidden)
	_, err = call("save_variant", id, variant, LegacyVariantInput{Name: "Changed", PriceQuota: 500000, Template: "fixed-content", Enabled: true, FixedContent: new(string)})
	wantError(t, err, ErrInvalid)
	_, err = s.Legacy(ctx, sellerToken, actor, LegacyRequest{Action: "stock_remove", ID: id, StockID: items[0]["id"].(string)})
	must(t, err)
	row, err = repo.FindProduct(ctx, id)
	must(t, err)
	if row.Variants[0].Stock != 1 || row.Variants[0].StockSequence != 2 {
		t.Fatal("inventory accounting changed")
	}
}
func TestLegacyStockTemplatesAndMissingKeyBeforeMoney(t *testing.T) {
	for _, template := range []string{"card-key", "text", "custom-text", "redemption-code", "license-key", "download-link", "account-details"} {
		in := LegacyVariantInput{Name: "Synthetic", PriceQuota: 500000, Template: template, Enabled: true}
		must(t, validateLegacyVariant(in))
		v := newLegacyVariant("variant", "product", in, 1)
		if !v.TrackStock || v.Quota != -1 || v.Legacy.FixedContent != "" {
			t.Fatal(template)
		}
	}
	f := a06StateSetup(t)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.Legacy.DeliveryTemplate = "card-key"
		o.StockItems = []string{"synthetic-stock"}
		return tx.Put("orders", f.id, o)
	}))
	_, err := f.change(buyerToken, "pay", "")
	wantError(t, err, ErrUnavailable)
	if f.funds.execCalls.Load() != 0 || f.order(t).State != AwaitingPayment {
		t.Fatal("missing key changed funds/state")
	}
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.State = Paid
		return tx.Put("orders", f.id, o)
	}))
	_, err = f.change(sellerToken, "fulfill", "synthetic")
	wantError(t, err, ErrUnavailable)
	_, err = f.s.FinalizeDelivery(context.Background(), f.shop, f.id)
	wantError(t, err, ErrUnavailable)
	if f.funds.captureCalls.Load() != 0 || f.order(t).Delivered {
		t.Fatal("missing key captured/delivered")
	}
}
