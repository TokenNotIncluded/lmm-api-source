package store

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

type flowSessionAuthority struct {
	bindings map[string]string
	rejected map[string]error
}

func (a *flowSessionAuthority) Check(_ context.Context, token string, account Account, _ Permission) (int64, error) {
	if err := a.rejected[token]; err != nil {
		return 0, err
	}
	if a.bindings[token] == "" {
		return 0, ErrUnauthorized
	}
	if account != (Account{Kind: "personal", ID: 1}) {
		return 0, ErrForbidden
	}
	return 1, nil
}
func (a *flowSessionAuthority) SessionBinding(ctx context.Context, token string, userID int64) (string, error) {
	if _, err := a.Check(ctx, token, Account{Kind: "personal", ID: userID}, Manage); err != nil {
		return "", err
	}
	return a.bindings[token], nil
}

func flowAuthority() *flowSessionAuthority {
	return &flowSessionAuthority{
		bindings: map[string]string{
			"access-before":  "v1:" + strings.Repeat("a", 64),
			"access-after":   "v1:" + strings.Repeat("a", 64),
			"another-login":  "v1:" + strings.Repeat("b", 64),
			"new-generation": "v1:" + strings.Repeat("c", 64),
			"revoked-access": "v1:" + strings.Repeat("a", 64),
		},
		rejected: map[string]error{"revoked-access": ErrUnauthorized},
	}
}

func flowService(t *testing.T) (*Service, *memoryRepo, *atomic.Int32) {
	t.Helper()
	s, repo := extoreServiceWithAuthority(t, flowAuthority())
	tokenCalls := &atomic.Int32{}
	origin := "https://extore.example"
	s.extoreHTTP = &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		var value any
		switch r.URL.Path {
		case "/.well-known/oauth-authorization-server":
			value = map[string]any{
				"issuer": origin, "authorization_endpoint": origin + "/oauth/authorize",
				"token_endpoint":                                 origin + "/api/integrations/commerce/token",
				"revocation_endpoint":                            origin + "/api/integrations/commerce/revoke",
				"response_types_supported":                       []string{"code"},
				"grant_types_supported":                          []string{"authorization_code"},
				"code_challenge_methods_supported":               []string{"S256"},
				"scopes_supported":                               []string{"products.read", "marketplace.sync"},
				"token_endpoint_auth_methods_supported":          []string{"none"},
				"authorization_response_iss_parameter_supported": true,
				"extore_commerce": map[string]any{
					"schema": "extore.commerce-import.v1", "listing_schema": "extore.product-listing.v1",
					"products_endpoint": origin + "/api/integrations/commerce/products",
					"cards_endpoint":    origin + "/api/integrations/commerce/cards",
				},
			}
		case "/api/integrations/commerce/token":
			tokenCalls.Add(1)
			if err := r.ParseForm(); err != nil {
				t.Error(err)
			}
			if r.Form.Get("grant_type") != "authorization_code" || r.Form.Get("code") != "one-time" || len(r.Form.Get("code_verifier")) != 43 {
				t.Error("invalid callback exchange")
			}
			value = map[string]any{
				"access_token": "PRIVATE_ACCESS", "refresh_token": "PRIVATE_REFRESH",
				"token_type": "Bearer", "scope": "products.read marketplace.sync", "expires_in": 600,
				"grant_id": "grant", "grant_expires": time.Now().Unix() + 3600,
			}
		case "/api/integrations/commerce/products":
			if r.Header.Get("Authorization") != "Bearer PRIVATE_ACCESS" {
				t.Error("missing verified connection credential")
			}
			value = map[string]any{
				"schema": "extore.commerce-catalog.v1", "issuer": origin, "grant_id": "grant",
				"shop":     map[string]string{"id": "shop", "name": "Shop"},
				"products": []json.RawMessage{},
			}
		default:
			t.Errorf("unexpected upstream path %s", r.URL.Path)
		}
		raw, err := json.Marshal(value)
		if err != nil {
			t.Error(err)
		}
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(string(raw)))}, nil
	})}
	return s, repo, tokenCalls
}

func startFlow(t *testing.T, s *Service) (string, string) {
	t.Helper()
	start, err := s.ExtoreAuthorize(context.Background(), "access-before", 1, "https://extore.example", "lmm-best-store")
	must(t, err)
	authorization, err := url.Parse(start["authorization_url"].(string))
	must(t, err)
	state := authorization.Query().Get("state")
	callback := "https://shop.example/store/manage?" + url.Values{
		"code": {"one-time"}, "state": {state}, "iss": {"https://extore.example"},
	}.Encode()
	return state, callback
}

