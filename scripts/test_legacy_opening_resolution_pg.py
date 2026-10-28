"""Real PG18 resolution tests for synthetic subscription and routing imports.

Run: python3 -B scripts/test_legacy_opening_resolution_pg.py -v
Uses its own Unix-only cluster and reviewed DDL, without compiling Core. Physical
restore changes the boot identity: old material must refuse, and freshly prepared
material must still leave a missing receipt UNKNOWN. This is not an external
continuity witness or compiled init-db acceptance.
"""
import copy
import json
from pathlib import Path
import shutil
import subprocess
import unittest
from unittest.mock import patch

import financial_import as F
import legacy_routing_grants as G
import legacy_subscription_opening as S
import test_financial_import_pg as P
import test_legacy_routing_grants as R
import test_legacy_subscription_opening_pg as SP
from test_legacy_subscription_opening import fixture as subscription_fixture
from test_opening_activity_guard_pg import PrivatePostgres


class NativePostgres:
    __unittest_skip__ = False

    @classmethod
    def setUpClass(cls):
        cls.fixture = PrivatePostgres()
        cls.addClassCleanup(cls.cleanup)
        cls.fixture.start()
        cls.db = "postgres"
        cls.sql("CREATE ROLE a03_owner NOLOGIN; CREATE ROLE a03_operator LOGIN;"
                "GRANT a03_owner TO a03_operator; CREATE ROLE a03_runtime NOLOGIN;"
                "CREATE ROLE a03_other LOGIN;", admin=True)

    @classmethod
    def cleanup(cls):
        cls.fixture.clean_success()

    @classmethod
    def command(cls, *, admin=False, application_name="legacy-resolution-test"):
        args = [cls.fixture.tools["psql"], "-X", "-w", "-qAt",
                "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=verbose",
                "-h", str(cls.fixture.socket), "-p", "65432",
                "-U", "opening_guard_test" if admin else cls.installer_role,
                "-d", cls.db]
        return args

    @classmethod
    def sql(cls, sql, *, admin=False, check=True, application_name="legacy-resolution-test"):
        env = {**cls.fixture.env, "PGAPPNAME": application_name,
               "PGOPTIONS": "-c lmm.migration_instance_nonce=" + F.sha(str(cls.fixture.root).encode()) +
               ("" if admin else " -c role=" + cls.owner_role)}
        result = subprocess.run(cls.command(admin=admin), input=sql, env=env,
                                text=True, capture_output=True, timeout=140)
        if check and result.returncode:
            raise AssertionError("private PostgreSQL SQL failed: " + result.stderr[-6000:])
        return result

    def resolve_unchanged(self, expected):
        before = self.full_state()
        result = json.loads(self.sql(self.resolver).stdout)
        self.assertEqual(result["status"], expected)
        self.assertEqual(result["material_sha256"], F.digest(self.profile))
        self.assertEqual(self.full_state(), before)

    def test_resolution_missing_receipt_and_exact_commit_are_read_only(self):
        self.resolve_unchanged("UNKNOWN")
        self.assertEqual(json.loads(self.sql(self.batch).stdout)["status"], "COMMITTED")
        self.resolve_unchanged("COMMITTED")
        committed = self.full_state()
        self.assertEqual(json.loads(self.sql(self.batch).stdout)["status"], "COMMITTED")
        self.assertEqual(self.full_state(), committed)
        sequence = self.inventory["sequences"][0]
        self.sql(f"ALTER SEQUENCE {F.relation(sequence)} RESTART WITH 999;")
        self.rejected_unchanged(self.resolver, self.conflict)

    def test_resolution_absence_drift_and_conflicting_receipt_are_refused(self):
        before = self.full_state()
        sequence = self.inventory["sequences"][0]
        self.sql(f"ALTER SEQUENCE {F.relation(sequence)} RESTART WITH 999;")
        self.rejected_unchanged(self.resolver, self.absence)
        seq = before["sequences"][sequence]
        self.sql(f"SELECT setval({F.literal(sequence)}, {seq['last_value']}, "
                 f"{str(seq['is_called']).lower()});")
        self.assertEqual(self.full_state(), before)
        self.sql("INSERT INTO core_migration.import_runs"
                 "(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES(" +
                 ",".join(map(F.literal, (self.profile["migration_id"], "0" * 64,
                     self.profile["mapping_sha256"], {"scope": self.profile["scope"],
                     "material": self.profile, "post_state": before}))) + ");")
        self.rejected_unchanged(self.resolver, self.conflict)

    def control(self, action):
        self.fixture.run("physical-" + action, [
            self.fixture.tools["pg_ctl"], "-D", str(self.fixture.data),
            "-l", str(self.fixture.root / "postgres.log"),
            "-w", "-t", "20", *([] if action == "start" else ["-m", "fast"]), action,
        ])

    def test_resolution_physical_restore_does_not_prove_nonexecution(self):
        backup = self.fixture.root / "physical-before"
        self.control("stop")
        shutil.copytree(self.fixture.data, backup)
        self.control("start")
        # Capture the new boot identity before the attempt, using unchanged source
        # inputs. No target guard or tool fingerprint is bypassed.
        self.prepare_profile()
        original = copy.deepcopy(self.profile)
        original_resolver = self.resolver
        before = self.full_state()
        identity = self.query(F.instance_sql())
        self.assertEqual(json.loads(self.sql(self.batch).stdout)["status"], "COMMITTED")
        self.resolve_unchanged("COMMITTED")
        self.assertNotEqual(self.full_state(), before)
        self.control("stop")
        shutil.rmtree(self.fixture.data)
        shutil.copytree(backup, self.fixture.data)
        self.control("start")
        shutil.rmtree(backup)
        restored_identity = self.query(F.instance_sql())
        self.assertEqual(restored_identity["system_identifier"], identity["system_identifier"])
        self.assertEqual(restored_identity["database_oid"], identity["database_oid"])
        self.assertNotEqual(restored_identity["postmaster_started_at"], identity["postmaster_started_at"])
        self.assertEqual(self.full_state(), before)
        self.rejected_unchanged(original_resolver, "financial_target_instance_mismatch")
        self.prepare_profile()
        refreshed = copy.deepcopy(self.profile)
        self.target(refreshed)["instance"]["postmaster_started_at"] = (
            self.target(original)["instance"]["postmaster_started_at"])
        self.assertEqual(refreshed, original)
        self.resolve_unchanged("UNKNOWN")
        print(f"PHYSICAL_RESTORE {self.profile['scope']}: exact tables/sequences, same system/OID; "
              "old boot REFUSED; refreshed missing receipt UNKNOWN; no resolver writes", flush=True)


