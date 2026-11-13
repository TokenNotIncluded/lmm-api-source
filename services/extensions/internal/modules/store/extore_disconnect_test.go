package store

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)

// readOnlyAuthority denies every Manage check so tests can prove the
// disconnect path demands the Manage permission, not just a valid session.
type readOnlyAuthority struct{ authority }

func (a *readOnlyAuthority) Check(ctx context.Context, token string, account Account, p Permission) (int64, error) {
	if p != Manage {
		return a.authority.Check(ctx, token, account, p)
	}
	return 0, ErrForbidden
}

func extoreServiceWithAuthority(t *testing.T, auth Authority) (*Service, *memoryRepo) {
	t.Helper()
	repo := newRepo()
	s, err := New(repo, auth, nil)
	must(t, err)
	key := make([]byte, 32)
	if _, err := rand.Read(key); err != nil {
		t.Fatal(err)
	}
	must(t, s.WithExtore("https://shop.example/store/manage", key))
	return s, repo
}

func revokeTransport(t *testing.T, status int, calls *int) *http.Client {
	t.Helper()
	return &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		*calls++
		if r.URL.Path != "/api/integrations/commerce/revoke" {
			t.Fatalf("unexpected upstream path %s", r.URL.Path)
		}
		body, _ := io.ReadAll(r.Body)
		if !strings.Contains(string(body), "token=refresh") || !strings.Contains(string(body), "token_type_hint=refresh_token") {
			t.Fatalf("missing revoke form: %s", body)
		}
		return &http.Response{StatusCode: status, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(`{}`))}, nil
	})}
}

func TestExtoreDisconnectRevokesAndDeactivates(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	connection := seedConnection(t, s, userID, time.Now().Unix()+600)
	fixed := "PRIVATE"
	source, err := s.syncExtoreListing(ctx, userID, connection.ID, listingRaw(t, "document", strings.Repeat("a", 64), 500000, "fixed-content", &fixed, json.RawMessage(`{}`)))
	must(t, err)
	calls := 0
	s.extoreHTTP = revokeTransport(t, http.StatusOK, &calls)
	value, err := s.ExtoreDisconnect(ctx, sellerToken, userID, connection.ID)
	must(t, err)
	if calls != 1 {
		t.Fatal(calls)
	}
	if value.(map[string]any)["id"] != connection.ID {
		t.Fatalf("%+v", value)
	}
	var stored extoreConnectionRow
	must(t, s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
		return tx.Get(extoreConnectionKind, connection.ID, &stored)
	}))
	if stored.RevokedAt == 0 {
		t.Fatal("connection must be explicitly deactivated")
	}
	out, err := s.ExtoreConnections(ctx, sellerToken, userID)
	must(t, err)
	if connections := out["connections"].([]map[string]any); len(connections) != 0 {
		t.Fatalf("revoked connection still listed: %+v", connections)
	}
	sources := out["sources"].([]map[string]any)
	if len(sources) != 1 || sources[0]["product_id"] != source.ProductID {
		t.Fatalf("source history must be preserved: %+v", sources)
	}
	legacy, err := s.legacyShop(ctx, Account{Kind: "personal", ID: userID})
	must(t, err)
	var product Product
	must(t, s.repo.Within(ctx, legacy.ID, func(tx Tx) error {
		return tx.Get("products", source.ProductID, &product)
	}))
	if product.ID != source.ProductID {
		t.Fatal("imported product must survive the disconnect")
	}
	_, err = s.ExtoreConnectionCatalog(ctx, sellerToken, userID, connection.ID)
	wantError(t, err, ErrExtoreReauthorize)
}

func TestExtoreDisconnectUpstreamFailureKeepsConnection(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	connection := seedConnection(t, s, userID, time.Now().Unix()+600)
	for _, failure := range []struct {
		name   string
		status int
		broken bool
	}{
		{"status", http.StatusBadGateway, false},
		{"transport", 0, true},
	} {
		t.Run(failure.name, func(t *testing.T) {
			calls := 0
			if failure.broken {
				s.extoreHTTP = &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
					calls++
					return nil, io.ErrUnexpectedEOF
				})}
			} else {
				s.extoreHTTP = revokeTransport(t, failure.status, &calls)
			}
			_, err := s.ExtoreDisconnect(ctx, sellerToken, userID, connection.ID)
			wantError(t, err, ErrUnavailable)
			if calls != 1 {
				t.Fatal(calls)
			}
			var stored extoreConnectionRow
			must(t, s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
				return tx.Get(extoreConnectionKind, connection.ID, &stored)
			}))
			if stored.RevokedAt != 0 {
				t.Fatal("failed revoke must not report the connection as disconnected")
			}
			out, err := s.ExtoreConnections(ctx, sellerToken, userID)
			must(t, err)
			if connections := out["connections"].([]map[string]any); len(connections) != 1 {
				t.Fatalf("connection must stay recoverable: %+v", connections)
			}
		})
	}
}

