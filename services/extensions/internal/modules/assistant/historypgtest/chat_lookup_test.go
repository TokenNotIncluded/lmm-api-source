// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Real PostgreSQL/HTTP and restricted SQL roles; Core authority/receipts are controlled fixtures.
package pgtest

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
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

type lookupReceiptPort struct {
	mu       sync.Mutex
	receipts map[string]string
	lookups  int
}

func (*lookupReceiptPort) BeginConversation(context.Context, string, string) (assistant.CoreConversationRegistration, error) {
	panic("lookup/recover must never Begin")
}
func (p *lookupReceiptPort) LookupConversation(_ context.Context, _ string, key string) (assistant.CoreConversationRegistration, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.lookups++
	if id := p.receipts[key]; id != "" {
		return assistant.CoreConversationRegistration{CreationKey: key, ID: id}, nil
	}
	return assistant.CoreConversationRegistration{}, access.ErrNotFound
}

type lookupNoModel struct {
	mu      sync.Mutex
	cancels int
}

func (*lookupNoModel) StartAgentTurn(context.Context, string, assistant.AgentTurnStart) (assistant.AgentTurnSnapshot, error) {
	panic("recovery dispatched model")
}
func (*lookupNoModel) GetAgentTurn(context.Context, string, assistant.AgentTurnKey) (assistant.AgentTurnSnapshot, error) {
	return assistant.AgentTurnSnapshot{}, access.ErrNotFound
}
func (*lookupNoModel) ClaimAgentTool(context.Context, string, assistant.AgentToolClaimInput) (assistant.AgentToolClaim, error) {
	panic("recovery claimed tool")
}
func (*lookupNoModel) CompleteAgentTool(context.Context, string, assistant.AgentToolCompletion) (assistant.AgentToolReceipt, error) {
	panic("recovery completed tool")
}
func (*lookupNoModel) StreamReady(context.Context) error { return nil }
func (*lookupNoModel) WatchAgentOutput(context.Context, string, assistant.AgentTurnKey, int64, []byte, func(assistant.AgentOutput) error) error {
	panic("mapping recovery observed model")
}
func (p *lookupNoModel) CancelAgentTurn(context.Context, string, assistant.AgentTurnKey) (assistant.AgentTurnSnapshot, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.cancels++
	return assistant.AgentTurnSnapshot{}, access.ErrNotFound
}
func lookupKey(turn string) string {
	s := sha256.Sum256([]byte("personal-chat-v1:" + turn))
	return hex.EncodeToString(s[:16])
}

type lookupHTTP struct{ base, token string }

func (h lookupHTTP) call(t *testing.T, method, path, token, body string) (int, []byte) {
	t.Helper()
	req, err := http.NewRequest(method, h.base+path, strings.NewReader(body))
	must(t, err)
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := http.DefaultClient.Do(req)
	must(t, err)
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	must(t, err)
	if strings.Contains(string(data), "creation_key") || strings.Contains(string(data), "core_conversation_id") {
		t.Fatal("internal mapping leaked", string(data))
	}
	return resp.StatusCode, data
}
func mappingResponse(t *testing.T, data []byte) assistant.ChatConversationMapping {
	t.Helper()
	var r struct {
		Data assistant.ChatConversationMapping `json:"data"`
	}
	must(t, json.Unmarshal(data, &r))
	return r.Data
}

type lookupPG struct {
	installer, runtime *sql.DB
	role, name         string
	authority          access.Authority
	identity           *testkit.Identity
	pending            *assistant.PendingConversations
	journal            *assistant.AgentPostgresJournal
	chat               *assistant.AgentChat
	receipt            *lookupReceiptPort
	model              *lookupNoModel
	http               lookupHTTP
}

