// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type pickupTransportFake struct {
	*storeLegacyTransportFake
	pickupCalls          int
	session, token, code string
	user                 int64
	collect              bool
}

func (s *pickupTransportFake) Pickup(_ context.Context, session string, user int64, token, code string, collect bool) (map[string]any, error) {
	s.pickupCalls++
	s.session, s.user, s.token, s.code, s.collect = session, user, token, code, collect
	return map[string]any{"order_id": "synthetic-order"}, nil
}

func TestStorePickupTransportAuthenticatesCollectionBeforeReadingSecret(t *testing.T) {
	for _, headers := range [][]string{nil, {"Bearer " + hostToken}, {"Bearer " + webToken, "Bearer " + webToken}} {
		service := &pickupTransportFake{storeLegacyTransportFake: newStoreLegacyTransportFake()}
		core := &authCore{}
		request := storeTransportRequest("POST", "/api/user/auth/store-claim/synthetic-token", headers)
		probe := &storeBodyProbe{}
		request.Body = probe
		response := httptest.NewRecorder()
		storeTransportMux(t, core, service).ServeHTTP(response, request)
		if response.Code != 401 || probe.reads != 0 || service.pickupCalls != 0 {
			t.Fatalf("unauthorized collection read secret: status=%d reads=%d calls=%d", response.Code, probe.reads, service.pickupCalls)
		}
	}
	service := &pickupTransportFake{storeLegacyTransportFake: newStoreLegacyTransportFake()}
	request := httptest.NewRequest("POST", "/api/user/auth/store-claim/synthetic-token", strings.NewReader(`{"pickup_code":"private-synthetic-code"}`))
	request.Header.Set("Authorization", "Bearer "+webToken)
	response := httptest.NewRecorder()
	storeTransportMux(t, &authCore{}, service).ServeHTTP(response, request)
	if response.Code != 200 || service.pickupCalls != 1 || service.session != webToken || service.user != 17 || service.token != "synthetic-token" || service.code != "private-synthetic-code" || !service.collect {
		t.Fatalf("wrong authenticated pickup request: status=%d service=%+v", response.Code, service)
	}
	if response.Header().Get("Cache-Control") != "no-store" {
		t.Fatal("private collection response was cacheable")
	}
}

func TestStorePromotionAndPickupRoutesKeepVerifiedActor(t *testing.T) {
	cases := []storeLegacyRouteCase{
		{method: "GET", path: "/api/store/products/product-1/promotions?offset=3&limit=10", action: "promotions", id: "product-1", offset: 3, limit: 10},
		{method: "POST", path: "/api/store/products/product-1/promotions", action: "save_promotion", id: "product-1", body: `{"discount_bps":2500,"variant_ids":[],"expires_at":null,"max_uses":1,"status":"active"}`},
		{method: "PUT", path: "/api/store/products/product-1/promotions/promotion-1", action: "save_promotion", id: "product-1", child: "promotion-1", body: `{"discount_bps":10000}`},
		{method: "POST", path: "/api/store/products/product-1/promotions/batch", action: "promotion_batch", id: "product-1", body: `{"ids":["promotion-1"],"action":"pause"}`},
		{method: "POST", path: "/api/store/products/product-1/promotions/cleanup", action: "promotion_cleanup", id: "product-1", body: `{"limit":100}`},
		{method: "POST", path: "/api/store/products/product-1/promotions/quote", action: "promotion_quote", id: "product-1", body: `{"promotion_code":"coupon","variant_id":"variant-1","quantity":2}`},
		{method: "GET", path: "/api/store/orders/order-1/pickup-link", action: "pickup_link", id: "order-1"},
		{method: "GET", path: "/api/store/products/product-1/variants/variant-1/fixed-content", action: "fixed_content", id: "product-1", child: "variant-1"},
	}
	for _, tc := range cases {
		t.Run(tc.method+" "+tc.path, func(t *testing.T) {
			core, service := &authCore{}, newStoreLegacyTransportFake()
			request := httptest.NewRequest(tc.method, tc.path, strings.NewReader(tc.body))
			request.Header.Set("Authorization", "Bearer "+webToken)
			response := httptest.NewRecorder()
			storeTransportMux(t, core, service).ServeHTTP(response, request)
			if response.Code != 200 || core.calls != 1 || len(service.requests) != 1 || service.actor != (store.Account{Kind: "personal", ID: 17}) {
				t.Fatalf("unverified route status=%d service=%+v", response.Code, service)
			}
			actual := service.requests[0]
			if actual.Action != tc.action || actual.ID != tc.id || actual.ChildID != tc.child || actual.Offset != tc.offset || actual.Limit != tc.limit || string(actual.Body) != tc.body {
				t.Fatalf("lost route arguments: actual=%+v expected=%+v", actual, tc)
			}
		})
	}
}
