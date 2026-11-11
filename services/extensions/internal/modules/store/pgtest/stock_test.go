// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// Real PostgreSQL shop transactions, with the existing synthetic authority and
// funds port. Real Core money and Session HTTP are verified separately.
func TestPostgresLegacyStockFiniteInventory(t *testing.T) {
	db := legacyDatabase(t)
	repo, err := store.NewPostgres(db)
	must(t, err)
	s, err := store.New(repo, legacyAuthority{}, &funds{seen: map[string]store.Receipt{}})
	must(t, err)
	key := make([]byte, 32)
	must(t, s.WithStockKey(key))
	ctx := context.Background()
	body := func(v any) json.RawMessage { b, e := json.Marshal(v); must(t, e); return b }
	call := func(token string, r store.LegacyRequest) (any, error) {
		return s.Legacy(ctx, token, store.Account{Kind: "personal", ID: map[string]int64{"seller": 1, "buyer": 2, "outsider": 3, "admin": 4}[token]}, r)
	}
	invoke := func(token string, r store.LegacyRequest) map[string]any {
		t.Helper()
		v, e := call(token, r)
		must(t, e)
		if v == nil {
			return nil
		}
		return v.(map[string]any)
	}
	terms := invoke("seller", store.LegacyRequest{Action: "save_terms", Body: body(map[string]any{"content": "Synthetic stock terms", "expected_version": ""})})
	p := invoke("seller", store.LegacyRequest{Action: "save_product", Body: body(map[string]any{"title": "Finite codes", "price_quota": 500000, "template": "card-key", "payment_methods": []string{"balance"}, "pickup_login_required": true})})
	id := p["id"].(string)
	variant := p["default_variant_id"].(string)
	in := body(map[string]any{"items": []string{"SYNTHETIC-CODE", "SYNTHETIC-CODE"}, "request_key": "import-one"})
	for i := 0; i < 2; i++ {
		invoke("seller", store.LegacyRequest{Action: "stock_add", ID: id, Body: in})
	}
	_, err = call("seller", store.LegacyRequest{Action: "stock_add", ID: id, Body: body(map[string]any{"items": []string{"different"}, "request_key": "import-one"})})
	if !errors.Is(err, store.ErrConflict) {
		t.Fatal("conflicting import accepted", err)
	}
	list := invoke("seller", store.LegacyRequest{Action: "stock_list", ID: id, Limit: 20})
	if len(list["items"].([]map[string]any)) != 2 {
		t.Fatal("import replay changed stock")
	}
	raw := string(body(list))
	if strings.Contains(raw, "SYNTHETIC-CODE") || strings.Contains(raw, "ciphertext") {
		t.Fatal("private inventory leaked")
	}
	invoke("seller", store.LegacyRequest{Action: "submit_product", ID: id})
	invoke("admin", store.LegacyRequest{Action: "review_product", ID: id, Body: body(map[string]any{"approve": true})})
	invoke("buyer", store.LegacyRequest{Action: "accept_disclaimer", Body: body(map[string]any{"version": "merchant-store-v1", "accepted": true})})
	makeOrder := func(key string) store.LegacyRequest {
		return store.LegacyRequest{Action: "create_order", Body: body(map[string]any{"product_id": id, "quantity": 1, "payment_method": "balance", "request_key": key, "disclaimer_version": "merchant-store-v1", "seller_terms_version": terms["version"], "accept_seller_terms": true})}
	}
	noKey, err := store.New(repo, legacyAuthority{}, &funds{seen: map[string]store.Receipt{}})
	must(t, err)
	_, err = noKey.Legacy(ctx, "buyer", store.Account{Kind: "personal", ID: 2}, makeOrder("no-key"))
	if !errors.Is(err, store.ErrUnavailable) {
		t.Fatal("missing key reserved stock", err)
	}
	pub, err := noKey.PublicProduct(ctx, id)
	must(t, err)
	if pub["trading_paused"] != true {
		t.Fatal("missing key advertised purchases")
	}
	var wg sync.WaitGroup
	results := make(chan error, 6)
	for i := 0; i < 6; i++ {
		r := makeOrder(fmt.Sprintf("order-%d", i))
		wg.Add(1)
		go func() { defer wg.Done(); _, e := call("buyer", r); results <- e }()
	}
	wg.Wait()
	close(results)
	successes := 0
	for e := range results {
		if e == nil {
			successes++
		} else if !errors.Is(e, store.ErrSoldOut) {
			t.Fatal("unexpected checkout rejection", e)
		}
	}
	if successes != 2 {
		t.Fatalf("finite inventory issued %d orders", successes)
	}
	var count, distinct int
	must(t, db.QueryRow(`SELECT count(*),count(DISTINCT body->>'order_id') FROM lmm_store.stock_items WHERE body->>'variant_id'=$1 AND body->>'state'='reserved'`, variant).Scan(&count, &distinct))
	if count != 2 || distinct != 2 {
		t.Fatalf("stock assigned more than once count=%d orders=%d", count, distinct)
	}
	_, err = call("outsider", store.LegacyRequest{Action: "stock_list", ID: id, Limit: 20})
	if !errors.Is(err, store.ErrForbidden) {
		t.Fatal("foreign seller inventory read", err)
	}
}