func lookupDatabase(t *testing.T) lookupPG {
	t.Helper()
	ctx := context.Background()
	dsn := os.Getenv("LMM_MK09_TEST_DSN")
	base, err := url.Parse(dsn)
	if err != nil || dsn == "" || (base.Hostname() != "127.0.0.1" && base.Hostname() != "localhost") {
		t.Fatal("disposable local PostgreSQL DSN required")
	}
	admin, err := sql.Open("postgres", dsn)
	must(t, err)
	t.Cleanup(func() { admin.Close() })
	role := fmt.Sprintf("lookup_runtime_%d", time.Now().UnixNano())
	_, err = admin.Exec(`CREATE ROLE ` + role + ` LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS`)
	must(t, err)
	t.Cleanup(func() { _, err := admin.Exec(`DROP ROLE ` + role); must(t, err) })
	installer, name := promptPresetDatabase(t, admin, base)
	_, err = installer.Exec(assistant.Schema + assistant.DelegationSchema + assistant.AgentSchema)
	must(t, err)
	_, err = installer.Exec(`GRANT USAGE ON SCHEMA lmm_assistant TO ` + role + `; GRANT SELECT ON ALL TABLES IN SCHEMA lmm_assistant TO ` + role)
	must(t, err)
	u := *base
	u.Path = "/" + name
	u.User = url.User(role)
	runtime, err := sql.Open("postgres", u.String())
	must(t, err)
	runtime.SetMaxOpenConns(6)
	t.Cleanup(func() { runtime.Close() })
	identity := testkit.NewIdentity()
	token, other := "lmms_"+strings.Repeat("u", 43), "lmms_"+strings.Repeat("v", 43)
	identity.Set(token, 1, 1)
	identity.Set(other, 2, 6)
	authority, err := access.New(identity)
	must(t, err)
	repo, err := assistant.NewPostgres(ctx, runtime, name)
	must(t, err)
	history, err := assistant.NewHistory(repo, authority, []byte(strings.Repeat("k", 32)))
	must(t, err)
	delegation, err := assistant.NewDelegationPostgresJournal(ctx, runtime, name)
	must(t, err)
	receipt := &lookupReceiptPort{receipts: map[string]string{}}
	pending, err := assistant.NewPendingConversations(authority, delegation, receipt)
	must(t, err)
	journal, err := assistant.NewAgentPostgresJournal(ctx, runtime, name)
	must(t, err)
	model := &lookupNoModel{}
	chat, err := assistant.NewAgentChat(authority, pending, journal, model, &assistant.AgentRegistry{}, history)
	must(t, err)
	webAuth, err := webcompat.NewAuthenticator(identity, []byte(strings.Repeat("h", 32)))
	must(t, err)
	mux := http.NewServeMux()
	webcompat.RegisterAssistant(mux, webAuth, history, chat)
	server := httptest.NewServer(mux)
	t.Cleanup(server.Close)
	return lookupPG{installer, runtime, role, name, authority, identity, pending, journal, chat, receipt, model, lookupHTTP{server.URL, token}}
}
func (f lookupPG) seed(t *testing.T, turn string) int64 {
	t.Helper()
	var id int64
	must(t, f.installer.QueryRow(`INSERT INTO lmm_assistant.conversations(user_id,title,last_message_preview,created_at,updated_at) VALUES(1,'Pending','Pending',1,1) RETURNING id`).Scan(&id))
	_, err := f.installer.Exec(`INSERT INTO lmm_assistant.delegation_conversations(user_id,conversation_id,creation_key,phase) VALUES(1,$1,$2,'creation_inflight')`, id, lookupKey(turn))
	must(t, err)
	return id
}
func (f lookupPG) writes(t *testing.T) {
	t.Helper()
	_, err := f.installer.Exec(`GRANT UPDATE ON lmm_assistant.conversations,lmm_assistant.delegation_conversations TO ` + f.role + `; GRANT INSERT,UPDATE ON lmm_assistant.agent_turns TO ` + f.role)
	must(t, err)
}
func (f lookupPG) snapshot(t *testing.T) string {
	t.Helper()
	var out string
	must(t, f.installer.QueryRow(`SELECT jsonb_build_array((SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM lmm_assistant.conversations c),(SELECT jsonb_agg(to_jsonb(m) ORDER BY conversation_id) FROM lmm_assistant.delegation_conversations m),(SELECT count(*) FROM lmm_assistant.agent_turns),(SELECT count(*) FROM lmm_assistant.history_messages),(SELECT count(*) FROM lmm_assistant.turn_receipts))::text`).Scan(&out))
	return out
}

