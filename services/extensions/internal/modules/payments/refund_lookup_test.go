// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"
	"testing"
)

type refundLookupAdapter struct {
	*fakeAdapter
	queries int
}

func (a *refundLookupAdapter) Lookup(ctx context.Context, order Order) ([]Event, error) {
	a.queries++
	return a.fakeAdapter.Lookup(ctx, order)
}

func TestRefundWorkerQueriesOriginalUnknownDispatchAfterRestart(t *testing.T) {
	h := setup(t)
	o := h.paid()
	h.a.refundErr = context.DeadlineExceeded
	r, err := h.s.RequestRefund(context.Background(), "user", o.ID, "refund_lookup_fixture_001", 300)
	if err != nil {
		t.Fatal(err)
	}
	h.work()
	if h.a.refundCalls.Load() != 1 || h.get(o.ID).Refunds[r.ID].Dispatch != DispatchUnknown {
		t.Fatal("lost response did not retain the original dispatch")
	}
	lookup := &refundLookupAdapter{fakeAdapter: h.a}
	restarted, err := New(h.m, h.c, lookup)
	if err != nil {
		t.Fatal(err)
	}
	restarted.now = h.s.now
	h.s = restarted
	e := h.event(o, "evt_refund_lookup", RefundSucceeded, 300)
	e.RefundID, e.LocalRefundID = "re_original_unknown", r.ID
	observation := e
	observation.Evidence.Source = "api"
	observation.Evidence.Signature = nil
	h.a.observations = []Event{observation}
	h.advance()
	h.work()
	h.work()
	h.work()
	current := h.get(o.ID)
	if lookup.queries != 1 || h.a.refundCalls.Load() != 1 || current.Refunds[r.ID].HoldReceipt != r.HoldReceipt || h.c.count("commit") != 0 || h.c.count("release") != 0 {
		t.Fatal("query retried the provider write or settled an unsigned result")
	}
	if current.Jobs[stableKey("refund-lookup", r.ID)].State != "pending" {
		t.Fatal("unsigned observation stopped recovery")
	}
	h.send(e)
	h.work()
	if h.get(o.ID).Refunds[r.ID].State != "committed" || h.c.count("commit") != 1 {
		t.Fatal("signed proof did not settle the original refund")
	}
	h.advance()
	h.work()
	if lookup.queries != 1 || h.a.refundCalls.Load() != 1 || h.get(o.ID).Jobs[stableKey("refund-lookup", r.ID)].State != "done" {
		t.Fatal("final refund continued provider polling")
	}
}
