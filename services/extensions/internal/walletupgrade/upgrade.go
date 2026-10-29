// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Package walletupgrade owns one explicit offline schema change. It cannot
// import bearers, write balances, or accept operator-supplied SQL or digests.
package walletupgrade

import (
	"context"
	"database/sql"
	"errors"
	"net/url"
	"regexp"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/postgresurl"
	"github.com/lib/pq"
)

const upgradeID = "wallet-request-key-bytes-v1"

var (
	ErrRejected = errors.New("offline wallet schema upgrade rejected")
	ErrUnknown  = errors.New("wallet schema commit outcome unknown; maintain the external writer fence and recover the original operation ID")
	ErrAbsent   = errors.New("original wallet schema operation has no commit receipt; no migration state was changed")
	name        = regexp.MustCompile(`^[a-z][a-z0-9_]{0,62}$`)
	operation   = regexp.MustCompile(`^[a-zA-Z0-9_-]{16,64}$`)
)

type Config struct {
	OwnerURLFile, Database, OwnerRole, RuntimeRole, OperationID string
}

type Receipt struct {
	Database    string `json:"database"`
	OperationID string `json:"operation_id"`
	Upgrade     string `json:"upgrade"`
}

// These are compiled reference tables, never executable input from a file.
// PostgreSQL itself builds their catalog definitions for this server version.
const referenceLinks = `CREATE TEMP TABLE transfer_links (
 owner_user bigint NOT NULL,
 request_key text NOT NULL CHECK(length(request_key) BETWEEN 16 AND 64),
 amount_credits bigint NOT NULL CHECK(amount_credits>0),
 token text NOT NULL UNIQUE CHECK(length(token)=64),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(owner_user,request_key)
) ON COMMIT DROP;`

const journalTable = `CREATE TABLE extension_wallet.schema_upgrades (
 upgrade_id text PRIMARY KEY CHECK(upgrade_id='wallet-request-key-bytes-v1'),
 operation_id text NOT NULL UNIQUE,
 database_oid oid NOT NULL,
 installed_by name NOT NULL,
 applied_at timestamptz NOT NULL DEFAULT clock_timestamp()
);`

func ownerURL(c Config) (string, error) {
	if !name.MatchString(c.Database) || (c.Database != "lmm_toolmarket" && !strings.HasPrefix(c.Database, "lmm_toolmarket_")) ||
		!name.MatchString(c.OwnerRole) || !name.MatchString(c.RuntimeRole) || c.OwnerRole == c.RuntimeRole || !operation.MatchString(c.OperationID) {
		return "", ErrRejected
	}
	raw, err := coreclient.ReadPrivateFile(c.OwnerURLFile, 8192)
	if err != nil {
		return "", ErrRejected
	}
	dsn := strings.TrimSpace(string(raw))
	u, err := url.Parse(dsn)
	if err != nil || u.Fragment != "" || strings.ContainsAny(dsn, "\r\n\x00") {
		return "", ErrRejected
	}
	unix, err := postgresurl.Unix(u)
	if err != nil || !unix || u.Path != "/"+c.Database {
		return "", ErrRejected
	}
	return dsn, nil
}

// Apply changes only the request-key check and creates an owner-only receipt.
// Operators must establish and maintain the site-wide writer fence externally;
// the database preflight and table lock cover this transaction only.
// Recover never creates a journal, changes a constraint, or retries an apply.
// Both serialize on the same database lock before looking up the original ID.
func Apply(ctx context.Context, c Config) (Receipt, error)   { return run(ctx, c, false) }
func Recover(ctx context.Context, c Config) (Receipt, error) { return run(ctx, c, true) }

func run(ctx context.Context, c Config, recoverOnly bool) (Receipt, error) {
	dsn, err := ownerURL(c)
	if err != nil {
		return Receipt{}, err
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		return Receipt{}, ErrRejected
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	// READ COMMITTED takes the receipt lookup snapshot after the advisory lock:
	// a waiting recovery must observe the preceding transaction's commit.
	tx, err := db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return Receipt{}, ErrRejected
	}
	defer tx.Rollback()
	reject := func() (Receipt, error) { return Receipt{}, ErrRejected }
	if _, err = tx.ExecContext(ctx, `SET LOCAL search_path=pg_catalog,pg_temp;
SET LOCAL lock_timeout='2s'; SET LOCAL statement_timeout='10s'`); err != nil {
		return reject()
	}
	if !authority(ctx, tx, c) {
		return reject()
	}
	if _, err = tx.ExecContext(ctx, "SET LOCAL ROLE "+pq.QuoteIdentifier(c.OwnerRole)); err != nil {
		return reject()
	}
	if _, err = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended(current_database()||':wallet-request-key-bytes-v1',0))`); err != nil {
		return reject()
	}
	if _, err = tx.ExecContext(ctx, `LOCK TABLE extension_wallet.transfer_links IN ACCESS EXCLUSIVE MODE`); err != nil {
		return reject()
	}
	if !storageBoundary(ctx, tx, c) {
		return reject()
	}
	if _, err = tx.ExecContext(ctx, referenceLinks); err != nil {
		return reject()
	}
	before, err := signature(ctx, tx, "pg_temp.transfer_links")
	if err != nil {
		return reject()
	}
	if _, err = tx.ExecContext(ctx, `ALTER TABLE pg_temp.transfer_links DROP CONSTRAINT transfer_links_request_key_check;
