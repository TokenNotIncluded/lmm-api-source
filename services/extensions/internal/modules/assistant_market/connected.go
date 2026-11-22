// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package assistant_market

import (
	"context"
	"encoding/json"
	"errors"
	"sort"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

// Listing and a selected-service page are separate bounded projections. Exact
// schemas and current grant counters are returned only for two selected tools.
func (c *catalogueExecutor) connected(ctx context.Context, in assistant.AgentInvocation, account string, values map[string]any) (any, error) {
	base, err := c.parent.market.AssistantCatalogue(ctx, in.Credential, account, toolmarket.AssistantMarketCatalogueRequest{})
	if err != nil {
		return nil, translate(err)
	}
	allowed := append([]string(nil), base.AllowedServiceIDs...)
	sort.Strings(allowed)
	service, _ := values["service_id"].(string)
	offset, toolOffset := 0, 0
	if n, ok := values["offset"].(json.Number); ok {
		v, _ := n.Int64()
		offset = int(v)
	}
	if n, ok := values["tool_offset"].(json.Number); ok {
		v, _ := n.Int64()
		toolOffset = int(v)
	}
	if service == "" {
		if toolOffset != 0 {
			return nil, access.ErrInvalid
		}
		end := offset + 10
		if end > len(allowed) {
			end = len(allowed)
		}
		if offset > end {
			offset = end
		}
		items := []map[string]any{}
		for _, id := range allowed[offset:end] {
			d, e := c.parent.market.PublicWebDetail(ctx, in.Credential, account, id)
			if errors.Is(e, toolmarket.ErrMissing) || errors.Is(e, toolmarket.ErrDenied) {
				continue
			}
			if e != nil {
				return nil, translate(e)
			}
			if d.Service.ID != id || d.Version.ServiceID != id || d.Version.ID != d.Service.LiveVersionID {
				return nil, access.ErrConflict
			}
			if d.Version.ExecutionType != "remote" || !d.Validated {
				continue
			}
			name, nameCut := clipped(d.Version.Name, 160)
			description, cut := clipped(d.Version.Description, 1200)
			items = append(items, map[string]any{"service_id": id, "version_id": d.Version.ID, "name": name, "name_truncated": nameCut, "description": description, "description_truncated": cut, "tool_count": len(d.Tools), "href": "/tool-market?service_id=" + id})
		}
		return map[string]any{"ok": true, "policy_revision": base.PolicyRevision, "services": items, "offset": offset, "limit": 10, "has_more": end < len(allowed), "next_offset": end, "select_service_for_tools": true}, nil
	}
	if offset != 0 {
		return nil, access.ErrInvalid
	}
	index := sort.SearchStrings(allowed, service)
	if index == len(allowed) || allowed[index] != service {
		return nil, access.ErrForbidden
	}
	d, err := c.parent.market.PublicWebDetail(ctx, in.Credential, account, service)
	if err != nil {
		return nil, translate(err)
	}
	if d.Service.ID != service || d.Version.ServiceID != service || d.Version.ID != d.Service.LiveVersionID || d.Version.ExecutionType != "remote" || !d.Validated {
		return nil, access.ErrConflict
	}
	tools := append([]toolmarket.WebTool(nil), d.Tools...)
	sort.Slice(tools, func(i, j int) bool { return tools[i].ToolID < tools[j].ToolID })
	for i, t := range tools {
		if t.ToolID == "" || t.VersionID != d.Version.ID || (i > 0 && tools[i-1].ToolID == t.ToolID) {
			return nil, access.ErrConflict
		}
	}
	end := toolOffset + 2
	if end > len(tools) {
		end = len(tools)
	}
	if toolOffset > end {
		toolOffset = end
	}
	selected := tools[toolOffset:end]
	ids := make([]string, 0, len(selected))
	for _, t := range selected {
		ids = append(ids, t.ToolID)
	}
	facts, err := c.parent.market.AssistantCatalogue(ctx, in.Credential, account, toolmarket.AssistantMarketCatalogueRequest{ServiceID: service, VersionID: d.Version.ID, ToolIDs: ids})
	if err != nil {
		return nil, translate(err)
	}
	if facts.PolicyRevision != base.PolicyRevision || facts.Owner != base.Owner {
		return nil, access.ErrConflict
	}
	items := []map[string]any{}
	for _, t := range selected {
		quote, e := c.parent.market.LegacyReadToolQuote(ctx, in.Credential, account, service, d.Version.ID, t.ToolID)
		if e != nil {
			return nil, translate(e)
		}
		price, e := toolmarket.LegacyQuotePricingDigest(t.LegacyPricing())
		if e != nil || quote.ID == "" || quote.ServiceID != service || quote.VersionID != d.Version.ID || quote.ToolID != t.ToolID || !validDigest(quote.RemoteDigest) || quote.PricingDigest != price {
			return nil, access.ErrConflict
		}
		description, cut := clipped(t.Description, 2000)
		name, nameCut := clipped(t.Name, 160)
		grants := []map[string]any{}
		for _, g := range facts.Consents {
			if g.ToolID != t.ToolID {
				continue
			}
			if g.VersionID != d.Version.ID || g.QuoteID != quote.ID || g.ClientID != ClientID || g.RulesHash != price {
				return nil, access.ErrConflict
			}
			grants = append(grants, map[string]any{"grant_id": g.ID, "max_price_quota": g.MaxPriceQuota, "max_total_quota": g.MaxTotalQuota, "reserved_quota": g.ReservedQuota, "spent_quota": g.SpentQuota, "remaining_quota": g.MaxTotalQuota - g.ReservedQuota - g.SpentQuota, "max_calls": g.MaxCalls, "reserved_calls": g.ReservedCalls, "successful_calls": g.SuccessfulCalls, "remaining_calls": g.MaxCalls - g.ReservedCalls - g.SuccessfulCalls, "expires_at": g.ExpiresAt})
		}
		item := map[string]any{"tool_id": t.ToolID, "version_id": t.VersionID, "name": name, "name_truncated": nameCut, "description": description, "description_truncated": cut, "resource_schema_sha256": quote.RemoteDigest, "price_sha256": price, "price_quota": t.PriceQuota, "billing_mode": t.BillingMode, "input_token_price_quota": t.InputTokenPriceQuota, "max_input_tokens": t.MaxInputTokens, "billing_rules": t.BillingRules, "grants": grants, "has_active_grant": len(grants) > 0, "schema_omitted": true, "schema_unavailable_reason": "schema_exceeds_bound", "confirmation_required_for_new_grant": true}
		if len(t.InputSchema) <= 8<<10 && json.Valid([]byte(t.InputSchema)) {
			item["input_schema"] = json.RawMessage(t.InputSchema)
			item["input_schema_json"] = t.InputSchema
			item["input_schema_sha256"] = digest([]byte(t.InputSchema))
			item["schema_omitted"] = false
			delete(item, "schema_unavailable_reason")
		} else if !json.Valid([]byte(t.InputSchema)) {
			return nil, access.ErrConflict
		}
		items = append(items, item)
	}
	return map[string]any{"ok": true, "policy_revision": facts.PolicyRevision, "service_id": service, "version_id": d.Version.ID, "tools": items, "tool_offset": toolOffset, "tool_limit": 2, "has_more_tools": end < len(tools), "next_tool_offset": end, "checked_at_unix": facts.CheckedAtUnix, "price_unit": "CREDIT", "credits_per_usd": int64(500000), "href": "/tool-market?service_id=" + service}, nil
}
