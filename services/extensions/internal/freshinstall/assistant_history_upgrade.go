// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package freshinstall

import (
	"context"
	"database/sql"
	"encoding/json"
	"flag"
	"io"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/lib/pq"
)

// RunAssistantHistoryUpgradeCLI is an explicit offline owner boundary. No
// server startup, Core client, model executor or financial grant is involved.
func RunAssistantHistoryUpgradeCLI(ctx context.Context, args []string, out io.Writer) error {
	var c Config
	var operation, batch string
	var recoverOnly bool
	f := flag.NewFlagSet("upgrade-assistant-history-db", flag.ContinueOnError)
	f.SetOutput(io.Discard)
	f.StringVar(&c.OwnerURLFile, "owner-url-file", "", "private installation connection")
	f.StringVar(&c.ExpectedDatabase, "database", "", "exact existing assistant database")
	f.StringVar(&c.OwnerRole, "owner-role", "", "NOLOGIN database owner")
	f.StringVar(&c.RuntimeRole, "runtime-role", "", "fenced NOLOGIN runtime")
	f.StringVar(&operation, "operation-id", "", "original stable operation ID")
	f.StringVar(&batch, "project-batch", "", "project this owner-only frozen batch after upgrading to v3")
	f.BoolVar(&recoverOnly, "recover", false, "read original operation outcome")
	if f.Parse(args) != nil || f.NArg() != 0 {
		return ErrRejected
	}
	c.Component = "assistant"
	plan, err := compiled(c.Component)
	if err != nil {
		return err
	}
	dsn, err := ownerURL(c, plan)
	if err != nil {
		return err
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		return ErrRejected
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		return ErrRejected
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `SET LOCAL lock_timeout='2s'; SET LOCAL statement_timeout='10s'`); err != nil {
		return ErrRejected
	}
	var allowed bool
	err = tx.QueryRowContext(ctx, `SELECT current_database()=$1 AND pg_get_userbyid(d.datdba)=$2 AND session_user<>$3
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,$2,$3) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user AND (has_database_privilege(oid,d.oid,'CONNECT') OR pg_has_role(oid,$2,'SET') OR pg_has_role(oid,$3,'SET')))
 FROM pg_database d WHERE datname=current_database()`, c.ExpectedDatabase, c.OwnerRole, c.RuntimeRole).Scan(&allowed)
	if err != nil || !allowed {
		return ErrRejected
	}
	if _, err = tx.ExecContext(ctx, "SET LOCAL ROLE "+pq.QuoteIdentifier(c.OwnerRole)); err != nil {
		return ErrRejected
	}
	if batch == "" {
		err = assistant.UpgradeHistory(ctx, tx, c.RuntimeRole, operation, recoverOnly)
	} else {
		err = assistant.ProjectLegacyHistory(ctx, tx, c.RuntimeRole, operation, batch, recoverOnly)
	}
	if err != nil {
		return err
	}
	if !recoverOnly {
		if err = tx.Commit(); err != nil {
			return ErrCommitUnknown
		}
	}
	if json.NewEncoder(out).Encode(map[string]any{"database": c.ExpectedDatabase, "version": 3, "operation_id": operation, "project_batch": batch, "recovered": recoverOnly}) != nil {
		return ErrCommitUnknown
	}
	return nil
}
