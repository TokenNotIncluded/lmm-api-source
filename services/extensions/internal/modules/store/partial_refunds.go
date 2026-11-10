// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"math"
	"math/big"
	"sort"
)

// RefundSpec extends only an explicitly negotiated refund command. Total and
// OrderSnapshot still describe the ORIGINAL payment, never a smaller order.
// Amount is in that original price's integer unit, not a new conversion quote.
type RefundSpec struct {
	ID              string  `json:"id"`
	RequestKey      string  `json:"request_key"`
	RequestedBy     Account `json:"requested_by"`
	Role            string  `json:"role"`
	Mode            string  `json:"mode"`
	Quantity        int64   `json:"quantity"`
	RetiredQuantity int64   `json:"retired_quantity"`
	Amount          int64   `json:"amount"`
	ItemsDigest     string  `json:"items_digest"`
}

// PartialRefund is an order-scoped, durable request. A failed acknowledgement
// retains this identity and its frozen command; it is never a new refund.
type PartialRefund struct {
	Spec           RefundSpec    `json:"spec"`
	InputHash      string        `json:"input_hash"`
	Reason         string        `json:"reason"`
	StockIDs       []string      `json:"stock_ids"`
	State          string        `json:"state"`
	Before         string        `json:"before,omitempty"`
	DecisionBy     int64         `json:"decision_by,omitempty"`
	Decision       string        `json:"decision,omitempty"`
	DecisionReason string        `json:"decision_reason,omitempty"`
	CreatedAt      int64         `json:"created_at"`
	DecidedAt      int64         `json:"decided_at,omitempty"`
	CompletedAt    int64         `json:"completed_at,omitempty"`
	Command        *MoneyCommand `json:"command,omitempty"`
	Receipt        *Receipt      `json:"receipt,omitempty"`
}

type partialRefundUsage struct {
	completed, reserved      int64
	quantity, quantityAmount int64
	retired                  int64
	items                    map[string]bool
}

func partialRefundActive(state string) bool {
	return state == "requested" || state == "pending" || state == "completed"
}

func partialRefundID(shop string, o Order, actor Account, role, key string) string {
	return ID("store-refund-v2", shop, o.ID, actor, role, key)
}

func partialMoneyKey(shop, order, refund string) string {
	return ID("store-refund-money-v1", shop, order, refund)
}

// Checked cumulative integer allocation preserves the final remainder without
// floating point or a change to the original unit. Inputs are nonnegative.
func refundFloor(total, quantity, originalQuantity int64) (int64, error) {
	if total < 0 || quantity < 0 || originalQuantity <= 0 || quantity > originalQuantity {
		return 0, ErrInvalid
	}
	n := new(big.Int).Mul(big.NewInt(total), big.NewInt(quantity))
	n.Quo(n, big.NewInt(originalQuantity))
	if !n.IsInt64() {
		return 0, ErrConflict
	}
	return n.Int64(), nil
}

func partialRefundTotals(o Order) (partialRefundUsage, error) {
	u := partialRefundUsage{items: map[string]bool{}}
	if o.Quantity <= 0 || !o.Total.valid() || o.Total.Amount < 0 {
		return u, ErrConflict
	}
	seen := map[string]bool{}
	for _, r := range o.PartialRefunds {
		if seen[r.Spec.ID] || r.Spec.Amount < 0 || r.Spec.Quantity < 0 || r.Spec.RetiredQuantity < 0 {
			return u, ErrConflict
		}
		seen[r.Spec.ID] = true
		if !partialRefundActive(r.State) {
			if r.State != "rejected" && r.State != "cancelled" {
				return u, ErrConflict
			}
			continue
		}
		if r.Spec.Amount > o.Total.Amount-u.completed-u.reserved || r.Spec.RetiredQuantity > o.Quantity-u.retired {
			return u, ErrConflict
		}
		if r.State == "completed" {
			u.completed += r.Spec.Amount
		} else {
			u.reserved += r.Spec.Amount
		}
		u.retired += r.Spec.RetiredQuantity
		if r.Spec.Mode == "quantity" {
			if r.Spec.Quantity > o.Quantity-u.quantity || r.Spec.Amount > math.MaxInt64-u.quantityAmount {
				return u, ErrConflict
			}
			u.quantity += r.Spec.Quantity
			u.quantityAmount += r.Spec.Amount
		}
		for _, id := range r.StockIDs {
			if u.items[id] {
				return u, ErrConflict
			}
			u.items[id] = true
		}
	}
	return u, nil
}

