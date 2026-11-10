// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package freshinstall

import (
	"context"
	"database/sql"
	"encoding/json"
	"flag"
	"io"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
	"github.com/lib/pq"
)

// RunOperationsUpgradeCLI performs an explicit owner upgrade to the selected version.
// Normal startup never reads owner credentials or invokes this code.
func RunOperationsUpgradeCLI(ctx context.Context, args []string, out io.Writer) error {
	return runOperationsUpgradeCLI(ctx, args, out, upgradeOperationsBase)
}

func RunScriptRepositoryCredentialsUpgradeCLI(ctx context.Context, args []string, out io.Writer) error {
	return runOperationsUpgradeCLI(ctx, args, out, upgradeScriptCredentials)
}

type operationsUpgradeMode uint8

const (
	upgradeOperationsBase operationsUpgradeMode = iota
	upgradeScriptCredentials
	upgradeIoNet
)

func RunIoNetUpgradeCLI(ctx context.Context, args []string, out io.Writer) error {
	return runOperationsUpgradeCLI(ctx, args, out, upgradeIoNet)
}
func runOperationsUpgradeCLI(ctx context.Context, args []string, out io.Writer, mode operationsUpgradeMode) error {
	credentials := mode == upgradeScriptCredentials
	ionet := mode == upgradeIoNet
	var c Config
	var operation string
	var recoverOnly bool
	var targetVersion int
	command := "upgrade-operations-db"
	if ionet {
		command = "upgrade-ionet"
	}
	if credentials {
		command = "upgrade-script-repository-credentials"
	}
	f := flag.NewFlagSet(command, flag.ContinueOnError)
	f.SetOutput(io.Discard)
	f.StringVar(&c.OwnerURLFile, "owner-url-file", "", "private offline owner connection")
	f.StringVar(&c.ExpectedDatabase, "database", "", "exact existing operations database")
	f.StringVar(&c.OwnerRole, "owner-role", "", "NOLOGIN database owner")
	f.StringVar(&c.RuntimeRole, "runtime-role", "", "fenced NOLOGIN runtime")
	f.StringVar(&operation, "operation-id", "", "original stable operation ID")
	f.BoolVar(&recoverOnly, "recover", false, "read original result without applying")
	if !credentials && !ionet {
		f.IntVar(&targetVersion, "target-version", 2, "2 for RSS, 3 for script repository")
	}
	if f.Parse(args) != nil || f.NArg() != 0 || (!credentials && !ionet && targetVersion != 2 && targetVersion != 3) {
		return ErrRejected
	}
	c.Component = "operations"
	p, _ := compiled(c.Component)
	dsn, err := ownerURL(c, p)
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
	if _, err := tx.ExecContext(ctx, `SET LOCAL lock_timeout='2s'; SET LOCAL statement_timeout='10s'`); err != nil {
		return ErrRejected
	}
	var allowed bool
	err = tx.QueryRowContext(ctx, `SELECT current_database()=$1 AND pg_get_userbyid(d.datdba)=$2 AND session_user<>$3
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$2 AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,$2,$3) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_auth_members WHERE member=(SELECT oid FROM pg_roles WHERE rolname=$3))
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
   AND (has_database_privilege(oid,d.oid,'CONNECT') OR pg_has_role(oid,$2,'SET') OR pg_has_role(oid,$3,'SET')))
 FROM pg_database d WHERE datname=current_database()`, c.ExpectedDatabase, c.OwnerRole, c.RuntimeRole).Scan(&allowed)
	if err != nil || !allowed {
		return ErrRejected
	}
	if _, err := tx.ExecContext(ctx, "SET LOCAL ROLE "+pq.QuoteIdentifier(c.OwnerRole)); err != nil {
		return ErrRejected
	}
	if ionet {
		err = operations.UpgradeIoNet(ctx, tx, c.RuntimeRole, operation, recoverOnly)
	} else if credentials {
		err = operations.UpgradeScriptRepositoryCredentials(ctx, tx, c.RuntimeRole, operation, recoverOnly)
	} else if targetVersion == 3 {
		err = operations.UpgradeScriptRepository(ctx, tx, c.RuntimeRole, operation, recoverOnly)
	} else {
		err = operations.UpgradeRSS(ctx, tx, c.RuntimeRole, operation, recoverOnly)
	}
	if err != nil {
		return err
	}
	if !recoverOnly {
		if err := tx.Commit(); err != nil {
			return ErrCommitUnknown
		}
	}
	receipt := map[string]any{"database": c.ExpectedDatabase, "operation_id": operation, "version": targetVersion}
	if ionet {
		receipt["version"] = 1
		receipt["component"] = "ionet"
	}
	if credentials {
		receipt["version"] = 1
		receipt["component"] = "script_repository_credentials"
	}
	if err := json.NewEncoder(out).Encode(receipt); err != nil {
		return ErrCommitUnknown
	}
	return nil
}
