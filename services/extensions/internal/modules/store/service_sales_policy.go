// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/json"
	"errors"
)

// ServiceSalesPolicy applies to one service in the configured shop, across all buyers.
type ServiceSalesPolicy struct {
	MonthlyLimitBytes *int64
	UnitCredits       int64
}

type serviceLimit struct {
	Bytes int64 `json:"bytes"`
}

func serviceLimitFor(tx Tx, service string) (*int64, error) {
	var limit serviceLimit
	err := tx.Get("commands", ID("service-monthly-sale-limit-v1", service), &limit)
	if errors.Is(err, ErrNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if limit.Bytes < 0 {
		return nil, ErrInvalid
	}
	return &limit.Bytes, nil
}

// The authenticated plugin administrator manages its configured catalogue variant.
// Price and monthly capacity change under the lock also used to reserve an order.
func (s *Service) ServiceSalesPolicy(ctx context.Context, shop, variant, service string, update *ServiceSalesPolicy) (ServiceSalesPolicy, error) {
	var policy ServiceSalesPolicy
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		var v Variant
		if err := tx.Get("variants", variant, &v); err != nil {
			return err
		}
		var p Product
		if err := tx.Get("products", v.ProductID, &p); err != nil {
			return err
		}
		if p.Legacy != nil || v.ID != variant || v.Price.Unit != "credits" || v.Price.Currency != "" {
			return ErrConflict
		}
		if update != nil {
			if update.MonthlyLimitBytes == nil || *update.MonthlyLimitBytes < 0 || update.UnitCredits <= 0 {
				return ErrInvalid
			}
			v.Price.Amount = update.UnitCredits
			if !v.Price.valid() {
				return ErrInvalid
			}
			if err := tx.Put("variants", variant, v); err != nil {
				return err
			}
			if err := tx.Put("commands", ID("service-monthly-sale-limit-v1", service), serviceLimit{*update.MonthlyLimitBytes}); err != nil {
				return err
			}
		}
		limit, err := serviceLimitFor(tx, service)
		policy = ServiceSalesPolicy{limit, v.Price.Amount}
		return err
	})
	return policy, err
}

// Count existing reservations and sales, including orders created before the
// administrator enabled the limit. Cancellation releases its reservation.
func checkServiceSaleLimit(tx Tx, terms ServiceTerms, quantity int64) error {
	limit, err := serviceLimitFor(tx, terms.Service)
	if err != nil || limit == nil {
		return err
	}
	requested := quantity * terms.UnitsPerItem // validated by PurchaseServiceEntitlement
	if requested > *limit {
		return ErrSoldOut
	}
	budget := *limit - requested
	if reader, ok := tx.(interface {
		ServiceSalesWithin(string, string, int64) (bool, error)
	}); ok {
		within, err := reader.ServiceSalesWithin(terms.Service, terms.Period, budget)
		if err != nil {
			return err
		}
		if !within {
			return ErrSoldOut
		}
		return nil
	}
	after := ""
	for {
		rows, err := tx.List("orders", after, 100)
		if err != nil {
			return err
		}
		for _, row := range rows {
			var o Order
			if err := json.Unmarshal(row, &o); err != nil {
				return err
			}
			if o.ID <= after {
				return ErrUnavailable
			}
			after = o.ID
			if o.Entitlement == nil || o.State == Cancelled {
				continue
			}
			t := o.Entitlement.Terms
			if t.Service != terms.Service || t.Period != terms.Period {
				continue
			}
			if o.Quantity <= 0 || t.UnitsPerItem <= 0 {
				return ErrInvalid
			}
			if o.Quantity > budget/t.UnitsPerItem {
				return ErrSoldOut
			}
			budget -= o.Quantity * t.UnitsPerItem
		}
		if len(rows) < 100 {
			return nil
		}
	}
}

func (t *postgresTx) ServiceSalesWithin(service, period string, budget int64) (bool, error) {
	var within bool
	err := t.tx.QueryRowContext(t.ctx, `SELECT COALESCE(SUM((body->>'quantity')::numeric *
        (body#>>'{service_entitlement,terms,units_per_item}')::numeric),0) <= $4::numeric
        FROM lmm_store.orders WHERE shop_id=$1 AND body ? 'service_entitlement'
        AND body#>>'{service_entitlement,terms,service}'=$2
        AND body#>>'{service_entitlement,terms,period}'=$3 AND body->>'state'<>'cancelled'`,
		t.shop, service, period, budget).Scan(&within)
	return within, err
}
