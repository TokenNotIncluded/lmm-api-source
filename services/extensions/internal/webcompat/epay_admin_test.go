// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"net/http"
	"testing"
)

func TestEPayConfigurationRequiresOwnerAndPaymentLifecycle(t *testing.T) {
	for _, tc := range []struct {
		name, token string
		level       int32
		want        int
	}{
		{"owner", webToken, 6, 200}, {"admin", webToken, 5, 403},
		{"anonymous", "", 6, 401}, {"host", hostToken, 6, 401},
	} {
		t.Run(tc.name, func(t *testing.T) {
			core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
				self := sessionSelf()
				self.PlatformLevel = tc.level
				return self, nil
			}}
			probe := &pancakeAdminProbe{}
			mux := http.NewServeMux()
			RegisterEPayAdmin(mux, newAuth(t, core), probe)
			response := pluginRequest(mux, "GET", "/api/option/epay/config", "", tc.token)
			if response.Code != tc.want || (probe.calls == 1) != (tc.want == 200) {
				t.Fatal("EPay configuration admission mismatch", response.Code, probe.calls)
			}
			if tc.want == 200 && (response.Header().Get("Cache-Control") != "private, no-store" || response.Header().Get("Vary") != "Authorization") {
				t.Fatal("private owner configuration could be cached")
			}
		})
	}
	host, core := mountTransportHost(t, nil)
	core.fn = func(context.Context, string) (*pb.AuthorizeResponse, error) {
		self := sessionSelf()
		self.PlatformLevel = 6
		return self, nil
	}
	probe := &pancakeAdminProbe{}
	plugins := &pluginControllerFake{enabled: map[string]bool{"payments": true}}
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{EPayAdmin: probe, Plugins: plugins})
	request := func() int { return pluginRequest(handler, "GET", "/api/option/epay/config", "", webToken).Code }
	if request() != 503 || probe.calls != 0 {
		t.Fatal("configuration bypassed preparing state")
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	if request() != 200 || probe.calls != 1 {
		t.Fatal("ready configuration not mounted")
	}
	plugins.enabled["payments"] = false
	if request() != 503 || probe.calls != 1 {
		t.Fatal("configuration bypassed disabled module")
	}
	plugins.enabled["payments"] = true
	host.BeginDrain()
	if request() != 503 || probe.calls != 1 {
		t.Fatal("configuration bypassed drain")
	}
}
