// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat_test

import (
	"context"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store/rustauth"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/webcompat"
)

type presetCoreFixture struct{ merchantCoreFixture }

func (c *presetCoreFixture) Authorize(ctx context.Context, token string) (*pb.AuthorizeResponse, error) {
	if token == presetRootToken {
		result, err := c.merchantCoreFixture.Authorize(ctx, merchantAdminToken)
		if err != nil {
			return nil, err
		}
		result.PlatformLevel = 6
		return result, nil
	}
	return c.merchantCoreFixture.Authorize(ctx, token)
}

var presetRootToken = "lmms_" + strings.Repeat("r", 43)

func TestPostgresProductLinkPresetHTTP(t *testing.T) {
	db := merchantDatabase(t)
	core := &presetCoreFixture{}
	auth, err := webcompat.NewAuthenticator(core, []byte(merchantHostToken))
	merchantMust(t, err)
	authority, err := rustauth.New(core)
	merchantMust(t, err)
	server := func() *httptest.Server {
		repo, err := store.NewPostgres(db)
		merchantMust(t, err)
		service, err := store.New(repo, authority, nil)
		merchantMust(t, err)
		mux := http.NewServeMux()
		webcompat.RegisterStore(mux, auth, service)
		host := httptest.NewServer(mux)
		t.Cleanup(host.Close)
		return host
	}
	host := server()
	request := func(method, path, token string, value any, status int) map[string]any {
		t.Helper()
		response := merchantRequest(host.URL, method, path, token, value)
		merchantMust(t, response.err)
		if response.code != status {
			t.Fatalf("expected HTTP %d, got %d", status, response.code)
		}
		return response.data
	}
	const path = "/api/store/product-link-presets"
	const configPath = "/api/store/config"
	input := []any{
		map[string]any{"id": "guide", "title": "Guide", "url": "https://docs.example/", "description": "Buyer guide"},
		map[string]any{"id": "return", "title": "Returns", "url": "https://docs.example/returns", "description": ""},
	}
	write := func(token string, entries any, code int) {
		request(http.MethodPut, path, token, map[string]any{"presets": entries}, code)
	}
	check := func(want []any) {
		t.Helper()
		got := request(http.MethodGet, configPath, "", nil, http.StatusOK)["product_link_presets"]
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("public presets = %#v, want %#v", got, want)
		}
	}
	check([]any{})
	write(merchantSellerToken, input, http.StatusForbidden)
	write(merchantAdminToken, input, http.StatusForbidden)
	write(presetRootToken, input, http.StatusOK)
	host.Close()
	host = server()
	check(input)
	write(presetRootToken, []any{input[0], input[0]}, http.StatusUnprocessableEntity)
	write(presetRootToken, []any{map[string]any{"id": "unsafe", "title": "Bad", "url": "javascript:void(0)"}}, http.StatusUnprocessableEntity)
	write(presetRootToken, nil, http.StatusUnprocessableEntity)
	check(input)
	write(presetRootToken, []any{}, http.StatusOK)
	check([]any{})
}
