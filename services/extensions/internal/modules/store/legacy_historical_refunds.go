// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/hex"
	"errors"
	"time"
)

// This is a separate negotiated command. Existing native MoneyCommand bytes,
// payment refs and hashes are unchanged. The Core grant, not this body, grants
// authority over the original source principal and parties.
type LegacyRefundCommand struct {
	Key             string  `json:"key"`
	Kind            string  `json:"kind"`
	ShopID          string  `json:"shop_id"`
	OrderID         string  `json:"order_id"`
	SourceRowSHA256 string  `json:"source_row_sha256"`
	Buyer           Account `json:"buyer"`
	Seller          Account `json:"seller"`
	Credits         int64   `json:"credits"`
	Quantity        int64   `json:"quantity"`
}

func legacySourceHash(s string) bool {
	if len(s) != 64 {
		return false
	}
	b, err := hex.DecodeString(s)
	return err == nil && hex.EncodeToString(b) == s
}
func (c LegacyRefundCommand) Valid() bool {
	return c.Kind == "legacy_refund" && legacySourceHash(c.ShopID) && legacySourceHash(c.OrderID) && legacySourceHash(c.SourceRowSHA256) &&
		c.Key == ID("store-legacy-refund-v1", c.ShopID, c.OrderID) && c.Buyer.Kind == "personal" && c.Seller.Kind == "personal" &&
		c.Buyer.Valid() && c.Seller.Valid() && c.Credits > 0 && c.Credits <= legacySafeInteger && c.Quantity > 0
}

type legacyHistoricalFunds interface {
	LegacyRefundAvailable() bool
	ExecuteLegacyRefund(context.Context, string, LegacyRefundCommand) (Receipt, error)
	ReconcileLegacyRefund(context.Context, LegacyRefundCommand) (Receipt, error)
}
type legacyHistoricalOperation struct {
	Command LegacyRefundCommand `json:"command"`
	Before  string              `json:"before"`
	Done    bool                `json:"done"`
	Receipt Receipt             `json:"receipt"`
}

