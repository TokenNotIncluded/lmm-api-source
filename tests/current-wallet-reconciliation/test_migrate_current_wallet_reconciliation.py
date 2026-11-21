"""Synthetic arithmetic/receipt regressions; never connects to a database."""
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

import migrate_current_wallet_reconciliation as W
from test_migrate_current_runtime_keys import inputs, snapshot, synthetic_contract, NAMESPACE


def source(quota, affiliate, archive):
    return {"archive_sha256": archive, "export_sha256": archive,
        "users": [{"id": 37, "quota": quota, "aff_quota": affiliate, "status": 1, "deleted_at": None}],
        "options": [{"key": "CreditsPerUSD", "value": "500000"}, {"key": "QuotaPerUnit", "value": "500000"}],
        "files": {}}


def fixture(*, old_wallet=100, old_affiliate=20, posted_affiliate=0, current_wallet=150,
            current_affiliate=30, native_wallet_delta=-20, native_affiliate_delta=0, reserved=9, initial=False):
    old = source(old_wallet, old_affiliate, "b" * 64)
    current = source(current_wallet, current_affiliate, "c" * 64)
    _, target = inputs()
    contract = synthetic_contract()
    snap = snapshot(target, contract)
    inv = snap["state"]["inventory"]
    inv["tables"] = sorted(set(inv["tables"]) | set(W.CHANGED))
    inv["sequences"] = sorted(set(inv["sequences"]) | set(W.SEQUENCES))
    for name in inv["tables"]:
        snap["state"]["tables"].setdefault(name, {"rows": 1, "sha256": "6" * 64})
    for name in W.SEQUENCES:
        snap["state"]["sequences"][name] = {"last_value": 30, "is_called": True, "start": 1, "increment": 1,
            "min": 1, "max": W.MAX, "cache": 1, "cycle": False}
    opening = {"migration_id": "synthetic-old-opening", "source_archive_sha256": old["archive_sha256"],
        "mapping_sha256": "a" * 64, "observed_at": 1000, "source_entity_key": "legacy.users/37",
        "source_row_sha256": W.digest(old["users"][0]), "account_id": 8,
        "available_units": old_wallet, "affiliate_units": posted_affiliate, "reserved_units": 0}
    receipt = {"migration_id": opening["migration_id"], "source_entity_key": opening["source_entity_key"],
        "source_row_sha256": opening["source_row_sha256"], "account_id": 8, "request": opening,
        "result": {"status": "posted", "journal_id": 10}}
    run = {k: opening[k] for k in ("migration_id", "source_archive_sha256", "mapping_sha256", "observed_at")}
    n = old_wallet + native_wallet_delta
    values = {"wallet": max(n, 0), "debt": min(n, 0), "affiliate": posted_affiliate + native_affiliate_delta, "reserved": reserved}
    initial_values = {"wallet": max(old_wallet, 0), "debt": min(old_wallet, 0), "affiliate": posted_affiliate, "reserved": 0}
    buckets = []
    for index, bucket in enumerate(W.BUCKETS):
        rev = 1 + int(values[bucket] != initial_values[bucket])
        buckets.append({"id": index + 10, "bucket": bucket, "unit": "credit_500k_usd", "units": values[bucket],
            "revision": rev, "entries_sum": values[bucket], "entries_count": rev, "path_valid": True,
            "opening_units": initial_values[bucket]})
    owner = {"user_id": 37, "account_id": 8, "opening": receipt, "opening_run": run, "buckets": buckets}
    if initial:
        owner = {"user_id": 37, "account_id": 8, "opening": None, "opening_run": None, "buckets": []}
    snap["ledger"] = {"owners": [owner], "clearing": {"id": 1, "units": -200, "revision": 3,
        "entries_sum": -200, "entries_count": 3, "path_valid": True}, "global_valid": True,
        "max_account_id": 13, "max_journal_id": 10}
    snap["reconciliation_runs"] = []
    mapping = [{"source_user_id": 37, "target_user_id": 37, "target_personal_account_id": 8}]
    return current, [old], mapping, snap, contract


