// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"errors"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

var (
	_ market.CoreLegacyMarketOAuth        = (*Adapter)(nil)
	_ market.CoreLegacyAccess             = (*Adapter)(nil)
	_ market.CoreLegacyMarketMeta         = (*Adapter)(nil)
	_ market.CoreLegacyMarketOAuthClients = (*Adapter)(nil)
)

func (a *Adapter) AuthorizeLegacyMarketOAuth(ctx context.Context, in market.LegacyMarketOAuthRequest) (market.LegacyMarketPrincipal, error) {
	var out market.LegacyMarketPrincipal
	err := a.requestVersion(ctx, 1, "oauth-authorize", "", in, &out)
	if errors.Is(err, market.ErrDenied) {
		return market.LegacyMarketPrincipal{}, market.ErrLegacyMarketScope
	}
	return out, err
}

// Core remains the owner of legacy quota policies, ledger counters and personal
// connection tokens. These methods only carry the closed v2 access DTOs over the
// dedicated UDS with the original credential; they never fall back to v1 USD.
func (a *Adapter) CreateLegacyConsent(ctx context.Context, credential string, in market.CoreLegacyConsentRequest) (market.CoreLegacyConsentReceipt, error) {
	var out market.CoreLegacyConsentReceipt
	err := a.requestVersion(ctx, 2, "access/consent-create", credential, in, &out)
	return out, err
}
func (a *Adapter) RevokeLegacyConsent(ctx context.Context, credential string, in market.CoreLegacyIDRequest) (market.CoreLegacyRevocationReceipt, error) {
	var out market.CoreLegacyRevocationReceipt
	err := a.requestVersion(ctx, 2, "access/consent-revoke", credential, in, &out)
	return out, err
}
func (a *Adapter) SetLegacyBudget(ctx context.Context, credential string, in market.CoreLegacyBudgetRequest) (market.CoreLegacyBudgetReceipt, error) {
	var out market.CoreLegacyBudgetReceipt
	err := a.requestVersion(ctx, 2, "access/budget-set", credential, in, &out)
	return out, err
}
func (a *Adapter) CreateLegacyToken(ctx context.Context, credential string, in market.CoreLegacyTokenRequest) (market.CoreLegacyTokenReceipt, error) {
	var out market.CoreLegacyTokenReceipt
	err := a.requestVersion(ctx, 2, "access/token-create", credential, in, &out)
	return out, err
}
func (a *Adapter) RevokeLegacyToken(ctx context.Context, credential string, in market.CoreLegacyIDRequest) (market.CoreLegacyRevocationReceipt, error) {
	var out market.CoreLegacyRevocationReceipt
	err := a.requestVersion(ctx, 2, "access/token-revoke", credential, in, &out)
	return out, err
}
func (a *Adapter) ValidateLegacyToken(ctx context.Context, credential string, in market.CoreLegacyValidateRequest) (market.CoreLegacyTokenReceipt, error) {
	var out market.CoreLegacyTokenReceipt
	err := a.requestVersion(ctx, 2, "access/token-validate", credential, in, &out)
	return out, err
}
func (a *Adapter) DisconnectLegacyClient(ctx context.Context, credential string, in market.CoreLegacyClientRequest) (market.CoreLegacyDisconnectReceipt, error) {
	var out market.CoreLegacyDisconnectReceipt
	err := a.requestVersion(ctx, 2, "access/client-disconnect", credential, in, &out)
	return out, err
}

func (a *Adapter) ListLegacyConsents(ctx context.Context, credential string, in market.CoreLegacyListRequest) (market.CoreLegacyConsentList, error) {
	var out market.CoreLegacyConsentList
	err := a.requestVersion(ctx, 2, "access/consent-list", credential, in, &out)
	return out, err
}
func (a *Adapter) ListLegacyBudgets(ctx context.Context, credential string, in market.CoreLegacyListRequest) (market.CoreLegacyBudgetList, error) {
	var out market.CoreLegacyBudgetList
	err := a.requestVersion(ctx, 2, "access/budget-list", credential, in, &out)
	return out, err
}
func (a *Adapter) ListLegacyTokens(ctx context.Context, credential string, in market.CoreLegacyListRequest) (market.CoreLegacyTokenList, error) {
	var out market.CoreLegacyTokenList
	err := a.requestVersion(ctx, 2, "access/token-list", credential, in, &out)
	return out, err
}

func (a *Adapter) ReadLegacyMarketMeta(ctx context.Context, raw string) (market.LegacyMarketMetaView, error) {
	var out market.LegacyMarketMetaView
	err := a.requestVersion(ctx, 2, "meta/read", raw, struct{}{}, &out)
	return out, err
}
func (a *Adapter) ReadLegacyMarketMetaOwner(ctx context.Context, raw string, in market.LegacyMarketMetaTarget) (market.LegacyMarketMetaOwnerView, error) {
	var out market.LegacyMarketMetaOwnerView
	err := a.requestVersion(ctx, 2, "meta/owner-read", raw, in, &out)
	return out, err
}
func (a *Adapter) SetLegacyMarketMetaOwner(ctx context.Context, raw string, in market.LegacyMarketMetaSet) (market.LegacyMarketMetaOwnerView, error) {
	var out market.LegacyMarketMetaOwnerView
	err := a.requestVersion(ctx, 2, "meta/owner-set", raw, in, &out)
	return out, err
}
func (a *Adapter) AuthorizeLegacyMarketMeta(ctx context.Context, raw string, in market.LegacyMarketMetaAuthorize) (market.CoreLegacyConsentReceipt, error) {
	var out market.CoreLegacyConsentReceipt
	err := a.requestVersion(ctx, 2, "meta/authorize", raw, in, &out)
	return out, err
}
func (a *Adapter) TightenLegacyMarketClientBudget(ctx context.Context, raw string, in market.LegacyMarketMetaClientBudget) (market.CoreLegacyBudgetReceipt, error) {
	var out market.CoreLegacyBudgetReceipt
	err := a.requestVersion(ctx, 2, "meta/client-budget", raw, in, &out)
	return out, err
}
func (a *Adapter) TightenLegacyMarketToolBudget(ctx context.Context, raw string, in market.LegacyMarketMetaToolBudget) (market.CoreLegacyConsentReceipt, error) {
	var out market.CoreLegacyConsentReceipt
	err := a.requestVersion(ctx, 2, "meta/tool-budget", raw, in, &out)
	return out, err
}
func (a *Adapter) ReadLegacyMarketUsage(ctx context.Context, raw string, in market.LegacyMarketMetaPage) (market.LegacyMarketMetaUsage, error) {
	var out market.LegacyMarketMetaUsage
	err := a.requestVersion(ctx, 2, "meta/usage", raw, in, &out)
	return out, err
}
func (a *Adapter) ReadLegacyMarketCalls(ctx context.Context, raw string, in market.LegacyMarketMetaPage) (market.LegacyMarketMetaCalls, error) {
	var out market.LegacyMarketMetaCalls
	err := a.requestVersion(ctx, 2, "meta/calls", raw, in, &out)
	return out, err
}

func (a *Adapter) ListLegacyMarketOAuthClients(ctx context.Context, raw string, in market.LegacyMarketOAuthClientsRequest) (market.LegacyMarketOAuthClients, error) {
	var out market.LegacyMarketOAuthClients
	err := a.requestVersion(ctx, 2, "meta/oauth-clients", raw, in, &out)
	return out, err
}
