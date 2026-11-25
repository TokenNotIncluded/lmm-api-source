// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"math"
	"time"
)

// ServiceTerms are supplied by the configured service, never by the ordinary
// store checkout. The shop transaction freezes these terms beside the sale.
type ServiceTerms struct {
	NonRefundable bool   `json:"non_refundable,omitempty"`
	Audience      string `json:"audience"`
	Service       string `json:"service"`
	Period        string `json:"period"`
	UnitsPerItem  int64  `json:"units_per_item"`
	UnitPrice     Price  `json:"unit_price"`
}
type ServiceEntitlement struct {
	Terms    ServiceTerms `json:"terms"`
	Dispatch string       `json:"dispatch,omitempty"`
}

func (s *Service) ServiceDeliveryAvailable() bool {
	if s == nil {
		return false
	}
	f, ok := s.funds.(deliveryFunds)
	return ok && !Missing(f) && f.DeliveryAvailable() && s.funds.Available()
}
func (s *Service) PurchaseServiceEntitlement(ctx context.Context, token string, p Purchase, terms ServiceTerms) (Order, error) {
	if _, err := time.Parse("2006-01", terms.Period); err != nil || !Text(terms.Service, 64) || len(terms.Audience) != 64 || terms.UnitsPerItem <= 0 || p.Quantity <= 0 || p.Quantity > math.MaxInt64/terms.UnitsPerItem || terms.UnitPrice.Unit != "credits" || terms.UnitPrice.Currency != "" || terms.UnitPrice.Amount <= 0 || p.Buyer.Kind != "personal" {
		return Order{}, ErrInvalid
	}
	return s.purchase(ctx, token, p, &terms)
}
func servicePassReference(shop string, o Order) string {
	return ID("settled-service-entitlement-v1", shop, o.ID, o.Buyer, o.VariantID, o.Quantity, o.Entitlement.Terms)
}

// With a verified paid order the buyer owns a fixed service voucher. Core
// captures the original reserve only after that voucher is durably delivered.
// A caller can redeem only their own voucher, from the creating service.
func (s *Service) ServiceOrder(ctx context.Context, token string, actor Account, shop, id, service string) (Order, error) {
	if _, err := s.check(ctx, token, actor, Read); err != nil {
		return Order{}, err
	}
	var o Order
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		if err := tx.Get("orders", id, &o); err != nil {
			return err
		}
		if !ownServiceOrder(o, actor, id, service) {
			return ErrNotFound
		}
		return nil
	})
	if err != nil {
		return Order{}, err
	}
	return s.reconcileOrderMoney(ctx, shop, id)
}
func ownServiceOrder(o Order, actor Account, id, service string) bool {
	return actor.Kind == "personal" && actor.Valid() && o.ID == id && o.Buyer == actor && o.CreatedBy == actor.ID && o.Imported == nil && o.Entitlement != nil && o.Entitlement.Terms.Service == service
}

// ClaimServiceDispatch fences a grant against refunds under the same shop
// transaction as money operations. Network I/O always happens after commit.
// A lost daemon ACK can only be retried with the exact persisted voucher.
func (s *Service) ClaimServiceDispatch(ctx context.Context, token string, actor Account, shop, id, service string) (Order, error) {
	if _, err := s.check(ctx, token, actor, Spend); err != nil {
		return Order{}, err
	}
	var o Order
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		if err := tx.Get("orders", id, &o); err != nil {
			return err
		}
		if !ownServiceOrder(o, actor, id, service) {
			return ErrNotFound
		}
		if o.State != Settled || !o.Delivered || o.Delivery != servicePassReference(shop, o) || o.UnitPrice != o.Entitlement.Terms.UnitPrice || o.Total.Amount <= 0 || len(o.PartialRefunds) > 0 {
			return ErrConflict
		}
		for _, kind := range []string{"pay", "settle"} {
			c := orderMoneyCommand(shop, o, kind)
			var op operation
			if err := tx.Get("money_operations", c.Key, &op); err != nil {
				return err
			}
			ref := o.PaymentRef
			if kind == "settle" {
				ref = o.SettlementRef
			}
			if !op.Done || validateOrderCommand(shop, o, op.Command) != nil || op.Command.Kind != kind || !validMoneyReceipt(c, op.Receipt) || op.Receipt.Outcome != "succeeded" || op.Receipt.Reference != ref || ref == "" {
				return ErrConflict
			}
		}
		switch o.Entitlement.Dispatch {
		case "granted", "unknown":
			return nil
		case "":
			if o.RefundState == "requested" || o.RefundRef != "" {
				return ErrConflict
			}
			o.Entitlement.Dispatch = "unknown"
			return tx.Put("orders", id, o)
		default:
			return ErrConflict
		}
	})
	if err != nil {
		return Order{}, err
	}
	return o, nil
}

// RecordServiceDispatch can finish only a fenced, unchanged voucher. A
// transport failure leaves the fence intact until the daemon confirms the
// original order. New refund executions wait while its outcome is unknown.
func (s *Service) RecordServiceDispatch(ctx context.Context, shop string, claimed Order, granted bool) error {
	if claimed.Entitlement == nil || claimed.Entitlement.Dispatch != "unknown" {
		return ErrConflict
	}
	return s.repo.Within(ctx, shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", claimed.ID, &o); err != nil {
			return err
		}
		if o.Entitlement == nil || o.SettlementRef != claimed.SettlementRef || servicePassReference(shop, o) != servicePassReference(shop, claimed) {
			return ErrConflict
		}
		if (o.Entitlement.Dispatch == "granted" && granted) || (o.Entitlement.Dispatch == "rejected" && !granted) {
			return nil
		}
		if o.Entitlement.Dispatch != "unknown" {
			return ErrConflict
		}
		if granted {
			o.Entitlement.Dispatch = "granted"
		} else {
			o.Entitlement.Dispatch = "rejected"
		}
		return tx.Put("orders", o.ID, o)
	})
}

// ServiceUnitPrice reads the actual variant. Purchase compares it again in the
// inventory transaction before reserving the item.
func (s *Service) ServiceUnitPrice(ctx context.Context, token string, buyer Account, shop, variant string) (Price, error) {
	if _, err := s.check(ctx, token, buyer, Read); err != nil {
		return Price{}, err
	}
	var v Variant
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		var merchant Shop
		if err := tx.Get("shops", shop, &merchant); err != nil {
			return err
		}
		if merchant.Owner == buyer {
			return ErrConflict
		}
		if err := tx.Get("variants", variant, &v); err != nil {
			return err
		}
		var product Product
		if err := tx.Get("products", v.ProductID, &product); err != nil {
			return err
		}
		if product.Legacy != nil || !product.Published || v.ID != variant || v.Price.Unit != "credits" || !v.Price.valid() || v.Price.Amount <= 0 {
			return ErrUnavailable
		}
		ok, err := productVisibleTo(LocatedProduct{Owner: merchant.Owner, Product: product}, buyer.ID)
		if err != nil {
			return err
		}
		if !ok {
			return ErrNotFound
		}
		return nil
	})
	if err != nil {
		return Price{}, err
	}
	return v.Price, nil
}
