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

func TestSignedRefundFailureSupersedesUnsignedSuccess(t *testing.T) {
	for _, historical := range []bool{false, true} {
		h := setup(t)
		o := h.paid()
		r, err := h.s.RequestRefund(context.Background(), "user", o.ID, "refund_observation_failure_001", 300)
		if err != nil {
			t.Fatal(err)
		}
		h.work()
		r = h.get(o.ID).Refunds[r.ID]
		e := h.event(o, "observed_refund_success", RefundSucceeded, 300)
		e.RefundID, e.LocalRefundID = r.ProviderID, r.ID
		e.Evidence.Source, e.Evidence.Signature = "api", nil
		if historical {
			failed := h.event(o, "signed_refund_failure", RefundFailed, 300)
			failed.RefundID, failed.LocalRefundID = r.ProviderID, r.ID
			h.send(failed)
			if err := h.m.FinishEvent(context.Background(), eventKey(failed), "done", "", h.s.now()); err != nil {
				t.Fatal(err)
			}
			if err := h.m.Mutate(context.Background(), o.ID, func(order *Order) error {
				old := order.Refunds[r.ID]
				old.ProviderState = RefundSucceeded
				order.Refunds[r.ID] = old
				order.Jobs["old_unsigned_commit"] = Job{Key: "old_unsigned_commit", Kind: "commit", RefundID: r.ID, State: "review", Evidence: e.Evidence}
				return nil
			}); err != nil {
				t.Fatal(err)
			}
		} else {
			h.a.observations = []Event{e}
			if err := h.s.Reconcile(context.Background(), o.ID); err != nil {
				t.Fatal(err)
			}
			h.work()
		}
		failed := h.event(o, "signed_refund_failure", RefundFailed, 300)
		failed.RefundID, failed.LocalRefundID = r.ProviderID, r.ID
		h.send(failed)
		h.work()
		got := h.get(o.ID)
		if got.Refunds[r.ID].State != "released" || got.Refunds[r.ID].ProviderState != RefundFailed || h.c.count("release") != 1 || h.c.count("commit") != 0 || got.State() != "credited" {
			t.Fatalf("unsigned success suppressed trusted failure (historical=%t): %+v", historical, got.Refunds[r.ID])
		}
	}
}
