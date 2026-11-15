# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.

import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import legacy_site_statistics as S
import migrate_legacy_available_wallet as W


def canonical(v):
    return json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def sha_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


class PartialOpeningAdapterTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.root.chmod(0o700)

        self.user_row = {"id": 1, "used_quota": 100, "request_count": 5, "aff_history": 0}
        self.tables = {
            "users": [copy.deepcopy(self.user_row)],
            "top_ups": [],
            "subscription_orders": [],
            "wallet_credit_rebases": [],
            "tokens": [],
            "violation_fee_records": [],
        }
        self.counts = {k: len(v) for k, v in self.tables.items()}
        self.archive_path = self._write_priv("archive.dump", b"synthetic-archive")
        self.archive_sha = sha_bytes(b"synthetic-archive")
        self.export_path = self._write_priv("export.json", canonical(self.tables).encode())
        self.export_sha = sha_bytes(canonical(self.tables).encode())

        self.plan_mapping_sha = "b" * 64  # distinct full-plan digest (bce)
        self.source_plan = {
            "source_archive_sha256": self.archive_sha,
            "mapping_sha256": self.plan_mapping_sha,
            "wallet_positions": [{
                "source_user_id": 1,
                "account_id": 101,
                "signed_available_units": 500,
                "source_row_sha256": S.F.digest(self.user_row),
            }],
            "source_evidence": {
                k: {"row_hashes": [S.F.digest(r) for r in v]} for k, v in self.tables.items()
            },
        }
        self.receipt = {
            "status": "snapshot-restored-baseline-verified-frozen-and-classified",
            "archive": {"path": str(self.archive_path), "sha256": self.archive_sha},
        }
        self.proof = {
            "source_archive_sha256": self.archive_sha,
            "source_export_sha256": self.export_sha,
            "source_column_coverage_exact": True,
            "source_row_baseline_exact": True,
            "metadata_hashes": {"m": "1" * 64},
            "selected_table_counts": self.counts,
            "source_instance": "synthetic-inst",
        }
        self.identity_records = [{"source_user_id": 1, "target_user_id": 1, "target_personal_account_id": 101}]

        self.receipt_path = self._write_priv("receipt.json", canonical(self.receipt).encode())
        self.proof_path = self._write_priv("proof.json", canonical(self.proof).encode())
        self.plan_path = self._write_priv("source-plan.json", canonical(self.source_plan).encode())
        self.id_map_path = self._write_priv("identity-map.jsonl", (canonical(self.identity_records[0]) + "\n").encode())

        positions = [{
            "source_user_id": 1,
            "target_user_id": 1,
            "account_id": 101,
            "available_units": 500,
            "source_row_sha256": S.F.digest(self.user_row),
        }]
        bound = {
            "scope": W.SCOPE,
            "policy": W.POLICY,
            "source_archive_sha256": self.archive_sha,
            "source_plan_mapping_sha256": self.plan_mapping_sha,
            "positions": positions,
        }
        self.partial_mapping_sha = W.digest(bound)  # distinct partial-opening digest (3eb)
        self.observed_at = 1794679637
        self.migration_id = "fresh69-26752cda-wallet-available-v1"
        self.opening = {
            **bound,
            "format_version": 1,
            "migration_id": self.migration_id,
            "mapping_sha256": self.partial_mapping_sha,
            "observed_at": self.observed_at,
            "identity_import": {"migration_id": "id-only", "source_archive_sha256": self.archive_sha},
            "source_review": {
                "scope": W.SCOPE,
                "policy": W.POLICY,
                "evidence_sha256": self.partial_mapping_sha,
            },
            "source_files": {"source_export": {"sha256": self.export_sha}},
        }
        self.opening_path = self._write_priv("partial-opening.json", canonical(self.opening).encode())

        self.adapter = {
            "scope": W.SCOPE,
            "policy": W.POLICY,
            "source_archive_sha256": self.archive_sha,
            "mapping_sha256": self.partial_mapping_sha,
            "source_plan_mapping_sha256": self.plan_mapping_sha,
            "source_files": {
                "archive": {"path": str(self.archive_path), "sha256": self.archive_sha},
                "source_receipt": {"path": str(self.receipt_path), "sha256": sha_bytes(self.receipt_path.read_bytes())},
                "source_export": {"path": str(self.export_path), "sha256": self.export_sha},
                "source_export_proof": {"path": str(self.proof_path), "sha256": sha_bytes(self.proof_path.read_bytes())},
                "identity_map": {"path": str(self.id_map_path), "sha256": sha_bytes(self.id_map_path.read_bytes())},
                "source_plan": {"path": str(self.plan_path), "sha256": sha_bytes(self.plan_path.read_bytes())},
            },
        }
        self.adapter_path = self._write_priv("partial-adapter.json", canonical(self.adapter).encode())

        counter_req = {
            "account_id": 101,
            "user_id": 1,
            "migration_id": self.migration_id,
            "source_entity_key": "legacy.users/1",
            "source_archive_sha256": self.archive_sha,
            "mapping_sha256": self.partial_mapping_sha,
            "source_row_sha256": S.F.digest(self.user_row),
            "observed_at": self.observed_at,
            "used_quota": 100,
            "request_count": 5,
            "aff_history": 0,
        }
        self.snapshot = {
            "native_users": [{"id": 1, "personal_account_id": 101}],
            "opening_counters": [{**counter_req, "request": copy.deepcopy(counter_req)}],
            "import_runs": [self.opening["identity_import"]],
            "finance": {"sources": [], "users": [], "topups": []},
            "source_namespace": {"source_count": 0, "source_keys": [], "archive_sha256s": [], "user_rows": 0, "topup_rows": 0},
            "source_seals": [],
            "native_intent_upper": 0,
            "native_intent_max_id": 0,
            "native_request_ids": [],
            "state": {
                "sequences": {
                    S.INTENT_SEQUENCE: {"increment": 1, "cycle": False, "is_called": False, "last_value": 1}
                },
                "tables": {"core_charging.requests": {"rows": 0}},
            },
        }

    def tearDown(self):
        self.tmp.cleanup()

    def _write_priv(self, name: str, data: bytes) -> Path:
        p = self.root / name
        p.write_bytes(data)
        p.chmod(0o600)
        return p

    def _run_prepare(self, opening_path=None, adapter_path=None, snapshot=None):
        with patch.object(S.I, "BACKUP_ROOT", self.root.parent), \
             patch.object(S.P, "metadata", return_value=({k: [] for k in self.tables}, self.counts, {"m": "1" * 64}, None, None)), \
             patch.object(S.P, "read_frozen_export", return_value=copy.deepcopy(self.tables)), \
             patch.object(S.P, "verify_plan_mapping", return_value=None), \
             patch.object(S.P, "verify_target_identity_binding", return_value=None):
            return S.prepare(
                opening_path or self.opening_path,
                snapshot if snapshot is not None else self.snapshot,
                ["app_runtime"],
                partial_adapter_path=adapter_path if adapter_path is not None else self.adapter_path,
            )

    def test_partial_opening_binds_partial_mapping_and_reconciles(self):
        material = self._run_prepare()
        plan = material["plan"]
        self.assertEqual(plan["source"]["mapping_sha256"], self.partial_mapping_sha)
        self.assertNotEqual(plan["source"]["mapping_sha256"], self.plan_mapping_sha)
        self.assertEqual(plan["source"]["observed_at"], self.observed_at)
        self.assertEqual(material["reconciliation"]["source_key"], plan["source"]["source_key"])

    def test_rejects_full_plan_mapping_substitution(self):
        bad_opening = copy.deepcopy(self.opening)
        bad_opening["mapping_sha256"] = self.plan_mapping_sha
        bad_opening["source_review"]["evidence_sha256"] = self.plan_mapping_sha
        bad_path = self._write_priv("bad-opening.json", canonical(bad_opening).encode())
        with self.assertRaises(Exception):
            self._run_prepare(opening_path=bad_path)

    def test_rejects_mismatched_counter_lineage(self):
        bad_snap = copy.deepcopy(self.snapshot)
        bad_snap["opening_counters"][0]["request"]["mapping_sha256"] = self.plan_mapping_sha
        with self.assertRaisesRegex(S.Error, "partial_opening_counter_lineage_mismatch"):
            self._run_prepare(snapshot=bad_snap)

    def test_rejects_mismatched_source_plan_digest(self):
        bad_plan = copy.deepcopy(self.source_plan)
        bad_plan["mapping_sha256"] = "f" * 64
        bad_plan_path = self._write_priv("bad-plan.json", canonical(bad_plan).encode())
        bad_adapter = copy.deepcopy(self.adapter)
        bad_adapter["source_files"]["source_plan"] = {
            "path": str(bad_plan_path),
            "sha256": sha_bytes(bad_plan_path.read_bytes()),
        }
        bad_adapter_path = self._write_priv("bad-adapter.json", canonical(bad_adapter).encode())
        with self.assertRaisesRegex(S.Error, "partial_source_plan_mismatch"):
            self._run_prepare(adapter_path=bad_adapter_path)

    def test_rejects_missing_partial_adapter_for_partial_opening(self):
        with patch.object(S.I, "BACKUP_ROOT", self.root.parent):
            with self.assertRaisesRegex(S.Error, "partial_adapter_required"):
                S.load_source(self.opening_path, partial_adapter_path=None, snapshot=self.snapshot)


if __name__ == "__main__":
    unittest.main(verbosity=2)
