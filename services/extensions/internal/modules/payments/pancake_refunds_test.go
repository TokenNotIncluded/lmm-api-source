// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"
)

func pancakeExternalRefund(t *testing.T, status, delivery string) string {
	t.Helper()
	var event map[string]any
	if err := json.Unmarshal([]byte(pancakePaid), &event); err != nil {
		t.Fatal(err)
	}
	event["id"], event["eventId"], event["eventType"] = delivery, "REF_business", "refund."+status
	d := event["data"].(map[string]any)
	d["refundStatus"], d["refundedAmount"], d["originalChargedAmount"] = status, "4.00", "10.00"
	d["refundTicketMerchantExternalId"] = "external_ticket_reference"
	raw, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	return string(raw)
}

func pancakeRefundAdapter() *Pancake {
	return &Pancake{cfg: PancakeConfig{Channel: Channel{ID: 4, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "USD"},
		StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl"}}
}

func TestPancakeExternalRefundRoutesExistingWalletLedger(t *testing.T) {
	for _, status := range []string{"succeeded", "failed"} {
		t.Run(status, func(t *testing.T) {
			ctx := context.Background()
			p := pancakeRefundAdapter()
			store, core := newMemory(), newCore(p.Channel())
			o := gatewayOrder(p.Channel())
			o.PaymentID, o.CreditState, o.CreditReceipt = "PAY_fixture", "confirmed", "original_credit_receipt"
			// The merchant's external ticket reference must not select or
			// release an existing local UNKNOWN hold, even if its text matches.
			hold := Refund{ID: "external_ticket_reference", AmountMinor: 100, HoldKey: "original_hold_key", HoldReceipt: "original_hold_receipt", State: "held", Dispatch: DispatchUnknown}
			o.Refunds[hold.ID] = hold
			if err := store.Insert(ctx, o); err != nil {
				t.Fatal(err)
			}
			for _, delivery := range []string{"delivery_first", "delivery_second", "delivery_first"} {
				// Recreate the service to exercise durable inbox/job identity.
				service, err := New(store, core, p)
				if err != nil {
					t.Fatal(err)
				}
				raw := pancakeExternalRefund(t, status, delivery)
				e, err := p.Verify(gatewayCallback(raw, "X-Waffo-Signature"), time.Now())
				if err != nil || e.RefundID != "REF_business" || e.Evidence.EventID != delivery || e.LocalRefundID != "" || e.AmountMinor != 400 || string(e.Evidence.Payload) != raw {
					t.Fatalf("lost provider identities/raw bytes: %+v %v", e, err)
				}
				if err = service.Receive(ctx, p.Channel().ID, gatewayCallback(raw, "X-Waffo-Signature")); err != nil {
					t.Fatal(err)
				}
				if err = service.Work(ctx, 100); err != nil {
					t.Fatal(err)
				}
			}
			last, err := store.Get(ctx, o.ID)
			if err != nil {
				t.Fatal(err)
			}
			want := 0
			if status == "succeeded" {
				want = 1
			}
			if core.count("external") != want || core.count("commit") != 0 || core.count("release") != 0 || last.Refunds[hold.ID] != hold || len(last.Refunds) != 2 {
				t.Fatalf("refund path/replay changed original hold: %+v, external=%d", last.Refunds, core.count("external"))
			}
		})
	}
}

func TestPancakeExternalRefundRequiresCoreProofAndActualAmounts(t *testing.T) {
	p := pancakeRefundAdapter()
	raw := pancakeExternalRefund(t, "succeeded", "delivery_untrusted")
	store, core := newMemory(), newCore(p.Channel())
	if err := store.Insert(context.Background(), gatewayOrder(p.Channel())); err != nil {
		t.Fatal(err)
	}
	service, err := New(store, callbackRejectProofCore{Core: core}, p)
	if err != nil {
		t.Fatal(err)
	}
	if err = service.Receive(context.Background(), p.Channel().ID, gatewayCallback(raw, "X-Waffo-Signature")); !errors.Is(err, ErrEvidence) || len(store.events) != 0 {
		t.Fatal("untrusted refund bypassed Core proof admission", err)
	}
	for _, pair := range [][2]string{
		{`"refundedAmount":"4.00"`, `"refundedAmount":null`},
		{`"refundedAmount":"4.00"`, `"refundedAmount":"0.00"`},
		{`"refundedAmount":"4.00"`, `"refundedAmount":"10.01"`},
		{`"originalChargedAmount":"10.00"`, `"originalChargedAmount":null`},
		{`"refundStatus":"succeeded"`, `"refundStatus":"failed"`},
		{`"eventId":"REF_business"`, `"eventId":""`},
		{`"currency":"USD"`, `"currency":"CNY"`},
		{`"lmm_merchant":"MER_0123456789abcdefghijkl"`, `"lmm_merchant":"other"`},
	} {
		wrong := strings.Replace(raw, pair[0], pair[1], 1)
		if wrong == raw {
			t.Fatal("invalid test mutation", pair)
		}
		if _, err = p.Verify(gatewayCallback(wrong, "X-Waffo-Signature"), time.Now()); err == nil {
			t.Fatal("invalid refund accepted", pair)
		}
	}
}

// Exercise the actual worker error classification, including a previously
// ambiguous send. These fixtures do not certify the provider refund protocol.
func TestPancakeUnsupportedRefundPreservesDispatchHistory(t *testing.T) {
	for _, ambiguous := range []bool{false, true} {
		name := "never_sent"
		if ambiguous {
			name = "unknown"
		}
		t.Run(name, func(t *testing.T) {
			ctx := context.Background()
			p := &Pancake{cfg: PancakeConfig{Channel: Channel{ID: 4, Provider: "waffo_pancake", Merchant: "MER_fixture", Environment: "test", Currency: "USD"}}}
			o := gatewayOrder(p.Channel())
			r := Refund{ID: stableKey("refund", o.ID, "original_request"), AmountMinor: 100,
				HoldKey: "original_hold_key", HoldReceipt: "original_hold_receipt", DispatchReceipt: "original_dispatch_receipt", State: "held"}
			key := stableKey("submit", r.ID)
			job := Job{Key: key, Kind: "refund_submit", RefundID: r.ID, State: "pending"}
			if ambiguous {
				r.Dispatch = DispatchUnknown
				job.Attempts = 1
				job.FirstAttempt = time.Now().Add(-time.Hour).UTC().Truncate(time.Second)
			}
			o.Refunds[r.ID], o.Jobs[key] = r, job
			store := newMemory()
			if err := store.Insert(ctx, o); err != nil {
				t.Fatal(err)
			}
			core := newCore(p.Channel())
			service, err := New(store, core, p)
			if err != nil {
				t.Fatal(err)
			}
			if err = service.Step(ctx, o.ID); err != nil {
				t.Fatal(err)
			}
			last, err := store.Get(ctx, o.ID)
			if err != nil {
				t.Fatal(err)
			}
			got, work := last.Refunds[r.ID], last.Jobs[key]
			if got != r || work.Key != key || work.State != "review" || core.count("commit") != 0 || core.count("release") != 0 {
				t.Fatalf("unsupported operation changed original hold/dispatch: refund=%+v job=%+v", got, work)
			}
			if ambiguous {
				if work.Attempts != 2 || !work.FirstAttempt.Equal(job.FirstAttempt) {
					t.Fatal("unknown dispatch lost its original attempt history")
				}
			} else if work.Attempts != 0 || !work.FirstAttempt.IsZero() {
				t.Fatal("local unsupported result was recorded as provider IO")
			}
		})
	}
}