func TestChatLookupPostgresPureReadAuthorityMissingInflightAndExplicitReceiptRecovery(t *testing.T) {
	f := lookupDatabase(t)
	path := "/api/assistant/turns/original/conversation"
	id := f.seed(t, "original")
	registered := f.seed(t, "registered")
	_, err := f.installer.Exec(`UPDATE lmm_assistant.delegation_conversations SET phase='registered',core_conversation_id=$2 WHERE conversation_id=$1`, registered, "lmm_cv_"+strings.Repeat("d", 64))
	must(t, err)
	before := f.snapshot(t)
	for _, tc := range []struct {
		token, path string
		code        int
	}{
		{f.http.token, path, 202}, {f.http.token, "/api/assistant/turns/registered/conversation", 200}, {f.http.token, "/api/assistant/turns/missing/conversation", 404},
		{"lmms_" + strings.Repeat("v", 43), path, 404}, {"", path, 401}, {f.http.token, path + "?user_id=1", 400},
	} {
		code, data := f.http.call(t, "GET", tc.path, tc.token, "")
		if code != tc.code {
			t.Fatal(code, string(data), tc)
		}
		if code == 202 && mappingResponse(t, data).ConversationID != id {
			t.Fatal(string(data))
		}
	}
	if f.snapshot(t) != before || f.receipt.lookups != 0 {
		t.Fatal("pure read changed state or called Core registration")
	}
	var allowed bool
	must(t, f.runtime.QueryRow(`SELECT has_table_privilege(current_user,'lmm_assistant.conversations','UPDATE')`).Scan(&allowed))
	if allowed {
		t.Fatal("fixture is not SELECT-only")
	}
	code, data := f.http.call(t, "POST", path+"/recover", f.http.token, `{}`)
	if code != 202 || !strings.Contains(string(data), "ASSISTANT_CONVERSATION_RECOVERY_UNKNOWN") || f.snapshot(t) != before {
		t.Fatal(code, string(data))
	}
	// A real receipt exists, but SELECT-only cannot bind it: fail closed.
	f.receipt.receipts[lookupKey("original")] = "lmm_cv_" + strings.Repeat("a", 64)
	code, data = f.http.call(t, "POST", path+"/recover", f.http.token, `{}`)
	if code != 503 || f.snapshot(t) != before {
		t.Fatal(code, string(data))
	}
	f.writes(t)
	code, data = f.http.call(t, "POST", path+"/recover", f.http.token, `{}`)
	if code != 200 || mappingResponse(t, data).Phase != "registered" || mappingResponse(t, data).ConversationID != id {
		t.Fatal(code, string(data))
	}
	before = f.snapshot(t)
	code, data = f.http.call(t, "GET", path, f.http.token, "")
	if code != 200 || f.snapshot(t) != before {
		t.Fatal(code, string(data))
	}
	code, data = f.http.call(t, "POST", path+"/recover", f.http.token, `{"user_id":1}`)
	if code != 400 {
		t.Fatal(code, string(data))
	}
	f.identity.Revoke(f.http.token)
	code, data = f.http.call(t, "GET", path, f.http.token, "")
	if code != 401 {
		t.Fatal(code, string(data))
	}
}

