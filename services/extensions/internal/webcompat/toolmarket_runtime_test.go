// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

type marketRuntimeFake struct {
	marketTransportFake
	requests []*http.Request
}

func (f *marketRuntimeFake) ToolMarketRuntimeHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { f.requests = append(f.requests, r); w.WriteHeader(202) })
}
func TestToolMarketRuntimeUsesVerifiedActorAndPreservesOAuthBinding(t *testing.T) {
	core, f := &authCore{}, &marketRuntimeFake{}
	req := marketHTTP("POST", "/api/tool-market/runtime/meta/search?query=keep", "{}", true)
	req.Header["x-lmm-user-credential"] = []string{"attacker"}
	req.Header["X-LMM-Account-ID"] = []string{"999"}
	req.Header["X-LMM-Host-Token"] = []string{"forged"}
	req.Header.Set("Cookie", "__Host-lmm-mcp-binding=synthetic")
	req.Header.Set("Origin", "https://synthetic.invalid")
	rec := httptest.NewRecorder()
	marketMux(t, core, f).ServeHTTP(rec, req)
	if rec.Code != 202 || len(f.requests) != 1 {
		t.Fatal(rec.Code, len(f.requests))
	}
	got := f.requests[0]
	if got.Header.Get("X-LMM-User-Credential") != webToken || got.Header.Get("X-LMM-Account-ID") != "17" || got.Header.Get("X-LMM-Host-Token") != "" || got.URL.Path != "/meta/search" || got.URL.RawQuery != "query=keep" || got.Header.Get("Cookie") != req.Header.Get("Cookie") || got.Header.Get("Origin") != req.Header.Get("Origin") {
		t.Fatal("runtime actor binding or OAuth transport changed")
	}
	if req.URL.Path != "/api/tool-market/runtime/meta/search" || req.Header["X-LMM-Account-ID"][0] != "999" {
		t.Fatal("mutated caller request")
	}
}
func TestToolMarketRuntimeRejectsMissingSessionAndDeniedActionBeforeReadingBody(t *testing.T) {
	for _, hasToken := range []bool{false, true} {
		core, f := &authCore{}, &marketRuntimeFake{marketTransportFake: marketTransportFake{authErr: toolmarket.ErrDenied}}
		req := marketHTTP("POST", "/api/tool-market/runtime/meta/execute", "", hasToken)
		body := &marketUnreadBody{}
		req.Body = body
		rec := httptest.NewRecorder()
		marketMux(t, core, f).ServeHTTP(rec, req)
		want := 401
		if hasToken {
			want = 403
		}
		if rec.Code != want || len(f.requests) != 0 || body.reads != 0 {
			t.Fatal(rec.Code, len(f.requests), body.reads)
		}
	}
}

func TestBrowserProviderOAuthReturnCarriesOnlyBoundFieldsToPortal(t *testing.T) {
	core, runtime := &authCore{}, &marketRuntimeFake{}
	mux := marketMux(t, core, runtime)
	state := strings.Repeat("A", 43)
	encoded, _ := json.Marshal(state)
	cookie := &http.Cookie{Name: "__Host-lmm-mcp-" + fmt.Sprintf("%x", sha256.Sum256(encoded))[:24], Value: strings.Repeat("B", 43)}
	for _, tc := range []struct {
		name, params    string
		cookies, status int
	}{
		{"approved", "code=test%26code&iss=https%3A%2F%2Fissuer.example", 1, http.StatusSeeOther},
		{"declined", "error=access_denied&error_description=private+provider+detail", 1, http.StatusSeeOther},
		{"missing binding", "code=test", 0, http.StatusUnauthorized},
		{"ambiguous binding", "code=test", 2, http.StatusUnauthorized},
		{"duplicate code", "code=test&code=other", 1, http.StatusUnprocessableEntity},
		{"unrecognized redirect parameter", "code=test&return_to=https%3A%2F%2Fother.example", 1, http.StatusUnprocessableEntity},
		{"mixed outcomes", "error=access_denied&code=test", 1, http.StatusUnprocessableEntity},
		{"oversized code", "code=" + strings.Repeat("x", 4097), 1, http.StatusUnprocessableEntity},
	} {
		t.Run(tc.name, func(t *testing.T) {
			req := marketHTTP("GET", toolmarket.ProviderOAuthCallbackPath+"?state="+state+"&"+tc.params, "", false)
			req.Header.Set("Sec-Fetch-Mode", "navigate")
			req.Header.Set("Sec-Fetch-Dest", "document")
			for i := 0; i < tc.cookies; i++ {
				req.AddCookie(cookie)
			}
			rec := httptest.NewRecorder()
			mux.ServeHTTP(rec, req)
			if rec.Code != tc.status || rec.Header().Get("Referrer-Policy") != "no-referrer" || !strings.Contains(rec.Header().Get("Cache-Control"), "no-store") {
				t.Fatalf("unusable callback response (%d)", rec.Code)
			}
			location := rec.Header().Get("Location")
			if tc.status != http.StatusSeeOther {
				if location != "" {
					t.Fatal("rejected callback redirected")
				}
				return
			}
			const portal = "/tool-market?tab=connections#mcp-oauth-callback&"
			if !strings.HasPrefix(location, portal) || strings.Contains(location, "private") || rec.Body.Len() != 0 {
				t.Fatal("callback escaped the portal or exposed provider text")
			}
			values, err := url.ParseQuery(strings.TrimPrefix(location, portal))
			if err != nil || values.Get("state") != state || values.Get("code") != "test&code" && values.Get("error") != "access_denied" {
				t.Fatal("callback fields lost")
			}
		})
	}
	if len(runtime.requests) != 0 || core.calls != 0 {
		t.Fatal("a document callback entered an authenticated operation")
	}
	req := marketHTTP("GET", toolmarket.ProviderOAuthCallbackPath+"?state="+state+"&code=test", "", false)
	req.AddCookie(cookie)
	rec := httptest.NewRecorder()
	mux.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized || rec.Header().Get("Location") != "" {
		t.Fatal("uncredentialed API read accepted")
	}
}
