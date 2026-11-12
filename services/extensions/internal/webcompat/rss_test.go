// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"database/sql"
	"encoding/json"
	"net/http"
	"os"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
)

func TestRSSRequiresFreshSessionAndSettingsAuthority(t *testing.T) {
	actor := sessionSelf()
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return actor, nil }}
	auth := newAuth(t, core)
	mux := http.NewServeMux()
	RegisterOperations(mux, auth, nil)
	for _, path := range []string{"/api/rss", "/api/rss/settings"} {
		for _, token := range []string{"", hostToken} {
			if response := operationsRequest(mux, "GET", path, "", token); response.Code != 401 {
				t.Fatal("anonymous or host read accepted", path, response.Code)
			}
		}
	}
	for level := int32(0); level <= 6; level++ {
		actor.PlatformLevel = level
		for _, request := range [][3]string{{"GET", "/api/rss", "0"}, {"GET", "/api/rss/settings", "5"}, {"PUT", "/api/rss/settings", "6"}} {
			before := core.calls
			response := operationsRequest(mux, request[0], request[1], "", webToken)
			want := http.StatusServiceUnavailable
			if (request[2] == "5" && level < 5) || (request[2] == "6" && level < 6) {
				want = http.StatusForbidden
			}
			if response.Code != want || core.calls != before+1 {
				t.Fatal("fresh RSS authorization", request, level, response.Code, core.calls-before)
			}
		}
	}
	host, err := modules.NewHost(nil, []byte(hostToken))
	if err != nil {
		t.Fatal(err)
	}
	handler := Mount(host, auth, nil, nil)
	if response := operationsRequest(handler, "GET", "/api/rss", "", webToken); response.Code != 503 {
		t.Fatal("RSS bypassed host preparing admission", response.Code)
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	if response := operationsRequest(handler, "GET", "/api/rss", "", webToken); response.Code != 503 {
		t.Fatal("missing RSS returned empty success or host 404", response.Code)
	}
	manager := &pluginControllerFake{enabled: map[string]bool{"operations": false}}
	handler = Mount(host, auth, nil, nil, AdditionalServices{Plugins: manager})
	for _, enabled := range []bool{false, true} {
		manager.enabled["operations"] = enabled
		for _, path := range []string{"/api/rss", "/api/rss/settings"} {
			before := core.calls
			response := operationsRequest(handler, http.MethodGet, path, "", webToken)
			wantCalls := before
			if enabled {
				wantCalls++
			}
			if response.Code != http.StatusServiceUnavailable || core.calls != wantCalls {
				t.Fatalf("RSS plugin control enabled=%v path=%s status=%d auth calls=%d want=%d", enabled, path, response.Code, core.calls, wantCalls)
			}
		}
	}
}

func TestRSSPostgresHTTPSettingsAndRead(t *testing.T) {
	ownerDSN, runtimeDSN := os.Getenv("RSS_HTTP_TEST_OWNER_DSN"), os.Getenv("RSS_HTTP_TEST_RUNTIME_DSN")
	if ownerDSN == "" || runtimeDSN == "" {
		t.Skip("requires fresh isolated RSS HTTP PostgreSQL fixture")
	}
	ctx := context.Background()
	owner, err := sql.Open("postgres", ownerDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	runtime, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	if err := operations.Initialize(ctx, owner, "rss_http_runtime"); err != nil {
		t.Fatal(err)
	}
	service, err := operations.New(ctx, operations.Config{Releases: runtime})
	if err != nil {
		t.Fatal(err)
	}
	actor := sessionSelf()
	actor.PlatformLevel = 6
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return actor, nil }}
	auth := newAuth(t, core)
	host, err := modules.NewHost([]modules.Module{NewOperationsModule(auth, service)}, []byte(hostToken))
	if err != nil {
		t.Fatal(err)
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	handler := Mount(host, auth, nil, nil, AdditionalServices{Operations: service})
	const body = `{"revision":0,"feeds":[{"id":"news","name":"News","url":"https://feed.example/news","enabled":false}]}`
	for _, invalid := range []string{`{"feeds":[]}`, `{"revision":0,"feeds":null}`, body + `{}`, `{"revision":0,"feeds":[],"url":"https://other.example"}`} {
		if response := operationsRequest(handler, "PUT", "/api/rss/settings", invalid, webToken); response.Code != 422 {
			t.Fatal("invalid settings admitted", response.Code, response.Body.String())
		}
	}
	if response := operationsRequest(handler, "PUT", "/api/rss/settings", body, webToken); response.Code != 200 {
		t.Fatal("settings save", response.Code, response.Body.String())
	}
	if response := operationsRequest(handler, "PUT", "/api/rss/settings", body, webToken); response.Code != 409 {
		t.Fatal("stale revision overwritten", response.Code, response.Body.String())
	}
	response := operationsRequest(handler, "GET", "/api/rss/settings", "", webToken)
	var settings struct {
		Success bool                   `json:"success"`
		Data    operations.RSSSettings `json:"data"`
	}
	if json.Unmarshal(response.Body.Bytes(), &settings) != nil || !settings.Success || settings.Data.Revision != 1 || len(settings.Data.Feeds) != 1 || settings.Data.Feeds[0].Enabled {
		t.Fatal("persisted settings readback", response.Body.String())
	}
	actor.PlatformLevel = 0
	response = operationsRequest(handler, "GET", "/api/rss", "", webToken)
	var reader struct {
		Success bool `json:"success"`
		Data    struct {
			Feeds []operations.RSSFeed `json:"feeds"`
		} `json:"data"`
	}
	if json.Unmarshal(response.Body.Bytes(), &reader) != nil || !reader.Success || reader.Data.Feeds == nil || len(reader.Data.Feeds) != 0 {
		t.Fatal("disabled source fetched or invalid empty state", response.Body.String())
	}
	if response := operationsRequest(handler, "GET", "/api/rss/settings", "", webToken); response.Code != 403 {
		t.Fatal("ordinary user read private settings", response.Code)
	}
}
