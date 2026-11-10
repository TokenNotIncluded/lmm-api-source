// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"database/sql"
	"encoding/json"
	"net/http"
	"os"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
)

func TestScriptRepositoryCredentialHTTPPostgresL6RedactionAndRecovery(t *testing.T) {
	ownerDSN, runtimeDSN := os.Getenv("SCRIPT_CREDENTIAL_HTTP_TEST_OWNER_DSN"), os.Getenv("SCRIPT_CREDENTIAL_HTTP_TEST_RUNTIME_DSN")
	if ownerDSN == "" || runtimeDSN == "" {
		t.Skip("requires fresh isolated credential HTTP PostgreSQL fixture")
	}
	ctx := context.Background()
	owner, err := sql.Open("postgres", ownerDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	owner.SetMaxOpenConns(1)
	if err := operations.Initialize(ctx, owner, "script_credential_http_runtime"); err != nil {
		t.Fatal(err)
	}
	if _, err := owner.Exec(`ALTER ROLE script_credential_http_runtime NOLOGIN`); err != nil {
		t.Fatal(err)
	}
	tx, err := owner.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer tx.Rollback()
	if err := operations.UpgradeScriptRepositoryCredentials(ctx, tx, "script_credential_http_runtime", "script_credential_http_original", false); err != nil {
		t.Fatal(err)
	}
	if err := tx.Commit(); err != nil {
		t.Fatal(err)
	}
	if _, err := owner.Exec(`ALTER ROLE script_credential_http_runtime LOGIN`); err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	s, err := operations.New(ctx, operations.Config{Releases: db})
	if err != nil {
		t.Fatal(err)
	}
	key := []byte(strings.Repeat("k", 32))
	if err := s.Repository.WithCredentialKey(ctx, key); err != nil {
		t.Fatal(err)
	}
	actor := sessionSelf()
	actor.PlatformLevel = 6
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return actor, nil }}
	mux := http.NewServeMux()
	RegisterOperations(mux, newAuth(t, core), s)
	const path = "/api/scripts/repository"
	const secret = "github_pat_http_fixture_secret"
	body := func(revision int64, token string, clear bool) string {
		b, e := json.Marshal(map[string]any{"revision": revision, "repository_url": "https://github.com/owner/private", "branch": "main", "github_key": token, "clear_github_key": clear})
		if e != nil {
			t.Fatal(e)
		}
		return string(b)
	}
	check := func(method, path, body string, status int) {
		t.Helper()
		before := core.calls
		w := operationsRequest(mux, method, path, body, webToken)
		if w.Code != status || core.calls != before+1 || strings.Contains(w.Body.String(), secret) || strings.Contains(w.Body.String(), "v1:") {
			t.Fatal("status/fresh authority/redaction", w.Code, status)
		}
	}
	for _, level := range []int32{1, 5} {
		actor.PlatformLevel = level
		check("PUT", path, body(0, secret, false), 403)
	}
	actor.PlatformLevel = 6
	check("PUT", path, body(0, secret, false), 200)
	check("PUT", path, body(0, "github_pat_stale", false), 409)
	// Unknown, duplicate and conflicting credential fields must not mutate state.
	check("PUT", path, strings.Replace(body(1, "", false), `"branch":"main"`, `"branch":"main","branch":"other"`, 1), 422)
	check("PUT", path, body(1, secret, true), 422)
	settings, err := s.Repository.Settings(ctx)
	if err != nil || !settings.GitHubKeySet || settings.Revision != 1 {
		t.Fatal("write persisted", err)
	}
	actor.PlatformLevel = 5
	check("GET", path, "", 200)
	check("PUT", path, body(1, "", false), 403)
	check("PUT", path, body(1, "", true), 403)
	check("POST", path+"/pull", "", 403)
	actor.PlatformLevel = 6
	check("PUT", path, body(1, "", false), 200)
	// Reconstruct the service with no key: public projection still works, and a
	// private pull fails closed before any outbound HTTP instead of becoming public.
	noKey, err := operations.New(ctx, operations.Config{Releases: db})
	if err != nil {
		t.Fatal(err)
	}
	mux = http.NewServeMux()
	RegisterOperations(mux, newAuth(t, core), noKey)
	check("GET", path, "", 200)
	check("POST", path+"/pull", "", 503)
	if err := noKey.Repository.WithCredentialKey(ctx, key); err != nil {
		t.Fatal(err)
	}
	check("PUT", path, body(2, "github_pat_replacement", false), 200)
	// Clear is allowed for L6 even if the encryption key is unavailable.
	clearing, err := operations.New(ctx, operations.Config{Releases: db})
	if err != nil {
		t.Fatal(err)
	}
	mux = http.NewServeMux()
	RegisterOperations(mux, newAuth(t, core), clearing)
	check("PUT", path, body(3, "", true), 200)
	settings, err = clearing.Repository.Settings(ctx)
	if err != nil || settings.GitHubKeySet || settings.Revision != 4 {
		t.Fatal("clear persisted", err)
	}
	var ciphertext string
	if db.QueryRow(`SELECT github_key_ciphertext FROM lmm_operations.script_repository`).Scan(&ciphertext) != nil || ciphertext != "" {
		t.Fatal("clear ciphertext")
	}
	actor.PlatformLevel = 5
	check("PUT", path, body(4, "", false), 200)
}
