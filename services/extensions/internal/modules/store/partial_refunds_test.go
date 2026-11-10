// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"errors"
	"testing"
)

// Synthetic Go state-machine regressions; no claim of Core/PG ledger evidence.
type store02Funds struct{ *a06StateFunds }

func (*store02Funds) PartialRefundAvailable() bool { return true }

func store02Setup(t *testing.T) *a06StateFixture {
	f := a06RefundSetup(t)
	f.s.funds = &store02Funds{f.funds}
	return f
}
func store02Request(t *testing.T, f *a06StateFixture, key, mode string, quantity, amount int64) string {
	t.Helper()
	v, err := a06RefundCall(f, buyerToken, "request_refund", "", map[string]any{
		"request_key": key, "reason": "Partial return", "mode": mode, "quantity": quantity, "amount_quota": amount})
	must(t, err)
	return v.(map[string]any)["id"].(string)
}
func store02Approve(f *a06StateFixture, id string) (any, error) {
	return a06RefundCall(f, sellerToken, "decide_refund", id, map[string]any{"decision": "approve", "reason": "Approved"})
}

func TestStore02QuantityBeforeCaptureRetainsOriginalEconomicsAndFinalRemainder(t *testing.T) {
	f := store02Setup(t)
	id := store02Request(t, f, "first", "quantity", 1, 0)
	_, err := store02Approve(f, id)
	must(t, err)
	o := f.order(t)
	if o.State != Paid || o.Total.Amount != 100 || o.Quantity != 2 || o.PartialRefunds[0].Spec.Amount != 50 {
		t.Fatal(o)
	}
	var v Variant
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error { return tx.Get("variants", o.VariantID, &v) }))
	if v.Reserved != 1 || v.Claimed != 1 || v.Stock != 10 {
		t.Fatal(v)
	}
	f.funds.capture = func(c MoneyCommand) (Receipt, error) {
		if c.Total.Amount != 100 || c.OrderSnapshot.Quantity != 2 || c.Refund != nil {
			t.Fatalf("shrunk original capture command: %+v", c)
		}
		return a06StateReceipt(c), nil
	}
	_, err = f.s.FinalizeDelivery(context.Background(), f.shop, f.id)
	must(t, err)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error { return tx.Get("variants", o.VariantID, &v) }))
	if v.Reserved != 0 || v.Stock != 9 {
		t.Fatal(v)
	}
	id = store02Request(t, f, "remainder", "full", 0, 0)
	_, err = store02Approve(f, id)
	must(t, err)
	o = f.order(t)
	if o.State != Refunded || !o.Delivered || len(o.PartialRefunds) != 2 || o.PartialRefunds[1].Spec.Amount != 50 || o.Total.Amount != 100 {
		t.Fatal(o)
	}
}

func TestStore02PendingCapsCancelAndIndependentKeys(t *testing.T) {
	f := store02Setup(t)
	id := store02Request(t, f, "amount", "amount", 0, 70)
	image := f.image(t)
	_, err := a06RefundCall(f, buyerToken, "request_refund", "", map[string]any{"request_key": "too-much", "reason": "Partial return", "mode": "quantity", "quantity": 1})
	if !errors.Is(err, ErrConflict) || image != f.image(t) {
		t.Fatal("reserved principal bypassed", err)
	}
	_, err = a06RefundCall(f, buyerToken, "cancel_refund", id, map[string]any{})
	must(t, err)
	second := store02Request(t, f, "quantity", "quantity", 1, 0)
	if id == second {
		t.Fatal("refund identities reused")
	}
	_, err = store02Approve(f, second)
	must(t, err)
	if f.order(t).PartialRefunds[0].State != "cancelled" {
		t.Fatal("cancelled request mutated")
	}
}

