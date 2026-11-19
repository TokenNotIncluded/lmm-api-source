// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat_test

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"io"
	"mime"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store/rustauth"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/webcompat"
	_ "github.com/lib/pq"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// The database, repository, HTTP handlers and Rust authorization adapter are
// real. This development test supplies synthetic Core responses; the separate
// commerce_process acceptance owns real Core Session and OS-process evidence.
type merchantCoreFixture struct{ revoked atomic.Bool }

var merchantSellerToken = "lmms_" + strings.Repeat("s", 43)
var merchantBuyerToken = "lmms_" + strings.Repeat("b", 43)
var merchantAdminToken = "lmms_" + strings.Repeat("a", 43)
var merchantHostToken = "lmms_" + strings.Repeat("h", 43)

func (c *merchantCoreFixture) Authorize(_ context.Context, token string) (*pb.AuthorizeResponse, error) {
	id := map[string]int64{merchantSellerToken: 1, merchantBuyerToken: 2, merchantAdminToken: 4}[token]
	if id == 0 || id == 1 && c.revoked.Load() {
		return nil, status.Error(codes.Unauthenticated, "synthetic session unavailable")
	}
	level := int32(0)
	if id == 4 {
		level = 5
	}
	return &pb.AuthorizeResponse{
		UserId: id, PlatformLevel: level,
		CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION,
		Owner:          &pb.Account{Kind: pb.AccountKind_ACCOUNT_KIND_PERSONAL, Id: id},
	}, nil
}

func (*merchantCoreFixture) ListTeams(context.Context, string, int64) (*pb.ListTeamsResponse, error) {
	return nil, status.Error(codes.PermissionDenied, "personal accounts only")
}

type merchantHTTPResult struct {
	code int
	data map[string]any
	err  error
}

func merchantRequest(base, method, path, token string, body any) merchantHTTPResult {
	var raw []byte
	var err error
	if body != nil {
		raw, err = json.Marshal(body)
		if err != nil {
			return merchantHTTPResult{err: err}
		}
	}
	request, err := http.NewRequest(method, base+path, bytes.NewReader(raw))
	if err != nil {
		return merchantHTTPResult{err: err}
	}
	if token != "" {
		request.Header.Set("Authorization", "Bearer "+token)
	}
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	response, err := (&http.Client{Timeout: 10 * time.Second}).Do(request)
	if err != nil {
		return merchantHTTPResult{err: err}
	}
	defer response.Body.Close()
	raw, err = io.ReadAll(io.LimitReader(response.Body, 1<<20))
	if err != nil {
		return merchantHTTPResult{err: err}
	}
	var envelope struct {
		Data map[string]any `json:"data"`
	}
	mediaType, _, _ := mime.ParseMediaType(response.Header.Get("Content-Type"))
	if len(raw) > 0 && mediaType == "application/json" {
		if err := json.Unmarshal(raw, &envelope); err != nil {
			return merchantHTTPResult{err: err}
		}
	} else if response.StatusCode == http.StatusOK {
		return merchantHTTPResult{err: fmt.Errorf("successful response was not JSON")}
	}
	return merchantHTTPResult{code: response.StatusCode, data: envelope.Data}
}

