"""Pure regression for the private used-target adapter; no database connections.

All owner/source values below are invented. The old review has valid bindings
only for mapper tests; it is deliberately never accepted as current economic
authority by the production entry point.
"""
from copy import deepcopy
import hashlib
import json
import unittest
from unittest.mock import patch

import migrate_current_runtime_keys as M


def token(**changes):
    return {"id": 17, "user_id": 37, "key": "SyntheticPrivateCredentialBytes1234567890",
            "status": 1, "name": "Synthetic fixture", "created_time": 100,
            "accessed_time": 200, "expired_time": -1, "remain_quota": 30,
            "unlimited_quota": False, "one_time_reveal": True, "account_balance_read": True,
            "model_limits_enabled": False, "model_limits": "", "allow_ips": None,
            "used_quota": 70, "group": "paid", "cross_group_retry": False,
            "auto_groups": "", "oauth_managed": False, "creation_source": "manual",
            "deleted_at": None, **changes}


def inputs(tokens=None):
    mapping = [{"source_user_id": 37, "target_user_id": 37, "target_personal_account_id": 8}]
    source = {"format_version": 1, "source_revision": M.P.LEGACY_REVISION,
              "source_archive_sha256": "a" * 64, "source_schema_sha256": "b" * 64,
              "source_export_sha256": "c" * 64, "source_projection_sha256": "d" * 64,
              "snapshot_at": 1000, "tokens": [token()] if tokens is None else tokens,
              "users": [{"id": 37, "status": 1, "deleted_at": None, "auth_version": 5,
                         "group": "paid", "billing_preference": "wallet_first"}],
              "policy": {"auto_groups": ["paid", "backup"], "max_token_auto_groups": 4,
                         "user_model_rate_limit_enabled": False, "source_options_sha256": "e" * 64,
                         "group_authority": {"user_usable_groups": {"paid": "Paid", "backup": "Backup", "auto": "Auto"},
                           "special_usable_groups": {}, "group_ratios": {"paid": 1, "backup": 1}, "group_group_ratios": {}}}}
    target = {"format_version": 1, "source_archive_sha256": "a" * 64,
              "source_target_id_map": mapping,
              "source_target_id_map_sha256": hashlib.sha256(M.P.id_map_bytes(mapping).encode()).hexdigest(),
              "users": [{"user_id": 37, "personal_account_id": 8, "auth_version": 5,
                         "active": True, "developer_access_granted": True, "routing_groups": ["paid", "backup"]}]}
    return source, target


def review(source, target):
    proposal = M.P.plan_api_keys(source, target)
    return {"format_version": 1, "scope": M.C.SCOPE,
            "decision": "approve-offline-native-equivalent-credentials",
            "source_input_sha256": M.P.sha256(source), "target_input_sha256": M.P.sha256(target),
            "proposal_sha256": proposal["proposal_sha256"], "source_frozen": True,
            "source_unit": "source_quota_integer", "native_unit": "credit_500k_usd",
            "numerator": 1, "denominator": 1, "evidence_sha256": "1" * 64,
            "economic_review_sha256": "2" * 64}


def synthetic_contract():
    # Unit-only shape fixture. The PG suite instead reads the real existing
    # Core78 binary manifest through M.core78_contract's pinned SHA256 guard.
    return {"manifest_sha256": M.CORE78_MANIFEST_SHA256,
            "catalog_fingerprint_sha256": M.CORE78_CATALOG_SHA256,
            "component_fingerprints": {"synthetic_" + str(i): "a" * 64 for i in range(78)},
            "catalog_query": "SELECT 'unit fixture only'::text AS definition",
            "identity_version": 24, "identity_fingerprint_sha256": "7" * 64}


def snapshot(target, contract=None):
    contract = synthetic_contract() if contract is None else contract
    tables = sorted(set(M.TABLES) | {"core_migration.import_runs", "core_identity.users",
        "core_meta.component_contract", "core_meta.schema_contract", "core_meta.runtime_contract",
        "core_billing.ledger_entries", "core_charging.subscriptions"})
    sequences = sorted(set(M.SEQUENCES) | {"core_identity.accounts_id_seq"})
    seq = {"last_value": 17, "is_called": True, "start": 1, "increment": 1,
           "min": 1, "max": M.Q.BIGINT_MAX, "cache": 1, "cycle": False}
    state = {"inventory": {"tables": tables, "sequences": sequences},
             "tables": {t: {"rows": 1, "sha256": "3" * 64} for t in tables},
             "sequences": {s: deepcopy(seq) for s in sequences}}
    return {"instance": {"database": "lmm_runtime_keys_test_unit", "role": "synthetic_owner",
                "session_user": "synthetic_installer", "effective_role_can_login": False,
                "database_owner": "synthetic_owner", "database_oid": "103", "system_identifier": "123456789",
                "server_instance_nonce": "5" * 64},
            "catalog_fingerprint_sha256": M.CORE78_CATALOG_SHA256, "runtime_catalog": M.CORE78_CATALOG_SHA256,
            "component_fingerprints": contract["component_fingerprints"], "ownership_sha256": "4" * 64,
            "identity_contract": {"version": contract["identity_version"], "fingerprint": contract["identity_fingerprint_sha256"]},
            "native_users": deepcopy(target["users"]), "state": state,
            "max_ids": {"credentials": 17, "budgets": 17}, "key_imports": []}


