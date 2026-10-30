// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"sync/atomic"
	"testing"
)

type categoryAdmin struct{ *authority }

func (a categoryAdmin) CheckAdmin(_ context.Context, token string, _ bool) (int64, error) {
	if token != sellerToken || a.revoked.Load() {
		return 0, ErrForbidden
	}
	return 1, nil
}

func TestCategoryAdministrationAndPublicPagination(t *testing.T) {
	ctx := context.Background()
	a := categoryAdmin{&authority{}}
	s, err := New(newRepo(), a, nil)
	must(t, err)
	body := func(name string, order int, active bool) json.RawMessage {
		raw, err := json.Marshal(map[string]any{"name": name, "sort_order": order, "active": active})
		must(t, err)
		return raw
	}
	write := func(token, id string, raw json.RawMessage) (any, error) {
		actor := Account{"personal", 1}
		if token == buyerToken {
			actor.ID = 2
		}
		return s.Legacy(ctx, token, actor, LegacyRequest{Action: "save_category", ID: id, Body: raw})
	}
	_, err = write(buyerToken, "", body("Unauthorized", 1, true))
	wantError(t, err, ErrForbidden)
	one, err := write(sellerToken, "", body("First", 2, true))
	must(t, err)
	first := one.(Category)
	two, err := write(sellerToken, "", body("Second", 1, true))
	must(t, err)
	second := two.(Category)
	_, err = write(sellerToken, "", body("first", 3, true))
	wantError(t, err, ErrConflict)
	page, err := s.PublicCategories(ctx, 0, 1)
	must(t, err)
	if page["has_more"] != true || page["items"].([]Category)[0].ID != second.ID {
		t.Fatalf("category order/pagination: %+v", page)
	}
	_, err = write(sellerToken, second.ID, body("Second", 1, false))
	must(t, err)
	page, err = s.PublicCategories(ctx, 0, 100)
	must(t, err)
	if items := page["items"].([]Category); len(items) != 1 || items[0].ID != first.ID {
		t.Fatalf("inactive category leaked: %+v", items)
	}
	adminPage, err := s.Legacy(ctx, sellerToken, Account{"personal", 1}, LegacyRequest{Action: "admin_categories", Limit: 100})
	must(t, err)
	if len(adminPage.(map[string]any)["items"].([]Category)) != 2 {
		t.Fatal("disabled category disappeared from admin list")
	}
	_, err = s.readCategory(ctx, second.ID, true)
	wantError(t, err, ErrNotFound)
	_, err = write(sellerToken, second.ID, body("Second restored", 1, true))
	must(t, err)
	a.revoked.Store(true)
	_, err = write(sellerToken, first.ID, body("Revoked", 0, false))
	wantError(t, err, ErrForbidden)
}

func limitFixture(t *testing.T, buyerLimit, orderLimit int64) (*fixture, *LegacyCheckoutPolicy) {
	t.Helper()
	f := setup(t, 500001, 100, -1, false, true)
	policy := &LegacyCheckoutPolicy{Version: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", Accepted: true}
	must(t, f.repo.Within(context.Background(), f.shop.ID, func(tx Tx) error {
		var product Product
		if err := tx.Get("products", f.product.ID, &product); err != nil {
			return err
		}
		product.Legacy = &LegacyProductMetadata{Status: "published", Approved: true, DefaultVariantID: f.variant.ID, MaxQuantityPerBuyer: &buyerLimit, MaxQuantityPerOrder: &orderLimit}
		if err := tx.Put("products", product.ID, product); err != nil {
			return err
		}
		var variant Variant
		if err := tx.Get("variants", f.variant.ID, &variant); err != nil {
			return err
		}
		variant.Price = Price{Amount: 500001, Unit: "credits"}
		variant.Legacy = &LegacyVariantMetadata{Enabled: true, Template: "fixed-content", FixedContent: "Synthetic delivery"}
		if err := tx.Put("variants", variant.ID, variant); err != nil {
			return err
		}
		variant.ID = "second-variant"
		if err := tx.Put("variants", variant.ID, variant); err != nil {
			return err
		}
		return tx.Put("commands", "@web:seller-terms", legacySellerTerms{Version: policy.Version, Content: "Synthetic terms", UpdatedAt: 1})
	}))
	return f, policy
}

func TestBuyerLimitSpansVariantsAndReleasesOnlyCancelledOrders(t *testing.T) {
	ctx := context.Background()
	f, policy := limitFixture(t, 3, 2)
	p := Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 2, Key: "limit-first", Policy: policy}
	first, err := f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	p.VariantID, p.Key = "second-variant", "limit-second"
	_, err = f.s.Purchase(ctx, buyerToken, p)
	wantError(t, err, ErrPurchaseLimit)
	p.Quantity = 1
	_, err = f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	// The same idempotency key reads the original order even at the limit.
	_, err = f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	p.Key = "limit-third"
	_, err = f.s.Purchase(ctx, buyerToken, p)
	wantError(t, err, ErrPurchaseLimit)
	_, err = f.s.ChangeOrder(ctx, buyerToken, OrderChange{ShopID: f.shop.ID, OrderID: first.ID, Action: "cancel"})
	must(t, err)
	p.Quantity = 2
	_, err = f.s.Purchase(ctx, buyerToken, p)
	must(t, err)
	if len(f.funds.commands) != 0 {
		t.Fatal("limit enforcement or reservations moved funds")
	}
}

func TestBuyerLimitSerializesCompetingReservations(t *testing.T) {
	f, policy := limitFixture(t, 3, 2)
	var passed atomic.Int64
	var group sync.WaitGroup
	for i := range 12 {
		group.Add(1)
		go func() {
			defer group.Done()
			_, err := f.s.Purchase(context.Background(), buyerToken, Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 1, Key: fmt.Sprintf("limit-race-%d", i), Policy: policy})
			if err == nil {
				passed.Add(1)
			} else if !errors.Is(err, ErrPurchaseLimit) {
				t.Errorf("unexpected reservation result: %v", err)
			}
		}()
	}
	group.Wait()
	if passed.Load() != 3 || f.stock(t).Reserved != 3 {
		t.Fatalf("limit exceeded: passed=%d reserved=%d", passed.Load(), f.stock(t).Reserved)
	}
}
