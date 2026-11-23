// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)

type assistantExecutionAuthority struct {
	executionAuthority
	scope                         AssistantMarketCatalogueReceipt
	scopeErr                      error
	scopeReads, assistantReserves int
	operation                     string
	reserveInput                  LegacyCreditReserve
}

func (a *assistantExecutionAuthority) ReadAssistantMarketCatalogue(_ context.Context, _ string, _ AssistantMarketCatalogueRequest) (AssistantMarketCatalogueReceipt, error) {
	a.scopeReads++
	return a.scope, a.scopeErr
}
func (a *assistantExecutionAuthority) ReserveAssistantCredits(_ context.Context, _ string, op string, in LegacyCreditReserve) (LegacyCreditCall, error) {
	a.assistantReserves++
	a.operation = op
	a.reserveInput = in
	if a.reserveErr != nil {
		return LegacyCreditCall{}, a.reserveErr
	}
	r := a.r
	if !a.reserveExisting {
		r.State = "reserved"
		r.DispatchID = ""
		r.DispatchClaimed = false
	}
	return r, nil
}
func assistantExecutionFixture(t *testing.T, remoteUnknown bool) (*Module, *assistantExecutionAuthority, LegacyExecuteInput, *int, *int) {
	t.Helper()
	tool := Tool{Name: "echo", Description: "echo", InputSchema: json.RawMessage(`{"type":"object"}`)}
	a := &assistantExecutionAuthority{}
	d := emptyDB()
	w := webState(&d)
	w.Services["service"] = WebServiceRecord{ID: "service", OwnerID: 1, LiveVersionID: "version", Status: "published"}
	w.Versions["version"] = webSnapshot{Version: WebVersion{ID: "version", ServiceID: "service", Status: "published", ExecutionType: "remote", Visibility: "public", Endpoint: "https://tools.example.test/mcp"}, Digest: "snapshot", RemoteDigests: map[string]string{"tool": digest(tool)}, Tools: []WebTool{{ToolID: "tool", VersionID: "version", Name: "echo", InputSchema: `{"type":"object"}`}}}
	m, err := New(&Store{data: d}, a, Config{WebOnly: true})
	if err != nil {
		t.Fatal(err)
	}
	m.now = func() time.Time { return time.Unix(100, 0) }
	in := LegacyExecuteInput{ClientID: "builtin-assistant", RequestKey: "original-assistant-op", ToolID: "tool", VersionID: "version", GrantID: "grant", Arguments: json.RawMessage(`{}`)}
	price, _ := LegacyQuotePricingDigest(WebTool{}.LegacyPricing())
	owner := Principal{User: "1", Account: "1"}
	a.r = LegacyCreditCall{PricingDigest: price, ID: "call", Owner: owner, ClientID: in.ClientID, RequestKey: in.RequestKey, QuoteID: "quote", GrantID: "grant", ToolID: "tool", VersionID: "version", InputDigest: digest(map[string]any{}), ResolveBy: 220, State: "running", DispatchID: "dispatch", DispatchClaimed: true}
	grant := CoreLegacyConsentReceipt{CoreLegacyConsentRequest: CoreLegacyConsentRequest{CoreLegacyOperation: CoreLegacyOperation{Owner: owner, OperationID: "browser-confirmed-consent"}, ClientID: in.ClientID, ToolID: in.ToolID, VersionID: in.VersionID, MaxCalls: 3, ExpiresAt: 1000}, ID: "grant", QuoteID: "quote", RulesHash: price, CreatedAt: 90}
	a.scope = AssistantMarketCatalogueReceipt{Owner: owner, PolicyRevision: "policy-v1", AllowedServiceIDs: []string{"service"}, Consents: []CoreLegacyConsentReceipt{grant}, CheckedAtUnix: 100}
	calls, discovery := 0, 0
	m.net.client = &http.Client{Transport: legacyProviderTransport(func(r *http.Request) (*http.Response, error) {
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
		}
		if json.NewDecoder(r.Body).Decode(&req) != nil {
			t.Fatal("invalid RPC")
		}
		var result any
		switch req.Method {
		case "initialize":
			result = map[string]any{"protocolVersion": ProtocolVersion, "capabilities": map[string]any{"tools": map[string]any{}}}
		case "notifications/initialized":
			return &http.Response{StatusCode: 202, Header: http.Header{}, Body: io.NopCloser(strings.NewReader(""))}, nil
		case "tools/list":
			discovery++
			result = map[string]any{"tools": []Tool{tool}}
		case "tools/call":
			calls++
			if remoteUnknown {
				return nil, ErrUnavailable
			}
			result = map[string]any{"content": []any{map[string]any{"type": "text", "text": "ok"}}}
		default:
			t.Fatal(req.Method)
		}
		raw, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": req.ID, "result": result})
		return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": []string{"application/json"}}, Body: io.NopCloser(strings.NewReader(string(raw)))}, nil
	})}
	return m, a, in, &calls, &discovery
}
func TestAssistantRemoteUsesCoreConsentAndOriginalOperationOnce(t *testing.T) {
	m, a, in, calls, discovery := assistantExecutionFixture(t, false)
	if _, err := m.LegacyExecuteRemote(context.Background(), "1", "1", in); !errors.Is(err, ErrDenied) || a.scopeReads != 0 {
		t.Fatal("ordinary route accepted assistant authority", err)
	}
	got, err := m.ExecuteAssistantRemote(context.Background(), "1", "1", in.RequestKey, in)
	if err != nil || got.State != "done" || got.AssistantToolOperationID != in.RequestKey || *calls != 1 || a.assistantReserves != 1 || a.reserves != 0 || a.operation != in.RequestKey || a.reserveInput.RequestKey != in.RequestKey {
		t.Fatalf("dedicated execution: %+v %v calls=%d assistant=%d ordinary=%d", got, err, *calls, a.assistantReserves, a.reserves)
	}
	if len(m.store.data.LegacyAccess.Grants) != 1 || len(m.store.data.Web.Installations) != 1 {
		t.Fatal("real Core consent was not mirrored")
	}
	a.scopeErr = ErrDenied
	replay, err := m.ExecuteAssistantRemote(context.Background(), "1", "1", in.RequestKey, in)
	if err != nil || replay.State != "done" || *calls != 1 || *discovery != 1 || a.assistantReserves != 1 {
		t.Fatalf("replay performed new work: %+v %v", replay, err)
	}
	changed := got
	changed.AssistantToolOperationID = "another"
	if !errors.Is(m.legacySave(changed), ErrConflict) {
		t.Fatal("original operation marker changed")
	}
	in.Arguments = json.RawMessage(`{"changed":true}`)
	if _, err = m.ExecuteAssistantRemote(context.Background(), "1", "1", in.RequestKey, in); !errors.Is(err, ErrConflict) || *calls != 1 {
		t.Fatal("changed original input accepted", err)
	}
}
func TestAssistantUnknownRemoteCannotDispatchAgain(t *testing.T) {
	m, a, in, calls, discovery := assistantExecutionFixture(t, true)
	got, err := m.ExecuteAssistantRemote(context.Background(), "1", "1", in.RequestKey, in)
	if !errors.Is(err, ErrPending) || got.State != "unknown" || *calls != 1 || a.unknowns != 1 {
		t.Fatalf("lost remote outcome: %+v %v", got, err)
	}
	for i := 0; i < 3; i++ {
		_, err = m.ExecuteAssistantRemote(context.Background(), "1", "1", in.RequestKey, in)
		if !errors.Is(err, ErrPending) {
			t.Fatal(err)
		}
	}
	if *calls != 1 || *discovery != 1 || a.dispatches != 1 || a.assistantReserves != 1 || a.finishes != 0 {
		t.Fatalf("unknown resent: calls=%d dispatch=%d reserve=%d", *calls, a.dispatches, a.assistantReserves)
	}
}
func TestAssistantLostReserveACKRetainsOriginalMarkerWithoutDispatch(t *testing.T) {
	m, a, in, calls, discovery := assistantExecutionFixture(t, false)
	a.reserveErr = ErrUnavailable
	got, err := m.ExecuteAssistantRemote(context.Background(), "1", "1", in.RequestKey, in)
	if !errors.Is(err, ErrPending) || got.AssistantToolOperationID != in.RequestKey || *calls != 0 || a.dispatches != 0 {
		t.Fatal("reserve ACK loss dispatched", got, err)
	}
	a.reserveErr = nil
	a.reserveExisting = true
	a.r.State = "unknown"
	a.r.DispatchClaimed = false
	_, err = m.ExecuteAssistantRemote(context.Background(), "1", "1", in.RequestKey, in)
	if !errors.Is(err, ErrPending) || a.assistantReserves != 2 || a.reserves != 0 || *calls != 0 || *discovery != 0 || a.operation != in.RequestKey || a.dispatches != 0 {
		t.Fatal("lost reserve recovery changed authority or resent", err)
	}
}
func TestAssistantConsentReadbackRejectsMismatchesBeforeNetwork(t *testing.T) {
	cases := map[string]func(*AssistantMarketCatalogueReceipt){
		"owner":             func(r *AssistantMarketCatalogueReceipt) { r.Owner.Account = "2" },
		"allowlist":         func(r *AssistantMarketCatalogueReceipt) { r.AllowedServiceIDs = []string{} },
		"duplicate_service": func(r *AssistantMarketCatalogueReceipt) { r.AllowedServiceIDs = append(r.AllowedServiceIDs, "service") },
		"missing_list":      func(r *AssistantMarketCatalogueReceipt) { r.Consents = nil },
		"grant_owner":       func(r *AssistantMarketCatalogueReceipt) { r.Consents[0].Owner.User = "2" },
		"grant_id":          func(r *AssistantMarketCatalogueReceipt) { r.Consents[0].ID = "other" },
		"client":            func(r *AssistantMarketCatalogueReceipt) { r.Consents[0].ClientID = "web" },
		"version":           func(r *AssistantMarketCatalogueReceipt) { r.Consents[0].VersionID = "old" },
		"revoked":           func(r *AssistantMarketCatalogueReceipt) { r.Consents[0].RevokedAt = 99 },
		"expired":           func(r *AssistantMarketCatalogueReceipt) { r.Consents[0].ExpiresAt = 100 },
		"overdrawn":         func(r *AssistantMarketCatalogueReceipt) { r.Consents[0].SpentQuota = 1 },
		"exhausted_corrupt": func(r *AssistantMarketCatalogueReceipt) { r.Consents[0].ReservedCalls = 4 },
	}
	for name, mutate := range cases {
		t.Run(name, func(t *testing.T) {
			m, a, in, calls, discovery := assistantExecutionFixture(t, false)
			mutate(&a.scope)
			if _, err := m.ExecuteAssistantRemote(context.Background(), "1", "1", in.RequestKey, in); err == nil {
				t.Fatal("invalid Core authority accepted")
			}
			if *calls != 0 || *discovery != 0 || a.assistantReserves != 0 || len(m.store.data.LegacyExecutions) != 0 || len(m.store.data.Web.Installations) != 0 {
				t.Fatal("rejected authority created an execution")
			}
		})
	}
}
