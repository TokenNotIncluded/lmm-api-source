package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func TestPostgresCategoriesAndBuyerLimits(t *testing.T) {
	db := legacyDatabase(t)
	ctx := context.Background()
	repo, err := store.NewPostgres(db)
	must(t, err)
	ledger := &funds{seen: map[string]store.Receipt{}}
	s, err := store.New(repo, legacyAuthority{}, ledger)
	must(t, err)
	body := func(value any) json.RawMessage {
		raw, err := json.Marshal(value)
		must(t, err)
		return raw
	}
	invoke := func(token string, request store.LegacyRequest) any {
		t.Helper()
		user := map[string]int64{"seller": 1, "buyer": 2, "admin": 4}[token]
		value, err := s.Legacy(ctx, token, store.Account{Kind: "personal", ID: user}, request)
		must(t, err)
		return value
	}
	category := invoke("admin", store.LegacyRequest{Action: "save_category", Body: body(map[string]any{"name": "Synthetic category", "sort_order": 2, "active": true})}).(store.Category)
	terms := invoke("seller", store.LegacyRequest{Action: "save_terms", Body: body(map[string]any{"content": "Synthetic category terms", "expected_version": ""})}).(map[string]any)
	input := map[string]any{"title": "Category product", "description": "Synthetic description", "image_urls": []string{}, "contact": "", "links": []any{}, "category_id": category.ID, "price_quota": int64(500001), "template": "fixed-content", "fixed_content": "Private synthetic material", "delivery_strategy": "sequential", "payment_methods": []string{"balance"}, "pickup_login_required": true, "max_quantity_per_order": 2, "max_quantity_per_buyer": 3}
	product := invoke("seller", store.LegacyRequest{Action: "save_product", Body: body(input)}).(map[string]any)
	id := product["id"].(string)
	invoke("seller", store.LegacyRequest{Action: "submit_product", ID: id})
	invoke("admin", store.LegacyRequest{Action: "review_product", ID: id, Body: body(map[string]any{"approve": true, "note": ""})})
	rows, err := s.PublicCatalogueQuery(ctx, store.CatalogueQuery{CategoryID: category.ID, Limit: 10})
	must(t, err)
	if len(rows) != 1 || rows[0]["category_id"] != category.ID {
		t.Fatalf("database category filtering failed: %+v", rows)
	}
	rows, err = s.PublicCatalogueQuery(ctx, store.CatalogueQuery{CategoryID: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", Limit: 10})
	must(t, err)
	if len(rows) != 0 {
		t.Fatal("unrelated category included a product")
	}
	row, err := s.LocateProduct(ctx, id)
	must(t, err)
	policy := &store.LegacyCheckoutPolicy{Version: terms["version"].(string), Accepted: true}
	var wg sync.WaitGroup
	var accepted atomic.Int64
	for i := range 12 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := s.Purchase(ctx, "buyer", store.Purchase{ShopID: row.ShopID, VariantID: row.Variants[0].ID, Buyer: store.Account{Kind: "personal", ID: 2}, Quantity: 1, Key: fmt.Sprintf("category-limit-%d", i), Policy: policy})
			if err == nil {
				accepted.Add(1)
			} else if !errors.Is(err, store.ErrPurchaseLimit) {
				t.Errorf("unexpected competing result: %v", err)
			}
		}()
	}
	wg.Wait()
	if accepted.Load() != 3 {
		t.Fatalf("PostgreSQL buyer limit accepted %d reservations", accepted.Load())
	}
	view, err := s.ProductForBuyer(ctx, "buyer", 2, id)
	must(t, err)
	if view["buyer_purchase_remaining"] != int64(0) {
		t.Fatalf("wrong authenticated remaining quantity: %+v", view)
	}
	public, err := s.ProductForBuyer(ctx, "", 0, id)
	must(t, err)
	if _, exists := public["buyer_purchase_remaining"]; exists {
		t.Fatal("anonymous catalogue leaked a buyer's remaining quantity")
	}
	invoke("admin", store.LegacyRequest{Action: "save_category", ID: category.ID, Body: body(map[string]any{"name": "Disabled synthetic", "sort_order": 2, "active": false})})
	categories, err := s.PublicCategories(ctx, 0, 10)
	must(t, err)
	if len(categories["items"].([]store.Category)) != 0 {
		t.Fatal("disabled category remained publicly selectable")
	}
	if _, err := s.PublicProduct(ctx, id); err != nil {
		t.Fatalf("disabling category removed product: %v", err)
	}
	if ledger.effects != 0 {
		t.Fatal("catalogue and reservation checks posted money")
	}
}
