#!/usr/bin/env python3
"""Explicit pure acceptance for the frozen Core81 ordinary-key adapter.

Requires reviewed support directories and the adapter release SHA256. Runs the
seven loader checks before importing fixtures for thirteen handoff checks.
No database, approval or production execution is performed.
"""
from copy import deepcopy
from contextlib import contextmanager
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--support-scripts", type=Path, action="append", required=True)
    parser.add_argument("--test-support-scripts", type=Path, required=True)
    parser.add_argument("--expected-adapter-sha256", required=True)
    args = parser.parse_args()
    args.support_scripts = [path.resolve() for path in args.support_scripts]
    args.test_support_scripts = args.test_support_scripts.resolve()
    if not all(path.is_dir() for path in [*args.support_scripts, args.test_support_scripts]):
        parser.error("all explicit support directories must exist")
    return args


ARGS = arguments()
SUPPORT_SCRIPTS = ARGS.support_scripts
ADAPTER_SHA256 = ARGS.expected_adapter_sha256
os.environ["LMM_PARTITION_TEST_SUPPORT"] = str(ARGS.test_support_scripts)
for support in reversed(SUPPORT_SCRIPTS):
    sys.path.insert(0, str(support))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import partitioned_keys81 as M

M.load_support(SUPPORT_SCRIPTS, expected_adapter_sha256=ADAPTER_SHA256)


@contextmanager
def changed_module_file(module):
    """Change only a disposable file; never run its modified source."""
    with TemporaryDirectory(prefix="partition81-dependency-test-") as directory:
        original = Path(module.__file__)
        changed = Path(directory) / original.name
        changed.write_bytes(original.read_bytes() + b"\n# Synthetic dependency tamper\n")
        with patch.object(module, "__file__", str(changed)):
            yield


class StableSupportLoaderTests(unittest.TestCase):
    def test_reviewed_support_bundle_and_adapter_release_are_loaded(self):
        loaded = M.load_support(SUPPORT_SCRIPTS, expected_adapter_sha256=ADAPTER_SHA256)
        self.assertEqual(loaded["adapter_version"], "lmm.partitioned-keys81-owner-adapter.v1")
        self.assertEqual(loaded["adapter_sha256"], ADAPTER_SHA256)
        self.assertEqual(loaded["frozen_helpers"]["final81_target_guard.py"],
            "5b928e9910fe4648b4e4d846625bb141eb880b5a5f864d0ef1e11ac662a6a193")
        self.assertEqual(loaded["frozen_helpers"]["final81_execution.py"],
            "b47735d642a2efdbd24c9c7c68b9a5ac1f3f47067fc04ab6029e1f8aaa3655dc")
        self.assertEqual(loaded["frozen_helpers"]["current_connection_history.py"],
            "45d13a84800a8c14e2d9171bfcd9d417e7f18705b54beb8d764c874e4ec34562")
        self.assertEqual(loaded["frozen_helpers"]["reviewed-fresh66-credit-quotes.json"],
            "bb76fe088c7925370043246a64bbd179f1642f520d4ba9ee9c96753920a7152e")
        self.assertFalse(loaded["full_execution_qualified"])

    def test_wrong_adapter_release_pin_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "partition81_adapter_bytes_changed"):
            M.load_support(SUPPORT_SCRIPTS, expected_adapter_sha256="0" * 64)

    def test_changed_adapter_file_is_rejected_after_load(self):
        with changed_module_file(M):
            with self.assertRaisesRegex(ValueError, "partition81_adapter_bytes_changed"):
                M.dependencies()

    def test_changed_partition_module_is_rejected(self):
        with changed_module_file(M.S):
            with self.assertRaisesRegex(ValueError, "partition81_dependency_changed"):
                M.dependencies()

    def test_changed_execution_module_is_rejected(self):
        with changed_module_file(M.E):
            with self.assertRaisesRegex(ValueError, "partition81_dependency_changed"):
                M.dependencies()

    def test_changed_target_guard_is_rejected_by_complete_frozen_loader(self):
        with changed_module_file(M.A.T):
            with self.assertRaisesRegex(M.H.Error, "connection_frozen_validator_changed"):
                M.dependencies()

    def test_changed_history_loader_is_rejected_before_its_validation(self):
        with changed_module_file(M.H):
            with patch.object(M.H, "need", lambda *_: None):
                with self.assertRaisesRegex(ValueError, "partition81_history_loader_changed"):
                    M.dependencies()


