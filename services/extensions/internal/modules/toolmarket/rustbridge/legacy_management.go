// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

var (
	_ market.CoreLegacyConfig    = (*Adapter)(nil)
	_ market.CoreLegacyLifecycle = (*Adapter)(nil)
	_ market.CoreLegacyReports   = (*Adapter)(nil)
)

func (a *Adapter) ReadLegacyConfig(ctx context.Context) (market.LegacyMarketConfig, error) {
	var out market.LegacyMarketConfig
	err := a.requestVersion(ctx, 2, "config/read", "", struct{}{}, &out)
	return out, err
}
func (a *Adapter) UpdateLegacyConfig(ctx context.Context, credential string, in market.LegacyMarketConfigUpdate) (market.LegacyMarketConfigUpdateReceipt, error) {
	var out market.LegacyMarketConfigUpdateReceipt
	err := a.requestVersion(ctx, 2, "config/update", credential, in, &out)
	return out, err
}

func (a *Adapter) SetLegacyServiceLifecycle(ctx context.Context, credential string, in market.LegacyServiceLifecycleRequest) (market.LegacyServiceLifecycleReceipt, error) {
	var out market.LegacyServiceLifecycleReceipt
	err := a.requestVersion(ctx, 2, "services/lifecycle", credential, in, &out)
	return out, err
}

func (a *Adapter) CreateLegacyReport(ctx context.Context, credential string, in market.LegacyReportRequest) (market.LegacyCallReport, error) {
	var out market.LegacyCallReport
	err := a.requestVersion(ctx, 2, "reports/create", credential, in, &out)
	return out, err
}
func (a *Adapter) ReviewLegacyReport(ctx context.Context, credential string, in market.LegacyReviewReportRequest) (market.LegacyCallReport, error) {
	var out market.LegacyCallReport
	err := a.requestVersion(ctx, 2, "reports/review", credential, in, &out)
	return out, err
}
func (a *Adapter) ListLegacyReports(ctx context.Context, credential string, in market.LegacyReportListRequest) (market.LegacyReportPage, error) {
	var out market.LegacyReportPage
	err := a.requestWithSpec(ctx, requestSpec{major: 2, action: "reports/list", requestBytes: 32 << 10, responseBytes: 4 << 20}, credential, in, &out)
	return out, err
}
func (a *Adapter) ListLegacyIncome(ctx context.Context, credential string, in market.LegacyIncomeRequest) (market.LegacyIncomePage, error) {
	var out market.LegacyIncomePage
	err := a.requestVersion(ctx, 2, "income/list", credential, in, &out)
	return out, err
}
