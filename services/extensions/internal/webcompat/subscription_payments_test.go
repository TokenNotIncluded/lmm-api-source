// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestSubscriptionPaymentMountUsesCurrentSessionAndHostAdmission(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	calls := 0
	service := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		if r.URL.Path != "/orders/original" || r.Header.Get("X-LMM-User-Credential") != webToken {
			t.Fatal("original order or authenticated session changed")
		}
		w.WriteHeader(http.StatusNoContent)
	})
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{SubscriptionPayments: service})
	request := func(token string) int {
		r := httptest.NewRequest("GET", "/api/user/payments/subscriptions/orders/original", nil)
		if token != "" {
			r.Header.Set("Authorization", "Bearer "+token)
		}
		r.Header.Set("X-LMM-User-Credential", "untrusted-user-header")
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, r)
		return w.Code
	}
	if request(webToken) != 503 || calls != 0 || core.calls != 0 {
		t.Fatal("unready host admitted request")
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	for _, token := range []string{"", hostToken} {
		if request(token) != 401 || calls != 0 {
			t.Fatal("missing user authority reached subscription handler")
		}
	}
	if request(webToken) != 204 || calls != 1 || core.calls != 1 {
		t.Fatal("current session did not reach subscription handler")
	}
	host.BeginDrain()
	if request(webToken) != 503 || calls != 1 {
		t.Fatal("draining host admitted subscription work")
	}
}
