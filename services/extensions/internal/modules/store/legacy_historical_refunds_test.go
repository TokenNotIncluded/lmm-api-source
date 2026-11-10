// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"strings"
	"testing"
)

// Synthetic persistence/ports only; these tests do not prove real PG money.
type historicalFundsFixture struct {
	*a06StateFunds
	executed, read int
	known          map[string]Receipt
	afterExecute   func()
}

func (*historicalFundsFixture) LegacyRefundAvailable() bool { return true }
func (f *historicalFundsFixture) ExecuteLegacyRefund(_ context.Context, _ string, c LegacyRefundCommand) (Receipt, error) {
	f.executed++
	r := Receipt{Key: c.Key, CommandHash: ID(c), Reference: "legacy-store-refund:" + c.Key, Outcome: "succeeded"}
	f.known[c.Key] = r
	if f.afterExecute != nil {
		f.afterExecute()
	}
	return r, nil
}
func (f *historicalFundsFixture) ReconcileLegacyRefund(_ context.Context, c LegacyRefundCommand) (Receipt, error) {
	f.read++
	r, ok := f.known[c.Key]
	if !ok {
		return Receipt{}, ErrPending
	}
	return r, nil
}
func historicalSetup(t *testing.T) (*a06StateFixture, *historicalFundsFixture, *a06RefundIndex) {
	t.Helper()
	f := a06StateSetup(t)
	funds := &historicalFundsFixture{a06StateFunds: f.funds, known: map[string]Receipt{}}
	index := &a06RefundIndex{memoryRepo: f.repo, shop: f.shop}
	var err error
	f.s, err = New(index, f.auth, funds)
	must(t, err)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.State = Settled
		o.Delivered = true
		o.Delivery = "original fixed delivery"
		o.Legacy.PaidAt = 20
		o.Imported = &ImportMarker{Source: "go-v0.2.118", Snapshot: strings.Repeat("c", 64)}
		return tx.Put("orders", f.id, o)
	}))
	return f, funds, index
}
func TestHistoricalRefundedHistoryNeedsSourceButNeverNativePaymentOrMoney(t *testing.T) {
	f, funds, _ := historicalSetup(t)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.State = Refunded
		o.RefundState = "completed"
		o.LegacyRefund = &legacyRefundRecord{ID: ID("original-refund"), State: "completed", CompletedAt: 30}
		return tx.Put("orders", f.id, o)
	}))
	before := f.image(t)
	v, err := a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	if v.(map[string]any)["refunded_quota"] != int64(100) || funds.executed != 0 || funds.read != 0 || f.funds.execCalls.Load() != 0 || f.image(t) != before {
		t.Fatal("historical read created a money operation or required native payment")
	}
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.Imported.Snapshot = "unproven"
		return tx.Put("orders", f.id, o)
	}))
	_, err = a06RefundCall(f, buyerToken, "refunds", "", nil)
	wantError(t, err, ErrConflict)
}
func TestHistoricalFullRefundLostACKRecoversOriginalOperationByReadOnlyReceipt(t *testing.T) {
	f, funds, index := historicalSetup(t)
	id := a06RefundOpen(t, f)
	decision := map[string]any{"decision": "approve", "reason": "Original full principal"}
	_, err := a06RefundCall(f, buyerToken, "decide_refund", id, decision)
	wantError(t, err, ErrForbidden)
	funds.afterExecute = func() { index.failWrites.Store(true) }
	_, err = a06RefundCall(f, sellerToken, "decide_refund", id, decision)
	wantError(t, err, ErrPending)
	if f.order(t).State != RefundPending || funds.executed != 1 {
		t.Fatal("lost ACK was finalized or operation was not retained")
	}
	saved := historicalCommand(f.shop, f.order(t))
	receipt := funds.known[saved.Key]
	delete(funds.known, saved.Key)
	_, err = a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	if f.order(t).State != RefundPending || funds.executed != 1 {
		t.Fatal("UNKNOWN GET executed or released a refund")
	}
	funds.known[saved.Key] = receipt
	_, err = a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	if f.order(t).State != Refunded || f.order(t).RefundRef != receipt.Reference || funds.executed != 1 || f.funds.execCalls.Load() != 0 {
		t.Fatal("receipt recovery changed identity or called native funds")
	}
	_, err = a06RefundCall(f, sellerToken, "decide_refund", id, decision)
	must(t, err)
	if funds.executed != 1 {
		t.Fatal("completed approval executed again")
	}
}
func TestHistoricalFullRefundDoesNotAcceptPartialOrUnnegotiatedPort(t *testing.T) {
	f, funds, _ := historicalSetup(t)
	before := f.image(t)
	in := a06RefundRequest()
	in["mode"] = "amount"
	in["amount_quota"] = 50
	_, err := a06RefundCall(f, buyerToken, "request_refund", "", in)
	wantError(t, err, ErrInvalid)
	if f.image(t) != before || funds.executed != 0 {
		t.Fatal("partial request changed historical order")
	}
	var errNew error
	f.s, errNew = New(&a06RefundIndex{memoryRepo: f.repo, shop: f.shop}, f.auth, f.funds)
	must(t, errNew)
	_, err = a06RefundCall(f, buyerToken, "request_refund", "", a06RefundRequest())
	wantError(t, err, ErrPaymentUnavailable)
	if f.image(t) != before {
		t.Fatal("missing negotiated bridge created pending")
	}
}
