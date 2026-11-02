# SPDX-License-Identifier: AGPL-3.0-or-later
"""Synthetic file/pure plan/transport tests. No database or network.

Generated SQL assertions do not prove PostgreSQL execution or permissions.
"""

import copy
import hashlib
import io
from decimal import Decimal
from fractions import Fraction
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import legacy_channel_accounting as C


def target():
    inventory = {
        "tables": sorted(
            set(C.F.CHANGED_TABLES)
            | set(C.F.FRESH_EMPTY_TABLES)
            | set(C.DESTINATIONS)
            | set(C.S.DESTINATIONS)
            | {
                C.BALANCES,
                "core_channels.identities",
                "core_channels.counter_baselines",
                "core_channels.projection_state",
                "core_channels.projection_dirty",
                "core_channels.usage_contributions",
                "core_channels.usage_rollups",
                "core_charging.requests",
                "core_migration.import_runs",
                "core_identity.users",
                "core_trust_pricing.policies",
                "public.routing_active_release",
                "public.routing_config_versions",
                "core_payments.intents",
            }
        ),
        "sequences": sorted(set(C.F.CHANGED_SEQUENCES) | {C.S.INTENT_SEQUENCE}),
    }
    return {
        "instance": {
            "database": "synthetic",
            "role": "offline_owner",
            "session_user": "offline_operator",
            "port": 5432,
        },
        "ownership_sha256": "a" * 64,
        "catalog_sha256": "b" * 64,
        "state": {
            "inventory": inventory,
            "tables": {n: {"rows": 0, "sha256": "c" * 64} for n in inventory["tables"]},
            "sequences": {},
        },
        "openings": {"sources": [], "rows": []},
        "balances": [],
        "identities": [{"id": 1, "retired_at": None, "history_ambiguous": False}],
        "active_channel_ids": [1],
        "native_request_ids": ["opaque:01", "opaque:9"],
        "projection_ready": True,
        "dirty": 0,
        "projection_consistent": True,
        "global_unknown": "0",
        "native": [
            {
                "channel_id": 1,
                "gross": "700000",
                "refunded": "100000",
                "pending": "0",
                "unknown": "0",
                "baseline_gross": "200000",
                "baseline_refunded": "50000",
                "historical_coverage": False,
                "source_channel_id": None,
            }
        ],
    }