def fixture(*, live=False, recoverable=False):
    parent, provenance, obs = PF.fixture()
    parent["tokens"][0]["one_time_reveal"] = not recoverable
    if not live:
        obs["oauth_server_grants"][0]["revoked_at_ms"] = 50_000
    context, acquisition = PF.bind(parent, provenance, obs)
    acquisition["instance"] = {"database": "synthetic_source", "database_oid": 42,
                               "system_identifier": "9988776655"}
    partition, view, archive = PF.S.assemble(PF.P, PF.F, PF.C, parent, provenance,
        context, obs, acquisition)
    fence = {"namespace": KT.NAMESPACE, "observed_at": "1970-01-01T00:02:00+00:00",
        "transaction_read_only": "on", "database_allows_connections": False,
        "other_database_backends": 0, "prepared_transactions": 0}
    return {"parent": parent, "provenance": provenance,
        "canonical_tables": {t: deepcopy(obs[t]) for t in PF.S.TABLES},
        "partition": partition, "view": view, "archive": archive,
        "observation": obs, "acquisition": acquisition, "fence": fence,
        "origin": "SYNTHETIC_PURE_TEST"}


def target(source):
    _, result = KT.inputs()
    result["source_archive_sha256"] = source["source_archive_sha256"]
    result["source_target_id_map"][0]["source_user_id"] = 1
    result["source_target_id_map_sha256"] = hashlib.sha256(
        M.P.id_map_bytes(result["source_target_id_map"]).encode()).hexdigest()
    result["users"][0]["auth_version"] = 1
    return result