ALTER TABLE pg_temp.transfer_links ADD CONSTRAINT transfer_links_request_key_check CHECK(octet_length(request_key) BETWEEN 16 AND 64)`); err != nil {
		return reject()
	}
	after, err := signature(ctx, tx, "pg_temp.transfer_links")
	if err != nil {
		return reject()
	}
	actual, err := signature(ctx, tx, "extension_wallet.transfer_links")
	if err != nil || (actual != before && actual != after) {
		return reject()
	}
	var hasJournal bool
	if err = tx.QueryRowContext(ctx, `SELECT to_regclass('extension_wallet.schema_upgrades') IS NOT NULL`).Scan(&hasJournal); err != nil {
		return reject()
	}
	if hasJournal {
		// Compare the journal to a compiled reference before reading its rows.
		ref := strings.Replace(journalTable, "CREATE TABLE extension_wallet.", "CREATE TEMP TABLE ", 1)
		if _, err = tx.ExecContext(ctx, ref); err != nil {
			return reject()
		}
		want, e := signature(ctx, tx, "pg_temp.schema_upgrades")
		got, e2 := signature(ctx, tx, "extension_wallet.schema_upgrades")
		var private bool
		e3 := tx.QueryRowContext(ctx, `SELECT c.relowner=(SELECT oid FROM pg_roles WHERE rolname=$1)
 AND c.relpersistence='p'
 AND NOT EXISTS(SELECT FROM aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a WHERE a.grantee<>c.relowner)
 AND NOT EXISTS(SELECT FROM pg_attribute WHERE attrelid=c.oid AND attacl IS NOT NULL)
 FROM pg_class c WHERE c.oid='extension_wallet.schema_upgrades'::regclass`, c.OwnerRole).Scan(&private)
		if e != nil || e2 != nil || e3 != nil || want != got || !private {
			return reject()
		}
		var same bool
		err = tx.QueryRowContext(ctx, `SELECT upgrade_id=$1 AND operation_id=$2
 AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database())
 AND installed_by=session_user FROM extension_wallet.schema_upgrades`, upgradeID, c.OperationID).Scan(&same)
		if err != nil || !same || actual != after {
			return reject()
		}
		// Replay/recovery is observational; rollback also removes temp tables.
		return Receipt{c.Database, c.OperationID, upgradeID}, nil
	}
	if recoverOnly {
		return Receipt{}, ErrAbsent
	}
	// A byte-shaped table without our receipt is not evidence this operation
	// ran, including databases created by the new fresh installer.
	if actual != before {
		return reject()
	}
	if _, err = tx.ExecContext(ctx, `ALTER TABLE extension_wallet.transfer_links DROP CONSTRAINT transfer_links_request_key_check;
ALTER TABLE extension_wallet.transfer_links ADD CONSTRAINT transfer_links_request_key_check CHECK(octet_length(request_key) BETWEEN 16 AND 64)`); err != nil {
		return reject()
	}
	if got, e := signature(ctx, tx, "extension_wallet.transfer_links"); e != nil || got != after {
		return reject()
	}
	if _, err = tx.ExecContext(ctx, journalTable); err != nil {
		return reject()
	}
	// Installation-owner default ACLs must not expose receipts to runtime or
	// PUBLIC. Revoke every actual non-owner grantee, not just the named runtime.
	rows, err := tx.QueryContext(ctx, `SELECT DISTINCT a.grantee,coalesce(r.rolname,'')
 FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a
 LEFT JOIN pg_roles r ON r.oid=a.grantee
 WHERE c.oid='extension_wallet.schema_upgrades'::regclass AND a.grantee<>c.relowner`)
	if err != nil {
		return reject()
	}
	var grantees []string
	for rows.Next() {
		var oid int64
		var role string
		if err = rows.Scan(&oid, &role); err != nil {
			rows.Close()
			return reject()
		}
		if oid == 0 {
			grantees = append(grantees, "PUBLIC")
		} else {
			grantees = append(grantees, pq.QuoteIdentifier(role))
		}
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return reject()
	}
	for _, role := range grantees {
		if _, err = tx.ExecContext(ctx, "REVOKE ALL ON extension_wallet.schema_upgrades FROM "+role); err != nil {
			return reject()
		}
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO extension_wallet.schema_upgrades(upgrade_id,operation_id,database_oid,installed_by)
 SELECT $1,$2,oid,session_user FROM pg_database WHERE datname=current_database()`, upgradeID, c.OperationID); err != nil {
		return reject()
	}
	if err = tx.Commit(); err != nil {
		return Receipt{}, ErrUnknown
	}
	return Receipt{c.Database, c.OperationID, upgradeID}, nil
}

