// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"

	pancake "github.com/waffo-com/waffo-pancake-sdk-go"
)

// Automatic ticket submission remains disabled. An unsupported local call did
// not start provider IO; existing ambiguous dispatches remain UNKNOWN in the
// worker and must never be retried through this method.
func (*Pancake) Refund(context.Context, Order, Refund, string) (RefundResult, error) {
	return RefundResult{}, refundBeforeDispatch{ErrUnsupported}
}

func (p *Pancake) refundCallback(v pancake.TypedWebhookEvent[pancake.WebhookEventData], raw, signature []byte) (Event, error) {
	d, c := v.Data, p.cfg.Channel
	mode := pancake.EnvironmentTest
	if c.Environment == "live" {
		mode = pancake.EnvironmentProd
	}
	status, kind := "succeeded", RefundSucceeded
	if v.EventType == "refund.failed" {
		status, kind = "failed", RefundFailed
	}
	if v.Mode != mode || v.StoreID != p.cfg.StoreID || !validID(v.EventID) ||
		d.OrderMerchantExternalID == nil || !validTopupOrderID(*d.OrderMerchantExternalID) ||
		d.PaymentID == nil || !validID(*d.PaymentID) || d.Currency != c.Currency ||
		d.OrderMetadata["lmm_product_id"] != p.cfg.ProductID || d.OrderMetadata["lmm_merchant"] != c.Merchant ||
		d.RefundStatus == nil || *d.RefundStatus != status ||
		d.PaymentStatus != nil && *d.PaymentStatus != "succeeded" {
		return Event{}, ErrConflict
	}
	// SDK v0.19 webhook-guide: these are actual channel amounts. Neither the
	// list-price snapshot nor a requested ticket amount proves money movement.
	if d.RefundedAmount == nil || d.OriginalChargedAmount == nil {
		return Event{}, ErrEvidence
	}
	amount, err := minorDecimal(*d.RefundedAmount)
	if err != nil || !validAmount(amount) {
		return Event{}, ErrEvidence
	}
	original, err := minorDecimal(*d.OriginalChargedAmount)
	if err != nil || !validAmount(original) || amount > original {
		return Event{}, ErrConflict
	}
	// go-v0.2.118's waffoPancakeRefundEventID prioritizes EventID as the
	// financial identity. ID identifies delivery only. Require the business ID
	// here; falling back to a delivery ID could debit the same refund twice.
	// A merchant ticket reference is not authority to select a local hold.
	return Event{OrderID: *d.OrderMerchantExternalID, Type: kind, Currency: c.Currency,
		AmountMinor: amount, RefundID: v.EventID,
		Evidence: Evidence{Provider: c.Provider, Merchant: c.Merchant, Environment: c.Environment,
			Transaction: *d.PaymentID, EventID: v.ID, Payload: raw, Signature: signature, Source: "webhook"}}, nil
}
