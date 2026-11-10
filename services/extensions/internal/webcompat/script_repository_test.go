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

func TestScriptRepositoryHTTPPostgresSettingsAndFreshAuthority(t *testing.T) {
	ownerDSN, runtimeDSN := os.Getenv("SCRIPT_REPOSITORY_HTTP_TEST_OWNER_DSN"), os.Getenv("SCRIPT_REPOSITORY_HTTP_TEST_RUNTIME_DSN")
	if ownerDSN == "" || runtimeDSN == "" {
		t.Skip("requires fresh private script repository HTTP PostgreSQL fixture")
	}
	ctx := context.Background()
	owner, err := sql.Open("postgres", ownerDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	db, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if err := operations.Initialize(ctx, owner, "script_http_runtime"); err != nil {
		t.Fatal(err)
	}
	s, err := operations.New(ctx, operations.Config{Releases: db})
	if err != nil {
		t.Fatal(err)
	}
	actor := sessionSelf()
	actor.PlatformLevel = 5
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return actor, nil }}
	mux := http.NewServeMux()
	RegisterOperations(mux, newAuth(t, core), s)
	const path = "/api/scripts/repository"
	const body = `{"revision":0,"repository_url":"https://github.com/owner/repo","branch":"main","github_key":"","clear_github_key":false}`
	for _, token := range []string{"", hostToken} {
		if w := operationsRequest(mux, "GET", path, "", token); w.Code != 401 {
			t.Fatal(w.Code, w.Body.String())
		}
	}
	for level := int32(0); level <= 6; level++ {
		actor.PlatformLevel = level
		before := core.calls
		w := operationsRequest(mux, "GET", path, "", webToken)
		want := 403
		if level == 5 || level == 6 {
			want = 200
		}
		if w.Code != want || core.calls != before+1 {
			t.Fatal("fresh authority", level, w.Code, core.calls-before)
		}
	}
	actor.PlatformLevel = 5
	w := operationsRequest(mux, "GET", path, "", webToken)
	var response struct {
		Success bool                                `json:"success"`
		Data    operations.ScriptRepositorySettings `json:"data"`
	}
	if json.Unmarshal(w.Body.Bytes(), &response) != nil || !response.Success || response.Data.URL != "" || response.Data.Revision != 0 || response.Data.Branch != "main" {
		t.Fatal(w.Body.String())
	}
	for _, bad := range []string{
		`{"repository_url":"https://github.com/owner/repo","branch":"main"}`,
		strings.Replace(body, `"revision":0`, `"revision":null`, 1), body + `{}`,
		strings.Replace(body, `"branch":"main"`, `"branch":"../main"`, 1),
	} {
		if w := operationsRequest(mux, "PUT", path, bad, webToken); w.Code != 422 {
			t.Fatal(w.Code, w.Body.String())
		}
	}
	for _, bad := range []string{strings.Replace(body, `"github_key":""`, `"github_key":"never-echo-secret"`, 1), strings.Replace(body, `"clear_github_key":false`, `"clear_github_key":true`, 1)} {
		w := operationsRequest(mux, "PUT", path, bad, webToken)
		if w.Code != 501 || !strings.Contains(w.Body.String(), "SCRIPT_REPOSITORY_CREDENTIALS_UNSUPPORTED") || strings.Contains(w.Body.String(), "never-echo-secret") {
			t.Fatal(w.Code, w.Body.String())
		}
	}
	actor.PlatformLevel = 1
	for _, tc := range [][3]string{{"PUT", path, body}, {"POST", path + "/pull", ""}} {
		if w := operationsRequest(mux, tc[0], tc[1], tc[2], webToken); w.Code != 403 {
			t.Fatal(w.Code, w.Body.String())
		}
	}
	actor.PlatformLevel = 5
	if w := operationsRequest(mux, "PUT", path, body, webToken); w.Code != 200 {
		t.Fatal(w.Code, w.Body.String())
	}
	if w := operationsRequest(mux, "PUT", path, body, webToken); w.Code != 409 {
		t.Fatal("stale save", w.Code, w.Body.String())
	}
	if w := operationsRequest(mux, "POST", path+"/pull", `{}`, webToken); w.Code != 422 {
		t.Fatal(w.Code, w.Body.String())
	}
	w = operationsRequest(mux, "GET", path, "", webToken)
	if json.Unmarshal(w.Body.Bytes(), &response) != nil || response.Data.Revision != 1 || response.Data.URL != "https://github.com/owner/repo" {
		t.Fatal(w.Body.String())
	}
}
