// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"errors"
	"time"
)

func partialRefundProof(tx Tx, shop string, o Order) error {
	if o.Legacy == nil || o.Imported != nil {
		return ErrUnavailable
	}
	if o.Legacy.PaymentMethod == "free" && o.Total.Unit == "credits" && o.Total.Currency == "" && o.Total.Amount == 0 && o.Quantity > 0 && o.PaymentRef == "" && o.SettlementRef == "" {
		return nil
	}
	// External payments require their original provider basis and refund receipt.
	// A wallet top-up receipt or caller-supplied price is never that evidence.
	return legacyRefundProof(tx, shop, o)
}

func partialRefundDTO(o Order, r PartialRefund) map[string]any {
	state := r.State
	if state == "pending" {
		state = "reconciliation_required"
	}
	quota, minor := int64(0), int64(0)
	if o.Total.Unit == "credits" {
		quota = r.Spec.Amount
	} else {
		minor = r.Spec.Amount
	}
	return map[string]any{"id": r.Spec.ID, "order_id": o.ID, "request_key": r.Spec.RequestKey,
		"mode": r.Spec.Mode, "quantity": r.Spec.Quantity, "retired_quantity": r.Spec.RetiredQuantity, "stock_ids": r.StockIDs, "amount_quota": quota,
		"amount_minor": minor, "currency": o.Total.Currency, "reason": r.Reason, "status": state,
		"requested_by": r.Spec.RequestedBy.ID, "requested_role": r.Spec.Role, "decision_by": r.DecisionBy,
		"decision_reason": r.DecisionReason, "retained_fee_quota": int64(0), "created_at": r.CreatedAt,
		"decided_at": r.DecidedAt, "completed_at": r.CompletedAt}
}

func (s *Service) partialRefundView(o Order) (map[string]any, error) {
	u, err := partialRefundTotals(o)
	if err != nil {
		return nil, err
	}
	refunds := []map[string]any{}
	if o.LegacyRefund != nil {
		old := o
		if old.LegacyRefund.State != "" {
			old.RefundState = old.LegacyRefund.State
		} else if old.LegacyRefund.Decision == "reject" {
			old.RefundState = "rejected"
		}
		refunds = append(refunds, legacyRefundDTO(old))
	}
	completedQuantity := int64(0)
	pending := false
	for _, r := range o.PartialRefunds {
		refunds = append(refunds, partialRefundDTO(o, r))
		if r.State == "completed" {
			completedQuantity += r.Spec.RetiredQuantity
		}
		if r.State == "pending" {
			pending = true
		}
	}
	items := []map[string]any{}
	for _, id := range o.StockItems {
		if !u.items[id] {
			items = append(items, map[string]any{"id": id, "stock_item_id": id})
		}
	}
	return map[string]any{"supports_provider_sync": false, "provider_reconciliation_pending": pending,
		"delivery_template": o.Legacy.DeliveryTemplate, "order_id": o.ID, "product_title": o.ProductTitle, "variant_name": o.Title,
		"payment_method": o.Legacy.PaymentMethod, "currency": o.Total.Currency, "principal_quota": o.Total.Amount,
		"refunded_quota": u.completed, "reserved_quota": u.reserved, "remaining_quota": o.Total.Amount - u.completed - u.reserved,
		"quantity": o.Quantity, "refunded_quantity": completedQuantity, "max_quantity": o.Quantity - u.retired, "eligible_items": items,
		"supports_quantity": s.partialRefundAvailable(o), "supports_amount": o.Total.Amount > 0 && s.partialRefundAvailable(o),
		"native_basis_verified": false, "refunds": refunds}, nil
}

