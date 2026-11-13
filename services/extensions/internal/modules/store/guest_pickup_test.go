// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"strings"
	"testing"
)

func TestBearerPickupFreezesLoginPolicyAndPreservesCodeAndRefundBoundary(t *testing.T) {
	ctx := context.Background()
	f, policy := limitFixture(t, 100, 10)
	promo := testPromotion(f, 10000, 2)
	writePromotion(t, f, promo)
	order, err := f.s.Purchase(ctx, buyerToken, Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 1, Key: "bearer-pickup", Policy: policy, PromotionCode: promo.Code, RequireFree: true, PickupCode: "Synthetic-pickup-code"})
	must(t, err)
	if order.Legacy.PickupLoginRequired == nil || *order.Legacy.PickupLoginRequired {
		t.Fatal("checkout did not freeze bearer pickup")
	}
	index := &a06RefundIndex{memoryRepo: f.repo, shop: f.shop.ID}
	f.s, err = New(index, f.auth, f.funds)
	must(t, err)
	value, err := f.s.legacyPickupLink(ctx, buyerToken, order.Buyer, order.ID)
	must(t, err)
	token := strings.TrimPrefix(value.(map[string]any)["pickup_url"].(string), "/store/claim/")
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var product Product
		if err := tx.Get("products", f.product.ID, &product); err != nil {
			return err
		}
		product.Legacy.PickupLoginRequired = true
		return tx.Put("products", product.ID, product)
	}))
	view, err := f.s.Pickup(ctx, "", 0, token, "", false)
	must(t, err)
	if view["pickup_login_required"] != false || view["pickup_login_satisfied"] != true {
		t.Fatal("metadata lost frozen pickup policy")
	}
	_, err = f.s.Pickup(ctx, "", 0, token, "wrong-pickup-code", true)
	wantError(t, err, ErrForbidden)
	view, err = f.s.Pickup(ctx, "", 0, token, "Synthetic-pickup-code", true)
	must(t, err)
	if view["fixed_content"] != "Synthetic delivery" {
		t.Fatal("bearer pickup did not deliver")
	}
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", order.ID, &o); err != nil {
			return err
		}
		o.State = Refunded
		return tx.Put("orders", o.ID, o)
	}))
	_, err = f.s.Pickup(ctx, "", 0, token, "Synthetic-pickup-code", true)
	wantError(t, err, ErrConflict)
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", order.ID, &o); err != nil {
			return err
		}
		o.Legacy.PickupLoginRequired = nil
		return tx.Put("orders", o.ID, o)
	}))
	_, err = f.s.Pickup(ctx, "", 0, token, "Synthetic-pickup-code", true)
	wantError(t, err, ErrUnauthorized)
	if len(f.funds.commands) != 0 {
		t.Fatal("pickup moved funds")
	}
}
