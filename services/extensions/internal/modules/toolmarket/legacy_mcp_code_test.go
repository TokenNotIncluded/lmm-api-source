// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"encoding/base64"
	"encoding/json"
	"net/http"
	"strings"
	"testing"

	"github.com/expr-lang/expr"
)

func TestMarketCodeDiscoveryAndOAuth(t *testing.T) {
	handler, _ := legacyMCPHTTPFixture(t)
	const list = `{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}`
	for _, suffix := range []string{"", "?mode=full", "?mode=compact"} {
		w := marketMCPRequest(handler, "http://x/mcp/market"+suffix, list, nil)
		var reply struct {
			Result struct {
				Tools []struct {
					Name        string
					InputSchema struct{ Required []string }
				}
			}
		}
		if w.Code != http.StatusOK || json.Unmarshal(w.Body.Bytes(), &reply) != nil || len(reply.Result.Tools) != 1 {
			t.Fatalf("list %s: %d %s", suffix, w.Code, w.Body)
		}
		tool := reply.Result.Tools[0]
		if tool.Name != "metamcp" || len(tool.InputSchema.Required) != 1 || tool.InputSchema.Required[0] != "code" {
			t.Fatal(tool)
		}
	}
	raw := "lmm_market_" + base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	w := marketMCPRequest(handler, "http://x/mcp/market", list, map[string]string{"Authorization": "Bearer " + raw})
	if w.Code != 401 || !strings.Contains(w.Header().Get("WWW-Authenticate"), "resource_metadata=") {
		t.Fatal(w.Code, w.Header())
	}
	// The SDK executes the expression and returns the existing catalogue result.
	w = marketMCPRequest(handler, "http://x/mcp/market", `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"metamcp","arguments":{"code":"search({limit:1})"}}}`, nil)
	var response struct {
		Result struct {
			IsError           bool
			StructuredContent struct{ Limit int }
		}
	}
	if w.Code != 200 || json.Unmarshal(w.Body.Bytes(), &response) != nil || response.Result.IsError || response.Result.StructuredContent.Limit != 1 {
		t.Fatal(w.Code, w.Body)
	}
}

func TestMarketCodeUsesEarlierReadResults(t *testing.T) {
	raw, _ := json.Marshal(map[string]string{"code": `let found=search({query:'drawing'}); details({service_id:found.items[0].id})`})
	steps, err := parseMarketCode(raw)
	if err != nil || len(steps) != 2 || steps[0].bind != "found" || steps[1].action != "details" {
		t.Fatal(steps, err)
	}
	v, err := expr.Run(steps[1].args, map[string]any{"found": map[string]any{"items": []any{map[string]any{"id": "svc"}}}})
	if err != nil || v.(map[string]any)["service_id"] != "svc" {
		t.Fatal(v, err)
	}
	for _, src := range []string{"missing({})", "let x=invoke({}); status({})", "details({service_id:host()})", "search({query:env.SECRET})", "search({limit:1..10000000})", "search({}); search({})"} {
		raw, _ := json.Marshal(map[string]string{"code": src})
		if _, err := parseMarketCode(raw); err == nil {
			t.Fatal("accepted", src)
		}
	}
}