func TestStore02LostACKRecoveryNeverDispatchesOrCancelsUnknown(t *testing.T) {
	f := store02Setup(t)
	id := store02Request(t, f, "ack", "quantity", 1, 0)
	var original MoneyCommand
	f.funds.execute = func(c MoneyCommand) (Receipt, error) { original = c; return Receipt{}, ErrPending }
	_, err := store02Approve(f, id)
	wantError(t, err, ErrPending)
	calls := f.funds.execCalls.Load()
	f.funds.query = func(c MoneyCommand) (Receipt, error) {
		if ID(c) != ID(original) {
			t.Fatal("changed recovery identity")
		}
		return Receipt{}, ErrPending
	}
	_, err = a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	_, err = a06RefundCall(f, buyerToken, "cancel_refund", id, map[string]any{})
	wantError(t, err, ErrConflict)
	if f.funds.execCalls.Load() != calls || f.order(t).State != RefundPending {
		t.Fatal("UNKNOWN caused dispatch/cancellation")
	}
	f.funds.query = func(c MoneyCommand) (Receipt, error) { return a06StateReceipt(c), nil }
	_, err = a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	if f.funds.execCalls.Load() != calls || f.order(t).PartialRefunds[0].State != "completed" || f.order(t).State != Paid {
		t.Fatal("read-only recovery failed")
	}
}

func TestStore02CapabilityFailureLeavesOrderUnchanged(t *testing.T) {
	f := a06RefundSetup(t)
	before := f.image(t)
	_, err := a06RefundCall(f, buyerToken, "request_refund", "", map[string]any{"request_key": "no-cap", "reason": "Return", "mode": "quantity", "quantity": 1})
	wantError(t, err, ErrUnavailable)
	if f.image(t) != before {
		t.Fatal("unnegotiated field reserved a refund")
	}
}

func TestStore02FreePendingSelectionDoesNotCountAsCompleted(t *testing.T) {
	f := a06StateSetup(t)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.UnitPrice.Amount = 0
		o.Total.Amount = 0
		o.State = Settled
		o.Delivered = true
		o.Delivery = "Free content"
		o.Legacy.PaymentMethod = "free"
		if err := tx.Put("orders", o.ID, o); err != nil {
			return err
		}
		return tx.Put("variants", o.VariantID, Variant{ID: o.VariantID, Stock: 8, Claimed: 2, Quota: -1})
	}))
	var err error
	f.s, err = New(&a06RefundIndex{memoryRepo: f.repo, shop: f.shop}, f.auth, f.funds)
	must(t, err)
	first := store02Request(t, f, "free-one", "quantity", 1, 0)
	second := store02Request(t, f, "free-two", "quantity", 1, 0)
	_, err = store02Approve(f, first)
	must(t, err)
	if f.order(t).State != Settled || f.funds.execCalls.Load() != 0 {
		t.Fatal("pending free units became completed or issued money")
	}
	_, err = store02Approve(f, second)
	must(t, err)
	if f.order(t).State != Refunded || f.funds.execCalls.Load() != 0 {
		t.Fatal("free remainder not retired locally")
	}
}

func TestStore02ProactiveUsesSellerIdentityAndSameCorePrincipal(t *testing.T) {
	f := store02Setup(t)
	input := map[string]any{"request_key": "seller-return", "reason": "Partial return", "mode": "amount", "amount_quota": 20}
	v, err := a06RefundCall(f, sellerToken, "proactive_refund", "", input)
	must(t, err)
	id := v.(map[string]any)["id"].(string)
	calls := f.funds.execCalls.Load()
	_, err = a06RefundCall(f, sellerToken, "proactive_refund", "", input)
	must(t, err)
	o := f.order(t)
	if len(o.PartialRefunds) != 1 || o.PartialRefunds[0].Spec.Role != "seller" || o.PartialRefunds[0].Spec.Amount != 20 || o.PartialRefunds[0].Spec.ID != id || calls != f.funds.execCalls.Load() {
		t.Fatal("proactive replay changed financial identity", o)
	}
}

func TestStore02QueuedAmountThenQuantityRetiresFinalPickupRightsAtDispatch(t *testing.T) {
	f := store02Setup(t)
	amount := store02Request(t, f, "queued-amount", "amount", 0, 50)
	quantity := store02Request(t, f, "queued-unit", "quantity", 1, 0)
	_, err := store02Approve(f, amount)
	must(t, err)
	_, err = store02Approve(f, quantity)
	must(t, err)
	o := f.order(t)
	if o.State != Refunded || o.PartialRefunds[1].Spec.Quantity != 1 || o.PartialRefunds[1].Spec.RetiredQuantity != 2 || o.PartialRefunds[1].Command.Refund.RetiredQuantity != 2 {
		t.Fatal("final money slice left an unfunded pickup right", o)
	}
	var v Variant
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error { return tx.Get("variants", o.VariantID, &v) }))
	if v.Reserved != 0 || v.Claimed != 0 || v.Stock != 10 {
		t.Fatal(v)
	}
}

