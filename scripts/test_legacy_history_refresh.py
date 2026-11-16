#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Focused synthetic append-only legacy history refresh contracts."""
import json
from pathlib import Path
import tempfile
import unittest

import legacy_history_refresh as R

BINDING = {
    "database": "synthetic_history", "database_oid": 42, "schema_version": 6,
    "schema_fingerprint": "b" * 64, "owner": "synthetic_owner",
}


def raw(log_id=1, user_id=71, **changes):
    return {
        "id": log_id, "user_id": user_id, "token_id": 9, "created_at": 1700000010,
        "type": 2, "quota": 17, "prompt_tokens": 3, "completion_tokens": 4,
        "username": "合成用户", "token_name": "retired-key", "model_name": "synthetic",
        "channel_id": 2, "group": "default", "request_id": "request",
        "upstream_request_id": "upstream",
        "other": '{"pricing_credits_per_usd":500000,"secret":"never-persist"}',
        **changes,
    }


def write_export(path, records):
    path.write_text("".join(json.dumps({"kind": kind, "data": row}, ensure_ascii=False) + "\n"
                            for kind, row in records))


def target_records(extra_user=False, orphan=365):
    mapping = [{"source_user_id": 71, "target_user_id": 71, "target_personal_account_id": 1}]
    archive = "a" * 64
    mapping_hash = R.H.sha({"identities": mapping, "key_policy": R.KEY_POLICY,
                           "source_archive": archive})
    source = {
        "contract_version": "core.history-source.v1", "source_archive_sha256": archive,
        "mapping_sha256": mapping_hash, "source_namespace": "legacy.logs:legacy_source",
        "observed_at": 1700000020, "complete": True,
    }
    row = R.normalize(raw(), (71, 1))
    row.update(source_id=1, canonical_event_key=f"legacy.logs:{archive}:1")
    scope = {
        "source_id": 1, "user_id": 71, "owner_account_id": 1, "token_id": None,
        "source_archive_sha256": archive, "mapping_sha256": mapping_hash,
        "source_scope_sha256": "c" * 64, "complete": True,
    }
    manifest = {
        "scope": "live-owner-history-no-legacy-key-binding.v1", "source": source,
        "identity_map": mapping, "orphans": [{"table": "logs", "source_user_id": orphan}],
    }
    rows = [
        ("metadata", {"database": BINDING["database"], "observed_at": 1700000030}),
        ("sources", {"source_id": 1, **source}),
        ("users", {"id": 71, "personal_account_id": 1}),
        ("log_records", row), ("scope_receipts", scope),
        ("import_runs", {
            "migration_id": "original-history", "source_archive_sha256": archive,
            "mapping_sha256": mapping_hash, "manifest": manifest,
        }),
    ]
    if extra_user:
        rows.append(("users", {"id": 72, "personal_account_id": 2}))
    return rows


def source_records(logs=None, users=(71,), complete=True):
    logs = [raw(), raw(2)] if logs is None else logs
    rows = [("metadata", {"database": "source", "schema": "legacy_source",
                           "observed_at": 1700000040})]
    rows.extend(("logs", row) for row in logs)
    rows.extend(("users", {"id": uid}) for uid in users)
    if complete:
        rows.append(("capture_complete", {"logs": len(logs), "users": len(users)}))
    return rows


class HistoryRefreshTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / "source.jsonl"
        self.target = self.root / "target.jsonl"

    def plan(self, logs=None, users=(71,), target=None, extra=()):
        write_export(self.source, source_records(logs, users))
        write_export(self.target, target_records() if target is None else target)
        return R.prepare(self.source, R.read_target(self.target), BINDING, extra)

    def test_append_preserves_original_rows_and_key_retirement(self):
        plan = self.plan()
        self.assertEqual(plan["manifest"]["summary"]["new_logs"], 1)
        self.assertEqual(plan["manifest"]["summary"]["existing_logs_matched"], 1)
        self.assertEqual(plan["rows"][0]["source_log_id"], 2)
        self.assertIsNone(plan["rows"][0]["token_id"])
        self.assertEqual(plan["rows"][0]["pricing_credits_per_usd"], 500000)
        self.assertEqual(plan["manifest"]["summary"]["delta_consume_quota"], 17)
        sql = R.build_sql(plan)
        self.assertIn("history_target_not_offline", sql)
        self.assertIn("history_target_state_changed:log_records", sql)
        self.assertIn("EXCEPT ALL", sql)
        self.assertNotIn("never-persist", sql)
        for table in ("core_billing", "core_charging", "core_identity.password_logins"):
            self.assertNotIn("INSERT INTO " + table, sql)

    def test_existing_source_row_changes_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "existing_log_changed:1"):
            self.plan([raw(quota=18), raw(2)])

    def test_log_ids_are_compared_without_a_high_watermark_shortcut(self):
        plan = self.plan([raw(0 + 1), raw(3), raw(2)])
        self.assertEqual([r["source_log_id"] for r in plan["rows"]], [2, 3])
        with self.assertRaisesRegex(ValueError, "duplicate_source_log"):
            self.plan([raw(), raw(2), raw(2)])

    def test_new_source_user_requires_explicit_existing_target_identity(self):
        with self.assertRaisesRegex(ValueError, "source_users_need_identity_mapping:72"):
            self.plan([raw(), raw(2, 72)], users=(71, 72))
        with self.assertRaisesRegex(ValueError, "target_identity_coverage_incomplete"):
            self.plan([raw(), raw(2, 72)], users=(71, 72), target=target_records(extra_user=True))

    def test_new_user_old_source_scope_has_committed_absence_evidence(self):
        extra = [{"source_user_id": 72, "target_user_id": 72, "target_personal_account_id": 2}]
        plan = self.plan([raw(), raw(2, 72)], users=(71, 72),
                         target=target_records(extra_user=True), extra=extra)
        self.assertEqual(len(plan["prior_scopes"]), 1)
        scope = plan["prior_scopes"][0]
        self.assertEqual((scope["source_id"], scope["user_id"], scope["owner_account_id"]), (1, 72, 2))
        self.assertEqual(scope["source_archive_sha256"], "a" * 64)
        self.assertEqual(len(plan["scopes"]), 2)
        with self.assertRaisesRegex(ValueError, "prior_source_absence_unproven"):
            self.plan([raw(), raw(2, 72)], users=(71, 72),
                      target=target_records(extra_user=True, orphan=72), extra=extra)

    def test_only_previously_reviewed_orphan_user_is_excluded(self):
        plan = self.plan([raw(), raw(2), raw(3, 365, quota=0)])
        self.assertEqual(plan["manifest"]["summary"]["orphan_logs_excluded"], 1)
        self.assertEqual(plan["manifest"]["summary"]["new_logs"], 1)
        with self.assertRaisesRegex(ValueError, "unreviewed_orphan_log_user"):
            self.plan([raw(), raw(2, 366, quota=0)])

    def test_unknown_old_price_remains_unknown(self):
        plan = self.plan([raw(), raw(2, other='{"secret":"never-persist"}')])
        self.assertIsNone(plan["rows"][0]["pricing_credits_per_usd"])

    def test_complete_capture_required(self):
        write_export(self.target, target_records())
        write_export(self.source, source_records(complete=False))
        with self.assertRaisesRegex(ValueError, "source_capture_incomplete"):
            R.prepare(self.source, R.read_target(self.target), BINDING)
        rows = source_records()
        rows[-1][1]["logs"] = 3
        write_export(self.source, rows)
        with self.assertRaisesRegex(ValueError, "source_capture_counts_mismatch"):
            R.prepare(self.source, R.read_target(self.target), BINDING)
        self.source.write_text(self.source.read_text().rstrip("\n"))
        with self.assertRaisesRegex(ValueError, "truncated_export_line"):
            R.prepare(self.source, R.read_target(self.target), BINDING)

    def test_original_map_is_bound_to_committed_source(self):
        rows = target_records()
        for kind, row in rows:
            if kind == "import_runs":
                row["manifest"]["identity_map"][0]["target_personal_account_id"] = 2
        with self.assertRaisesRegex(ValueError, "history_mapping_hash_mismatch"):
            self.plan(target=rows)

    def test_replanning_same_snapshot_does_not_duplicate_history(self):
        first = self.plan()
        m = first["manifest"]
        after = target_records()
        after.append(("sources", {"source_id": 2, **m["source"]}))
        after.extend(("log_records", {"source_id": 2, **r}) for r in first["rows"])
        after.extend(("scope_receipts", {**r, "source_id": 2}) for r in first["scopes"])
        after.append(("import_runs", {
            "migration_id": first["migration_id"],
            "source_archive_sha256": m["source"]["source_archive_sha256"],
            "mapping_sha256": m["source"]["mapping_sha256"], "manifest": m,
        }))
        second = self.plan(target=after)
        self.assertEqual(second["rows"], [])
        self.assertEqual(second["prior_scopes"], [])
        self.assertIn("HISTORY_ALREADY_CURRENT", R.build_sql(second))

    def test_replan_accepts_same_explicit_additional_identity(self):
        extra = [{"source_user_id": 72, "target_user_id": 72, "target_personal_account_id": 2}]
        records = target_records(extra_user=True)
        first = self.plan([raw(), raw(2, 72)], users=(71, 72), target=records, extra=extra)
        m = first["manifest"]
        records.append(("sources", {"source_id": 2, **m["source"]}))
        records.extend(("log_records", {"source_id": 2, **r}) for r in first["rows"])
        records.extend(("scope_receipts", {**r, "source_id": 2}) for r in first["scopes"])
        records.extend(("scope_receipts", r) for r in first["prior_scopes"])
        records.append(("import_runs", {
            "migration_id": first["migration_id"],
            "source_archive_sha256": m["source"]["source_archive_sha256"],
            "mapping_sha256": m["source"]["mapping_sha256"], "manifest": m,
        }))
        second = self.plan([raw(), raw(2, 72)], users=(71, 72), target=records, extra=extra)
        self.assertEqual(second["rows"], [])
        self.assertEqual(second["prior_scopes"], [])

    def test_preexisting_legacy_key_binding_requires_its_own_review(self):
        rows = target_records()
        for kind, row in rows:
            if kind == "log_records":
                row["token_id"] = 9
        with self.assertRaisesRegex(ValueError, "legacy_key_binding_present"):
            self.plan(target=rows)


if __name__ == "__main__":
    unittest.main()
