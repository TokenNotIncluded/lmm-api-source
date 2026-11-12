package payments

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"sync/atomic"
	"testing"
	"time"
)

func TestPancakePortalUsesAuthenticatedBuyerAndConfiguredStore(t *testing.T) {
	var calls atomic.Int32
	var badResponse atomic.Bool
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		var body map[string]string
		if r.Method != "POST" || r.URL.Path != "/v1/actions/auth/issue-session-token" || json.NewDecoder(r.Body).Decode(&body) != nil ||
			len(body) != 3 || body["storeId"] != "STO_0123456789abcdefghijkl" || body["buyerIdentity"] != "lmm-user-42" || body["purpose"] != "portal" {
			t.Error("portal request did not bind the configured store and current user")
			w.WriteHeader(400)
			return
		}
		if badResponse.Load() {
			w.WriteHeader(502)
			_, _ = w.Write([]byte(`{"errors":[{"message":"synthetic_secret_should_not_escape"}]}`))
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"data": map[string]string{
			"token": "synthetic_portal_token", "expiresAt": time.Now().Add(time.Minute).UTC().Format(time.RFC3339),
			"portalUrl": "https://pancake.waffo.ai/consumer/portal/store/STO_0123456789abcdefghijkl",
		}})
	}))
	defer server.Close()
	endpoint, _ := url.Parse(server.URL)
	client := &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		r = r.Clone(r.Context())
		r.URL.Scheme, r.URL.Host = endpoint.Scheme, endpoint.Host
		return server.Client().Transport.RoundTrip(r)
	})}
	usd, cny := dualPancakeAdapters(t, client)
	svc, err := New(newMemory(), newCore(usd.Channel()), usd, cny)
	if err != nil {
		t.Fatal(err)
	}
	link, err := svc.PancakePortal(context.Background(), 42)
	if err != nil || link == nil || link.PortalURL != "https://pancake.waffo.ai/consumer/portal/store/STO_0123456789abcdefghijkl#token=synthetic_portal_token" || calls.Load() != 1 {
		t.Fatal("server-bound portal unavailable", err)
	}
	if _, err := svc.PancakePortal(context.Background(), 0); !errors.Is(err, ErrDenied) || calls.Load() != 1 {
		t.Fatal("invalid user reached provider", err)
	}
	badResponse.Store(true)
	if _, err := svc.PancakePortal(context.Background(), 42); err != ErrUnavailable {
		t.Fatal("provider failure escaped stable error")
	}
	cny.cfg.StoreID = "STO_abcdefghijkl0123456789"
	if _, err := svc.PancakePortal(context.Background(), 42); !errors.Is(err, ErrConflict) || calls.Load() != 2 {
		t.Fatal("ambiguous stores reached provider", err)
	}
}
