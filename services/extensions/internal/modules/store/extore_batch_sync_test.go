package store

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)

func TestExtoreSyncConnectionReportsPartialResultsFromOneCatalog(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	row := seedConnection(t, s, 1, time.Now().Unix()+600)
	good := listingRaw(t, "document", strings.Repeat("a", 64), 500000, "card-key", nil, json.RawMessage(`{"total":2}`))
	bad := listingRaw(t, "unpriced", strings.Repeat("b", 64), 0, "card-key", nil, json.RawMessage(`{}`))
	payload, err := json.Marshal(map[string]any{
		"schema": "extore.commerce-catalog.v1", "issuer": row.Origin,
		"grant_id": "grant", "shop": map[string]string{"id": "shop", "name": "Shop"},
		"products": []json.RawMessage{good, bad},
	})
	must(t, err)
	calls := 0
	s.extoreHTTP = &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		calls++
		if r.URL.Path != "/api/integrations/commerce/products" || r.Header.Get("Authorization") != "Bearer access" {
			t.Fatal("unexpected upstream request")
		}
		return &http.Response{StatusCode: http.StatusOK, Header: make(http.Header), Body: io.NopCloser(bytes.NewReader(payload))}, nil
	})}
	value, err := s.ExtoreSyncConnection(ctx, sellerToken, 1, row.ID)
	must(t, err)
	result := value.(map[string]any)
	items := result["results"].([]ExtoreSyncResult)
	if calls != 1 || result["connection_id"] != row.ID || len(items) != 2 {
		t.Fatalf("calls=%d result=%+v", calls, result)
	}
	if !items[0].Success || items[0].ProductID == "" || items[1].Success || items[1].ProductID != "" {
		t.Fatalf("unexpected per-source outcomes: %+v", items)
	}
	if _, err := s.ExtoreSyncConnection(ctx, buyerToken, 2, row.ID); err == nil || calls != 1 {
		t.Fatal("cross-seller sync reached the upstream")
	}
}

func TestExtoreImportAndBatchSyncRequireManage(t *testing.T) {
	s, _ := extoreServiceWithAuthority(t, &readOnlyAuthority{})
	ctx := context.Background()
	row := seedConnection(t, s, 1, time.Now().Unix()+600)
	calls := 0
	s.extoreHTTP = &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
		calls++
		t.Fatal("read-only import reached the upstream")
		return nil, nil
	})}
	_, err := s.ExtoreAuthorize(ctx, sellerToken, 1, "https://extore.example", "lmm-best-store")
	wantError(t, err, ErrForbidden)
	_, err = s.ExtoreCatalog(ctx, sellerToken, 1, "https://shop.example/store/manage")
	wantError(t, err, ErrForbidden)
	_, err = s.ExtoreSyncConnection(ctx, sellerToken, 1, row.ID)
	wantError(t, err, ErrForbidden)
	if calls != 0 {
		t.Fatal("read-only mutation made an upstream request")
	}
}

func TestExtoreConnectionsExposeRecoveryStatusWithoutCredentials(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	row := seedConnection(t, s, 1, time.Now().Unix()+600)
	for _, state := range []struct {
		status           string
		expired, pending bool
	}{
		{"connected", false, false},
		{"expired", true, false},
		{"reauthorization_required", false, true},
	} {
		row.ExpiresAt = time.Now().Unix() + 3600
		if state.expired {
			row.ExpiresAt = time.Now().Unix() - 1
		}
		row.RefreshPending = state.pending
		must(t, s.repo.Within(ctx, extoreSellerShop(1), func(tx Tx) error {
			return tx.Put(extoreConnectionKind, row.ID, row)
		}))
		result, err := s.ExtoreConnections(ctx, sellerToken, 1)
		must(t, err)
		rows := result["connections"].([]map[string]any)
		if len(rows) != 1 || rows[0]["status"] != state.status {
			t.Fatalf("unexpected status: %+v", result)
		}
		raw, err := json.Marshal(result)
		must(t, err)
		for _, secret := range []string{"ciphertext", "access_token", "refresh_token"} {
			if strings.Contains(string(raw), secret) {
				t.Fatalf("credential field exposed: %s", secret)
			}
		}
	}
}
