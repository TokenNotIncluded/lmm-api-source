package store

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"net/http"
	"os"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/extore"
)

// Run with the real Rust Extore fixture in packages/extore/tests/
// native_plugin_probe.py. The fixture owns isolated SQLite, PKCE/consent, TLS
// and two distinct shops. Store uses its existing transactional memory test
// repository and authority double; this is not independent CoreSession proof.
func TestNativePluginCatalogStore(t *testing.T) {
	configPath := os.Getenv("EXTORE_PLUGIN_PROBE_CONFIG")
	if configPath == "" {
		t.Skip("requires the real Extore native_plugin_probe.py fixture")
	}
	type shop struct {
		ID                  string             `json:"id"`
		Name                string             `json:"name"`
		ClientID            string             `json:"client_id"`
		ProductID           string             `json:"product_id"`
		UnapprovedProductID string             `json:"unapproved_product_id"`
		Credentials         extore.Credentials `json:"credentials"`
	}
	var cfg struct {
		Origin        string `json:"origin"`
		RootCAFile    string `json:"root_ca_file"`
		HostLibrary   string `json:"host_library"`
		PluginLibrary string `json:"plugin_library"`
		ReportPath    string `json:"report_path"`
		ShopA         shop   `json:"shop_a"`
		ShopB         shop   `json:"shop_b"`
	}
	raw, err := os.ReadFile(configPath)
	must(t, err)
	must(t, json.Unmarshal(raw, &cfg))
	if cfg.ShopA.ID == "" || cfg.ShopB.ID == "" || cfg.ShopA.ID == cfg.ShopB.ID {
		t.Fatal("fixture must have two distinct real Extore shops")
	}
	certificate, err := os.ReadFile(cfg.RootCAFile)
	must(t, err)
	roots := x509.NewCertPool()
	if !roots.AppendCertsFromPEM(certificate) {
		t.Fatal("fixture CA is invalid")
	}
	transport := &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}}
	defer transport.CloseIdleConnections()
	var httpCatalogCalls atomic.Int32
	httpClient := &http.Client{
		Timeout:       20 * time.Second,
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
		Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
			if request.URL.Path == "/api/integrations/commerce/products" {
				httpCatalogCalls.Add(1)
				return nil, errors.New("catalog must use the configured native plugin")
			}
			return transport.RoundTrip(request)
		}),
	}
	reader, err := extore.NewNativeCatalogReader(extore.NativeConfig{
		HostLibrary: cfg.HostLibrary, PluginLibrary: cfg.PluginLibrary, CAFile: cfg.RootCAFile,
	})
	must(t, err)
	defer reader.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	client := extore.Client{HTTP: httpClient, Catalog: reader}
	for _, item := range []shop{cfg.ShopA, cfg.ShopB} {
		catalog, err := client.ConnectionCatalog(ctx, cfg.Origin, item.Credentials)
		must(t, err)
		if catalog.Shop.ID != item.ID || len(catalog.Products) != 1 {
			t.Fatal("native catalog escaped the approved shop or product set")
		}
		var listing struct {
			ID     string `json:"id"`
			ShopID string `json:"shop_id"`
		}
		must(t, json.Unmarshal(catalog.Products[0], &listing))
		if listing.ID != item.ProductID || listing.ID == item.UnapprovedProductID || listing.ShopID != item.ID {
			t.Fatal("native catalog returned an unapproved product")
		}
	}
	invalid := cfg.ShopA.Credentials
	invalid.Access = "isolated-native-probe-invalid-token"
	_, err = client.ConnectionCatalog(ctx, cfg.Origin, invalid)
	wantError(t, err, extore.ErrDenied)
	wrongGrant := cfg.ShopA.Credentials
	wrongGrant.GrantID = cfg.ShopB.Credentials.GrantID
	_, err = client.ConnectionCatalog(ctx, cfg.Origin, wrongGrant)
	wantError(t, err, extore.ErrProtocol)

	// Concurrent callers must still use one OS thread for load/call/free/close.
	// The real Rust bridge returns an error for any thread-affinity violation.
	var group sync.WaitGroup
	results := make(chan error, 4)
	for i := range 4 {
		item := []shop{cfg.ShopA, cfg.ShopB}[i%2]
		group.Go(func() {
			_, err := client.ConnectionCatalog(ctx, cfg.Origin, item.Credentials)
			results <- err
		})
	}
	group.Wait()
	close(results)
	for err := range results {
		must(t, err)
	}
	service, _ := extoreService(t)
	service.extoreHTTP = httpClient
	must(t, service.WithExtoreCatalogReader(reader))
	seed := func(userID int64, item shop) extoreConnectionRow {
		t.Helper()
		credentials := item.Credentials
		credentials.AccessExpires = time.Now().Unix() + int64(credentials.Expires)
		secret, err := json.Marshal(credentials)
		must(t, err)
		id := ID(extorePurpose, "connection", userID, cfg.Origin, item.ID)
		sealed, err := service.extore.seal(extoreConnectionAAD(id), secret)
		must(t, err)
		row := extoreConnectionRow{
			ID: id, Origin: cfg.Origin, ShopID: item.ID, ShopName: item.Name,
			ClientID: item.ClientID, Ciphertext: sealed, ExpiresAt: int64(credentials.GrantExpires),
			Version: 1, UpdatedAt: time.Now().Unix(),
		}
		must(t, service.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
			return tx.Put(extoreConnectionKind, id, row)
		}))
		return row
	}
	rowA, rowB := seed(1, cfg.ShopA), seed(2, cfg.ShopB)
	localShop, err := service.legacyShop(ctx, Account{Kind: "personal", ID: 1})
	must(t, err)
	for _, item := range []shop{cfg.ShopA, cfg.ShopB} {
		if item.ID == localShop.ID || item.ID == extoreSellerShop(1) || item.ID == extoreSellerShop(2) {
			t.Fatal("fixture conflates Extore shop IDs with LMM shop namespaces")
		}
	}
	_, err = service.ExtoreConnectionCatalog(ctx, sellerToken, 1, rowA.ID)
	must(t, err)
	_, err = service.ExtoreConnectionCatalog(ctx, buyerToken, 2, rowB.ID)
	must(t, err)
	_, err = service.ExtoreConnectionCatalog(ctx, sellerToken, 2, rowB.ID)
	wantError(t, err, ErrForbidden)
	_, err = service.ExtoreConnectionCatalog(ctx, sellerToken, 1, rowB.ID)
	wantError(t, err, ErrNotFound)

	// A valid B grant deliberately sealed under A's connection AAD still must
	// not change the Extore shop bound to that persisted connection.
	mismatched := rowA
	bCredentials := cfg.ShopB.Credentials
	bCredentials.AccessExpires = time.Now().Unix() + int64(bCredentials.Expires)
	bSecret, err := json.Marshal(bCredentials)
	must(t, err)
	mismatched.Ciphertext, err = service.extore.seal(extoreConnectionAAD(rowA.ID), bSecret)
	must(t, err)
	must(t, service.repo.Within(ctx, extoreSellerShop(1), func(tx Tx) error {
		return tx.Put(extoreConnectionKind, rowA.ID, mismatched)
	}))
	_, err = service.ExtoreConnectionCatalog(ctx, sellerToken, 1, rowA.ID)
	wantError(t, err, ErrForbidden)
	must(t, service.repo.Within(ctx, extoreSellerShop(1), func(tx Tx) error {
		return tx.Put(extoreConnectionKind, rowA.ID, rowA)
	}))
	syncResult, err := service.ExtoreSyncConnection(ctx, sellerToken, 1, rowA.ID)
	must(t, err)
	items := syncResult.(map[string]any)["results"].([]ExtoreSyncResult)
	if len(items) != 1 || !items[0].Success || items[0].ProductID == "" {
		t.Fatal("actual plugin catalog did not synchronize its approved product")
	}
	_, err = service.ExtoreSyncProduct(ctx, sellerToken, 1, rowA.ID, cfg.ShopA.UnapprovedProductID)
	wantError(t, err, ErrForbidden)
	cancelled, stop := context.WithCancel(ctx)
	stop()
	_, err = reader.ReadCatalog(cancelled, cfg.Origin, cfg.ShopA.Credentials.Access)
	wantError(t, err, context.Canceled)
	must(t, reader.Close())
	_, err = client.ConnectionCatalog(ctx, cfg.Origin, cfg.ShopA.Credentials)
	wantError(t, err, extore.ErrUnavailable)
	if httpCatalogCalls.Load() != 0 {
		t.Fatal("explicit native catalog silently fell back to the HTTP client")
	}
	report := map[string]any{
		"status": "passed", "host_abi": 1, "interface": "extore.commerce",
		"real_extore_https_sqlite": true, "tls_verification": true,
		"native_thread_affinity": true, "native_repeated_open_call_release_close": true,
		"approved_products_per_shop": 1, "invalid_token_denied": true,
		"grant_validation_retained": true, "cross_user_connection_denied": true,
		"connection_upstream_shop_mismatch_denied": true,
		"extore_and_lmm_shop_ids_distinct":         true, "store_sync_persisted": true,
		"http_catalog_fallback_calls": httpCatalogCalls.Load(),
		"store_repository":            "transactional memory test repository",
		"core_authority":              "existing Store test double; independent CoreSession acceptance not run",
		"secret_values_logged":        false, "production_accounts_created": false,
	}
	encoded, err := json.MarshalIndent(report, "", "  ")
	must(t, err)
	must(t, os.WriteFile(cfg.ReportPath, append(encoded, '\n'), 0600))
}