func TestChatLookupPostgresUncommittedCreationIsNotProofOfAbsence(t *testing.T) {
	f := lookupDatabase(t)
	tx, err := f.installer.Begin()
	must(t, err)
	defer tx.Rollback()
	var id int64
	must(t, tx.QueryRow(`INSERT INTO lmm_assistant.conversations(user_id,title,last_message_preview,created_at,updated_at) VALUES(1,'Pending','Pending',1,1) RETURNING id`).Scan(&id))
	_, err = tx.Exec(`INSERT INTO lmm_assistant.delegation_conversations(user_id,conversation_id,creation_key,phase) VALUES(1,$1,$2,'creation_inflight')`, id, lookupKey("race"))
	must(t, err)
	path := "/api/assistant/turns/race/conversation"
	code, data := f.http.call(t, "GET", path, f.http.token, "")
	if code != 404 {
		t.Fatal(code, string(data))
	}
	must(t, tx.Commit())
	code, data = f.http.call(t, "GET", path, f.http.token, "")
	if code != 202 || mappingResponse(t, data).ConversationID != id {
		t.Fatal(code, string(data))
	}
	if f.receipt.lookups != 0 {
		t.Fatal("read sent registration RPC")
	}
}

func TestChatLookupPostgresUnstartedCancelFencesLateOriginalAndPreservesIntentUnknown(t *testing.T) {
	f := lookupDatabase(t)
	f.writes(t)
	id := f.seed(t, "fenced")
	f.receipt.receipts[lookupKey("fenced")] = "lmm_cv_" + strings.Repeat("b", 64)
	path := fmt.Sprintf("/api/assistant/conversations/%d/turns/fenced/cancel", id)
	code, data := f.http.call(t, "POST", path, f.http.token, `{}`)
	if code != 200 || !strings.Contains(string(data), `"state":"cancelled_before_start"`) {
		t.Fatal(code, string(data))
	}
	code, data = f.http.call(t, "POST", path, f.http.token, `{}`)
	if code != 200 {
		t.Fatal(code, string(data))
	}
	if _, err := f.chat.Chat(context.Background(), f.http.token, assistant.PersonalChatRequest{ClientTurnID: "fenced", Message: "late original"}); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("late original escaped durable fence", err)
	}
	var restricted int64
	must(t, f.installer.QueryRow(`SELECT restricted_at FROM lmm_assistant.conversations WHERE id=$1`, id).Scan(&restricted))
	if restricted <= 0 {
		t.Fatal("no durable fence")
	}
	// A second original intent holds the same real PG locks while cancellation
	// arrives. Only after it commits can cancel inspect it; absence cannot win.
	id2 := f.seed(t, "inflight")
	f.receipt.receipts[lookupKey("inflight")] = "lmm_cv_" + strings.Repeat("c", 64)
	_, err := f.pending.RecoverChatConversation(context.Background(), f.http.token, "inflight")
	must(t, err)
	entered, release := make(chan struct{}), make(chan struct{})
	intentDone := make(chan error, 1)
	go func() {
		intentDone <- f.journal.WithinAgent(context.Background(), 1, id2, "inflight", func(local *assistant.AgentLocalState) error {
			text := "original input"
			sum := sha256.Sum256([]byte(text))
			local.Turn = &assistant.AgentStoredTurn{ID: "inflight", Input: text, InputSHA256: hex.EncodeToString(sum[:]), Phase: "start_inflight", Tools: map[string]*assistant.AgentStoredTool{}}
			close(entered)
			<-release
			return nil
		})
	}()
	<-entered
	result := make(chan error, 1)
	go func() {
		closed, err := f.journal.FenceUnstarted(context.Background(), 1, id2, "inflight")
		if closed {
			err = fmt.Errorf("fenced an intent that won the lock")
		}
		result <- err
	}()
	select {
	case err := <-result:
		t.Fatal("fence did not wait on original intent", err)
	case <-time.After(30 * time.Millisecond):
	}
	close(release)
	must(t, <-intentDone)
	must(t, <-result)
	must(t, f.installer.QueryRow(`SELECT restricted_at FROM lmm_assistant.conversations WHERE id=$1`, id2).Scan(&restricted))
	if restricted != 0 {
		t.Fatal("existing uncertain intent marked safely closed")
	}
	_, err = f.chat.Cancel(context.Background(), f.http.token, id2, "inflight")
	if !errors.Is(err, assistant.ErrDelegationUnknown) {
		t.Fatal("missing Core ACK claimed terminal", err)
	}
	if f.model.cancels != 1 {
		t.Fatal("did not cancel original Core binding")
	}
}