NAMESPACE = {"system_identifier": "9988776655", "database": "synthetic_source", "schema": "synthetic_public"}


def material(tokens=None, *, source=None, target=None, snap=None, encryption_key=None, migration_id="synthetic-used78-keys"):
    if source is None:
        source, target = inputs(tokens)
    contract = synthetic_contract()
    snap = snapshot(target, contract) if snap is None else snap
    return M.prepare_mechanics(source, target, review(source, target), snapshot=snap, contract=contract,
        migration_id=migration_id, source_namespace=NAMESPACE, runtime_roles=["synthetic_runtime"],
        encryption_key=encryption_key)


class MapperAndPreservationTests(unittest.TestCase):
    def test_pg_scalar_parser_handles_native_booleans_without_changing_json(self):
        from test_migrate_current_runtime_keys_pg import parse_single_result
        for text, value in (("t\n", True), ("f\n", False), ("true\n", True), ("false\n", False),
                            ("42\n", 42), ('"t"\n', "t"), ("null\n", None),
                            ('{"ok":true,"ids":[1,2]}\n', {"ok": True, "ids": [1, 2]})):
            with self.subTest(text=text):
                self.assertEqual(parse_single_result(text), value)
        for text in ("", "t\nf\n", "1\n2\n"):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                parse_single_result(text)
        with self.assertRaises(json.JSONDecodeError):
            parse_single_result("T\n")

    def test_pg_fixture_default_group_preserves_legacy_inheritance(self):
        from test_migrate_current_runtime_keys_pg import Regression
        fixture = Regression.__new__(Regression)
        fixture.session_ids = [1, 2, 3, 4]
        fixture.group = "default"
        fixture.owner = {"user_id": 37, "personal_account_id": 8, "auth_version": 5,
                         "active": True, "developer_access_granted": True, "routing_groups": ["default"]}
        fixture.query = lambda sql: [deepcopy(fixture.owner)]
        source, target = fixture.source_target()
        self.assertEqual([r["group"] for r in source["tokens"]], ["", "", "", ""])
        proposal = M.P.plan_api_keys(source, target)
        mapped = M.C._supported_rows(proposal, target, 10)
        self.assertTrue(all(r["native_effective_group"] == "default" for r in mapped))

    def test_colliding_legacy_id_is_remapped_without_touching_old_state(self):
        s, t = inputs()
        before = snapshot(t)
        frozen = deepcopy(before)
        m = material(source=s, target=t, snap=before)
        self.assertEqual(m["records"][0]["source_token_id"], 17)
        self.assertEqual(m["records"][0]["target_credential_id"], 18)
        self.assertEqual(m["quota_plan"]["key_records"][0]["target_credential_id"], 18)
        self.assertEqual(before, frozen)
        self.assertEqual(m["before_state"], frozen["state"])
        self.assertFalse(m["production_authorized"])

    def test_disabled_expired_and_exhausted_states_survive_shared_mapper(self):
        for changes, enabled in (({"status": 2}, False), ({"status": 3, "expired_time": 500}, True),
                                 ({"status": 4, "remain_quota": 0}, True)):
            with self.subTest(changes=changes):
                m = material([token(**changes)])
                r = m["records"][0]
                self.assertEqual(r["enabled"], enabled)
                self.assertEqual(r["quota"]["opening_consumed"], 70)
                self.assertEqual(r["quota"]["remaining"], changes.get("remain_quota", 30))

    def test_unlimited_signed_display_and_consumption_are_conserved(self):
        m = material([token(unlimited_quota=True, remain_quota=-15)])
        r = m["records"][0]
        self.assertTrue(r["quota"]["unlimited"])
        self.assertEqual(r["quota"]["display_limit_credits"] - r["quota"]["opening_consumed"], -15)
        self.assertEqual(m["quota_plan"]["key_records"][0]["remaining_units"], -15)

    def test_recovery_ciphertext_binds_new_native_id_not_old_source_id(self):
        raw = token(one_time_reveal=False)
        key = bytes(range(32))
        m = material([raw], encryption_key=key)
        row = m["records"][0]
        M.C.verify_encrypted_secret(key, row, raw["key"])
        wrong = {**row, "target_credential_id": row["source_token_id"]}
        with self.assertRaises(M.C.Error):
            M.C.verify_encrypted_secret(key, wrong, raw["key"])
        self.assertNotIn(raw["key"], M.canonical(m))

    def test_auth_generation_and_group_permission_are_not_silently_repaired(self):
        for mutation in ("auth", "group", "developer"):
            s, t = inputs()
            if mutation == "auth":
                t["users"][0]["auth_version"] += 1
            elif mutation == "group":
                t["users"][0]["routing_groups"] = []
            else:
                t["users"][0]["developer_access_granted"] = False
            with self.subTest(mutation=mutation), self.assertRaises((M.Error, M.C.Error, M.P.ProposalError)):
                material(source=s, target=t)

    def test_namespace_and_source_key_prevent_duplicate_new_archive_batch(self):
        m = material()
        receipt = {"manifest": {"scope": M.SCOPE, "source_namespace_sha256": m["source_namespace_sha256"],
                               "mapping": m["credential_receipt"]["records"]}}
        with self.assertRaisesRegex(M.Error, "source_identity_already_imported"):
            M.check_source_identity_conflicts(NAMESPACE, m["records"], [receipt])
        another_site = {**NAMESPACE, "system_identifier": "111222333"}
        M.check_source_identity_conflicts(another_site, m["records"], [receipt])

    def test_old_receipts_without_namespace_require_explicit_reconciliation(self):
        m = material()
        with self.assertRaisesRegex(M.Error, "namespace_reconciliation"):
            M.check_source_identity_conflicts(NAMESPACE, m["records"], [{"manifest": {"scope": M.C.SCOPE}}])

    def test_sequence_gaps_are_allowed_but_backward_or_cached_sequences_are_rejected(self):
        s, t = inputs()
        for change in ("behind", "cache", "cycle"):
            snap = snapshot(t)
            seq = snap["state"]["sequences"][M.SEQUENCES[0]]
            if change == "behind":
                seq["last_value"] = 2
            elif change == "cache":
                seq["cache"] = 10
            else:
                seq["cycle"] = True
            with self.subTest(change=change), self.assertRaises((M.Error, M.C.Error)):
                material(source=s, target=t, snap=snap)
        snap = snapshot(t)
        snap["state"]["sequences"][M.SEQUENCES[0]]["last_value"] = 38
        self.assertEqual(material(source=s, target=t, snap=snap)["records"][0]["target_credential_id"], 39)

    def test_quota_or_mapping_mutation_rejected_before_sql_generation(self):
        m = material()
        m["quota_plan"]["key_records"][0]["used_units"] -= 1
        with self.assertRaises((M.Error, ValueError, M.Q.IDENTITY.MappingError)):
            M.build_synthetic_transaction_sql(m, "e" * 64)
        m = material()
        m["records"][0]["target_credential_id"] = 17
        with self.assertRaises((M.Error, ValueError, M.Q.IDENTITY.MappingError)):
            M.build_synthetic_transaction_sql(m, "e" * 64)

    def test_exact_core78_required_and_stale_native_projection_rejected(self):
        s, t = inputs()
        snap = snapshot(t)
        snap["component_fingerprints"].pop(next(iter(snap["component_fingerprints"])))
        with self.assertRaisesRegex(M.Error, "not_exact_core78"):
            material(source=s, target=t, snap=snap)
        snap = snapshot(t)
        snap["native_users"][0]["active"] = False
        with self.assertRaisesRegex(M.Error, "projection_stale"):
            material(source=s, target=t, snap=snap)

    def test_fake_approval_or_partial_receipt_cannot_reach_transaction_generator(self):
        m = material()
        for fake in ("approved", {"decision": "approved", "sha256": "a" * 64},
                     {"manifest": {"scope": "legacy-available-wallet-v1"}}, m["source_review"]):
            with self.subTest(fake=type(fake).__name__), patch.object(M, "_mechanics_transaction_sql") as generator:
                with self.assertRaisesRegex(M.Error, M.BLOCKER):
                    M.build_production_transaction_sql(m, fake)
                generator.assert_not_called()

    def test_fixture_entry_cannot_be_pointed_at_an_unmarked_database(self):
        m = material()
        m["snapshot"]["instance"]["database"] = "lmm_production"
        with self.assertRaisesRegex(M.Error, "synthetic_database_required"):
            M.build_synthetic_transaction_sql(m, "e" * 64)

    def test_duplicate_source_rows_and_bad_review_units_use_existing_validation(self):
        with self.assertRaises((M.Error, M.C.Error, M.P.ProposalError)):
            material([token(), token()])
        s, t = inputs()
        r = review(s, t)
        r["denominator"] = 500000
        with self.assertRaisesRegex(M.C.Error, "unit_conversion_unsupported"):
            M.prepare_mechanics(s, t, r, snapshot=snapshot(t), contract=synthetic_contract(),
                migration_id="synthetic-used78-unit", source_namespace=NAMESPACE,
                runtime_roles=["synthetic_runtime"])

    def test_existing_readback_helper_interface_is_reused(self):
        # Generate the field-level SQL once to detect a helper interface change.
        # Real semantics (triggers, preservation, rollback, replay) are tested
        # against PostgreSQL by the separate suite, not by string comparisons.
        self.assertTrue(M.build_synthetic_transaction_sql(material(), "e" * 64))
        self.assertTrue(M.build_resolve_sql(material(), "lmm-current-keys-" + "f" * 32))


if __name__ == "__main__":
    unittest.main()