func TestPostgresMerchantHomeHTTP(t *testing.T) {
	db := merchantDatabase(t)
	core := &merchantCoreFixture{}
	newServer := func(pool *sql.DB, enabled bool) *httptest.Server {
		t.Helper()
		auth, err := webcompat.NewAuthenticator(core, []byte(merchantHostToken))
		merchantMust(t, err)
		var service *store.Service
		if enabled {
			repo, err := store.NewPostgres(pool)
			merchantMust(t, err)
			authority, err := rustauth.New(core)
			merchantMust(t, err)
			service, err = store.New(repo, authority, nil)
			merchantMust(t, err)
		}
		mux := http.NewServeMux()
		webcompat.RegisterStore(mux, auth, service)
		server := httptest.NewServer(mux)
		t.Cleanup(server.Close)
		return server
	}
	server := newServer(db, true)
	request := func(method, path, token string, body any, want int) map[string]any {
		t.Helper()
		result := merchantRequest(server.URL, method, path, token, body)
		merchantMust(t, result.err)
		if result.code != want {
			t.Fatalf("%s %s: status=%d want=%d data=%v", method, path, result.code, want, result.data)
		}
		return result.data
	}
	homeInput := func(version int64, biography string) map[string]any {
		return map[string]any{
			"biography": biography, "announcement": "Synthetic merchant notice",
			"header_image": "https://example.invalid/header.png", "expected_version": version,
		}
	}
	rows := func(table string) int {
		t.Helper()
		var count int
		merchantMust(t, db.QueryRow("SELECT count(*) FROM lmm_store."+table).Scan(&count))
		return count
	}

	config := request("GET", "/api/store/config", "", nil, 200)
	if config["store_merchant_home_supported"] != true || config["store_access_supported"] != false {
		t.Fatalf("incorrect assembled capabilities: %v", config)
	}
	request("GET", "/api/store/my/home", "", nil, 401)
	request("GET", "/api/store/my/home", merchantHostToken, nil, 401)
	empty := request("GET", "/api/store/my/home", merchantSellerToken, nil, 200)
	if empty["version"] != float64(0) || empty["biography"] != "" || empty["seller"].(map[string]any)["id"] != float64(1) {
		t.Fatalf("incorrect new merchant profile: %v", empty)
	}
	request("GET", "/api/store/merchants/1", "", nil, 404)
	request("GET", "/api/store/merchants/not-a-number", "", nil, 422)
	if rows("shops") != 0 || rows("commands") != 0 {
		t.Fatal("reading an empty or unknown merchant created records")
	}

	saved := request("PUT", "/api/store/my/home", merchantSellerToken, homeInput(0, "商店介绍"), 200)
	if saved["version"] != float64(1) {
		t.Fatalf("first saved version: %v", saved)
	}
	public := request("GET", "/api/store/merchants/1", "", nil, 200)
	if public["biography"] != "商店介绍" || public["header_image"] != "https://example.invalid/header.png" {
		t.Fatalf("public profile lost persisted fields: %v", public)
	}
	seller := public["seller"].(map[string]any)
	if len(seller) != 3 || seller["username"] != "" || seller["display_name"] != "" || seller["id"] != float64(1) {
		t.Fatalf("public profile invented or exposed identity fields: %v", seller)
	}
	other := request("GET", "/api/store/my/home", merchantBuyerToken, nil, 200)
	if other["version"] != float64(0) || other["seller"].(map[string]any)["id"] != float64(2) {
		t.Fatalf("another user read the seller's private home: %v", other)
	}
	spoof := homeInput(1, "Wrong owner")
	spoof["seller_id"] = 1
	request("PUT", "/api/store/my/home", merchantBuyerToken, spoof, 422)
	request("PUT", "/api/store/my/home?seller_id=1", merchantBuyerToken, homeInput(1, "Wrong owner"), 422)
	request("PUT", "/api/store/merchants/1", merchantBuyerToken, homeInput(1, "Wrong owner"), 405)
	request("PUT", "/api/store/my/home", merchantSellerToken, homeInput(0, "Stale"), 409)

	for _, invalid := range []map[string]any{
		{"expected_version": 1, "header_image": "javascript:alert(1)"},
		{"expected_version": 1, "header_image": "https://user:secret@example.invalid/x"},
		{"expected_version": 1, "header_image": "data:image/svg+xml,%3Csvg/%3E"},
		{"expected_version": 1, "biography": strings.Repeat("界", 4097)},
		{"expected_version": 1, "announcement": "bad\x00notice"},
		{"biography": "missing version"},
	} {
		request("PUT", "/api/store/my/home", merchantSellerToken, invalid, 422)
	}

	// Independent SQL pools and HTTP handlers must serialize at the database,
	// not through an in-process service mutex.
	secondDB, err := sql.Open("postgres", os.Getenv("LMM_STORE_TEST_DSN"))
	merchantMust(t, err)
	t.Cleanup(func() { _ = secondDB.Close() })
	second := newServer(secondDB, true)
	results := make(chan merchantHTTPResult, 2)
	for i, target := range []string{server.URL, second.URL} {
		go func(target, text string) {
			results <- merchantRequest(target, "PUT", "/api/store/my/home", merchantSellerToken, homeInput(1, text))
		}(target, fmt.Sprintf("concurrent-%d", i))
	}
	wins, conflicts := 0, 0
	for range 2 {
		result := <-results
		merchantMust(t, result.err)
		switch result.code {
		case 200:
			wins++
			if result.data["version"] != float64(2) {
				t.Fatal(result.data)
			}
		case 409:
			conflicts++
		default:
			t.Fatalf("unexpected concurrent status: %d", result.code)
		}
	}
	if wins != 1 || conflicts != 1 {
		t.Fatalf("CAS winners=%d conflicts=%d", wins, conflicts)
	}
	shopID := store.ID("legacy-web-shop-v1", store.Account{Kind: "personal", ID: 1})
	var version int64
	var biography string
	merchantMust(t, db.QueryRow("SELECT (body->>'version')::bigint,body->>'biography' FROM lmm_store.commands WHERE shop_id=$1 AND id='@web:merchant-home'", shopID).Scan(&version, &biography))
	if version != 2 || !strings.HasPrefix(biography, "concurrent-") || rows("shops") != 1 || rows("commands") != 1 {
		t.Fatalf("unexpected stored profile state: version=%d biography=%q", version, biography)
	}

	server.Close()
	server = newServer(secondDB, true)
	reloaded := request("GET", "/api/store/merchants/1", "", nil, 200)
	if reloaded["version"] != float64(version) || reloaded["biography"] != biography {
		t.Fatalf("new service did not read the same durable profile: %v", reloaded)
	}
	request("PUT", "/api/store/my/home", merchantSellerToken, map[string]any{
		"biography": "", "announcement": "", "header_image": "", "expected_version": 2,
	}, 200)
	cleared := request("GET", "/api/store/merchants/1", "", nil, 200)
	if cleared["version"] != float64(3) || cleared["biography"] != "" || cleared["announcement"] != "" || cleared["header_image"] != "" {
		t.Fatal(cleared)
	}

	announcement := request("GET", "/api/store/announcement", "", nil, 200)
	if announcement["content"] != "" {
		t.Fatal(announcement)
	}
	request("PUT", "/api/store/announcement", merchantSellerToken, map[string]any{"content": "not admin"}, 403)
	request("PUT", "/api/store/announcement", merchantAdminToken, map[string]any{"content": "公告"}, 200)
	announcement = request("GET", "/api/store/announcement", "", nil, 200)
	if announcement["content"] != "公告" {
		t.Fatal(announcement)
	}
	var persistedAnnouncement string
	merchantMust(t, db.QueryRow("SELECT body->>'content' FROM lmm_store.commands WHERE shop_id='@web-legal' AND id='@web:store-announcement'").Scan(&persistedAnnouncement))
	if persistedAnnouncement != "公告" {
		t.Fatal("announcement was not stored in PostgreSQL")
	}
	request("PUT", "/api/store/announcement", merchantAdminToken, map[string]any{"content": ""}, 200)
	core.revoked.Store(true)
	request("PUT", "/api/store/my/home", merchantSellerToken, homeInput(3, "revoked"), 401)
	merchantMust(t, db.QueryRow("SELECT (body->>'version')::bigint FROM lmm_store.commands WHERE shop_id=$1 AND id='@web:merchant-home'", shopID).Scan(&version))
	if version != 3 || rows("orders") != 0 || rows("money_operations") != 0 {
		t.Fatal("refused profile write or announcement affected unrelated commerce state")
	}

	// Existing native catalogue shops are also genuine merchant homes before
	// their owners first save Web profile metadata.
	repo, err := store.NewPostgres(db)
	merchantMust(t, err)
	authority, err := rustauth.New(core)
	merchantMust(t, err)
	native, err := store.New(repo, authority, nil)
	merchantMust(t, err)
	_, err = native.Catalog(context.Background(), merchantBuyerToken, store.CatalogChange{
		Action: "create_shop", Owner: store.Account{Kind: "personal", ID: 2}, Title: "Native merchant", Key: "native-home",
	})
	merchantMust(t, err)
	beforeShops, beforeCommands := rows("shops"), rows("commands")
	nativeHome := request("GET", "/api/store/merchants/2", "", nil, 200)
	if nativeHome["version"] != float64(0) || nativeHome["biography"] != "" || nativeHome["seller"].(map[string]any)["id"] != float64(2) {
		t.Fatalf("native merchant missing its empty home: %v", nativeHome)
	}
	request("GET", "/api/store/merchants/999999", "", nil, 404)
	if rows("shops") != beforeShops || rows("commands") != beforeCommands {
		t.Fatal("public native or unknown merchant read created metadata")
	}

	disabled := newServer(db, false)
	for _, path := range []string{"/api/store/config", "/api/store/merchants/1", "/api/store/announcement"} {
		result := merchantRequest(disabled.URL, "GET", path, "", nil)
		merchantMust(t, result.err)
		if result.code != 503 {
			t.Fatalf("disabled store %s returned %d", path, result.code)
		}
	}
}

func merchantMust(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func merchantDatabase(t *testing.T) *sql.DB {
	t.Helper()
	dsn := os.Getenv("LMM_STORE_TEST_DSN")
	if dsn == "" {
		t.Skip("set LMM_STORE_TEST_DSN to an isolated empty *_test database")
	}
	db, err := sql.Open("postgres", dsn)
	merchantMust(t, err)
	t.Cleanup(func() { _ = db.Close() })
	db.SetMaxOpenConns(4)
	var name string
	merchantMust(t, db.QueryRow("SELECT current_database()").Scan(&name))
	if !strings.HasSuffix(name, "_test") {
		t.Fatal("refusing non-test database")
	}
	var exists bool
	merchantMust(t, db.QueryRow("SELECT EXISTS(SELECT FROM pg_namespace WHERE nspname='lmm_store')").Scan(&exists))
	if exists {
		t.Fatal("fresh standalone test database required")
	}
	_, err = db.Exec(store.Schema)
	merchantMust(t, err)
	t.Cleanup(func() { _, _ = db.Exec("DROP SCHEMA lmm_store CASCADE") })
	return db
}
