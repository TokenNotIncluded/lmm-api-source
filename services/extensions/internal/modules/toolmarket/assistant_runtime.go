// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"strconv"
)

// CoreAssistantMarket is a closed host port. Its reserve requires the original
// persisted, paid Core tool operation; client_id alone never grants authority.
type CoreAssistantMarket interface {
	ReserveAssistantCredits(context.Context, string, string, LegacyCreditReserve) (LegacyCreditCall, error)
	ReadAssistantMarketCatalogue(context.Context, string, AssistantMarketCatalogueRequest) (AssistantMarketCatalogueReceipt, error)
}
type AssistantMarketCatalogueRequest struct {
	Owner     Principal `json:"owner"`
	ServiceID string    `json:"service_id,omitempty"`
	VersionID string    `json:"version_id,omitempty"`
	ToolIDs   []string  `json:"tool_ids,omitempty"`
	GrantID   string    `json:"grant_id,omitempty"`
}
type AssistantMarketCatalogueReceipt struct {
	Owner             Principal                  `json:"owner"`
	PolicyRevision    string                     `json:"policy_revision"`
	AllowedServiceIDs []string                   `json:"allowed_service_ids"`
	Consents          []CoreLegacyConsentReceipt `json:"consents"`
	CheckedAtUnix     int64                      `json:"checked_at_unix"`
}

// AssistantCatalogue does not write a grant or install a tool. It joins the
// current Core operator policy with this exact Session's current consent rows.
func (m *Module) AssistantCatalogue(ctx context.Context, credential, account string, in AssistantMarketCatalogueRequest) (AssistantMarketCatalogueReceipt, error) {
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return AssistantMarketCatalogueReceipt{}, err
	}
	owner := Principal{User: strconv.FormatInt(user, 10), Account: account}
	if owner.User != owner.Account || (in.Owner != (Principal{}) && in.Owner != owner) || len(in.ToolIDs) > 2 ||
		(in.ServiceID == "" && (in.VersionID != "" || len(in.ToolIDs) != 0 || in.GrantID != "")) ||
		(in.ServiceID != "" && (!validID.MatchString(in.ServiceID) || !validID.MatchString(in.VersionID))) ||
		(in.GrantID != "" && (!validID.MatchString(in.GrantID) || len(in.ToolIDs) != 1)) {
		return AssistantMarketCatalogueReceipt{}, ErrInvalid
	}
	tools := map[string]bool{}
	for _, tool := range in.ToolIDs {
		if !validID.MatchString(tool) || tools[tool] {
			return AssistantMarketCatalogueReceipt{}, ErrInvalid
		}
		tools[tool] = true
	}
	in.Owner = owner
	port, ok := m.authority.(CoreAssistantMarket)
	if !ok || missingInterface(port) {
		return AssistantMarketCatalogueReceipt{}, ErrUnavailable
	}
	out, err := port.ReadAssistantMarketCatalogue(ctx, credential, in)
	if err != nil {
		return AssistantMarketCatalogueReceipt{}, err
	}
	if out.Owner != owner || !legacyClientValid(out.PolicyRevision) || out.AllowedServiceIDs == nil ||
		len(out.AllowedServiceIDs) > 128 || out.Consents == nil || len(out.Consents) > 20 ||
		out.CheckedAtUnix <= 0 || out.CheckedAtUnix > m.now().Unix()+60 {
		return AssistantMarketCatalogueReceipt{}, ErrDenied
	}
	allowed := map[string]bool{}
	for _, service := range out.AllowedServiceIDs {
		if !validID.MatchString(service) || allowed[service] {
			return AssistantMarketCatalogueReceipt{}, ErrDenied
		}
		allowed[service] = true
	}
	if in.ServiceID != "" && !allowed[in.ServiceID] {
		return AssistantMarketCatalogueReceipt{}, ErrDenied
	}
	if in.ServiceID == "" && len(out.Consents) != 0 {
		return AssistantMarketCatalogueReceipt{}, ErrDenied
	}
	if in.GrantID != "" && len(out.Consents) > 1 {
		return AssistantMarketCatalogueReceipt{}, ErrDenied
	}
	seen := map[string]bool{}
	perTool := map[string]int{}
	for _, r := range out.Consents {
		perTool[r.ToolID]++
		if r.Owner != owner || !validID.MatchString(r.ID) || seen[r.ID] || !validID.MatchString(r.OperationID) ||
			r.ClientID != "builtin-assistant" || !tools[r.ToolID] || r.VersionID != in.VersionID ||
			(in.GrantID != "" && r.ID != in.GrantID) || !validID.MatchString(r.QuoteID) || !legacyRulesHash(r.RulesHash) ||
			!legacyQuotaValid(r.FrozenPriceQuota) || !legacyQuotaValid(r.MaxPriceQuota) || r.FrozenPriceQuota > r.MaxPriceQuota ||
			!legacyReadbackCounters(r.MaxTotalQuota, r.ReservedQuota, r.SpentQuota) || r.MaxCalls < 1 || r.MaxCalls > 100 ||
			!legacyReadbackCounters(r.MaxCalls, r.ReservedCalls, r.SuccessfulCalls) || r.CreatedAt <= 0 ||
			r.CreatedAt > out.CheckedAtUnix || r.ExpiresAt <= out.CheckedAtUnix || r.RevokedAt != 0 || perTool[r.ToolID] > 10 {
			return AssistantMarketCatalogueReceipt{}, ErrDenied
		}
		seen[r.ID] = true
	}
	if current, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read"); err != nil || current != user {
		return AssistantMarketCatalogueReceipt{}, ErrDenied
	}
	return out, nil
}