func (s *Service) cancelLegacyRefund(ctx context.Context, token string, actor Account, row LocatedOrder, request LegacyRequest) (any, error) {
	if actor != row.Order.Buyer {
		return nil, ErrForbidden
	}
	if _, err := s.check(ctx, token, actor, Spend); err != nil {
		return nil, err
	}
	var in struct{}
	if legacyDecode(request.Body, &in) != nil {
		return nil, ErrInvalid
	}
	var o Order
	err := s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		if err := tx.Get("orders", row.Order.ID, &o); err != nil {
			return err
		}
		if validateOrderCommand(row.ShopID, o, orderMoneyCommand(row.ShopID, row.Order, "pay")) != nil {
			return ErrConflict
		}
		if err := legacyRefundProof(tx, row.ShopID, o); err != nil {
			return err
		}
		if o.LegacyRefund == nil || o.LegacyRefund.ID != request.ChildID {
			return ErrNotFound
		}
		if o.LegacyRefund.State == "cancelled" {
			return nil
		}
		if o.RefundState != "requested" || o.State == RefundPending {
			return ErrConflict
		}
		var op operation
		if err := tx.Get("money_operations", ID("store-money-v1", row.ShopID, o.ID, "refund"), &op); err == nil {
			return ErrConflict
		} else if !errors.Is(err, ErrNotFound) {
			return err
		}
		o.LegacyRefund.State = "cancelled"
		o.RefundState = "cancelled"
		return tx.Put("orders", o.ID, o)
	})
	if err != nil {
		return nil, err
	}
	return legacyRefundDTO(o), nil
}

func refreshPartialRefundState(o *Order) {
	o.RefundState = ""
	for _, r := range o.PartialRefunds {
		if r.State == "requested" || r.State == "pending" {
			o.RefundState = "requested"
		}
	}
}