func TestExtoreFlowSurvivesVerifiedSessionRefreshAndRetainsFences(t *testing.T) {
	s, repo, calls := flowService(t)
	state, callback := startFlow(t, s)
	ctx := context.Background()
	for _, test := range []struct {
		token string
		user  int64
		want  error
	}{
		{"another-login", 1, ErrConflict},
		{"new-generation", 1, ErrConflict},
		{"revoked-access", 1, ErrUnauthorized},
		{"access-after", 2, ErrForbidden},
	} {
		_, err := s.ExtoreCatalog(ctx, test.token, test.user, callback)
		wantError(t, err, test.want)
	}
	var flow extoreFlowRow
	must(t, repo.Within(ctx, extoreSellerShop(1), func(tx Tx) error { return tx.Get(extoreFlowKind, state, &flow) }))
	if flow.Consumed || calls.Load() != 0 {
		t.Fatal("rejected callback consumed state or exchanged a code")
	}

	result, err := s.ExtoreCatalog(ctx, "access-after", 1, callback)
	must(t, err)
	if calls.Load() != 1 {
		t.Fatal("refresh caused more than one exchange")
	}
	reply, err := json.Marshal(result)
	must(t, err)
	if strings.Contains(string(reply), "PRIVATE_") {
		t.Fatal("connection credentials leaked")
	}

	id := ID(extorePurpose, "connection", int64(1), "https://extore.example", "shop")
	var connection extoreConnectionRow
	must(t, repo.Within(ctx, extoreSellerShop(1), func(tx Tx) error { return tx.Get(extoreConnectionKind, id, &connection) }))
	if connection.Version != 1 || strings.Contains(connection.Ciphertext, "PRIVATE_") {
		t.Fatal("invalid encrypted connection")
	}
	secret, err := s.extore.open(extoreConnectionAAD(id), connection.Ciphertext)
	must(t, err)
	if !strings.Contains(string(secret), "PRIVATE_REFRESH") {
		t.Fatal("lost encrypted token")
	}
	_, err = s.ExtoreCatalog(ctx, "access-after", 1, callback)
	wantError(t, err, ErrConflict)
	if calls.Load() != 1 {
		t.Fatal("callback replay exchanged the code")
	}
}

func TestExtoreFlowExpiryAndMissingSessionContractFailClosed(t *testing.T) {
	s, repo, calls := flowService(t)
	state, callback := startFlow(t, s)
	must(t, repo.Within(context.Background(), extoreSellerShop(1), func(tx Tx) error {
		var row extoreFlowRow
		if err := tx.Get(extoreFlowKind, state, &row); err != nil {
			return err
		}
		row.ExpiresAt = time.Now().Unix() - 1
		return tx.Put(extoreFlowKind, state, row)
	}))
	_, err := s.ExtoreCatalog(context.Background(), "access-after", 1, callback)
	wantError(t, err, ErrConflict)
	if calls.Load() != 0 {
		t.Fatal("expired flow reached token exchange")
	}

	old, _ := extoreService(t) // Authority.Check exists, stable-session contract does not.
	_, err = old.ExtoreAuthorize(context.Background(), sellerToken, 1, "https://extore.example", "lmm-best-store")
	wantError(t, err, ErrUnavailable)
}

func TestExtoreConcurrentRefreshedCallbacksConsumeOnce(t *testing.T) {
	s, _, calls := flowService(t)
	_, callback := startFlow(t, s)
	var wait sync.WaitGroup
	errorsSeen := make(chan error, 2)
	for _, token := range []string{"access-before", "access-after"} {
		wait.Add(1)
		go func(token string) {
			defer wait.Done()
			_, err := s.ExtoreCatalog(context.Background(), token, 1, callback)
			errorsSeen <- err
		}(token)
	}
	wait.Wait()
	close(errorsSeen)
	success, conflicts := 0, 0
	for err := range errorsSeen {
		if err == nil {
			success++
		} else if errors.Is(err, ErrConflict) {
			conflicts++
		} else {
			t.Fatal(err)
		}
	}
	if success != 1 || conflicts != 1 || calls.Load() != 1 {
		t.Fatalf("success=%d conflicts=%d exchanges=%d", success, conflicts, calls.Load())
	}
}