class ChannelOpeningTests(unittest.TestCase):
    def setUp(self):
        self.raw = [
            {
                "id": 17,
                "used_quota": 7,
                "balance": Decimal("0.125"),
                "balance_updated_time": 0,
                "key": "SYNTHETIC-SECRET-NOT-FOR-MATERIAL",
            }
        ]
        self.files = {
            k: {"path": "/synthetic/" + k, "sha256": str(i + 1) * 64}
            for i, k in enumerate(sorted(C.FILES))
        }
        self.mapping = {
            "format_version": 1,
            "archive_sha256": self.files["archive"]["sha256"],
            "export_sha256": self.files["source_export"]["sha256"],
            "source_revision": "traditional3948",
            "observed_at": None,
            "channels": [
                {
                    "source_channel_id": 17,
                    "channel_id": 1,
                    "source_row_sha256": C.F.digest(self.raw[0]),
                    "historical_coverage": False,
                }
            ],
        }
        self.proofs = {"8" * 64, "9" * 64}
        self.evidence = {
            "contract": C.CONTRACT,
            "archive_sha256": self.mapping["archive_sha256"],
            "export_sha256": self.mapping["export_sha256"],
            "mapping_sha256": self.files["mapping"]["sha256"],
            "rows_sha256": C.F.digest(C.source_rows(self.raw, self.mapping)),
            "native_overlap": "disjoint",
            "overlap_proof_sha256": "8" * 64,
            "channels": [
                {
                    "source_channel_id": 17,
                    "source_row_sha256": self.mapping["channels"][0][
                        "source_row_sha256"
                    ],
                    "credits_per_usd": 3,
                    "basis_kind": "captured_counter_unit",
                    "proof_sha256": "9" * 64,
                    "balance_unit_label": "USD",
                }
            ],
        }
        self.snapshot = target()
        self.addCleanup(patch.stopall)
        patch.object(
            C, "tool_hashes", return_value={"synthetic_tool_sha256": "f" * 64}
        ).start()
        patch.object(
            C,
            "load_source",
            side_effect=lambda *_args, **_kwargs: (
                self.raw,
                self.mapping,
                self.evidence,
                self.proofs,
            ),
        ).start()

    def plan(self):
        return C.build_plan(
            self.raw,
            self.mapping,
            self.evidence,
            self.proofs,
            self.files,
            self.snapshot,
        )

    def material(self):
        return C.prepare(self.files, self.snapshot, ["runtime_reader"])

    def counter(self, value):
        self.raw[0]["used_quota"] = value
        self.mapping["channels"][0]["source_row_sha256"] = C.F.digest(self.raw[0])
        self.evidence["channels"][0]["source_row_sha256"] = C.F.digest(self.raw[0])
        self.evidence["rows_sha256"] = C.F.digest(C.source_rows(self.raw, self.mapping))

    def test_non_native_k_is_exact_rational_not_fractional_native_credits(self):
        plan = self.plan()
        self.assertEqual(plan["status"], "READY")
        self.assertEqual(
            (
                plan["rows"][0]["offset_numerator"],
                plan["rows"][0]["offset_denominator"],
            ),
            ("7", "3"),
        )
        self.assertEqual(plan["rows"][0]["credits_per_usd"], 3)
        self.assertNotIn("SYNTHETIC-SECRET", C.F.canonical(plan))

    def test_signed_int64_min_and_max_are_preserved_without_float_or_clamp(self):
        for raw in (C.I64_MIN, -1, 0, C.I64_MAX):
            with self.subTest(raw=raw):
                self.counter(raw)
                row = self.plan()["rows"][0]
                self.assertEqual(row["raw_used_quota"], str(raw))
                self.assertEqual(
                    Fraction(
                        int(row["offset_numerator"]), int(row["offset_denominator"])
                    ),
                    Fraction(raw, 3),
                )

    def test_missing_counter_is_not_zero(self):
        for raw in (None, True, 1.0, C.I64_MAX + 1):
            with self.subTest(raw=raw), self.assertRaises(C.F.Error):
                self.counter(raw)

    def test_unknown_basis_and_overlap_produce_no_approved_writer(self):
        for change in (
            {"credits_per_usd": None},
            {"basis_kind": "startup_current"},
            {"proof_sha256": "0" * 64},
        ):
            with self.subTest(change=change):
                original = copy.deepcopy(self.evidence["channels"][0])
                self.evidence["channels"][0].update(change)
                material = self.material()
                self.assertEqual(material["plan"]["status"], "UNKNOWN")
                with self.assertRaisesRegex(C.F.Error, "preview_unknown"):
                    C.build_apply_sql(material)
                self.evidence["channels"][0] = original
        self.evidence["native_overlap"] = "unknown"
        self.assertEqual(self.plan()["status"], "UNKNOWN")

    def test_fractional_or_out_of_schema_k_is_unknown_without_rounding(self):
        for k in (Decimal("2.5"), "2.5", 0, C.SAFE + 1):
            with self.subTest(k=k):
                self.evidence["channels"][0]["credits_per_usd"] = k
                plan = self.plan()
                self.assertEqual(plan["status"], "UNKNOWN")
                self.assertIn("basis_outside_installed_schema:1", plan["reasons"])
                self.assertEqual(plan["rows"], [])

    def test_included_subtracts_combined_net_once_including_copied_native_baseline(
        self,
    ):
        self.evidence["native_overlap"] = "included_at_seal"
        plan = self.plan()
        self.assertEqual(plan["status"], "READY")
        # 7/3 - ((700000-100000)+(200000-50000))/500000 = 5/6.
        self.assertEqual(
            (
                plan["rows"][0]["offset_numerator"],
                plan["rows"][0]["offset_denominator"],
            ),
            ("5", "6"),
        )
        self.assertEqual(
            plan["source"]["native_request_ids"], ["opaque:01", "opaque:9"]
        )

    def test_included_pending_unknown_or_global_unknown_blocks_confirmation(self):
        self.evidence["native_overlap"] = "included_at_seal"
        for field in ("pending", "unknown", "global_unknown"):
            with self.subTest(field=field):
                row = (
                    self.snapshot
                    if field == "global_unknown"
                    else self.snapshot["native"][0]
                )
                row[field] = "1"
                self.assertEqual(self.plan()["status"], "UNKNOWN")
                row[field] = "0"

    def test_root_legacy_gap_can_open_but_unknown_copied_baseline_cannot(self):
        self.assertEqual(self.plan()["status"], "READY")
        self.snapshot["native"][0]["source_channel_id"] = 99
        self.assertEqual(self.plan()["status"], "UNKNOWN")
        self.snapshot["native"][0]["historical_coverage"] = True
        self.assertEqual(self.plan()["status"], "READY")

    def test_included_negative_requires_separate_correction_evidence(self):
        self.counter(-1)
        self.evidence["native_overlap"] = "included_at_seal"
        self.assertEqual(self.plan()["status"], "UNKNOWN")
        self.evidence["channels"][0]["negative_offset_proof_sha256"] = "8" * 64
        row = self.plan()["rows"][0]
        self.assertEqual(self.plan()["status"], "READY")
        self.assertEqual(
            Fraction(int(row["offset_numerator"]), int(row["offset_denominator"])),
            Fraction(-11, 6),
        )

    def test_coverage_claim_requires_evidence_not_just_raw_counter(self):
        self.mapping["channels"][0]["historical_coverage"] = True
        self.evidence["rows_sha256"] = C.F.digest(C.source_rows(self.raw, self.mapping))
        self.assertEqual(self.plan()["status"], "UNKNOWN")
        self.evidence["channels"][0]["coverage_proof_sha256"] = "8" * 64
        self.assertEqual(self.plan()["status"], "READY")

    def test_projection_unready_dirty_or_inconsistent_is_unknown(self):
        for field, value in (
            ("projection_ready", False),
            ("dirty", 1),
            ("projection_consistent", False),
        ):
            with self.subTest(field=field):
                old = self.snapshot[field]
                self.snapshot[field] = value
                self.assertEqual(self.plan()["status"], "UNKNOWN")
                self.snapshot[field] = old

    def test_ambiguous_retired_or_inactive_target_is_refused(self):
        for change in ("ambiguous", "retired", "inactive"):
            snap = copy.deepcopy(self.snapshot)
            if change == "ambiguous":
                snap["identities"][0]["history_ambiguous"] = True
            if change == "retired":
                snap["identities"][0]["retired_at"] = "2026-01-01"
            if change == "inactive":
                snap["active_channel_ids"] = []
            with self.subTest(change=change), self.assertRaises(C.F.Error):
                C.build_plan(
                    self.raw, self.mapping, self.evidence, self.proofs, self.files, snap
                )

    def test_source_key_excludes_target_seal_but_plan_binds_it(self):
        first = self.plan()
        self.snapshot["native_request_ids"].append("opaque:new")
        self.snapshot["native"][0]["gross"] = "999999999999999999999999"
        second = self.plan()
        self.assertEqual(first["source"]["source_key"], second["source"]["source_key"])
        self.assertNotEqual(
            first["source"]["native_seal_sha256"],
            second["source"]["native_seal_sha256"],
        )
        self.assertNotEqual(
            first["source"]["plan_sha256"], second["source"]["plan_sha256"]
        )

    def committed(self, material):
        current = copy.deepcopy(material["snapshot"])
        current["openings"] = {
            "sources": [copy.deepcopy(material["plan"]["source"])],
            "rows": copy.deepcopy(material["plan"]["rows"]),
        }
        return current

    def test_noop_preserves_original_seal_and_offset_after_growth_refunds_and_copies(
        self,
    ):
        self.evidence["native_overlap"] = "included_at_seal"
        material = self.material()
        before = copy.deepcopy(material)
        current = self.committed(material)
        current["native_request_ids"].append("opaque:new")
        current["native"][0].update(gross="99999999", refunded="9999999", pending="1")
        current["state"]["tables"]["core_charging.requests"] = {
            "rows": 30,
            "sha256": "e" * 64,
        }
        current["openings"]["rows"].append(
            {
                **material["plan"]["rows"][0],
                "channel_id": 2,
                "kind": "copied",
                "parent_channel_id": 1,
            }
        )
        self.assertEqual(C.reconcile(material, current), "NOOP")
        self.assertEqual(material, before)
        # A fresh plan from the now-running target cannot replace the receipt.
        fresh = C.build_plan(
            self.raw, self.mapping, self.evidence, self.proofs, self.files, current
        )
        self.assertEqual(fresh["status"], "UNKNOWN")
        self.assertEqual(fresh["stored_openings"], current["openings"])
        self.assertNotIn("balance_patches", fresh)
        self.assertNotIn("rows", fresh)

    def test_noop_rejects_changed_receipt_offset_rows_or_source_files(self):
        material = self.material()
        for field in (
            "native_request_ids",
            "plan_sha256",
            "unit_evidence_sha256",
            "mapping_sha256",
        ):
            current = self.committed(material)
            current["openings"]["sources"][0][field] = (
                [] if field == "native_request_ids" else "0" * 64
            )
            with self.subTest(field=field), self.assertRaises(C.F.Error):
                C.reconcile(material, current)
        current = self.committed(material)
        current["openings"]["rows"][0]["offset_numerator"] = "0"
        with self.assertRaises(C.F.Error):
            C.reconcile(material, current)
        self.raw[0]["used_quota"] = 9
        with self.assertRaises(C.F.Error):
            C.reconcile(material, self.committed(material))

    def test_missing_receipt_with_different_target_is_not_safe_replay(self):
        material = self.material()
        current = copy.deepcopy(material["snapshot"])
        current["native_request_ids"].append("opaque:new")
        with self.assertRaises(C.F.Error):
            C.reconcile(material, current)

    def test_original_balance_time_zero_stays_null_and_raw_zero_stays_zero(self):
        self.counter(0)
        row = self.plan()["rows"][0]
        self.assertEqual((row["raw_used_quota"], row["offset_numerator"]), ("0", "0"))
        self.assertIsNone(self.plan()["source"]["observed_at"])
        self.assertIsNone(self.plan()["balance_patches"][0]["balance_updated_time"])
        self.assertEqual(self.plan()["balance_patches"][0]["balance"], "0.125")

    def test_sql_limits_balance_copy_to_existing_unknown_preserves_provider_and_seal(
        self,
    ):
        sql = C.build_apply_sql(self.material())
        self.assertIn("b.kind='unknown' AND b.balance IS NULL", sql)
        self.assertIn(
            "kind='copied',credential_id=NULL,credential_revision=NULL,source_channel_id=NULL",
            sql,
        )
        self.assertNotIn("INSERT INTO core_channels.balance_observations", sql)
        self.assertNotIn("UPDATE core_channels.counter_baselines", sql)
        self.assertIn("((actual->'state')-'tables') IS DISTINCT FROM ((before_state->'state')-'tables')", sql)
        self.assertIn("channel_opening_copy_guard_missing", sql)
        self.assertIn("can_insert", sql)
        self.assertGreater(
            sql.index("SELECT jsonb_build_object('status'"), sql.index("COMMIT;")
        )
        noop = sql.split("IF actual->'openings'->'sources'<>'[]'::jsonb THEN", 1)[
            1
        ].split("\n ELSE", 1)[0]
        self.assertNotIn("UPDATE ", noop)
        self.assertNotIn("INSERT ", noop)
        self.assertNotIn("target_changed", noop)

    def test_resolution_sql_cannot_write_even_when_receipt_absent(self):
        sql = C.build_apply_sql(
            self.material(), resolve="lmm-channel-opening-" + "a" * 32
        )
        self.assertIn("READ ONLY", sql)
        self.assertIn("channel_original_session_active", sql)
        self.assertIn("'UNKNOWN'", sql)
        self.assertNotIn("INSERT INTO", sql)
        self.assertNotIn("UPDATE core_channels", sql)
        self.assertNotIn("LOCK TABLE", sql)

    def test_owner_approval_changes_with_material_or_sql(self):
        material = self.material()
        sql = C.build_apply_sql(material)
        approval = C.approval_for(material, sql)
        self.assertNotEqual(approval, C.approval_for(material, sql + "\n"))
        changed = copy.deepcopy(material)
        changed["runtime_roles"] = ["different_runtime"]
        self.assertNotEqual(approval, C.approval_for(changed, sql))

    def test_unknown_transport_is_single_attempt_and_resolve_never_replays(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "apply"
            output.mkdir(mode=0o700)
            resolution = root / "resolve"
            resolution.mkdir(mode=0o700)
            material = self.material()
            material["apply_output"] = str(output)
            approval = C.approval_for(material, C.build_apply_sql(material))
            connection = {
                "role": "offline_owner",
                "login_role": "offline_operator",
                "set_role": "offline_owner",
                "database": "synthetic",
                "port": 5432,
            }
            with (
                patch.object(C.IDENTITY, "BACKUP_ROOT", root),
                patch.object(C.IDENTITY, "socket_connection", return_value={}),
                patch.object(C.F, "run_sql", return_value=(1, "")) as run,
            ):
                self.assertEqual(
                    C.execute(
                        material,
                        approval=approval,
                        connection=connection,
                        output=output,
                    )["status"],
                    "UNKNOWN",
                )
                self.assertEqual(run.call_count, 1)
                with self.assertRaises(FileExistsError):
                    C.execute(
                        material,
                        approval=approval,
                        connection=connection,
                        output=output,
                    )
                self.assertEqual(run.call_count, 1)
                attempt = C.P.parse_json(
                    C.F.read_private(output / "attempt.private.json")
                )
                expected = {
                    "status": "COMMITTED",
                    "source_key": material["plan"]["source"]["source_key"],
                    "plan_sha256": material["plan"]["source"]["plan_sha256"],
                }
                run.return_value = (0, C.F.canonical(expected))
                result = C.execute(
                    material,
                    approval=approval,
                    connection=connection,
                    output=resolution,
                    original_attempt=attempt,
                )
                self.assertEqual(result, expected)
                self.assertEqual(run.call_count, 2)
                self.assertTrue(run.call_args.kwargs["readonly"])
                self.assertNotIn("INSERT INTO", run.call_args.args[1])

    def test_file_inspection_contains_only_counter_fields_and_hashes(self):
        result = C.inspect_source(self.files)
        self.assertEqual(result["rows_sha256"], self.evidence["rows_sha256"])
        self.assertNotIn("SYNTHETIC-SECRET", C.F.canonical(result))
        self.assertEqual(result["target_writes"], 0)

    def test_post_submission_receipt_requires_successful_commit_transport(self):
        for code, reported, expected in (
            (0, "COMMITTED", "COMMITTED"),
            (0, "NOOP", "NOOP"),
            (1, "COMMITTED", "UNKNOWN"),
            (0, "wrong_receipt", "UNKNOWN"),
        ):
            with (
                self.subTest(code=code, reported=reported),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                output = root / "apply"
                output.mkdir(mode=0o700)
                material = self.material()
                material["apply_output"] = str(output)
                approval = C.approval_for(material, C.build_apply_sql(material))
                source = material["plan"]["source"]
                receipt = {
                    "status": reported,
                    "source_key": source["source_key"],
                    "plan_sha256": source["plan_sha256"],
                }
                connection = {
                    "role": "offline_owner",
                    "login_role": "offline_operator",
                    "set_role": "offline_owner",
                    "database": "synthetic",
                    "port": 5432,
                }
                with (
                    patch.object(C.IDENTITY, "BACKUP_ROOT", root),
                    patch.object(C.IDENTITY, "socket_connection", return_value={}),
                    patch.object(
                        C.F, "run_sql", return_value=(code, C.F.canonical(receipt))
                    ) as run,
                ):
                    outcome = C.execute(
                        material,
                        approval=approval,
                        connection=connection,
                        output=output,
                    )
                    self.assertEqual(outcome["status"], expected)
                    self.assertEqual(run.call_count, 1)
                    self.assertTrue((output / "attempt.private.json").exists())
                    self.assertTrue((output / "result.private.json").exists())


class EntryPointTests(unittest.TestCase):
    def test_help_never_connects(self):
        result = subprocess.run(
            [sys.executable, "-B", str(Path(C.__file__)), "--help"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("file-only preview", result.stdout)


class FrozenSourceFileTests(unittest.TestCase):
    def test_archive_hash_reads_only_one_mib_chunks(self):
        payload = b"s" * (1024 * 1024) + b"end"
        sizes = []

        class Stream(io.BytesIO):
            def read(self, size=-1):
                sizes.append(size)
                if size != 1024 * 1024:
                    raise AssertionError("archive hashing must use 1 MiB reads")
                return super().read(size)

        with patch.object(Path, "open", return_value=Stream(payload)):
            actual = C.IDENTITY.sha256_file(Path("/synthetic/archive"))
        self.assertEqual(actual, hashlib.sha256(payload).hexdigest())
        self.assertEqual(sizes, [1024 * 1024] * 3)

    def test_actual_shared_frozen_export_reader_and_file_hash_binding(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(C.IDENTITY, "BACKUP_ROOT", Path(directory)),
        ):
            work = Path(directory) / "source"
            work.mkdir(mode=0o700)

            def write(name, value):
                text = value if isinstance(value, str) else C.F.canonical(value)
                path = C.F.write_private(work, name, text)
                return {"path": str(path), "sha256": C.IDENTITY.sha256_file(path)}

            row = {
                "id": 17,
                "used_quota": C.I64_MIN,
                "balance": 0,
                "balance_updated_time": 0,
                "key": "SYNTHETIC-PRIVATE",
            }
            identity = {
                "database": "source_synthetic",
                "role": "legacy_reader",
                "port": 5432,
                "tcp_connection": False,
                "read_only": "on",
                "database_oid": 123,
                "postmaster_started_at": "2026-01-01T00:00:00+00:00",
                "server_version_num": 170000,
            }
            archive = write("archive.private", "SYNTHETIC ARCHIVE ONLY")
            metadata = write(
                "native-catalog-metadata.jsonl",
                "\n".join(
                    C.F.canonical(p)
                    for p in (
                        [],
                        [],
                        [
                            {
                                "schema": "public",
                                "table_name": "channels",
                                "column_name": k,
                            }
                            for k in row
                        ],
                        [],
                        [],
                        [],
                        [],
                    )
                ),
            )
            counts = write(
                "native-table-row-count-baseline.jsonl",
                {"table": "channels", "row_count": 1},
            )
            receipt = {
                "status": "snapshot-restored-baseline-verified-frozen-and-classified",
                "archive": archive,
                "private_metadata_files": [metadata["path"], counts["path"]],
                "source": {"schema": "public"},
                "readonly_reader": {
                    "database": "source_synthetic",
                    "role": "legacy_reader",
                    "port": 5432,
                },
                "source_instance_identity": {
                    **identity,
                    "listen_addresses": "",
                    "schema_owner": "legacy_owner",
                },
            }
            export = write(
                "source-export.private.jsonl",
                "\n".join(
                    C.F.canonical(p)
                    for p in (
                        {"kind": "identity", "row": identity},
                        {
                            "kind": "schema",
                            "row": {"schema": "public", "schema_owner": "legacy_owner"},
                        },
                        {"kind": "row", "table": "channels", "row": row},
                    )
                ),
            )
            proof = {
                "source_archive_sha256": archive["sha256"],
                "source_export_sha256": export["sha256"],
                "source_column_coverage_exact": True,
                "source_row_baseline_exact": True,
                "metadata_hashes": C.P.metadata(receipt)[2],
                "selected_table_counts": {"channels": 1},
                "source_instance": identity,
            }
            files = {
                "archive": archive,
                "source_receipt": write("receipt.private.json", receipt),
                "source_export": export,
                "source_export_proof": write("proof.private.json", proof),
                "evidence:unit": write(
                    "unit-evidence.private.txt", "SYNTHETIC EVIDENCE"
                ),
            }
            with (
                patch.object(
                    C.F,
                    "run_sql",
                    side_effect=AssertionError("inspection must never connect"),
                ),
                patch.object(
                    Path,
                    "read_bytes",
                    side_effect=AssertionError("no whole-file binary reads"),
                ),
                patch.object(
                    Path, "glob", side_effect=AssertionError("no directory scanning")
                ),
                patch.object(
                    Path, "rglob", side_effect=AssertionError("no recursive scanning")
                ),
            ):
                result = C.inspect_source(files)
                counter = result["recorded_counters"][0]
                self.assertEqual(counter["raw_used_quota"], str(C.I64_MIN))
                self.assertEqual(counter["balance"], "0")
                self.assertIsNone(counter["balance_updated_time"])
                self.assertNotIn("SYNTHETIC-PRIVATE", C.F.canonical(result))
                Path(export["path"]).write_text(Path(export["path"]).read_text() + "\n")
                with self.assertRaisesRegex(C.F.Error, "source_file_changed"):
                    C.inspect_source(files)


if __name__ == "__main__":
    unittest.main()
