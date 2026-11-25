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
	return &handlers{s: &Service{db: db, sidebarNavigation: true}, auth: fakeAuth{
		"user":  {UserId: 1, PlatformLevel: 1, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION},
		"admin": {UserId: 2, PlatformLevel: 5, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION},
		"root":  {UserId: 3, PlatformLevel: 6, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION},
		"key":   {UserId: 3, PlatformLevel: 6, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_API_KEY},
	}}
}
func navigationRequest(h *handlers, method, token, body string) *httptest.ResponseRecorder {
	path := "/api/site/navigation"
	if method == http.MethodPost {
		path += "/validate"
	}
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	if token != "" {
		r.Header.Set("Authorization", "Bearer "+token)
	}
	w := httptest.NewRecorder()
	mux := http.NewServeMux()
	h.s.Register(mux, h.auth)
	mux.ServeHTTP(w, r)
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
	if _, err = db.Exec(`UPDATE lmm_community.navigation SET value='{}',revision=1,updated_by=NULL,updated_at=0,sidebar_value='{}',sidebar_revision=1,sidebar_updated_by=NULL,sidebar_updated_at=0 WHERE singleton`); err != nil {
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
	// Saving sidebar display preferences must not alter the independent header
	// document, revision, or original actor.
	sidebar := `{"console":{"log":false},"personal":{"company":true}}`
	sidebarBody, _ := json.Marshal(map[string]string{"key": "SidebarModulesAdmin", "value": sidebar, "expected_value": "{}"})
	w = navigationRequest(h, http.MethodPut, "root", string(sidebarBody))
	if w.Code != http.StatusOK {
		t.Fatalf("sidebar save: %d %s", w.Code, w.Body.String())
	}
	w = navigationRequest(h, http.MethodPut, "root", string(sidebarBody))
	if w.Code != http.StatusConflict {
		t.Fatalf("stale sidebar write: %d", w.Code)
	}
	var updated struct {
		Data struct {
			navigationConfig
			Sidebar navigationConfig `json:"sidebar"`
		} `json:"data"`
	}
	w = navigationRequest(navigationHandlers(db), http.MethodGet, "", "")
	if w.Code != http.StatusOK || json.Unmarshal(w.Body.Bytes(), &updated) != nil ||
		updated.Data.Value != raw || updated.Data.Revision != 2 ||
		updated.Data.Sidebar.Value != sidebar || updated.Data.Sidebar.Revision != 2 {
		t.Fatalf("independent sidebar readback: %d %s", w.Code, w.Body.String())
	}
	if err = db.QueryRow(`SELECT sidebar_updated_by FROM lmm_community.navigation WHERE singleton`).Scan(&actor); err != nil || actor != 3 {
		t.Fatalf("sidebar actor=%d err=%v", actor, err)
	}
	// The normal runtime role has no Core schema or account/ledger privileges.
	var core bool
	if err = db.QueryRowContext(context.Background(), `SELECT EXISTS(SELECT FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\')`).Scan(&core); err != nil || core {
		t.Fatalf("Core schema present: %v %v", core, err)
	}
}

func TestLegacySidebarNavigation(t *testing.T) {
	raw := `{"console":{"log":" FALSE ","requests":1e0},"custom":{"show":0.0}}`
	if !validSidebarNavigation(raw, true) || validSidebarNavigation(raw, false) {
		t.Fatal("legacy boolean and numeric spellings are accepted only on imported reads")
	}
	for _, invalid := range []string{`{"custom":{"show":null}}`, `{"custom":{"show":2}}`, `{"custom":true}`} {
		if validSidebarNavigation(invalid, true) {
			t.Fatalf("invalid imported flags accepted: %s", invalid)
		}
	}
}

func TestSidebarNavigationValidationAndRootSession(t *testing.T) {
	h := navigationHandlers(nil)
	raw := func(value string) string {
		body, err := json.Marshal(map[string]string{"key": "SidebarModulesAdmin", "value": value})
		if err != nil {
			t.Fatal(err)
		}
		return string(body)
	}
	for _, token := range []string{"", "user", "admin", "key"} {
		w := navigationRequest(h, http.MethodPost, token, raw(`{"admin":{"setting":false}}`))
		if w.Code != http.StatusUnauthorized && w.Code != http.StatusForbidden {
			t.Fatalf("sidebar validation allowed %q: %d", token, w.Code)
		}
	}
	for _, value := range []string{`{}`, `{"personal":{"company":true},"custom":{"enabled":false}}`} {
		if w := navigationRequest(h, http.MethodPost, "root", raw(value)); w.Code != http.StatusOK {
			t.Fatalf("valid sidebar input rejected: %d %s", w.Code, w.Body.String())
		}
	}
	for _, value := range []string{`[]`, `null`, `{"chat":true}`, `{"console":{"log":"false"}}`, `{"console":{"log":null}}`, `{"admin":{"setting":{"role":6}}}`, `{} {}`} {
		if w := navigationRequest(h, http.MethodPost, "root", raw(value)); w.Code != http.StatusBadRequest {
			t.Fatalf("invalid sidebar input accepted: %s: %d", value, w.Code)
		}
	}
	if w := navigationRequest(h, http.MethodPost, "root", `{"key":"OtherOption","value":"{}"}`); w.Code != http.StatusBadRequest {
		t.Fatalf("unexpected navigation option accepted: %d", w.Code)
	}
	if w := navigationRequest(h, http.MethodPut, "root", raw(`{}`)); w.Code != http.StatusBadRequest {
		t.Fatalf("unconditional sidebar write accepted: %d", w.Code)
	}
	h.s.sidebarNavigation = false
	if w := navigationRequest(h, http.MethodPost, "root", raw(`{}`)); w.Code != http.StatusServiceUnavailable {
		t.Fatalf("sidebar advertised on version 3: %d", w.Code)
	}
	if w := navigationRequest(h, http.MethodPost, "root", `{"value":"{}"}`); w.Code != http.StatusOK {
		t.Fatalf("version 3 header preview failed: %d", w.Code)
	}
}
