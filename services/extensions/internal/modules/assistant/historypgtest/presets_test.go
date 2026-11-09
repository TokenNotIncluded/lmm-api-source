// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Actual HTTP and isolated PostgreSQL; identity/model fixtures are synthetic.
package pgtest

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/testkit"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/webcompat"
)

type presetChatFixture struct {
	input assistant.PersonalChatRequest
	calls int
}

func (p *presetChatFixture) Chat(_ context.Context, _ string, input assistant.PersonalChatRequest) (assistant.PersonalChatReply, error) {
	p.input, p.calls = input, p.calls+1
	return assistant.PersonalChatReply{ConversationID: 1, ClientTurnID: input.ClientTurnID, State: "settled", Text: "Synthetic response"}, nil
}
func (*presetChatFixture) Recover(context.Context, string, int64, string) (assistant.PersonalChatReply, error) {
	return assistant.PersonalChatReply{}, access.ErrNotFound
}

func TestPromptPresetsPostgresHTTPAuthorityCASRestartAndBoundedStorage(t *testing.T) {
	dsn := os.Getenv("LMM_MK09_TEST_DSN")
	base, err := url.Parse(dsn)
	if err != nil || dsn == "" || (base.Hostname() != "127.0.0.1" && base.Hostname() != "localhost") {
		t.Fatal("a disposable local PostgreSQL instance is required")
	}
	admin, err := sql.Open("postgres", dsn)
	must(t, err)
	t.Cleanup(func() { admin.Close() })
	role := fmt.Sprintf("presets_runtime_%d", time.Now().UnixNano())
	_, err = admin.Exec(`CREATE ROLE ` + role + ` LOGIN PASSWORD 'presets-synthetic-only' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS`)
	must(t, err)
	t.Cleanup(func() { _, err := admin.Exec(`DROP ROLE ` + role); must(t, err) })
	installer, name := promptPresetDatabase(t, admin, base)
	_, err = installer.Exec(assistant.Schema)
	must(t, err)
	_, err = installer.Exec(`GRANT USAGE ON SCHEMA lmm_assistant TO ` + role + `;
GRANT SELECT ON lmm_assistant.contract,lmm_assistant.preset_stats TO ` + role + `;
GRANT SELECT,INSERT,UPDATE ON lmm_assistant.records TO ` + role + `;
GRANT UPDATE(clicks) ON lmm_assistant.preset_stats TO ` + role)
	must(t, err)
	u := *base
	u.Path, u.User = "/"+name, url.UserPassword(role, "presets-synthetic-only")
	identity := testkit.NewIdentity()
	user, reader, writer := "lmms_"+strings.Repeat("u", 43), "lmms_"+strings.Repeat("r", 43), "lmms_"+strings.Repeat("w", 43)
	identity.Set(user, 1, 1)
	identity.Set(reader, 2, 5)
	identity.Set(writer, 3, 6)
	authority, err := access.New(identity)
	must(t, err)
	webAuth, err := webcompat.NewAuthenticator(identity, []byte(strings.Repeat("h", 43)))
	must(t, err)
	chat := &presetChatFixture{}
	open := func() (*assistant.HistoryService, *httptest.Server, *sql.DB) {
		db, err := sql.Open("postgres", u.String())
		must(t, err)
		db.SetMaxOpenConns(4)
		t.Cleanup(func() { db.Close() })
		repo, err := assistant.NewPostgres(context.Background(), db, name)
		must(t, err)
		service, err := assistant.NewHistory(repo, authority, []byte(strings.Repeat("k", 32)))
		must(t, err)
		mux := http.NewServeMux()
		webcompat.RegisterAssistant(mux, webAuth, service, chat)
		server := httptest.NewServer(mux)
		t.Cleanup(server.Close)
		return service, server, db
	}
	_, server, db := open()
	request := func(host, method, path, token string, input any) (int, []byte) {
		var raw []byte
		if input != nil {
			var err error
			raw, err = json.Marshal(input)
			must(t, err)
		}
		r, err := http.NewRequest(method, host+path, strings.NewReader(string(raw)))
		must(t, err)
		r.Header.Set("Content-Type", "application/json")
		if token != "" {
			r.Header.Set("Authorization", "Bearer "+token)
		}
		response, err := server.Client().Do(r)
		must(t, err)
		defer response.Body.Close()
		body, err := io.ReadAll(response.Body)
		must(t, err)
		if response.Header.Get("Cache-Control") != "no-store" {
			t.Fatal("configuration was cacheable")
		}
		return response.StatusCode, body
	}
	const settingsPath = "/api/assistant/settings/pre-conversation-presets"
	const publicPath = "/api/assistant/pre-conversation-presets"
	readSettings := func(host, token string, want int) assistant.PromptPresetSettings {
		code, body := request(host, "GET", settingsPath, token, nil)
		if code != want {
			t.Fatalf("read status %d: %s", code, body)
		}
		var envelope struct {
			Data assistant.PromptPresetSettings `json:"data"`
		}
		must(t, json.Unmarshal(body, &envelope))
		return envelope.Data
	}
	put := func(host, token, config string, revision int64, want int) assistant.PromptPresetSettings {
		code, body := request(host, "PUT", settingsPath, token, map[string]any{"configuration": config, "revision": revision})
		if code != want {
			t.Fatalf("write status %d: %s", code, body)
		}
		var envelope struct {
			Data assistant.PromptPresetSettings `json:"data"`
		}
		must(t, json.Unmarshal(body, &envelope))
		return envelope.Data
	}
	readSettings(server.URL, user, 403)
	readSettings(server.URL, "", 401)
	initial := readSettings(server.URL, reader, 200)
	if initial != (assistant.PromptPresetSettings{}) {
		t.Fatal(initial)
	}
	put(server.URL, reader, "[]", 0, 403)
	raw := `[{"id":"custom_help","label":{"default":"Help","zh-TW":"協助"},"prompt":{"default":"Help me","zh_Hant":"請協助我"}}]`
	saved := put(server.URL, writer, raw, 0, 200)
	if saved.Configuration != raw || saved.Revision != 1 || saved.UpdatedAt <= 0 {
		t.Fatal(saved)
	}
	code, body := request(server.URL, "GET", publicPath+"?language=zh-TW&copy_version=legacy", "", nil)
	if code != 200 || !strings.Contains(string(body), "請協助我") || !strings.Contains(string(body), `"source":"custom"`) || !strings.Contains(string(body), `"version":"custom-v1"`) {
		t.Fatal(code, string(body))
	}
	code, body = request(server.URL, "POST", "/api/assistant/chat", user, map[string]any{"preset_id": "custom_help", "client_turn_id": "custom-turn", "message": "Edited user text"})
	if code != 200 || chat.calls != 1 || chat.input.Message != "Edited user text" || chat.input.ClientTurnID != "custom-turn" {
		t.Fatal(code, string(body), chat)
	}
	code, _ = request(server.URL, "POST", "/api/assistant/chat", user, map[string]any{"preset_id": "custom_help", "message": "text", "payer": 3, "tools": []string{"gift.claim"}})
	if code != 400 || chat.calls != 1 {
		t.Fatal("preset granted authority", code, chat.calls)
	}
	code, _ = request(server.URL, "POST", publicPath+"/getting_started/click", "", nil)
	if code != 404 {
		t.Fatal("unpublished builtin click accepted", code)
	}
	var wg sync.WaitGroup
	for n := 0; n < 12; n++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			code, body := request(server.URL, "POST", publicPath+"/custom_help/click", "", nil)
			if code != 200 {
				t.Errorf("click status %d: %s", code, body)
			}
		}()
	}
	wg.Wait()
	var clicks int
	must(t, db.QueryRow(`SELECT (body->>'clicks')::int FROM lmm_assistant.records WHERE scope='public:prompt-presets' AND bucket='preferences' AND id='pre-conversation-presets:clicks:0'`).Scan(&clicks))
	if clicks != 12 {
		t.Fatal("lost concurrent anonymous clicks", clicks)
	}
	// A fresh pool, repository, service and HTTP server read the committed copy.
	_, restarted, _ := open()
	if recovered := readSettings(restarted.URL, reader, 200); recovered != saved {
		t.Fatal("restart lost configuration", recovered)
	}
	statuses := make(chan int, 2)
	for _, host := range []string{server.URL, restarted.URL} {
		wg.Add(1)
		go func(host string) {
			defer wg.Done()
			code, _ := request(host, "PUT", settingsPath, writer, map[string]any{"configuration": "[]", "revision": saved.Revision})
			statuses <- code
		}(host)
	}
	wg.Wait()
	close(statuses)
	counts := map[int]int{}
	for code := range statuses {
		counts[code]++
	}
	if counts[200] != 1 || counts[409] != 1 {
		t.Fatal("CAS failed across services", counts)
	}
	code, body = request(server.URL, "GET", publicPath, "", nil)
	if code != 200 || !strings.Contains(string(body), `"presets":[]`) {
		t.Fatal(code, string(body))
	}
	code, _ = request(server.URL, "POST", publicPath+"/custom_help/click", "", nil)
	if code != 404 {
		t.Fatal("deleted preset remained clickable", code)
	}
	code, _ = request(server.URL, "POST", "/api/assistant/chat", user, map[string]any{"preset_id": "custom_help", "message": "text", "client_turn_id": "deleted"})
	if code != 400 || chat.calls != 1 {
		t.Fatal("deleted preset reached chat", code)
	}
	saved = put(server.URL, writer, "", 2, 200)
	code, _ = request(server.URL, "POST", publicPath+"/getting_started/click", "", nil)
	if code != 200 {
		t.Fatal("default preset not restored", code)
	}
	must(t, db.QueryRow(`SELECT clicks FROM lmm_assistant.preset_stats WHERE preset_id='getting_started'`).Scan(&clicks))
	if clicks != 1 {
		t.Fatal("seed analytics changed", clicks)
	}
	entries := []map[string]string{}
	for n := 0; n < 20; n++ {
		entries = append(entries, map[string]string{"id": fmt.Sprintf("large_%d", n), "label": "Label", "prompt": strings.Repeat("😀", 2000)})
	}
	large, err := json.Marshal(entries)
	must(t, err)
	saved = put(server.URL, writer, string(large), saved.Revision, 200)
	if readSettings(restarted.URL, reader, 200) != saved {
		t.Fatal("large Unicode configuration lost on reload")
	}
	var records, maxBytes int
	must(t, db.QueryRow(`SELECT count(*),max(octet_length(body::text)) FROM lmm_assistant.records WHERE scope='public:prompt-presets'`).Scan(&records, &maxBytes))
	if records > 41 || maxBytes > 16384 {
		t.Fatal("unbounded persisted preset records", records, maxBytes)
	}
	put(server.URL, writer, `[{"id":"invalid","label":"Label","prompt":""}]`, saved.Revision, 400)
	if readSettings(restarted.URL, reader, 200) != saved {
		t.Fatal("invalid write changed committed configuration")
	}
	identity.Set(writer, 3, 5)
	put(server.URL, writer, "[]", saved.Revision, 403)
	identity.Revoke(reader)
	readSettings(server.URL, reader, 401)
	if _, err := db.Exec(`CREATE TABLE public.unauthorized_preset_runtime(x int)`); err == nil {
		t.Fatal("runtime acquired DDL authority")
	}
	_, err = installer.Exec(`UPDATE lmm_assistant.records SET body=jsonb_set(body,'{chunks}','21'::jsonb) WHERE scope='public:prompt-presets' AND id='pre-conversation-presets'`)
	must(t, err)
	code, body = request(server.URL, "GET", publicPath, "", nil)
	if code != 503 || strings.Contains(string(body), `"success":true`) {
		t.Fatal("corruption silently became default success", code, string(body))
	}
}

func promptPresetDatabase(t *testing.T, admin *sql.DB, base *url.URL) (*sql.DB, string) {
	t.Helper()
	name := fmt.Sprintf("lmm_assistant_presets_%d", time.Now().UnixNano())
	_, err := admin.Exec(`CREATE DATABASE ` + name)
	must(t, err)
	u := *base
	u.Path = "/" + name
	db, err := sql.Open("postgres", u.String())
	must(t, err)
	t.Cleanup(func() {
		db.Close()
		_, err := admin.Exec(`DROP DATABASE ` + name + ` WITH (FORCE)`)
		must(t, err)
	})
	return db, name
}
