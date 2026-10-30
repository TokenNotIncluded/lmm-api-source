// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func writePromotion(t *testing.T, f *fixture, p Promotion) {
	t.Helper()
	must(t, f.repo.Within(context.Background(), f.shop.ID, func(tx Tx) error {
		return tx.Put("commands", "@web:promotions:"+f.product.ID, promotionList{Items: []Promotion{p}})
	}))
}

func testPromotion(f *fixture, bps, maximum int64) Promotion {
	return Promotion{ID: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", ProductID: f.product.ID, SellerID: 1, Code: "SYNTHETIC25", DiscountBPS: bps, MaxUses: &maximum, Status: "active", VariantIDs: []string{}}
}

func TestPromotionReservationFreezesPriceAndReleasesCancellation(t *testing.T) {
	ctx := context.Background()
	f, policy := limitFixture(t, 100, 10)
	promo := testPromotion(f, 2500, 1)
	writePromotion(t, f, promo)
	p := Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 2, Key: "coupon-original", Policy: policy, PromotionCode: promo.Code}
	original, err := f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	if original.UnitPrice.Amount != 375001 || original.Total.Amount != 750002 || original.Legacy.OriginalUnitPrice != 500001 || original.Legacy.DiscountBPS != 2500 {
		t.Fatalf("discount snapshot: %+v", original)
	}
	p.Key = "coupon-exhausted"
	_, err = f.s.Purchase(ctx, buyerToken, p)
	wantError(t, err, ErrConflict)
	promo.DiscountBPS = 5000
	writePromotion(t, f, promo)
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var v Variant
		if err := tx.Get("variants", f.variant.ID, &v); err != nil {
			return err
		}
		v.Price.Amount = 1000000
		return tx.Put("variants", v.ID, v)
	}))
	p.Key = "coupon-original"
	replay, err := f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	if replay.ID != original.ID || replay.Total != original.Total || replay.Legacy.DiscountBPS != 2500 {
		t.Fatal("repricing or promotion edits changed an existing reservation")
	}
	_, err = f.s.ChangeOrder(ctx, buyerToken, OrderChange{ShopID: f.shop.ID, OrderID: original.ID, Action: "cancel"})
	must(t, err)
	p.Key = "coupon-after-cancel"
	next, err := f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	if next.UnitPrice.Amount != 500000 || next.Total.Amount != 1000000 {
		t.Fatalf("new snapshot: %+v", next)
	}
	if len(f.funds.commands) != 0 {
		t.Fatal("reservations moved funds")
	}
}

func TestPromotionSerializesCompetingReservations(t *testing.T) {
	f, policy := limitFixture(t, 100, 10)
	promo := testPromotion(f, 2500, 3)
	writePromotion(t, f, promo)
	var accepted atomic.Int64
	var group sync.WaitGroup
	for i := range 12 {
		group.Add(1)
		go func() {
			defer group.Done()
			_, err := f.s.Purchase(context.Background(), buyerToken, Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 1, Key: fmt.Sprintf("coupon-race-%d", i), Policy: policy, PromotionCode: promo.Code})
			if err == nil {
				accepted.Add(1)
			} else if !errors.Is(err, ErrConflict) {
				t.Errorf("unexpected reservation result: %v", err)
			}
		}()
	}
	group.Wait()
	if accepted.Load() != 3 || f.stock(t).Reserved != 3 {
		t.Fatalf("coupon use cap exceeded: accepted=%d reserved=%d", accepted.Load(), f.stock(t).Reserved)
	}
}

func TestFreePromotionDeliversWithoutFundsAndProtectsPickupCode(t *testing.T) {
	ctx := context.Background()
	f, policy := limitFixture(t, 100, 10)
	var err error
	f.s, err = New(f.repo, f.auth, nil)
	must(t, err)
	promo := testPromotion(f, 10000, 2)
	writePromotion(t, f, promo)
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var product Product
		if err := tx.Get("products", f.product.ID, &product); err != nil {
			return err
		}
		product.Legacy.PickupCodeRequired = true
		return tx.Put("products", product.ID, product)
	}))
	p := Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 2, Key: "free-with-pickup", Policy: policy, PromotionCode: promo.Code, RequireFree: true, PickupCode: "Synthetic-secret-123"}
	order, err := f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	if order.State != Settled || !order.Delivered || order.Delivery != "Synthetic delivery" || order.Total.Amount != 0 || order.PaymentRef != "" || order.Legacy.PaymentMethod != "free" || !order.Legacy.PickupCodeRequired {
		t.Fatalf("free delivery did not complete atomically: %+v", order)
	}
	stock := f.stock(t)
	if stock.Reserved != 0 || stock.Claimed != 2 || stock.Stock != 98 {
		t.Fatalf("free delivery stock: %+v", stock)
	}
	raw, err := json.Marshal(order)
	must(t, err)
	if strings.Contains(string(raw), p.PickupCode) || strings.Contains(string(raw), "\"hash\"") || strings.Contains(string(raw), "\"salt\"") {
		t.Fatal("pickup secret appeared in order projection")
	}
	replay, err := f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	if replay.ID != order.ID || f.stock(t).Claimed != 2 {
		t.Fatal("free replay consumed additional goods")
	}
	p.PickupCode = "Incorrect-secret-123"
	_, err = f.s.Purchase(ctx, buyerToken, p)
	wantError(t, err, ErrConflict)
	promo.DiscountBPS = 2500
	writePromotion(t, f, promo)
	p.Key = "free-price-changed"
	_, err = f.s.Purchase(ctx, buyerToken, p)
	wantError(t, err, ErrConflict)
	if f.stock(t).Claimed != 2 || len(f.funds.commands) != 0 {
		t.Fatal("rejected free price change reserved goods or moved funds")
	}
}

func TestPromotionUnavailableScopeCannotReserve(t *testing.T) {
	for _, state := range []string{"paused", "expired", "other-variant"} {
		t.Run(state, func(t *testing.T) {
			f, policy := limitFixture(t, 100, 10)
			promo := testPromotion(f, 2500, 10)
			switch state {
			case "paused":
				promo.Status = "paused"
			case "expired":
				at := time.Now().Unix() - 1
				promo.ExpiresAt = &at
			case "other-variant":
				promo.VariantIDs = []string{"second-variant"}
			}
			writePromotion(t, f, promo)
			_, err := f.s.Purchase(context.Background(), buyerToken, Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 1, Key: "invalid-coupon", Policy: policy, PromotionCode: promo.Code})
			wantError(t, err, ErrConflict)
			if f.stock(t).Reserved != 0 {
				t.Fatal("ineligible coupon reserved stock")
			}
		})
	}
}
