// Real PostgreSQL storage and shop locks; session authority and paid-money port
// are explicitly synthetic. Free coupon delivery requires no financial port.
package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func TestPostgresPromotionsAndProtectedPickup(t *testing.T) {
	db := legacyDatabase(t)
	ctx := context.Background()
	repo, err := store.NewPostgres(db)
	must(t, err)
	ledger := &funds{seen: map[string]store.Receipt{}}
	service, err := store.New(repo, legacyAuthority{}, ledger)
	must(t, err)
	body := func(value any) json.RawMessage { raw, err := json.Marshal(value); must(t, err); return raw }
	call := func(token string, request store.LegacyRequest) (any, error) {
		return service.Legacy(ctx, token, store.Account{Kind: "personal", ID: map[string]int64{"seller": 1, "buyer": 2, "outsider": 3, "admin": 4}[token]}, request)
	}
	invoke := func(token string, request store.LegacyRequest) any {
		t.Helper()
		value, err := call(token, request)
		must(t, err)
		return value
	}
	terms := invoke("seller", store.LegacyRequest{Action: "save_terms", Body: body(map[string]any{"content": "Synthetic promotion terms", "expected_version": ""})}).(map[string]any)
	product := invoke("seller", store.LegacyRequest{Action: "save_product", Body: body(map[string]any{"title": "Protected synthetic document", "description": "Coupon fixture", "price_quota": 500001, "template": "fixed-content", "fixed_content": "PRIVATE synthetic fixed material", "payment_methods": []string{"balance"}, "pickup_login_required": true, "pickup_code_required": true, "max_quantity_per_buyer": 100, "max_quantity_per_order": 10})}).(map[string]any)
	id := product["id"].(string)
	invoke("seller", store.LegacyRequest{Action: "submit_product", ID: id})
	invoke("admin", store.LegacyRequest{Action: "review_product", ID: id, Body: body(map[string]any{"approve": true})})
	row, err := service.LocateProduct(ctx, id)
	must(t, err)
	variant := row.Product.Legacy.DefaultVariantID
	input := map[string]any{"discount_bps": 5000, "variant_ids": []string{variant}, "max_uses": 1, "expires_at": nil, "status": "active"}
	_, err = call("outsider", store.LegacyRequest{Action: "save_promotion", ID: id, Body: body(input)})
	if !errors.Is(err, store.ErrForbidden) {
		t.Fatalf("non-owner coupon creation: %v", err)
	}
	promo := invoke("seller", store.LegacyRequest{Action: "save_promotion", ID: id, Body: body(input)}).(store.Promotion)
	resolved, err := service.ResolvePromotion(ctx, id, promo.Code)
	must(t, err)
	if resolved["discount_bps"] != int64(5000) {
		t.Fatal("wrong public promotion")
	}
	quote := invoke("buyer", store.LegacyRequest{Action: "promotion_quote", ID: id, Body: body(map[string]any{"promotion_code": promo.Code, "variant_id": variant, "quantity": 2})}).(map[string]any)
	if quote["price_quota"] != int64(500002) || quote["discount_quota"] != int64(500000) || quote["checkout_allowed"] != true {
		t.Fatalf("wrong locked quote: %+v", quote)
	}
	invoke("buyer", store.LegacyRequest{Action: "accept_disclaimer", Body: body(map[string]any{"version": "merchant-store-v1", "accepted": true})})
	checkout := map[string]any{"product_id": id, "variant_id": variant, "quantity": 2, "payment_method": "balance", "promotion_code": promo.Code, "pickup_code": "Synthetic-pickup-123", "request_key": "promotion-paid-reservation", "disclaimer_version": "merchant-store-v1", "seller_terms_version": terms["version"], "accept_seller_terms": true}
	created := invoke("buyer", store.LegacyRequest{Action: "create_order", Body: body(checkout)}).(map[string]any)
	paidOrder := created["order"].(map[string]any)
	paidID := paidOrder["id"].(string)
	if paidOrder["price_quota"] != int64(500002) || paidOrder["pickup_code_required"] != true {
		t.Fatalf("wrong saved coupon order: %+v", paidOrder)
	}
	pendingLink := invoke("buyer", store.LegacyRequest{Action: "pickup_link", ID: paidID}).(map[string]any)
	token := strings.TrimPrefix(pendingLink["pickup_url"].(string), "/store/claim/")
	_, err = service.Pickup(ctx, "buyer", 2, token, "Synthetic-pickup-123", true)
	if !errors.Is(err, store.ErrConflict) {
		t.Fatalf("unpaid order collected: %v", err)
	}
	invoke("buyer", store.LegacyRequest{Action: "cancel_order", ID: paidID})
	input["discount_bps"] = 10000
	invoke("seller", store.LegacyRequest{Action: "save_promotion", ID: id, ChildID: promo.ID, Body: body(input)})
	// A new instance has no money adapter. The free promotion must still
	// reserve and deliver exactly once, without fabricating a Core receipt.
	service, err = store.New(repo, legacyAuthority{}, nil)
	must(t, err)
	checkout["payment_method"], checkout["request_key"] = "free", "promotion-free-order"
	created = invoke("buyer", store.LegacyRequest{Action: "create_order", Body: body(checkout)}).(map[string]any)
	orderView := created["order"].(map[string]any)
	orderID := orderView["id"].(string)
	if orderView["price_quota"] != int64(0) || orderView["status"] != "paid" || orderView["payment_method"] != "free" || created["created"] != true {
		t.Fatalf("free order incomplete: %+v", created)
	}
	link := invoke("buyer", store.LegacyRequest{Action: "pickup_link", ID: orderID}).(map[string]any)
	token = strings.TrimPrefix(link["pickup_url"].(string), "/store/claim/")
	metadata, err := service.Pickup(ctx, "", 0, token, "", false)
	must(t, err)
	if metadata["pickup_login_required"] != true || metadata["pickup_login_satisfied"] != false || metadata["pickup_code_required"] != true {
		t.Fatalf("wrong collection policy: %+v", metadata)
	}
	if _, exists := metadata["fixed_content"]; exists {
		t.Fatal("metadata revealed delivery")
	}
	_, err = service.Pickup(ctx, "", 0, token, "Synthetic-pickup-123", true)
	if !errors.Is(err, store.ErrUnauthorized) {
		t.Fatalf("anonymous collection accepted: %v", err)
	}
	_, err = service.Pickup(ctx, "outsider", 3, token, "Synthetic-pickup-123", true)
	if !errors.Is(err, store.ErrNotFound) {
		t.Fatalf("different account collected: %v", err)
	}
	_, err = service.Pickup(ctx, "buyer", 2, token, "wrong-synthetic-code", true)
	if !errors.Is(err, store.ErrForbidden) {
		t.Fatalf("incorrect pickup code accepted: %v", err)
	}
	claim, err := service.Pickup(ctx, "buyer", 2, token, "Synthetic-pickup-123", true)
	must(t, err)
	if claim["fixed_content"] != "PRIVATE synthetic fixed material" {
		t.Fatalf("missing committed material: %+v", claim)
	}
	persisted, err := repo.FindOrder(ctx, orderID)
	must(t, err)
	if !persisted.Order.Delivered || persisted.Order.State != store.Settled || persisted.Order.PaymentRef != "" {
		t.Fatalf("free financial state: %+v", persisted.Order)
	}
	encoded := string(body(orderView))
	if strings.Contains(encoded, "Synthetic-pickup-123") || strings.Contains(encoded, "PRIVATE synthetic") || strings.Contains(encoded, "\"hash\"") {
		t.Fatal("private pickup material appeared in order DTO")
	}
	var count int
	must(t, db.QueryRow(`SELECT COUNT(*) FROM lmm_store.money_operations`).Scan(&count))
	if count != 0 || ledger.effects != 0 {
		t.Fatal("free coupon or reservations moved money")
	}
	list := invoke("seller", store.LegacyRequest{Action: "promotions", ID: id, Limit: 20}).(map[string]any)["items"].([]store.Promotion)
	if len(list) != 1 || list[0].UsesCount != 1 || list[0].ReservedCount != 0 {
		t.Fatalf("PostgreSQL coupon counts wrong: %+v", list)
	}
	_, err = call("seller", store.LegacyRequest{Action: "promotion_batch", ID: id, Body: body(map[string]any{"ids": []string{promo.ID}, "action": "delete"})})
	if !errors.Is(err, store.ErrConflict) {
		t.Fatalf("used coupon deleted: %v", err)
	}
	invoke("seller", store.LegacyRequest{Action: "promotion_batch", ID: id, Body: body(map[string]any{"ids": []string{promo.ID}, "action": "pause"})})
	invoke("seller", store.LegacyRequest{Action: "unlist_product", ID: id})
	replay := invoke("buyer", store.LegacyRequest{Action: "create_order", Body: body(checkout)}).(map[string]any)
	if replay["created"] != false || replay["order"].(map[string]any)["id"] != orderID {
		t.Fatal("replay after unlisting/pause did not return original order")
	}
	invoke("seller", store.LegacyRequest{Action: "listing", ID: id, Body: body(map[string]any{"listed": true})})
	input["max_uses"] = 3
	concurrent := invoke("seller", store.LegacyRequest{Action: "save_promotion", ID: id, Body: body(input)}).(store.Promotion)
	policy := &store.LegacyCheckoutPolicy{Version: terms["version"].(string), Accepted: true}
	var accepted atomic.Int64
	var group sync.WaitGroup
	for i := range 12 {
		group.Add(1)
		go func() {
			defer group.Done()
			_, err := service.Purchase(ctx, "buyer", store.Purchase{ShopID: row.ShopID, VariantID: variant, Buyer: store.Account{Kind: "personal", ID: 2}, Quantity: 1, Key: fmt.Sprintf("pg-promotion-%d", i), Policy: policy, PromotionCode: concurrent.Code, RequireFree: true, PickupCode: "Concurrent-code-123"})
			if err == nil {
				accepted.Add(1)
			} else if !errors.Is(err, store.ErrConflict) {
				t.Errorf("competing coupon result: %v", err)
			}
		}()
	}
	group.Wait()
	if accepted.Load() != 3 {
		t.Fatalf("PostgreSQL promotion cap accepted %d", accepted.Load())
	}
}
