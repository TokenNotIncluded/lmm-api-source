// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package freshinstall

import (
	"context"
	"database/sql"
	"encoding/json"
	"flag"
	"io"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"github.com/lib/pq"
)

// RunCheckinHistoryCLI uses only a private offline extension owner connection.
// Normal service startup cannot install the addon or import historical records.
func RunCheckinHistoryCLI(parent context.Context, args []string, out io.Writer) error {
	var c Config
	var input, expected, operation string
	var recoverOnly bool
	f := flag.NewFlagSet("import-checkin-history", flag.ContinueOnError)
	f.SetOutput(io.Discard)
	f.StringVar(&c.OwnerURLFile, "owner-url-file", "", "private offline owner connection")
	f.StringVar(&c.ExpectedDatabase, "database", "", "exact extension database")
	f.StringVar(&c.OwnerRole, "owner-role", "", "NOLOGIN database owner")
	f.StringVar(&c.RuntimeRole, "runtime-role", "", "fenced NOLOGIN runtime")
	f.StringVar(&input, "input-file", "", "private complete history and committed owner mapping")
	f.StringVar(&expected, "input-sha256", "", "reviewed exact input bytes")
	f.StringVar(&operation, "operation-id", "", "original stable operation ID")
	f.BoolVar(&recoverOnly, "recover", false, "read the original committed result without applying")
	if f.Parse(args) != nil || f.NArg() != 0 {
		return ErrRejected
	}
	raw, e := coreclient.ReadPrivateFile(input, 16*1024*1024)
	if e != nil {
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
	ctx, cancel := context.WithTimeout(parent, 30*time.Second)
	defer cancel()
	tx, e := db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if e != nil {
		return ErrRejected
	}
	defer tx.Rollback()
	if _, e = tx.ExecContext(ctx, `SET LOCAL lock_timeout='2s'; SET LOCAL statement_timeout='20s'`); e != nil {
		return ErrRejected
	}
	var allowed bool
	e = tx.QueryRowContext(ctx, `SELECT current_database()=$1 AND pg_get_userbyid(d.datdba)=$2 AND session_user<>$3
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$2 AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,$2,$3) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 FROM pg_database d WHERE datname=current_database()`, c.ExpectedDatabase, c.OwnerRole, c.RuntimeRole).Scan(&allowed)
	if e != nil || !allowed {
		return ErrRejected
	}
	if _, e = tx.ExecContext(ctx, "SET LOCAL ROLE "+pq.QuoteIdentifier(c.OwnerRole)); e != nil {
		return ErrRejected
	}
	receipt, e := checkins.ImportLegacyHistory(ctx, tx, c.RuntimeRole, operation, raw, expected, recoverOnly)
	if e != nil {
		return e
	}
	if !recoverOnly && tx.Commit() != nil {
		return ErrCommitUnknown
	}
	return json.NewEncoder(out).Encode(map[string]any{"database": c.ExpectedDatabase, "history_version": 1, "receipt": receipt, "recovered": recoverOnly, "balance_effect": "none"})
}
