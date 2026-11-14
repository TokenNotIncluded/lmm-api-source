// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant_test

import (
	"bytes"
	"context"
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/freshinstall"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/testkit"
	"github.com/lib/pq"
)

//go:embed testdata/history_v2.sql
var historyV2Fixture string

type historyUpgradeFixture struct {
	admin                                                           *sql.DB
	dsn, runtimeDSN, database, owner, runtime, installer, ownerFile string
}

func historyV3PG(t *testing.T) historyUpgradeFixture {
	t.Helper()
	path := os.Getenv("LMM_ASSISTANT_V3_TEST_ADMIN_DSN_FILE")
	if path == "" {
		t.Skip("dedicated isolated PostgreSQL fixture not configured")
	}
	st, err := os.Stat(path)
	if err != nil || st.Mode().Perm()&0077 != 0 {
		t.Fatal("private fixture DSN required")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	u, err := url.Parse(strings.TrimSpace(string(raw)))
	if err != nil || u.Query().Get("host") == "" || u.Hostname() != "" {
		t.Fatal("isolated Unix socket required")
	}
	a, err := sql.Open("postgres", u.String())
	if err != nil {
		t.Fatal(err)
	}
	a.SetMaxOpenConns(1)
	suffix := fmt.Sprintf("%d", time.Now().UnixNano())
	f := historyUpgradeFixture{admin: a, database: "lmm_assistant_test_" + suffix, owner: "ah_owner_" + suffix, runtime: "ah_runtime_" + suffix, installer: "ah_installer_" + suffix}
	for _, q := range []string{"CREATE ROLE " + pq.QuoteIdentifier(f.owner) + " NOLOGIN", "CREATE ROLE " + pq.QuoteIdentifier(f.runtime) + " NOLOGIN", "CREATE ROLE " + pq.QuoteIdentifier(f.installer) + " LOGIN", "GRANT " + pq.QuoteIdentifier(f.owner) + " TO " + pq.QuoteIdentifier(f.installer), "CREATE DATABASE " + pq.QuoteIdentifier(f.database) + " OWNER " + pq.QuoteIdentifier(f.owner) + " ENCODING 'UTF8' TEMPLATE template0", "REVOKE ALL ON DATABASE " + pq.QuoteIdentifier(f.database) + " FROM PUBLIC", "GRANT CONNECT ON DATABASE " + pq.QuoteIdentifier(f.database) + " TO " + pq.QuoteIdentifier(f.installer) + "," + pq.QuoteIdentifier(f.runtime)} {
		if _, err = a.Exec(q); err != nil {
			t.Fatal(err)
		}
	}
	u.Path = "/" + f.database
	f.dsn = u.String()
	u.User = url.UserPassword(f.runtime, "isolated-fixture-only")
	f.runtimeDSN = u.String()
	u.User = url.UserPassword(f.installer, "isolated-fixture-only")
	f.ownerFile = filepath.Join(t.TempDir(), "owner-url")
	if err = os.WriteFile(f.ownerFile, []byte(u.String()), 0600); err != nil {
		t.Fatal(err)
	}
	f.exec(t, "SET ROLE "+pq.QuoteIdentifier(f.owner)+";"+historyV2Fixture+"; GRANT USAGE ON SCHEMA lmm_assistant TO "+pq.QuoteIdentifier(f.runtime)+"; GRANT SELECT ON lmm_assistant.contract,lmm_assistant.conversations,lmm_assistant.history_messages TO "+pq.QuoteIdentifier(f.runtime))
	t.Cleanup(func() {
		a.Exec("DROP DATABASE " + pq.QuoteIdentifier(f.database))
		a.Exec("DROP ROLE " + pq.QuoteIdentifier(f.installer))
		a.Exec("DROP ROLE " + pq.QuoteIdentifier(f.runtime))
		a.Exec("DROP ROLE " + pq.QuoteIdentifier(f.owner))
		a.Close()
	})
	return f
}
func (f historyUpgradeFixture) exec(t *testing.T, q string) {
	t.Helper()
	db, e := sql.Open("postgres", f.dsn)
	if e != nil {
		t.Fatal(e)
	}
	defer db.Close()
	if _, e = db.Exec(q); e != nil {
		t.Fatal(e)
	}
}
func (f historyUpgradeFixture) command(extra ...string) error {
	args := []string{"--owner-url-file", f.ownerFile, "--database", f.database, "--owner-role", f.owner, "--runtime-role", f.runtime, "--operation-id", "assistant-upgrade-test-0001"}
	return freshinstall.RunAssistantHistoryUpgradeCLI(context.Background(), append(args, extra...), &bytes.Buffer{})
}
func TestHistoryV3OfflineUpgradeFencingRecoveryAndPreservation(t *testing.T) {
	f := historyV3PG(t)
	f.exec(t, `INSERT INTO lmm_assistant.conversations(id,user_id,title,last_message_preview,created_at,updated_at) OVERRIDING SYSTEM VALUE VALUES(1,9001,'before','preview',10,11);INSERT INTO lmm_assistant.history_messages(id,conversation_id,sequence,role,content,created_at) OVERRIDING SYSTEM VALUE VALUES(1,1,1,'user','unchanged',10)`)
	if e := f.command("--recover"); !errors.Is(e, access.ErrNotFound) {
		t.Fatalf("unapplied recovery: %v", e)
	}
	f.exec(t, "ALTER ROLE "+pq.QuoteIdentifier(f.runtime)+" LOGIN")
	if e := f.command(); e == nil {
		t.Fatal("unfenced runtime accepted")
	}
	f.exec(t, "ALTER ROLE "+pq.QuoteIdentifier(f.runtime)+" NOLOGIN")
	if e := f.command(); e != nil {
		t.Fatal(e)
	}
	if e := f.command("--recover"); e != nil {
		t.Fatal(e)
	}
	if e := f.command(); e != nil {
		t.Fatal("replay", e)
	}
	if e := f.command("--operation-id", "different-upgrade-test-0001"); !errors.Is(e, access.ErrConflict) {
		t.Fatalf("wrong operation: %v", e)
	}
	f.exec(t, "ALTER ROLE "+pq.QuoteIdentifier(f.runtime)+" LOGIN")
	db, e := sql.Open("postgres", f.runtimeDSN)
	if e != nil {
		t.Fatal(e)
	}
	defer db.Close()
	repo, e := assistant.NewPostgres(context.Background(), db, f.database)
	if e != nil {
		t.Fatal(e)
	}
	_, m, e := repo.ReadHistory(context.Background(), 9001, 1, 100)
	if e != nil || len(m) != 1 || m[0].Content != "unchanged" || m[0].ActorUserID != nil || m[0].Sequence != 1 {
		t.Fatalf("upgrade lost original: %+v %v", m, e)
	}
	if _, e = db.Exec(`UPDATE lmm_assistant.contract SET version=2`); e == nil {
		t.Fatal("runtime changed contract")
	}
	if _, e = db.Exec(`SELECT * FROM lmm_assistant.history_upgrades`); e == nil {
		t.Fatal("runtime read owner receipt")
	}
}
func TestHistoryV3ProjectsHumanOrderAuthorsShellsAndRejectsCrossOwner(t *testing.T) {
	f := historyV3PG(t)
	if e := f.command(); e != nil {
		t.Fatal(e)
	}
	f.exec(t, "SET ROLE "+pq.QuoteIdentifier(f.owner)+";"+`CREATE TABLE lmm_assistant.legacy_restore_batches(batch text PRIMARY KEY,lineage jsonb,visibility text);CREATE TABLE lmm_assistant.legacy_restore_conversations(batch text,source_id bigint,owner_user_id bigint,payload jsonb);CREATE TABLE lmm_assistant.legacy_restore_messages(batch text,source_id bigint,conversation_id bigint,sequence bigint,role text,actor_user_id bigint,payload jsonb);
 INSERT INTO lmm_assistant.legacy_restore_batches VALUES('frozen-fixture','{"source_kind":"FROZEN_OFFLINE_LEGACY","conversations":2,"messages":4,"human":2}','owner_only_raw');
 INSERT INTO lmm_assistant.legacy_restore_conversations SELECT 'frozen-fixture',id,9001,jsonb_build_object('id',id,'user_id',9001,'title','','last_message_preview','','created_at',10,'updated_at',20,'archived_at',0,'restricted_at',0,'restriction_reason','') FROM generate_series(101,102) id;
 INSERT INTO lmm_assistant.legacy_restore_messages SELECT 'frozen-fixture',id,101,seq,role,actor,jsonb_build_object('id',id,'conversation_id',101,'sequence',seq,'role',role,'actor_user_id',actor,'content','人工回复 retained','created_at',15) FROM (VALUES(1,10,'user',9001::bigint),(2,20,'human',9003::bigint),(3,30,'human',NULL::bigint),(4,40,'assistant',0::bigint)) x(id,seq,role,actor);`)
	f.exec(t, "GRANT SELECT ON lmm_assistant.legacy_restore_messages TO "+pq.QuoteIdentifier(f.runtime))
	if e := f.command("--project-batch", "frozen-fixture"); !errors.Is(e, access.ErrForbidden) {
		t.Fatalf("runtime could read raw staging: %v", e)
	}
	f.exec(t, "REVOKE SELECT ON lmm_assistant.legacy_restore_messages FROM "+pq.QuoteIdentifier(f.runtime))
	f.exec(t, `UPDATE lmm_assistant.legacy_restore_batches SET lineage=jsonb_set(lineage,'{source_kind}','"LIVE_ONLY_UNKNOWN"')`)
	if e := f.command("--project-batch", "frozen-fixture"); !errors.Is(e, access.ErrForbidden) {
		t.Fatalf("unknown lineage was projected: %v", e)
	}
	f.exec(t, `UPDATE lmm_assistant.legacy_restore_batches SET lineage=jsonb_set(lineage,'{source_kind}','"FROZEN_OFFLINE_LEGACY"');UPDATE lmm_assistant.legacy_restore_conversations SET owner_user_id=9002 WHERE source_id=101`)
	if e := f.command("--project-batch", "frozen-fixture"); !errors.Is(e, access.ErrConflict) {
		t.Fatalf("owner mismatch: %v", e)
	}
	f.exec(t, `UPDATE lmm_assistant.legacy_restore_conversations SET owner_user_id=9001 WHERE source_id=101;UPDATE lmm_assistant.legacy_restore_messages SET actor_user_id=-1,payload=jsonb_set(payload,'{actor_user_id}','-1') WHERE source_id=2`)
	if e := f.command("--project-batch", "frozen-fixture"); e == nil {
		t.Fatal("invalid known author accepted")
	}
	f.exec(t, `DO $$BEGIN IF EXISTS(SELECT FROM lmm_assistant.conversations) THEN RAISE EXCEPTION 'partial commit';END IF;END$$;UPDATE lmm_assistant.legacy_restore_messages SET actor_user_id=9003,payload=jsonb_set(payload,'{actor_user_id}','9003') WHERE source_id=2`)
	if e := f.command("--project-batch", "frozen-fixture"); e != nil {
		t.Fatal(e)
	}
	if e := f.command("--project-batch", "frozen-fixture", "--recover"); e != nil {
		t.Fatal(e)
	}
	if e := f.command("--project-batch", "frozen-fixture"); e != nil {
		t.Fatal("replay", e)
	}
	f.exec(t, "ALTER ROLE "+pq.QuoteIdentifier(f.runtime)+" LOGIN")
	db, e := sql.Open("postgres", f.runtimeDSN)
	if e != nil {
		t.Fatal(e)
	}
	defer db.Close()
	repo, e := assistant.NewPostgres(context.Background(), db, f.database)
	if e != nil {
		t.Fatal(e)
	}
	identity := testkit.NewIdentity()
	identity.Set("owner", 9001, 1)
	identity.Set("ledger-id", 9002, 6)
	auth, e := access.New(identity)
	if e != nil {
		t.Fatal(e)
	}
	service, e := assistant.NewHistory(repo, auth, []byte(strings.Repeat("c", 32)))
	if e != nil {
		t.Fatal(e)
	}
	detail, e := service.Detail(context.Background(), "owner", 101, 100)
	if e != nil {
		t.Fatal(e)
	}
	if len(detail.Messages) != 4 {
		t.Fatal(detail)
	}
	for i, m := range detail.Messages {
		if m.ConversationID != 101 || m.Sequence != int64((i+1)*10) || m.CreatedAt != 15 {
			t.Fatal("ordering/link/time", m)
		}
	}
	if detail.Messages[1].Role != "human" || detail.Messages[1].ActorUserID == nil || *detail.Messages[1].ActorUserID != 9003 || detail.Messages[2].Role != "human" || detail.Messages[2].ActorUserID != nil {
		t.Fatal("fabricated author/role", detail.Messages)
	}
	encoded, e := json.Marshal(detail)
	if e != nil || !bytes.Contains(encoded, []byte(`"actor_user_id":null`)) {
		t.Fatal("unknown author", e)
	}
	page, e := service.List(context.Background(), "owner", 0, false, 30, "")
	if e != nil || len(page.Conversations) != 1 || page.Conversations[0].ID != 101 {
		t.Fatal("shell visibility changed", page, e)
	}
	shell, e := service.Detail(context.Background(), "owner", 102, 100)
	if e != nil || len(shell.Messages) != 0 || shell.Conversation.ArchivedAt != 0 || shell.Conversation.Title != "" {
		t.Fatal("shell lost", shell, e)
	}
	// A later real message makes a retained shell eligible exactly as the old EXISTS rule did.
	f.exec(t, `INSERT INTO lmm_assistant.history_messages(id,conversation_id,sequence,role,actor_user_id,content,created_at) OVERRIDING SYSTEM VALUE VALUES(5,102,1,'user',9001,'later message',21)`)
	page, e = service.List(context.Background(), "owner", 0, false, 30, "")
	if e != nil || len(page.Conversations) != 2 {
		t.Fatal("retained shell stayed hidden after actual history", page, e)
	}
	if _, e = service.Detail(context.Background(), "ledger-id", 101, 100); !errors.Is(e, access.ErrNotFound) {
		t.Fatal("cross-owner", e)
	}
}
