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
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	_ "github.com/lib/pq"
)

func solve(seed uint32, size int) []int {
	c := signalGenerate(seed, size)
	tiles := append([]int(nil), c.Tiles...)
	var actions []int
	for _, i := range c.Route {
		for n := 0; n < 4 && tiles[i] != c.Solution[i]; n++ {
			tiles[i] = signalRotate(tiles[i])
			actions = append(actions, i)
			if signalWon(tiles, size) {
				return actions
			}
		}
	}
	return actions
}

func TestSignalReplay(t *testing.T) {
	seed, err := signalDailySeed("2026-11-03", 5)
	if err != nil {
		t.Fatal(err)
	}
	actions := solve(seed, 5)
	if hints, err := signalReplay(seed, 5, actions, false); err != nil || hints != 0 {
		t.Fatalf("solution rejected: %v", err)
	}
	if _, err := signalReplay(seed, 5, actions[:len(actions)-1], false); err == nil {
		t.Fatal("unfinished board accepted")
	}
	if _, err := signalReplay(seed, 5, append(actions, 0), false); err == nil {
		t.Fatal("moves after completion accepted")
	}
	if _, err := signalReplay(seed, 5, []int{-1}, false); err == nil {
		t.Fatal("challenge hint accepted")
	}
	if _, err := signalDailySeed("2026-13-01", 5); err == nil {
		t.Fatal("invalid day accepted")
	}
}

