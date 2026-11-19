//go:build unix

package extore

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func TestPluginConfigRequiresCompleteTrustedBinding(t *testing.T) {
	good := PluginConfig{
		Origin: "https://extore.example", Socket: "/run/lmm/extore/catalog.sock",
		Host: "/usr/libexec/lmm-extore-host", Library: "/usr/lib/lmm/libextore_zplugin.so",
		SHA256: strings.Repeat("a", 64), DataDir: "/var/lib/extore",
	}
	if err := good.Validate(); err != nil {
		t.Fatal(err)
	}
	if err := (PluginConfig{}).Validate(); err != nil {
		t.Fatal(err)
	}
	for name, change := range map[string]func(*PluginConfig){
		"missing origin":      func(c *PluginConfig) { c.Origin = "" },
		"noncanonical origin": func(c *PluginConfig) { c.Origin += "/" },
		"other protocol":      func(c *PluginConfig) { c.Origin = "http://extore.example" },
		"relative socket":     func(c *PluginConfig) { c.Socket = "catalog.sock" },
		"missing host":        func(c *PluginConfig) { c.Host = "" },
		"missing library":     func(c *PluginConfig) { c.Library = "" },
		"missing data":        func(c *PluginConfig) { c.DataDir = "" },
		"missing digest":      func(c *PluginConfig) { c.SHA256 = "" },
	} {
		t.Run(name, func(t *testing.T) {
			cfg := good
			change(&cfg)
			if cfg.Validate() == nil {
				t.Fatal("accepted incomplete plugin binding")
			}
		})
	}
}

// This exercises the real Unix HTTP transport and existing catalog validator.
// Native plugin authorization is exercised separately with an actual .so.
func TestPluginCatalogTransportAndValidation(t *testing.T) {
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	socket := filepath.Join(dir, "catalog.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(socket, 0600); err != nil {
		t.Fatal(err)
	}
	var mu sync.Mutex
	catalog := fixture(t)
	requests := 0
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		requests++
		if r.Method != http.MethodGet || r.URL.Path != catalogPath ||
			r.Header.Get("X-Extore-Origin") != "https://extore.example" ||
			r.Header.Get("Authorization") != "Bearer private-plugin-test-token" {
			t.Error("catalog did not retain its exact binding")
			w.WriteHeader(400)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(catalog)
	})}
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(func() { _ = server.Close() })
	remoteCalls := 0
	base := &http.Client{Transport: roundTrip(func(*http.Request) (*http.Response, error) {
		remoteCalls++
		return reply(map[string]bool{"remote": true}), nil
	})}
	host := &PluginHost{origin: "https://extore.example", transport: pluginTransport(socket)}
	httpClient, err := host.WrapHTTP(base)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(host.transport.CloseIdleConnections)
	client := Client{HTTP: httpClient}
	credentials := Credentials{Access: "private-plugin-test-token", GrantID: "grant"}
	result, err := client.ConnectionCatalog(context.Background(), host.origin, credentials)
	if err != nil || result.Shop.ID != "shop" {
		t.Fatal("catalog read failed", err)
	}
	if remoteCalls != 0 {
		t.Fatal("catalog escaped to the remote transport")
	}

	mu.Lock()
	catalog.GrantID = "different-grant"
	mu.Unlock()
	if _, err = client.ConnectionCatalog(context.Background(), host.origin, credentials); !errors.Is(err, ErrProtocol) {
		t.Fatal("plugin output bypassed grant validation", err)
	}
	mu.Lock()
	catalog.GrantID, catalog.Shop.ID = "grant", "different-shop"
	mu.Unlock()
	if _, err = client.ConnectionCatalog(context.Background(), host.origin, credentials); !errors.Is(err, ErrProtocol) {
		t.Fatal("plugin output bypassed shop validation", err)
	}

	for _, request := range []struct{ method, path string }{
		{"GET", host.origin + "/.well-known/oauth-authorization-server"},
		{"POST", host.origin + tokenPath},
		{"POST", host.origin + revokePath},
		{"GET", host.origin + "/api/integrations/commerce/cards"},
		{"GET", "https://other.example" + catalogPath},
		{"GET", host.origin + catalogPath + "?shop=other"},
		{"POST", host.origin + catalogPath},
	} {
		req, err := http.NewRequest(request.method, request.path, nil)
		if err != nil {
			t.Fatal(err)
		}
		response, err := httpClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		_ = response.Body.Close()
	}
	if remoteCalls != 7 {
		t.Fatalf("remote route count = %d", remoteCalls)
	}
	mu.Lock()
	count := requests
	mu.Unlock()
	if count != 3 {
		t.Fatalf("unexpected local routes: %d", count)
	}

	_ = server.Close()
	if _, err = client.ConnectionCatalog(context.Background(), host.origin, credentials); !errors.Is(err, ErrUnavailable) {
		t.Fatal("unavailable plugin did not fail closed", err)
	}
	if remoteCalls != 7 {
		t.Fatal("unavailable plugin fell back to HTTP")
	}
	if strings.Contains(err.Error(), credentials.Access) {
		t.Fatal("credential escaped through error")
	}
}

func TestPluginSocketRequiresPrivateOwnerDirectory(t *testing.T) {
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	socket := filepath.Join(dir, "catalog.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	if err := os.Chmod(socket, 0600); err != nil {
		t.Fatal(err)
	}
	if err := checkPluginSocket(socket); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(dir, 0755); err != nil {
		t.Fatal(err)
	}
	if err := checkPluginSocket(socket); err == nil {
		t.Fatal("public socket directory accepted")
	}
}