func TestStore02LostNegativeACKReleasesReservationWithoutMoneyOrFakeCancellation(t *testing.T) {
	f := store02Setup(t)
	id := store02Request(t, f, "negative-ack", "amount", 0, 30)
	f.funds.execute = func(MoneyCommand) (Receipt, error) { return Receipt{}, ErrPending }
	_, err := store02Approve(f, id)
	wantError(t, err, ErrPending)
	calls := f.funds.execCalls.Load()
	f.funds.query = func(c MoneyCommand) (Receipt, error) { r := a06StateReceipt(c); r.Outcome = "rejected"; return r, nil }
	v, err := a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	view := v.(map[string]any)
	if view["reserved_quota"] != int64(0) || view["remaining_quota"] != int64(100) || f.order(t).PartialRefunds[0].State != "rejected" || f.funds.execCalls.Load() != calls {
		t.Fatal("negative receipt stayed UNKNOWN or caused money", view)
	}
}

func TestStore02ReturnedUnitCanBeReissuedWithoutLaterDeliveryOrRefundTouchingIt(t *testing.T) {
	f := store02Setup(t)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.StockItems = []string{"stock-a", "stock-b"}
		o.Legacy.DeliveryTemplate = "card-key"
		for _, id := range o.StockItems {
			if err := tx.Put("stock_items", id, StockItem{ID: id, VariantID: o.VariantID, OrderID: o.ID, State: stockReserved, Ciphertext: "synthetic-opaque-fixture"}); err != nil {
				return err
			}
		}
		return tx.Put("orders", o.ID, o)
	}))
	first := store02Request(t, f, "stock-first", "quantity", 1, 0)
	_, err := store02Approve(f, first)
	must(t, err)
	variantID := f.order(t).VariantID
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var item StockItem
		if err := tx.Get("stock_items", "stock-a", &item); err != nil {
			return err
		}
		if item.State != stockAvailable || item.OrderID != "" {
			t.Fatal("undelivered unit not released", item)
		}
		item.State = stockReserved
		item.OrderID = "another-order"
		if err := tx.Put("stock_items", item.ID, item); err != nil {
			return err
		}
		var v Variant
		if err := tx.Get("variants", variantID, &v); err != nil {
			return err
		}
		if err := v.reserve(1); err != nil {
			return err
		}
		return tx.Put("variants", v.ID, v)
	}))
	_, err = f.s.FinalizeDelivery(context.Background(), f.shop, f.id)
	must(t, err)
	last := store02Request(t, f, "stock-last", "amount", 0, 50)
	_, err = store02Approve(f, last)
	must(t, err)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var a, b StockItem
		if err := tx.Get("stock_items", "stock-a", &a); err != nil {
			return err
		}
		if err := tx.Get("stock_items", "stock-b", &b); err != nil {
			return err
		}
		if a.State != stockReserved || a.OrderID != "another-order" || b.State != stockRefunded {
			t.Fatal("retired/reissued ownership crossed", a, b)
		}
		return nil
	}))
}

func TestStore02LegacyFullCancellationRequiresAbsentOriginalDispatchFence(t *testing.T) {
	f := a06RefundSetup(t)
	id := a06RefundOpen(t, f)
	_, err := a06RefundCall(f, buyerToken, "cancel_refund", id, map[string]any{})
	must(t, err)
	if f.order(t).RefundState != "cancelled" || f.funds.execCalls.Load() != 1 {
		t.Fatal("unsubmitted full refund cancellation moved money")
	}
	f = a06RefundSetup(t)
	id = a06RefundOpen(t, f)
	f.funds.execute = func(MoneyCommand) (Receipt, error) { return Receipt{}, ErrPending }
	_, err = store02Approve(f, id)
	wantError(t, err, ErrPending)
	_, err = a06RefundCall(f, buyerToken, "cancel_refund", id, map[string]any{})
	wantError(t, err, ErrConflict)
	if f.order(t).State != RefundPending {
		t.Fatal("dispatched full refund became cancelled")
	}
}