class SubscriptionResolutionTests(NativePostgres, SP.SubscriptionPostgreSQLTests):
    conflict = "subscription_resolution_conflict"
    absence = "subscription_absence_not_proven"

    def setUp(self):
        super().setUp()
        self.prepare_profile()

    @staticmethod
    def target(material):
        return material["financial_attestation"]["target"]

    def prepare_profile(self):
        candidate, review = subscription_fixture()
        snapshot = json.loads(self.sql(F.snapshot_sql(self.inventory, "synthetic-subscription")).stdout)
        candidate["before_state"] = snapshot["state"]
        candidate["target"].update({key: snapshot[key] for key in
            ("instance", "catalog_fingerprint_sha256", "ownership_sha256", "component_fingerprints", "trust_policy_seed")})
        candidate["native_users"] = snapshot["native_users"]
        candidate["identity_import"] = snapshot["import_runs"][0]
        self.profile = S.prepare_material(candidate, review, migration_id="synthetic-subscription")
        self.batch = S.build_apply_sql(self.profile)
        self.resolver = S.build_resolve_sql(self.profile, "lmm-subscription-" + "a" * 32)


class RoutingResolutionTests(NativePostgres, P.PostgreSQLTests):
    conflict = "routing_resolve_conflict"
    absence = "routing_absence_not_proven"

    def setUp(self):
        self.create_test_database()
        self.install_fresh_core()
        self.inputs = R.fixture()
        plan, manifest, mapping, exported, snapshot, files, data = self.inputs
        for user in snapshot["native_users"]:
            self.sql("INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES(" +
                     str(user["personal_account_id"]) + ",'personal');"
                     "INSERT INTO core_identity.users(id,personal_account_id,active,auth_version) VALUES(" +
                     f"{user['id']},{user['personal_account_id']},false,{user['auth_version']});")
        imported = snapshot["identity_import"]
        self.sql("INSERT INTO core_migration.import_runs"
                 "(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES(" +
                 ",".join(F.literal(imported[key]) for key in
                          ("migration_id", "source_archive_sha256", "mapping_sha256", "manifest")) + ");")
        self.inventory = self.query(F.inventory_sql())
        self.prepare_profile()

    @staticmethod
    def target(material):
        return material["target"]

    def prepare_profile(self):
        plan, manifest, mapping, exported, _, files, data = self.inputs
        snapshot = json.loads(self.sql(G.snapshot_sql(self.inventory, "synthetic-routing-grants")).stdout)
        with patch.object(G.IDENTITY, "BACKUP_ROOT", Path(R.SYNTHETIC_ROOT.name).parent), \
                patch.object(G.ROUTING.F.IDENTITY, "BACKUP_ROOT", Path(R.SYNTHETIC_ROOT.name).parent):
            self.profile = G.prepare_material(plan, manifest, mapping, exported, data["source_financial_export"],
                migration_id="synthetic-routing-grants", snapshot=snapshot,
                runtime_roles=[self.runtime_role], source_files=files)
            G.verify_source_files(self.profile)
        self.batch = G.build_apply_sql(self.profile)
        self.resolver = G.build_resolve_sql(self.profile, "lmm-routing-" + "a" * 32)


def load_tests(loader, tests, pattern):
    return unittest.TestSuite(cls(name) for cls in (SubscriptionResolutionTests, RoutingResolutionTests)
        for name in loader.getTestCaseNames(cls)
        if name.startswith(("test_resolution_", "test_sub_")))


if __name__ == "__main__":
    unittest.main()
