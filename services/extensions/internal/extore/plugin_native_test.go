//go:build unix

package extore

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type nativeAuthorization struct {
	Credentials
	ClientID string `json:"client_id"`
	ShopID   string `json:"shop_id"`
}

func TestExtoreNativePlugin(t *testing.T) {
	path := os.Getenv("LMM_EXTORE_PLUGIN_FIXTURE")
	if path == "" {
		t.Skip("requires the isolated real Extore native fixture")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal("native fixture could not be read")
	}
	var fixture struct {
		Config     PluginConfig        `json:"config"`
		HTTPOrigin string              `json:"http_origin"`
		A          nativeAuthorization `json:"a"`
		B          nativeAuthorization `json:"b"`
	}
	if json.Unmarshal(raw, &fixture) != nil {
		t.Fatal("invalid native fixture")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	cfg := fixture.Config
	cfg.Socket = filepath.Join(dir, "catalog.sock")

	// A rejected real native startup must not prevent the following valid start.
	rejected := cfg
	rejected.SHA256 = strings.Repeat("0", 64)
	if host, err := StartPlugin(ctx, rejected); err == nil {
		_ = host.Close()
		t.Fatal("unreviewed native artifact loaded")
	}
	if _, err := os.Lstat(cfg.Socket); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("rejected startup left a socket")
	}
	badState := cfg
	badState.DataDir = filepath.Join(dir, "uninitialized")
	if err := os.Mkdir(badState.DataDir, 0700); err != nil {
		t.Fatal(err)
	}
	if host, err := StartPlugin(ctx, badState); err == nil {
		_ = host.Close()
		t.Fatal("native plugin initialized an unprovisioned store")
	}
	for _, path := range []string{cfg.Socket, filepath.Join(badState.DataDir, "extore.sqlite3")} {
		if _, err := os.Lstat(path); !errors.Is(err, os.ErrNotExist) {
			t.Fatal("failed configuration left a socket or created state")
		}
	}
	host, err := StartPlugin(ctx, cfg)
	if err != nil {
		t.Fatal("native host could not become ready", err)
	}
	defer host.Close()

	local, err := url.Parse(fixture.HTTPOrigin)
	if err != nil || local.Scheme != "http" || local.Hostname() != "127.0.0.1" {
		t.Fatal("fixture must use its isolated loopback HTTP server")
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.Proxy = nil
	defer transport.CloseIdleConnections()
	base := &http.Client{Timeout: 20 * time.Second, Transport: roundTrip(func(request *http.Request) (*http.Response, error) {
		if request.URL.Scheme+"://"+request.URL.Host != cfg.Origin {
			return nil, ErrInput
		}
		copy := request.Clone(request.Context())
		endpoint := *request.URL
		endpoint.Scheme, endpoint.Host = local.Scheme, local.Host
		copy.URL, copy.Host = &endpoint, local.Host
		return transport.RoundTrip(copy)
	})}
	httpClient, err := host.WrapHTTP(base)
	if err != nil {
		t.Fatal(err)
	}
	client := Client{HTTP: httpClient}

	checkCatalog := func(auth nativeAuthorization) *Catalog {
		t.Helper()
		catalog, err := client.ConnectionCatalog(ctx, cfg.Origin, auth.Credentials)
		if err != nil {
			t.Fatal("authorized real plugin catalog failed", err)
		}
		if catalog.Shop.ID != auth.ShopID || len(catalog.Products) != 1 {
			t.Fatal("real plugin returned a different shop's catalog")
		}
		data, _ := json.Marshal(catalog)
		if strings.Contains(string(data), "PRIVATE_") || !strings.Contains(string(data), "12.3400") {
			t.Fatal("real catalog lost price text or leaked a private fixture field")
		}
		return catalog
	}
	first := checkCatalog(fixture.A)
	second := checkCatalog(fixture.A)
	if string(first.Products[0]) != string(second.Products[0]) {
		t.Fatal("repeated native reads changed the same catalog revision")
	}
	checkCatalog(fixture.B)

	unauthorized := fixture.A.Credentials
	unauthorized.Access = ""
	if _, err := client.ConnectionCatalog(ctx, cfg.Origin, unauthorized); !errors.Is(err, ErrUnavailable) {
		t.Fatal("native catalog accepted an unauthenticated request")
	}
	crossShop := fixture.A.Credentials
	crossShop.Access = fixture.B.Access
	if _, err := client.ConnectionCatalog(ctx, cfg.Origin, crossShop); !errors.Is(err, ErrProtocol) {
		t.Fatal("native transport bypassed the stored grant binding")
	}
	if err := client.RevokeConnection(ctx, cfg.Origin, fixture.A.ClientID, fixture.A.Credentials); err != nil {
		t.Fatal("real HTTP revocation failed", err)
	}
	if _, err := client.ConnectionCatalog(ctx, cfg.Origin, fixture.A.Credentials); !errors.Is(err, ErrUnavailable) {
		t.Fatal("configured native instance retained a revoked grant")
	}
	checkCatalog(fixture.B)
	if err := host.Close(); err != nil {
		t.Fatal("native host did not close")
	}
	if _, err := os.Lstat(cfg.Socket); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("native host left its socket after termination")
	}
	t.Log("real native catalog: repeated read, two shops, unauthenticated and cross-grant refusal, live HTTP revoke, owner cleanup")
}