func (s *Service) partialRefundAPI(ctx context.Context, token string, actor Account, row LocatedOrder, request LegacyRequest) (any, error) {
	o := row.Order
	permission := Spend
	if request.Action == "decide_refund" || request.Action == "proactive_refund" {
		if actor != o.Seller {
			return nil, ErrForbidden
		}
		permission = Manage
	} else if request.Action != "refunds" && actor != o.Buyer {
		return nil, ErrForbidden
	}
	if request.Action != "refunds" {
		if _, err := s.check(ctx, token, actor, permission); err != nil {
			return nil, err
		}
	}
	approve := false
	selected := ""
	err := s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		if err := tx.Get("orders", o.ID, &o); err != nil {
			return err
		}
		if validateOrderCommand(row.ShopID, o, orderMoneyCommand(row.ShopID, row.Order, "pay")) != nil {
			return ErrConflict
		}
		if err := partialRefundProof(tx, row.ShopID, o); err != nil {
			return err
		}
		if request.Action == "refunds" {
			return nil
		}
		switch request.Action {
		case "request_refund", "proactive_refund":
			var in legacyRefundInput
			if legacyDecode(request.Body, &in) != nil {
				return ErrInvalid
			}
			role := "buyer"
			if request.Action == "proactive_refund" {
				role = "seller"
				approve = true
			}
			selected = partialRefundID(row.ShopID, o, actor, role, in.RequestKey)
			if i := partialRefundIndex(o, selected); i >= 0 {
				if o.PartialRefunds[i].InputHash != ID(in) {
					return ErrConflict
				}
				return nil
			}
			if !s.partialRefundAvailable(o) {
				return ErrUnavailable
			}
			if o.State != Paid && o.State != Fulfilled && o.State != Settled || o.ActiveRefundID != "" {
				return ErrConflict
			}
			if o.LegacyRefund != nil && o.RefundState == "requested" {
				return ErrConflict
			}
			// A full remainder is meaningful only after earlier reserved requests
			// settle. Never silently shrink Core's original full-remainder command.
			if in.Mode == "full" {
				for _, r := range o.PartialRefunds {
					if r.State == "requested" || r.State == "pending" {
						return ErrPending
					}
				}
			}
			r, err := planPartialRefund(row.ShopID, o, actor, role, in)
			if err != nil {
				return err
			}
			r.CreatedAt = time.Now().Unix()
			if approve {
				r.Decision = "approve"
				r.DecisionBy = actor.ID
				r.DecidedAt = r.CreatedAt
			}
			o.PartialRefunds = append(o.PartialRefunds, r)
		case "cancel_refund":
			var in struct{}
			if legacyDecode(request.Body, &in) != nil {
				return ErrInvalid
			}
			i := partialRefundIndex(o, request.ChildID)
			if i < 0 {
				return ErrNotFound
			}
			r := &o.PartialRefunds[i]
			selected = r.Spec.ID
			if r.Spec.RequestedBy != actor || r.Spec.Role != "buyer" {
				return ErrForbidden
			}
			if r.State == "cancelled" {
				return nil
			}
			// Saving the command is the dispatch fence. Cancellation and this
			// fence serialize under the same shop lock, including after approval.
			if r.State != "requested" || r.Command != nil || o.ActiveRefundID == selected {
				return ErrConflict
			}
			r.State = "cancelled"
		case "decide_refund":
			var in struct {
				Decision string `json:"decision"`
				Reason   string `json:"reason"`
			}
			if legacyDecode(request.Body, &in) != nil || (in.Decision != "approve" && in.Decision != "reject") || !legacyRefundText(in.Reason, false) {
				return ErrInvalid
			}
			i := partialRefundIndex(o, request.ChildID)
			if i < 0 {
				return ErrNotFound
			}
			r := &o.PartialRefunds[i]
			selected = r.Spec.ID
			approve = in.Decision == "approve"
			if r.DecisionBy != 0 {
				if r.DecisionBy != actor.ID || r.DecisionReason != in.Reason || r.Decision != in.Decision {
					return ErrConflict
				}
				return nil
			}
			if r.State != "requested" || o.ActiveRefundID != "" || o.State != Paid && o.State != Fulfilled && o.State != Settled {
				return ErrConflict
			}
			if approve && !s.partialRefundAvailable(o) {
				return ErrPaymentUnavailable
			}
			r.Decision = in.Decision
			r.DecisionBy = actor.ID
			r.DecisionReason = in.Reason
			r.DecidedAt = time.Now().Unix()
			if !approve {
				r.State = "rejected"
			}
		default:
			return ErrInvalid
		}
		refreshPartialRefundState(&o)
		return tx.Put("orders", o.ID, o)
	})
	if err != nil {
		return nil, err
	}
	if request.Action == "refunds" {
		if o.State == RefundPending {
			var recovered Order
			recovered, err = s.reconcileOrderMoney(ctx, row.ShopID, o.ID)
			if err == nil {
				o = recovered
			} else if errors.Is(err, ErrConflict) {
				// A recovered negative receipt is final data. Read back the
				// committed row; never infer rejection from an error alone.
				if readErr := s.repo.Within(ctx, row.ShopID, func(tx Tx) error { return tx.Get("orders", o.ID, &o) }); readErr != nil {
					return nil, readErr
				}
				if o.State == RefundPending {
					return nil, err
				}
			} else if !errors.Is(err, ErrPending) && !errors.Is(err, ErrPaymentUnavailable) {
				return nil, err
			}
		}
		return s.partialRefundView(o)
	}
	if approve {
		if o.State == RefundPending {
			var recovered Order
			recovered, err = s.reconcileOrderMoney(ctx, row.ShopID, o.ID)
			if err == nil {
				o = recovered
			} else if !errors.Is(err, ErrPending) {
				return nil, err
			}
		}
		// Only an explicit repeat of the same seller decision may retry its
		// original immutable Core key after UNKNOWN (e.g. rolled-back liquidity).
		// GET and workers never execute; no new refund identity is generated.
		i := partialRefundIndex(o, selected)
		if i < 0 {
			return nil, ErrConflict
		}
		if o.PartialRefunds[i].State != "completed" && o.PartialRefunds[i].State != "rejected" {
			o, err = s.executePartialRefund(ctx, token, row.ShopID, o.ID, selected)
			if err != nil {
				return nil, err
			}
		}
	}
	i := partialRefundIndex(o, selected)
	if i < 0 {
		return nil, ErrConflict
	}
	return partialRefundDTO(o, o.PartialRefunds[i]), nil
}
