// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"database/sql"
	"net/http"
	"os"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
)

func TestIoNetHTTPFreshAuthorityBeforeStorageAndProvider(t *testing.T) {
	actor := sessionSelf()
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return actor, nil }}
	mux := http.NewServeMux()
	RegisterOperations(mux, newAuth(t, core), nil)
	for level := int32(0); level <= 6; level++ {
		actor.PlatformLevel = level
		for _, tc := range []struct {
			method, path, body string
			write              bool
		}{
			{"GET", "/api/deployments/settings", "", false},
			{"GET", "/api/deployments/operations", "", false},
			{"PUT", "/api/deployments/settings", `{}`, true},
			{"POST", "/api/deployments/", `{}`, true},
			{"POST", "/api/deployments/deployment-1/extend", `{}`, true},
			{"POST", "/api/deployments/operations/receipts/original_operation_0001/recover", `{}`, true},
			{"POST", "/api/deployments/operations/receipts/original_operation_0001/cancel", `{}`, true},
		} {
			before := core.calls
			want := 403
			if level == 6 || level == 5 && !tc.write {
				want = 503
			}
			w := operationsRequest(mux, tc.method, tc.path, tc.body, webToken)
			if w.Code != want || core.calls != before+1 {
				t.Fatalf("level %d %s: status %d want %d fresh calls %d", level, tc.path, w.Code, want, core.calls-before)
			}
		}
	}
	if w := operationsRequest(mux, "GET", "/api/deployments/settings", "", ""); w.Code != 401 {
		t.Fatal("anonymous", w.Code)
	}
}

func TestIoNetHTTPPostgresSettingsAndMissingReceiptFence(t *testing.T) {
	ownerDSN, runtimeDSN := os.Getenv("IONET_HTTP_TEST_OWNER_DSN"), os.Getenv("IONET_HTTP_TEST_RUNTIME_DSN")
	if ownerDSN == "" || runtimeDSN == "" {
		t.Skip("requires fresh isolated HTTP PostgreSQL fixture")
	}
	ctx := context.Background()
	owner, err := sql.Open("postgres", ownerDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	if err := operations.Initialize(ctx, owner, "ionet_http_runtime"); err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	service, err := operations.New(ctx, operations.Config{Releases: db})
	if err != nil {
		t.Fatal(err)
	}
	if err := service.IoNet.WithCredentialKey(ctx, []byte(strings.Repeat("j", 32))); err != nil {
		t.Fatal(err)
	}
	actor := sessionSelf()
	actor.PlatformLevel = 6
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return actor, nil }}
	mux := http.NewServeMux()
	RegisterOperations(mux, newAuth(t, core), service)
	check := func(method, path, body string, want int) {
		t.Helper()
		before := core.calls
		w := operationsRequest(mux, method, path, body, webToken)
		if w.Code != want || core.calls != before+1 || strings.Contains(w.Body.String(), "offline-http-key") || strings.Contains(w.Body.String(), "v1:") {
			t.Fatal("fresh authority/status/redaction", method, path, w.Code, want)
		}
	}
	check("PUT", "/api/deployments/settings", `{"revision":0,"enabled":true,"api_key":"offline-http-key","clear_api_key":false}`, 200)
	check("PUT", "/api/deployments/settings", `{"revision":0,"enabled":false}`, 409)
	check("PUT", "/api/deployments/settings", `{"revision":1,"revision":2,"enabled":false}`, 422)
	check("PUT", "/api/deployments/settings", `{"revision":1,"enabled":true,"api_key":"offline-http-key","clear_api_key":true}`, 422)
	actor.PlatformLevel = 5
	check("GET", "/api/deployments/settings", "", 200)
	check("PUT", "/api/deployments/settings", `{"revision":1,"enabled":false}`, 403)
	check("POST", "/api/deployments/settings/test-connection", `{"api_key":"offline-http-key"}`, 403)
	actor.PlatformLevel = 6
	const operation = "/api/deployments/operations/receipts/missing_original_0001"
	check("POST", operation+"/recover", `{}`, 404)
	check("POST", operation+"/cancel", `{}`, 200)
	check("POST", operation+"/recover", `{}`, 200)
	originalID := actor.UserId
	actor.UserId++
	actor.Owner.Id++
	check("GET", operation, "", 404)
	actor.UserId = originalID
	actor.Owner.Id = originalID
	check("PUT", "/api/deployments/settings", `{"revision":1,"enabled":false,"clear_api_key":true}`, 200)
	settings, err := service.IoNet.Settings(ctx)
	if err != nil || settings.Configured || settings.Revision != 2 {
		t.Fatal("clear persisted", err)
	}
}
