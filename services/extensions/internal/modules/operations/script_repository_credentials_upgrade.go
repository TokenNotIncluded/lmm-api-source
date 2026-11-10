// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"

	"github.com/lib/pq"
)

// UpgradeScriptRepositoryCredentials installs the v1 credential extension only
// under an explicit offline owner transaction. Base operations stays v3; its
// catalog fingerprint binds the new column/immutable upgrade receipt as well.
func UpgradeScriptRepositoryCredentials(ctx context.Context, tx *sql.Tx, runtimeRole, operation string, recoverOnly bool) error {
	if !upgradeOperation.MatchString(operation) || runtimeRole == "" {
		return ErrInvalid
	}
	var allowed bool
	err := tx.QueryRowContext(ctx, `SELECT current_user=pg_get_userbyid(d.datdba)
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND NOT rolcanlogin AND NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid())
 AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database())
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname='lmm_operations' AND nspowner<>d.datdba)
 FROM pg_database d WHERE datname=current_database()`, runtimeRole).Scan(&allowed)
	if err != nil || !allowed {
		return ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `LOCK TABLE lmm_operations.contract,lmm_operations.script_repository IN ACCESS EXCLUSIVE MODE`); err != nil {
		return ErrUnavailable
	}
	var installed, actual string
	if tx.QueryRowContext(ctx, `SELECT fingerprint FROM lmm_operations.contract WHERE version=3`).Scan(&installed) != nil || tx.QueryRowContext(ctx, fingerprintSQL).Scan(&actual) != nil || actual != installed || len(actual) != 32 {
		return ErrUnavailable
	}
	var exists bool
	if tx.QueryRowContext(ctx, `SELECT to_regclass('lmm_operations.script_repository_credentials_upgrades') IS NOT NULL`).Scan(&exists) != nil {
		return ErrUnavailable
	}
	if exists {
		var same bool
		if tx.QueryRowContext(ctx, `SELECT version=1 AND operation_id=$1 AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND installed_by=session_user FROM lmm_operations.script_repository_credentials_upgrades`, operation).Scan(&same) != nil || !same {
			return ErrConflict
		}
		return nil
	}
	if recoverOnly {
		return ErrNotFound
	}
	if _, err = tx.ExecContext(ctx, ScriptRepositoryCredentialsSchema); err != nil {
		return ErrUnavailable
	}
	// Neutralize owner default ACLs before sealing the dedicated receipt.
	rows, err := tx.QueryContext(ctx, `SELECT a.grantee,coalesce(r.rolname,'') FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a LEFT JOIN pg_roles r ON r.oid=a.grantee WHERE c.oid='lmm_operations.script_repository_credentials_upgrades'::regclass AND a.grantee<>c.relowner`)
	if err != nil {
		return ErrUnavailable
	}
	var grantees []string
	for rows.Next() {
		var id int64
		var role string
		if rows.Scan(&id, &role) != nil {
			rows.Close()
			return ErrUnavailable
		}
		if id == 0 {
			grantees = append(grantees, "PUBLIC")
		} else {
			grantees = append(grantees, pq.QuoteIdentifier(role))
		}
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return ErrUnavailable
	}
	for _, role := range grantees {
		if _, err = tx.ExecContext(ctx, `REVOKE ALL ON lmm_operations.script_repository_credentials_upgrades FROM `+role); err != nil {
			return ErrUnavailable
		}
	}
	if _, err = tx.ExecContext(ctx, `UPDATE lmm_operations.script_repository_credentials_upgrades SET operation_id=$1`, operation); err != nil {
		return ErrUnavailable
	}
	return SealInstalledSchema(ctx, tx)
}