func TestValidation(t *testing.T) {
	if _, err := ValidateDirectoryLinks([]byte(`[{"id":"a","name":"A","url":"https://a.example.com","category":"chat","summary":"","description":"","enabled":true}]`)); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{`{}`, `[{"id":"a"}]`, `[{"id":"a","name":"A","url":"javascript:alert(1)","category":"chat","summary":"","description":"","enabled":true}]`,
		`[{"id":"a","name":"A","url":"https://a.com","category":"chat","summary":"","description":"","enabled":true,"x":1}]`} {
		if _, err := ValidateDirectoryLinks([]byte(bad)); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
	now := time.Date(2026, 11, 3, 0, 0, 0, 0, time.UTC)
	tokens := int64(5)
	ok := []LinkedProfile{{Provider: "chatgpt", URL: "https://chatgpt.com/u/someone", Snapshot: &ProfileUsageSnapshot{Tokens: &tokens, Period: "all", ObservedAt: "2026-10-03T18:46:00Z"}}}
	if err := ValidateLinkedProfiles(ok, now); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []LinkedProfile{{Provider: "cursor", URL: "https://evil.com/@x"}, {Provider: "custom", URL: "https://127.0.0.1/x"}, {Provider: "custom", URL: "http://a.com/x"}} {
		if err := ValidateLinkedProfiles([]LinkedProfile{bad}, now); err == nil {
			t.Fatalf("accepted %+v", bad)
		}
	}
}

func TestAggregateSVGEscapes(t *testing.T) {
	o, err := parseAggregateSVGOptions(map[string][]string{"layout": {"aggregate"}, "title": {"<b>x</b>"}})
	if err != nil {
		t.Fatal(err)
	}
	svg := renderAggregateSVG(o, []aggregateSource{{Label: `"><script>`, Status: "snapshot", Period: "all"}})
	if strings.Contains(svg, "<script>") || strings.Contains(svg, "<b>") || !strings.HasPrefix(svg, "<svg") {
		t.Fatal("unescaped SVG text")
	}
	if _, err := parseAggregateSVGOptions(map[string][]string{"bg": {"red"}}); err == nil {
		t.Fatal("invalid color accepted")
	}
}

type fakeAuth map[string]*pb.AuthorizeResponse

func (f fakeAuth) lookup(r *http.Request) (*pb.AuthorizeResponse, error) {
	token, _ := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
	if a := f[token]; a != nil {
		return a, nil
	}
	return nil, store.ErrUnauthorized
}
func (f fakeAuth) Verify(_ context.Context, r *http.Request) (*pb.AuthorizeResponse, error) {
	a, err := f.lookup(r)
	if err == nil && a.GetCredentialKind() != pb.CredentialKind_CREDENTIAL_KIND_SESSION {
		return nil, store.ErrForbidden
	}
	return a, err
}
func (f fakeAuth) VerifyAPIKey(_ context.Context, r *http.Request) (*pb.AuthorizeResponse, error) {
	a, err := f.lookup(r)
	if err == nil && a.GetCredentialKind() != pb.CredentialKind_CREDENTIAL_KIND_API_KEY {
		return nil, store.ErrForbidden
	}
	return a, err
}

// TestPostgresHTTP needs COMMUNITY_TEST_DSN: a fresh database with schema.sql
// installed. It is skipped otherwise and is not persistence evidence then.
func TestPostgresHTTP(t *testing.T) {
	dsn := os.Getenv("COMMUNITY_TEST_DSN")
	if dsn == "" {
		t.Skip("COMMUNITY_TEST_DSN not set")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	svc, err := New(context.Background(), db, "https://example.test")
	if err != nil {
		t.Fatal(err)
	}
	session := pb.CredentialKind_CREDENTIAL_KIND_SESSION
	auth := fakeAuth{
		"user":  {UserId: 900001, PlatformLevel: 1, CredentialKind: session},
		"l0":    {UserId: 900002, PlatformLevel: 0, CredentialKind: session},
		"admin": {UserId: 900003, PlatformLevel: 5, CredentialKind: session},
		"key":   {UserId: 900001, PlatformLevel: 1, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_API_KEY},
	}
	mux := http.NewServeMux()
	svc.Register(mux, auth)
	call := func(method, path, token, body string) (int, map[string]any) {
		req := httptest.NewRequest(method, path, strings.NewReader(body))
		if token != "" {
			req.Header.Set("Authorization", "Bearer "+token)
		}
		rec := httptest.NewRecorder()
		mux.ServeHTTP(rec, req)
		var out map[string]any
		_ = json.Unmarshal(rec.Body.Bytes(), &out)
		return rec.Code, out
	}
	t.Cleanup(func() {
		for _, q := range []string{`DELETE FROM lmm_community.profile_shares WHERE user_id>=900001`, `DELETE FROM lmm_community.onboarding_todos WHERE user_id>=900001`, `DELETE FROM lmm_community.signal_records WHERE user_id>=900001`, `DELETE FROM lmm_community.directory_settings`} {
			_, _ = db.Exec(q)
		}
	})
	if code, _ := call("PUT", "/api/ai-directory/links", "user", `{"links":[]}`); code != 403 {
		t.Fatalf("non-admin link update: %d", code)
	}
	if code, out := call("PUT", "/api/ai-directory/links", "admin", `{"links":[{"id":"x","name":"X","url":"https://x.com","category":"chat","summary":"","description":"","enabled":true}]}`); code != 200 {
		t.Fatalf("admin link update: %d %v", code, out)
	}
	if _, out := call("GET", "/api/ai-directory", "", ""); len(out["data"].(map[string]any)["links"].([]any)) != 1 {
		t.Fatalf("links: %v", out)
	}
	if code, out := call("POST", "/api/ai-directory/ads", "user", `{"name":"n","url":"https://x.com","summary":"","description":"","bid_cents":100,"expected_quota":1,"request_id":"aaaaaaaaaaaaaaaaaaaa"}`); code != 503 || out["code"] != "AI_DIRECTORY_AD_CURRENCY_UNAVAILABLE" {
		t.Fatalf("new paid ad must stay disabled: %d %v", code, out)
	}
	if code, _ := call("GET", "/api/user/self/profile-share", "", ""); code != 401 {
		t.Fatalf("anonymous share state: %d", code)
	}
	_, out := call("POST", "/api/user/self/profile-share", "user", `{"aggregate_usage_enabled":true,"linked_profiles":[{"provider":"chatgpt","url":"https://chatgpt.com/u/x","snapshot":{"tokens":7,"period":"all","observed_at":"2026-10-01T00:00:00Z","approximate":true}}]}`)
	data := out["data"].(map[string]any)
	token := data["token"].(string)
	if data["enabled"] != true || data["url"] != "https://example.test/api/share/profile/"+token+".svg" {
		t.Fatalf("share: %v", out)
	}
	req := httptest.NewRequest("GET", "/api/share/profile/"+token+".svg?layout=aggregate", nil)
	rec := httptest.NewRecorder()
	mux.ServeHTTP(rec, req)
	if rec.Code != 200 || !strings.Contains(rec.Body.String(), "≈7 tokens") {
		t.Fatalf("svg: %d %s", rec.Code, rec.Body.String())
	}
	call("DELETE", "/api/user/self/profile-share", "user", "")
	rec = httptest.NewRecorder()
	mux.ServeHTTP(rec, httptest.NewRequest("GET", "/api/share/profile/"+token+".svg?layout=aggregate", nil))
	if rec.Code != 404 {
		t.Fatalf("revoked svg: %d", rec.Code)
	}
	if _, out := call("GET", "/api/user/self/onboarding/todo", "l0", ""); out["data"].(map[string]any)["status"] != "unavailable" {
		t.Fatalf("l0 onboarding: %v", out)
	}
	if code, _ := call("POST", "/api/onboarding/todo/proof", "user", `{"step":"install_client","client":"cli"}`); code != 403 {
		t.Fatalf("session proof accepted: %d", code)
	}
	if code, _ := call("POST", "/api/onboarding/todo/proof", "key", `{"step":"configure_client","client":"cli","base_url":"https://x.com","group":"default"}`); code != 409 {
		t.Fatalf("out of order proof: %d", code)
	}
	call("POST", "/api/onboarding/todo/proof", "key", `{"step":"install_client","client":"cli"}`)
	_, out = call("PATCH", "/api/user/self/onboarding/todo", "user", "")
	if out["data"].(map[string]any)["current_step"] != "configure_client" {
		t.Fatalf("onboarding: %v", out)
	}
	_, out = call("POST", "/api/games/signal/attempts", "", `{"size":5,"actor":"human","model_id":"","harness":"","agent_name":""}`)
	attempt := out["data"].(map[string]any)
	day := attempt["day"].(string)
	seed, _ := signalDailySeed(day, 5)
	actions, _ := json.Marshal(solve(seed, 5))
	if code, _ := call("POST", "/api/games/signal/finish", "", `{"token":"`+attempt["token"].(string)+`","actions":`+string(actions)+`}`); code != 409 {
		t.Fatalf("countdown not enforced: %d", code)
	}
	svc.now = func() time.Time { return time.Now().Add(5 * time.Second) }
	if code, out := call("POST", "/api/games/signal/finish", "", `{"token":"`+attempt["token"].(string)+`","actions":`+string(actions)+`}`); code != 200 {
		t.Fatalf("finish: %d %v", code, out)
	}
	if code, out := call("POST", "/api/games/signal/records", "user", `{"mode":"challenge","size":5,"seed":0,"rules_version":2,"token":"`+attempt["token"].(string)+`","publish":true,"email":"","note":"hi","actor":"human","model_id":"","harness":"","agent_name":""}`); code != 200 {
		t.Fatalf("save: %d %v", code, out)
	}
	_, out = call("GET", "/api/games/signal/leaderboard?size=5&day="+day, "", "")
	found := false
	for _, e := range out["data"].(map[string]any)["entries"].([]any) {
		found = found || e.(map[string]any)["player"] == float64(900001)
	}
	if !found {
		t.Fatalf("leaderboard: %v", out)
	}
	if code, out := call("GET", "/api/rankings", "", ""); code != 503 || out["success"] != false {
		t.Fatalf("rankings must fail explicitly: %d", code)
	}
}