// A source marker permits only historical display/application bookkeeping.
// It is deliberately NOT a native payment proof or a Core financial grant.
func legacyHistoricalSource(o Order) error {
	if o.Imported == nil || o.Imported.Source != "go-v0.2.118" || !legacySourceHash(o.Imported.Snapshot) ||
		o.Legacy == nil || o.Legacy.PaymentMethod != "balance" || o.PaymentRef != "" || o.SettlementRef != "" ||
		len(o.PartialRefunds) != 0 || o.ActiveRefundID != "" || o.Total.Unit != "credits" || o.Total.Currency != "" || o.Total.Amount <= 0 || o.Total.Amount > legacySafeInteger ||
		validateOrderCommand(ID("legacy-web-shop-v1", o.Seller), o, orderMoneyCommand(ID("legacy-web-shop-v1", o.Seller), o, "pay")) != nil {
		return ErrConflict
	}
	if o.State != Settled && o.State != RefundPending && o.State != Refunded {
		return ErrConflict
	}
	if o.State == Refunded && (o.LegacyRefund == nil || o.RefundState != "completed") {
		return ErrConflict
	}
	return nil
}
func legacyApplicationProof(tx Tx, shop string, o Order) error {
	if o.Imported != nil {
		return legacyHistoricalSource(o)
	}
	return legacyRefundProof(tx, shop, o)
}
func historicalCommand(shop string, o Order) LegacyRefundCommand {
	return LegacyRefundCommand{Key: ID("store-legacy-refund-v1", shop, o.ID), Kind: "legacy_refund", ShopID: shop, OrderID: o.ID,
		SourceRowSHA256: o.Imported.Snapshot, Buyer: o.Buyer, Seller: o.Seller, Credits: o.Total.Amount, Quantity: o.Quantity}
}
func (s *Service) historicalFunds() (legacyHistoricalFunds, error) {
	f, ok := s.funds.(legacyHistoricalFunds)
	if !ok || Missing(f) || !f.LegacyRefundAvailable() {
		return nil, ErrPaymentUnavailable
	}
	return f, nil
}
func (s *Service) historicalRefundView(ctx context.Context, shop string, o Order) (any, error) {
	if err := legacyHistoricalSource(o); err != nil {
		return nil, err
	}
	if o.State == RefundPending {
		recovered, err := s.reconcileHistoricalRefund(ctx, shop, o.ID)
		if err != nil && !errors.Is(err, ErrPending) {
			return nil, err
		}
		if err == nil {
			o = recovered
		}
	}
	return legacyRefundView(o), nil
}
func (s *Service) executeHistoricalRefund(ctx context.Context, token, shop, order string) (Order, error) {
	var out Order
	var op legacyHistoricalOperation
	funds, err := s.historicalFunds()
	if err != nil {
		return out, err
	}
	err = s.repo.Within(ctx, shop, func(tx Tx) error {
		if err := tx.Get("orders", order, &out); err != nil {
			return err
		}
		if err := legacyHistoricalSource(out); err != nil {
			return err
		}
		if out.LegacyRefund == nil || out.LegacyRefund.Decision != "approve" {
			return ErrConflict
		}
		command := historicalCommand(shop, out)
		if !command.Valid() {
			return ErrConflict
		}
		if err := tx.Get("commands", command.Key, &op); err == nil {
			if op.Command != command || op.Before != Settled || (!op.Done && (out.State != RefundPending || out.RefundState != "requested")) {
				return ErrConflict
			}
			return nil
		} else if !errors.Is(err, ErrNotFound) {
			return err
		}
		if out.State != Settled || out.RefundState != "requested" {
			return ErrConflict
		}
		op = legacyHistoricalOperation{Command: command, Before: out.State}
		out.State = RefundPending
		if err := tx.Put("orders", out.ID, out); err != nil {
			return err
		}
		return tx.Put("commands", command.Key, op)
	})
	if err != nil {
		return out, err
	}
	if op.Done {
		return s.finishHistoricalRefund(ctx, shop, order, op, op.Receipt)
	}
	bounded, cancel := context.WithTimeout(ctx, 2*time.Second)
	receipt, err := funds.ExecuteLegacyRefund(bounded, token, op.Command)
	cancel()
	if err != nil {
		return out, err
	}
	return s.finishHistoricalRefund(ctx, shop, order, op, receipt)
}
func (s *Service) reconcileHistoricalRefund(ctx context.Context, shop, order string) (Order, error) {
	var out Order
	var op legacyHistoricalOperation
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		if err := tx.Get("orders", order, &out); err != nil {
			return err
		}
		if err := legacyHistoricalSource(out); err != nil {
			return err
		}
		if out.State != RefundPending {
			return nil
		}
		command := historicalCommand(shop, out)
		if err := tx.Get("commands", command.Key, &op); err != nil {
			return err
		}
		if op.Command != command || op.Done || out.RefundState != "requested" || out.LegacyRefund == nil || out.LegacyRefund.Decision != "approve" {
			return ErrConflict
		}
		return nil
	})
	if err != nil || op.Command.Key == "" {
		return out, err
	}
	funds, err := s.historicalFunds()
	if err != nil {
		return out, err
	}
	bounded, cancel := context.WithTimeout(ctx, 2*time.Second)
	receipt, err := funds.ReconcileLegacyRefund(bounded, op.Command)
	cancel()
	if err != nil {
		return out, err
	}
	return s.finishHistoricalRefund(ctx, shop, order, op, receipt)
}
func (s *Service) finishHistoricalRefund(ctx context.Context, shop, order string, op legacyHistoricalOperation, r Receipt) (Order, error) {
	var out Order
	if r.Key != op.Command.Key || r.CommandHash != ID(op.Command) || r.Reference != "legacy-store-refund:"+op.Command.Key || r.Outcome != "succeeded" {
		return out, ErrPending
	}
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		if err := tx.Get("orders", order, &out); err != nil {
			return err
		}
		if err := legacyHistoricalSource(out); err != nil {
			return err
		}
		var current legacyHistoricalOperation
		if err := tx.Get("commands", op.Command.Key, &current); err != nil {
			return err
		}
		if current.Command != op.Command || current.Before != Settled || historicalCommand(shop, out) != op.Command {
			return ErrConflict
		}
		if current.Done {
			if current.Receipt != r || out.State != Refunded {
				return ErrConflict
			}
			return nil
		}
		if out.State != RefundPending || out.LegacyRefund == nil || out.LegacyRefund.Decision != "approve" || out.RefundState != "requested" {
			return ErrConflict
		}
		if len(out.StockItems) > 0 {
			if err := moveStock(tx, out, stockDelivered, stockRefunded); err != nil {
				return err
			}
		}
		out.State = Refunded
		out.RefundState = "completed"
		out.RefundRef = r.Reference
		out.LegacyRefund.CompletedAt = time.Now().Unix()
		current.Done = true
		current.Receipt = r
		if err := tx.Put("orders", out.ID, out); err != nil {
			return err
		}
		return tx.Put("commands", op.Command.Key, current)
	})
	if err != nil {
		if errors.Is(err, ErrConflict) {
			return out, err
		}
		return out, ErrPending
	}
	return out, nil
}
