// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"
	"encoding/json"
	"sort"
	"time"
)

// Wire fields and terminal states follow official waffo-go/v2 v2.1.0.
// Global has its own original request identity; it is not Pancake's refund ticket.
type waffoRefund struct {
	RequestID        string `json:"refundRequestId"`
	MerchantRefundID string `json:"merchantRefundOrderId"`
	Transaction      string `json:"acquiringOrderId"`
	RefundID         string `json:"acquiringRefundOrderId"`
	OrderID          string `json:"origPaymentRequestId"`
	Amount           string `json:"refundAmount"`
	Status           string `json:"refundStatus"`
	Currency         string `json:"userCurrency"`
	ExtendInfo       string `json:"extendInfo"`
}

type waffoRefundBinding struct {
	Hold   string `json:"lmm_hold_receipt_id"`
	Refund string `json:"lmm_refund_id"`
}

func waffoRefundType(status string) (string, error) {
	switch status {
	case "REFUND_IN_PROGRESS":
		return RefundPending, nil
	case "ORDER_PARTIALLY_REFUNDED", "ORDER_FULLY_REFUNDED":
		return RefundSucceeded, nil
	case "ORDER_REFUND_FAILED":
		return RefundFailed, nil
	default:
		return "", ErrUnsupported
	}
}

func (w *Waffo) refundEvent(v waffoRefund) (Event, error) {
	kind, err := waffoRefundType(v.Status)
	if err != nil {
		return Event{}, err
	}
	amount, err := minorDecimal(v.Amount)
	if err != nil || !validAmount(amount) || v.Currency != w.cfg.Channel.Currency || !validTopupOrderID(v.OrderID) || !validID(v.RequestID) || !validID(v.Transaction) || !validID(v.RefundID) {
		return Event{}, ErrConflict
	}
	// Unrelated external metadata is not a local refund identity. Only our
	// complete, immutable dispatch binding may select an existing Go hold.
	var binding waffoRefundBinding
	var fields map[string]json.RawMessage
	if jsonObject([]byte(v.ExtendInfo), &fields) == nil && fields != nil {
		_, hasHold := fields["lmm_hold_receipt_id"]
		_, hasRefund := fields["lmm_refund_id"]
		if hasHold || hasRefund {
			if jsonObject([]byte(v.ExtendInfo), &binding) != nil || !validID(binding.Hold) || !validKey(binding.Refund) || v.MerchantRefundID != binding.Refund || v.RequestID != stableKey("submit", binding.Refund) {
				return Event{}, ErrConflict
			}
		}
	}
	channel := w.cfg.Channel
	return Event{OrderID: v.OrderID, Type: kind, Currency: v.Currency, AmountMinor: amount, RefundID: v.RefundID, LocalRefundID: binding.Refund,
		Evidence: Evidence{Provider: channel.Provider, Merchant: channel.Merchant, Environment: channel.Environment, Transaction: v.Transaction, EventID: stableKey(v.RefundID, "REFUND_NOTIFICATION", v.Status)}}, nil
}

func (w *Waffo) Refund(ctx context.Context, o Order, r Refund, key string) (RefundResult, error) {
	if !providerOrderMatches(o, w.cfg.Channel) || !validID(o.PaymentID) || !validKey(r.ID) || !validID(r.HoldReceipt) || !validAmount(r.AmountMinor) || r.AmountMinor > o.Intent.AmountMinor || key != stableKey("submit", r.ID) {
		return RefundResult{}, refundBeforeDispatch{ErrInvalid}
	}
	// Confirm the configured merchant's original transaction before sending
	// the irreversible provider write. This observation never posts money.
	payment, err := w.api(ctx, "order/inquiry", map[string]string{"paymentRequestId": o.ID})
	if err != nil {
		return RefundResult{}, refundBeforeDispatch{err}
	}
	var paid waffoPayment
	if jsonObject(payment, &paid) != nil {
		return RefundResult{}, refundBeforeDispatch{ErrConflict}
	}
	event, err := w.payment(paid)
	if err != nil || event.OrderID != o.ID || event.Evidence.Transaction != o.PaymentID || event.AmountMinor != o.Intent.AmountMinor {
		return RefundResult{}, refundBeforeDispatch{ErrConflict}
	}
	binding, err := json.Marshal(waffoRefundBinding{Hold: r.HoldReceipt, Refund: r.ID})
	if err != nil {
		return RefundResult{}, refundBeforeDispatch{ErrInvalid}
	}
	raw, err := w.api(ctx, "order/refund", map[string]string{
		"refundRequestId": key, "merchantRefundOrderId": r.ID, "acquiringOrderId": o.PaymentID, "merchantId": w.cfg.Channel.Merchant,
		"requestedAt": time.Now().UTC().Format("2006-01-02T15:04:05.000Z"), "refundAmount": decimalMinor(r.AmountMinor),
		"refundReason": "Requested account top-up refund", "refundNotifyUrl": w.cfg.NotifyURL, "extendInfo": string(binding),
	})
	if err != nil {
		return RefundResult{}, err
	}
	var response waffoRefund
	if jsonObject(raw, &response) != nil || response.RequestID != key || response.MerchantRefundID != r.ID || response.Transaction != o.PaymentID || !validID(response.RefundID) {
		return RefundResult{}, ErrConflict
	}
	amount, err := minorDecimal(response.Amount)
	if err != nil || amount != r.AmountMinor {
		return RefundResult{}, ErrConflict
	}
	if _, err := waffoRefundType(response.Status); err != nil {
		return RefundResult{}, err
	}
	// Neither an API success nor a terminal status releases/commits the Core
	// hold. The independent signed provider proof gate owns that transition.
	return RefundResult{ID: response.RefundID, Status: response.Status}, nil
}

func (w *Waffo) lookupRefunds(ctx context.Context, o Order) ([]Event, error) {
	ids := make([]string, 0, len(o.Refunds))
	for id, r := range o.Refunds {
		if r.Dispatch != "" && r.State != "committed" && r.State != "released" && r.State != "rejected" {
			ids = append(ids, id)
		}
	}
	sort.Strings(ids)
	events := make([]Event, 0, len(ids))
	for _, id := range ids {
		r := o.Refunds[id]
		key := stableKey("submit", r.ID)
		raw, err := w.api(ctx, "refund/inquiry", map[string]string{"refundRequestId": key})
		if err != nil {
			return nil, err
		}
		var response waffoRefund
		if jsonObject(raw, &response) != nil {
			return nil, ErrConflict
		}
		event, err := w.refundEvent(response)
		if err != nil || response.RequestID != key || event.OrderID != o.ID || event.Evidence.Transaction != o.PaymentID || event.AmountMinor != r.AmountMinor || event.LocalRefundID != r.ID || r.ProviderID != "" && r.ProviderID != event.RefundID {
			return nil, ErrConflict
		}
		event.Evidence.Payload = raw
		apiEvidence(&event, event.RefundID, event.Type)
		events = append(events, event)
	}
	return events, nil
}
