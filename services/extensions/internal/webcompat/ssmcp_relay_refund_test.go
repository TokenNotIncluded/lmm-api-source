// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcprelay"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func TestRelayRefundRequiresLiveOperatorAndFixedOrderInput(t *testing.T) {
	id := strings.Repeat("a", 64)
	valid := `{"order_id":"` + id + `","user_id":42}`
	for _, tc := range []struct {
		name, token, body string
		level             int32
		revoked           bool
		want              int
	}{
		{"no session", "", valid, 6, false, 401},
		{"service credential", hostToken, valid, 6, false, 401},
		{"ordinary buyer", webToken, valid, 0, false, 403},
		{"revoked operator", webToken, valid, 6, true, 401},
		{"wrong order", webToken, `{"order_id":"wrong","user_id":42}`, 5, false, http.StatusUnprocessableEntity},
		{"missing owner", webToken, `{"order_id":"` + id + `"}`, 6, false, http.StatusUnprocessableEntity},
		{"chosen amount", webToken, `{"order_id":"` + id + `","user_id":42,"amount":5}`, 6, false, http.StatusUnprocessableEntity},
		{"conflicting owners", webToken, `{"order_id":"` + id + `","user_id":42,"user_id":43}`, 6, false, http.StatusUnprocessableEntity},
	} {
		t.Run(tc.name, func(t *testing.T) {
			core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
				if tc.revoked {
					return nil, status.Error(codes.Unauthenticated, "revoked")
				}
				self := sessionSelf()
				self.PlatformLevel = tc.level
				return self, nil
			}}
			mux := http.NewServeMux()
			// Invalid requests must return before touching orders or funds.
			RegisterSSMCPRelay(mux, newAuth(t, core), new(ssmcprelay.Service))
			req := httptest.NewRequest(http.MethodPost, "/api/ssmcpd/admin/orders/"+id+"/refund", strings.NewReader(tc.body))
			req.Header.Set("Content-Type", "application/json")
			if tc.token != "" {
				req.Header.Set("Authorization", "Bearer "+tc.token)
			}
			response := httptest.NewRecorder()
			mux.ServeHTTP(response, req)
			if response.Code != tc.want {
				t.Fatalf("status %d, want %d: %s", response.Code, tc.want, response.Body.String())
			}
		})
	}
}
