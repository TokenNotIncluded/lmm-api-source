"""Private PG18 migration/guard tests, with opt-in actual Core59 init receipts.

No Core build, production connection or paid service. The default is the reviewed
Core58 fixture. Candidate59 guards require an explicit frozen DDL path/hash;
actual59 init requires an explicit binary and binary/manifest/catalog hashes.
Profile admission is bound independently to committed3579ac7. The migration
case uses real init on reviewed59 and the old DDL fixture only on reviewed58.
Use an explicit single method filter; independent acceptance is separate. Evidence is retained in
~/.cache/lmm-import-profile-pg-*; successful owned PGDATA is removed after stop.
"""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import uuid
from urllib.parse import urlencode

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

    @contextmanager
    def private_cluster(self):
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
                     "GRANT profile_owner TO profile_installer; CREATE ROLE profile_runtime NOLOGIN;",
                     owner=False, database="postgres")
            yield
            passed = True
        finally:
            if started:
                self.run_command(["pg_ctl", "-D", str(data), "-m", "fast", "-w", "stop"])
            if passed:
                shutil.rmtree(data)


    @unittest.skipUnless(os.environ.get("LMM_IMPORT_CANDIDATE59_DDL"), "explicit frozen candidate59 DDL required")
    def test_candidate59_acl_receipt_and_empty_state_guards(self):
        # This is a guard-only candidate SQL fixture. Never install a candidate
        # manifest/runtime receipt or call an importer through a patched profile.
        contract = I.current_contract()
        self.assertEqual(contract["import_profile"], I.CORE58_SEED_CONTRACT)
        ddl = Path(os.environ["LMM_IMPORT_CANDIDATE59_DDL"]).read_bytes()
        ddl_hash = hashlib.sha256(ddl).hexdigest()
        self.assertEqual(ddl_hash, os.environ["LMM_IMPORT_CANDIDATE59_SHA256"])
        candidate = {**contract, "component_fingerprints": {
            **contract["component_fingerprints"], "subscription-epay": ddl_hash}}
        with self.private_cluster():
            self.sql("CREATE DATABASE profile_target OWNER profile_owner TEMPLATE template0;"
                     "REVOKE ALL ON DATABASE profile_target FROM PUBLIC;"
                     "GRANT CONNECT ON DATABASE profile_target TO profile_installer;",
                     owner=False, database="postgres")
            self.sql("REVOKE ALL ON SCHEMA public FROM PUBLIC;" + reviewed_fixture_ddl(contract) + ddl.decode())
            guard = I.profile_acl_guard_sql(candidate)
            self.assertEqual(self.sql("SELECT (" + guard + ")"), "f")
            grants = [
                "GRANT SELECT ON core_meta.subscription_epay_upgrades TO profile_runtime;",
                "GRANT SELECT(request) ON core_meta.subscription_epay_upgrades TO profile_runtime;",
                "GRANT SELECT(subscription_order_id) ON core_payments.provider_identifiers TO profile_runtime;",
                "GRANT SELECT(subscription_order_id) ON core_payments.provider_identifiers TO PUBLIC;",
            ] + ["GRANT EXECUTE ON FUNCTION " + signature + " TO PUBLIC;" for signature in (
                "core_subscriptions.guard_external_order()", "core_subscriptions.check_external_grant()",
                "core_subscriptions.external_ends_at(jsonb,bigint)",
                "core_subscriptions.apply_epay_grant(text,text,text)")]
            for grant in grants:
                self.sql("BEGIN;" + grant + "DO $guard$ BEGIN IF " + guard
                         + " THEN RAISE EXCEPTION 'candidate59_profile_acl_rejected'; END IF; END $guard$;",
                         error="candidate59_profile_acl_rejected")
                self.assertEqual(self.sql("SELECT (" + guard + ")"), "f")  # Failed connection rolled back.
            # Expected runtime function grants remain legal. New-column ACLs do
            # not: table-level SELECT/INSERT already covers the nullable subject.
            self.sql("BEGIN; GRANT EXECUTE ON FUNCTION core_subscriptions.external_ends_at(jsonb,bigint),"
                     "core_subscriptions.apply_epay_grant(text,text,text) TO profile_runtime;"
                     "DO $guard$ BEGIN IF " + guard
                     + " THEN RAISE EXCEPTION 'legitimate_runtime_grant_rejected'; END IF; END $guard$; ROLLBACK;")
            tables = (*I.fresh_business_tables(candidate), "core_meta.subscription_epay_upgrades")
            readback = {table: int(self.sql("SELECT count(*) FROM " + table)) for table in tables}
            self.assertEqual(set(readback.values()), {0})
            seed_query = I.fresh_seed_hash_query(I.CORE59_SEED_CONTRACT)
            seed_before = self.sql(seed_query)
            receipt = "INSERT INTO core_meta.subscription_epay_upgrades VALUES ('unexpected',"
            receipt += "decode(repeat('00',32),'hex'),'{}',decode(repeat('00',32),'hex'),"
            receipt += "decode(repeat('00',32),'hex'),decode(repeat('00',32),'hex'),clock_timestamp());"
            self.sql("BEGIN;" + receipt + "DO $guard$ BEGIN IF " + I.upgrade_receipts_guard_sql(candidate)
                     + " THEN RAISE EXCEPTION 'candidate59_upgrade_receipt_rejected'; END IF; END $guard$;",
                     error="candidate59_upgrade_receipt_rejected")
            self.assertEqual(self.sql("SELECT count(*) FROM core_meta.subscription_epay_upgrades"), "0")
            seed_with_receipt = self.sql("BEGIN;" + receipt + seed_query + "; ROLLBACK;")
            self.assertNotEqual(seed_with_receipt, seed_before)
            self.assertEqual(self.sql("SELECT count(*) FROM core_payments.provider_identifiers"
                                     " WHERE subscription_order_id IS NOT NULL"), "0")
            (self.output / "result.json").write_text(json.dumps({
                "status": "CANDIDATE_GUARD_SELF_TEST_PASS", "candidate_ddl_sha256": ddl_hash,
                "acl_refusals": len(grants), "upgrade_receipt_refusals": 1,
                "empty_table_readback": readback, "profile59_enabled": False,
                "actual_core59_init_db": False, "migration59_executed": False,
                "independent_acceptance": False}, indent=2) + "\n")

    def init_actual_core59(self):
        contract = I.current_contract()
        self.assertEqual(contract["import_profile"], I.CORE59_SEED_CONTRACT)
        self.assertEqual(os.environ["LMM_IMPORT_CORE59_MANIFEST_SHA256"], contract["compiled_manifest_sha256"])
        self.assertEqual(os.environ["LMM_IMPORT_CORE59_CATALOG_SHA256"], contract["fresh_catalog_sha256"])
        binary = Path(os.environ["LMM_IMPORT_CORE59_BINARY"])
        expected_binary = os.environ["LMM_IMPORT_CORE59_BINARY_SHA256"]
        def binary_hash():
            with binary.open("rb") as file:
                return hashlib.file_digest(file, "sha256").hexdigest()
        self.assertEqual(binary_hash(), expected_binary)
        manifest_text = self.run_command([str(binary), "schema-manifest"])
        manifest = json.loads(manifest_text)
        manifest_hash = hashlib.sha256((manifest_text + "\n").encode()).hexdigest()
        self.assertEqual(manifest_hash, os.environ["LMM_IMPORT_CORE59_MANIFEST_SHA256"])
        self.assertEqual(len(manifest["components"]), 59)
        self.assertEqual(manifest["components"], [
            {"name": name, "raw_schema_sha256": value}
            for name, value in contract["component_fingerprints"].items()])
        self.assertEqual(manifest["components"][58]["name"], "subscription-epay")
        self.assertEqual(manifest["catalog"]["query_utf8"], contract["catalog_query"])
        self.sql("CREATE DATABASE profile_target OWNER profile_owner TEMPLATE template0;"
                 "REVOKE ALL ON DATABASE profile_target FROM PUBLIC;"
                 "GRANT CONNECT ON DATABASE profile_target TO profile_installer;",
                 owner=False, database="postgres")
        self.sql("REVOKE ALL ON SCHEMA public FROM PUBLIC;")
        database_url = self.output / "database-url"
        database_url.write_text("postgresql://profile_installer@localhost/profile_target?" + urlencode({
            "host": self.env["PGHOST"], "port": self.env["PGPORT"],
            "options": "-c role=profile_owner"}) + "\n")
        database_url.chmod(0o600)
        config = self.output / "owner.toml"
        config.write_text('version=1\n[database]\nurl_file=' + json.dumps(str(database_url))
                          + '\nmax_connections=1\n[server]\nworker_threads=1\n')
        config.chmod(0o600)
        self.run_command([str(binary), "--config", str(config), "init-db"])
        actual_catalog = self.sql(I.catalog_hash_sql(contract))
        self.assertEqual(actual_catalog, os.environ["LMM_IMPORT_CORE59_CATALOG_SHA256"])
        components = self.query("SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract")
        self.assertEqual(components, {row["name"]: row["raw_schema_sha256"] for row in manifest["components"]})
        self.assertEqual(self.sql("SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract"), actual_catalog)
        observed = {"component_fingerprints": components}
        self.assertEqual(self.sql("SELECT (" + I.profile_acl_guard_sql(observed) + ")"), "f")
        self.assertEqual(self.sql("SELECT (" + I.upgrade_receipts_guard_sql(observed) + ")"), "f")
        empty = {name: int(self.sql("SELECT count(*) FROM " + name)) for name in (
            *I.fresh_business_tables(observed), *I.upgrade_receipt_tables(observed),
            "core_identity.users", "core_migration.import_runs")}
        self.assertEqual(set(empty.values()), {0})
        acl = self.query("""SELECT jsonb_build_object(
 'roles',(SELECT jsonb_agg(jsonb_build_object('name',rolname,'login',rolcanlogin,'super',rolsuper,
 'create_role',rolcreaterole,'create_db',rolcreatedb,'replication',rolreplication,'bypass_rls',rolbypassrls)
 ORDER BY rolname) FROM pg_roles WHERE rolname IN ('profile_owner','profile_installer','profile_runtime')),
 'memberships',(SELECT coalesce(jsonb_agg(jsonb_build_array(pg_get_userbyid(roleid),pg_get_userbyid(member),
 admin_option,inherit_option,set_option) ORDER BY roleid,member),'[]'::jsonb) FROM pg_auth_members
 WHERE member IN (SELECT oid FROM pg_roles WHERE rolname IN ('profile_owner','profile_installer','profile_runtime'))),
 'objects',(SELECT jsonb_agg(v ORDER BY v::text) FROM (
 SELECT jsonb_build_array('schema',n.nspname,pg_get_userbyid(n.nspowner),n.nspacl::text) v
 FROM pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
 UNION ALL SELECT jsonb_build_array('relation',n.nspname,c.relname,pg_get_userbyid(c.relowner),c.relacl::text)
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
 UNION ALL SELECT jsonb_build_array('column',n.nspname,c.relname,t.attname,t.attacl::text)
 FROM pg_attribute t JOIN pg_class c ON c.oid=t.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE t.attnum>0 AND NOT t.attisdropped AND n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
 UNION ALL SELECT jsonb_build_array('function',n.nspname,p.proname,pg_get_function_identity_arguments(p.oid),
 pg_get_userbyid(p.proowner),p.proacl::text,p.prosecdef,p.proconfig)
 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_') facts))""")
        (self.output / "actual-acl.json").write_text(json.dumps(acl, indent=2) + "\n")
        receipt = {"status": "actual-core59-init-catalog-acl-observed", "binary": str(binary),
            "binary_sha256": expected_binary, "compiled_manifest_sha256": manifest_hash,
            "actual_catalog_sha256": actual_catalog, "components": components,
            "instance": self.query(I.IDENTITY_SQL), "empty_table_readback": empty,
            "initialized_core_seed_contract": I.CORE59_SEED_CONTRACT,
            "initialized_core_seed_sha256": self.sql(I.fresh_seed_hash_query(I.CORE59_SEED_CONTRACT)),
            "actual_acl_sha256": hashlib.sha256((self.output / "actual-acl.json").read_bytes()).hexdigest(),
            "application_or_worker_started": False, "runtime_grants_installed": False,
            "profile59_enabled": contract.get("import_profile") == I.CORE59_SEED_CONTRACT, "migration59_executed": False,
            "source_status": "3579ac7 reviewed schema sources; actual binary snapshot separately bound, not a final combined release",
            "independent_acceptance": False}
        self.assertEqual(binary_hash(), expected_binary)
        (self.output / "actual-core59-init-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        return receipt


    @unittest.skipUnless(os.environ.get("LMM_IMPORT_CORE59_BINARY"), "explicit fixed actual Core59 binary required")
    def test_actual_core59_init_catalog_acl_receipt(self):
        with self.private_cluster():
            receipt = self.init_actual_core59()
            (self.output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")


    def test_restored_source_identity_opening_and_profile_refusals(self):
        # Refuse an unreviewed source before allocating or starting a database.
        contract = I.current_contract()
        profile59 = contract["import_profile"] == I.CORE59_SEED_CONTRACT
        self.assertIn(contract["import_profile"], (I.CORE58_SEED_CONTRACT, I.CORE59_SEED_CONTRACT))
        if profile59:
            self.assertTrue(os.environ.get("LMM_IMPORT_CORE59_BINARY"), "Core59 migration requires an explicit actual binary")
        with self.private_cluster():
            self.sql("CREATE DATABASE profile_source; CREATE DATABASE profile_restored;",
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
            if profile59:
                actual_init = self.init_actual_core59()
            else:
                actual_init = None
                self.sql("CREATE DATABASE profile_target OWNER profile_owner TEMPLATE template0;",
                         owner=False, database="postgres")
                self.sql("REVOKE ALL ON DATABASE profile_target FROM PUBLIC;"
                         "GRANT CONNECT ON DATABASE profile_target TO profile_installer;",
                         owner=False, database="postgres")
                self.sql("REVOKE ALL ON SCHEMA public FROM PUBLIC;" + reviewed_fixture_ddl(contract))
            self.sql("GRANT EXECUTE ON FUNCTION pg_control_system() TO profile_owner;", owner=False)
            actual_catalog = self.sql(I.catalog_hash_sql(contract))
            self.assertEqual(actual_catalog, contract["fresh_catalog_sha256"])
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
            suffix_receipt = "core_meta.subscription_epay_upgrades" if profile59 else "core_meta.commerce_display_rate_upgrades"
            self.sql("INSERT INTO " + suffix_receipt + " VALUES ('unexpected',"
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
            self.sql("GRANT SELECT(request) ON " + suffix_receipt + " TO profile_runtime;")
            self.sql(opening_sql, error="financial_profile_privileges_mismatch")
            self.sql("REVOKE SELECT(request) ON " + suffix_receipt + " FROM profile_runtime;")
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
            (self.output / "result.json").write_text(json.dumps({"status": "SELF_TEST_PASS", "schema": 59 if profile59 else 58,
                "scope": "synthetic dump/restore, strict identity/opening SQL, target readback and ACL/receipt refusals",
                "actual_core_init_db": profile59, "core59_verified": profile59, "independent_acceptance": False,
                "actual_init_receipt": actual_init, "source_schema_revision": contract["source_schema_revision"],
                "continuing_entitlements_and_permissions_imported": False,
                "replay_scope": "same phase replay leaves full state unchanged; upstream identity replay refuses after opening",
                "archive_sha256": archive, "catalog_sha256": actual_catalog,
                "identity_readback": first, "financial_readback": financial_readback}, indent=2) + "\n")


if __name__ == "__main__":
    unittest.main()