def preview(**kwargs):
    cur, old, mapping, snap, contract = fixture(**kwargs)
    m = W.prepare(cur, old, mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
    m["source_observed_at"] = 2000
    return m


class ArithmeticTests(unittest.TestCase):
    def test_source_replacement_preserves_native_delta_instead_of_refunding_it(self):
        m = preview()
        self.assertEqual(m["blockers"], [])
        row = m["positions"][0]
        self.assertEqual((row["opening_B"], row["native_D"], row["native_N"]), (100, -20, 80))
        self.assertEqual(row["target_buckets"]["wallet"], 130)  # S=150, native spend=20 remains spent.
        self.assertEqual(row["legs"]["wallet"], 50)  # S-B, not S-N=70.
        self.assertEqual(row["target_buckets"]["reserved"], 9)
        self.assertEqual(sum(row["legs"].values()), 0)

    def test_negative_source_wallet_creates_signed_debt_without_clamping(self):
        row = preview(current_wallet=-10, current_affiliate=0)["positions"][0]
        self.assertEqual(row["target_buckets"]["wallet"], 0)
        self.assertEqual(row["target_buckets"]["debt"], -30)
        self.assertEqual(row["legs"]["wallet"] + row["legs"]["debt"], -110)
        self.assertEqual(sum(row["legs"].values()), 0)

    def test_debt_crossing_zero_does_not_create_extra_credits(self):
        row = preview(old_wallet=-100, native_wallet_delta=20, current_wallet=50, current_affiliate=0)["positions"][0]
        self.assertEqual(row["native_N"], -80)
        self.assertEqual(row["target_buckets"]["wallet"], 70)
        self.assertEqual(row["target_buckets"]["debt"], 0)
        self.assertEqual(row["legs"]["wallet"] + row["legs"]["debt"], 150)

    def test_negative_affiliate_and_prior_native_claim_are_preserved(self):
        row = preview(posted_affiliate=20, native_affiliate_delta=-5, current_affiliate=-7)["positions"][0]
        self.assertEqual(row["native_affiliate_D"], -5)
        self.assertEqual(row["target_buckets"]["affiliate"], -12)
        self.assertEqual(row["legs"]["affiliate"], -27)

    def test_deferred_affiliate_uses_zero_actual_posted_basis(self):
        row = preview(old_affiliate=500, posted_affiliate=0, current_affiliate=37)["positions"][0]
        self.assertEqual(row["opening_affiliate_B"], 0)
        self.assertEqual(row["legs"]["affiliate"], 37)

    def test_no_source_delta_does_not_erase_native_spending(self):
        row = preview(current_wallet=100, current_affiliate=0)["positions"][0]
        self.assertEqual(row["native_N"], 80)
        self.assertEqual(row["target_buckets"]["wallet"], 80)
        self.assertEqual(row["journal_amount"], 0)
        self.assertTrue(all(v == 0 for v in row["legs"].values()))

    def test_new_empty_owner_uses_actual_initial_opening_contract(self):
        m = preview(initial=True, current_wallet=-8, current_affiliate=-3)
        row = m["positions"][0]
        payload = W.initial_payload(m, row)
        self.assertTrue(row["initial"])
        self.assertEqual((payload["available_units"], payload["affiliate_units"], payload["reserved_units"]), (-8, -3, 0))
        self.assertEqual(payload["source_entity_key"], "legacy.users/37")
        self.assertEqual(row["native_D"], 0)

    def test_unmapped_users_remain_in_complete_source_coverage_and_block_apply(self):
        cur, old, _mapping, snap, contract = fixture()
        m = W.prepare(cur, old, [], snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
        self.assertEqual(m["source_users"], 1)
        self.assertFalse(m["ready_for_review"])
        self.assertEqual(m["blockers"], [{"code": "wallet_source_owner_unmapped", "source_user_id": 37}])
        with self.assertRaisesRegex(W.Error, "unresolved_facts"):
            W.build_sql(m)

    def test_both_wallet_and_debt_cannot_be_netted_without_review(self):
        cur, old, mapping, snap, contract = fixture()
        debt = next(x for x in snap["ledger"]["owners"][0]["buckets"] if x["bucket"] == "debt")
        debt["units"] = debt["entries_sum"] = -10
        m = W.prepare(cur, old, mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
        self.assertEqual(m["blockers"][0]["code"], "wallet_native_split_requires_explicit_reconciliation")

    def test_old_archive_unit_cannot_be_replaced_by_live_unit_declaration(self):
        cur, old, mapping, snap, contract = fixture()
        old[0]["options"][0]["value"] = "1000000"
        m = W.prepare(cur, old, mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
        self.assertEqual(m["blockers"][0]["code"], "wallet_snapshot_unit_options_unproven")
        self.assertEqual(m["blockers"][0]["source_archive_sha256"], old[0]["archive_sha256"])

    def test_opening_row_digest_and_actual_initial_legs_are_verified(self):
        for mutation in ("row", "entry"):
            cur, old, mapping, snap, contract = fixture()
            if mutation == "row":
                old[0]["users"][0]["quota"] += 1
            else:
                snap["ledger"]["owners"][0]["buckets"][0]["opening_units"] += 1
            m = W.prepare(cur, old, mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
            self.assertTrue(m["blockers"])

    def test_global_or_per_bucket_ledger_drift_refuses_planning(self):
        cur, old, mapping, snap, contract = fixture()
        snap["ledger"]["global_valid"] = False
        with self.assertRaisesRegex(W.Error, "global_ledger_invalid"):
            W.prepare(cur, old, mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])

    def test_previous_different_epoch_not_counted_as_new_native_delta(self):
        cur, old, mapping, snap, contract = fixture()
        snap["reconciliation_runs"] = [{"migration_id": "old-catchup", "manifest": {
            "source_namespace_sha256": W.U.namespace_key(NAMESPACE)}}]
        m = W.prepare(cur, old, mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
        self.assertEqual(m["blockers"][0]["code"], "wallet_source_already_reconciled")

    def test_relabeling_namespace_cannot_reapply_to_previously_reconciled_owner(self):
        cur, old, mapping, snap, contract = fixture()
        snap["reconciliation_runs"] = [{"migration_id": "old-catchup", "manifest": {
            "source_namespace_sha256": "1" * 64, "material": {"positions": [{"account_id": 8}]}}}]
        m = W.prepare(cur, old, mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
        self.assertEqual(m["blockers"][0]["code"], "wallet_source_already_reconciled")

    def test_preservation_revision_only_changes_for_nonzero_actual_legs(self):
        m = preview(current_wallet=100, current_affiliate=0)
        values = W.expected_balances(m)
        self.assertTrue(all(v["units"] == v["before_units"] and v["revision"] == v["before_revision"] for v in values.values()))

    def test_sql_generation_and_snapshot_interface_do_not_require_approval_fabrication(self):
        m = preview()
        self.assertTrue(W.build_sql(m, fixture_nonce="e" * 64))
        self.assertTrue(W.snapshot_sql(m["contract"], m["before_state"]["inventory"], m["migration_id"]))
        self.assertNotIn("source_review", m)
        self.assertFalse(m["full_financial_import_complete"])

    def test_retained_partial_opening_baseline_uses_immutable_manifest_not_fake_full_rows(self):
        cur, old, mapping, snap, contract = fixture()
        request = snap["ledger"]["owners"][0]["opening"]["request"]
        base = {"source_kind": "committed-opening-baseline", "archive_sha256": old[0]["archive_sha256"],
                "export_sha256": old[0]["export_sha256"], "options": old[0]["options"]}
        material = {"scope": "financial-available-wallet-debt-v1",
            "policy": "original-signed-users-quota-only-no-rescale-all-other-rights-explicitly-deferred",
            "source_archive_sha256": base["archive_sha256"], "source_plan_mapping_sha256": "a" * 64,
            "positions": [{"source_user_id": 37, "target_user_id": 37, "account_id": 8,
                           "available_units": 100, "source_row_sha256": request["source_row_sha256"]}]}
        material["mapping_sha256"] = W.json_sha(material)
        material["source_files"] = {"source_export": {"sha256": base["export_sha256"]}}
        request["mapping_sha256"] = material["mapping_sha256"]
        snap["ledger"]["owners"][0]["opening_run"]["mapping_sha256"] = material["mapping_sha256"]
        snap["ledger"]["opening_manifests"] = [{"migration_id": request["migration_id"],
            "mapping_sha256": material["mapping_sha256"], "source_archive_sha256": base["archive_sha256"],
            "manifest": {"scope": material["scope"], "material": material}}]
        m = W.prepare(cur, [base], mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
        self.assertFalse(m["blockers"])
        self.assertEqual(m["positions"][0]["opening_B"], 100)
        material["positions"][0]["available_units"] = 90
        failed = W.prepare(cur, [base], mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])
        self.assertEqual(failed["blockers"][0]["code"], "wallet_original_partial_mapping_changed")


class PrivateEvidenceTests(unittest.TestCase):
    def setUp(self):
        W.I.BACKUP_ROOT.mkdir(parents=True, mode=0o700, exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(prefix="synthetic-wallet-unit-", dir=W.I.BACKUP_ROOT)
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def write(self, name, value):
        path = self.root / name
        path.write_text(W.canonical(value) + "\n" if not isinstance(value, str) else value)
        os.chmod(path, 0o600)
        return path

    def selected_fixture(self):
        archive = self.write("source.dump", "synthetic fixture archive only")
        sql = self.write("source-selected.sql", "-- synthetic readonly collector fixture\n")
        producer = self.write("producer.py", "# synthetic test producer, never production evidence\n")
        baseline = self.write("baseline.json", {"observed_at": "2026-11-21T00:00:00Z", "local_snapshot": "synthetic",
            "tables": [{"schema": "lmm_prod_20260802", "table": "users", "rows": 1}]})
        restore = {"status": "RESTORED_SOURCE_ONLY_READONLY_PREVIEW_NOT_FINANCIAL_APPROVAL",
            "restore_actual_exit": 0, "original_archive_preserved": True, "production_writes": 0,
            "target_database_accessed_by_restore": False, "financial_approval": False,
            "archive": {"path": str(archive), "sha256": W.raw_sha(archive)},
            "restored_instance": {"database": "synthetic_source", "nonce": "a" * 64, "socket_directory": "/synthetic",
                "port": 5432, "listen_addresses": "", "transaction_read_only": "on", "system_identifier": "222"},
            "production_source_observation": {"system_identifier": "111", "schema": "lmm_prod_20260802",
                "database": "lmm_api", "not_production_dump_snapshot_identity": True},
            "connection": {"database": "synthetic_source", "host": "/synthetic", "port": 5432},
            "readonly_reader_proof": {"read_only": "on", "row_security": "off", "role_memberships": 0,
                "database_create": False, "database_temp": False, "application_table_write_privileges": 0,
                "application_schema_create_privileges": 0, "application_security_definer_execute": 0, "rls_tables": 0,
                "role_flags": {k: False for k in ("superuser", "createrole", "createdb", "replication", "bypassrls")}},
            "table_baseline_path": str(baseline), "table_baseline_count": 1}
        restore_path = self.write("restore.json", restore)
        raw = {"id": 37, "quota": 150, "affiliate": -7, "role": 1, "status": 1,
               "group": "default", "created_at": 1000, "deleted_at": None}
        row = {**raw, "source_selected_facts_sha256": W.json_sha(raw), "source_identity_facts_sha256": "f" * 64}
        local = {"database": "synthetic_source", "nonce": "a" * 64, "observed_at": "2026-11-21T00:00:00Z",
                 "local_snapshot": "synthetic"}
        source = {**local, "users": [row], "unit_options": [{"key": "CreditsPerUSD", "value": "500000"},
                                                          {"key": "QuotaPerUnit", "value": "500000"}]}
        bindings = {"fresh_archive_path": str(archive), "fresh_archive_sha256": W.raw_sha(archive),
            "restore_receipt_path": str(restore_path), "restore_receipt_sha256": W.raw_sha(restore_path),
            "source_sql_sha256": W.raw_sha(sql), "source_selected_inventory_sha256": W.json_sha(source)}
        selected = self.write("selected.json", {"bindings": bindings, "source": source})
        proof = {"format": "lmm.selected-source-facts-proof.v1",
            "scope": "SELECTED_FIELDS_ONLY_NOT_CANONICAL_FINANCIAL_EXPORT_NOT_APPROVAL",
            "actual_source_selected_query_exit": 0, "original_source_columns_complete": False, "financial_approval": False,
            "fresh_archive_sha256": bindings["fresh_archive_sha256"], "restore_receipt_sha256": bindings["restore_receipt_sha256"],
            "source_sql_path": str(sql), "source_sql_sha256": W.raw_sha(sql),
            "source_selected_file_path": str(selected), "source_selected_file_sha256": W.raw_sha(selected),
            "execution_assertion_producer_path": str(producer), "execution_assertion_producer_sha256": W.raw_sha(producer),
            "source_selected_inventory_sha256": W.json_sha(source), "local_source_identity": local,
            "selected_user_count": 1, "restored_baseline_users": 1, "all_source_user_ids_unique": True,
            "unit_options_exact_rows": source["unit_options"], "source_production_fenced": False}
        proof_path = self.write("proof.json", proof)
        return proof_path, selected, baseline

    def test_selected_source_checks_real_file_bytes_row_digests_restore_and_baseline(self):
        proof, selected, baseline = self.selected_fixture()
        value = W.load_selected_source(proof)
        self.assertEqual(value["source_kind"], "selected-wallet-source-facts-v1")
        self.assertFalse(value["source_production_fenced"])
        self.assertEqual(value["users"][0]["aff_quota"], -7)
        self.assertEqual(W.unit_basis(value)["credits_per_usd"], 500000)
        selected.write_text(selected.read_text().replace('"quota":150', '"quota":151'))
        with self.assertRaisesRegex(W.Error, "file_mismatch"):
            W.load_selected_source(proof)

    def test_selected_proof_count_cannot_override_actual_users_baseline(self):
        proof, _selected, baseline = self.selected_fixture()
        value = json.loads(baseline.read_text())
        value["tables"][0]["rows"] = 2
        baseline.write_text(W.canonical(value))
        with self.assertRaisesRegex(W.Error, "actual_baseline_count_mismatch"):
            W.load_selected_source(proof)

    def test_selected_source_cannot_be_assigned_another_namespace(self):
        proof, _selected, _baseline = self.selected_fixture()
        current = W.load_selected_source(proof)
        _cur, old, mapping, snap, contract = fixture()
        with self.assertRaisesRegex(W.Error, "namespace_observation_mismatch"):
            W.prepare(current, old, mapping, snap, contract, NAMESPACE, "synthetic-wallet-reconcile", {}, ["synthetic_runtime"])

    def fence_fixture(self, m):
        observed = {"format_version": 1, "scope": "current-source-wallet-fence-observation-v1",
            "source_namespace": m["source_namespace"], "observed_at": "2026-11-21T00:01:00Z",
            "local_snapshot": "synthetic-fence-observation-only", "transaction_read_only": "on",
            "database_allows_connections": False, "other_database_backends": 0, "prepared_transactions": 0,
            "source_user_count": m["source_users"], "invalid_user_rows": 0,
            "users": [{"id": p["source_user_id"], "quota": p["source_S"], "affiliate": p["source_affiliate"]}
                      for p in m["positions"]],
            "unit_options": [{"key": "CreditsPerUSD", "value": "500000"}, {"key": "QuotaPerUnit", "value": "500000"}]}
        observation = self.write("synthetic-fence-observation.json", observed)
        sql = self.write("synthetic-fence-observation.sql", W.final_fence_observation_sql(m["source_namespace"]))
        raw_out = self.write("synthetic-fence-action.stdout", "synthetic test fixture, no source command executed\n")
        raw_err = self.write("synthetic-fence-action.stderr", "")
        operation = {"format_version": 1, "scope": "source-database-write-fence-operations-v1",
            "method": "database-disallow-connections-and-drain", "source_namespace": m["source_namespace"],
            "operator": "synthetic-test-only", "started_at": "2026-11-21T00:00:00Z", "finished_at": "2026-11-21T00:00:30Z",
            "original_database_allows_connections": True,
            "original_services": [{"unit": "synthetic-test-only.service", "active_state": "active", "sub_state": "running"}],
            "actions": [{"action": action, "command": "synthetic test fixture only", "exit_code": 0,
                         "stdout": W.file_ref(raw_out), "stderr": W.file_ref(raw_err)} for action in (
                         "stop_source_writers", "disallow_source_database_connections", "drain_source_database_backends")]}
        operation_path = self.write("synthetic-operation.json", operation)
        fence = {"format_version": 1, "scope": "current-source-wallet-fence-v1",
            "source_namespace_sha256": m["source_namespace_sha256"], "source_archive_sha256": m["source_archive_sha256"],
            "source_writes_fenced": True, "verification_actual_exit": 0,
            "post_fence_wallet_facts_sha256": m["source_wallet_facts_sha256"], "post_fence_source_user_count": m["source_users"],
            "observation": W.file_ref(observation), "observation_sql": W.file_ref(sql),
            "verification_stderr": W.file_ref(raw_err), "operation_receipt": W.file_ref(operation_path)}
        return fence, observation, operation_path

    def test_source_review_requires_post_fence_current_fact_equality(self):
        m = preview()
        facts = self.write("synthetic-review-evidence.json", {"synthetic": "review of test-only facts"})
        fence, _observation, _operation = self.fence_fixture(m)
        fence_path = self.write("fence.json", fence)
        review = {"format_version": 1, "scope": W.SCOPE, "decision": "review-current-wallet-debt-affiliate-reconciliation",
            "source_namespace_sha256": m["source_namespace_sha256"], "source_archive_sha256": m["source_archive_sha256"],
            "source_export_sha256": m["source_export_sha256"], "identity_map_sha256": m["identity_map_sha256"],
            "positions_sha256": m["mapping_sha256"], "units_sha256": W.digest(m["units"]),
            "other_rights_policy": W.OTHER_RIGHTS, "wallet_policy": W.POLICY,
            "source_review_evidence": W.file_ref(facts), "source_write_fence_evidence": W.file_ref(fence_path)}
        review_path = self.write("review.json", review)
        W.validate_source_review(m, review, review_path)
        fence["post_fence_wallet_facts_sha256"] = "0" * 64
        self.write("fence.json", fence)
        review["source_write_fence_evidence"] = W.file_ref(fence_path)
        with self.assertRaisesRegex(W.Error, "final_source_fence_not_verified"):
            W.validate_source_review(m, review, review_path)

    def test_matching_declared_hash_cannot_override_actual_post_fence_rows(self):
        m = preview()
        fence, observation, _operation = self.fence_fixture(m)
        value = json.loads(observation.read_text())
        value["users"][0]["quota"] += 1
        observation.write_text(W.canonical(value))
        fence["observation"] = W.file_ref(observation)
        self.assertEqual(fence["post_fence_wallet_facts_sha256"], m["source_wallet_facts_sha256"])
        with self.assertRaisesRegex(W.Error, "final_source_fence_not_verified"):
            W.validate_final_fence(m, fence)

    def test_actual_pg_fence_prepared_and_remaining_backend_checks_fail_closed(self):
        m = preview()
        fence, observation, _operation = self.fence_fixture(m)
        original = json.loads(observation.read_text())
        for field, changed in (("database_allows_connections", True), ("other_database_backends", 1),
                               ("prepared_transactions", 1), ("invalid_user_rows", 1)):
            with self.subTest(field=field):
                observation.write_text(W.canonical({**original, field: changed}))
                fence["observation"] = W.file_ref(observation)
                with self.assertRaisesRegex(W.Error, "actual_pg_barrier_missing"):
                    W.validate_final_fence(m, fence)

    def test_fence_unit_and_null_source_values_cannot_be_coalesced(self):
        m = preview()
        fence, observation, _operation = self.fence_fixture(m)
        original = json.loads(observation.read_text())
        invalid = deepcopy(original)
        invalid["unit_options"][0]["value"] = "1000000"
        observation.write_text(W.canonical(invalid))
        fence["observation"] = W.file_ref(observation)
        with self.assertRaisesRegex(W.Error, "snapshot_unit_options_unproven"):
            W.validate_final_fence(m, fence)
        invalid = deepcopy(original)
        invalid["users"][0]["affiliate"] = None
        observation.write_text(W.canonical(invalid))
        fence["observation"] = W.file_ref(observation)
        with self.assertRaisesRegex(W.Error, "fence_affiliate_invalid"):
            W.validate_final_fence(m, fence)

    def test_unexecuted_fence_template_never_acts_as_evidence(self):
        m = preview()
        with self.assertRaisesRegex(W.Error, "final_source_fence_not_verified"):
            W.validate_final_fence(m, W.fence_template(m))

    def test_approval_requires_exact_external_authorization_record_and_preview_sql(self):
        m = preview()
        sql = W.build_sql(m, fixture_nonce="e" * 64)
        review_path = self.write("synthetic-review.json", {"synthetic": "no production approval"})
        review_ref = W.file_ref(review_path)
        binding = {"format_version": 1, "scope": W.SCOPE, "decision": W.DECISION, "material_sha256": W.digest(m),
            "sql_sha256": hashlib.sha256(sql.encode()).hexdigest(), "source_review_sha256": review_ref["sha256"]}
        authorization = {**binding, "user_id": "synthetic-test-user", "message_reference": "synthetic-test-only",
            "approved_at": "2026-11-21T00:00:00Z", "exact_user_message": "Authorize only the disposable synthetic test fixture."}
        path = self.write("synthetic-authorization.json", authorization)
        approved = {**binding, "user_authorization_record": W.file_ref(path)}
        W.validate_approval(m, sql, approved, review_ref)
        for invalid in ("approved", {"decision": "approved"}, {**approved, "material_sha256": "a" * 64}):
            with self.subTest(invalid=type(invalid).__name__), self.assertRaises(W.Error):
                W.validate_approval(m, sql, invalid, review_ref)
        with self.assertRaisesRegex(W.Error, "approval_mismatch"):
            W.validate_approval(m, sql + "\n", approved, review_ref)


if __name__ == "__main__":
    unittest.main()
