"""Synthetic offline producer contracts; real PostgreSQL acceptance is separate."""
from __future__ import annotations

from copy import deepcopy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import legacy_credential_import as IMPORT
from test_legacy_api_key_migration import inputs, token, SYNTHETIC_SECRET


def fixture(*, changes=None, owner_inactive=False):
    source, target = inputs([token(**(changes or {}))])
    if owner_inactive:
        source["users"][0]["status"] = 2
        target["users"][0]["active"] = False
    proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
    economic_review = {"format_version": 1, "scope": IMPORT.FIN.SCOPE, "decision": "approved-offline-opening",
        "source_archive_sha256": source["source_archive_sha256"], "mapping_sha256": "d" * 64,
        "identity_map_sha256": "9" * 64, "credits_per_usd": 500000, "evidence_sha256": "e" * 64,
        "other_rights_policy": "remain-disabled-pending-separate-import"}
    review = {"format_version": 1, "scope": IMPORT.SCOPE,
        "decision": "approve-offline-native-equivalent-credentials",
        "source_input_sha256": IMPORT.PROPOSAL.sha256(source),
        "target_input_sha256": IMPORT.PROPOSAL.sha256(target),
        "proposal_sha256": proposal["proposal_sha256"], "source_frozen": True,
        "source_unit": "source_quota_integer", "native_unit": "credit_500k_usd",
        "numerator": 1, "denominator": 1, "evidence_sha256": "e" * 64,
        "economic_review_sha256": IMPORT.digest(economic_review)}
    tables = sorted(set(IMPORT.FIN.CHANGED_TABLES) | set(IMPORT.FIN.fresh_empty_tables()) |
        set(IMPORT.CHANGED_TABLES) | {"core_migration.import_runs", "core_identity.users", "core_trust_pricing.policies"})
    sequences = sorted(set(IMPORT.FIN.CHANGED_SEQUENCES) | set(IMPORT.CHANGED_SEQUENCES))
    state = {"inventory": {"tables": tables, "sequences": sequences},
        "tables": {name: {"rows": 0, "sha256": IMPORT.FIN.sha(b"")} for name in tables},
        "sequences": {name: {"last_value": 201, "is_called": False, "start": 1, "increment": 1,
            "min": 1, "max": 9223372036854775807, "cache": 1, "cycle": False} for name in sequences}}
    contract = IMPORT.IDENTITY.current_contract()
    identity = {"migration_id": "synthetic-identity-import", "source_archive_sha256": source["source_archive_sha256"],
        "mapping_sha256": "e" * 64, "manifest": {"scope": "identity-only", "source_archive_sha256": source["source_archive_sha256"],
            "source_target_id_map_sha256": target["source_target_id_map_sha256"]}}
    snapshot = {"instance": {"database": "synthetic_credential_import", "role": "synthetic_owner",
        "session_user": "synthetic_installer", "database_owner": "synthetic_owner", "database_oid": 13,
        "effective_role_can_login": False, "tcp_connection": False, "listen_addresses": "",
        "port": 5432, "postmaster_started_at": "2026-10-26 01:00:00+00", "system_identifier": "123456789",
        "server_instance_nonce": "f" * 64, "server_version_num": "180000"},
        "catalog_fingerprint_sha256": "c" * 64, "ownership_sha256": "d" * 64,
        "component_fingerprints": contract["component_fingerprints"], "trust_policy_seed": {
            "singleton": True, "revision": 1, "spec": IMPORT.IDENTITY.DEFAULT_TRUST_POLICY,
            "source_digest": hashlib.sha256(b"core-native-trust-policy-v1").hexdigest(),
            "native_user_floor": 1, "source_cutoff": 1000, "native_since": "2026-10-26T00:00:00+00:00"},
        "state": state, "native_users": target["users"], "identity_import": identity, "financial_import": {
            "migration_id": "synthetic-financial-opening", "source_archive_sha256": source["source_archive_sha256"],
            "mapping_sha256": "d" * 64, "manifest": {"scope": IMPORT.FIN.SCOPE, "material": {
                "identity_import": identity, "source_archive_sha256": source["source_archive_sha256"],
                "mapping_sha256": "d" * 64, "identity_map_sha256": "9" * 64,
                "source_review": economic_review, "source_files": {"source_export": {"sha256": source["source_export_sha256"]}}}}}}
    files = {name: {"path": "/home/lightjunction/.local/state/lmm-api/synthetic-credential-test/" + name,
                    "sha256": "a" * 64} for name in IMPORT.FILES}
    if not source["tokens"][0]["one_time_reveal"]:
        files["encryption_key"] = {"path": "/home/lightjunction/.local/state/lmm-api/synthetic-credential-test/key", "sha256": "b" * 64}
    return source, target, proposal, review, snapshot, files


def material_fixture(**kwargs):
    source, target, proposal, review, snapshot, files = fixture(**kwargs)
    return IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-credential-import",
        snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files, encryption_key=bytes(range(32)))


