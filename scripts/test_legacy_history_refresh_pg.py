#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Opt-in PostgreSQL execution of the log-only refresh; synthetic data only."""
import getpass
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import legacy_history_refresh as R
from test_legacy_history_refresh import BINDING, raw, source_records, target_records, write_export


@unittest.skipUnless(os.environ.get("LMM_HISTORY_REFRESH_PG_TEST") == "1",
                     "isolated PostgreSQL test not enabled")
class PostgreSQLRefreshTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="lmm-history-refresh-")
        cls.root = Path(cls.tmp.name)
        cls.data, cls.socket = cls.root / "data", cls.root / "s"
        cls.socket.mkdir()
        cls.env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        cls.env.update(PGHOST=str(cls.socket), PGPORT="5947", PGUSER=getpass.getuser(),
                       PGDATABASE="synthetic_history")
        subprocess.run(["initdb", "-D", str(cls.data), "-A", "trust", "--no-locale",
                        "--encoding=UTF8"], env=cls.env, check=True, capture_output=True)
        try:
            subprocess.run(["pg_ctl", "-D", str(cls.data), "-l", str(cls.root / "postgres.log"),
                            "-w", "-o", f"-k {cls.socket} -h '' -p 5947 "
                            "-c shared_buffers=16MB -c max_connections=10", "start"],
                           env=cls.env, check=True, capture_output=True)
        except BaseException:
            cls.tmp.cleanup()
            raise

    @classmethod
    def tearDownClass(cls):
        try:
            subprocess.run(["pg_ctl", "-D", str(cls.data), "-m", "fast", "-w", "stop"],
                           env=cls.env, check=True, capture_output=True)
        finally:
            cls.tmp.cleanup()

    @classmethod
    def sql(cls, text, database=None, check=True):
        env = cls.env if database is None else {**cls.env, "PGDATABASE": database}
        result = subprocess.run(["psql", "-XqAt", "-v", "ON_ERROR_STOP=1"],
                              input=text + "\n", text=True, capture_output=True, env=env, check=False)
        if check and result.returncode:
            raise AssertionError(result.stderr)
        return result

    def setUp(self):
        self.sql("DROP DATABASE IF EXISTS synthetic_history;", "postgres")
        self.sql("CREATE DATABASE synthetic_history;", "postgres")
        self.sql("""
CREATE SCHEMA core_meta;
CREATE TABLE core_meta.schema_contract(singleton boolean PRIMARY KEY,version int,fingerprint bytea);
INSERT INTO core_meta.schema_contract VALUES(true,6,decode(repeat('b',64),'hex'));
CREATE SCHEMA core_identity;
CREATE TABLE core_identity.accounts(id bigint PRIMARY KEY);
CREATE TABLE core_identity.users(id bigint PRIMARY KEY,personal_account_id bigint REFERENCES core_identity.accounts);
CREATE TABLE core_identity.credentials(id bigint PRIMARY KEY);
CREATE SCHEMA core_billing;
CREATE TABLE core_billing.balance_state(account_id bigint PRIMARY KEY,balance bigint);
INSERT INTO core_billing.balance_state VALUES(1,12345);
CREATE FUNCTION core_billing.ledger_immutable() RETURNS trigger LANGUAGE plpgsql AS
$$BEGIN RAISE EXCEPTION 'immutable'; END$$;
""")
        repo = Path(__file__).resolve().parent.parent
        self.sql((repo / "services/core/schema/history.sql").read_text())
        self.sql((repo / "services/core/schema/migration.sql").read_text())
        records = target_records(extra_user=True)
        self.sql("INSERT INTO core_identity.accounts VALUES(1),(2);"
                 "INSERT INTO core_identity.users VALUES(71,1),(72,2);")
        for kind, row in records:
            if kind == "sources":
                columns = tuple(row)
                self.sql("INSERT INTO core_history.sources(" + ",".join(columns)
                         + ") OVERRIDING SYSTEM VALUE VALUES("
                         + ",".join(R.lit(row[c]) for c in columns) + ");"
                         "SELECT setval('core_history.sources_source_id_seq',1,true);")
            elif kind == "log_records":
                self.sql(R.H.copy_block("core_history.log_records", tuple(row), [row]))
            elif kind == "scope_receipts":
                self.sql(R.H.copy_block("core_history.scope_receipts", tuple(row), [row]))
            elif kind == "import_runs":
                self.sql("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,"
                         "mapping_sha256,manifest) VALUES("
                         + ",".join((R.lit(row["migration_id"]), R.lit(row["source_archive_sha256"]),
                                     R.lit(row["mapping_sha256"]), R.H.jlit(row["manifest"]))) + ");")
        source_path, target_path = self.root / "source.jsonl", self.root / "target.jsonl"
        write_export(source_path, source_records([raw(), raw(2, 72)], users=(71, 72)))
        write_export(target_path, records)
        binding = {**BINDING, "database_oid": int(self.sql(
            "SELECT oid FROM pg_database WHERE datname=current_database()").stdout),
            "owner": getpass.getuser()}
        extra = [{"source_user_id": 72, "target_user_id": 72, "target_personal_account_id": 2}]
        self.plan = R.prepare(source_path, R.read_target(target_path), binding, extra)
        self.generated = R.build_sql(self.plan)

    def state(self):
        return json.loads(self.sql("""
SELECT jsonb_build_object(
 'logs',(SELECT count(*) FROM core_history.log_records),
 'sources',(SELECT count(*) FROM core_history.sources),
 'scopes',(SELECT count(*) FROM core_history.scope_receipts),
 'runs',(SELECT count(*) FROM core_migration.import_runs),
 'balance',(SELECT balance FROM core_billing.balance_state WHERE account_id=1));
""").stdout)

    def test_real_sql_append_scope_extension_and_identical_replay(self):
        result = self.sql(self.generated)
        self.assertIn("HISTORY_REFRESH_COMMITTED", result.stdout)
        expected = {"logs": 2, "sources": 2, "scopes": 4, "runs": 2, "balance": 12345}
        self.assertEqual(self.state(), expected)
        self.sql(self.generated)
        self.assertEqual(self.state(), expected)
        self.assertEqual(self.sql("SELECT count(*) FROM core_history.scope_receipts "
                                  "WHERE source_id=1 AND user_id=72").stdout.strip(), "1")
        self.assertEqual(self.sql("SELECT count(*) FROM core_history.log_records "
                                  "WHERE token_id IS NOT NULL").stdout.strip(), "0")

    def test_compare_and_swap_rejects_changed_existing_projection(self):
        self.sql("UPDATE core_history.log_records SET quota=99 WHERE source_log_id=1;")
        before = self.state()
        result = self.sql(self.generated, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("history_target_state_changed:log_records", result.stderr)
        self.assertEqual(self.state(), before)

    def test_failed_log_append_rolls_back_source_and_scope_rows(self):
        self.sql("""
CREATE FUNCTION reject_new_log() RETURNS trigger LANGUAGE plpgsql AS
$$BEGIN IF NEW.source_log_id=2 THEN RAISE EXCEPTION 'synthetic_append_failure'; END IF; RETURN NEW; END$$;
CREATE TRIGGER reject_new_log BEFORE INSERT ON core_history.log_records
FOR EACH ROW EXECUTE FUNCTION reject_new_log();
""")
        before = self.state()
        result = self.sql(self.generated, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("synthetic_append_failure", result.stderr)
        self.assertEqual(self.state(), before)


if __name__ == "__main__":
    unittest.main()
