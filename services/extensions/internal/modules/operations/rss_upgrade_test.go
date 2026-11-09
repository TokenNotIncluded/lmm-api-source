// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	_ "embed"
	"errors"
	"os"
	"testing"
)

// Frozen v1 at 33ba2e186a80fda2587778471e639cdf7416087f.
//
//go:embed testdata/schema_v1.sql
var rssPreviousSchema string

func TestRSSPostgresOfflineUpgradeAndOriginalRecovery(t *testing.T) {
	dsn := os.Getenv("RSS_UPGRADE_TEST_OWNER_DSN")
	if dsn == "" {
		t.Skip("requires fresh isolated operations v1 upgrade fixture")
	}
	ctx := context.Background()
	owner, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	for _, query := range []string{rssPreviousSchema,
		`REVOKE ALL ON DATABASE lmm_operations_upgrade_test FROM PUBLIC; REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT CONNECT ON DATABASE lmm_operations_upgrade_test TO rss_upgrade_runtime;
GRANT USAGE ON SCHEMA lmm_operations TO rss_upgrade_runtime;
GRANT SELECT ON lmm_operations.contract TO rss_upgrade_runtime;
GRANT SELECT,INSERT ON lmm_operations.publications,lmm_operations.reads TO rss_upgrade_runtime;
GRANT SELECT,USAGE ON SEQUENCE lmm_operations.publication_id TO rss_upgrade_runtime;
INSERT INTO lmm_operations.publications(version,revision,content,published_at,published_by) VALUES('retained',1,'existing release',1,501);
INSERT INTO lmm_operations.reads VALUES(502,1,1);`} {
		if _, err := owner.ExecContext(ctx, query); err != nil {
			t.Fatal(err)
		}
	}
	var fingerprint string
	if err := owner.QueryRowContext(ctx, fingerprintSQL).Scan(&fingerprint); err != nil {
		t.Fatal(err)
	}
	if _, err := owner.ExecContext(ctx, `UPDATE lmm_operations.contract SET fingerprint=$1`, fingerprint); err != nil {
		t.Fatal(err)
	}
	const operation = "rss_upgrade_original_001"
	run := func(id string, recover bool) error {
		tx, err := owner.BeginTx(ctx, nil)
		if err != nil {
			return err
		}
		defer tx.Rollback()
		if err := UpgradeRSS(ctx, tx, "rss_upgrade_runtime", id, recover); err != nil {
			return err
		}
		if recover {
			return nil
		}
		return tx.Commit()
	}
	if err := run(operation, true); !errors.Is(err, ErrNotFound) {
		t.Fatal("recovery before upgrade", err)
	}
	if err := run(operation, false); err != nil {
		t.Fatal("offline upgrade", err)
	}
	for _, recover := range []bool{false, true} {
		if err := run(operation, recover); err != nil {
			t.Fatal("original operation replay/recovery", err)
		}
	}
	if err := run("rss_upgrade_unrelated_002", true); !errors.Is(err, ErrConflict) {
		t.Fatal("different operation recovered", err)
	}
	var preserved bool
	err = owner.QueryRowContext(ctx, `SELECT (SELECT content='existing release' FROM lmm_operations.publications WHERE id=1)
 AND (SELECT count(*)=1 FROM lmm_operations.reads WHERE user_id=502 AND note_id=1)
 AND (SELECT revision=0 AND feeds='[]'::jsonb FROM lmm_operations.rss_config WHERE singleton)
 AND NOT has_table_privilege('rss_upgrade_runtime','lmm_operations.schema_upgrades','SELECT,INSERT,UPDATE,DELETE')
 AND has_table_privilege('rss_upgrade_runtime','lmm_operations.rss_config','SELECT')
 AND has_table_privilege('rss_upgrade_runtime','lmm_operations.rss_config','UPDATE')`).Scan(&preserved)
	if err != nil || !preserved {
		t.Fatal("upgrade changed old content or permissions", err, preserved)
	}
	if _, err := owner.ExecContext(ctx, `ALTER TABLE lmm_operations.rss_config ADD COLUMN drift text`); err != nil {
		t.Fatal(err)
	}
	if err := run(operation, true); !errors.Is(err, ErrUnavailable) {
		t.Fatal("recovery accepted schema drift", err)
	}
}