class ActivationLineageWriterTests(unittest.TestCase):
    def records(self, source, target, **kwargs):
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        return IMPORT._supported_rows(proposal, target, 201, **kwargs)

    def test_writer_issues_generation7_not_source6_without_revoking_valid_key(self):
        from test_legacy_api_key_migration import activated_inputs
        source, target = activated_inputs()
        row = self.records(source, target)[0]
        self.assertEqual((row["source_auth_version"], row["target_auth_version"]), (6, 7))
        self.assertFalse(row["revoked"])
        self.assertTrue(row["enabled"])
        self.assertNotIn("owner_auth_version_changed", row["blocked_reasons"])

    def test_revoke_unsupported_cannot_hide_unproved_version_drift(self):
        from test_legacy_api_key_migration import activated_inputs
        source, target = activated_inputs()
        del target[IMPORT.LINEAGE.KEY]
        with self.assertRaisesRegex(IMPORT.Error, "credential_owner_auth_version_changed"):
            self.records(source, target, revoke_unsupported=True)

    def test_writer_recomputes_lineage_audit_and_keeps_source_denied_inert(self):
        from test_legacy_api_key_migration import activated_inputs
        source, target = activated_inputs()
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        for field, value in (("expected_target_auth_version", 6), ("source_auth_version", 7),
                             ("auth_version_binding_sha256", "0" * 64), ("target_auth_version", 6)):
            bad = deepcopy(proposal)
            bad["records"][0][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(IMPORT.Error, "credential_owner_auth_version_changed"):
                IMPORT._supported_rows(bad, target, 201)
        source, target = activated_inputs([token(group="default")])
        source["users"][0]["group"] = "default"
        target["users"][0]["routing_groups"].append("default")
        with self.assertRaisesRegex(IMPORT.Error, "credential_native_semantics_unsupported"):
            self.records(source, target)
        row = self.records(source, target, revoke_unsupported=True)[0]
        self.assertTrue(row["revoked"])
        self.assertFalse(row["enabled"])
        self.assertIn("source_token_group_deprecated", row["blocked_reasons"])

    def test_sql_guard_keeps_existing_guards_and_adds_durable_recheck(self):
        from test_legacy_api_key_migration import activated_inputs
        from test_financial_import import trust_seed_fixture
        _, target = activated_inputs()
        material = {"target_projection": target, "target": {"trust_policy_seed": trust_seed_fixture()}}
        with patch.object(IMPORT.FIN, "_guard", return_value="-- existing authority fences\n"):
            sql = IMPORT._guard(material)
        self.assertIn("-- existing authority fences", sql)
        self.assertIn("admin_activation_durable_evidence_changed", sql)
        self.assertIn("credential_authority_or_financial_opening_changed", sql)
        self.assertIn("FOR SHARE OF u,a,p,l", sql)

    def test_synthetic_eleven_valid_keys_remain_live_denied_and_managed_stay_inert(self):
        from test_legacy_api_key_migration import activated_inputs
        tokens = [token(id=17+i, key=SYNTHETIC_SECRET + str(i)) for i in range(11)]
        tokens += [token(id=28, key=SYNTHETIC_SECRET + "denied", group="default"),
                   token(id=29, key=SYNTHETIC_SECRET + "managed", oauth_managed=True)]
        source, target = activated_inputs(tokens)
        source["users"][0]["group"] = "default"
        target["users"][0]["routing_groups"].append("default")
        rows = self.records(source, target, revoke_unsupported=True)
        self.assertEqual(sum(r["enabled"] and not r["revoked"] for r in rows), 11)
        self.assertTrue(all(r["target_auth_version"] == 7 and r["source_auth_version"] == 6 for r in rows))
        self.assertTrue(all(r["revoked"] and not r["enabled"] for r in rows[-2:]))
        self.assertIn("source_token_group_deprecated", rows[-2]["blocked_reasons"])
        self.assertIn("managed_credential_requires_separate_migration", rows[-1]["blocked_reasons"])

    def test_before_activation_live_source_keys_cannot_be_revoked_while_owner_dormant(self):
        from test_legacy_api_key_migration import activated_inputs
        source, target = activated_inputs()
        del target[IMPORT.LINEAGE.KEY]
        target["users"][0].update(auth_version=6, active=False)
        with self.assertRaisesRegex(IMPORT.Error, "credential_live_source_key_requires_complete_native_authority"):
            self.records(source, target, revoke_unsupported=True)

    def test_valid_root_keys_cannot_be_revoked_to_hide_missing_dev_or_routing(self):
        from test_legacy_api_key_migration import activated_inputs
        for field, value in (("developer_access_granted", False), ("routing_groups", [])):
            source, target = activated_inputs()
            target["users"][0][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(IMPORT.Error,
                    "credential_live_source_key_requires_complete_native_authority"):
                self.records(source, target, revoke_unsupported=True)


class ProducerTests(unittest.TestCase):
    def test_native_default_does_not_revive_source_explicit_default(self):
        source, target = inputs([token(group="default")])
        source["users"][0]["group"] = "default"
        target["users"][0]["routing_groups"].append("default")
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        self.assertIn("source_token_group_deprecated", proposal["records"][0]["blocked_reasons"])
        # The writer independently rebuilds source denial after reason removal.
        proposal["records"][0]["blocked_reasons"] = []
        with self.assertRaisesRegex(IMPORT.Error, "credential_native_semantics_unsupported"):
            IMPORT._supported_rows(proposal, target, 201)
        row = IMPORT._supported_rows(proposal, target, 201, revoke_unsupported=True)[0]
        self.assertTrue(row["revoked"])
        self.assertFalse(row["enabled"])
        self.assertIn("source_token_group_deprecated", row["blocked_reasons"])
        self.assertFalse(row["source_group_authority"]["allowed"])

    def test_default_snapshot_sql_uses_native_group_without_overwriting_raw_group(self):
        source, target, _, review, snapshot, files = fixture(changes={"group": ""})
        source["users"][0]["group"] = "default"
        target["users"][0]["routing_groups"].append("default")
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        review.update(source_input_sha256=IMPORT.PROPOSAL.sha256(source),
                      target_input_sha256=IMPORT.PROPOSAL.sha256(target), proposal_sha256=proposal["proposal_sha256"])
        material = IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-default-snapshot",
            snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        row = material["records"][0]
        self.assertEqual(row["group"], {"mode": "dynamic_user", "value": ""})
        self.assertEqual(row["native_effective_group"], "default")
        self.assertFalse(row["revoked"])
        self.assertTrue(row["enabled"])
        sql = IMPORT.build_apply_sql(material)
        self.assertIn("s.group_name='default'", sql)
        self.assertIn("::inet[],'default',ARRAY[", sql)
        self.assertNotIn("INSERT INTO core_identity.user_routing_groups", sql)
        self.assertNotIn("UPDATE core_identity.users", sql)

    def test_writer_rejects_source_group_audit_and_native_group_tampering(self):
        _, target, proposal, _, _, _ = fixture(changes={"group": ""})
        for field, value in (("policy_sha256", "0" * 64), ("user_snapshot_sha256", "0" * 64),
                             ("mapping_sha256", "0" * 64), ("raw_group", "paid"), ("allowed", False)):
            bad = deepcopy(proposal)
            bad["records"][0]["source_group_authority"][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(IMPORT.Error, "credential_source_group_audit_mismatch"):
                IMPORT._supported_rows(bad, target, 201)
        bad = deepcopy(proposal)
        bad["records"][0]["native_effective_group"] = "backup"
        with self.assertRaisesRegex(IMPORT.Error, "credential_source_group_audit_mismatch"):
            IMPORT._supported_rows(bad, target, 201)
        bad = deepcopy(proposal)
        bad["source_policy"]["group_authority"]["user_usable_groups"]["retired"] = "Retired"
        with self.assertRaisesRegex(IMPORT.Error, "credential_source_policy_hash_mismatch"):
            IMPORT._supported_rows(bad, target, 201)

    def test_reviewed_origins_preserve_audit_time_and_distinct_owner_mapping(self):
        for origin in ("assistant", "drawing_mcp"):
            with self.subTest(origin=origin):
                material = material_fixture(changes={"creation_source": origin, "created_time": 123})
                row = material["records"][0]
                self.assertEqual(row["creation_source"], origin)
                self.assertEqual(row["created_time"], 123)
                self.assertEqual(material["proposal"]["records"][0]["creation_source"], origin)
                self.assertEqual((row["target_user_id"], row["target_personal_account_id"]), (37, 8))
                self.assertFalse(row.get("revoked_rekey_required", False))
                sql = IMPORT.build_apply_sql(material)
                self.assertIn("to_timestamp(123),to_timestamp(123)", sql)
                self.assertIn("INSERT INTO core_migration.import_runs", sql)
                self.assertNotIn("UPDATE core_identity.users", sql)
                self.assertNotIn("INSERT INTO core_identity.developer_access", sql)
                self.assertNotIn("INSERT INTO core_identity.user_routing_groups", sql)

    def test_reviewed_origins_leave_independent_authority_and_state_guards(self):
        for origin in ("assistant", "drawing_mcp"):
            for changes in ({"oauth_managed": True}, {"group": "retired"}, {"group": "auto"},
                            {"model_limits_enabled": True, "model_limits": "model-a"}):
                with self.subTest(origin=origin, changes=changes):
                    with self.assertRaisesRegex(IMPORT.Error, "credential_native_semantics_unsupported"):
                        material_fixture(changes={"creation_source": origin, **changes})
            for field, value in (("active", False), ("developer_access_granted", False),
                                 ("routing_groups", [])):
                with self.subTest(origin=origin, target_guard=field):
                    source, target = inputs([token(creation_source=origin)])
                    target["users"][0][field] = value
                    proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
                    with self.assertRaisesRegex(IMPORT.Error, "credential_native_semantics_unsupported"):
                        IMPORT._supported_rows(proposal, target, 201)
            material = material_fixture(changes={"creation_source": origin, "status": 2,
                "deleted_at": "2026-10-26T00:00:00Z"})
            self.assertFalse(material["records"][0]["enabled"])
            self.assertTrue(material["records"][0]["revoked"])

    def test_writer_independently_rejects_unreviewed_origin_without_mapper_block(self):
        for origin in ("system", "red_packet_cover", "assistant_runtime", "future-plugin"):
            with self.subTest(origin=origin):
                _, target, proposal, _, _, _ = fixture()
                proposal["records"][0]["creation_source"] = origin
                proposal["records"][0]["blocked_reasons"] = []
                with self.assertRaisesRegex(IMPORT.Error, "credential_native_semantics_unsupported"):
                    IMPORT._supported_rows(proposal, target, 201)

    def test_atomic_native_credentials_and_quota_keep_original_digest(self):
        material = material_fixture()
        row = material["records"][0]
        self.assertEqual(row["digest_sha256"], hashlib.sha256(SYNTHETIC_SECRET.encode()).hexdigest())
        self.assertEqual((row["target_user_id"], row["target_personal_account_id"], row["target_auth_version"]), (37, 8, 5))
        self.assertEqual(material["credential_receipt"]["scope"], "a02-credential-mapping-receipt")
        self.assertEqual(material["quota_plan"]["key_records"][0]["used_units"], 70)
        self.assertNotIn(SYNTHETIC_SECRET, IMPORT.canonical(material))
        sql = IMPORT.build_apply_sql(material)
        self.assertEqual(sql.count("BEGIN ISOLATION LEVEL SERIALIZABLE;"), 1)
        self.assertEqual(sql.count("COMMIT;"), 1)
        self.assertLess(sql.index("pg_try_advisory_lock(741803121)"), sql.index("BEGIN ISOLATION"))
        self.assertLess(sql.index("INSERT INTO core_identity.credentials"), sql.index("INSERT INTO core_charging.budgets"))
        self.assertLess(sql.index("INSERT INTO core_charging.budget_opening_consumption"), sql.index("UPDATE core_identity.api_key_settings SET enabled=TRUE"))
        self.assertIn("credential_exact_readback_failed", sql)
        self.assertIn("credential_unrelated_state_changed", sql)
        self.assertIn("credential_replay_conflict", sql)
        self.assertNotIn(SYNTHETIC_SECRET, sql)
        self.assertNotIn("setval(", sql)
        self.assertNotIn("nextval(", sql)

    def test_all_generated_psql_scripts_stop_on_the_first_error(self):
        material = material_fixture()
        for sql in (IMPORT.build_apply_sql(material),
                    IMPORT.build_verify_sql(material, "lmm-credential-" + "a" * 32),
                    IMPORT.snapshot_sql(material["before_state"]["inventory"], material["migration_id"])):
            with self.subTest(prefix=sql[:60]):
                self.assertTrue(sql.startswith("\\set ON_ERROR_STOP on\n"))

    def test_missing_routing_grant_requires_fresh_projection_proposal_and_review(self):
        source, target, _, review, snapshot, files = fixture()
        target["users"][0]["routing_groups"] = []
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        review.update(target_input_sha256=IMPORT.PROPOSAL.sha256(target),
                      proposal_sha256=proposal["proposal_sha256"])
        self.assertIn("target_routing_grant_missing", proposal["records"][0]["blocked_reasons"])
        with self.assertRaisesRegex(IMPORT.Error, "credential_native_semantics_unsupported"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)

        # Simulate a fresh observed projection after the original group became
        # effective; reusing either the old proposal or old review must fail.
        target["users"][0]["routing_groups"] = [source["users"][0]["group"]]
        with self.assertRaisesRegex(IMPORT.Error, "credential_proposal_rebuild_mismatch"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        with self.assertRaisesRegex(IMPORT.Error, "credential_review_input_mismatch"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        review.update(target_input_sha256=IMPORT.PROPOSAL.sha256(target),
                      proposal_sha256=proposal["proposal_sha256"])
        material = IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
            snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        sql = IMPORT.build_apply_sql(material)
        self.assertNotIn("INSERT INTO core_identity.user_routing_groups", sql)
        self.assertNotIn("UPDATE core_identity.users", sql)
        self.assertEqual(material["records"][0]["group"]["value"], "paid")

    def test_inclusive_expiry_and_disabled_deleted_history_preserved(self):
        material = material_fixture(changes={"expired_time": 2000, "status": 2,
            "deleted_at": "2026-10-26T00:00:00Z", "accessed_time": 0})
        row = material["records"][0]
        self.assertEqual(row["expiry"]["native_exclusive_epoch"], 2001)
        self.assertTrue(row["revoked"])
        self.assertFalse(row["enabled"])
        sql = IMPORT.build_apply_sql(material)
        self.assertIn("to_timestamp(2001)", sql)
        self.assertIn("'2026-10-26T00:00:00Z'::timestamptz", sql)
        self.assertIn("SET enabled=FALSE", sql)
        self.assertIn("s.accessed_at IS NOT DISTINCT FROM NULL", sql)

    def test_inactive_source_owner_stays_inactive(self):
        material = material_fixture(owner_inactive=True)
        self.assertFalse(material["target_projection"]["users"][0]["active"])
        self.assertIn("target_owner_inactive", material["proposal"]["records"][0]["blocked_reasons"])
        self.assertNotIn("UPDATE core_identity.users", IMPORT.build_apply_sql(material))

    def test_recovery_ciphertext_is_bound_to_final_owner_and_id(self):
        material = material_fixture(changes={"one_time_reveal": False})
        row = material["records"][0]
        IMPORT.verify_encrypted_secret(bytes(range(32)), row, SYNTHETIC_SECRET)
        for field in ("target_credential_id", "target_user_id"):
            changed = deepcopy(row)
            changed[field] += 1
            with self.assertRaisesRegex(IMPORT.Error, "recovery_ciphertext_verification_failed"):
                IMPORT.verify_encrypted_secret(bytes(range(32)), changed, SYNTHETIC_SECRET)
        with self.assertRaisesRegex(IMPORT.Error, "recovery_ciphertext_verification_failed"):
            IMPORT.verify_encrypted_secret(bytes([43]) * 32, row, SYNTHETIC_SECRET)
        changed = deepcopy(row)
        changed["encrypted_secret_hex"] = "00" + row["encrypted_secret_hex"][2:]
        with self.assertRaisesRegex(IMPORT.Error, "recovery_ciphertext_verification_failed"):
            IMPORT.verify_encrypted_secret(bytes(range(32)), changed, SYNTHETIC_SECRET)

    def test_core_format_fixed_fixture_for_independent_rust_decrypt(self):
        with patch.object(IMPORT.os, "urandom", return_value=bytes(range(12))):
            encrypted = IMPORT.encrypt_secret(bytes(range(32)), 201, 37, SYNTHETIC_SECRET)
        self.assertEqual(encrypted, "000102030405060708090a0b72922ff6df25d694dc59e1b06ecb9bdf028fa5a681692a92669b0b5bc11d3640360f5902dabe255ed12006180cddd2c433066b62f9ae692fd1")

    def test_recovery_key_missing_rejects_instead_of_disabling_reveal(self):
        source, target, proposal, review, snapshot, files = fixture(changes={"one_time_reveal": False})
        with self.assertRaisesRegex(IMPORT.Error, "credential_recovery_key_required"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)

    def test_managed_retired_auto_and_unproven_matching_reject_whole_batch(self):
        for changes in ({"oauth_managed": True}, {"group": "retired"}, {"group": "auto"},
                        {"model_limits_enabled": True, "model_limits": "model-a"},
                        {"creation_source": "assistant_runtime"}):
            with self.subTest(changes=changes), self.assertRaisesRegex(IMPORT.Error, "credential_native_semantics_unsupported"):
                material_fixture(changes=changes)

    def test_reserved_wire_prefixes_cannot_be_imported_as_ordinary_keys(self):
        for prefix in IMPORT.RESERVED_PREFIXES:
            with self.subTest(prefix=prefix), self.assertRaisesRegex(IMPORT.Error, "credential_ordinary_wire_contract_unsupported"):
                material_fixture(changes={"key": prefix + "a" * 64})

    def test_core_ordinary_ascii_case_and_middle_markers_match_before_prepare(self):
        rejected = ["LegacySyntheticlmm_PrivateKey0123456789012345", "OAUTH_MANAGED_" + "A" * 32,
                    "LMMK_" + "A" * 32, "LegacySyntheticOAUTH_MANAGED_PrivateKey0123456789",
                    "A" * 31, "A" * 257, "A" * 32 + "-", "Ä" + "A" * 31]
        rejected += [prefix.upper() + "A" * 32 for prefix in IMPORT.RESERVED_PREFIXES]
        for raw in rejected:
            self.assertFalse(IMPORT.ordinary_secret(raw))
            source, target, proposal, review, snapshot, files = fixture()
            source["tokens"][0]["key"] = raw
            with self.subTest(raw=raw), self.assertRaisesRegex(IMPORT.Error, "credential_ordinary_wire_contract_unsupported"):
                IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                    snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        for raw in ("A" * 32, "z" * 256, "_" * 32, SYNTHETIC_SECRET):
            self.assertTrue(IMPORT.ordinary_secret(raw))

    def test_alias_limits_and_nonzero_digest_are_precommit_requirements(self):
        source, target, proposal, review, snapshot, files = fixture()
        source["tokens"] *= IMPORT.MAX_ALIAS_KEYS + 1
        with patch.object(IMPORT.PROPOSAL, "plan_api_keys") as planner:
            with self.assertRaisesRegex(IMPORT.Error, "credential_alias_key_count_invalid"):
                IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                    snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
            planner.assert_not_called()
        source, target, proposal, review, snapshot, files = fixture()
        source["source_archive_sha256"] = "0" * 64
        with self.assertRaisesRegex(IMPORT.Error, "credential_alias_source_digest_invalid"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        source, target, proposal, review, snapshot, files = fixture()
        snapshot["instance"]["server_instance_nonce"] = "0" * 64
        with self.assertRaisesRegex(IMPORT.Error, "credential_alias_target_nonce_invalid"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        for value in ("0" * 64, "A" * 64, "f" * 63, "g" * 64):
            with self.subTest(digest=value), self.assertRaises(IMPORT.Error):
                IMPORT.alias_digest(value)
        self.assertEqual(IMPORT.FIN.checked_hash("0" * 64), "0" * 64)

    def test_snapshot_string_oid_is_normalized_only_in_core_alias_target(self):
        source, target, proposal, review, snapshot, files = fixture()
        snapshot["instance"]["database_oid"] = "13"
        before = deepcopy(snapshot["instance"])
        material = IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
            snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        self.assertEqual(snapshot["instance"], before)
        self.assertEqual(material["target"]["instance"]["database_oid"], "13")
        receipt_bytes = (IMPORT.canonical(material["credential_receipt"]) + "\n").encode()
        source_bytes = (IMPORT.canonical(source) + "\n").encode()
        material["source_files"]["source"]["sha256"] = IMPORT.FIN.sha(source_bytes)
        with patch.object(IMPORT, "private_bytes", return_value=source_bytes):
            manifest = IMPORT.build_alias_manifest(material, receipt_bytes)
        self.assertEqual(manifest["target"]["database_oid"], 13)
        self.assertIs(type(manifest["target"]["database_oid"]), int)
        self.assertEqual(material["target"]["instance"]["database_oid"], "13")

    def test_alias_target_oid_rejects_non_decimal_nonpositive_and_out_of_i64_values(self):
        instance = fixture()[4]["instance"]
        rejected = (True, False, 0, -1, 1.0, None, "", " ", " 13", "13 ", "+13", "-13", "13.0",
                    "1e2", "0x0d", "１３", "١٣", "0", "000", "9223372036854775808",
                    9223372036854775808, "1" * 1000)
        for oid in rejected:
            with self.subTest(oid=oid), self.assertRaisesRegex(IMPORT.Error, "credential_alias_target_invalid"):
                IMPORT.alias_target({**instance, "database_oid": oid})
        for oid, expected in (("00013", 13), ("9223372036854775807", 9223372036854775807), (13, 13)):
            self.assertEqual(IMPORT.alias_target({**instance, "database_oid": oid})["database_oid"], expected)

    def test_one_unsupported_row_rejects_the_entire_multi_key_batch(self):
        _, target, _, _, snapshot, files = fixture()
        source, _ = inputs([token(), token(id=18, key="SyntheticOtherCredentialBytes1234567890", oauth_managed=True)])
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        review = fixture()[3]
        review["source_input_sha256"] = IMPORT.PROPOSAL.sha256(source)
        review["proposal_sha256"] = proposal["proposal_sha256"]
        with patch.object(IMPORT, "encrypt_secret") as encrypt, self.assertRaisesRegex(IMPORT.Error, "credential_native_semantics_unsupported"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        encrypt.assert_not_called()

    def test_proposal_or_owner_mapping_edit_cannot_sneak_past_rebuild(self):
        for field in ("target_user_id", "target_personal_account_id", "target_auth_version"):
            source, target, proposal, review, snapshot, files = fixture()
            proposal["records"][0][field] += 1
            with self.subTest(field=field), self.assertRaisesRegex(IMPORT.Error, "credential_proposal_rebuild_mismatch"):
                IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                    snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)

    def test_all_review_economic_and_frozen_bindings_are_required(self):
        for field, value in (("decision", "REVIEW_REQUIRED"), ("source_frozen", False),
                             ("numerator", True), ("denominator", 2), ("source_unit", "unknown"),
                             ("proposal_sha256", "0" * 64)):
            source, target, proposal, review, snapshot, files = fixture()
            review[field] = value
            with self.subTest(field=field), self.assertRaises(IMPORT.Error):
                IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                    snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)

    def test_target_authority_projection_changes_fail(self):
        source, target, proposal, review, snapshot, files = fixture()
        snapshot["native_users"] = deepcopy(target["users"])
        snapshot["native_users"][0]["auth_version"] += 1
        with self.assertRaisesRegex(IMPORT.Error, "credential_target_projection_stale"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)

    def test_resigned_owner_remapping_cannot_replace_installed_identity_receipt(self):
        source, target, _, _, snapshot, files = fixture()
        target["source_target_id_map"][0]["target_user_id"] = 999
        target["users"][0]["user_id"] = 999
        target["source_target_id_map_sha256"] = hashlib.sha256(IMPORT.PROPOSAL.id_map_bytes(target["source_target_id_map"]).encode()).hexdigest()
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        review = fixture()[3]
        review["target_input_sha256"] = IMPORT.PROPOSAL.sha256(target)
        review["proposal_sha256"] = proposal["proposal_sha256"]
        with self.assertRaisesRegex(IMPORT.Error, "credential_identity_import_binding_mismatch"):
            IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
                snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)

    def test_nonempty_key_target_and_missing_financial_opening_fail(self):
        for name in IMPORT.CHANGED_TABLES:
            material = material_fixture()
            material["before_state"]["tables"][name]["rows"] = 1
            with self.subTest(name=name), self.assertRaisesRegex(IMPORT.Error, "credential_target_not_empty"):
                IMPORT.validate_material(material)
        material = material_fixture()
        material["financial_import"] = {}
        with self.assertRaisesRegex(IMPORT.Error, "credential_financial_opening_required"):
            IMPORT.validate_material(material)

    def test_quota_requires_same_committed_economic_review_and_source_export(self):
        for edit in (lambda m: m["source_review"].update(economic_review_sha256="1" * 64),
                     lambda m: m["source_review"].update(evidence_sha256="1" * 64),
                     lambda m: m["financial_import"]["manifest"]["material"]["source_files"]["source_export"].update(sha256="1" * 64)):
            material = material_fixture()
            edit(material)
            with self.assertRaisesRegex(IMPORT.Error, "credential_economic_review_binding_mismatch"):
                IMPORT.validate_material(material)

    def test_signed_unlimited_display_and_history_do_not_become_new_money(self):
        material = material_fixture(changes={"unlimited_quota": True, "remain_quota": -80})
        self.assertEqual(material["records"][0]["quota"]["display_limit_credits"], -10)
        sql = IMPORT.build_apply_sql(material)
        self.assertIn("'lifetime',0,TRUE,-10)", sql)
        self.assertNotIn("INSERT INTO core_billing.", sql)

    def test_row_and_quota_tampering_rejected_even_after_resigning_mapping(self):
        material = material_fixture()
        material["records"][0]["quota"]["opening_consumed"] += 1
        material["mapping_sha256"] = IMPORT.digest(material["records"])
        with self.assertRaisesRegex(IMPORT.Error, "credential_record_proposal_mismatch"):
            IMPORT.validate_material(material)
        material = material_fixture()
        material["quota_plan"]["key_records"][0]["target_account_id"] += 1
        material["quota_plan"]["mapping_sha256"] = IMPORT.digest(material["quota_plan"]["key_records"])
        with self.assertRaisesRegex(IMPORT.Error, "credential_quota_row_mismatch"):
            IMPORT.validate_material(material)

    def test_sql_delimiter_in_name_does_not_break_dollar_quoting(self):
        material = material_fixture(changes={"name": "fixture $lmm_credential$ ' ; SELECT secret"})
        sql = IMPORT.build_apply_sql(material)
        self.assertIn("DO $lmm_credential_", sql)
        self.assertNotIn("DO $lmm_credential$", sql)
        self.assertIn(IMPORT.literal(material["records"][0]["name"]), sql)

    def test_verify_binds_original_attempt_and_exact_whole_post_state(self):
        material = material_fixture()
        sql = IMPORT.build_verify_sql(material, "lmm-credential-" + "a" * 32)
        self.assertIn("SERIALIZABLE READ ONLY", sql)
        self.assertIn("credential_original_attempt_active", sql)
        self.assertIn("credential_absence_not_proven", sql)
        self.assertIn("previous.manifest->'post_state' IS DISTINCT FROM actual", sql)
        self.assertIn("credential_exact_readback_failed", sql)
        self.assertIn("ELSE 'UNKNOWN'", sql)
        self.assertNotIn("'NOT_APPLIED'", sql)
        self.assertNotIn("INSERT INTO", sql)
        with self.assertRaises(IMPORT.Error):
            IMPORT.build_verify_sql(material, "sql-looking-unknown-attempt")

    def test_receipt_not_emitted_on_unknown_or_refused(self):
        material = material_fixture()
        sql = IMPORT.build_apply_sql(material)
        approval = IMPORT.approval_for(material, sql, "approve-offline-credential-and-quota-import")
        connection = {"database": "synthetic_credential_import", "role": "synthetic_owner", "login_role": "synthetic_installer",
                      "set_role": "synthetic_owner", "port": 5432}
        writes = []
        for failure in ((1, ""), (0, "not-json"), RuntimeError("synthetic transport failure")):
            writes.clear()
            with patch.object(IMPORT, "verify_source_files"), patch.object(IMPORT.IDENTITY, "socket_connection"), \
                 patch.object(IMPORT.FIN, "write_private", side_effect=lambda d, n, v: writes.append(n)), \
                 patch.object(IMPORT.FIN, "run_sql", side_effect=failure if isinstance(failure, Exception) else None,
                              return_value=failure if not isinstance(failure, Exception) else None):
                result = IMPORT.execute(material, original_sql=sql, approval=approval, connection=connection, output=Path("/unused"))
                self.assertEqual(result["status"], "UNKNOWN")
                self.assertNotIn(IMPORT.MAPPING_FILE, writes)
                self.assertNotIn(IMPORT.ALIAS_FILE, writes)
        with patch.object(IMPORT.FIN, "run_sql") as run:
            with self.assertRaisesRegex(IMPORT.Error, "credential_approval_mismatch"):
                IMPORT.execute(material, original_sql=sql, approval={}, connection=connection, output=Path("/unused"))
            run.assert_not_called()

    def test_protocol_ack_emits_exact_consumer_receipt_after_durable_attempt(self):
        material = material_fixture()
        sql = IMPORT.build_apply_sql(material)
        outcome = {"status": "COMMITTED", "migration_id": material["migration_id"], "material_sha256": IMPORT.digest(material)}
        connection = {"database": "synthetic_credential_import", "role": "synthetic_owner", "login_role": "synthetic_installer",
                      "set_role": "synthetic_owner", "port": 5432}
        with tempfile.TemporaryDirectory(dir=IMPORT.IDENTITY.BACKUP_ROOT) as directory:
            output = Path(directory)
            source_text = IMPORT.canonical(fixture()[0]) + "\n"
            source_path = IMPORT.FIN.write_private(output, "synthetic-source.private.json", source_text)
            material["source_files"]["source"] = {"path": str(source_path), "sha256": IMPORT.FIN.sha(source_text.encode())}
            sql = IMPORT.build_apply_sql(material)
            outcome["material_sha256"] = IMPORT.digest(material)
            def run(*args):
                self.assertTrue((output / "attempt.private.json").is_file())
                self.assertTrue((output / "executed.private.sql").is_file())
                self.assertFalse((output / IMPORT.MAPPING_FILE).exists())
                self.assertFalse((output / IMPORT.ALIAS_FILE).exists())
                return 0, json.dumps(outcome)
            with patch.object(IMPORT, "verify_source_files"), patch.object(IMPORT.IDENTITY, "socket_connection"), \
                 patch.object(IMPORT.FIN, "run_sql", side_effect=run):
                result = IMPORT.execute(material, original_sql=sql,
                    approval=IMPORT.approval_for(material, sql, "approve-offline-credential-and-quota-import"),
                    connection=connection, output=output)
            self.assertEqual(result, outcome)
            receipt_bytes = IMPORT.private_bytes(output / IMPORT.MAPPING_FILE)
            receipt = json.loads(receipt_bytes)
            manifest = IMPORT.load_json(output / IMPORT.ALIAS_FILE)
            self.assertEqual(set(receipt), {"scope", "source_archive_sha256", "records"})
            self.assertEqual(set(receipt["records"][0]), {"source_key_id", "target_credential_id", "target_user_id",
                "target_account_id", "source_secret_sha256"})
            self.assertEqual(set(manifest), {"version", "scope", "source_archive_sha256", "credential_receipt_sha256", "target", "keys"})
            self.assertEqual(manifest["version"], 1)
            self.assertEqual(manifest["scope"], "legacy-api-key-wire-aliases-v1")
            self.assertEqual(manifest["source_archive_sha256"], receipt["source_archive_sha256"])
            self.assertEqual(manifest["credential_receipt_sha256"], IMPORT.FIN.sha(receipt_bytes))
            self.assertTrue(receipt_bytes.endswith(b"\n"))
            self.assertNotEqual(manifest["credential_receipt_sha256"], IMPORT.FIN.sha(receipt_bytes.rstrip(b"\n")))
            self.assertEqual(manifest["target"], {"database": "synthetic_credential_import", "database_oid": 13, "instance_nonce": "f" * 64})
            self.assertEqual(manifest["keys"], [{**receipt["records"][0], "credential_user_version": 5,
                "sk_alias_sha256": IMPORT.FIN.sha(("sk-" + SYNTHETIC_SECRET).encode())}])
            self.assertNotIn(SYNTHETIC_SECRET, IMPORT.canonical(manifest))
            audit = IMPORT.load_json(output / IMPORT.ARTIFACT_AUDIT_FILE)
            self.assertEqual(audit["status"], "COMMITTED")
            self.assertEqual(audit["alias_manifest_sha256"], IMPORT.FIN.sha(IMPORT.private_bytes(output / IMPORT.ALIAS_FILE)))

    def test_alias_manifest_rejects_reformatted_receipt_and_changed_source_bytes(self):
        material = material_fixture()
        receipt_bytes = (IMPORT.canonical(material["credential_receipt"]) + "\n").encode()
        for changed in (receipt_bytes.rstrip(b"\n"), receipt_bytes + b"\n", json.dumps(material["credential_receipt"], indent=2).encode()):
            with self.subTest(receipt=changed), self.assertRaisesRegex(IMPORT.Error, "credential_receipt_bytes_changed"):
                IMPORT.build_alias_manifest(material, changed)
        with patch.object(IMPORT, "private_bytes", return_value=b"different-source"):
            with self.assertRaisesRegex(IMPORT.Error, "credential_source_file_changed"):
                IMPORT.build_alias_manifest(material, receipt_bytes)

    def test_unknown_or_unsupported_absence_ack_cannot_authorize_reapply_or_publish(self):
        material = material_fixture()
        sql = IMPORT.build_apply_sql(material)
        outcome = {"status": "UNKNOWN", "migration_id": material["migration_id"], "material_sha256": IMPORT.digest(material)}
        original = {"operation": "apply", "application_name": "lmm-credential-" + "a" * 32,
            "migration_id": material["migration_id"], "material_sha256": IMPORT.digest(material), "sql_sha256": IMPORT.FIN.sha(sql.encode())}
        connection = {"database": "synthetic_credential_import", "role": "synthetic_owner", "login_role": "synthetic_installer",
                      "set_role": "synthetic_owner", "port": 5432}
        for status in ("UNKNOWN", "NOT_APPLIED"):
            writes = []
            with self.subTest(status=status), patch.object(IMPORT, "verify_source_files"), \
                 patch.object(IMPORT.IDENTITY, "socket_connection"), \
                 patch.object(IMPORT.FIN, "write_private", side_effect=lambda d, n, v: writes.append(n)), \
                 patch.object(IMPORT.FIN, "run_sql", return_value=(0, json.dumps({**outcome, "status": status}))) as run:
                result = IMPORT.execute(material, original_sql=sql,
                    approval=IMPORT.approval_for(material, sql, "approve-offline-credential-and-quota-import"),
                    connection=connection, output=Path("/unused"), original_attempt=original)
            self.assertEqual(result, outcome)
            run.assert_called_once()
            self.assertIn("SERIALIZABLE READ ONLY", run.call_args.args[1])
            self.assertNotIn("INSERT INTO", run.call_args.args[1])
            self.assertNotIn(IMPORT.MAPPING_FILE, writes)
            self.assertNotIn(IMPORT.ALIAS_FILE, writes)
            self.assertNotIn(IMPORT.ARTIFACT_AUDIT_FILE, writes)

    def test_alias_publication_requires_commit_of_the_same_material(self):
        material = material_fixture()
        committed = {"status": "COMMITTED", "migration_id": material["migration_id"], "material_sha256": IMPORT.digest(material)}
        for status in ("PREPARED_NOT_APPLIED", "UNKNOWN", "NOT_APPLIED"):
            with self.subTest(status=status), self.assertRaisesRegex(IMPORT.Error, "credential_artifacts_require_commit"):
                IMPORT.publish_committed_outputs(material, {**committed, "status": status}, Path("/unused"))
        material["target"]["instance"]["database_oid"] += 1
        with self.assertRaisesRegex(IMPORT.Error, "credential_artifacts_require_commit"):
            IMPORT.publish_committed_outputs(material, committed, Path("/unused"))

    def test_changed_receipt_file_during_publication_cannot_leave_installable_outputs(self):
        material = material_fixture()
        original_write = IMPORT.FIN.write_private
        with tempfile.TemporaryDirectory(dir=IMPORT.IDENTITY.BACKUP_ROOT) as directory:
            output = Path(directory)
            source_text = IMPORT.canonical(fixture()[0]) + "\n"
            source_path = original_write(output, "synthetic-source.private.json", source_text)
            material["source_files"]["source"] = {"path": str(source_path), "sha256": IMPORT.FIN.sha(source_text.encode())}
            committed = {"status": "COMMITTED", "migration_id": material["migration_id"], "material_sha256": IMPORT.digest(material)}
            def changed_write(directory, name, value):
                return original_write(directory, name, value + "\n" if name == IMPORT.MAPPING_FILE else value)
            with patch.object(IMPORT.FIN, "write_private", side_effect=changed_write):
                with self.assertRaisesRegex(IMPORT.PublicationError, "credential_artifact_publication_failed"):
                    IMPORT.publish_committed_outputs(material, committed, output)
            self.assertFalse((output / IMPORT.MAPPING_FILE).exists())
            self.assertFalse((output / IMPORT.ALIAS_FILE).exists())

    def test_final_result_write_failure_returns_unknown_and_removes_both_core_inputs(self):
        material = material_fixture()
        original_write = IMPORT.FIN.write_private
        connection = {"database": "synthetic_credential_import", "role": "synthetic_owner", "login_role": "synthetic_installer",
                      "set_role": "synthetic_owner", "port": 5432}
        with tempfile.TemporaryDirectory(dir=IMPORT.IDENTITY.BACKUP_ROOT) as directory:
            output = Path(directory)
            source_text = IMPORT.canonical(fixture()[0]) + "\n"
            source_path = original_write(output, "synthetic-source.private.json", source_text)
            material["source_files"]["source"] = {"path": str(source_path), "sha256": IMPORT.FIN.sha(source_text.encode())}
            sql = IMPORT.build_apply_sql(material)
            committed = {"status": "COMMITTED", "migration_id": material["migration_id"], "material_sha256": IMPORT.digest(material)}
            def failing_write(directory, name, value):
                if name == "result.private.json":
                    raise OSError("synthetic final receipt failure")
                return original_write(directory, name, value)
            with patch.object(IMPORT, "verify_source_files"), patch.object(IMPORT.IDENTITY, "socket_connection"), \
                 patch.object(IMPORT.FIN, "write_private", side_effect=failing_write), \
                 patch.object(IMPORT.FIN, "run_sql", return_value=(0, json.dumps(committed))):
                result = IMPORT.execute(material, original_sql=sql,
                    approval=IMPORT.approval_for(material, sql, "approve-offline-credential-and-quota-import"),
                    connection=connection, output=output)
            self.assertEqual(result["status"], "UNKNOWN")
            self.assertFalse(result["publication_incomplete"])
            self.assertFalse((output / IMPORT.MAPPING_FILE).exists())
            self.assertFalse((output / IMPORT.ALIAS_FILE).exists())
            self.assertFalse((output / IMPORT.ARTIFACT_AUDIT_FILE).exists())
            self.assertTrue((output / "attempt.private.json").exists())
            self.assertTrue((output / "executed.private.sql").exists())

    def test_cleanup_first_unlink_failure_preserves_unknown_and_reports_exact_residual(self):
        original_write = IMPORT.FIN.write_private
        original_unlink = Path.unlink
        connection = {"database": "synthetic_credential_import", "role": "synthetic_owner", "login_role": "synthetic_installer",
                      "set_role": "synthetic_owner", "port": 5432}
        for fault in ("result", "audit"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory(dir=IMPORT.IDENTITY.BACKUP_ROOT) as directory:
                output = Path(directory)
                material = material_fixture()
                source_text = IMPORT.canonical(fixture()[0]) + "\n"
                source_path = original_write(output, "synthetic-source.private.json", source_text)
                material["source_files"]["source"] = {"path": str(source_path), "sha256": IMPORT.FIN.sha(source_text.encode())}
                sql = IMPORT.build_apply_sql(material)
                committed = {"status": "COMMITTED", "migration_id": material["migration_id"], "material_sha256": IMPORT.digest(material)}
                attempted = []
                def failed_unlink(path, *args, **kwargs):
                    attempted.append(path.name)
                    if path.name == IMPORT.MAPPING_FILE:
                        raise PermissionError("synthetic mapping deletion denied")
                    return original_unlink(path, *args, **kwargs)
                def failing_write(directory, name, value):
                    if fault == "result" and name == "result.private.json":
                        raise OSError("synthetic final result failure")
                    if fault == "audit" and name == IMPORT.ARTIFACT_AUDIT_FILE:
                        value += "\n"
                    return original_write(directory, name, value)
                with patch.object(IMPORT, "verify_source_files"), patch.object(IMPORT.IDENTITY, "socket_connection"), \
                     patch.object(IMPORT.FIN, "write_private", side_effect=failing_write), \
                     patch.object(IMPORT.FIN, "run_sql", return_value=(0, json.dumps(committed))), \
                     patch.object(Path, "unlink", failed_unlink):
                    result = IMPORT.execute(material, original_sql=sql,
                        approval=IMPORT.approval_for(material, sql, "approve-offline-credential-and-quota-import"),
                        connection=connection, output=output)
                self.assertEqual(result["status"], "UNKNOWN")
                self.assertTrue(result["publication_incomplete"])
                self.assertEqual(result["publication_error_code"], "credential_artifact_publication_failed")
                report = result["publication_cleanup"]
                self.assertEqual(report["remaining_files"], [IMPORT.MAPPING_FILE])
                self.assertEqual(report["absent_files"], [IMPORT.ALIAS_FILE, IMPORT.ARTIFACT_AUDIT_FILE])
                self.assertEqual(report["unconfirmed_files"], [])
                self.assertTrue(report["directory_fsynced"])
                self.assertEqual(attempted, [IMPORT.MAPPING_FILE, IMPORT.ALIAS_FILE, IMPORT.ARTIFACT_AUDIT_FILE])
                durable = "publication-status.private.json" if fault == "result" else "result.private.json"
                self.assertEqual(IMPORT.load_json(output / durable), result)
                self.assertTrue((output / "attempt.private.json").exists())
                self.assertTrue((output / "executed.private.sql").exists())
                self.assertTrue((output / IMPORT.MAPPING_FILE).exists())
                self.assertFalse((output / IMPORT.ALIAS_FILE).exists())
                self.assertFalse((output / IMPORT.ARTIFACT_AUDIT_FILE).exists())

    def test_cleanup_fsync_and_presence_failures_are_not_claimed_as_clean(self):
        original_lstat = Path.lstat
        with tempfile.TemporaryDirectory(dir=IMPORT.IDENTITY.BACKUP_ROOT) as directory:
            output = Path(directory)
            for name in (IMPORT.MAPPING_FILE, IMPORT.ALIAS_FILE, IMPORT.ARTIFACT_AUDIT_FILE):
                IMPORT.FIN.write_private(output, name, "synthetic artifact\n")
            def unconfirmed_lstat(path, *args, **kwargs):
                if path.name == IMPORT.ALIAS_FILE:
                    raise PermissionError("synthetic presence check denied")
                return original_lstat(path, *args, **kwargs)
            with patch.object(IMPORT.os, "fsync", side_effect=OSError("synthetic directory sync failure")), \
                 patch.object(Path, "lstat", unconfirmed_lstat):
                report = IMPORT.cleanup_outputs(output, [IMPORT.MAPPING_FILE, IMPORT.ALIAS_FILE, IMPORT.ARTIFACT_AUDIT_FILE])
            self.assertFalse(report["directory_fsynced"])
            self.assertEqual(report["unconfirmed_files"], [IMPORT.ALIAS_FILE])
            self.assertEqual(report["remaining_files"], [])
            self.assertEqual(report["absent_files"], [IMPORT.MAPPING_FILE, IMPORT.ARTIFACT_AUDIT_FILE])
            result = {"status": "COMMITTED"}
            IMPORT._publication_failure(result, report)
            self.assertEqual(result["status"], "UNKNOWN")
            self.assertTrue(result["publication_incomplete"])

    def test_cli_failure_never_echoes_secret_decoder_input(self):
        for secret in (SYNTHETIC_SECRET, "syntheticprivatesecretalllowercase"):
            with patch.object(IMPORT.sys, "argv", ["legacy_credential_import.py", "--apply", "--output", "/unused/output",
                    "--target-receipt", "/unused/target", "--material", "/unused/material", "--sql", "/unused/sql", "--approval", "/unused/approval"]), \
                 patch.object(IMPORT.IDENTITY, "private_backup_path", side_effect=ValueError(secret)), \
                 patch.object(IMPORT.sys, "stderr", new_callable=io.StringIO) as stderr:
                self.assertEqual(IMPORT.main(), 1)
                self.assertNotIn(secret, stderr.getvalue())
                self.assertIn("credential_input_or_validation_failed", stderr.getvalue())

    def test_cli_missing_target_receipt_refuses_without_files_or_database(self):
        with patch.object(IMPORT.sys, "argv", ["legacy_credential_import.py", "--prepare", "--output", "/unused/output"]), \
             patch.object(IMPORT.FIN, "run_sql") as run, \
             patch.object(IMPORT.sys, "stderr", new_callable=io.StringIO) as stderr:
            self.assertEqual(IMPORT.main(), 1)
            self.assertIn("credential_target_receipt_required", stderr.getvalue())
            run.assert_not_called()

    def test_revoke_unsupported_decision_revokes_unimportable_keys_and_marks_alias_manifest(self):
        _, target, _, _, snapshot, files = fixture()
        managed_secret = "oauth_managed_" + "A" * 32
        source, _ = inputs([
            token(),
            token(id=18, key=managed_secret, oauth_managed=True, one_time_reveal=False),
            token(id=19, key="SyntheticRetiredGroupKeyBytes01234567890", group="retired"),
        ])
        proposal = IMPORT.PROPOSAL.plan_api_keys(source, target)
        review = fixture()[3]
        review["decision"] = "approve-offline-revoke-unsupported-credentials"
        review["source_input_sha256"] = IMPORT.PROPOSAL.sha256(source)
        review["proposal_sha256"] = proposal["proposal_sha256"]
        files = {name: {"path": "/home/lightjunction/.local/state/lmm-api/synthetic-credential-test/" + name,
                        "sha256": "a" * 64} for name in IMPORT.FILES}
        material = IMPORT.prepare_material(source, target, proposal, review, migration_id="synthetic-import",
            snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)
        self.assertEqual(len(material["records"]), 3)
        self.assertNotIn("revoked_rekey_required", material["records"][0])
        self.assertFalse(material["records"][0]["revoked"])
        self.assertTrue(material["records"][0]["enabled"])
        for idx in (1, 2):
            self.assertTrue(material["records"][idx]["revoked_rekey_required"])
            self.assertTrue(material["records"][idx]["revoked"])
            self.assertFalse(material["records"][idx]["enabled"])
            self.assertTrue(material["records"][idx]["one_time_reveal"])
            self.assertIsNone(material["records"][idx]["encrypted_secret_hex"])
            self.assertEqual(material["records"][idx]["group"]["value"], "paid" if idx == 1 else "retired")
        sql = IMPORT.build_apply_sql(material)
        self.assertIn("to_timestamp(1000)", sql)
        receipt_bytes = (IMPORT.canonical(material["credential_receipt"]) + "\n").encode()
        source_bytes = (IMPORT.canonical(source) + "\n").encode()
        material["source_files"]["source"]["sha256"] = IMPORT.FIN.sha(source_bytes)
        with patch.object(IMPORT, "private_bytes", return_value=source_bytes):
            manifest = IMPORT.build_alias_manifest(material, receipt_bytes)
        self.assertNotIn("revoked_rekey_required", manifest["keys"][0])
        self.assertTrue(manifest["keys"][1]["revoked_rekey_required"])
        self.assertTrue(manifest["keys"][2]["revoked_rekey_required"])


if __name__ == "__main__":
    unittest.main()
