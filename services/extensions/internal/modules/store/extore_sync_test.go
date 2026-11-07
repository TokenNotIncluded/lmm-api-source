package store

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/extore"
)

func extoreService(t *testing.T) (*Service, *memoryRepo) {
	t.Helper()
	repo := newRepo()
	s, err := New(repo, &authority{}, nil)
	must(t, err)
	key := make([]byte, 32)
	if _, err := rand.Read(key); err != nil {
		t.Fatal(err)
	}
	must(t, s.WithExtore("https://shop.example/store/manage", key))
	return s, repo
}

func TestExtoreUSDCredits(t *testing.T) {
	got, err := extoreUSDCredits("1.00")
	if err != nil || got != 500000 {
		t.Fatalf("1.00 -> %d %v", got, err)
	}
	got, err = extoreUSDCredits("2.5")
	if err != nil || got != 1250000 {
		t.Fatalf("2.5 -> %d %v", got, err)
	}
	for _, bad := range []string{"", "1.2345678", "0", "-1", "abc", "1e3"} {
		if _, err := extoreUSDCredits(bad); !errors.Is(err, ErrInvalid) {
			t.Fatalf("%q accepted: %v", bad, err)
		}
	}
}

func TestExtoreDeliveryTemplate(t *testing.T) {
	fixed := "secret delivery"
	empty := ""
	if err := extoreDeliveryTemplate("card-key", nil); err != nil {
		t.Fatal(err)
	}
	if err := extoreDeliveryTemplate("card-key", &fixed); !errors.Is(err, ErrInvalid) {
		t.Fatal(err)
	}
	if err := extoreDeliveryTemplate("fixed-content", &fixed); err != nil {
		t.Fatal(err)
	}
	if err := extoreDeliveryTemplate("fixed-content", &empty); !errors.Is(err, ErrInvalid) {
		t.Fatal(err)
	}
	if err := extoreDeliveryTemplate("card", nil); !errors.Is(err, ErrInvalid) {
		t.Fatal(err)
	}
}

func listingRaw(t *testing.T, sourceID, revision string, priceQuota int64, template string, fixed *string, summary json.RawMessage) json.RawMessage {
	t.Helper()
	variantMarket := map[string]any{"price_quota": priceQuota, "template": template}
	if fixed != nil {
		variantMarket["fixed_content"] = *fixed
	}
	listing := map[string]any{
		"schema": "extore.product-listing.v1", "id": sourceID, "shop_id": "shop",
		"revision": revision, "sync_revision": revision,
		"redemption_url": "https://extore.example/",
		"semantics": map[string]any{
			"price": "reference", "inventory": "not_exported",
			"payment": "external_sales_platform", "redemption": "extore",
		},
		"product": map[string]any{
			"name": "Notes", "description": "Body", "public": true,
			"marketplace": map[string]any{
				"title": "Notes", "description": "Body", "image_urls": []string{},
				"contact": "", "links": []any{}, "payment_methods": []string{"balance"},
				"pickup_login_required": true, "delivery_strategy": "sequential",
			},
		},
		"variants": []any{
			map[string]any{
				"id": "basic", "name": "Basic", "enabled": true,
				"marketplace": variantMarket,
			},
		},
		"card_summary": summary,
	}
	raw, err := json.Marshal(listing)
	must(t, err)
	return raw
}

func seedConnection(t *testing.T, s *Service, userID int64, accessExpires int64) extoreConnectionRow {
	t.Helper()
	credentials := extore.Credentials{
		Access: "access", Refresh: "refresh", Type: "Bearer",
		Scope: "products.read marketplace.sync", Expires: 600,
		GrantID: "grant", GrantExpires: float64(time.Now().Unix() + 3600),
		AccessExpires: accessExpires,
	}
	secret, err := json.Marshal(credentials)
	must(t, err)
	connectionID := ID(extorePurpose, "connection", userID, "https://extore.example", "shop")
	cipher, err := s.extore.seal(extoreConnectionAAD(connectionID), secret)
	must(t, err)
	row := extoreConnectionRow{
		ID: connectionID, Origin: "https://extore.example", ShopID: "shop", ShopName: "Shop",
		ClientID: "lmm-best-store", Ciphertext: cipher, ExpiresAt: int64(credentials.GrantExpires),
		Version: 1, UpdatedAt: time.Now().Unix(),
	}
	must(t, s.repo.Within(context.Background(), extoreSellerShop(userID), func(tx Tx) error {
		return tx.Put(extoreConnectionKind, connectionID, row)
	}))
	return row
}

