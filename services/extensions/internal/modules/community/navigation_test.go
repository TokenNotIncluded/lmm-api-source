// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"database/sql"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

func TestNavigationValidation(t *testing.T) {
	for _, raw := range []string{`{}`, `{"home":false,"directory":1,"docs":" TRUE ","extension":{"custom":42}}`, `{"pricing":{"enabled":true,"requireAuth":"0"}}`, `{"rankings":{},"security":false}`} {
		if err := ValidateNavigation(raw); err != nil {
			t.Errorf("rejected %s: %v", raw, err)
		}
	}
	for _, raw := range []string{`[]`, `null`, `not-json`, `{} {}`, `{"home":null}`, `{"home":1.0}`, `{"directory":2}`, `{"pricing":{"requireAuth":"invalid"}}`, `{"security":[]}`, `{"extra":"` + strings.Repeat("x", navigationLimit) + `"}`} {
		if err := ValidateNavigation(raw); err == nil {
			t.Errorf("accepted %s", raw[:min(80, len(raw))])
		}
	}
}
func navigationHandlers(db *sql.DB) *handlers {
	return &handlers{s: &Service{db: db}, auth: fakeAuth{
		"user":  {UserId: 1, PlatformLevel: 1, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION},
		"admin": {UserId: 2, PlatformLevel: 5, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION},
		"root":  {UserId: 3, PlatformLevel: 6, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION},
		"key":   {UserId: 3, PlatformLevel: 6, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_API_KEY},
	}}
}
func navigationRequest(h *handlers, method, token, body string) *httptest.ResponseRecorder {
	r := httptest.NewRequest(method, "/api/site/navigation", strings.NewReader(body))
	if token != "" {
		r.Header.Set("Authorization", "Bearer "+token)
	}
	w := httptest.NewRecorder()
	if method == http.MethodGet {
		h.navigation(w, r)
	} else {
		h.updateNavigation(w, r)
	}
	return w
}
func TestNavigationWriteRequiresLiveRootSession(t *testing.T) {
	h := navigationHandlers(nil)
	for _, token := range []string{"", "user", "admin", "key"} {
		w := navigationRequest(h, http.MethodPut, token, `{"value":"{}"}`)
		if w.Code != http.StatusUnauthorized && w.Code != http.StatusForbidden {
			t.Fatalf("%s: %d %s", token, w.Code, w.Body.String())
		}
	}
	w := navigationRequest(h, http.MethodPost, "root", `{"value":"{\"home\":false}"}`)
	if w.Code != 200 {
		t.Fatalf("preview %d %s", w.Code, w.Body.String())
	}
	w = navigationRequest(h, http.MethodPut, "root", `{"value":"{\"home\":null}"}`)
	if w.Code != http.StatusBadRequest {
		t.Fatalf("invalid %d %s", w.Code, w.Body.String())
	}
}

func TestNavigationPostgresPersistenceAndCAS(t *testing.T) {
	dsn := os.Getenv("NAVIGATION_TEST_DSN")
	if dsn == "" {
		t.Skip("requires isolated synthetic navigation database")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var name, marker string
	if err = db.QueryRow(`SELECT current_database(),current_setting('lmm.synthetic',true)`).Scan(&name, &marker); err != nil || !strings.HasPrefix(name, "lmm_navigation_") || marker != "navigation-extension-test" {
		t.Fatalf("not an isolated navigation database: %v", err)
	}
	if _, err = db.Exec(`UPDATE lmm_community.navigation SET value='{}',revision=1,updated_by=NULL,updated_at=0 WHERE singleton`); err != nil {
		t.Fatal(err)
	}
	h := navigationHandlers(db)
	raw := `{ "home":false,"pricing":{"enabled":true,"requireAuth":true},"extension":{"custom":42} }`
	body, _ := json.Marshal(map[string]string{"value": raw, "expected_value": "{}"})
	w := navigationRequest(h, http.MethodPut, "root", string(body))
	if w.Code != 200 {
		t.Fatalf("save %d %s", w.Code, w.Body.String())
	}
	w = navigationRequest(h, http.MethodPut, "root", string(body))
	if w.Code != http.StatusConflict {
		t.Fatalf("stale %d %s", w.Code, w.Body.String())
	}
	h = navigationHandlers(db)
	w = navigationRequest(h, http.MethodGet, "", "")
	var response struct {
		Data navigationConfig `json:"data"`
	}
	if w.Code != 200 || json.Unmarshal(w.Body.Bytes(), &response) != nil || response.Data.Value != raw || response.Data.Revision != 2 {
		t.Fatalf("reload %d %s", w.Code, w.Body.String())
	}
	var actor int64
	if err = db.QueryRow(`SELECT updated_by FROM lmm_community.navigation WHERE singleton`).Scan(&actor); err != nil || actor != 3 {
		t.Fatalf("actor=%d err=%v", actor, err)
	}
	// The normal runtime role has no Core schema or account/ledger privileges.
	var core bool
	if err = db.QueryRowContext(context.Background(), `SELECT EXISTS(SELECT FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\')`).Scan(&core); err != nil || core {
		t.Fatalf("Core schema present: %v %v", core, err)
	}
}
