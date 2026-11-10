// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"testing"

	"github.com/lib/pq"
)

func TestIoNetPostgresOfflineUpgradeRollbackAndOriginalReceipt(t *testing.T) {
	dsn := os.Getenv("IONET_UPGRADE_TEST_OWNER_DSN")
	if dsn == "" {
		t.Skip("requires fresh isolated upgrade PostgreSQL fixture")
	}
	ctx := context.Background()
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	// Seed the unchanged pre-addon operations v3 contract. This is not a
	// runtime downgrade: the test database is empty and owned by the fixture.
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := tx.Exec(baseSchema + "\n" + scriptRepositorySchema); err != nil {
		tx.Rollback()
		t.Fatal(err)
	}
	if err := SealInstalledSchema(ctx, tx); err != nil {
		tx.Rollback()
		t.Fatal(err)
	}
	if err := tx.Commit(); err != nil {
		t.Fatal(err)
	}
	const operation = "original_ionet_upgrade_0001"
	run := func(key string, recover, commit bool) error {
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			return err
		}
		defer tx.Rollback()
		if err = UpgradeIoNet(ctx, tx, "ionet_upgrade_runtime", key, recover); err != nil {
			return err
		}
		if commit {
			return tx.Commit()
		}
		return nil
	}
	if err := run(operation, false, true); !errors.Is(err, ErrUnavailable) {
		t.Fatal("LOGIN fence", err)
	}
	if _, err := db.Exec(`ALTER ROLE ionet_upgrade_runtime NOLOGIN`); err != nil {
		t.Fatal(err)
	}
	var database string
	if err := db.QueryRow(`SELECT current_database()`).Scan(&database); err != nil {
		t.Fatal(err)
	}
	for _, privilege := range []string{"CREATE", "TEMPORARY"} {
		if _, err := db.Exec("GRANT " + privilege + " ON DATABASE " + pq.QuoteIdentifier(database) + " TO ionet_upgrade_runtime"); err != nil {
			t.Fatal(err)
		}
		if err := run(operation, false, true); !errors.Is(err, ErrUnavailable) {
			t.Fatal("runtime database DDL privilege accepted", privilege, err)
		}
		var unchanged bool
		if err := db.QueryRow(`SELECT to_regclass('lmm_operations.ionet_upgrades') IS NULL AND to_regclass('lmm_operations.ionet_config') IS NULL`).Scan(&unchanged); err != nil || !unchanged {
			t.Fatal("rejected upgrade changed schema", err)
		}
		if _, err := db.Exec("REVOKE " + privilege + " ON DATABASE " + pq.QuoteIdentifier(database) + " FROM ionet_upgrade_runtime"); err != nil {
			t.Fatal(err)
		}
	}
	if err := run(operation, true, true); !errors.Is(err, ErrNotFound) {
		t.Fatal("missing recovery dispatched", err)
	}
	if err := run(operation, false, false); err != nil {
		t.Fatal("rolled back install", err)
	}
	if err := run(operation, true, true); !errors.Is(err, ErrNotFound) {
		t.Fatal("rolled back receipt survived", err)
	}
	if err := run(operation, false, true); err != nil {
		t.Fatal("install", err)
	}
	if err := run(operation, true, true); err != nil {
		t.Fatal("original receipt", err)
	}
	if err := run("different_ionet_upgrade_0002", true, true); !errors.Is(err, ErrConflict) {
		t.Fatal("foreign receipt", err)
	}
	var safe bool
	err = db.QueryRow(`SELECT (SELECT revision=0 AND NOT enabled AND api_key_ciphertext='' FROM lmm_operations.ionet_config) AND NOT has_table_privilege('ionet_upgrade_runtime','lmm_operations.ionet_upgrades','SELECT,INSERT,UPDATE,DELETE') AND has_table_privilege('ionet_upgrade_runtime','lmm_operations.ionet_config','SELECT') AND has_table_privilege('ionet_upgrade_runtime','lmm_operations.ionet_config','UPDATE') AND NOT has_table_privilege('ionet_upgrade_runtime','lmm_operations.ionet_operations','DELETE')`).Scan(&safe)
	if err != nil || !safe {
		t.Fatal("receipt/runtime ACL", err)
	}
}