// planPartialRefund runs under the existing cross-process shop lock. Requested
// and unknown refunds consume caps just like completed refunds. Core repeats
// the financial/quantity checks in its own original-order transaction.
func planPartialRefund(shop string, o Order, actor Account, role string, in legacyRefundInput) (PartialRefund, error) {
	var out PartialRefund
	if !legacyRefundKey(in.RequestKey) || !legacyRefundText(in.Reason, true) ||
		in.Quantity < 0 || in.AmountQuota < 0 || in.AmountMinor < 0 || len(in.StockIDs) > 1000 ||
		o.Imported != nil || o.State == Refunded || o.State == Cancelled ||
		(role != "buyer" && role != "seller") || (role == "buyer" && actor != o.Buyer) ||
		(role == "seller" && actor != o.Seller) {
		return out, ErrInvalid
	}
	u, err := partialRefundTotals(o)
	if err != nil {
		return out, err
	}
	left := o.Total.Amount - u.completed - u.reserved
	if left == 0 && o.Total.Amount > 0 || u.retired == o.Quantity {
		return out, ErrConflict
	}
	ids := append([]string{}, in.StockIDs...)
	sort.Strings(ids)
	eligible := []string{}
	for _, id := range o.StockItems {
		if !u.items[id] {
			eligible = append(eligible, id)
		}
	}
	quantity, amount := int64(0), int64(0)
	switch in.Mode {
	case "full":
		if in.Quantity != 0 || len(ids) != 0 || in.AmountQuota != 0 || in.AmountMinor != 0 {
			return out, ErrInvalid
		}
		quantity, amount = o.Quantity-u.retired, left
		ids = append([]string{}, eligible...)
	case "quantity":
		if in.Quantity < 1 || in.Quantity > 1000 || in.Quantity > o.Quantity-u.retired ||
			in.AmountQuota != 0 || in.AmountMinor != 0 {
			return out, ErrInvalid
		}
		quantity = in.Quantity
		cumulative, e := refundFloor(o.Total.Amount, u.quantity+quantity, o.Quantity)
		if e != nil || cumulative < u.quantityAmount {
			return out, ErrConflict
		}
		amount = cumulative - u.quantityAmount
		if len(o.StockItems) == 0 {
			if len(ids) != 0 {
				return out, ErrInvalid
			}
		} else if len(ids) == 0 {
			if int64(len(eligible)) < quantity {
				return out, ErrConflict
			}
			ids = append([]string{}, eligible[:int(quantity)]...)
		} else {
			if int64(len(ids)) != quantity {
				return out, ErrInvalid
			}
			available := map[string]bool{}
			for _, id := range eligible {
				available[id] = true
			}
			for i, id := range ids {
				if !available[id] || i > 0 && ids[i-1] == id {
					return out, ErrConflict
				}
			}
		}
	case "amount":
		if in.Quantity != 0 || len(ids) != 0 || (in.AmountQuota > 0) == (in.AmountMinor > 0) {
			return out, ErrInvalid
		}
		if o.Total.Unit == "credits" && in.AmountMinor == 0 {
			amount = in.AmountQuota
		} else if o.Total.Unit == "" && in.AmountQuota == 0 {
			amount = in.AmountMinor
		} else {
			return out, ErrInvalid
		}
	default:
		return out, ErrInvalid
	}
	if amount > left || amount < 0 || amount == 0 && o.Total.Amount > 0 {
		return out, ErrConflict
	}
	// Exhausting the remaining principal retires every remaining delivery right.
	// Pending requests already own their selections and are never stolen here.
	retired := quantity
	if o.Total.Amount > 0 && amount == left && u.reserved == 0 {
		retired = o.Quantity - u.retired
		ids = append([]string{}, eligible...)
	}
	sort.Strings(ids)
	out = PartialRefund{Spec: RefundSpec{ID: partialRefundID(shop, o, actor, role, in.RequestKey),
		RequestKey: in.RequestKey, RequestedBy: actor, Role: role, Mode: in.Mode,
		Quantity: quantity, RetiredQuantity: retired, Amount: amount, ItemsDigest: ID(ids)},
		InputHash: ID(in), Reason: in.Reason, StockIDs: ids, State: "requested"}
	return out, nil
}
