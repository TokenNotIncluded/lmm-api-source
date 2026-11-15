# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import unittest

spec = importlib.util.spec_from_file_location("site_statistics_jsonb_candidate", Path(__file__).with_name("legacy_site_statistics.py"))
C = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = C
spec.loader.exec_module(C)


def plan():
    return {"version": 1, "kind": "offline_credit_balance_rebase_preview",
            "migration_id": "private-synthetic-rebase", "has_complete_history": True,
            "usd_credit_conversion": 500000, "snapshot_at": 10, "divisor": "2",
            "rounding": "half-away-from-zero", "user_ids": [1],
            "user_sources": [{"id": 1, "used_quota": 100}], "token_sources": [],
            "other_credit_bases": [], "unknown_original_field": {"retained": [1, 2]}}


def tables(raw):
    return {"users": [{"id": 1, "used_quota": 125}, {"id": 2, "used_quota": 17}],
            "wallet_credit_rebases": [{"plan": raw}], "tokens": [], "violation_fee_records": []}


class JSONBPlanTests(unittest.TestCase):
    def test_both_formats_same_exact_usage_and_original_evidence_unchanged(self):
        hashes = []
        for raw in (plan(), json.dumps(plan(), ensure_ascii=False, indent=2)):
            with self.subTest(kind=type(raw).__name__):
                original = tables(raw); before = copy.deepcopy(original)
                digest = C.F.digest(original)
                byte_hash = hashlib.sha256(json.dumps(original, sort_keys=True).encode()).hexdigest()
                self.assertEqual(C.normalized_usage(original), {1: 75, 2: 17})
                self.assertEqual(original, before)
                self.assertEqual(C.F.digest(original), digest)
                self.assertEqual(hashlib.sha256(json.dumps(original, sort_keys=True).encode()).hexdigest(), byte_hash)
                hashes.append(digest)
        # Text and JSONB remain distinct original evidence, never rewritten to match.
        self.assertNotEqual(hashes[0], hashes[1])

    def test_reject_non_object_json_and_non_supported_python_values(self):
        for raw in (None, False, 1, [], b"{}", "null", "[]", "true", "1", "not JSON"):
            with self.subTest(raw_type=type(raw).__name__, value=repr(raw)):
                with self.assertRaises(C.Error):
                    C.normalized_usage(tables(raw))

    def test_semantic_guards_still_reject_both_formats(self):
        for field, value in (("version", 2), ("has_complete_history", False),
                             ("usd_credit_conversion", 1), ("usd_credit_conversion", 500000.0),
                             ("divisor", "1"), ("rounding", "guessed"),
                             ("snapshot_at", 0), ("kind", "different")):
            bad = plan(); bad[field] = value
            for raw in (bad, json.dumps(bad)):
                with self.subTest(field=field, value=value, raw_type=type(raw).__name__):
                    with self.assertRaises(C.Error):
                        C.normalized_usage(tables(raw))

    def test_missing_original_user_baseline_rejects(self):
        bad = plan(); bad["user_sources"] = []
        for raw in (bad, json.dumps(bad)):
            with self.assertRaises(C.Error):
                C.normalized_usage(tables(raw))

    def test_unproven_counter_reduction_and_safe_bound_reject(self):
        for raw in (plan(), json.dumps(plan())):
            for used in (99, 9007199254740992):
                bad = tables(raw); bad["users"][0]["used_quota"] = used
                with self.subTest(used=used, raw_type=type(raw).__name__):
                    with self.assertRaises(C.Error):
                        C.normalized_usage(bad)


if __name__ == "__main__":
    unittest.main(verbosity=2)
