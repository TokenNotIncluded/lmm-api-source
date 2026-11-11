"""Synthetic unit checks for the read-only native observation boundary."""
import unittest
import os
import tempfile
from argparse import Namespace
import json
import stat
from unittest.mock import patch
from pathlib import Path

import export_legacy_key_target_observation as E


def fixtures():
    receipt = {"scope": E.C.RECEIPT_SCOPE, "source_archive_sha256": "a" * 64,
        "records": [{"source_key_id": 11, "target_credential_id": 21,
                     "target_user_id": 31, "target_account_id": 41,
                     "source_secret_sha256": "b" * 64}]}
    raw = (E.F.canonical(receipt) + "\n").encode()
    alias = {"version": 1, "scope": "legacy-api-key-wire-aliases-v1",
        "source_archive_sha256": "a" * 64, "credential_receipt_sha256": E.F.sha(raw),
        "target": {"database": "synthetic", "database_oid": 51, "instance_nonce": "c" * 64},
        "keys": [{**receipt["records"][0], "credential_user_version": 1, "sk_alias_sha256": "d" * 64}]}
    alias_raw = (E.F.canonical(alias) + "\n").encode()
    audit = {"format_version": 1, "scope": "legacy-credential-committed-artifacts-v1",
        "status": "COMMITTED", "migration_id": "synthetic-keys",
        "material_sha256": "e" * 64, "source_archive_sha256": "a" * 64,
        "credential_count": 1, "credential_receipt_sha256": E.F.sha(raw),
        "alias_manifest_sha256": E.F.sha(alias_raw), "target": alias["target"]}
    return receipt, alias, audit, raw, alias_raw


def observation():
    common = {"scope": "key", "period": "lifetime", "account_id": None,
              "user_id": None, "timezone": "UTC", "anchor": 0, "seconds": 0}
    return {"receipt_count": 1, "receipt_matches": True, "records": [{
        "target_key_id": 21, "user_id": 31, "owner_account_id": 41,
        "personal_account_id": 41, "account_kind": "personal", "secret_sha256": "b" * 64,
        "user_version": 1, "personal_billing_preference": "personal_only",
        "expires_at": "2030-01-01T00:00:00+00:00", "revoked_at": None,
        "created_at": "2020-01-01T00:00:00+00:00", "current_user_version": 1,
        "user_active": False, "account_active": True, "settings": [{"enabled": False, "one_time_reveal": True,
            "model_limits_enabled": False, "model_limits": [], "allow_ips": [],
            "group_name": "default", "auto_groups": [], "cross_group_retry": False,
            "account_balance_read": False}],
        "funding_rules": [{"position": 0, "payer_account_id": 41}],
        "budgets": [{**common, "issuer_user_id": 31, "limit_credits": 100,
                     "display_limit_credits": None, "unlimited": False}],
        "opening": [{**common, "start_at": E.C.QUOTA.LIFETIME_START,
                     "end_at": E.C.QUOTA.LIFETIME_END, "consumed_credits": 40,
                     "import_reference": "key-quota:synthetic-keys:21"}]}]}


