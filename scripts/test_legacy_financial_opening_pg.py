"""Real PG18 reconciliation checks; no Core build or existing database is used.

Run: python3 -B scripts/test_legacy_financial_opening_pg.py -v
Successful clusters are stopped and removed. Failures retain SQL, output and
the stopped cluster at the printed private fixture path.
"""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import subprocess
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import test_legacy_financial_opening_counters as counters
from test_opening_activity_guard_pg import PrivatePostgres

F, O = counters.F, counters.O
I = F.IDENTITY


class OpeningReconciliationPostgresTests(unittest.TestCase):
    def sql(self, text, *, admin=False):
        self.command_count += 1
        label = f"command-{self.command_count}"
        I.write_private(self.fixture.root, label + ".sql", text)
        command = [
            self.fixture.tools["psql"], "-X", "-w", "-qAt",
            "-v", "ON_ERROR_STOP=1", "-h", str(self.fixture.socket),
            "-p", "65432", "-U", "opening_guard_test" if admin else "opening_i",
            "-d", "postgres" if admin else self.database,
        ]
        env = self.fixture.env.copy()
        if not admin:
            env["PGOPTIONS"] = "-c role=opening_o"
        result = subprocess.run(command, input=text, env=env, text=True,
                                capture_output=True, timeout=140)
        I.write_private(self.fixture.root, label + ".stdout", result.stdout)
        I.write_private(self.fixture.root, label + ".stderr", result.stderr)
        self.assertEqual(result.returncode, 0, f"{label}: {result.stderr}")
        return result.stdout.strip()

    def query(self, text):
        return F.parse_json(self.sql(text))

    def prepare(self, include, suffix=""):
        self.database = ("opening_counters" if include else "opening_wallet") + suffix
        fixture = counters.CounterApprovalTests()
        fixture.setUp()
        self.inputs = fixture
        contract = fixture.contract
        L = I.sql_literal
        self.sql(
            f"CREATE DATABASE {self.database} OWNER opening_o TEMPLATE template0;",
            admin=True,
        )
        self.sql(
            f"REVOKE ALL ON DATABASE {self.database} FROM PUBLIC;"
            f"GRANT CONNECT ON DATABASE {self.database} TO opening_i;"
            f"ALTER DATABASE {self.database} SET lmm.migration_instance_nonce="
            "'synthetic-opening-reconcile';",
            admin=True,
        )
        self.sql("REVOKE ALL ON SCHEMA public FROM PUBLIC;")
        ddl = "\n".join(data.decode() for data in I.component_schemas().values())
        ddl += I.installer_metadata().decode()
        ddl += (
            "INSERT INTO core_meta.schema_contract(version,fingerprint) VALUES("
            f"{contract['identity_version']},"
            f"decode({L(contract['identity_fingerprint_sha256'])},'hex'));"
        )
        for name, fingerprint in contract["component_fingerprints"].items():
            ddl += (
                "INSERT INTO core_meta.component_contract VALUES("
                f"{L(name)},decode({L(fingerprint)},'hex'));"
            )
        oauth_catalog = I.catalog_hash_sql({
            "catalog_query": "SELECT d AS definition FROM ("
            + I.oauth_catalog_query() + ") oauth_definitions",
        })
        ddl += (
            "INSERT INTO core_oauth.contract(version,ddl_fingerprint,catalog_fingerprint)"
            f" SELECT 1,decode({L(contract['component_fingerprints']['oauth-grants'])},'hex'),"
            f"decode(({oauth_catalog}),'hex');"
            "INSERT INTO core_meta.runtime_contract(catalog_fingerprint)"
            f" SELECT decode(({I.catalog_hash_sql(contract)}),'hex');"
        )
        self.sql("BEGIN;\n" + ddl + "\nCOMMIT;")
        manifest = fixture.identity_manifest
        self.sql(
            "INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE"
            " VALUES(2,'personal');"
            "INSERT INTO core_identity.users(id,personal_account_id) VALUES(71,2);"
            "INSERT INTO core_migration.import_runs"
            "(migration_id,source_archive_sha256,mapping_sha256,manifest)"
            f" VALUES('synthetic-identity',{L('a' * 64)},"
            f"{L(manifest['mapping_sha256'])},{L(F.canonical(manifest))}::jsonb);"
        )
        ident = self.query(I.IDENTITY_SQL)
        ident["database_oid"] = int(ident["database_oid"])
        ident["cluster_system_identifier"] = self.sql(
            "SELECT system_identifier::text FROM pg_control_system();"
        )
        pgpass = I.write_private(self.fixture.root, self.database + ".pgpass", "")
        target = copy.deepcopy(fixture.target)
        target["instance_identity"] = ident
        target["connection"].update(database=self.database, port=65432,
                                    socket_directory=str(self.fixture.socket),
                                    pgpass_file=str(pgpass))
        target["contract"].update(
            catalog_fingerprint_sha256=self.sql(I.catalog_hash_sql(contract)),
            trust_policy_seed=self.query(I.trust_policy_seed_query()),
            initialized_core_seed_sha256=self.sql(I.fresh_seed_hash_query()),
        )
        target["identity_registry_row"] = self.query(
            "SELECT to_jsonb(r) FROM core_migration.import_runs r"
            " WHERE migration_id='synthetic-identity';"
        )
        target["identity_binding_sha256"] = F.digest({
            "identity_run": target["identity_registry_row"],
            "native_users": [{"id": 71, "personal_account_id": 2}],
        })
        fixture.target = target
        fixture.snapshot = self.query(O.snapshot_query())
        self.approval = fixture.approval(include=include)
        self.include = include
        self.writer = fixture.sql(self.approval, include=include)

    def attempt(self, label, *, commit=False):
        output = self.fixture.root / label
        output.mkdir(mode=0o700)
        backend = I.write_private(output, "opening-backend.private.json", "")
        session = O.attempt_session_sql(F, self.inputs.target, backend)
        I.write_private(output, "opening-session-prefix.private.sql", session)
        I.write_private(output, "opening-import.private.sql", self.writer)
        record = {
            "format_version": 1, "scope": self.approval["scope"],
            "migration_id": self.approval["migration_id"],
            "approval_sha256": F.digest(self.approval),
            "sql_sha256": hashlib.sha256(self.writer.encode()).hexdigest(),
            "session_sql_sha256": hashlib.sha256(session.encode()).hexdigest(),
            "target_receipt_file_sha256": self.approval["target"]["target_receipt_file_sha256"],
            "backend_file": str(backend), "prestate": self.inputs.snapshot,
        }
        attempt = I.write_private(output, "opening-attempt.private.json",
                                  F.canonical(record) + "\n")
        raw = self.sql(session + (self.writer if commit else ""))
        if commit:
            self.assertEqual(F.parse_json(raw)["status"], "applied")
        return attempt

    def reconcile(self, attempt, label, *, unknown):
        output = self.fixture.root / label
        output.mkdir(mode=0o700)
        before = self.query(O.snapshot_query())
        args = SimpleNamespace(
            opening_attempt=attempt, include_opening_counters=self.include,
            _opening_plan=self.inputs.plan, _opening_identities={71: 2},
            _opening_identity_manifest=self.inputs.identity_manifest,
        )
        result = O.reconcile(F, args, target=self.inputs.target,
                             approval=self.approval, snapshot=self.inputs.snapshot,
                             output=output)
        after = self.query(O.snapshot_query())
        self.assertEqual(before, after, "reconciliation changed tables or sequences")
        raw = F.parse_json(
            (output / "opening-reconcile.stdout.private.txt").read_text().strip()
        )
        saved = F.parse_json((output / "opening-reconciliation.private.json").read_text())
        self.assertEqual(saved["status"], result["status"])
        self.assertFalse(result["automatic_retry"])
        if unknown:
            self.assertEqual(raw["status"], "outcome-unknown")
            self.assertFalse(raw["retry_permitted"])
            self.assertEqual(result["status"], "opening-outcome-unknown")
            self.assertEqual(result["error_code"], "opening_history_continuity_unproven")
            self.assertFalse(result["retry_permitted"])
            self.assertNotIn("new_reviewed_sequence_envelope_required", result)
        else:
            self.assertEqual(raw["status"], "replay-noop")
            self.assertEqual(result["status"], "opening-confirmed-committed")
        if self.include:
            self.assertEqual(result["historical_counter_rows_verified"], int(not unknown))

    def test_missing_receipt_is_unknown_and_committed_receipt_still_confirms(self):
        self.fixture = PrivatePostgres()
        self.command_count = 0
        try:
            self.fixture.start()
            with patch.object(I, "BACKUP_ROOT", self.fixture.root):
                self.sql(
                    "CREATE ROLE opening_o NOLOGIN; CREATE ROLE opening_i LOGIN NOINHERIT;"
                    "GRANT opening_o TO opening_i; CREATE ROLE opening_r NOLOGIN;"
                    "GRANT EXECUTE ON FUNCTION pg_control_system() TO opening_o;",
                    admin=True,
                )
                for include in (False, True):
                    self.prepare(include)
                    prefix = self.database
                    attempt = self.attempt(prefix + "-not-started")
                    self.reconcile(attempt, prefix + "-exact-prestate", unknown=True)
                    committed = self.attempt(prefix + "-committed", commit=True)
                    self.reconcile(committed, prefix + "-receipt-present", unknown=False)
                    # A real rollback leaves no receipts but advances sequences.
                    # Even this matching observation is not continuity proof.
                    self.prepare(include, "_rollback")
                    attempt = self.attempt(self.database + "-rolled-back")
                    self.sql(
                        "BEGIN ISOLATION LEVEL SERIALIZABLE;"
                        "SET LOCAL synchronous_commit=on;"
                        "SELECT core_billing.import_opening_balance("
                        + I.sql_literal(F.canonical(self.approval["payloads"][0]))
                        + "::jsonb); ROLLBACK;"
                    )
                    rolled_back = self.query(O.snapshot_query())
                    self.assertEqual(
                        {k: v for k, v in rolled_back.items() if k != "sequence-state"},
                        {k: v for k, v in self.inputs.snapshot.items() if k != "sequence-state"},
                    )
                    self.assertNotEqual(rolled_back["sequence-state"],
                                        self.inputs.snapshot["sequence-state"])
                    self.reconcile(attempt, self.database + "-no-receipt", unknown=True)
        except BaseException:
            print(f"\nFAILURE EVIDENCE RETAINED: {self.fixture.root}", flush=True)
            if (self.fixture.data / "postmaster.pid").exists():
                self.fixture.run("pg_ctl-stop-failure", [
                    self.fixture.tools["pg_ctl"], "-D", str(self.fixture.data),
                    "-w", "-t", "20", "-m", "fast", "stop",
                ])
            raise
        else:
            self.fixture.clean_success()


if __name__ == "__main__":
    unittest.main()