func TestExtorePersistAndRevisionFence(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	connection := seedConnection(t, s, userID, time.Now().Unix()+600)
	revision := strings.Repeat("a", 64)
	fixed := "PRIVATE"
	raw := listingRaw(t, "document", revision, 500000, "fixed-content", &fixed, json.RawMessage(`{"total":1,"states":{"available":1}}`))
	source, err := s.syncExtoreListing(ctx, userID, connection.ID, raw)
	must(t, err)
	if source.ProductID == "" || source.Revision != revision || source.LastSyncedAt == 0 {
		t.Fatalf("%+v", source)
	}
	firstProduct := source.ProductID
	firstSynced := source.LastSyncedAt
	updated := listingRaw(t, "document", revision, 500000, "fixed-content", &fixed, json.RawMessage(`{"total":2,"states":{"available":2}}`))
	again, err := s.syncExtoreListing(ctx, userID, connection.ID, updated)
	must(t, err)
	if again.ProductID != firstProduct || again.Revision != revision {
		t.Fatal("same revision must keep product identity")
	}
	if !bytes.Equal(again.CardSummary, []byte(`{"total":2,"states":{"available":2}}`)) {
		t.Fatalf("card summary not refreshed: %s", again.CardSummary)
	}
	if again.LastSyncedAt < firstSynced {
		t.Fatal("last synced must move forward")
	}
	legacy, err := s.legacyShop(ctx, Account{Kind: "personal", ID: userID})
	must(t, err)
	var product Product
	must(t, s.repo.Within(ctx, legacy.ID, func(tx Tx) error {
		return tx.Get("products", firstProduct, &product)
	}))
	if product.Legacy == nil || product.Legacy.Status != "draft" {
		t.Fatalf("unexpected status %+v", product.Legacy)
	}
	product.Legacy.Status = "pending"
	must(t, s.repo.Within(ctx, legacy.ID, func(tx Tx) error {
		return tx.Put("products", product.ID, product)
	}))
	nextRev := strings.Repeat("b", 64)
	_, err = s.syncExtoreListing(ctx, userID, connection.ID, listingRaw(t, "document", nextRev, 600000, "fixed-content", &fixed, json.RawMessage(`{}`)))
	wantError(t, err, ErrConflict)
}

func TestExtoreSourceManagedBlocksLocalEdits(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	actor := Account{Kind: "personal", ID: userID}
	connection := seedConnection(t, s, userID, time.Now().Unix()+600)
	fixed := "PRIVATE"
	source, err := s.syncExtoreListing(ctx, userID, connection.ID, listingRaw(t, "document", strings.Repeat("c", 64), 500000, "fixed-content", &fixed, json.RawMessage(`{}`)))
	must(t, err)
	wantError(t, s.requireLocalProductSource(ctx, actor, source.ProductID), ErrExtoreSourceManaged)
	if err := s.requireLocalProductSource(ctx, actor, "unrelated"); err != nil {
		t.Fatal(err)
	}
}