func TestExtoreDisconnectRequiresManageAndOwnership(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	connection := seedConnection(t, s, userID, time.Now().Unix()+600)
	calls := 0
	s.extoreHTTP = revokeTransport(t, http.StatusOK, &calls)
	if _, err := s.ExtoreDisconnect(ctx, buyerToken, 2, connection.ID); err == nil {
		t.Fatal("cross-user disconnect accepted")
	}
	if calls != 0 {
		t.Fatal("cross-user attempt must not reach the upstream")
	}
	var stored extoreConnectionRow
	must(t, s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
		return tx.Get(extoreConnectionKind, connection.ID, &stored)
	}))
	if stored.RevokedAt != 0 {
		t.Fatal("cross-user attempt changed the connection")
	}
	if _, err := s.ExtoreDisconnect(ctx, "bad-token", userID, connection.ID); err == nil {
		t.Fatal("invalid session accepted")
	}

	ro, _ := extoreServiceWithAuthority(t, &readOnlyAuthority{})
	roConnection := seedConnection(t, ro, userID, time.Now().Unix()+600)
	roCalls := 0
	ro.extoreHTTP = revokeTransport(t, http.StatusOK, &roCalls)
	_, err := ro.ExtoreDisconnect(ctx, sellerToken, userID, roConnection.ID)
	wantError(t, err, ErrForbidden)
	if roCalls != 0 {
		t.Fatal("read-only session must not reach the upstream")
	}
}

func TestExtoreDisconnectRejectsUnusableCredentials(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	connection := seedConnection(t, s, userID, time.Now().Unix()+600)
	calls := 0
	s.extoreHTTP = revokeTransport(t, http.StatusOK, &calls)
	must(t, s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
		var row extoreConnectionRow
		if err := tx.Get(extoreConnectionKind, connection.ID, &row); err != nil {
			return err
		}
		row.RefreshPending = true
		return tx.Put(extoreConnectionKind, connection.ID, row)
	}))
	_, err := s.ExtoreDisconnect(ctx, sellerToken, userID, connection.ID)
	wantError(t, err, ErrExtoreReauthorize)
	if calls != 0 {
		t.Fatal("unusable credentials must not be sent upstream")
	}
	var stored extoreConnectionRow
	must(t, s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
		return tx.Get(extoreConnectionKind, connection.ID, &stored)
	}))
	if stored.RevokedAt != 0 {
		t.Fatal("connection state changed without an upstream revoke")
	}
}

func TestExtoreDisconnectUsesRefreshCredentialWithoutAccess(t *testing.T) {
	for _, mode := range []string{"missing_access", "expired_access"} {
		t.Run(mode, func(t *testing.T) {
			s, _ := extoreService(t)
			ctx := context.Background()
			connection := seedConnection(t, s, 1, time.Now().Unix()+600)
			credentials, err := s.decryptExtoreCredentials(connection)
			must(t, err)
			if mode == "missing_access" {
				credentials.Access = ""
			} else {
				credentials.AccessExpires = time.Now().Unix() - 60
			}
			secret, err := json.Marshal(credentials)
			must(t, err)
			connection.Ciphertext, err = s.extore.seal(extoreConnectionAAD(connection.ID), secret)
			must(t, err)
			must(t, s.repo.Within(ctx, extoreSellerShop(1), func(tx Tx) error {
				return tx.Put(extoreConnectionKind, connection.ID, connection)
			}))
			calls := 0
			s.extoreHTTP = revokeTransport(t, http.StatusOK, &calls)
			if mode == "missing_access" {
				_, err := s.ExtoreConnectionCatalog(ctx, sellerToken, 1, connection.ID)
				wantError(t, err, ErrExtoreReauthorize)
				if calls != 0 {
					t.Fatal("sync without access credentials reached upstream")
				}
			}
			_, err = s.ExtoreDisconnect(ctx, sellerToken, 1, connection.ID)
			must(t, err)
			if calls != 1 {
				t.Fatalf("expected only the revoke request, got %d requests", calls)
			}
			stored, err := s.readExtoreConnection(ctx, 1, connection.ID)
			must(t, err)
			if stored.RevokedAt == 0 {
				t.Fatal("successful revocation did not deactivate the connection")
			}
		})
	}
}
