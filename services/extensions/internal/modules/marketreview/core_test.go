// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package marketreview

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestCoreBoundaryUsesSeparateCredentialAndExactSubmission(t *testing.T) {
	ctx := context.Background()
	seen := map[string]int{}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		action := strings.TrimPrefix(r.URL.Path, "/internal/market-review/v1/")
		seen[action]++
		if r.Method != "POST" || r.Header.Get("Authorization") != "Bearer "+strings.Repeat("r", 32) || r.Header.Get("X-LMM-Protocol") != "1" {
			t.Error("private capability framing")
		}
		if action != "authorize" && action != "admit" && r.Header.Get("X-LMM-User-Credential") != "" {
			t.Error("retained owner Session")
		}
		var body map[string]json.RawMessage
		if json.NewDecoder(r.Body).Decode(&body) != nil {
			t.Fatal("invalid DTO")
		}
		switch action {
		case "capabilities":
			json.NewEncoder(w).Encode(map[string]any{"protocol_major": 1, "route": coreRoute{"synthetic", "model", "openai_moderation", []string{"violence"}}, "auto_approve_tools": true})
		case "owner":
			if string(body["owner_id"]) != `"7"` {
				t.Error("owner coerced")
			}
			w.Write([]byte(`{"enabled":true}`))
		case "authorize":
			if r.Header.Get("X-LMM-User-Credential") != "session" || string(body["root"]) != "true" {
				t.Error("admin binding")
			}
			w.Write([]byte(`{"user_id":"7"}`))
		case "classify":
			if string(body["source"]) != `"tool_market"` || string(body["owner_id"]) != `"7"` || string(body["version"]) != `"v1"` || len(body) != 8 {
				t.Error("classification DTO", body)
			}
			w.Write([]byte(`{"flagged":false,"categories":{"violence":false},"response_model":"model","receipt":"core-proof"}`))
		case "admit":
			if r.Header.Get("X-LMM-User-Credential") != "owner-session" || len(body) != 3 {
				t.Error("submission admission framing")
			}
			var bound coreSubmission
			if json.Unmarshal(body["submission"], &bound) != nil || bound.Token != "t1" || bound.Source != "tool_market" || bound.OwnerID != "7" {
				t.Error("admission binding")
			}
			w.Write([]byte(`{"token":"t1","revision":2}`))
		case "approve":
			if string(body["receipt"]) != `"core-proof"` {
				t.Error("not bound to Core receipt")
			}
			w.Write([]byte(`{"token":"t1","revision":3,"approved":true}`))
		case "invalidate":
			w.Write([]byte(`{"token":"t1","revision":4,"invalidated":true}`))
		default:
			t.Error("unexpected route", action)
		}
	}))
	defer server.Close()
	core, err := NewCore("/synthetic/review.sock", []byte(strings.Repeat("r", 32)))
	if err != nil {
		t.Fatal(err)
	}
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", server.Listener.Addr().String())
	}}
	core.client = &http.Client{Transport: transport}
	defer core.Close()
	engine := &Engine{Classifier: core}
	if _, err := engine.Classify(ctx, Job{Record: Record{Source: "tool"}, OwnerID: 7}); err == nil || err.Error() != "review_disabled" {
		t.Fatal("default-off review reached Core", err)
	}
	route, err := core.Route(ctx)
	if err != nil || !core.AutoApproveTools() {
		t.Fatal(route, err)
	}
	if _, err = core.Authorize(ctx, "session", true); err != nil {
		t.Fatal(err)
	}
	if enabled, err := core.OwnerEnabled(ctx, 7); err != nil || !enabled {
		t.Fatal(enabled, err)
	}
	r := Request{Source: "tool", TargetID: "service", Version: "v1", Token: "t1", OwnerID: 7, ContentHash: strings.Repeat("a", 64), Text: "Synthetic title", Route: route}
	result, err := core.Classify(ctx, r)
	if err != nil || result.Flagged || len(result.Categories) != 0 || result.Receipt != "core-proof" {
		t.Fatal(result, err)
	}
	if rev, err := core.AdmitTool(ctx, "owner-session", r, []string{strings.Repeat("b", 64)}, ""); err != nil || rev != 2 {
		t.Fatal(rev, err)
	}
	if rev, err := core.ApproveTool(ctx, result.Receipt, []string{strings.Repeat("b", 64)}, r.Token); err != nil || rev != 3 {
		t.Fatal(rev, err)
	}
	if err = core.InvalidateTool(ctx, 7, r.Token); err != nil {
		t.Fatal(err)
	}
	for _, action := range []string{"capabilities", "authorize", "owner", "classify", "admit", "approve", "invalidate"} {
		if seen[action] != 1 {
			t.Fatal("unexpected retry", seen)
		}
	}
}

func TestCoreInvalidAndRateLimitErrorsAreBounded(t *testing.T) {
	for _, code := range []int{400, 403, 429, 502} {
		t.Run(http.StatusText(code), func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(code)
				w.Write([]byte("PRIVATE_PROVIDER_BODY"))
			}))
			defer server.Close()
			core := &Core{token: strings.Repeat("r", 32), client: &http.Client{Transport: &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				return (&net.Dialer{}).DialContext(ctx, "tcp", server.Listener.Addr().String())
			}}}}
			defer core.Close()
			_, err := core.Route(context.Background())
			if err == nil || strings.Contains(err.Error(), "PRIVATE") {
				t.Fatal("provider error leaked", err)
			}
			var fail *Failure
			if code == 429 || code == 502 {
				if !errors.As(err, &fail) || !fail.Temporary {
					t.Fatal("transient not retryable", err)
				}
			}
		})
	}
}
