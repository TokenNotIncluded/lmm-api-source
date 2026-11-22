// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package assistant_market

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

func connectedFixture(t *testing.T) (*Executor, *marketFixture, assistant.AgentInvocation) {
	e, f, in := setup(t)
	owner := toolmarket.Principal{User: "7", Account: "7"}
	grant := toolmarket.CoreLegacyConsentReceipt{CoreLegacyConsentRequest: toolmarket.CoreLegacyConsentRequest{CoreLegacyOperation: toolmarket.CoreLegacyOperation{Owner: owner, OperationID: "browser-consent"}, ClientID: ClientID, ToolID: "remote-tool", VersionID: "version-one", MaxPriceQuota: 20, MaxTotalQuota: 100, MaxCalls: 4, ExpiresAt: 200}, ID: "grant-one", QuoteID: f.quote.ID, RulesHash: f.quote.PricingDigest, CreatedAt: 50, ReservedQuota: 10, SpentQuota: 20, ReservedCalls: 1, SuccessfulCalls: 1}
	f.marketScope = toolmarket.AssistantMarketCatalogueReceipt{Owner: owner, PolicyRevision: "policy-v1", AllowedServiceIDs: []string{"service-one"}, Consents: []toolmarket.CoreLegacyConsentReceipt{grant}, CheckedAtUnix: 100}
	return e, f, in
}
func TestConnectedCatalogueReadsActualPolicySchemaAndGrantCounters(t *testing.T) {
	e, f, in := connectedFixture(t)
	executor, selected := readInvocation(t, e, in, "get_connected_market_tools", `{"service_id":"service-one"}`)
	got, err := executor.Execute(context.Background(), selected)
	if err != nil || got.State != assistant.AgentToolSucceeded {
		t.Fatalf("%+v %v", got, err)
	}
	var value struct {
		Tools []struct {
			Input  string `json:"input_schema_json"`
			SHA    string `json:"input_schema_sha256"`
			Grants []struct {
				Remaining int64 `json:"remaining_quota"`
				Calls     int64 `json:"remaining_calls"`
			}
		}
	}
	if json.Unmarshal(got.ResultJSON, &value) != nil || len(value.Tools) != 1 || value.Tools[0].Input != f.detail.Tools[0].InputSchema || value.Tools[0].SHA != digest([]byte(value.Tools[0].Input)) || len(value.Tools[0].Grants) != 1 || value.Tools[0].Grants[0].Remaining != 70 || value.Tools[0].Grants[0].Calls != 2 {
		t.Fatal("did not project exact schema and current counters", string(got.ResultJSON))
	}
	for _, secret := range []string{"owner_id", "private.invalid", "browser-consent", in.Credential} {
		if strings.Contains(string(got.ResultJSON), secret) {
			t.Fatal("private data exposed", secret)
		}
	}
	if f.executes != 0 || len(f.scopeRequests) != 2 || len(f.scopeRequests[1].ToolIDs) != 1 {
		t.Fatal("read did not use closed Core scope")
	}
	executor, listed := readInvocation(t, e, in, "get_connected_market_tools", `{}`)
	got, err = executor.Execute(context.Background(), listed)
	if err != nil || strings.Contains(string(got.ResultJSON), "grant-one") || strings.Contains(string(got.ResultJSON), "input_schema") {
		t.Fatal("service listing included unselected authority/schema", err)
	}
}
func TestConnectedCatalogueCannotBypassMissingAuthority(t *testing.T) {
	for _, name := range []string{"unavailable", "not_allowed", "snapshot", "quote"} {
		t.Run(name, func(t *testing.T) {
			e, f, in := connectedFixture(t)
			switch name {
			case "unavailable":
				f.marketScopeErr = toolmarket.ErrUnavailable
			case "not_allowed":
				f.marketScope.AllowedServiceIDs = []string{}
			case "snapshot":
				f.detail.Service.LiveVersionID = "changed"
			case "quote":
				f.quote.VersionID = "changed"
			}
			executor, in := readInvocation(t, e, in, "get_connected_market_tools", `{"service_id":"service-one"}`)
			if _, err := executor.Execute(context.Background(), in); err == nil {
				t.Fatal("missing authority accepted")
			}
			if f.executes != 0 {
				t.Fatal("read dispatched")
			}
		})
	}
	e, _, in := connectedFixture(t)
	executor, in := readInvocation(t, e, in, "get_connected_market_tools", `{"service_id":"service-one","offset":1}`)
	if _, err := executor.Execute(context.Background(), in); !errors.Is(err, access.ErrInvalid) {
		t.Fatal("ambiguous pagination accepted", err)
	}
}
func TestConnectedSchemaOmissionNeverPublishesPermissiveFallback(t *testing.T) {
	for _, name := range []string{"large", "redaction"} {
		t.Run(name, func(t *testing.T) {
			e, f, in := connectedFixture(t)
			if name == "large" {
				f.detail.Tools[0].InputSchema = `{"type":"object","description":"` + strings.Repeat("a", 9000) + `"}`
			} else {
				f.detail.Tools[0].InputSchema = `{"type":"object","properties":{"token":{"type":"string"}},"additionalProperties":false}`
			}
			executor, in := readInvocation(t, e, in, "get_connected_market_tools", `{"service_id":"service-one"}`)
			got, err := executor.Execute(context.Background(), in)
			if err != nil || !strings.Contains(string(got.ResultJSON), `"schema_omitted":true`) || strings.Contains(string(got.ResultJSON), "input_schema_json") {
				t.Fatalf("unsafe schema fallback %s %v", got.ResultJSON, err)
			}
		})
	}
}
