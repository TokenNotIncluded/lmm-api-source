// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"errors"
	"sort"
	"time"
)

type partialRefundFunds interface{ PartialRefundAvailable() bool }

func (s *Service) partialRefundAvailable(o Order) bool {
	if o.Total.Amount == 0 {
		return o.Legacy != nil && o.Legacy.PaymentMethod == "free"
	}
	f, ok := s.funds.(partialRefundFunds)
	return ok && !Missing(f) && f.PartialRefundAvailable()
}

func partialRefundIndex(o Order, id string) int {
	for i := range o.PartialRefunds {
		if o.PartialRefunds[i].Spec.ID == id {
			return i
		}
	}
	return -1
}

func partialRefundCommand(shop string, o Order, r PartialRefund) MoneyCommand {
	c := orderMoneyCommand(shop, o, "refund")
	c.Key = partialMoneyKey(shop, o.ID, r.Spec.ID)
	spec := r.Spec
	c.Refund = &spec
	return c
}

func validatePartialCommand(shop string, o Order, c MoneyCommand) error {
	if c.Refund == nil || c.Kind != "refund" {
		return ErrConflict
	}
	i := partialRefundIndex(o, c.Refund.ID)
	if i < 0 {
		return ErrConflict
	}
	r := o.PartialRefunds[i]
	expected := partialRefundCommand(shop, o, r)
	if r.Command != nil {
		expected.SettlementRef = r.Command.SettlementRef
	}
	if ID(c) != ID(expected) {
		return ErrConflict
	}
	return nil
}

func validatePendingPartial(shop string, o Order, op operation) error {
	if op.Command.Refund == nil {
		return ErrConflict
	}
	if op.Done || validateOrderCommand(shop, o, op.Command) != nil ||
		o.State != RefundPending || o.ActiveRefundID != op.Command.Refund.ID {
		return ErrConflict
	}
	i := partialRefundIndex(o, o.ActiveRefundID)
	r := o.PartialRefunds[i]
	if r.State != "pending" || r.Before != op.Before || r.Command == nil || ID(*r.Command) != ID(op.Command) ||
		(r.Before != Paid && r.Before != Fulfilled && r.Before != Settled) || o.Delivered != (r.Before == Fulfilled || r.Before == Settled) {
		return ErrConflict
	}
	_, err := partialRefundTotals(o)
	return err
}

// Only the authenticated seller approval calls this method. A read/recovery
// path can only query a saved command and finish its exact existing receipt.
func (s *Service) executePartialRefund(ctx context.Context, user, shop, order, id string) (Order, error) {
	var out Order
	var op operation
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		if err := tx.Get("orders", order, &out); err != nil {
			return err
		}
		i := partialRefundIndex(out, id)
		if i < 0 {
			return ErrNotFound
		}
		r := &out.PartialRefunds[i]
		if r.State == "completed" {
			return nil
		}
		if r.State != "requested" && r.State != "pending" || r.DecisionBy == 0 {
			return ErrConflict
		}
		if !s.partialRefundAvailable(out) {
			return ErrPaymentUnavailable
		}
		if out.ActiveRefundID != "" && out.ActiveRefundID != id {
			return ErrPending
		}
		if r.Command == nil {
			if out.State != Paid && out.State != Fulfilled && out.State != Settled {
				return ErrConflict
			}
			// Requests may have been approved in another order. Derive terminal
			// pickup retirement from COMPLETED refunds immediately before the
			// first dispatch, then freeze it in this command forever. The user's
			// amount/quantity/input identity and original payment never change.
			completed := int64(0)
			for _, prior := range out.PartialRefunds {
				if prior.State == "completed" {
					completed += prior.Spec.Amount
				}
			}
			if out.Total.Amount > 0 && r.Spec.Amount == out.Total.Amount-completed {
				remaining, err := remainingDeliveryQuantity(out)
				if err != nil {
					return err
				}
				r.Spec.RetiredQuantity = remaining
				r.StockIDs = remainingStockItems(out)
				sort.Strings(r.StockIDs)
				r.Spec.ItemsDigest = ID(r.StockIDs)
			}
			if len(r.StockIDs) > 0 {
				selection := out
				selection.StockItems = append([]string{}, r.StockIDs...)
				state := stockReserved
				if out.Delivered {
					state = stockDelivered
				}
				// from==to is a read-only ownership/state check before dispatch.
				if err := moveStock(tx, selection, state, state); err != nil {
					return err
				}
			}
			r.Before = out.State
			command := partialRefundCommand(shop, out, *r)
			r.Command = &command
			r.State = "pending"
		}
		out.ActiveRefundID = id
		out.State = RefundPending
		key := partialMoneyKey(shop, order, id)
		if err := tx.Get("money_operations", key, &op); err == nil {
			if op.Done || ID(op.Command) != ID(*r.Command) {
				return ErrConflict
			}
		} else if errors.Is(err, ErrNotFound) {
			op = operation{Command: *r.Command, Before: r.Before}
		} else {
			return err
		}
		if err := validatePendingPartial(shop, out, op); err != nil {
			return err
		}
		if err := tx.Put("orders", order, out); err != nil {
			return err
		}
		return tx.Put("money_operations", key, op)
	})
	if err != nil {
		return out, err
	}
	if op.Command.Key == "" {
		return out, nil
	}
	receipt := Receipt{Key: op.Command.Key, CommandHash: ID(op.Command), Reference: "free:" + op.Command.Key, Outcome: "succeeded"}
	if out.Total.Amount > 0 {
		bounded, cancel := context.WithTimeout(ctx, 2*time.Second)
		receipt, err = s.funds.Execute(bounded, user, op.Command)
		cancel()
		if err != nil {
			return out, ErrPending
		}
	}
	return s.finishPartialRefund(ctx, shop, order, op, receipt)
}

