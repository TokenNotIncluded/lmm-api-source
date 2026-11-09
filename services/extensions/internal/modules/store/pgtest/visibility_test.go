package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func TestPostgresProductVisibilityBeforePagination(t *testing.T) {
	db := legacyDatabase(t)
	repo, err := store.NewPostgres(db)
	must(t, err)
	s, err := store.New(repo, legacyAuthority{}, nil)
	must(t, err)
	ctx := context.Background()
	actors := map[string]int64{"seller": 1, "buyer": 2, "admin": 4}
	call := func(token, action, id string, body any) map[string]any {
		t.Helper()
		raw, err := json.Marshal(body)
		must(t, err)
		if body == nil {
			raw = nil
		}
		result, err := s.Legacy(ctx, token, store.Account{Kind: "personal", ID: actors[token]}, store.LegacyRequest{Action: action, ID: id, Body: raw})
		must(t, err)
		if result == nil {
			return nil
		}
		return result.(map[string]any)
	}
	call("seller", "save_terms", "", map[string]any{"content": "Visibility fixture terms", "expected_version": ""})
	ids := map[string]string{}
	for _, visibility := range []string{"private", "registered", "public"} {
		body := map[string]any{"title": "Visibility " + visibility, "visibility": visibility, "price_quota": 500000, "template": "fixed-content", "fixed_content": "Private fulfillment", "payment_methods": []string{"balance"}, "pickup_login_required": true}
		product := call("seller", "save_product", "", body)
		id := product["id"].(string)
		ids[visibility] = id
		// An ordinary edit that omits visibility preserves the restriction.
		delete(body, "visibility")
		product = call("seller", "save_product", id, body)
		if product["visibility"] != visibility {
			t.Fatalf("edit downgraded %s: %v", visibility, product["visibility"])
		}
		if visibility != "private" {
			call("seller", "submit_product", id, nil)
			call("admin", "review_product", id, map[string]any{"approve": true, "note": "Private review note"})
		}
	}
	for _, tc := range []struct {
		token string
		user  int64
		want  int
	}{{"", 0, 1}, {"buyer", 2, 2}, {"seller", 1, 3}} {
		items, err := s.CatalogueForViewer(ctx, tc.token, tc.user, store.CatalogueQuery{Limit: 10, Sort: "newest"})
		must(t, err)
		if len(items) != tc.want {
			t.Fatalf("viewer %d got %d products, want %d", tc.user, len(items), tc.want)
		}
		for offset := 0; offset <= tc.want; offset++ {
			page, err := s.CatalogueForViewer(ctx, tc.token, tc.user, store.CatalogueQuery{Offset: offset, Limit: 1, Sort: "newest"})
			must(t, err)
			if offset == tc.want {
				if len(page) != 0 {
					t.Fatal("inaccessible products affected page end")
				}
			} else if len(page) != 1 || page[0]["id"] != items[offset]["id"] || page[0]["review_note"] != "" {
				t.Fatalf("viewer %d page %d: %v", tc.user, offset, page)
			}
		}
	}
	for _, tc := range []struct {
		visibility, token string
		user              int64
		visible           bool
	}{{"registered", "", 0, false}, {"registered", "buyer", 2, true}, {"private", "buyer", 2, false}, {"private", "seller", 1, true}} {
		view, err := s.ProductForBuyer(ctx, tc.token, tc.user, ids[tc.visibility])
		if tc.visible {
			must(t, err)
			if view["visibility"] != tc.visibility || view["review_note"] != "" {
				t.Fatal(view)
			}
		} else if !errors.Is(err, store.ErrNotFound) {
			t.Fatalf("restricted detail %s viewer %d: %v", tc.visibility, tc.user, err)
		}
	}
	if _, err := s.CatalogueForViewer(ctx, "buyer", 1, store.CatalogueQuery{Limit: 10}); !errors.Is(err, store.ErrForbidden) {
		t.Fatalf("forged catalogue viewer: %v", err)
	}
	private, err := s.LocateProduct(ctx, ids["private"])
	must(t, err)
	if _, err := s.Purchase(ctx, "buyer", store.Purchase{ShopID: private.ShopID, VariantID: private.Variants[0].ID, Buyer: store.Account{Kind: "personal", ID: 2}, Quantity: 1, Key: "hidden-purchase"}); !errors.Is(err, store.ErrConflict) {
		t.Fatalf("hidden product was purchasable: %v", err)
	}
	// The native module's public endpoints obey the same audience restriction.
	products, err := s.Browse(ctx, private.ShopID, "", 100)
	must(t, err)
	variants, err := s.Variants(ctx, private.ShopID, "", 100)
	must(t, err)
	if len(products.Items) != 1 || products.Items[0].ID != ids["public"] || len(variants.Items) != 1 || variants.Items[0].ProductID != ids["public"] {
		t.Fatalf("native catalogue leaked restricted products: %v %v", products, variants)
	}
	// Historical test mode without canonical visibility stays owner-only.
	_, err = db.Exec(`UPDATE lmm_store.products SET body=jsonb_set(body #- '{legacy,visibility}','{legacy,test_mode}','true') WHERE id=$1`, ids["public"])
	must(t, err)
	if _, err := s.PublicProduct(ctx, ids["public"]); !errors.Is(err, store.ErrNotFound) {
		t.Fatalf("legacy test mode became public: %v", err)
	}
	rows, err := s.CatalogueForViewer(ctx, "buyer", 2, store.CatalogueQuery{Limit: 10})
	must(t, err)
	if len(rows) != 1 || rows[0]["id"] != ids["registered"] {
		t.Fatal("legacy test mode leaked in catalogue")
	}
}

func TestPostgresNativeVisiblePagination(t *testing.T) {
	db := legacyDatabase(t)
	repo, err := store.NewPostgres(db)
	must(t, err)
	s, err := store.New(repo, legacyAuthority{}, nil)
	must(t, err)
	ctx := context.Background()
	shop := "native-page"
	must(t, repo.Within(ctx, shop, func(tx store.Tx) error {
		if err := tx.Put("shops", shop, store.Shop{ID: shop, Owner: store.Account{Kind: "personal", ID: 1}, Title: "Pagination fixture"}); err != nil {
			return err
		}
		for _, id := range []string{"a-hidden", "b-visible", "c-hidden", "d-visible"} {
			visibility := "public"
			if id == "a-hidden" || id == "c-hidden" {
				visibility = "registered"
			}
			p := store.Product{ID: id, Title: id, Published: true, Legacy: &store.LegacyProductMetadata{Visibility: visibility, Status: "published", Approved: true}}
			if err := tx.Put("products", id, p); err != nil {
				return err
			}
			if err := tx.Put("variants", id, store.Variant{ID: id, ProductID: id, Title: id, Price: store.Price{Amount: 1, Currency: "USD"}, Quota: -1}); err != nil {
				return err
			}
		}
		return nil
	}))
	for _, tc := range []struct{ after, id, next string }{{"", "b-visible", "b-visible"}, {"b-visible", "d-visible", ""}} {
		products, err := s.Browse(ctx, shop, tc.after, 1)
		must(t, err)
		if len(products.Items) != 1 || products.Items[0].ID != tc.id || products.Next != tc.next {
			t.Fatalf("product page: %v", products)
		}
		variants, err := s.Variants(ctx, shop, tc.after, 1)
		must(t, err)
		if len(variants.Items) != 1 || variants.Items[0].ID != tc.id || variants.Next != tc.next {
			t.Fatalf("variant page: %v", variants)
		}
	}
}
