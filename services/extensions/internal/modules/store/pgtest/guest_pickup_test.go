// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// Real SQL persistence and service reconstruction; authority is synthetic.
// Free promotion orders must never invoke the available synthetic funds port.
func TestPostgresAnonymousPickupFrozenPolicyAndLegacyFallback(t *testing.T) {
	db := legacyDatabase(t)
	ctx := context.Background()
	repo, err := store.NewPostgres(db)
	must(t, err)
	ledger := &funds{seen: map[string]store.Receipt{}}
	service, err := store.New(repo, legacyAuthority{}, ledger)
	must(t, err)
	body := func(v any) json.RawMessage { raw, err := json.Marshal(v); must(t, err); return raw }
	invoke := func(token string, r store.LegacyRequest) any {
		t.Helper()
		v, err := service.Legacy(ctx, token, store.Account{Kind: "personal", ID: map[string]int64{"seller": 1, "buyer": 2, "admin": 4}[token]}, r)
		must(t, err)
		return v
	}
	terms := invoke("seller", store.LegacyRequest{Action: "save_terms", Body: body(map[string]any{"content": "Synthetic guest pickup terms", "expected_version": ""})}).(map[string]any)
	invoke("buyer", store.LegacyRequest{Action: "accept_disclaimer", Body: body(map[string]any{"version": "merchant-store-v1", "accepted": true})})
	for _, required := range []bool{false, true} {
		t.Run(fmt.Sprintf("login-required-%t", required), func(t *testing.T) {
			product := invoke("seller", store.LegacyRequest{Action: "save_product", Body: body(map[string]any{"title": "Synthetic bearer pickup", "price_quota": 500001, "template": "fixed-content", "fixed_content": "PRIVATE durable delivery", "payment_methods": []string{"balance"}, "pickup_login_required": required, "pickup_code_required": true})}).(map[string]any)
			id := product["id"].(string)
			invoke("seller", store.LegacyRequest{Action: "submit_product", ID: id})
			invoke("admin", store.LegacyRequest{Action: "review_product", ID: id, Body: body(map[string]any{"approve": true})})
			row, err := service.LocateProduct(ctx, id)
			must(t, err)
			promo := invoke("seller", store.LegacyRequest{Action: "save_promotion", ID: id, Body: body(map[string]any{"discount_bps": 10000, "variant_ids": []string{row.Product.Legacy.DefaultVariantID}, "max_uses": 1, "status": "active"})}).(store.Promotion)
			created := invoke("buyer", store.LegacyRequest{Action: "create_order", Body: body(map[string]any{"product_id": id, "quantity": 1, "payment_method": "free", "promotion_code": promo.Code, "pickup_code": "Synthetic-pickup-123", "request_key": fmt.Sprintf("frozen-pickup-%t", required), "disclaimer_version": "merchant-store-v1", "seller_terms_version": terms["version"], "accept_seller_terms": true})}).(map[string]any)
			orderView := created["order"].(map[string]any)
			orderID := orderView["id"].(string)
			if orderView["pickup_login_required"] != required {
				t.Fatal("order DTO did not freeze login policy")
			}
			link := invoke("buyer", store.LegacyRequest{Action: "pickup_link", ID: orderID}).(map[string]any)
			token := strings.TrimPrefix(link["pickup_url"].(string), "/store/claim/")
			must(t, repo.Within(ctx, row.ShopID, func(tx store.Tx) error {
				var product store.Product
				if err := tx.Get("products", id, &product); err != nil {
					return err
				}
				product.Legacy.PickupLoginRequired = !required
				return tx.Put("products", id, product)
			}))
			// Reconstruct both objects: no Service or Repository memory may supply the policy/link.
			repo, err = store.NewPostgres(db)
			must(t, err)
			service, err = store.New(repo, legacyAuthority{}, ledger)
			must(t, err)
			metadata, err := service.Pickup(ctx, "", 0, token, "", false)
			must(t, err)
			if metadata["pickup_login_required"] != required || metadata["pickup_login_satisfied"] != !required {
				t.Fatal("reconstruction used the live product policy")
			}
			if _, ok := metadata["fixed_content"]; ok {
				t.Fatal("metadata exposed delivery")
			}
			claim, err := service.Pickup(ctx, "", 0, token, "Synthetic-pickup-123", true)
			if required {
				if !errors.Is(err, store.ErrUnauthorized) {
					t.Fatalf("required login: %v", err)
				}
			} else {
				must(t, err)
				if claim["fixed_content"] != "PRIVATE durable delivery" {
					t.Fatal("anonymous pickup lost persisted delivery")
				}
			}
			// Simulate an old JSON record which predates the explicit policy field.
			_, err = db.ExecContext(ctx, `UPDATE lmm_store.orders SET body=body #- '{legacy,pickup_login_required}' WHERE shop_id=$1 AND id=$2`, row.ShopID, orderID)
			must(t, err)
			service, err = store.New(repo, legacyAuthority{}, ledger)
			must(t, err)
			metadata, err = service.Pickup(ctx, "", 0, token, "", false)
			must(t, err)
			if metadata["pickup_login_required"] != true {
				t.Fatal("old record downgraded to bearer pickup")
			}
			_, err = service.Pickup(ctx, "", 0, token, "Synthetic-pickup-123", true)
			if !errors.Is(err, store.ErrUnauthorized) {
				t.Fatalf("old order anonymous pickup: %v", err)
			}
			claim, err = service.Pickup(ctx, "buyer", 2, token, "Synthetic-pickup-123", true)
			must(t, err)
			if claim["fixed_content"] != "PRIVATE durable delivery" {
				t.Fatal("old order owner lost delivery")
			}
			persisted, err := repo.FindOrder(ctx, orderID)
			must(t, err)
			if persisted.Order.Legacy.PickupLoginRequired != nil || persisted.Order.PaymentRef != "" || persisted.Order.SettlementRef != "" || persisted.Order.RefundRef != "" || persisted.Order.Total.Amount != 0 {
				t.Fatal("pickup changed financial facts or backfilled old policy")
			}
		})
	}
	var count int
	must(t, db.QueryRowContext(ctx, "SELECT COUNT(*) FROM lmm_store.money_operations").Scan(&count))
	if count != 0 || ledger.effects != 0 {
		t.Fatal("pickup created a money operation")
	}
}
