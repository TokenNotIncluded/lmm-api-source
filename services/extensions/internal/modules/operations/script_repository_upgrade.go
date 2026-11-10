// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"

	"github.com/lib/pq"
)

// UpgradeScriptRepository runs only inside an explicit offline owner transaction. The caller
// must keep writers fenced until the original operation's outcome is read back.
// A runtime session can neither alter the schema nor write its receipt.
func UpgradeScriptRepository(ctx context.Context, tx *sql.Tx, runtimeRole, operation string, recoverOnly bool) error {
	if !upgradeOperation.MatchString(operation) || runtimeRole == "" {
		return ErrInvalid
	}
	var allowed bool
	err := tx.QueryRowContext(ctx, `SELECT current_user=pg_get_userbyid(d.datdba)
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND NOT rolcanlogin AND NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid())
 AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database())
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname='lmm_operations' AND nspowner<>d.datdba)
 FROM pg_database d WHERE datname=current_database()`, runtimeRole).Scan(&allowed)
	if err != nil || !allowed {
		return ErrUnavailable
	}
	if _, err := tx.ExecContext(ctx, `LOCK TABLE lmm_operations.contract,lmm_operations.publications,lmm_operations.reads,lmm_operations.rss_config IN ACCESS EXCLUSIVE MODE`); err != nil {
		return ErrUnavailable
	}
	var version int
	var installed, actual string
	if err := tx.QueryRowContext(ctx, `SELECT version,fingerprint FROM lmm_operations.contract`).Scan(&version, &installed); err != nil {
		return ErrUnavailable
	}
	if err := tx.QueryRowContext(ctx, fingerprintSQL).Scan(&actual); err != nil || actual != installed || len(actual) != 32 {
		return ErrUnavailable
	}
	if version == 3 {
		var same bool
		err := tx.QueryRowContext(ctx, `SELECT operation_id=$1 AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND installed_by=session_user FROM lmm_operations.script_repository_upgrades WHERE version=3`, operation).Scan(&same)
		if err != nil || !same {
			return ErrConflict
		}
		return nil
	}
	if version != 2 {
		return ErrUnavailable
	}
	if recoverOnly {
		return ErrNotFound
	}
	_, err = tx.ExecContext(ctx, scriptRepositorySchema+`
CREATE TABLE lmm_operations.script_repository_upgrades (
 version integer PRIMARY KEY CHECK (version=3),
 operation_id text NOT NULL,
 database_oid oid NOT NULL,
 installed_by name NOT NULL
);
ALTER TABLE lmm_operations.contract DROP CONSTRAINT contract_version_check;
UPDATE lmm_operations.contract SET version=3;
ALTER TABLE lmm_operations.contract ADD CONSTRAINT contract_version_check CHECK (version=3);`)
	if err != nil {
		return ErrUnavailable
	}
	// New objects inherit owner defaults. Remove every non-owner grant before
	// applying the runtime permissions required by repository settings.
	rows, err := tx.QueryContext(ctx, `SELECT DISTINCT c.relname,a.grantee,coalesce(r.rolname,'') FROM pg_class c
 CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a
 LEFT JOIN pg_roles r ON r.oid=a.grantee
 WHERE c.oid IN ('lmm_operations.script_repository'::regclass,'lmm_operations.script_repository_upgrades'::regclass) AND a.grantee<>c.relowner`)
	if err != nil {
		return ErrUnavailable
	}
	var revoke []string
	for rows.Next() {
		var table, role string
		var id int64
		if err := rows.Scan(&table, &id, &role); err != nil {
			rows.Close()
			return ErrUnavailable
		}
		grantee := pq.QuoteIdentifier(role)
		if id == 0 {
			grantee = "PUBLIC"
		}
		revoke = append(revoke, "REVOKE ALL ON lmm_operations."+pq.QuoteIdentifier(table)+" FROM "+grantee)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return ErrUnavailable
	}
	for _, statement := range revoke {
		if _, err := tx.ExecContext(ctx, statement); err != nil {
			return ErrUnavailable
		}
	}
	if _, err := tx.ExecContext(ctx, "GRANT SELECT,UPDATE ON lmm_operations.script_repository TO "+pq.QuoteIdentifier(runtimeRole)); err != nil {
		return ErrUnavailable
	}
	if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_operations.script_repository_upgrades(version,operation_id,database_oid,installed_by) SELECT 3,$1,oid,session_user FROM pg_database WHERE datname=current_database()`, operation); err != nil {
		return ErrUnavailable
	}
	return SealInstalledSchema(ctx, tx)
}
