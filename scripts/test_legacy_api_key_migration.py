"""Synthetic offline proposal contracts; no production records or database."""

from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("legacy_api_key_migration.py")
SPEC = importlib.util.spec_from_file_location("legacy_api_key_migration", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
SYNTHETIC_SECRET = "SyntheticPrivateCredentialBytes1234567890"


def token(**changes):
    row = {"id": 17, "user_id": 37, "key": SYNTHETIC_SECRET, "status": 1,
           "name": "Synthetic fixture", "created_time": 100, "accessed_time": 200,
           "expired_time": -1, "remain_quota": 30, "unlimited_quota": False,
           "one_time_reveal": True, "account_balance_read": True,
           "model_limits_enabled": False, "model_limits": "", "allow_ips": None,
           "used_quota": 70, "group": "paid", "cross_group_retry": False,
           "auto_groups": "", "oauth_managed": False, "creation_source": "manual",
           "deleted_at": None}
    return {**row, **changes}


def inputs(tokens=None):
    mapping = [{"source_user_id": 37, "target_user_id": 37, "target_personal_account_id": 8}]
    source = {"format_version": 1, "source_revision": MODULE.LEGACY_REVISION,
              "source_archive_sha256": "a" * 64, "source_schema_sha256": "b" * 64,
              "source_export_sha256": "c" * 64, "source_projection_sha256": "d" * 64,
              "snapshot_at": 1000, "tokens": [token()] if tokens is None else tokens,
              "users": [{"id": 37, "status": 1, "deleted_at": None, "auth_version": 5,
                         "group": "paid", "billing_preference": "wallet_first"}],
              "policy": {"auto_groups": ["paid", "backup"], "max_token_auto_groups": 4,
                         "user_model_rate_limit_enabled": False, "source_options_sha256": "e" * 64,
                         "group_authority": {"user_usable_groups": {"paid": "Paid", "backup": "Backup", "auto": "Auto"},
                                             "special_usable_groups": {}, "group_ratios": {"paid": 1, "backup": 1},
                                             "group_group_ratios": {}}}}
    target = {"format_version": 1, "source_archive_sha256": "a" * 64,
              "source_target_id_map": mapping,
              "source_target_id_map_sha256": hashlib.sha256(MODULE.id_map_bytes(mapping).encode()).hexdigest(),
              "users": [{"user_id": 37, "personal_account_id": 8, "auth_version": 5,
                         "active": True, "developer_access_granted": True,
                         "routing_groups": ["paid", "backup"]}]}
    return source, target


class ProposalTests(unittest.TestCase):
    def test_source_explicit_gate_cannot_be_replaced_by_native_membership(self):
        for group, reason in (("default", "source_token_group_deprecated"),
                              ("retired", "source_token_group_not_usable"),
                              ("auto", "source_token_group_not_usable")):
            with self.subTest(group=group):
                source, target = inputs([token(group=group)])
                source["users"][0]["group"] = "default"
                del source["policy"]["group_authority"]["user_usable_groups"]["auto"]
                if group != "auto":  # Native membership stores concrete groups only.
                    target["users"][0]["routing_groups"].append(group)
                row = MODULE.plan_api_keys(source, target)["records"][0]
                self.assertIn(reason, row["blocked_reasons"])
                self.assertFalse(row["source_group_authority"]["allowed"])
                self.assertFalse(row["source_usable_at_snapshot"])
                self.assertEqual(row["group"]["value"], group)
                self.assertEqual(row["native_effective_group"], group)

    def test_default_snapshot_binds_raw_policy_user_and_distinct_account_mapping(self):
        source, target = inputs([token(group="")])
        source["users"][0]["group"] = "default"
        target["users"][0]["routing_groups"].append("default")
        row = MODULE.plan_api_keys(source, target)["records"][0]
        audit = row["source_group_authority"]
        self.assertEqual(row["group"]["value"], "")
        self.assertEqual(row["native_effective_group"], "default")
        self.assertEqual(row["blocked_reasons"], [])
        self.assertEqual(audit["user_snapshot"], source["users"][0])
        self.assertEqual(audit["user_snapshot_sha256"], MODULE.sha256(source["users"][0]))
        self.assertEqual(audit["mapping_sha256"], MODULE.sha256(target["source_target_id_map"][0]))
        self.assertEqual(audit["policy_sha256"], MODULE.sha256(source["policy"]))
        self.assertEqual(audit["default_ratio_basis"], {"kind": "cc52_missing_group_ratio_fallback", "value": 1,
            "source_revision": MODULE.LEGACY_REVISION, "producer_sha256": MODULE.GROUP_RATIO_SOURCE_SHA256})
        target["users"][0]["routing_groups"].remove("default")
        row = MODULE.plan_api_keys(source, target)["records"][0]
        self.assertTrue(row["source_group_authority"]["allowed"])
        self.assertIn("target_routing_grant_missing", row["blocked_reasons"])

    def test_missing_or_unreviewed_source_group_policy_never_defaults_to_authorized(self):
        for mutate in (lambda p: p.pop("group_authority"),
                       lambda p: p.pop("source_options_sha256"),
                       lambda p: p["group_authority"].pop("group_ratios"),
                       lambda p: p["group_authority"].update(unknown={}),
                       lambda p: p["group_authority"].update(special_usable_groups={"paid": {"+:other": "Other"}}),
                       lambda p: p["group_authority"].update(group_group_ratios={"paid": {"paid": 0.5}})):
            source, target = inputs()
            mutate(source["policy"])
            with self.assertRaises(MODULE.ProposalError):
                MODULE.plan_api_keys(source, target)
        source, target = inputs([token(group="")])
        source["users"][0]["group"] = "unknown-source-group"
        with self.assertRaisesRegex(MODULE.ProposalError, "source_user_group_not_reviewed"):
            MODULE.plan_api_keys(source, target)

    def test_reviewed_ordinary_origins_preserve_raw_label_and_creation_time(self):
        for origin in (None, "", "manual", "assistant", "drawing_mcp"):
            with self.subTest(origin=origin):
                source, target = inputs([token(creation_source=origin, created_time=123)])
                result = MODULE.plan_api_keys(source, target)
                row = result["records"][0]
                self.assertEqual(row["blocked_reasons"], [])
                self.assertEqual(row["creation_source"], origin)
                self.assertEqual(row["created_time"], 123)
                self.assertEqual(row["target_personal_account_id"], 8)
                self.assertFalse(result["apply_supported"])
                self.assertFalse(result["activation_approved"])

    def test_origin_policy_does_not_admit_runtime_or_initial_system_keys(self):
        for origin in ("system", "red_packet_cover", "assistant_runtime"):
            with self.subTest(origin=origin):
                source, target = inputs([token(creation_source=origin)])
                row = MODULE.plan_api_keys(source, target)["records"][0]
                self.assertIn("creation_source_metadata_requires_adapter", row["blocked_reasons"])
        for origin in ("manual", "assistant", "drawing_mcp"):
            with self.subTest(initial_name_origin=origin):
                source, target = inputs([token(creation_source=origin, name="Synthetic的初始令牌")])
                row = MODULE.plan_api_keys(source, target)["records"][0]
                self.assertIn("creation_source_metadata_requires_adapter", row["blocked_reasons"])

    def test_preserves_secret_digest_owner_quota_and_funding_without_authorizing_apply(self):
        source, target = inputs()
        before = deepcopy((source, target))
        result = MODULE.plan_api_keys(source, target)
        self.assertEqual(before, (source, target))
        self.assertEqual(result["status"], "proposal")
        self.assertFalse(result["apply_supported"])
        self.assertFalse(result["activation_approved"])
        row = result["records"][0]
        self.assertEqual(row["digest_sha256"], hashlib.sha256(SYNTHETIC_SECRET.encode()).hexdigest())
        self.assertEqual(row["target_personal_account_id"], 8)
        self.assertEqual(row["quota"], {"remaining": 30, "opening_consumed": 70,
                                      "display_limit_credits": 100, "limit_credits": 100, "unlimited": False,
                                      "source_unit": "source_quota_integer", "native_unit": "credit_500k_usd",
                                      "unit_conversion_approved": False})
        self.assertEqual(row["funding"], {"personal_billing_preference": "wallet_first", "account_order": [8]})
        self.assertTrue(row["account_balance_read"])
        self.assertNotIn(SYNTHETIC_SECRET, json.dumps(result))
        self.assertEqual(result["summary"]["rows_written"], 0)

    def test_unlimited_retains_negative_remaining_and_historical_consumption(self):
        result = MODULE.plan_api_keys(*inputs([token(unlimited_quota=True, remain_quota=-80)]))
        quota = result["records"][0]["quota"]
        self.assertEqual(quota["limit_credits"], 0)
        self.assertEqual(quota["display_limit_credits"], -10)
        self.assertEqual(quota["opening_consumed"], 70)
        self.assertEqual(quota["remaining"], -80)
        self.assertTrue(result["records"][0]["source_usable_at_snapshot"])

    def test_status_expiry_deleted_owner_and_managed_states_never_revive(self):
        cases = [{"status": 2}, {"status": 3}, {"status": 4}, {"expired_time": 999},
                 {"remain_quota": 0}, {"oauth_managed": True},
                 {"creation_source": "assistant_runtime"},
                 {"deleted_at": "2026-10-26T00:00:00+00:00"}]
        for changes in cases:
            with self.subTest(changes=changes):
                result = MODULE.plan_api_keys(*inputs([token(**changes)]))
                self.assertFalse(result["records"][0]["source_usable_at_snapshot"])
                self.assertEqual(result["summary"]["source_token_count"], 1)
                self.assertEqual(result["summary"]["dropped_token_count"], 0)
        for changes in ({"status": 2}, {"deleted_at": "2026-10-26T00:00:00+00:00"}):
            source, target = inputs()
            source["users"][0].update(changes)
            result = MODULE.plan_api_keys(source, target)
            self.assertFalse(result["records"][0]["source_owner_enabled"])
            self.assertFalse(result["records"][0]["source_usable_at_snapshot"])
            self.assertEqual(result["status"], "blocked")
            self.assertIn("source_owner_disabled_or_deleted_target_active", result["records"][0]["blocked_reasons"])

    def test_stored_delimiter_key_is_not_claimed_to_roundtrip_legacy_wire_parser(self):
        result = MODULE.plan_api_keys(*inputs([token(key=SYNTHETIC_SECRET + "-7")]))
        row = result["records"][0]
        self.assertFalse(row["source_stored_key_roundtrips_relay_parser"])
        self.assertIn("legacy_secret_requires_wire_adapter", row["blocked_reasons"])

    def test_expiry_preserves_legacy_inclusive_second_and_never_sentinel(self):
        row = MODULE.plan_api_keys(*inputs([token(expired_time=1000)]))["records"][0]
        self.assertTrue(row["source_usable_at_snapshot"])
        self.assertEqual(row["expiry"]["native_exclusive_epoch"], 1001)
        never = MODULE.plan_api_keys(*inputs())["records"][0]["expiry"]
        self.assertEqual(never["kind"], "never")
        self.assertEqual(never["source_epoch"], -1)
        self.assertEqual(never["native_exclusive_epoch"], MODULE.NEVER)

    def test_empty_group_is_audited_snapshot_while_dynamic_auto_remains_blocked(self):
        for raw in (None, "", "[]", "null"):
            result = MODULE.plan_api_keys(*inputs([token(group="auto", auto_groups=raw)]))
            row = result["records"][0]
            self.assertEqual(row["auto_groups"], {"mode": "dynamic_global", "values": []})
            self.assertEqual(row["source_auto_groups_sql_null"], raw is None)
            self.assertIn("dynamic_global_auto_groups_requires_adapter", row["blocked_reasons"])
            self.assertEqual(result["status"], "blocked")
        row = MODULE.plan_api_keys(*inputs([token(group="")]))["records"][0]
        self.assertEqual(row["group"], {"mode": "dynamic_user", "value": ""})
        self.assertEqual(row["native_effective_group"], "paid")
        self.assertEqual(row["source_group_authority"]["default_ratio_basis"],
                         {"kind": "source_group_ratio", "value": 1})
        self.assertEqual(row["blocked_reasons"], [])

    def test_invalid_auto_json_preserves_deny_all_instead_of_granting_global_auto(self):
        row = MODULE.plan_api_keys(*inputs([token(group="auto", auto_groups="not-json")]))["records"][0]
        self.assertEqual(row["auto_groups"], {"mode": "deny_all_invalid_json", "values": []})
        self.assertIn("invalid_auto_groups_deny_all_requires_adapter", row["blocked_reasons"])

    def test_snapshot_order_ip_and_model_policy_are_retained(self):
        row = MODULE.plan_api_keys(*inputs([token(group="auto", auto_groups='["backup","paid"]',
                                                 model_limits_enabled=True, model_limits="gpt-4, thinking-*",
                                                 allow_ips=" 192.0.2.3/24,\n2001:db8::1/64")]))["records"][0]
        self.assertEqual(row["auto_groups"]["values"], ["backup", "paid"])
        self.assertEqual(row["allow_ips"], ["192.0.2.3/24", "2001:db8::1/64"])
        self.assertEqual(row["model_limits"], ["gpt-4", " thinking-*"])
        self.assertIn("model_name_matching_requires_adapter", row["blocked_reasons"])
        self.assertIn("cross_group_retry_semantics_require_verification", row["blocked_reasons"])

    def test_missing_target_grants_rate_and_recovery_material_block_without_activation(self):
        source, target = inputs([token(one_time_reveal=False)])
        source["policy"]["user_model_rate_limit_enabled"] = True
        target["users"][0].update(active=False, developer_access_granted=False, routing_groups=[])
        row = MODULE.plan_api_keys(source, target)["records"][0]
        self.assertTrue({"recoverable_secret_encryption_required", "target_owner_inactive",
                         "target_developer_grant_missing", "target_routing_grant_missing",
                         "user_aggregate_rate_limit_requires_adapter"}.issubset(row["blocked_reasons"]))

    def test_changed_source_target_mapping_policy_and_input_are_bound(self):
        source, target = inputs()
        original = MODULE.plan_api_keys(source, target)
        source["source_archive_sha256"] = "c" * 64
        with self.assertRaisesRegex(MODULE.ProposalError, "target_source_hash_mismatch"):
            MODULE.plan_api_keys(source, target)
        source, target = inputs()
        source["policy"]["auto_groups"].reverse()
        self.assertNotEqual(original["proposal_sha256"], MODULE.plan_api_keys(source, target)["proposal_sha256"])
        target["source_target_id_map"][0]["target_personal_account_id"] = 9
        with self.assertRaisesRegex(MODULE.ProposalError, "id_map_hash_mismatch"):
            MODULE.plan_api_keys(source, target)

    def test_unknown_fields_types_and_unsupported_values_are_rejected(self):
        changes = [{"future_authority": True}, {"status": True}, {"status": 99},
                   {"creation_source": "future-plugin"}, {"user_id": 99},
                   {"unlimited_quota": 1}, {"used_quota": -1},
                   {"remain_quota": MODULE.MAX_I64}, {"allow_ips": "not-an-ip"},
                   {"auto_groups": '{"grant":"paid"}'}, {"expired_time": -2},
                   {"key": "\ud800"}, {"name": "\ud800"}, {"creation_source": {}},
                   {"auto_groups": "\ud800"}]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(MODULE.ProposalError):
                MODULE.plan_api_keys(*inputs([token(**change)]))

    def test_duplicate_keys_ids_and_secret_echo_are_rejected_without_echoing_secret(self):
        for rows in ([token(), token(id=18)], [token(), token(key="OtherSyntheticPrivateBytes123456789012")],
                     [token(name=SYNTHETIC_SECRET)]):
            with self.assertRaises(MODULE.ProposalError) as caught:
                MODULE.plan_api_keys(*inputs(rows))
            self.assertNotIn(SYNTHETIC_SECRET, str(caught.exception))

    def test_cli_cannot_accept_unverified_source_labels_without_frozen_export_proof(self):
        source, target = inputs()
        with tempfile.TemporaryDirectory() as folder:
            paths = [Path(folder) / "source.json", Path(folder) / "target.json"]
            for path, data in zip(paths, (source, target)):
                path.write_text(json.dumps(data))
                path.chmod(0o600)
            command = [sys.executable, "-B", str(SCRIPT), "--source", str(paths[0]), "--target", str(paths[1])]
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 2)
            self.assertNotIn(SYNTHETIC_SECRET, result.stdout + result.stderr)
            paths[0].chmod(0o644)
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 2)
            self.assertNotIn(SYNTHETIC_SECRET, result.stdout + result.stderr)

    def test_source_projection_is_bound_to_actual_export_columns_rows_and_runtime(self):
        source, _ = inputs()
        tables = {"tokens": source["tokens"], "users": [{**source["users"][0], "setting": '{"billing_preference":"wallet_first"}'}],
                  "options": [{"key": key, "value": json.dumps(source["policy"]["group_authority"][name])}
                              for name, key in (("user_usable_groups", "UserUsableGroups"),
                                                ("special_usable_groups", "group_ratio_setting.group_special_usable_group"),
                                                ("group_ratios", "GroupRatio"), ("group_group_ratios", "GroupGroupRatio"))]}
        del tables["users"][0]["billing_preference"]
        proof = {"source_archive_sha256": "a" * 64, "source_export_sha256": "c" * 64,
                 "source_column_schema": {"tokens": [{"column_name": name} for name in sorted(MODULE.TOKEN_FIELDS)], "users": [], "options": []},
                 "source_instance": {"database": "synthetic"}, "metadata_hashes": {"baseline": "e" * 64}}
        observed = {"source_revision": MODULE.LEGACY_REVISION, "source_archive_sha256": "a" * 64, "buildinfo_matches_live_binary": True}
        provenance = {"source_receipt_sha256": "1" * 64, "source_export_proof_sha256": "2" * 64, "legacy_runtime_observation_sha256": "3" * 64}
        result, receipt = MODULE.source_projection_from_export(tables, proof, snapshot_at=1000,
                                                              runtime_observation=observed, provenance=provenance)
        self.assertEqual(result["users"], source["users"])
        self.assertEqual(result["tokens"], source["tokens"])
        self.assertEqual(result["source_projection_sha256"], MODULE.sha256(receipt))
        self.assertEqual(receipt["snapshot_at"], 1000)
        _, changed_time = MODULE.source_projection_from_export(tables, proof, snapshot_at=1001,
                                                               runtime_observation=observed, provenance=provenance)
        self.assertNotEqual(MODULE.sha256(changed_time), MODULE.sha256(receipt))
        self.assertEqual(result["source_export_sha256"], "c" * 64)
        self.assertEqual(receipt["source_row_counts"], {"tokens": 1, "users": 1, "options": 4})
        self.assertEqual(result["policy"]["group_authority"], source["policy"]["group_authority"])
        self.assertEqual(result["policy"]["source_options_sha256"], MODULE.sha256(tables["options"]))
        incomplete = {**tables, "options": tables["options"][:-1]}
        with self.assertRaisesRegex(MODULE.ProposalError, "source_group_options_incomplete"):
            MODULE.source_projection_from_export(incomplete, proof, snapshot_at=1000,
                                                 runtime_observation=observed, provenance=provenance)
        observed["source_revision"] = "unreviewed"
        with self.assertRaisesRegex(MODULE.ProposalError, "legacy_runtime_source_binding_invalid"):
            MODULE.source_projection_from_export(tables, proof, snapshot_at=1000,
                                                 runtime_observation=observed, provenance=provenance)
        observed["source_revision"] = MODULE.LEGACY_REVISION
        proof["source_column_schema"]["tokens"].pop()
        with self.assertRaisesRegex(MODULE.ProposalError, "source_token_column_coverage_mismatch"):
            MODULE.source_projection_from_export(tables, proof, snapshot_at=1000,
                                                 runtime_observation=observed, provenance=provenance)

    def test_duplicate_json_fields_are_rejected(self):
        with self.assertRaises(MODULE.ProposalError):
            json.loads('{"status":1,"status":2}', object_pairs_hook=MODULE.unique_object)

    def test_private_reader_hashes_the_same_bytes_it_parses(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "private.json"
            data = b'{"version": 1}\n'
            path.write_bytes(data)
            path.chmod(0o600)
            parsed, digest = MODULE.private_json_with_digest(path)
            self.assertEqual(parsed, {"version": 1})
            self.assertEqual(digest, hashlib.sha256(data).hexdigest())

    def test_upstream_private_verifier_exception_cannot_echo_private_input(self):
        class UpstreamMappingError(Exception):
            pass

        def invalid_instance(*arguments, **options):
            raise ValueError(SYNTHETIC_SECRET)

        fake_module = SimpleNamespace(F=SimpleNamespace(MappingError=UpstreamMappingError),
                                      load_routing_export=invalid_instance)
        fake_spec = SimpleNamespace(loader=SimpleNamespace(exec_module=lambda module: None))
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ("receipt.json", "proof.json", "runtime.json")]
            for path in paths:
                path.write_text("{}")
                path.chmod(0o600)
            with patch.object(MODULE.importlib.util, "spec_from_file_location", return_value=fake_spec), \
                 patch.object(MODULE.importlib.util, "module_from_spec", return_value=fake_module), \
                 self.assertRaises(MODULE.ProposalError) as caught:
                MODULE.source_export_provenance_cli(*paths, snapshot_at=1000)
            self.assertEqual(str(caught.exception), "frozen_source_export_verification_failed")
            self.assertNotIn(SYNTHETIC_SECRET, str(caught.exception))


if __name__ == "__main__":
    unittest.main()
