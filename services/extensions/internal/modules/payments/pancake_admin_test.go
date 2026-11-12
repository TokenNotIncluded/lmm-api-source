package payments

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
)

func TestPancakeAdministrationCatalogAndPairUseBoundedSDK(t *testing.T) {
	var failProduct atomic.Bool
	var failGraphQL atomic.Bool
	var mutations atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		if json.NewDecoder(r.Body).Decode(&body) != nil {
			t.Error("invalid SDK request")
			return
		}
		var data any
		switch r.URL.Path {
		case "/v1/graphql":
			if failGraphQL.Load() {
				_ = json.NewEncoder(w).Encode(map[string]any{"errors": []map[string]string{{"message": "synthetic_private_error"}}})
				return
			}
			if strings.Contains(body["query"].(string), "onetimeProducts") {
				variables, _ := body["variables"].(map[string]any)
				if variables["storeId"] != "STO_0123456789abcdefghijkl" {
					t.Error("catalog lost store binding")
				}
				data = map[string]any{"onetimeProducts": []map[string]string{{"id": "PROD_0123456789abcdefghijkl", "name": "Wallet", "status": "active"}, {"id": "PROD_inactive", "status": "inactive"}}, "subscriptionProducts": []map[string]string{{"id": "PROD_subscription", "name": "Plan", "status": "active", "billingPeriod": "monthly"}}}
			} else {
				data = map[string]any{"stores": []map[string]any{{"id": "STO_0123456789abcdefghijkl", "name": "LMM API", "status": "active", "prodEnabled": true}}}
			}
		case "/v1/actions/store/create-store":
			mutations.Add(1)
			if !strings.HasPrefix(r.Header.Get("X-Idempotency-Key"), "lmm-store-") {
				t.Error("store mutation lost stable key")
			}
			data = map[string]any{"store": map[string]string{"id": "STO_0123456789abcdefghijkl", "name": "LMM API"}}
		case "/v1/actions/onetime-product/create-product":
			mutations.Add(1)
			if !strings.HasPrefix(r.Header.Get("X-Idempotency-Key"), "lmm-product-") || body["storeId"] != "STO_0123456789abcdefghijkl" || body["successUrl"] != "https://example.test/wallet" {
				t.Error("product lost binding or stable key")
			}
			if failProduct.Load() {
				w.WriteHeader(503)
				_, _ = w.Write([]byte(`{"errors":[{"message":"synthetic_private_error"}]}`))
				return
			}
			data = map[string]any{"product": map[string]string{"id": "PROD_0123456789abcdefghijkl", "storeId": "STO_0123456789abcdefghijkl", "name": "Wallet", "status": "active"}}
		case "/v1/actions/onetime-product/publish-product":
			mutations.Add(1)
			if !strings.HasPrefix(r.Header.Get("X-Idempotency-Key"), "lmm-publish-") || body["id"] != "PROD_0123456789abcdefghijkl" {
				t.Error("publication lost product binding")
			}
			data = map[string]any{"product": map[string]string{"id": "PROD_0123456789abcdefghijkl", "storeId": "STO_0123456789abcdefghijkl", "name": "Wallet", "status": "active"}}
		default:
			t.Error("unexpected provider path", r.URL.Path)
			w.WriteHeader(404)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"data": data})
	}))
	defer server.Close()
	endpoint, _ := url.Parse(server.URL)
	client := &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		r = r.Clone(r.Context())
		r.URL.Scheme, r.URL.Host = endpoint.Scheme, endpoint.Host
		return server.Client().Transport.RoundTrip(r)
	})}
	p, _ := dualPancakeAdapters(t, client)
	catalog, err := p.AdminCatalog(context.Background())
	if err != nil || len(catalog.Stores) != 1 || len(catalog.Stores[0].OnetimeProducts) != 1 || len(catalog.Stores[0].SubscriptionProducts) != 1 || mutations.Load() != 0 {
		t.Fatal("catalog did not preserve active product types", err)
	}
	paired, err := p.AdminPair(context.Background(), "https://example.test/wallet", []string{"USD", "CNY"})
	if err != nil || paired.OrphanStore || paired.ProductID != "PROD_0123456789abcdefghijkl" || mutations.Load() != 3 {
		t.Fatal("pair not published", err)
	}
	failProduct.Store(true)
	partial, err := p.AdminPair(context.Background(), "https://example.test/wallet", []string{"USD", "CNY"})
	if !errors.Is(err, ErrUnavailable) || partial == nil || !partial.OrphanStore || partial.StoreID != "STO_0123456789abcdefghijkl" || mutations.Load() != 5 {
		t.Fatal("lost partial provider result", err)
	}
	failGraphQL.Store(true)
	if _, err := p.AdminCatalog(context.Background()); err != ErrUnavailable {
		t.Fatal("GraphQL provider error escaped")
	}
	if _, err := p.AdminPair(context.Background(), "http://localhost/wallet", []string{"USD"}); !errors.Is(err, ErrInvalid) || mutations.Load() != 5 {
		t.Fatal("invalid return target reached provider")
	}
}
