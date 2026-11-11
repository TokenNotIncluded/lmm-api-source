package payments

import (
	"context"
	"testing"
)

type zeroCommitCore struct{ *fakeCore }

func (c *zeroCommitCore) Commit(ctx context.Context, id int64, key, hold string, e Evidence) (Receipt, error) {
	r, err := c.fakeCore.Commit(ctx, id, key, hold, e)
	if err != nil {
		return r, err
	}
	r.JournalID = 0
	c.receipts[key] = r
	return r, nil
}
func TestZeroCreditRefundCommitPreservesReceiptAndIdempotency(t *testing.T) {
	h := setup(t)
	order := h.paid()
	h.s.core = &zeroCommitCore{h.c}
	refund, err := h.s.RequestRefund(context.Background(), "user", order.ID, "zero_credit_refund_01", 1)
	if err != nil {
		t.Fatal(err)
	}
	h.work()
	order = h.get(order.ID)
	refund = order.Refunds[refund.ID]
	e := h.event(order, "zero_credit_refund_event", RefundSucceeded, 1)
	e.RefundID = refund.ProviderID
	e.LocalRefundID = refund.ID
	h.send(e)
	h.work()
	saved := h.get(order.ID)
	if saved.Refunds[refund.ID].State != "committed" || h.c.count("commit") != 1 {
		t.Fatal("zero-unit commit did not complete", saved.Refunds[refund.ID])
	}
	found := false
	for _, receipt := range h.c.receipts {
		if receipt.State == RefundCommitted {
			found = true
			if receipt.JournalID != 0 {
				t.Fatal("invented refund journal")
			}
		}
	}
	if !found {
		t.Fatal("missing zero-journal receipt")
	}
	h.send(e)
	h.work()
	if h.c.count("commit") != 1 {
		t.Fatal("duplicate proof committed twice")
	}
	for _, state := range []string{CreditApplied, RefundHeld, RefundReleased} {
		if receiptMatches(Receipt{ID: "receipt", IntentID: 1, Key: "key", RequestHash: make([]byte, 32), State: state}, 1, "key", state) {
			t.Fatal("journal-less credit/hold/release accepted", state)
		}
	}
	for _, journal := range []int64{-1, 0} {
		r := Receipt{ID: "receipt", IntentID: 1, Key: "key", RequestHash: make([]byte, 32), State: RefundCommitted, JournalID: journal}
		if receiptMatches(r, 1, "key", RefundCommitted) != (journal == 0) {
			t.Fatal("invalid commit journal boundary")
		}
	}
}
