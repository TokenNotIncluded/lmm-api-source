"""Private PG18 migration/guard tests, with opt-in actual Core59 init receipts.

No Core build, production connection or paid service. Reviewed Core58 retains its
historical fixture. Candidate59 guards require an explicit frozen DDL path/hash;
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
import sys
import tempfile
import unittest
import uuid
from urllib.parse import urlencode

import test_migrate_legacy_snapshot as T
import test_financial_import as M
from test_financial_import_pg import reviewed_fixture_ddl

I, F = T.MODULE, M.F


def continuity_source_fixture():
    """Synthetic source for the next combined PG case, not an import approval.

    Keep all three rights in one source. Times are frozen at November 2026;
    never infer a subscription renewal amount from current product pricing.
    These rows must still be dumped, restored and exported before a PG apply.
    """
    from datetime import datetime, timezone
    from test_migrate_legacy_financial import tables, subscription, plan
    from test_legacy_api_key_migration import token

    epoch = lambda year, month: int(datetime(year, month, 1, tzinfo=timezone.utc).timestamp())
    users = [{**T.user(uid, username="profile-" + str(uid), email="", role=role),
              **M.user(uid, quota=quota, aff_quota=affiliate),
              "group": group, "status": status, "trust_level_override": override,
              "setting": '{"billing_preference":"wallet_first"}'}
             for uid, role, quota, affiliate, group, status, override in (
                 (71, 1, -120, 0, "paid", 1, 1),
                 (72, 100, 0, -5, "inactive-original", 2, None),
                 (73, 1, 43, 0, "paid", 1, 1))]
    common = dict(upgrade_group="", prev_user_group="", downgrade_group="",
                  start_time=epoch(2026, 11), end_time=epoch(2028, 11),
                  last_reset_time=epoch(2026, 11), next_reset_time=epoch(2026, 12))
    return tables(users=users, top_ups=[], tokens=[
        token(id=41, user_id=71),
        token(id=42, user_id=73, key="SyntheticOtherPrivateCredentialBytes1234567890",
              unlimited_quota=True, remain_quota=-80, one_time_reveal=False)],
        user_subscriptions=[subscription(4, **common, reset_amount=2000),
            subscription(5, **common, user_id=73, amount_total=0, amount_used=0,
                         reset_amount=0, status="cancelled"),
            subscription(6, **common, user_id=73, amount_total=0, reset_amount=None)],
        subscription_plans=[plan()], options=[
            {"key": "AutoGroups", "value": '["paid"]'},
            {"key": "MaxTokenAutoGroups", "value": "4"},
            {"key": "ModelRequestRateLimitEnabled", "value": "false"},
            {"key": "PasswordLoginEnabled", "value": "true"}])


class ContinuityPreparationTests(unittest.TestCase):
    """Pure preparation checks. No DB, fabricated PG receipt or apply approval."""

    def setUp(self):
        from copy import deepcopy
        from test_legacy_subscription_opening import fixture
        import legacy_subscription_opening as S
        self.S = S
        self.tables = continuity_source_fixture()
        attestation, review = fixture()
        self.plan = F.PLANNER.plan_financial(self.tables, {71: 1, 72: 2, 73: 3},
            source_sha256=attestation["source_archive_sha256"], source_timezone="UTC",
            identity_provenance=attestation["source_plan"]["identity_provenance"])
        self.review = {**deepcopy(review), "mapping_sha256": self.plan["mapping_sha256"],
            "identity_map_sha256": self.plan["identity_map_sha256"], "policies": [
                {"source_subscription_id": row["source_subscription_id"],
                 "models": ["synthetic-model"], "groups": ["paid"],
                 "source_row_sha256": row["source_row_sha256"],
                 "plan_row_sha256": row["plan_row_sha256"]}
                for row in self.plan["subscription_entitlements"]]}

    def test_current_future_cancelled_zero_and_unlimited_are_distinct(self):
        records = self.S.build_records(self.plan, self.review)
        self.assertEqual([(r["source_subscription_id"], r["limit_credits"], r["opening_used"],
                           r["reset_limit_credits"], r["unlimited"], r["active"], r["reset_unlimited"])
                          for r in records],
                         [(4, 1000, 400, 2000, False, True, False),
                          (5, 0, 0, 0, False, False, False),
                          (6, 0, 400, 0, True, True, True)])
        self.assertEqual([(r["opening_start"], r["opening_end"], r["ends_at"]) for r in records],
                         [(1793491200, 1796083200, 1856649600)] * 3)

    def test_missing_finite_reset_basis_still_refuses(self):
        from copy import deepcopy
        tables = deepcopy(self.tables)
        tables["user_subscriptions"][0]["reset_amount"] = None
        plan = F.PLANNER.plan_financial(tables, {71: 1, 72: 2, 73: 3},
            source_sha256=self.plan["source_archive_sha256"], source_timezone="UTC",
            identity_provenance=self.plan["identity_provenance"])
        review = {**self.review, "mapping_sha256": plan["mapping_sha256"], "policies": [
            {**policy, "source_row_sha256": row["source_row_sha256"]}
            for policy, row in zip(self.review["policies"], plan["subscription_entitlements"])]}
        with self.assertRaises(F.Error):
            self.S.build_records(plan, review)

    def test_original_group_and_disabled_owner_are_source_facts(self):
        from copy import deepcopy
        import legacy_routing_migration as R
        before = deepcopy(self.tables)
        tables = {**self.tables, "channels": [], "abilities": []}
        plan = R.plan(tables, {71: 1, 72: 2, 73: 3},
            {"source_archive_sha256": self.plan["source_archive_sha256"],
             "source_export_sha256": "c" * 64, "source_target_id_map_sha256": "d" * 64},
            source_counts={"public_relay_preferences": 0})
        self.assertEqual([(r["user_id"], r["group_name"], r["enabled"])
                          for r in plan["user_group_plan"]],
                         [(71, "paid", True), (72, "inactive-original", False), (73, "paid", True)])
        self.assertEqual(self.tables, before)
        self.assertFalse(plan["apply_supported"])

    def test_key_proposal_retains_original_secrets_and_unlimited_remaining(self):
        from test_legacy_api_key_migration import MODULE as K, inputs
        source, target = inputs(tokens=self.tables["tokens"])
        source["users"] = [{k: row[k] for k in K.USER_FIELDS - {"billing_preference"}}
                           | {"billing_preference": "wallet_first"} for row in self.tables["users"]]
        mapping = [{"source_user_id": uid, "target_user_id": uid,
                    "target_personal_account_id": account} for uid, account in ((71, 1), (72, 2), (73, 3))]
        target.update(source_target_id_map=mapping,
            source_target_id_map_sha256=hashlib.sha256(K.id_map_bytes(mapping).encode()).hexdigest(),
            users=[{"user_id": row["id"], "personal_account_id": i, "auth_version": 1,
                    "active": row["status"] == 1, "developer_access_granted": row["role"] == 1,
                    "routing_groups": [row["group"]]} for i, row in enumerate(self.tables["users"], 1)])
        proposal = K.plan_api_keys(source, target)
        self.assertEqual([r["digest_sha256"] for r in proposal["records"]],
                         [hashlib.sha256(row["key"].encode()).hexdigest() for row in self.tables["tokens"]])
        self.assertEqual(proposal["status"], "blocked")
        self.assertEqual(proposal["summary"]["blocked_reason_counts"],
                         {"recoverable_secret_encryption_required": 1})
        self.assertEqual([(r["quota"]["remaining"], r["quota"]["opening_consumed"],
                           r["quota"]["display_limit_credits"], r["quota"]["limit_credits"],
                           r["quota"]["unlimited"], r["one_time_reveal"])
                          for r in proposal["records"]],
                         [(30, 70, 100, 100, False, True), (-80, 70, -10, 0, True, False)])
        import legacy_credential_import as C
        prepared = C._supported_rows(proposal, target, 1)
        recovery = prepared[1]
        synthetic_key = bytes(range(32))  # Public test material; never a production key.
        recovery["encrypted_secret_hex"] = C.encrypt_secret(synthetic_key,
            recovery["target_credential_id"], recovery["target_user_id"], self.tables["tokens"][1]["key"])
        C.verify_encrypted_secret(synthetic_key, recovery, self.tables["tokens"][1]["key"])
        with self.assertRaises(C.Error):
            C.verify_encrypted_secret(bytes(reversed(range(32))), recovery, self.tables["tokens"][1]["key"])
        self.assertFalse(proposal["apply_supported"])
        self.assertFalse(proposal["activation_approved"])


class Schema58PostgresTests(unittest.TestCase):
    def private_json(self, directory, name, value):
        return F.write_private(directory, name, F.canonical(value) + "\n")

    def import_cli(self, script, output, *, expected_error=None, **arguments):
        command = [sys.executable, "-B", str(Path(__file__).with_name(script))]
        for name, value in arguments.items():
            flag = "--" + name.replace("_", "-")
            if value is True:
                command.append(flag)
            elif value is not None:
                for item in value if isinstance(value, list) else [value]:
                    command.extend([flag, str(item)])
        command.extend(["--output", str(output)])
        raw = self.run_command(command, error=expected_error)
        return None if expected_error else json.loads(raw)

    def record_phase(self, name, before, tables, sequences):
        after = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.private_json(self.private, name + "-before.private.json", before)
        self.private_json(self.private, name + "-after.private.json", after)
        for kind, changed in (("tables", set(tables) | {"core_migration.import_runs"}),
                              ("sequences", set(sequences))):
            self.assertEqual({k: v for k, v in before[kind].items() if k not in changed},
                             {k: v for k, v in after[kind].items() if k not in changed}, name)
        return after

    def replay_phase(self, name, sql, resolve):
        before = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.assertEqual(self.query(sql)["status"], "COMMITTED")
        self.assertEqual(self.query(resolve)["status"], "COMMITTED")
        self.assertEqual(before, self.query(F.state_sql(self.inventory, "no-exclusion")))
        self.private_json(self.private, name + "-replay.private.json",
                          {"status": "COMMITTED", "full_state_unchanged": True})

    def restored_physical_source(self):
        """Actual restore, native catalog/count and effective reader ACL proofs."""
        from datetime import datetime, timezone
        from decimal import Decimal
        from test_migrate_legacy_financial import topup
        source = continuity_source_fixture()
        source.update(two_fas=[], channels=[], abilities=[], public_relay_preferences=[])
        prototypes = {name: rows[0] for name, rows in source.items() if rows}
        prototypes.update(top_ups=topup(), two_fas={"user_id": 71, "is_enabled": False, "deleted_at": None},
            channels={"id": 1, "status": 1, "group": "paid", "models": "synthetic-model"},
            abilities={"channel_id": 1, "group": "paid", "model": "synthetic-model", "enabled": True},
            public_relay_preferences={"id": 1, "user_id": 71})
        self.sql("CREATE DATABASE profile_source TEMPLATE template0; CREATE DATABASE profile_restored TEMPLATE template0;",
                 owner=False, database="postgres")
        definitions = []
        for name, rows in source.items():
            prototype = prototypes[name]
            self.assertTrue(I.IDENTIFIER.fullmatch(name))
            columns = []
            for field, value in prototype.items():
                self.assertTrue(I.IDENTIFIER.fullmatch(field))
                kind = "boolean" if type(value) is bool else "bigint" if type(value) is int else \
                       "numeric" if isinstance(value, Decimal) else "text"
                columns.append('"' + field + '" ' + kind)
            primary = "id" if "id" in prototype else "key" if name == "options" else None
            if primary:
                columns.append('PRIMARY KEY("' + primary + '")')
            definitions.append('CREATE TABLE public."' + name + '"(' + ','.join(columns) + ');')
            if rows:
                definitions.append('INSERT INTO public."' + name + '" SELECT * FROM jsonb_populate_recordset('
                    'NULL::public."' + name + '",' + I.sql_literal(json.dumps(rows)) + '::jsonb);')
        self.sql('\n'.join(definitions), database="profile_source", owner=False)
        started = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        dump = self.private / "synthetic-source.dump"
        self.run_command(["pg_dump", "-Fc", "-f", str(dump), "profile_source"])
        dump.chmod(0o600)
        self.run_command(["pg_restore", "--exit-on-error", "-d", "profile_restored", str(dump)])
        self.sql("CREATE ROLE profile_reader LOGIN NOINHERIT; ALTER ROLE profile_reader"
                 " SET default_transaction_read_only=on; GRANT CONNECT ON DATABASE profile_restored TO profile_reader;",
                 database="postgres", owner=False)
        import legacy_snapshot_oauth as oauth_phase
        reader_grants = "REVOKE ALL ON SCHEMA public FROM PUBLIC; GRANT USAGE ON SCHEMA public TO profile_reader;"
        for table in sorted(source):
            if table == "custom_oauth_providers":
                reader_grants += 'GRANT SELECT (' + ','.join(oauth_phase.PROVIDER_COLUMNS) + ') ON public.custom_oauth_providers TO profile_reader;'
            elif table == "user_oauth_bindings":
                reader_grants += 'GRANT SELECT (' + ','.join(oauth_phase.BINDING_COLUMNS) + ') ON public.user_oauth_bindings TO profile_reader;'
            else:
                # Financial/API-key tests still require the complete original rows.
                reader_grants += 'GRANT SELECT ON public."' + table + '" TO profile_reader;'
        self.sql(reader_grants, database="profile_restored", owner=False)
        reader_command = ["psql", "-X", "-w", "-qAt", "-v", "ON_ERROR_STOP=1", "-U", "profile_reader"]
        def read(sql):
            return json.loads(self.run_command(reader_command, sql="BEGIN READ ONLY;SET LOCAL row_security=off;"
                              + sql + ";COMMIT;", database="profile_restored"))
        def reader_projection(name):
            columns = ','.join(oauth_phase.PROVIDER_COLUMNS) if name == "custom_oauth_providers" else '*'
            return '(SELECT ' + columns + ' FROM public."' + name + '") r'
        if "custom_oauth_providers" in source:
            self.assertTrue(read("SELECT to_json(NOT has_column_privilege(current_user,"
                                 "'public.custom_oauth_providers','client_secret','SELECT'))"))
        actual_rows = read("SELECT jsonb_build_object(" + ','.join(I.sql_literal(name) +
            ',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),\'[]\'::jsonb) FROM '
            + reader_projection(name) + ')' for name in sorted(source)) + ')')
        self.assertEqual({k: sorted(v, key=F.canonical) for k, v in actual_rows.items()},
                         {k: sorted([{field: value for field, value in row.items()
                                      if k != "custom_oauth_providers" or field in oauth_phase.PROVIDER_COLUMNS}
                                     for row in v], key=F.canonical) for k, v in source.items()})
        audit = read("""SELECT jsonb_build_object(
 'no_role_write_authority',(SELECT NOT(rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls)
                          AND NOT EXISTS(SELECT 1 FROM pg_auth_members WHERE member=pg_roles.oid)
                          FROM pg_roles WHERE rolname=current_user),
 'no_application_write',NOT EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='public' AND c.relkind IN ('r','p') AND
 (has_table_privilege(c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')
  OR has_any_column_privilege(c.oid,'INSERT,UPDATE,REFERENCES'))),
 'no_schema_create',NOT has_schema_privilege('public','CREATE'),
 'no_security_definer_execute',NOT EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname='public' AND p.prosecdef AND has_function_privilege(p.oid,'EXECUTE')),
 'row_security_off',current_setting('row_security')='off',
 'read_only',current_setting('transaction_read_only')='on',
 'readback_users',(SELECT count(*) FROM public.users))""")
        self.assertEqual(audit, {"no_role_write_authority": True, "no_application_write": True,
            "no_schema_create": True, "no_security_definer_execute": True,
            "row_security_off": True, "read_only": True, "readback_users": len(source["users"])})
        self.run_command(reader_command, sql="BEGIN READ WRITE; UPDATE public.users SET quota=quota+1; ROLLBACK;",
                         database="profile_restored", error="permission denied for table users")
        self.run_command(reader_command, sql="SET ROLE profile_admin;", database="profile_restored",
                         error="permission denied to set role")
        self.private_json(self.private, "reader-audit.private.json", audit)
        metadata = [read("SELECT to_jsonb(d) FROM pg_database d WHERE datname=current_database()"),
            read("SELECT to_jsonb(n) FROM pg_namespace n WHERE nspname='public'"),
            read("""SELECT jsonb_agg(jsonb_build_object('schema',n.nspname,'table_name',c.relname,
 'column_name',a.attname,'type',format_type(a.atttypid,a.atttypmod),'not_null',a.attnotnull)
 ORDER BY c.relname,a.attnum) FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid
 JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='public' AND c.relkind IN ('r','p') AND a.attnum>0 AND NOT a.attisdropped"""),
            read("""SELECT coalesce(jsonb_agg(jsonb_build_object('schema',n.nspname,'table_name',c.relname,
 'contype',k.contype,'definition',pg_get_constraintdef(k.oid)) ORDER BY c.relname,k.conname),'[]'::jsonb)
 FROM pg_constraint k JOIN pg_class c ON c.oid=k.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='public'"""),
            read("SELECT coalesce(jsonb_agg(to_jsonb(i) ORDER BY indexname),'[]'::jsonb) FROM pg_indexes i WHERE schemaname='public'"),
            read("SELECT coalesce(jsonb_agg(to_jsonb(s) ORDER BY sequencename),'[]'::jsonb) FROM pg_sequences s WHERE schemaname='public'"),
            read("SELECT coalesce(jsonb_agg(jsonb_build_object('name',p.proname,'secdef',p.prosecdef) ORDER BY p.oid),'[]'::jsonb)"
                 " FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'")]
        catalog = F.write_private(self.private, "native-catalog-metadata.jsonl", '\n'.join(F.canonical(v) for v in metadata) + '\n')
        counts = read("SELECT jsonb_build_object(" + ','.join(I.sql_literal(name) +
                      ',(SELECT count(*) FROM public."' + name + '")' for name in sorted(source)) + ')')
        self.assertEqual(counts, {name: len(rows) for name, rows in source.items()})
        baseline = F.write_private(self.private, "native-table-row-count-baseline.jsonl",
            '\n'.join(F.canonical({"table": name, "row_count": count}) for name, count in sorted(counts.items())) + '\n')
        identity = read(I.IDENTITY_SQL.rstrip(';'))
        schema_owner = read("SELECT to_jsonb(pg_get_userbyid(nspowner)) FROM pg_namespace WHERE nspname='public'")
        pgpass = F.write_private(self.private, "pgpass.private", "")
        reader = {"socket_directory": self.env["PGHOST"], "port": int(self.env["PGPORT"]),
            "database": "profile_restored", "role": "profile_reader", "pgpass_file": str(pgpass),
            "verified_no_application_write_privileges": audit["no_application_write"] and audit["no_role_write_authority"] and audit["no_schema_create"],
            "verified_no_security_definer_application_function_execute": audit["no_security_definer_execute"],
            "row_security_off_guard_verified": audit["row_security_off"] and audit["read_only"] and audit["readback_users"] == len(source["users"])}
        receipt = {"status": "snapshot-restored-baseline-verified-frozen-and-classified", "started_utc": started,
            "archive": {"path": str(dump), "sha256": I.sha256_file(dump)}, "source": {"schema": "public"},
            "readonly_reader": reader, "source_instance_identity": {**identity, "listen_addresses": "", "schema_owner": schema_owner},
            "private_metadata_files": [str(catalog), str(baseline)]}
        return self.private_json(self.private, "source-receipt.private.json", receipt), receipt, actual_rows

    @unittest.skipUnless(os.environ.get("LMM_IMPORT_CORE59_BINARY"), "explicit frozen Core59 binary required")
    def test_full_physical_legacy_continuity_cli_chain(self):
        """One source/archive/map, actual CLI writes and per-phase target reads.

        This method is the full-chain entry. It never calls the payload-only case.
        All approvals below authorize only this locally created synthetic source.
        No production unit, source-runtime attestation or activation is inferred.
        """
        contract = I.current_contract()
        self.assertEqual(contract["import_profile"], I.CORE59_SEED_CONTRACT)
        for field, variable in (("compiled_manifest_sha256", "LMM_IMPORT_CORE59_MANIFEST_SHA256"),
                                ("fresh_catalog_sha256", "LMM_IMPORT_CORE59_CATALOG_SHA256")):
            self.assertEqual(contract[field], os.environ[variable])
        root = I.BACKUP_ROOT / "core59-continuity"
        root.mkdir(mode=0o700, exist_ok=True)
        I.private_backup_path(root, directory=True)
        self.private = Path(tempfile.mkdtemp(prefix="run-", dir=root))
        print("Complete-chain private evidence: " + str(self.private), flush=True)
        # Check both actual destinations before the >100 MB PGDATA write.
        preflight = {}
        for name, command in (("cache_mount", ["findmnt", "-T", str(Path.home() / ".cache")]),
                              ("private_mount", ["findmnt", "-T", str(self.private)]),
                              ("space", ["df", "-h", str(Path.home() / ".cache"), str(self.private)]),
                              ("memory", ["free", "-m"])):
            result = subprocess.run(command, text=True, capture_output=True, check=True)
            preflight[name] = {"command": command, "stdout": result.stdout, "stderr": result.stderr}
        preflight["retained_owned_data"] = [str(p / "data") for p in
            sorted((Path.home() / ".cache").glob("lmm-import-profile-pg-*")) if (p / "data").is_dir()]
        self.private_json(self.private, "resource-preflight.private.json", preflight)
        self.assertGreater(shutil.disk_usage(self.private).free, 1024 ** 3)
        self.assertGreater(shutil.disk_usage(Path.home() / ".cache").free, 1024 ** 3)
        with self.private_cluster():
            (self.output / "private-evidence-path.txt").write_text(str(self.private) + "\n")
            self.stage = "physical-source-restore"
            try:
                self.execute_continuity_chain(contract)
            except Exception as error:
                import traceback
                self.private_json(self.private, "failure.private.json", {
                    "status": "FAILED_OR_OUTCOME_UNCONFIRMED", "stage": self.stage,
                    "exception_type": type(error).__name__, "traceback": traceback.format_exc(),
                    "pg_evidence": str(self.output), "independent_acceptance": False})
                raise

    def execute_continuity_chain(self, contract):
        import legacy_subscription_opening as S
        import legacy_routing_grants as G
        import legacy_credential_import as C
        source_root = Path(__file__).resolve().parents[1]
        def source_hashes():
            return {str(p.relative_to(source_root)): I.sha256_file(p) for p in sorted(source_root.rglob('*'))
                    if p.is_file() and '__pycache__' not in p.parts}
        frozen_sources = source_hashes()
        self.private_json(self.private, "inputs-before.private.json", frozen_sources)
        source_path, source_receipt, original = self.restored_physical_source()
        archive = source_receipt["archive"]["sha256"]
        self.stage = "actual-core59-init-for-import"
        actual_init = self.init_actual_core59()
        self.sql("GRANT EXECUTE ON FUNCTION pg_control_system() TO profile_owner;", owner=False)
        self.inventory = self.query(F.inventory_sql())
        target_contract = {**contract, "catalog_fingerprint_sha256": self.sql(I.catalog_hash_sql(contract)),
            "allow_initialized_core_seed": True, "initialized_core_seed_contract": I.CORE59_SEED_CONTRACT,
            "initialized_core_seed_sha256": self.sql(I.fresh_seed_hash_query(I.CORE59_SEED_CONTRACT)),
            "trust_policy_seed": self.query(I.trust_policy_seed_query())}
        connection = {"socket_directory": self.env["PGHOST"], "port": int(self.env["PGPORT"]),
            "database": "profile_target", "role": "profile_owner", "login_role": "profile_installer",
            "set_role": "profile_owner", "pgpass_file": source_receipt["readonly_reader"]["pgpass_file"]}
        instance = self.query(F.instance_sql())
        identity_target = self.private_json(self.private, "identity-target.private.json", {
            "status": "initialized-empty-identity-target", "migration_id": "chain-identity",
            "source_archive_sha256": archive, "registry_provisioned": True,
            "application_or_worker_started": False, "connection": connection,
            "instance_identity": instance, "contract": target_contract})
        self.stage = "identity-export-and-cli-apply"
        identity_output = self.private / "identity-apply"
        before = self.query(F.state_sql(self.inventory, "no-exclusion"))
        identity_result = self.import_cli("migrate-legacy-snapshot.py", identity_output,
            source_receipt=source_path, target_receipt=identity_target, migration_id="chain-identity",
            apply=True, confirm_source_sha256=archive)
        self.assertEqual(identity_result["status"], "identity-import-verified")
        identity_sql = (identity_output / "identity-import.private.sql").read_text()
        after_identity = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.assertEqual(self.query(identity_sql)["status"], "identity-import-verified")
        self.assertEqual(after_identity, self.query(F.state_sql(self.inventory, "no-exclusion")))
        # Identity deliberately writes all native identity settings, but no money
        # or unrelated component. Its SQL also checks every untouched sequence.
        self.record_phase("identity", before,
            ["core_identity.accounts", "core_identity.users", "core_identity.user_profiles",
             "core_identity.password_logins", "core_identity.developer_access",
             "core_identity.privileged_import_facts", "core_billing.account_policies"],
            ["core_identity.users_id_seq", "core_identity.accounts_id_seq"])
        identity_readback = self.query("SELECT jsonb_agg(jsonb_build_array(id,personal_account_id,platform_role,active,auth_version)"
                                       " ORDER BY id) FROM core_identity.users")
        self.assertEqual(identity_readback, [[71, 1, "user", True, 1], [72, 2, "superadmin", False, 1],
                                            [73, 3, "user", True, 1]])
        identity_manifest = identity_output / "manifest.private.json"
        identity_map = identity_output / "source-target-id-map.private.jsonl"
        manifest = json.loads(identity_manifest.read_text())
        map_rows = [json.loads(line) for line in identity_map.read_text().splitlines()]
        identities = F.PLANNER.identity_index(map_rows)
        self.stage = "financial-real-export-and-plan-cli"
        financial_export = self.private / "financial-export"
        self.import_cli("migrate-legacy-financial.py", financial_export,
            source_receipt=source_path, identity_map=identity_map, identity_manifest=identity_manifest,
            migration_id="chain-opening", source_timezone="UTC")
        plan_path = financial_export / "financial-plan.private.json"
        proof_path = financial_export / "source-export-proof.private.json"
        plan = F.PLANNER.parse_json(plan_path.read_text())
        proof = F.PLANNER.parse_json(proof_path.read_text())
        columns, baseline, _, _, _ = F.PLANNER.metadata(source_receipt)
        selected = sorted(name for name in columns if F.PLANNER.table_families(name) or F.PLANNER.FINANCIAL_NAME.search(name))
        tables = F.PLANNER.read_frozen_export(financial_export / "source-financial-export.private.jsonl",
            source_receipt, columns, baseline, selected, proof["source_instance"])
        self.assertEqual(sorted(tables["users"], key=lambda r: r["id"]),
                         sorted(original["users"], key=lambda r: r["id"]))
        self.assertEqual(proof["selected_table_counts"], {name: baseline[name] for name in selected})
        # Evidence and approvals concern known synthetic integers and zero holds
        # in this newly created fixture, never production financial authority.
        evidence = self.private_json(self.private, "synthetic-economic-evidence.private.json", {
            "scope": "local-synthetic-source-only", "source_archive_sha256": archive,
            "source_export_sha256": proof["source_export_sha256"], "credit_units_per_source_integer": 1,
            "wallet_debt_affiliate": [[71, -120, 0], [72, 0, -5], [73, 43, 0]],
            "reserved_units": [[71, 0], [72, 0], [73, 0]],
            "source_baseline": baseline, "production_approval": False})
        review = {"format_version": 1, "scope": F.SCOPE, "decision": "approved-offline-opening",
            "credits_per_usd": 500000, "source_archive_sha256": archive,
            "mapping_sha256": plan["mapping_sha256"], "identity_map_sha256": plan["identity_map_sha256"],
            "evidence_sha256": I.sha256_file(evidence), "other_rights_policy": "remain-disabled-pending-separate-import"}
        review_path = self.private_json(self.private, "opening-review.private.json", review)
        reservation_path = self.private_json(self.private, "reservations.private.json", {
            "source_archive_sha256": archive, "mapping_sha256": plan["mapping_sha256"],
            "reviewed_evidence_sha256": I.sha256_file(evidence),
            "positions": [{"source_user_id": uid, "reserved_units": 0} for uid in sorted(identities)]})
        financial_target = self.private_json(self.private, "financial-target.private.json", {
            "status": "initialized-identity-bound-financial-target", "migration_id": "chain-opening",
            "source_archive_sha256": archive, "application_or_worker_started": False,
            "connection": connection, "instance_identity": instance, "contract": target_contract})
        self.stage = "financial-cli-prepare-approval-apply-replay-resolve"
        prepared = self.private / "opening-prepare"
        self.assertEqual(self.import_cli("financial_import.py", prepared, prepare=True, plan=plan_path,
            source_receipt=source_path, export_proof=proof_path, identity_map=identity_map,
            identity_manifest=identity_manifest, source_review=review_path, reservations=reservation_path,
            target_receipt=financial_target, migration_id="chain-opening", runtime_role=["profile_runtime"])["status"],
            "PREPARED_NOT_APPLIED")
        opening_material = json.loads((prepared / "material.private.json").read_text())
        opening_sql = (prepared / "opening.private.sql").read_text()
        approval = json.loads((prepared / "approval-required.private.json").read_text())
        self.assertEqual(approval["decision"], "REVIEW_REQUIRED")
        approved = self.private_json(self.private, "opening-approval.private.json",
                                     {**approval, "decision": "approve-offline-opening"})
        before = self.query(F.state_sql(self.inventory, "no-exclusion"))
        invalid = self.private_json(self.private, "opening-invalid-approval.private.json", approval)
        opening_args = dict(material=prepared / "material.private.json", sql=prepared / "opening.private.sql",
                            target_receipt=financial_target)
        self.import_cli("financial_import.py", self.private / "opening-refused", apply=True,
                        approval=invalid, expected_error="offline_approval_mismatch", **opening_args)
        self.assertEqual(before, self.query(F.state_sql(self.inventory, "no-exclusion")))
        opening_applied = self.private / "opening-apply"
        self.assertEqual(self.import_cli("financial_import.py", opening_applied, apply=True,
            approval=approved, **opening_args)["status"], "COMMITTED")
        self.record_phase("opening", before, F.CHANGED_TABLES, F.CHANGED_SEQUENCES)
        committed = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.assertEqual(self.import_cli("financial_import.py", self.private / "opening-replay", apply=True,
            approval=approved, **opening_args)["status"], "COMMITTED")
        self.assertEqual(self.import_cli("financial_import.py", self.private / "opening-resolve",
            resolve_attempt=opening_applied / "attempt.private.json", approval=approved, **opening_args)["status"], "COMMITTED")
        self.assertEqual(committed, self.query(F.state_sql(self.inventory, "no-exclusion")))
        money_sql = ("SELECT jsonb_agg(jsonb_build_array(a.owner_account_id,a.bucket,b.balance_units) ORDER BY a.owner_account_id,a.bucket)"
                     " FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id"
                     " WHERE a.owner_account_id IS NOT NULL")
        money = self.query(money_sql)
        self.assertEqual(money, [[1,"affiliate",0],[1,"debt",-120],[1,"reserved",0],[1,"wallet",0],
            [2,"affiliate",-5],[2,"debt",0],[2,"reserved",0],[2,"wallet",0],
            [3,"affiliate",0],[3,"debt",0],[3,"reserved",0],[3,"wallet",43]])
        self.stage = "subscription-reviewed-apply-replay-resolve"
        # A new observation of the target for the NEXT phase, never retarget the
        # already approved opening material or replay it against later state.
        subscription_attestation = F.prepare_material(plan, migration_id="chain-opening", identities=identities,
            observation=F.PLANNER.source_observation(source_receipt),
            reservations=json.loads(reservation_path.read_text()), review=review,
            snapshot=self.query(F.snapshot_sql(self.inventory, "chain-subscriptions")),
            target_contract=target_contract, runtime_roles=["profile_runtime"], source_files=opening_material["source_files"])
        F.verify_source_files(subscription_attestation)
        subscription_review = {"format_version": 1, "scope": S.SCOPE, "source_contract": S.SOURCE_CONTRACT,
            "decision": "approved-offline-subscriptions", "source_archive_sha256": archive,
            "mapping_sha256": plan["mapping_sha256"], "identity_map_sha256": plan["identity_map_sha256"],
            "credits_per_usd": 500000, "evidence_sha256": I.sha256_file(evidence), "source_timezone": "UTC",
            "renewal_policy": "separate-grant-after-expiry", "policies": [
                {"source_subscription_id": row["source_subscription_id"], "models": ["synthetic-model"], "groups": ["paid"],
                 "source_row_sha256": row["source_row_sha256"], "plan_row_sha256": row["plan_row_sha256"]}
                for row in plan["subscription_entitlements"]]}
        sub = S.prepare_material(subscription_attestation, subscription_review, migration_id="chain-subscriptions")
        sub_sql = S.build_apply_sql(sub)
        sub_approval = {"format_version": 1, "scope": S.SCOPE, "decision": "approve-offline-subscriptions",
            "material_sha256": F.digest(sub), "sql_sha256": F.sha(sub_sql.encode()),
            "source_review_sha256": F.digest(subscription_review)}
        self.private_json(self.private, "subscriptions-material.private.json", sub)
        self.private_json(self.private, "subscriptions-approval.private.json", sub_approval)
        F.write_private(self.private, "subscriptions.private.sql", sub_sql)
        S.verify_approval(sub, sub_sql, sub_approval)
        sub_app = "lmm-subscription-" + uuid.uuid4().hex
        self.private_json(self.private, "subscriptions-attempt.private.json", {"application_name": sub_app,
            "material_sha256": F.digest(sub), "sql_sha256": F.sha(sub_sql.encode())})
        sub_resolve = S.build_resolve_sql(sub, sub_app)
        before = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.assertEqual(self.query(sub_resolve)["status"], "UNKNOWN")
        self.assertEqual(before, self.query(F.state_sql(self.inventory, "no-exclusion")))
        self.assertEqual(self.query(sub_sql, application_name=sub_app)["status"], "COMMITTED")
        self.record_phase("subscriptions", before, S.CHANGED_TABLES, [S.SEQUENCE])
        self.replay_phase("subscriptions", sub_sql, sub_resolve)
        subscriptions = self.query("SELECT jsonb_agg(jsonb_build_array(s.grant_reference,s.limit_credits,o.consumed_credits,"
            "s.reset_limit_credits,s.unlimited,s.reset_unlimited,s.active,s.models,s.groups) ORDER BY s.id)"
            " FROM core_charging.subscriptions s JOIN core_charging.subscription_opening_consumption o ON o.subscription_id=s.id")
        self.assertEqual(subscriptions, [["legacy-sub:chain-subscriptions:4",1000,400,2000,False,False,True,["synthetic-model"],["paid"]],
            ["legacy-sub:chain-subscriptions:5",0,0,0,False,False,False,["synthetic-model"],["paid"]],
            ["legacy-sub:chain-subscriptions:6",0,400,0,True,True,True,["synthetic-model"],["paid"]]])
        for at, limit in ((1793491200, 1000), (1796083200, 2000)):
            self.assertEqual(self.query("SELECT jsonb_build_array(limit_credits,unlimited) FROM core_charging.subscription_window(1,"
                                        + str(at) + ")"), [limit, False])
        self.stage = "original-routing-cli-prepare-apply-replay-resolve"
        routing = G.ROUTING.plan({**tables, "channels": original["channels"], "abilities": original["abilities"]}, identities,
            {"source_archive_sha256": archive, "source_export_sha256": proof["source_export_sha256"],
             "identity_map_file_sha256": I.sha256_file(identity_map)},
            source_counts={"public_relay_preferences": baseline["public_relay_preferences"]})
        routing_path = self.private_json(self.private, "original-routing-plan.private.json", routing)
        routing_snapshot = self.private_json(self.private, "routing-snapshot.private.json",
            self.query(G.snapshot_sql(self.inventory, "chain-routing")))
        routing_output = self.private / "routing-prepare"
        self.assertEqual(self.import_cli("legacy_routing_grants.py", routing_output, routing_plan=routing_path,
            identity_manifest=identity_manifest, identity_map=identity_map,
            source_identity_export=identity_output / "source-identity-export.private.jsonl",
            source_financial_export=financial_export / "source-financial-export.private.jsonl",
            source_receipt=source_path, export_proof=proof_path, snapshot=routing_snapshot,
            migration_id="chain-routing", runtime_role=["profile_runtime"])["status"], "PREPARED_NOT_APPLIED")
        grants = (routing_output / "grants.private.sql").read_text()
        routing_app = "lmm-routing-" + uuid.uuid4().hex
        self.private_json(self.private, "routing-attempt.private.json", {"application_name": routing_app,
            "sql_sha256": F.sha(grants.encode())})
        routing_resolve_output = self.private / "routing-resolve"
        self.import_cli("legacy_routing_grants.py", routing_resolve_output,
            material=routing_output / "material.private.json", original_application_name=routing_app)
        routing_resolve = (routing_resolve_output / "resolve.private.sql").read_text()
        before = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.assertEqual(self.query(routing_resolve)["status"], "UNKNOWN")
        self.assertEqual(before, self.query(F.state_sql(self.inventory, "no-exclusion")))
        self.assertEqual(self.query(grants, application_name=routing_app)["status"], "COMMITTED")
        self.record_phase("routing", before, [G.TABLE], [])
        self.replay_phase("routing", grants, routing_resolve)
        groups = self.query("SELECT jsonb_agg(jsonb_build_array(user_id,group_name,enabled,source_reference) ORDER BY user_id)"
                            " FROM core_identity.user_routing_groups")
        self.assertEqual(groups, [[71,"paid",True,"legacy.users/71"], [72,"inactive-original",False,"legacy.users/72"],
                                  [73,"paid",True,"legacy.users/73"]])
        # Only an empty synthetic routing release with the restored original
        # paid group. No provider/channel or monetary authority is fabricated.
        self.stage = "synthetic-routing-release-for-key-group-resolution"
        before = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.sql("INSERT INTO public.routing_price_versions(version,document) VALUES(1,'{\"version\":1}');"
                 "INSERT INTO public.routing_config_versions(version,price_version,document) VALUES(1,1,'{\"version\":1}');"
                 "INSERT INTO public.routing_groups(version,name,enabled) VALUES(1,'paid',true);"
                 "UPDATE public.routing_price_versions SET sealed=true WHERE version=1;"
                 "UPDATE public.routing_config_versions SET sealed=true WHERE version=1;"
                 "UPDATE public.routing_active_release SET config_version=1;")
        self.record_phase("synthetic-routing-release", before, ["public.routing_price_versions",
            "public.routing_config_versions", "public.routing_groups", "public.routing_active_release"], [])
        self.stage = "credential-source-projection-and-economic-review"
        snapshot = self.query(C.snapshot_sql(self.inventory, "chain-credentials"))
        # A synthetic protocol fixture, not a claim that a legacy production
        # binary was observed. Project actual exported fields; invent no runtime
        # buildinfo_matches_live_binary receipt to use the production projector.
        K = C.PROPOSAL
        schemas = {name: proof["source_column_schema"][name] for name in ("tokens", "users", "options")}
        self.assertEqual({row["column_name"] for row in schemas["tokens"]}, K.TOKEN_FIELDS)
        options = {row["key"]: row["value"] for row in tables["options"]}
        projection_evidence = {"scope": "synthetic-export-projection-only", "source_archive_sha256": archive,
            "source_export_sha256": proof["source_export_sha256"], "source_receipt_sha256": I.sha256_file(source_path),
            "export_proof_sha256": I.sha256_file(proof_path), "actual_column_schema": schemas,
            "legacy_live_runtime_observed": False}
        projection_path = self.private_json(self.private, "synthetic-key-projection-evidence.private.json", projection_evidence)
        wire = {"format_version": 1, "source_revision": K.LEGACY_REVISION, "source_archive_sha256": archive,
            "source_schema_sha256": K.sha256(schemas), "source_export_sha256": proof["source_export_sha256"],
            "source_projection_sha256": I.sha256_file(projection_path),
            "snapshot_at": F.PLANNER.source_observation(source_receipt)["observed_at"], "tokens": tables["tokens"],
            "users": [{**{name: row[name] for name in K.USER_FIELDS - {"billing_preference"}},
                       "billing_preference": json.loads(row["setting"])["billing_preference"]} for row in tables["users"]],
            "policy": {"auto_groups": json.loads(options["AutoGroups"]), "max_token_auto_groups": int(options["MaxTokenAutoGroups"]),
                       "user_model_rate_limit_enabled": options["ModelRequestRateLimitEnabled"] == "true"}}
        target_projection = {"format_version": 1, "source_archive_sha256": archive, "source_target_id_map": map_rows,
            "source_target_id_map_sha256": manifest["source_target_id_map_sha256"], "users": snapshot["native_users"]}
        proposal = K.plan_api_keys(wire, target_projection)
        self.assertEqual(proposal["summary"]["blocked_reason_counts"], {"recoverable_secret_encryption_required": 1})
        credential_review = {"format_version": 1, "scope": C.SCOPE,
            "decision": "approve-offline-native-equivalent-credentials", "source_input_sha256": K.sha256(wire),
            "target_input_sha256": K.sha256(target_projection), "proposal_sha256": proposal["proposal_sha256"],
            "source_frozen": True, "source_unit": "source_quota_integer", "native_unit": "credit_500k_usd",
            "numerator": 1, "denominator": 1, "evidence_sha256": review["evidence_sha256"],
            "economic_review_sha256": F.digest(review)}
        credential_files = {name: self.private_json(self.private, "credential-" + name + ".private.json", value)
            for name, value in (("source", wire), ("target_projection", target_projection),
                                ("proposal", proposal), ("review", credential_review))}
        key_path = self.private / "credential-recovery-key.private"
        key = os.urandom(32)
        with os.fdopen(os.open(key_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), "wb") as stream:
            stream.write(key); stream.flush(); os.fsync(stream.fileno())
        credential_target = self.private_json(self.private, "credential-target.private.json", {
            "application_or_worker_started": False, "connection": connection, "instance_identity": instance})
        self.stage = "credential-cli-prepare-approval-apply-replay-verify"
        credential_prepared = self.private / "credential-prepare"
        self.assertEqual(self.import_cli("legacy_credential_import.py", credential_prepared, prepare=True,
            **credential_files, encryption_key=key_path, target_receipt=credential_target,
            migration_id="chain-credentials", runtime_role=["profile_runtime"])["status"], "PREPARED_NOT_APPLIED")
        credential_material = json.loads((credential_prepared / "material.private.json").read_text())
        credential_sql = (credential_prepared / "credentials.private.sql").read_text()
        credential_approval = json.loads((credential_prepared / "approval-required.private.json").read_text())
        self.assertEqual(credential_approval["decision"], "REVIEW_REQUIRED")
        credential_approved = self.private_json(self.private, "credential-approval.private.json",
            {**credential_approval, "decision": "approve-offline-credential-and-quota-import"})
        credential_args = dict(material=credential_prepared / "material.private.json",
            sql=credential_prepared / "credentials.private.sql", target_receipt=credential_target)
        before = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.import_cli("legacy_credential_import.py", self.private / "credential-refused", apply=True,
            approval=credential_prepared / "approval-required.private.json", expected_error="credential_approval_mismatch",
            **credential_args)
        self.assertEqual(before, self.query(F.state_sql(self.inventory, "no-exclusion")))
        credential_applied = self.private / "credential-apply"
        self.assertEqual(self.import_cli("legacy_credential_import.py", credential_applied, apply=True,
            approval=credential_approved, **credential_args)["status"], "COMMITTED")
        self.record_phase("credentials", before, C.CHANGED_TABLES, C.CHANGED_SEQUENCES)
        committed = self.query(F.state_sql(self.inventory, "no-exclusion"))
        self.assertEqual(self.import_cli("legacy_credential_import.py", self.private / "credential-replay", apply=True,
            approval=credential_approved, **credential_args)["status"], "COMMITTED")
        self.assertEqual(self.import_cli("legacy_credential_import.py", self.private / "credential-verify",
            verify_attempt=credential_applied / "attempt.private.json", approval=credential_approved,
            **credential_args)["status"], "COMMITTED")
        self.assertEqual(committed, self.query(F.state_sql(self.inventory, "no-exclusion")))
        self.stage = "credential-and-original-key-target-readback"
        keys = self.query("SELECT jsonb_agg(jsonb_build_array(c.user_id,c.owner_account_id,c.user_version,s.enabled,"
            "s.one_time_reveal,b.limit_credits,b.display_limit_credits,b.unlimited,o.consumed_credits,"
            "coalesce(b.display_limit_credits,b.limit_credits)-o.consumed_credits) ORDER BY c.user_id)"
            " FROM core_identity.credentials c JOIN core_identity.api_key_settings s ON s.credential_id=c.id"
            " JOIN core_charging.budgets b ON b.key_id=c.id JOIN core_charging.budget_opening_consumption o ON o.key_id=c.id")
        self.assertEqual(keys, [[71,1,1,True,True,100,None,False,70,30], [73,3,1,True,False,0,-10,True,70,-80]])
        native_keys = self.query("SELECT jsonb_agg(jsonb_build_object('target_credential_id',c.id,'target_user_id',c.user_id,"
            "'digest_sha256',encode(c.digest,'hex'),'encrypted_secret_hex',encode(s.encrypted_secret,'hex')) ORDER BY c.id)"
            " FROM core_identity.credentials c JOIN core_identity.api_key_settings s ON s.credential_id=c.id")
        self.assertEqual([r["digest_sha256"] for r in native_keys],
                         [hashlib.sha256(row["key"].encode()).hexdigest() for row in tables["tokens"]])
        self.assertIsNone(native_keys[0]["encrypted_secret_hex"])
        C.verify_encrypted_secret(key, native_keys[1], tables["tokens"][1]["key"])
        with self.assertRaises(C.Error):
            C.verify_encrypted_secret(bytes(b ^ 255 for b in key), native_keys[1], tables["tokens"][1]["key"])
        for name in (C.MAPPING_FILE, C.ALIAS_FILE):
            self.assertEqual((credential_applied / name).read_bytes(),
                             (self.private / "credential-replay" / name).read_bytes())
            self.assertEqual((credential_applied / name).read_bytes(),
                             (self.private / "credential-verify" / name).read_bytes())
        receipt = json.loads((credential_applied / C.MAPPING_FILE).read_text())
        self.assertEqual(receipt, credential_material["credential_receipt"])
        self.assertEqual([(r["source_key_id"],r["target_user_id"],r["target_account_id"]) for r in receipt["records"]],
                         [(41,71,1),(42,73,3)])
        self.stage = "final-cross-phase-refusals-and-conservation"
        final_state = self.query(F.state_sql(self.inventory, "no-exclusion"))
        for sql, reason in ((identity_sql, "non-canonical initialized ledger seed"),
                            (opening_sql, "financial_replay_conflict"), (sub_sql, "subscription_replay_conflict"),
                            (grants, "routing_replay_conflict")):
            self.sql(sql, error=reason)
            self.assertEqual(final_state, self.query(F.state_sql(self.inventory, "no-exclusion")))
        self.assertEqual(money, self.query(money_sql))
        self.assertEqual(identity_readback, self.query("SELECT jsonb_agg(jsonb_build_array(id,personal_account_id,platform_role,active,auth_version)"
                                                       " ORDER BY id) FROM core_identity.users"))
        self.assertEqual(self.sql("SELECT count(*) FROM core_identity.credentials WHERE user_id=72"), "0")
        self.assertEqual(self.sql("SELECT count(*) FROM core_migration.import_runs"), "5")
        self.assertEqual(self.sql("SELECT count(*) FROM core_migration.import_runs WHERE source_archive_sha256<>" + F.literal(archive)), "0")
        self.assertEqual(self.sql(I.catalog_hash_sql(contract)), contract["fresh_catalog_sha256"])
        self.assertEqual(I.sha256_file(Path(source_receipt["archive"]["path"])), archive)
        F.verify_source_files(opening_material)
        self.private_json(self.private, "inputs-after.private.json", source_hashes())
        self.assertEqual(frozen_sources, source_hashes())
        self.assertEqual(I.sha256_file(Path(os.environ["LMM_IMPORT_CORE59_BINARY"])), os.environ["LMM_IMPORT_CORE59_BINARY_SHA256"])
        report = {"status": "SELF_TEST_PASS", "scope": "physical synthetic legacy dump/restore, actual exports and staged identity/opening/subscription/routing/credential import",
            "source_schema_revision": contract["source_schema_revision"], "actual_init_receipt": actual_init,
            "consumer_source_root": str(source_root), "consumer_inputs_sha256": F.digest(frozen_sources),
            "binary_path": os.environ["LMM_IMPORT_CORE59_BINARY"],
            "binary_sha256": os.environ["LMM_IMPORT_CORE59_BINARY_SHA256"],
            "archive_sha256": archive, "source_receipt": str(source_path), "private_evidence": str(self.private),
            "identity_cli": identity_result, "financial_target_readback": money,
            "subscription_target_readback": subscriptions, "routing_target_readback": groups, "credential_target_readback": keys,
            "original_recoverable_key_matches_target_ciphertext": True, "same_phase_replays_and_resolvers_verified": True,
            "upstream_replays_refused_after_later_phases": True, "source_manifest_unchanged": True,
            "legacy_live_runtime_observed": False, "runtime_application_started": False, "production_approval": False,
            "independent_acceptance": False, "full_recipe_release_acceptance": False}
        self.private_json(self.private, "result.private.json", report)
        (self.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")

    def run_command(self, command, *, sql=None, owner=False, database="postgres", error=None,
                    application_name=None):
        self.number += 1
        label = f"{self.number:03d}"
        env = dict(self.env, PGDATABASE=database)
        if owner:
            env.update(PGUSER="profile_installer", PGOPTIONS="-c role=profile_owner")
        if application_name:
            env["PGAPPNAME"] = application_name
        if sql is not None:
            (self.output / (label + ".sql")).write_text(sql)
        result = subprocess.run(command, input=sql, env=env, text=True,
                                capture_output=True, timeout=120)
        record = {"command": command, "database": database, "owner": owner,
                  "application_name": env.get("PGAPPNAME"),
                  "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
        (self.output / (label + ".json")).write_text(json.dumps(record, indent=2) + "\n")
        if error is None:
            self.assertEqual(result.returncode, 0, f"{label}: {result.stderr}")
        else:
            self.assertNotEqual(result.returncode, 0, label)
            self.assertIn(error, result.stderr, label)
        return result.stdout.strip()

    def sql(self, text, *, owner=True, database="profile_target", error=None, application_name=None):
        return self.run_command(["psql", "-X", "-w", "-qAt", "-v", "ON_ERROR_STOP=1"],
                                sql=text, owner=owner, database=database, error=error,
                                application_name=application_name)

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
