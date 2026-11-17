// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package freshinstall

import (
	"context"
	"database/sql"
	"encoding/json"
	"flag"
	"io"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"github.com/lib/pq"
)

func RunCheckinSettingsUpgradeCLI(parent context.Context, args []string, out io.Writer) error {
	var c Config
	var operation string
	var recoverOnly bool
	f := flag.NewFlagSet("upgrade-checkin-settings", flag.ContinueOnError)
	f.SetOutput(io.Discard)
	f.StringVar(&c.OwnerURLFile, "owner-url-file", "", "private offline owner connection")
	f.StringVar(&c.ExpectedDatabase, "database", "", "exact extension database")
	f.StringVar(&c.OwnerRole, "owner-role", "", "NOLOGIN database owner")
	f.StringVar(&c.RuntimeRole, "runtime-role", "", "fenced NOLOGIN runtime")
	f.StringVar(&operation, "operation-id", "", "stable operation ID")
	f.BoolVar(&recoverOnly, "recover", false, "read the original result without applying")
	if f.Parse(args) != nil || f.NArg() != 0 {
		return ErrRejected
	}
	dsn, e := ownerURL(c, plan{prefix: "lmm_checkins"})
	if e != nil {
		return ErrRejected
	}
	db, e := sql.Open("postgres", dsn)
	if e != nil {
		return ErrRejected
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	ctx, cancel := context.WithTimeout(parent, 15*time.Second)
	defer cancel()
	tx, e := db.BeginTx(ctx, nil)
	if e != nil {
		return ErrRejected
	}
	defer tx.Rollback()
	if _, e = tx.ExecContext(ctx, `SET LOCAL lock_timeout='2s'; SET LOCAL statement_timeout='10s'`); e != nil {
		return ErrRejected
	}
	var allowed bool
	e = tx.QueryRowContext(ctx, `SELECT current_database()=$1 AND pg_get_userbyid(d.datdba)=$2 AND session_user<>$3
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$2 AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,$2,$3) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_auth_members WHERE member=(SELECT oid FROM pg_roles WHERE rolname=$3))
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
   AND (has_database_privilege(oid,d.oid,'CONNECT') OR pg_has_role(oid,$2,'SET') OR pg_has_role(oid,$3,'SET')))
 FROM pg_database d WHERE datname=current_database()`, c.ExpectedDatabase, c.OwnerRole, c.RuntimeRole).Scan(&allowed)
	if e != nil || !allowed {
		return ErrRejected
	}
	if _, e = tx.ExecContext(ctx, "SET LOCAL ROLE "+pq.QuoteIdentifier(c.OwnerRole)); e != nil {
		return ErrRejected
	}
	if e = checkins.UpgradeSettings(ctx, tx, c.RuntimeRole, operation, recoverOnly); e != nil {
		return e
	}
	if !recoverOnly {
		if tx.Commit() != nil {
			return ErrCommitUnknown
		}
	}
	return json.NewEncoder(out).Encode(map[string]any{"database": c.ExpectedDatabase, "operation_id": operation, "settings_version": 1, "recovered": recoverOnly})
}
