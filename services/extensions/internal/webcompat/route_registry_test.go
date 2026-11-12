// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"net/http"
	"strings"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
)

func TestRegisteredRouteAutomaticallyUsesPluginAndHostLifecycle(t *testing.T) {
	host, err := modules.NewHostWithLimit(nil, []byte(hostToken), 1)
	if err != nil {
		t.Fatal(err)
	}
	manager := &pluginControllerFake{enabled: map[string]bool{"custom-plugin": false}}
	mux := http.NewServeMux()
	routes := routeRegistry{mux: mux, host: host, plugins: manager}.forPlugins("custom-plugin")
	calls := 0
	routes.HandleFunc("GET /custom-prefix/{id}", func(w http.ResponseWriter, r *http.Request) {
		calls++
		if r.PathValue("id") != "item" {
			t.Errorf("path parameter lost: %q", r.PathValue("id"))
		}
		w.WriteHeader(http.StatusNoContent)
	})
	routes.Handle("POST /custom-action", http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		calls++
		w.WriteHeader(http.StatusNoContent)
	}))
	mux.Handle("/extensions/v1/", host)
	mux.Handle("/health/", host)
	requests := []struct{ method, path string }{{"GET", "/custom-prefix/item"}, {"POST", "/custom-action"}}
	check := func(want int, retry string) {
		t.Helper()
		for _, request := range requests {
			response := mountTransportCall(mux, request.method, request.path, nil)
			if response.Code != want || response.Header().Get("Retry-After") != retry {
				t.Fatalf("%s %s: status=%d retry=%q body=%s", request.method, request.path, response.Code, response.Header().Get("Retry-After"), response.Body)
			}
		}
	}
	check(http.StatusServiceUnavailable, "1")
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	check(http.StatusServiceUnavailable, "")
	if calls != 0 {
		t.Fatal("disabled plugin handler ran")
	}
	manager.enabled["custom-plugin"] = true
	check(http.StatusNoContent, "")
	if response := mountTransportCall(mux, http.MethodPost, "/custom-prefix/item", nil); response.Code != http.StatusMethodNotAllowed || response.Header().Get("Allow") != "GET, HEAD" {
		t.Fatalf("registered route lost method restriction: status=%d allow=%q", response.Code, response.Header().Get("Allow"))
	}
	if calls != len(requests) {
		t.Fatalf("registered handler not invoked exactly once: %d", calls)
	}
	manager.enabled["custom-plugin"] = false
	check(http.StatusServiceUnavailable, "")
	manager.enabled["custom-plugin"] = true
	host.BeginDrain()
	check(http.StatusServiceUnavailable, "1")
	if calls != len(requests) {
		t.Fatal("disabled or draining plugin handler ran")
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if err := host.Wait(ctx); err != nil {
		t.Fatalf("registered route leaked admission: %v", err)
	}
}

func TestMountDeploymentRoutesFollowOperationsPluginState(t *testing.T) {
	host, _ := mountTransportHost(t, nil)
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
		actor := sessionSelf()
		actor.PlatformLevel = 6
		return actor, nil
	}}
	manager := &pluginControllerFake{enabled: map[string]bool{"operations": false}}
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{Plugins: manager})
	requests := []struct{ method, path string }{
		{"GET", "/api/deployments"},
		{"GET", "/api/deployments/"},
		{"POST", "/api/deployments"},
		{"GET", "/api/deployments/settings"},
		{"PUT", "/api/deployments/settings"},
		{"POST", "/api/deployments/test-connection"},
		{"GET", "/api/deployments/operations"},
		{"POST", "/api/deployments/operations/receipts/original_operation_0001/recover"},
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	for _, request := range requests {
		response := pluginRequest(handler, request.method, request.path, "", webToken)
		if response.Code != http.StatusServiceUnavailable || !strings.Contains(response.Body.String(), "STORE_UNAVAILABLE") || response.Header().Get("Location") != "" {
			t.Fatalf("disabled %s: status=%d body=%s", request.path, response.Code, response.Body)
		}
	}
	if core.calls != 0 {
		t.Fatalf("disabled operations reached authentication: %d", core.calls)
	}
	manager.enabled["operations"] = true
	for _, request := range requests {
		response := pluginRequest(handler, request.method, request.path, "", webToken)
		if response.Code != http.StatusServiceUnavailable || !strings.Contains(response.Body.String(), "OPERATIONS_UNAVAILABLE") {
			t.Fatalf("enabled route did not reach unavailable service %s: status=%d body=%s", request.path, response.Code, response.Body)
		}
	}
	if core.calls != len(requests) {
		t.Fatalf("enabled operations skipped live authentication: %d", core.calls)
	}
	host.BeginDrain()
	for _, request := range requests {
		response := pluginRequest(handler, request.method, request.path, "", webToken)
		if response.Code != http.StatusServiceUnavailable || response.Header().Get("Retry-After") != "1" {
			t.Fatalf("draining %s: status=%d body=%s", request.path, response.Code, response.Body)
		}
	}
	if core.calls != len(requests) {
		t.Fatal("draining operations reached authentication")
	}
}

func TestMountMarketReviewRoutesUseTheirRegisteredOwners(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	manager := &pluginControllerFake{}
	handler := Mount(host, newAuth(t, core), newStoreTransportFake(), nil, AdditionalServices{Plugins: manager, ToolMarket: &marketTransportFake{}})
	for _, enabled := range []map[string]bool{
		{"store": true},
		{"toolmarket": true},
		{},
	} {
		manager.enabled = enabled
		for _, request := range []struct {
			path    string
			allowed bool
		}{
			{"/api/security/market-ai-review/settings", enabled["store"] || enabled["toolmarket"]},
			{"/api/store/products/product-1/ai-reviews", enabled["store"]},
			{"/api/tool-market/services/tool-1/ai-reviews", enabled["toolmarket"]},
		} {
			before := core.calls
			response := pluginRequest(handler, http.MethodGet, request.path, "", webToken)
			wantCalls := before
			if request.allowed {
				wantCalls++
			}
			if response.Code != http.StatusServiceUnavailable || core.calls != wantCalls {
				t.Fatalf("owners=%v path=%s: status=%d auth calls=%d want=%d", enabled, request.path, response.Code, core.calls, wantCalls)
			}
		}
	}
}