func authority(ctx context.Context, tx *sql.Tx, c Config) bool {
	var allowed bool
	err := tx.QueryRowContext(ctx, `SELECT current_database()=$1
 AND pg_get_userbyid(d.datdba)=$2 AND session_user<>$3
 AND current_setting('server_encoding')='UTF8'
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,$2,$3)
   AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$2 AND NOT rolcanlogin)
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$3 AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname<>$3
   AND (pg_has_role($3,oid,'MEMBER') OR pg_has_role($3,oid,'SET') OR pg_has_role($3,oid,'USAGE')))
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
   AND (has_database_privilege(oid,d.oid,'CONNECT') OR pg_has_role(oid,$2,'SET') OR pg_has_role(oid,$3,'SET')))
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid())
 AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database())
 FROM pg_database d WHERE datname=current_database()`, c.Database, c.OwnerRole, c.RuntimeRole).Scan(&allowed)
	return err == nil && allowed
}

func storageBoundary(ctx context.Context, tx *sql.Tx, c Config) bool {
	var allowed bool
	err := tx.QueryRowContext(ctx, `SELECT n.nspowner=(SELECT oid FROM pg_roles WHERE rolname=$1)
 AND c.relowner=n.nspowner AND c.relpersistence='p'
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\')
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname NOT IN ('public','information_schema','extension_wallet','lmm_toolmarket','lmm_bounties')
   AND nspname NOT LIKE 'pg\_%' ESCAPE '\')
 AND NOT has_database_privilege($2,current_database(),'CREATE,TEMP')
 AND NOT has_schema_privilege($2,n.oid,'CREATE')
 AND NOT EXISTS(SELECT FROM aclexplode(coalesce(n.nspacl,acldefault('n',n.nspowner))) WHERE grantee=0)
 AND NOT EXISTS(SELECT FROM aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) WHERE grantee=0)
 AND NOT EXISTS(SELECT FROM pg_attribute a CROSS JOIN LATERAL aclexplode(a.attacl) acl WHERE a.attrelid=c.oid AND acl.grantee=0)
 AND has_table_privilege($2,c.oid,'SELECT') AND has_table_privilege($2,c.oid,'INSERT')
 AND NOT has_table_privilege($2,c.oid,'UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')
 AND NOT has_any_column_privilege($2,c.oid,'UPDATE,REFERENCES')
 AND NOT EXISTS(SELECT FROM pg_attribute a JOIN pg_collation co ON co.oid=a.attcollation WHERE a.attrelid=c.oid AND NOT co.collisdeterministic)
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.oid='extension_wallet.transfer_links'::regclass`, c.OwnerRole, c.RuntimeRole).Scan(&allowed)
	return err == nil && allowed
}

// Excludes names/OIDs that differ for temporary reference tables; compares
// columns, defaults, constraints, index semantics, triggers, RLS and inheritance.
func signature(ctx context.Context, tx *sql.Tx, table string) (string, error) {
	var result string
	err := tx.QueryRowContext(ctx, `SELECT jsonb_build_object(
 'kind',c.relkind::text,'rls',c.relrowsecurity,'force_rls',c.relforcerowsecurity,'replica',c.relreplident::text,
 'options',c.reloptions,'access',c.relam,
 'columns',(SELECT jsonb_agg(jsonb_build_array(a.attname,a.atttypid,a.atttypmod,a.attnotnull,
   a.attidentity::text,a.attgenerated::text,a.attcollation,pg_get_expr(d.adbin,d.adrelid)) ORDER BY a.attnum)
   FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
   WHERE a.attrelid=c.oid AND a.attnum>0),
 'constraints',(SELECT jsonb_agg(jsonb_build_array(conname,contype::text,condeferrable,condeferred,
   convalidated,connoinherit,pg_get_constraintdef(oid)) ORDER BY conname) FROM pg_constraint WHERE conrelid=c.oid),
 'indexes',(SELECT jsonb_agg(jsonb_build_array(i.indisunique,i.indisprimary,i.indisexclusion,i.indimmediate,
   i.indisvalid,i.indisready,i.indislive,i.indnkeyatts,i.indnatts,i.indkey::text,i.indclass::text,
   i.indcollation::text,i.indoption::text,pg_get_expr(i.indexprs,i.indrelid),pg_get_expr(i.indpred,i.indrelid),
   x.relam,x.reloptions,i.indnullsnotdistinct) ORDER BY x.relname)
   FROM pg_index i JOIN pg_class x ON x.oid=i.indexrelid WHERE i.indrelid=c.oid),
 'triggers',(SELECT count(*) FROM pg_trigger WHERE tgrelid=c.oid),
 'rules',(SELECT count(*) FROM pg_rewrite WHERE ev_class=c.oid),
 'inheritance',(SELECT count(*) FROM pg_inherits WHERE inhrelid=c.oid OR inhparent=c.oid)
 )::text FROM pg_class c WHERE c.oid=to_regclass($1)`, table).Scan(&result)
	return result, err
}
