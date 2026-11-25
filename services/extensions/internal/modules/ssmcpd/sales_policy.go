// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package ssmcpd

import (
	"context"
	"strconv"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type SalesPolicyInput struct {
	MonthlyLimitBytes string `json:"monthly_limit_bytes"`
	UnitCredits       string `json:"unit_credits"`
}

type SalesPolicy struct {
	MonthlyLimitBytes *string `json:"monthly_limit_bytes"`
	UnitCredits       string  `json:"unit_credits"`
	BytesPerUnit      string  `json:"bytes_per_unit"`
}

func (s *Service) SalesPolicy(ctx context.Context, in *SalesPolicyInput) (SalesPolicy, error) {
	if !s.CommerceConfigured() {
		return SalesPolicy{}, store.ErrPaymentUnavailable
	}
	c := s.commerce
	var update *store.ServiceSalesPolicy
	if in != nil {
		limit, err := strconv.ParseInt(in.MonthlyLimitBytes, 10, 64)
		if err != nil || limit < 0 || strconv.FormatInt(limit, 10) != in.MonthlyLimitBytes {
			return SalesPolicy{}, store.ErrInvalid
		}
		price, err := strconv.ParseInt(in.UnitCredits, 10, 64)
		if err != nil || price <= 0 || strconv.FormatInt(price, 10) != in.UnitCredits {
			return SalesPolicy{}, store.ErrInvalid
		}
		update = &store.ServiceSalesPolicy{MonthlyLimitBytes: &limit, UnitCredits: price}
	}
	saved, err := c.shop.ServiceSalesPolicy(ctx, c.cfg.ShopID, c.cfg.VariantID, trafficService, update)
	if err != nil {
		return SalesPolicy{}, err
	}
	out := SalesPolicy{UnitCredits: strconv.FormatInt(saved.UnitCredits, 10), BytesPerUnit: strconv.FormatInt(c.cfg.BytesPerUnit, 10)}
	if saved.MonthlyLimitBytes != nil {
		v := strconv.FormatInt(*saved.MonthlyLimitBytes, 10)
		out.MonthlyLimitBytes = &v
	}
	return out, nil
}