class KeyObservationTests(unittest.TestCase):
    def test_committed_bindings(self):
        self.assertEqual(E.validate_bindings(*fixtures())[0]["target_credential_id"], 21)

    def test_refuses_unknown_changed_alias_and_duplicate(self):
        for change in ("unknown", "alias", "duplicate", "boolean"):
            receipt, alias, audit, raw, alias_raw = fixtures()
            if change == "unknown":
                audit["status"] = "UNKNOWN"
            elif change == "alias":
                alias["keys"][0]["target_account_id"] = 99
            elif change == "duplicate":
                receipt["records"] *= 2
            else:
                receipt["records"][0]["source_key_id"] = True
            with self.subTest(change=change), self.assertRaises(E.H.MappingError):
                E.validate_bindings(receipt, alias, audit, raw, alias_raw)

    def test_projects_actual_signed_opening_and_native_restrictions(self):
        o = observation()
        o["records"][0]["opening"][0]["consumed_credits"] = 120
        rows = E.classify(o, fixtures()[0]["records"])
        self.assertEqual(rows[0]["opening_remaining_credits"], -20)
        self.assertFalse(rows[0]["settings"][0]["enabled"])
        self.assertNotIn("status", rows[0])
        self.assertNotIn("deleted_at", rows[0])

    def test_unlimited_display_basis(self):
        o = observation()
        o["records"][0]["budgets"][0].update(unlimited=True, limit_credits=0,
                                                  display_limit_credits=7)
        self.assertEqual(E.classify(o, fixtures()[0]["records"])[0]["opening_remaining_credits"], -33)

    def test_incomplete_foreign_duplicate_or_wrong_owner_refused(self):
        for change in ("missing", "extra", "foreign", "owner", "receipt", "settings", "budget", "secret"):
            o = observation()
            if change == "missing":
                o["records"] = []
            elif change == "extra":
                o["records"] *= 2
            elif change == "foreign":
                o["records"][0]["target_key_id"] = 22
            elif change == "owner":
                o["records"][0]["owner_account_id"] = 42
            elif change == "receipt":
                o["receipt_matches"] = False
            elif change == "settings":
                o["records"][0]["settings"] = []
            elif change == "budget":
                o["records"][0]["budgets"][0]["period"] = "daily"
            else:
                o["records"][0]["secret_sha256"] = "f" * 64
            with self.subTest(change=change), self.assertRaises(E.H.MappingError):
                E.classify(o, fixtures()[0]["records"])

    def test_unprojected_values_are_rejected(self):
        for section in (None, "settings", "budgets", "opening"):
            o = observation()
            row = o["records"][0] if section is None else o["records"][0][section][0]
            row["encrypted_secret"] = "synthetic-forbidden"
            with self.subTest(section=section), self.assertRaises(E.H.MappingError):
                E.classify(o, fixtures()[0]["records"])

    def test_opening_reference_is_bound_to_committed_operation(self):
        o = observation()
        with self.assertRaises(E.H.MappingError):
            E.classify(o, fixtures()[0]["records"], "other-operation")

    def test_query_projects_no_secret_cipher_or_raw_material(self):
        sql = E.build_sql(fixtures()[0], "synthetic-keys", {
            "catalog_query": "SELECT 'synthetic'", "catalog_query_sha256": "a" * 64})
        for forbidden in ("encrypted_secret", "to_jsonb(c)", "to_jsonb(s)", "SELECT * FROM core_identity"):
            self.assertNotIn(forbidden, sql)
        self.assertIn("REPEATABLE READ READ ONLY", sql)
        self.assertIn("ROLLBACK;", sql)
        self.assertIn("WHERE c.kind='api_key'", sql)
        self.assertNotIn("'material',", sql)



@unittest.skipUnless(os.environ.get("LMM_KEY_OBSERVATION_PG_TEST") == "1",
                     "isolated native observation PostgreSQL test not enabled")
class KeyObservationPostgreSQLTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from test_legacy_credential_import_pg import CredentialImportPostgreSQLTests
        class ShortSocketHarness(CredentialImportPostgreSQLTests):
            @classmethod
            def command(harness, argv, **kwargs):
                if argv[0] == "pg_ctl" and argv[-1] == "start":
                    short_socket = Path(tempfile.mkdtemp(prefix="keyobs-", dir="/tmp"))
                    harness.addClassCleanup(short_socket.rmdir)
                    argv = list(argv)
                    option = argv.index("-o") + 1
                    argv[option] = argv[option].replace(str(harness.socket), str(short_socket))
                    harness.socket = short_socket
                return super().command(argv, **kwargs)
        cls.harness = ShortSocketHarness
        cls.harness.setUpClass()
        cls.addClassCleanup(cls.harness.doClassCleanups)

    def test_actual_sql_after_committed_identity_financial_and_key_import(self):
        h = self.harness(methodName="runTest")
        # The shared fixture still names the earlier seed constant. Select the
        # current reviewed profile for this fixture without editing its source.
        with patch.object(E.H, "CORE_SEED_CONTRACT", E.H.reviewed_profile()):
            h.setUp()
        self.assertEqual(h.query(h.apply)["status"], "COMMITTED")
        receipt = h.material["credential_receipt"]
        sql = E.build_sql(receipt, h.material["migration_id"], E.H.current_contract())
        before = h.full_state()
        observed = h.query(sql)
        rows = E.classify(observed, receipt["records"], h.material["migration_id"])
        self.assertEqual(len(rows), len(receipt["records"]))
        self.assertEqual(observed["read_only"], "on")
        self.assertEqual(observed["isolation"], "repeatable read")
        self.assertEqual(before, h.full_state())
        self.assertEqual(observed["components"], E.H.current_contract()["component_fingerprints"])
        self.assertEqual(rows[0]["secret_sha256"], receipt["records"][0]["source_secret_sha256"])
        self.assertNotIn("encrypted_secret", E.F.canonical(observed))
        self.assertTrue(all(type(row["user_active"]) is bool
                            and type(row["account_active"]) is bool for row in rows))
        h.write(h.db + "-native-key-observation.json", E.F.canonical(observed))
        # Use the real committed-artifact publisher and private libpq inputs.
        outcome, committed = h.execute_attempt("observation-entrypoint")
        self.assertEqual(outcome["status"], "COMMITTED")
        connection = {"socket_directory": str(h.socket), "port": 55474,
            "database": h.db, "role": h.owner, "login_role": h.operator,
            "set_role": h.owner, "pgpass_file": str(committed / "pgpass.private")}
        contract = E.H.current_contract()
        target = {"application_or_worker_started": False, "connection": connection,
            "instance_identity": observed["instance"],
            "contract": {**contract, "catalog_fingerprint_sha256": observed["catalog"]}}
        target_path = h.write(h.db + "-observation-target.private.json", E.F.canonical(target))
        args = Namespace(target_receipt=target_path,
            credential_receipt=committed / E.C.MAPPING_FILE,
            alias_manifest=committed / E.C.ALIAS_FILE,
            commit_audit=committed / E.C.ARTIFACT_AUDIT_FILE,
            output=h.output / (h.db + "-entrypoint-observation"))
        entry_before = h.full_state()
        result = E.run(args)
        self.assertEqual(result, {"status": "COMPLETE_SCOPED_OBSERVATION",
                                  "record_count": len(rows)})
        exported_path = args.output / "target-key-observation.private.json"
        self.assertEqual(stat.S_IMODE(args.output.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(exported_path.stat().st_mode), 0o600)
        self.assertEqual(list(args.output.iterdir()), [exported_path])
        exported = json.loads(exported_path.read_text())
        self.assertEqual(exported["records"], rows)
        self.assertEqual(exported["instance"], observed["instance"])
        self.assertEqual(exported["input_sha256"]["pgpass"],
                         E.F.sha((committed / "pgpass.private").read_bytes()))
        self.assertTrue(exported["committed_receipt_mapping_verified"])
        self.assertFalse(exported["audit_material_digest_verified"])
        self.assertFalse(exported["legacy_inventory_created"])
        self.assertEqual(entry_before, h.full_state())
        self.assertNotIn("encrypted_secret", exported_path.read_text())
        for source_token in json.loads(Path(h.files["source"]["path"]).read_text())["tokens"]:
            self.assertNotIn(source_token["key"], exported_path.read_text())
        h.write(h.db + "-entrypoint-result.json", E.F.canonical(result))
        wrong = E.build_sql(receipt, "missing-operation", E.H.current_contract())
        with self.assertRaises(E.H.MappingError):
            E.classify(h.query(wrong), receipt["records"])
        h.sql("UPDATE core_identity.api_key_settings SET enabled=false")
        changed = E.classify(h.query(sql), receipt["records"], h.material["migration_id"])
        self.assertTrue(all(row["settings"][0]["enabled"] is False for row in changed))


if __name__ == "__main__":
    unittest.main()
