// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"regexp"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/freshinstall"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/lib/pq"
)

// This fixture is newly provisioned and disposable: the installer is a safe
// LOGIN member of OwnerRole, while RuntimeRole has no memberships. The separate
// administrator injects and restores ACL/schema faults only in this database.
func TestPostgresAssistantStorageBundleGuard(t *testing.T) {
	path := os.Getenv("LMM_ASSISTANT_STORAGE_TEST_MANIFEST")
	if path == "" {
		t.Skip("requires a newly owned isolated PostgreSQL 18 fixture")
	}
	var f struct {
		OwnerURLFile, RuntimeURLFile, AdminURLFile string
		Database, OwnerRole, RuntimeRole           string
	}
	raw, err := coreclient.ReadPrivateFile(path, 8192)
	if err != nil || json.Unmarshal(raw, &f) != nil {
		t.Fatal("invalid private assistant fixture")
	}
	id := regexp.MustCompile(`^lmm_assistant_([a-f0-9]{12})_guard$`).FindStringSubmatch(f.Database)
	if len(id) != 2 || f.OwnerRole != "a08_"+id[1]+"_o" || f.RuntimeRole != "a08_"+id[1]+"_r" {
		t.Fatal("invalid isolated assistant fixture names")
	}
	for _, path := range []string{f.OwnerURLFile, f.RuntimeURLFile, f.AdminURLFile} {
		raw, err := coreclient.ReadPrivateFile(path, 8192)
		if err != nil {
			t.Fatal("private fixture connection unavailable")
		}
		u, err := url.Parse(strings.TrimSpace(string(raw)))
		if err != nil || u.Scheme != "postgres" || u.Hostname() != "127.0.0.1" || u.Path != "/"+f.Database {
			t.Fatal("fixture must target the named database on numeric loopback")
		}
	}
	ctx := context.Background()
	if _, err := freshinstall.Install(ctx, freshinstall.Config{OwnerURLFile: f.OwnerURLFile, ExpectedDatabase: f.Database, OwnerRole: f.OwnerRole, RuntimeRole: f.RuntimeRole, Component: "assistant"}); err != nil {
		t.Fatal("formal fresh assistant installation failed", err)
	}
	dsn, err := coreclient.ReadPrivateFile(f.AdminURLFile, 8192)
	if err != nil {
		t.Fatal("private fixture administrator unavailable")
	}
	admin, err := sql.Open("postgres", strings.TrimSpace(string(dsn)))
	if err != nil {
		t.Fatal("private fixture administrator unavailable")
	}
	defer admin.Close()
	var database string
	if err := admin.QueryRowContext(ctx, `SELECT current_database()`).Scan(&database); err != nil || database != f.Database {
		t.Fatal("fixture administrator connected to the wrong database")
	}
	cfg := StorageConfig{Component: "assistant", URLFile: f.RuntimeURLFile, Database: f.Database}
	role, owner := pq.QuoteIdentifier(f.RuntimeRole), pq.QuoteIdentifier(f.OwnerRole)
	exec := func(t *testing.T, query string) {
		t.Helper()
		if _, err := admin.ExecContext(ctx, query); err != nil {
			t.Fatal("owned fixture mutation failed", err)
		}
	}
	mutate := func(t *testing.T, query, restore string) {
		t.Helper()
		exec(t, query)
		t.Cleanup(func() { exec(t, restore) })
	}
	check := func(t *testing.T, bundle storageBundle, wantReady bool) {
		t.Helper()
		db, err := openStorageBundle(ctx, cfg, bundle)
		if db != nil {
			db.Close()
		}
		if (err == nil) != wantReady {
			t.Fatalf("bundle %d readiness = %v, want %v: %v", bundle, err == nil, wantReady, err)
		}
	}
	bundles := []storageBundle{defaultStorageBundle, assistantDelegationBundle, assistantAgentBundle}
	for _, bundle := range bundles {
		check(t, bundle, true)
	}

	// Exercise inserts and the exact column updates used by all three journals.
	// Roll back the synthetic conversation so later fault cases share no data.
	t.Run("journal writes", func(t *testing.T) {
		db, err := openStorageBundle(ctx, cfg, assistantAgentBundle)
		if err != nil {
			t.Fatal(err)
		}
		defer db.Close()
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			t.Fatal(err)
		}
		defer tx.Rollback()
		var conversation int64
		if err := tx.QueryRowContext(ctx, `INSERT INTO lmm_assistant.conversations(user_id,title,last_message_preview,created_at,updated_at) VALUES(1,'A08 fixture','A08 fixture',1,1) RETURNING id`).Scan(&conversation); err != nil {
			t.Fatal("runtime conversation insert failed", err)
		}
		for _, query := range []string{
			`INSERT INTO lmm_assistant.delegation_conversations(user_id,conversation_id,creation_key,phase) VALUES(1,$1,'a08-fixture','creation_inflight')`,
			`UPDATE lmm_assistant.delegation_conversations SET core_conversation_id='lmm_cv_'||repeat('a',64),phase='registered' WHERE conversation_id=$1`,
			`INSERT INTO lmm_assistant.delegation_turns(user_id,conversation_id,turn_id,body) VALUES(1,$1,'a08-fixture','{}')`,
			`UPDATE lmm_assistant.delegation_turns SET body='{"fixture":true}',revision=revision+1 WHERE conversation_id=$1`,
			`INSERT INTO lmm_assistant.agent_turns(user_id,conversation_id,turn_id,body,body_bytes) VALUES(1,$1,'a08-fixture','{}',convert_to('{}','UTF8'))`,
			`UPDATE lmm_assistant.agent_turns SET body='{"fixture":true}',body_bytes=convert_to('{"fixture":true}','UTF8'),revision=revision+1 WHERE conversation_id=$1`,
		} {
			if result, err := tx.ExecContext(ctx, query, conversation); err != nil {
				t.Fatal("minimal runtime journal write failed", err)
			} else if count, err := result.RowsAffected(); err != nil || count != 1 {
				t.Fatal("runtime journal write did not affect its fixture row")
			}
		}
	})

	// Even an installed but unselected agent table must not hide PUBLIC rights,
	// delegation authority, or runtime ownership from the schema-wide ACL guard.
	for _, fault := range []struct{ name, query, restore string }{
		{"PUBLIC schema", "GRANT USAGE ON SCHEMA lmm_assistant TO PUBLIC", "REVOKE USAGE ON SCHEMA lmm_assistant FROM PUBLIC"},
		{"PUBLIC table", "GRANT SELECT ON lmm_assistant.agent_turns TO PUBLIC", "REVOKE SELECT ON lmm_assistant.agent_turns FROM PUBLIC"},
		{"PUBLIC column", "GRANT UPDATE(body) ON lmm_assistant.agent_turns TO PUBLIC", "REVOKE UPDATE(body) ON lmm_assistant.agent_turns FROM PUBLIC"},
		{"schema grant option", "GRANT USAGE ON SCHEMA lmm_assistant TO " + role + " WITH GRANT OPTION", "REVOKE GRANT OPTION FOR USAGE ON SCHEMA lmm_assistant FROM " + role},
		{"table grant option", "GRANT SELECT ON lmm_assistant.agent_turns TO " + role + " WITH GRANT OPTION", "REVOKE GRANT OPTION FOR SELECT ON lmm_assistant.agent_turns FROM " + role},
		{"column grant option", "GRANT UPDATE(body) ON lmm_assistant.agent_turns TO " + role + " WITH GRANT OPTION", "REVOKE GRANT OPTION FOR UPDATE(body) ON lmm_assistant.agent_turns FROM " + role},
		{"runtime owner", "ALTER TABLE lmm_assistant.agent_contract OWNER TO " + role, "ALTER TABLE lmm_assistant.agent_contract OWNER TO " + owner + "; GRANT SELECT ON lmm_assistant.agent_contract TO " + role},
		{"owner role membership", "GRANT " + owner + " TO " + role + " WITH INHERIT TRUE, SET TRUE", "REVOKE " + owner + " FROM " + role},
		{"missing schema USAGE", "REVOKE USAGE ON SCHEMA lmm_assistant FROM " + role, "GRANT USAGE ON SCHEMA lmm_assistant TO " + role},
		{"schema CREATE", "GRANT CREATE ON SCHEMA lmm_assistant TO " + role, "REVOKE CREATE ON SCHEMA lmm_assistant FROM " + role},
	} {
		t.Run(fault.name, func(t *testing.T) {
			mutate(t, fault.query, fault.restore)
			for _, bundle := range bundles {
				check(t, bundle, false)
			}
		})
	}

	for _, fault := range []struct{ name, query, restore string }{
		{"missing sequence USAGE", "REVOKE USAGE ON SEQUENCE lmm_assistant.memories_id_seq FROM " + role, "GRANT USAGE ON SEQUENCE lmm_assistant.memories_id_seq TO " + role},
		{"sequence UPDATE", "GRANT UPDATE ON SEQUENCE lmm_assistant.memories_id_seq TO " + role, "REVOKE UPDATE ON SEQUENCE lmm_assistant.memories_id_seq FROM " + role},
		{"missing sequence", "ALTER SEQUENCE lmm_assistant.memories_id_seq RENAME TO memories_id_seq_absent", "ALTER SEQUENCE lmm_assistant.memories_id_seq_absent RENAME TO memories_id_seq"},
	} {
		t.Run(fault.name, func(t *testing.T) {
			mutate(t, fault.query, fault.restore)
			check(t, assistantAgentBundle, false)
		})
	}

	for _, journal := range []struct {
		name    string
		columns []string
		bundle  storageBundle
	}{
		{"delegation_conversations", []string{"core_conversation_id", "phase"}, assistantDelegationBundle},
		{"delegation_turns", []string{"body", "revision"}, assistantDelegationBundle},
		{"agent_turns", []string{"body", "body_bytes", "revision"}, assistantAgentBundle},
	} {
		table := "lmm_assistant." + journal.name
		for _, privilege := range []string{"SELECT", "INSERT"} {
			t.Run(journal.name+" missing "+privilege, func(t *testing.T) {
				mutate(t, "REVOKE "+privilege+" ON "+table+" FROM "+role, "GRANT "+privilege+" ON "+table+" TO "+role)
				check(t, journal.bundle, false)
			})
		}
		for _, privilege := range []string{"UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN", "UPDATE(user_id)"} {
			t.Run(journal.name+" extra "+privilege, func(t *testing.T) {
				mutate(t, "GRANT "+privilege+" ON "+table+" TO "+role, "REVOKE "+privilege+" ON "+table+" FROM "+role)
				for _, bundle := range bundles {
					check(t, bundle, false)
				}
			})
		}
		for _, column := range journal.columns {
			t.Run(journal.name+" missing UPDATE "+column, func(t *testing.T) {
				privilege := "UPDATE(" + column + ") ON " + table
				mutate(t, "REVOKE "+privilege+" FROM "+role, "GRANT "+privilege+" TO "+role)
				check(t, journal.bundle, false)
			})
		}
	}

	for _, object := range []struct {
		name    string
		bundle  storageBundle
		version int
	}{
		{"contract", defaultStorageBundle, 2},
		{"records", defaultStorageBundle, 0},
		{"delegation_contract", assistantDelegationBundle, 1},
		{"delegation_conversations", assistantDelegationBundle, 0},
		{"delegation_turns", assistantDelegationBundle, 0},
		{"agent_contract", assistantAgentBundle, 1},
		{"agent_turns", assistantAgentBundle, 0},
		{"services_contract", assistantAgentBundle, 1},
	} {
		table := "lmm_assistant." + object.name
		t.Run("missing "+object.name, func(t *testing.T) {
			mutate(t, "ALTER TABLE "+table+" RENAME TO "+object.name+"_absent", "ALTER TABLE "+table+"_absent RENAME TO "+object.name)
			check(t, object.bundle, false)
			var missing bool
			if err := admin.QueryRowContext(ctx, `SELECT to_regclass($1) IS NULL`, table).Scan(&missing); err != nil || !missing {
				t.Fatal("startup recreated a missing table")
			}
		})
		if object.version == 0 {
			continue
		}
		t.Run("version "+object.name, func(t *testing.T) {
			var constraint, definition string
			if err := admin.QueryRowContext(ctx, `SELECT c.conname,pg_get_constraintdef(c.oid) FROM pg_constraint c JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attname='version' WHERE c.conrelid=to_regclass($1) AND c.contype='c' AND c.conkey=ARRAY[a.attnum]`, table).Scan(&constraint, &definition); err != nil {
				t.Fatal("installed version constraint unavailable", err)
			}
			constraint = pq.QuoteIdentifier(constraint)
			mutate(t, "ALTER TABLE "+table+" DROP CONSTRAINT "+constraint,
				fmt.Sprintf("UPDATE %s SET version=%d; ALTER TABLE %s ADD CONSTRAINT %s %s", table, object.version, table, constraint, definition))
			exec(t, "UPDATE "+table+" SET version=version+1")
			check(t, object.bundle, false)
		})
	}

	// AgentChat must assemble with or without PersonalText capabilities; an
	// explicit empty extension selection still exercises its real storage.
	for _, mode := range []struct {
		name, token  string
		required     bool
		personalText bool
	}{
		{"off", "", false, true},
		{"text", "configured", false, true},
		{"agent", "configured", true, true},
		{"agent only", "configured", true, false},
	} {
		t.Run(mode.name, func(t *testing.T) {
			features := []string{
				coreclient.AssistantAgentTurnFeature,
				fmt.Sprintf("assistant.agent_extensions.sha256:%x", sha256.Sum256([]byte("[]"))),
			}
			if mode.personalText {
				features = append(features, coreclient.AssistantPersonalTextReserveFeature, coreclient.AssistantPersonalTextExecuteFeature)
			}
			socket, control, capabilities := assistantCapabilityFixture(t, features)
			if !mode.required {
				mutate(t, "ALTER TABLE lmm_assistant.agent_turns RENAME TO agent_turns_absent", "ALTER TABLE lmm_assistant.agent_turns_absent RENAME TO agent_turns")
			}
			config := Config{Socket: socket, Assistant: cfg, AssistantTokenFile: mode.token, AssistantAgentRequired: mode.required}
			db, err := openStorageBundle(ctx, cfg, assistantStorageBundle(config))
			if err != nil {
				t.Fatal("configured assistant storage unavailable", err)
			}
			defer db.Close()
			authority, err := access.New(control)
			if err != nil {
				t.Fatal(err)
			}
			repo, err := assistant.NewPostgres(ctx, db, cfg.Database)
			if err != nil {
				t.Fatal(err)
			}
			history, err := assistant.NewHistory(repo, authority, []byte(strings.Repeat("h", 32)))
			if err != nil {
				t.Fatal(err)
			}
			runtime := &Runtime{Client: control, AssistantHistory: history}
			defer runtime.Close()
			var token []byte
			if mode.token != "" {
				token = []byte(strings.Repeat("a", 32))
			}
			before := capabilities.calls.Load()
			if err := runtime.composeAssistantDelegation(ctx, config, db, authority, token); err != nil {
				t.Fatal("configured assistant composition failed", err)
			}
			switch mode.name {
			case "off":
				if runtime.AssistantChat != nil || runtime.AssistantModelClient != nil || capabilities.calls.Load() != before {
					t.Fatal("unconfigured assistant probed or enabled Core capabilities")
				}
			case "text":
				if _, ok := runtime.AssistantChat.(*assistant.PersonalChat); !ok || runtime.AssistantServices != nil {
					t.Fatal("disabled agent did not preserve PersonalChat")
				}
				if calls := capabilities.calls.Load() - before; calls != 2 {
					t.Fatalf("personal mode made %d probes; want only PersonalText and gift readiness", calls)
				}
			case "agent", "agent only":
				if _, ok := runtime.AssistantChat.(*assistant.AgentChat); !ok || runtime.AssistantServices == nil || runtime.AssistantPending == nil {
					t.Fatal("required ready agent did not compose AgentChat")
				}
				if runtime.AssistantTurns != nil {
					t.Fatal("required agent composed PersonalText turns")
				}
			}
		})
	}
	for _, bundle := range bundles {
		check(t, bundle, true)
	}
}
