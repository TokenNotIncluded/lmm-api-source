// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"net/http"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

func TestProviderGatewayConfigurationOwnerSession(t *testing.T) {
	for _, tc := range []struct {
		name, method, token string
		level               int32
		want                int
	}{
		{"owner", "GET", webToken, 6, http.StatusOK},
		{"admin", "GET", webToken, 5, http.StatusForbidden},
		{"anonymous", "GET", "", 6, http.StatusUnauthorized},
		{"host", "GET", hostToken, 6, http.StatusUnauthorized},
		{"write", "PUT", webToken, 6, http.StatusMethodNotAllowed},
	} {
		t.Run(tc.name, func(t *testing.T) {
			core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
				self := sessionSelf()
				self.PlatformLevel = tc.level
				return self, nil
			}}
			mux := http.NewServeMux()
			registerProviderGatewayConfiguration(mux, newAuth(t, core), payments.DescribeProviderGateways(nil, nil, true))
			response := pluginRequest(mux, tc.method, "/api/option/payment-gateways/config", "", tc.token)
			hasConfiguration := strings.Contains(response.Body.String(), "extensions.provider-gateways.v1")
			if response.Code != tc.want || hasConfiguration != (tc.want == http.StatusOK) {
				t.Fatal("gateway configuration admission mismatch", response.Code)
			}
			if hasConfiguration && (response.Header().Get("Cache-Control") != "no-store" || response.Header().Get("Vary") != "Authorization") {
				t.Fatal("owner payment configuration can be cached")
			}
		})
	}
}