func (s *Service) finishPartialRefund(ctx context.Context, shop, order string, op operation, receipt Receipt) (Order, error) {
	var out Order
	if op.Command.Refund == nil || !validMoneyReceipt(op.Command, receipt) {
		return out, ErrPending
	}
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		if err := tx.Get("orders", order, &out); err != nil {
			return err
		}
		var current operation
		if err := tx.Get("money_operations", op.Command.Key, &current); err != nil {
			return err
		}
		if ID(current.Command) != ID(op.Command) || current.Before != op.Before || validateOrderCommand(shop, out, current.Command) != nil {
			return ErrConflict
		}
		if current.Done {
			if current.Receipt != receipt {
				return ErrConflict
			}
			return nil
		}
		if err := validatePendingPartial(shop, out, current); err != nil {
			return err
		}
		i := partialRefundIndex(out, current.Command.Refund.ID)
		r := &out.PartialRefunds[i]
		out.State = r.Before
		out.ActiveRefundID = ""
		if receipt.Outcome == "rejected" {
			r.State = "rejected"
		} else {
			if err := finishPartialStock(tx, shop, &out, *r); err != nil {
				return err
			}
			r = &out.PartialRefunds[i]
			r.State = "completed"
			r.CompletedAt = time.Now().Unix()
			u, err := partialRefundTotals(out)
			if err != nil {
				return err
			}
			remaining, err := remainingDeliveryQuantity(out)
			if err != nil {
				return err
			}
			if u.completed == out.Total.Amount && out.Total.Amount > 0 || remaining == 0 && out.Total.Amount == 0 {
				out.State = Refunded
				out.RefundRef = receipt.Reference
			}
		}
		r = &out.PartialRefunds[i]
		r.Receipt = &receipt
		refreshPartialRefundState(&out)
		if out.State == Refunded {
			out.RefundState = "completed"
		}
		current.Done = true
		current.Receipt = receipt
		if err := tx.Put("orders", order, out); err != nil {
			return err
		}
		return tx.Put("money_operations", current.Command.Key, current)
	})
	if err != nil {
		if errors.Is(err, ErrConflict) {
			return out, err
		}
		return out, ErrPending
	}
	if receipt.Outcome == "rejected" {
		return out, ErrConflict
	}
	return out, nil
}

func finishPartialStock(tx Tx, shop string, o *Order, r PartialRefund) error {
	qty := r.Spec.RetiredQuantity
	if qty < 0 || qty > o.Quantity {
		return ErrConflict
	}
	if qty == 0 {
		return nil
	}
	if len(o.StockItems) > 0 && int64(len(r.StockIDs)) != qty {
		return ErrConflict
	}
	if !o.Delivered {
		var v Variant
		if err := tx.Get("variants", o.VariantID, &v); err != nil {
			return err
		}
		if err := v.release(qty); err != nil {
			return err
		}
		if err := tx.Put("variants", v.ID, v); err != nil {
			return err
		}
	}
	// Delivered secrets are retired and never restocked or reissued. Before
	// delivery only the refunded selection is released, retaining all others.
	if len(r.StockIDs) > 0 {
		selection := *o
		selection.StockItems = append([]string{}, r.StockIDs...)
		from, to := stockReserved, stockAvailable
		if o.Delivered {
			from, to = stockDelivered, stockRefunded
		}
		if err := moveStock(tx, selection, from, to); err != nil {
			return err
		}
	}
	return nil
}

func remainingDeliveryQuantity(o Order) (int64, error) {
	quantity := o.Quantity
	for _, r := range o.PartialRefunds {
		if r.State == "completed" {
			if r.Spec.RetiredQuantity < 0 || r.Spec.RetiredQuantity > quantity {
				return 0, ErrConflict
			}
			quantity -= r.Spec.RetiredQuantity
		}
	}
	return quantity, nil
}

func remainingStockItems(o Order) []string {
	retired := map[string]bool{}
	for _, r := range o.PartialRefunds {
		if r.State == "completed" {
			for _, id := range r.StockIDs {
				retired[id] = true
			}
		}
	}
	ids := []string{}
	for _, id := range o.StockItems {
		if !retired[id] {
			ids = append(ids, id)
		}
	}
	return ids
}