func TestExtoreRefreshFence(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	row := seedConnection(t, s, userID, time.Now().Unix()-10)
	credentials, err := s.openExtoreCredentials(row)
	must(t, err)
	calls := 0
	s.extoreHTTP = &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		calls++
		if r.URL.Path != "/api/integrations/commerce/token" {
			t.Fatalf("unexpected path %s", r.URL.Path)
		}
		body, _ := io.ReadAll(r.Body)
		if !strings.Contains(string(body), "refresh_token=refresh") {
			t.Fatalf("missing refresh token: %s", body)
		}
		payload, _ := json.Marshal(extore.Credentials{
			Access: "next-access", Refresh: "next-refresh", Type: "Bearer",
			Scope: "products.read marketplace.sync", Expires: 600,
			GrantID: "grant", GrantExpires: float64(time.Now().Unix() + 7200),
		})
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(bytes.NewReader(payload))}, nil
	})}
	oldCipher := row.Ciphertext
	next, err := s.refreshExtoreCredentials(ctx, userID, &row, credentials)
	must(t, err)
	if next.Access != "next-access" || next.Refresh != "next-refresh" || calls != 1 {
		t.Fatalf("%+v calls=%d", next, calls)
	}
	var stored extoreConnectionRow
	must(t, s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
		return tx.Get(extoreConnectionKind, row.ID, &stored)
	}))
	if stored.RefreshPending || stored.Version != 2 || stored.Ciphertext == oldCipher {
		t.Fatalf("%+v", stored)
	}
	opened, err := s.openExtoreCredentials(stored)
	must(t, err)
	if opened.Access != "next-access" {
		t.Fatal(opened.Access)
	}
	stored.RefreshPending = true
	must(t, s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
		return tx.Put(extoreConnectionKind, stored.ID, stored)
	}))
	_, err = s.openExtoreCredentials(stored)
	wantError(t, err, ErrExtoreReauthorize)
}

func TestExtoreRefreshFailureLeavesPending(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	row := seedConnection(t, s, userID, time.Now().Unix()-10)
	credentials, err := s.openExtoreCredentials(row)
	must(t, err)
	s.extoreHTTP = &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: 500, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(`{}`))}, nil
	})}
	_, err = s.refreshExtoreCredentials(ctx, userID, &row, credentials)
	wantError(t, err, ErrExtoreReauthorize)
	var stored extoreConnectionRow
	must(t, s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
		return tx.Get(extoreConnectionKind, row.ID, &stored)
	}))
	if !stored.RefreshPending || stored.Version != 1 {
		t.Fatalf("failed refresh must leave pending fence: %+v", stored)
	}
	_, err = s.openExtoreCredentials(stored)
	wantError(t, err, ErrExtoreReauthorize)
}

func TestExtoreRejectsMissingVariantPrice(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	connection := seedConnection(t, s, userID, time.Now().Unix()+600)
	raw := listingRaw(t, "document", strings.Repeat("e", 64), 0, "card-key", nil, json.RawMessage(`{}`))
	_, err := s.syncExtoreListing(ctx, userID, connection.ID, raw)
	if err == nil || !strings.HasPrefix(err.Error(), "Configure the selling price") {
		t.Fatalf("want missing price error, got %v", err)
	}
}

func TestExtoreConnectionsList(t *testing.T) {
	s, _ := extoreService(t)
	ctx := context.Background()
	userID := int64(1)
	connection := seedConnection(t, s, userID, time.Now().Unix()+600)
	fixed := "PRIVATE"
	_, err := s.syncExtoreListing(ctx, userID, connection.ID, listingRaw(t, "document", strings.Repeat("d", 64), 500000, "fixed-content", &fixed, json.RawMessage(`{}`)))
	must(t, err)
	out, err := s.ExtoreConnections(ctx, sellerToken, userID)
	must(t, err)
	connections := out["connections"].([]map[string]any)
	sources := out["sources"].([]map[string]any)
	if len(connections) != 1 || connections[0]["id"] != connection.ID {
		t.Fatalf("%+v", connections)
	}
	if len(sources) != 1 || sources[0]["source_id"] != "document" || sources[0]["connection_id"] != connection.ID {
		t.Fatalf("%+v", sources)
	}
	if _, ok := connections[0]["ciphertext"]; ok {
		t.Fatal("ciphertext must never leave the connection list")
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (fn roundTripFunc) RoundTrip(r *http.Request) (*http.Response, error) { return fn(r) }
