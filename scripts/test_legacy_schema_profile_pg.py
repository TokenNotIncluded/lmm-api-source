"""Core58 SQL-fixture migration, readback and refusal tests on private PG18.

No Core build, production connection or paid service. This tests the reviewed
DDL fixture, not actual Core init-db or independent Core59 acceptance. Run with
python3 -B scripts/test_legacy_schema_profile_pg.py -v. Evidence is retained in
~/.cache/lmm-import-profile-pg-*; successful owned PGDATA is removed after stop.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import uuid

import test_migrate_legacy_snapshot as T
import test_financial_import as M
from test_financial_import_pg import reviewed_fixture_ddl

I, F = T.MODULE, M.F


class Schema58PostgresTests(unittest.TestCase):
    def run_command(self, command, *, sql=None, owner=False, database="postgres", error=None):
        self.number += 1
        label = f"{self.number:03d}"
        env = dict(self.env, PGDATABASE=database)
        if owner:
            env.update(PGUSER="profile_installer", PGOPTIONS="-c role=profile_owner")
        if sql is not None:
            (self.output / (label + ".sql")).write_text(sql)
        result = subprocess.run(command, input=sql, env=env, text=True,
                                capture_output=True, timeout=120)
        record = {"command": command, "database": database, "owner": owner,
                  "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
        (self.output / (label + ".json")).write_text(json.dumps(record, indent=2) + "\n")
        if error is None:
            self.assertEqual(result.returncode, 0, f"{label}: {result.stderr}")
        else:
            self.assertNotEqual(result.returncode, 0, label)
            self.assertIn(error, result.stderr, label)
        return result.stdout.strip()

    def sql(self, text, *, owner=True, database="profile_target", error=None):
        return self.run_command(["psql", "-X", "-w", "-qAt", "-v", "ON_ERROR_STOP=1"],
                                sql=text, owner=owner, database=database, error=error)

    def query(self, text, **options):
        return json.loads(self.sql(text, **options))

    def test_restored_source_identity_opening_and_profile_refusals(self):
        # Refuse an unreviewed source before allocating or starting a database.
        contract = I.current_contract()
        self.assertEqual(contract["import_profile"], I.CORE58_SEED_CONTRACT)
        cache = Path.home() / ".cache"
        self.assertGreater(shutil.disk_usage(cache).free, 512 * 1024 * 1024)
        self.output = Path(tempfile.mkdtemp(prefix="lmm-import-profile-pg-", dir=cache))
        self.number = 0
        print("Private synthetic SQL-fixture evidence: " + str(self.output), flush=True)
        socket = self.output / "sock"
        socket.mkdir(mode=0o700)
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("PG", "LMM_")) and key not in ("DATABASE_URL", "SQL_DSN")}
        self.env.update(PGHOST=str(socket), PGPORT="65431", PGUSER="profile_admin",
                        PGDATABASE="postgres", PGPASSFILE="/dev/null", PGSERVICEFILE="/dev/null",
                        LC_ALL="C", PSQL_HISTORY="/dev/null")
        data = self.output / "data"
        started = False
        passed = False
        try:
            for name in ("postgres", "initdb", "pg_ctl", "psql", "pg_dump", "pg_restore"):
                self.assertIn("(PostgreSQL) 18.", self.run_command([name, "--version"]))
            self.run_command(["findmnt", "-T", str(self.output)])
            self.run_command(["df", "-h", str(self.output)])
            self.run_command(["initdb", "-D", str(data), "--auth=trust", "--username=profile_admin",
                              "--no-locale", "--encoding=UTF8"])
            self.run_command(["pg_ctl", "-D", str(data), "-l", str(self.output / "postgres.log"),
                              "-o", f"-k {socket} -p 65431 -c listen_addresses='' -c max_connections=10"
                              " -c shared_buffers=16MB -c autovacuum=off"
                              " -c lmm.migration_instance_nonce=" + uuid.uuid4().hex + uuid.uuid4().hex,
                              "-w", "start"])
            started = True
            self.sql("CREATE ROLE profile_owner NOLOGIN; CREATE ROLE profile_installer LOGIN NOINHERIT;"
                     "GRANT profile_owner TO profile_installer; CREATE ROLE profile_runtime NOLOGIN;"
                     "CREATE DATABASE profile_source; CREATE DATABASE profile_restored;",
                     owner=False, database="postgres")
            source_rows = [{**T.user(uid, username="profile-" + str(uid), email="", role=role),
                            **M.user(uid, quota=quota, aff_quota=affiliate)}
                           for uid, role, quota, affiliate in ((71, 1, -120, 0), (72, 100, 0, -5), (73, 1, 43, 0))]
            self.sql("CREATE TABLE source_users(payload jsonb NOT NULL); INSERT INTO source_users VALUES "
                     + ",".join("(" + I.sql_literal(I.canonical(row)) + "::jsonb)" for row in source_rows) + ";",
                     database="profile_source", owner=False)
            dump = self.output / "synthetic-source.dump"
            self.run_command(["pg_dump", "-Fc", "-f", str(dump), "profile_source"])
            self.run_command(["pg_restore", "-d", "profile_restored", str(dump)])
            rows = self.query("BEGIN READ ONLY; SELECT json_agg(payload ORDER BY (payload->>'id')::bigint)"
                              " FROM source_users; COMMIT;", database="profile_restored", owner=False)
            self.assertEqual(rows, source_rows)
            archive = hashlib.sha256(dump.read_bytes()).hexdigest()
            plan = I.map_identity(rows, [], source_sha256=archive)
            manifest = T.manifest_for(plan, source_archive_sha256=archive,
                                      source_export_sha256=F.sha(I.canonical(rows).encode()))
            self.sql("CREATE DATABASE profile_target OWNER profile_owner TEMPLATE template0;",
                     owner=False, database="postgres")
            self.sql("REVOKE ALL ON DATABASE profile_target FROM PUBLIC;"
                     "GRANT CONNECT ON DATABASE profile_target TO profile_installer;",
                     owner=False, database="postgres")
            self.sql("REVOKE ALL ON SCHEMA public FROM PUBLIC;" + reviewed_fixture_ddl(contract))
            self.sql("GRANT EXECUTE ON FUNCTION pg_control_system() TO profile_owner;", owner=False)
            actual_catalog = self.sql(I.catalog_hash_sql(contract))
            self.assertEqual(actual_catalog, I.CORE58_CATALOG_SHA256)
            target = T.target_contract(catalog_fingerprint_sha256=actual_catalog,
                                       trust_policy_seed=self.query(I.trust_policy_seed_query()),
                                       initialized_core_seed_sha256=self.sql(I.fresh_seed_hash_query()))
            identity = self.query(I.IDENTITY_SQL)
            identity_sql = I.build_import_sql(plan, migration_id="profile-identity", source_sha256=archive,
                                             target_contract=target, manifest=manifest, target_identity=identity)
            for grant, revoke in (
                ("GRANT EXECUTE ON FUNCTION core_console.write_display_exchange_rate(text) TO PUBLIC;",
                 "REVOKE EXECUTE ON FUNCTION core_console.write_display_exchange_rate(text) FROM PUBLIC;"),
                ("GRANT EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) TO PUBLIC;",
                 "REVOKE EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) FROM PUBLIC;"),
                ("GRANT SELECT(request) ON core_meta.commerce_display_rate_upgrades TO profile_runtime;",
                 "REVOKE SELECT(request) ON core_meta.commerce_display_rate_upgrades FROM profile_runtime;"),
            ):
                self.sql(grant)
                self.sql(identity_sql, error="fresh target profile privileges mismatch")
                self.assertEqual(self.sql("SELECT count(*) FROM core_identity.users"), "0")
                self.sql(revoke)
            self.sql(identity_sql)
            first = self.query("SELECT jsonb_build_object('users',(SELECT jsonb_agg(jsonb_build_array(id,platform_role,active) ORDER BY id) FROM core_identity.users),"
                               "'finance',(SELECT count(*) FROM core_billing.opening_imports),"
                               "'receipts',(SELECT count(*) FROM core_migration.import_runs))")
            self.assertEqual(first, {"users": [[71, "user", True], [72, "superadmin", False], [73, "user", True]],
                                     "finance": 0, "receipts": 1})
            inventory = self.query(F.inventory_sql())
            before_replay = self.query(F.state_sql(inventory, "no-exclusion"))
            self.sql(identity_sql)
            self.assertEqual(before_replay, self.query(F.state_sql(inventory, "no-exclusion")))
            # Clone only our stopped-client test DB; preserve the populated
            # immutable receipt and its refusal rather than trying to erase it.
            self.sql("CREATE DATABASE profile_receipt_bad TEMPLATE profile_target OWNER profile_owner;",
                     database="postgres", owner=False)
            self.sql("INSERT INTO core_meta.commerce_display_rate_upgrades VALUES ('unexpected',"
                     "decode(repeat('00',32),'hex'),'{}',decode(repeat('00',32),'hex'),"
                     "decode(repeat('00',32),'hex'),decode(repeat('00',32),'hex'),clock_timestamp());",
                     database="profile_receipt_bad")
            bad_identity = self.query(I.IDENTITY_SQL, database="profile_receipt_bad")
            bad_sql = I.build_import_sql(plan, migration_id="profile-identity", source_sha256=archive,
                                        target_contract=target, manifest=manifest, target_identity=bad_identity)
            self.sql(bad_sql, database="profile_receipt_bad", error="fresh target has upgrade receipts")
            identities = {row["user_id"]: row["account_id"] for row in plan["records"]}
            provenance = F.PLANNER.bind_identity_provenance(manifest,
                map_file_sha256=manifest["source_target_id_map_sha256"], source_archive_sha256=archive)
            financial_plan = F.PLANNER.plan_financial(M.tables(users=rows, tokens=[], user_subscriptions=[], subscription_plans=[]),
                                                     identities, source_sha256=archive, identity_provenance=provenance)
            snapshot = self.query(F.snapshot_sql(inventory, "profile-opening"))
            reservations = {"source_archive_sha256": archive, "mapping_sha256": financial_plan["mapping_sha256"],
                            "reviewed_evidence_sha256": "d" * 64,
                            "positions": [{"source_user_id": uid, "reserved_units": 0} for uid in identities]}
            review = {"format_version": 1, "scope": F.SCOPE, "decision": "approved-offline-opening",
                      "credits_per_usd": 500000, "source_archive_sha256": archive,
                      "mapping_sha256": financial_plan["mapping_sha256"], "identity_map_sha256": financial_plan["identity_map_sha256"],
                      "evidence_sha256": "e" * 64, "other_rights_policy": "remain-disabled-pending-separate-import"}
            # The approvals and economics here are explicitly synthetic fixture
            # inputs. They certify no real source units or production reviewer.
            files = {name: {"path": str(self.output / name), "sha256": "f" * 64} for name in F.SOURCE_FILES}
            files["archive"]["sha256"] = archive
            files["identity_map"]["sha256"] = manifest["source_target_id_map_sha256"]
            material = F.prepare_material(financial_plan, migration_id="profile-opening", identities=identities,
                observation=F.PLANNER.source_observation({"started_utc": "20261001T000000Z"}),
                reservations=reservations, review=review, snapshot=snapshot, runtime_roles=["profile_runtime"],
                source_files=files, target_contract=target)
            opening_sql = F.build_apply_sql(material)
            F.verify_approval(material, opening_sql, M.approval_fixture(material, opening_sql))
            self.sql("GRANT SELECT(request) ON core_meta.commerce_display_rate_upgrades TO profile_runtime;")
            self.sql(opening_sql, error="financial_profile_privileges_mismatch")
            self.sql("REVOKE SELECT(request) ON core_meta.commerce_display_rate_upgrades FROM profile_runtime;")
            self.sql(opening_sql)
            financial_readback = self.query("SELECT jsonb_agg(jsonb_build_array(a.owner_account_id,a.bucket,b.balance_units)"
                " ORDER BY a.owner_account_id,a.bucket) FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b"
                " ON b.ledger_account_id=a.id WHERE a.owner_account_id IS NOT NULL")
            self.assertEqual(financial_readback, [[1, "affiliate", 0], [1, "debt", -120], [1, "reserved", 0], [1, "wallet", 0],
                [2, "affiliate", -5], [2, "debt", 0], [2, "reserved", 0], [2, "wallet", 0],
                [3, "affiliate", 0], [3, "debt", 0], [3, "reserved", 0], [3, "wallet", 43]])
            before_replay = self.query(F.state_sql(inventory, "no-exclusion"))
            self.sql(opening_sql)
            self.assertEqual(before_replay, self.query(F.state_sql(inventory, "no-exclusion")))
            self.sql(identity_sql, error="non-canonical initialized ledger seed")  # Upstream replay is closed after financial writes.
            (self.output / "result.json").write_text(json.dumps({"status": "SELF_TEST_PASS", "schema": 58,
                "scope": "synthetic dump/restore, strict identity/opening SQL, target readback and ACL/receipt refusals",
                "actual_core_init_db": False, "core59_verified": False, "independent_acceptance": False,
                "archive_sha256": archive, "catalog_sha256": actual_catalog,
                "identity_readback": first, "financial_readback": financial_readback}, indent=2) + "\n")
            passed = True
        finally:
            if started:
                self.run_command(["pg_ctl", "-D", str(data), "-m", "fast", "-w", "stop"])
            if passed:
                shutil.rmtree(data)


if __name__ == "__main__":
    unittest.main()
