// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"encoding/json"
	"net/http"
	"strings"
	"testing"
)

func TestLegacyMCPExternalClientMigration(t *testing.T) {
	handler, _ := legacyMCPHTTPFixture(t)
	const list = `{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}`
	const challenge = `Bearer resource_metadata="https://example.test/.well-known/oauth-protected-resource/mcp/market", scope="market:discover"`
	for _, tc := range []struct {
		name            string
		headers         map[string]string
		status          int
		challenge, hint string
	}{
		{"no_auth", map[string]string{"Authorization": ""}, 401, challenge, "Connect this MCP client with OAuth"},
		{"personal_bearer", map[string]string{"Authorization": "Bearer lmm_market_fixture"}, 401, challenge + `, error="invalid_token"`, "Connect this MCP client with OAuth"},
		{"api_key_header", map[string]string{"Authorization": "", "X-Api-Key": "fixture"}, 401, challenge, "Connect this MCP client with OAuth"},
		{"mixed_headers", map[string]string{"X-Api-Key": "fixture"}, 400, "", "Remove additional credentials"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			w := marketMCPRequest(handler, "https://example.test/mcp/market", list, tc.headers)
			if w.Code != tc.status || w.Header().Get("WWW-Authenticate") != tc.challenge || !strings.Contains(w.Body.String(), tc.hint) || strings.Contains(w.Body.String(), "fixture") {
				t.Fatalf("HTTP %d, challenge %q, body %q", w.Code, w.Header().Get("WWW-Authenticate"), w.Body.String())
			}
		})
	}
	for _, query := range []string{"", "?mode=full", "?mode=compact"} {
		w := marketMCPRequest(handler, "https://example.test/mcp/market"+query, list, nil)
		var doc struct {
			Result struct{ Tools []struct{ Name string } }
		}
		if err := json.Unmarshal(w.Body.Bytes(), &doc); err != nil || w.Code != 200 || len(doc.Result.Tools) != 1 || doc.Result.Tools[0].Name != "metamcp" {
			t.Fatalf("%q: %d %s", query, w.Code, w.Body.String())
		}
	}
	for _, tc := range []struct {
		name, arguments, want string
		toolError             bool
		rpcError              int
	}{
		{"metamcp", `{"action":"search"}`, "metamcp expects a single code string", true, 0},
		{"lmm_market_search", `{}`, `unknown tool "lmm_market_search"`, false, -32602},
		{"metamcp", `{"code":"search({query:'drawing',limit:1})"}`, "", false, 0},
	} {
		payload, err := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": tc.name, "arguments": json.RawMessage(tc.arguments)}})
		if err != nil {
			t.Fatal(err)
		}
		w := marketMCPRequest(handler, "https://example.test/mcp/market?mode=full", string(payload), nil)
		var doc struct {
			Result struct {
				IsError           bool
				Content           []struct{ Text string }
				StructuredContent map[string]any
			}
			Error struct {
				Code    int
				Message string
			}
		}
		if err := json.Unmarshal(w.Body.Bytes(), &doc); err != nil || w.Code != http.StatusOK || doc.Result.IsError != tc.toolError || doc.Error.Code != tc.rpcError {
			t.Fatalf("%s: %d %s", tc.arguments, w.Code, w.Body.String())
		}
		switch {
		case tc.toolError:
			if len(doc.Result.Content) != 1 || !strings.Contains(doc.Result.Content[0].Text, tc.want) {
				t.Fatal(w.Body.String())
			}
		case tc.rpcError != 0:
			if doc.Error.Message != tc.want {
				t.Fatal(w.Body.String())
			}
		default:
			if _, ok := doc.Result.StructuredContent["items"]; !ok {
				t.Fatal(w.Body.String())
			}
		}
	}
}

func TestLegacyMCPPendingResultUsesCodeStatusLookup(t *testing.T) {
	got, err := legacyMCPExecutionOutput(LegacyWebExecutionResponse{Call: map[string]any{"id": "existing", "execution_status": "pending"}})
	if err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(got)
	if err != nil || !got.IsError || !strings.Contains(string(raw), "call_status({call_id:'CALL_ID'})") || !strings.Contains(string(raw), `"id":"existing"`) {
		t.Fatalf("missing original receipt or status lookup: %s, %v", raw, err)
	}
}
