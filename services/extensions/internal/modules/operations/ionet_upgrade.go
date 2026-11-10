// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	"github.com/lib/pq"
)

// Explicit offline owner upgrade; normal startup only reads installed objects.
func UpgradeIoNet(ctx context.Context, tx *sql.Tx, runtimeRole, operation string, recoverOnly bool) error {
	if !upgradeOperation.MatchString(operation) || runtimeRole == "" {
		return ErrInvalid
	}
	var ready bool
	if tx.QueryRowContext(ctx, `SELECT current_user=pg_get_userbyid(d.datdba)
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND NOT rolcanlogin AND NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid())
 AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database())
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname='lmm_operations' AND nspowner<>d.datdba)
 FROM pg_database d WHERE datname=current_database()`, runtimeRole).Scan(&ready) != nil || !ready {
		return ErrUnavailable
	}
	if _, err := tx.ExecContext(ctx, `LOCK TABLE lmm_operations.contract IN ACCESS EXCLUSIVE MODE`); err != nil {
		return ErrUnavailable
	}
	var installed, actual string
	if tx.QueryRowContext(ctx, `SELECT fingerprint FROM lmm_operations.contract WHERE version=3`).Scan(&installed) != nil || tx.QueryRowContext(ctx, fingerprintSQL).Scan(&actual) != nil || installed != actual || len(actual) != 32 {
		return ErrUnavailable
	}
	var exists bool
	if tx.QueryRowContext(ctx, `SELECT to_regclass('lmm_operations.ionet_upgrades') IS NOT NULL`).Scan(&exists) != nil {
		return ErrUnavailable
	}
	if exists {
		var same bool
		if tx.QueryRowContext(ctx, `SELECT version=1 AND operation_id=$1 AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND installed_by=session_user FROM lmm_operations.ionet_upgrades`, operation).Scan(&same) != nil || !same {
			return ErrConflict
		}
		return nil
	}
	if recoverOnly {
		return ErrNotFound
	}
	if _, err := tx.ExecContext(ctx, IoNetSchema); err != nil {
		return ErrUnavailable
	}
	for _, table := range []string{"ionet_config", "ionet_operations", "ionet_upgrades"} {
		rows, err := tx.QueryContext(ctx, `SELECT a.grantee,coalesce(r.rolname,'') FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a LEFT JOIN pg_roles r ON r.oid=a.grantee WHERE c.oid=$1::regclass AND a.grantee<>c.relowner`, "lmm_operations."+table)
		if err != nil {
			return ErrUnavailable
		}
		var names []string
		for rows.Next() {
			var id int64
			var name string
			if rows.Scan(&id, &name) != nil {
				rows.Close()
				return ErrUnavailable
			}
			if id == 0 {
				names = append(names, "PUBLIC")
			} else {
				names = append(names, pq.QuoteIdentifier(name))
			}
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return ErrUnavailable
		}
		for _, name := range names {
			if _, err := tx.ExecContext(ctx, `REVOKE ALL ON lmm_operations.`+pq.QuoteIdentifier(table)+` FROM `+name); err != nil {
				return ErrUnavailable
			}
		}
	}
	if _, err := tx.ExecContext(ctx, `GRANT SELECT,UPDATE ON lmm_operations.ionet_config TO `+pq.QuoteIdentifier(runtimeRole)+`;GRANT SELECT,INSERT,UPDATE ON lmm_operations.ionet_operations TO `+pq.QuoteIdentifier(runtimeRole)); err != nil {
		return ErrUnavailable
	}
	if _, err := tx.ExecContext(ctx, `UPDATE lmm_operations.ionet_upgrades SET operation_id=$1`, operation); err != nil {
		return ErrUnavailable
	}
	return SealInstalledSchema(ctx, tx)
}