class PartitionHandoffTests(unittest.TestCase):
    def test_frozen_dependency_contract_is_loaded(self):
        M.dependencies()

    def test_complete_partition_reaches_unchanged_real_mapper(self):
        data = fixture()
        before = deepcopy(data)
        view = M.handoff(**data)
        keys = CT.prepare(view, target(view), {}, {})
        self.assertEqual([r["source_token_id"] for r in keys["records"]], [1])
        self.assertEqual(keys["records"][0]["quota"]["remaining"], -15)
        self.assertEqual(keys["records"][0]["quota"]["opening_consumed"], 70)
        self.assertEqual([r["id"] for r in data["archive"]["rows"]], [2, 3])
        self.assertEqual(data, before)
        self.assertFalse(keys["production_authorized"])

    def test_unused_live_refresh_blocks_key_preparation(self):
        with self.assertRaisesRegex(ValueError, "partition81_oauth_continuation_unresolved"):
            M.handoff(**fixture(live=True))

    def test_final_source_grant_drift_cannot_hide_behind_matching_tokens(self):
        data = fixture()
        data["canonical_tables"]["oauth_server_grants"][0]["revoked_at_ms"] = 0
        with self.assertRaisesRegex(ValueError, "partition81_oauth_not_final_canonical_source"):
            M.handoff(**data)

    def test_missing_complete_oauth_table_rejected(self):
        data = fixture()
        del data["canonical_tables"]["oauth_server_authorizations"]
        with self.assertRaisesRegex(ValueError, "partition81_oauth_not_final_canonical_source"):
            M.handoff(**data)

    def test_derived_source_tamper_rejected(self):
        data = fixture()
        data["view"]["tokens"][0]["remain_quota"] = 0
        with self.assertRaisesRegex(ValueError, "saved_partition_not_exact_rebuild"):
            M.handoff(**data)

    def test_foreign_instance_fence_rejected(self):
        data = fixture()
        data["fence"]["namespace"] = {**KT.NAMESPACE, "system_identifier": "different"}
        with self.assertRaisesRegex(ValueError, "partition81_source_instance_changed"):
            M.handoff(**data)

    def test_open_source_fence_rejected(self):
        data = fixture()
        data["fence"]["database_allows_connections"] = True
        with self.assertRaisesRegex(ValueError, "partition81_source_fence_not_held"):
            M.handoff(**data)

    def test_later_oauth_observation_cannot_claim_prior_fence(self):
        data = fixture()
        data["fence"]["observed_at"] = "1970-01-01T00:00:59+00:00"
        with self.assertRaisesRegex(ValueError, "partition81_oauth_observation_after_final_fence"):
            M.handoff(**data)

    def test_reviewed_ciphertext_rebuild_preserves_nonce_and_original_plaintext(self):
        view = M.handoff(**fixture(recoverable=True))
        key = bytes(range(32))
        saved = CT.prepare(view, target(view), {}, {})
        rebuilt = CT.prepare(view, target(view), {}, {})
        self.assertNotEqual(saved["records"][0]["encrypted_secret_hex"],
                            rebuilt["records"][0]["encrypted_secret_hex"])
        M.retain_ciphertexts(rebuilt, saved, view, key)
        self.assertEqual(rebuilt, saved)
        M.C.verify_encrypted_secret(key, rebuilt["records"][0], view["tokens"][0]["key"])

    def test_ciphertext_tamper_is_rejected(self):
        view = M.handoff(**fixture(recoverable=True))
        key = bytes(range(32))
        saved = CT.prepare(view, target(view), {}, {})
        rebuilt = CT.prepare(view, target(view), {}, {})
        raw = saved["records"][0]["encrypted_secret_hex"]
        saved["records"][0]["encrypted_secret_hex"] = raw[:-2] + ("00" if raw[-2:] != "00" else "01")
        with self.assertRaisesRegex(M.C.Error, "recovery_ciphertext_verification_failed"):
            M.retain_ciphertexts(rebuilt, saved, view, key)

    def test_execution_rebuilder_rejects_changed_fence_before_source_load(self):
        rebuilder = M.production_rebuilder({"format_version": 1, "scope": M.SCOPE,
            "source_partition": {"format": M.PARTITION_BINDING, "source_fence_sha256": "b" * 64}})
        with self.assertRaisesRegex(ValueError, "partition81_execution_evidence_changed"):
            rebuilder({"facts": {"source_fence_sha256": "c" * 64}})

    def test_partition_audit_metadata_keeps_existing_history_receipt_contract(self):
        expected = HF.H.map_committed_rows(*HF.fixture())
        projection, identities, keys, outer, credentials, accounts = HF.fixture()
        component = keys["manifest"]["material"]
        self.assertEqual(component["scope"], M.SCOPE)
        component["source_partition"] = {
            "format": M.PARTITION_BINDING,
            "material": fixture()["partition"],
            "inputs": {},
            "source_fence_sha256": "b" * 64,
        }
        outer["manifest"]["material"]["component_material"] = deepcopy(component)
        users, records = HF.H.map_committed_rows(
            projection, identities, keys, outer, credentials, accounts)
        self.assertEqual([row["target_credential_id"] for row in records], [17, 18])
        self.assertEqual((users, records), expected)


def run_case(case, expected):
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(case)
    if suite.countTestCases() != expected:
        raise ValueError("partition81_acceptance_test_scope_changed")
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return result.testsRun if result.wasSuccessful() and not result.skipped else None


def main():
    global PF, CT, KT, HF
    loader_count = run_case(StableSupportLoaderTests, 7)
    if loader_count is None:
        return 1
    import test_current_key_source_partition as PF
    import test_current_key_production as CT
    import test_migrate_current_runtime_keys as KT
    import test_current_connection_history as HF
    M.need(PF.S is M.S and PF.P is M.P and PF.R is M.K.R
           and PF.F is M.K.R.F and PF.C is M.C,
           "partition81_fixture_support_objects_changed")
    M.dependencies()
    handoff_count = run_case(PartitionHandoffTests, 13)
    if handoff_count is None:
        return 1
    M.dependencies()
    print(json.dumps({"status": "PURE_CHECKS_PASS", "loader_tests": loader_count,
        "handoff_tests": handoff_count, "full_execution_qualified": False}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