// ExecuteAssistantRemote is called only by the compiled Agent adapter. Neither
// public JSON nor the ordinary market endpoint accepts an operation marker.
func (m *Module) ExecuteAssistantRemote(ctx context.Context, credential, account, operation string, in LegacyExecuteInput) (LegacyExecution, error) {
	if operation == "" || operation != in.RequestKey || in.ClientID != "builtin-assistant" {
		return LegacyExecution{}, ErrInvalid
	}
	return m.legacyExecute(ctx, credential, account, in, false, operation)
}

func (m *Module) mirrorAssistantConsent(ctx context.Context, credential, account string, user int64, in LegacyExecuteInput) error {
	var service string
	if err := m.store.view(func(d database) error {
		if _, e := legacyLiveTool(&d, user, in.ToolID, in.VersionID); e != nil {
			return e
		}
		service = d.Web.Versions[in.VersionID].Version.ServiceID
		return nil
	}); err != nil {
		return err
	}
	out, err := m.AssistantCatalogue(ctx, credential, account, AssistantMarketCatalogueRequest{ServiceID: service, VersionID: in.VersionID, ToolIDs: []string{in.ToolID}, GrantID: in.GrantID})
	if err != nil {
		return err
	}
	if len(out.Consents) != 1 {
		return ErrConsent
	}
	row := out.Consents[0]
	return m.store.update(func(d *database) error {
		if err := ctx.Err(); err != nil {
			return err
		}
		if _, err := legacyLiveTool(d, user, in.ToolID, in.VersionID); err != nil {
			return err
		}
		if d.Web.Versions[in.VersionID].Version.ServiceID != service {
			return ErrConflict
		}
		if err := legacyMirrorCoreConsents(d, user, []CoreLegacyConsentReceipt{row}); err != nil {
			return err
		}
		key := digest([]any{user, "builtin-assistant", in.ToolID})
		if prior, ok := d.Web.Installations[key]; ok && (prior.UserID != user || prior.ClientID != "builtin-assistant" || prior.ToolID != in.ToolID) {
			return ErrConflict
		}
		if _, ok := d.Web.Installations[key]; !ok && len(d.Web.Installations) >= 50000 {
			return ErrLimit
		}
		d.Web.Installations[key] = WebInstallation{UserID: user, ClientID: "builtin-assistant", ToolID: in.ToolID, VersionID: in.VersionID, CreatedAt: row.CreatedAt}
		return nil
	})
}

func reserveLegacyExecution(ctx context.Context, port CoreLegacyCredits, credential string, r LegacyExecution) (LegacyCreditCall, error) {
	in := LegacyCreditReserve{Owner: r.Owner, ClientID: r.Input.ClientID, RequestKey: r.Input.RequestKey, QuoteID: r.QuoteID, GrantID: r.Input.GrantID, ToolID: r.Input.ToolID, VersionID: r.Input.VersionID, InputDigest: r.InputDigest, ResolveBy: r.ResolveBy}
	if r.AssistantToolOperationID != "" {
		if r.Input.ClientID != "builtin-assistant" || r.Input.RequestKey != r.AssistantToolOperationID {
			return LegacyCreditCall{}, ErrDenied
		}
		assistant, ok := port.(CoreAssistantMarket)
		if !ok || missingInterface(assistant) {
			return LegacyCreditCall{}, ErrUnavailable
		}
		return assistant.ReserveAssistantCredits(ctx, credential, r.AssistantToolOperationID, in)
	}
	if r.Input.ClientID == "builtin-assistant" || r.Input.ClientID == "assistant" {
		return LegacyCreditCall{}, ErrDenied
	}
	return port.ReserveLegacyCredits(ctx, credential, in)
}
