"""Synthetic source mapping and refusal checks; no database or real credentials."""

import copy
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import test_migrate_legacy_financial as fixtures
import legacy_wallet_transfer_mapping as M

F = fixtures.MODULE
KEY = "\u754c" * 5 + "a"
TOKEN = "aB" * 32


def transfer(tid=3, **changes):
    return fixtures.transfer(tid, **{"request_key": KEY, "token": TOKEN, **changes})


def inputs(rows=None, identities=None):
    identities = {71: 2, 72: 9} if identities is None else identities
    tables = fixtures.tables(
        users=[fixtures.user(uid, aff_quota=0) for uid in identities],
        wallet_transfers=[transfer()] if rows is None else rows,
    )
    keys = {table: ["id"] for table in tables}
    columns = {table: [{"column_name": key} for key in sorted(records[0])] if records else []
               for table, records in tables.items()}
    plan = F.plan_financial(tables, identities, source_sha256="a" * 64,
                           primary_keys=keys, column_schema=columns)
    proof = {"source_archive_sha256": "a" * 64, "source_export_sha256": "b" * 64,
             "source_column_coverage_exact": True, "source_row_baseline_exact": True,
             "selected_table_counts": {table: len(records) for table, records in tables.items()},
             "source_primary_keys": keys, "source_column_schema": copy.deepcopy(columns)}
    return {"tables": tables, "plan": plan, "source_proof": proof,
            "identity_records": [{"source_user_id": uid, "target_user_id": uid,
                                  "target_personal_account_id": aid} for uid, aid in identities.items()],
            "source_namespace": "synthetic-legacy-source"}


def reseal(plan):
    material = {key: value for key, value in plan.items() if key not in ("mapping_sha256", "public_summary")}
    plan["mapping_sha256"] = F.digest(material)
    plan["public_summary"]["mapping_sha256"] = plan["mapping_sha256"]


class WalletTransferMappingTests(unittest.TestCase):
    def test_pending_preserves_exact_bytes_text_digest_and_no_second_debit(self):
        args = inputs()
        before = copy.deepcopy(args)
        result = M.build(F, **args)
        row = result["records"][0]
        self.assertEqual(args, before)
        self.assertTrue(result["source_mapping_complete"])
        self.assertEqual(row["request_key_utf8_length"], 16)
        self.assertEqual(bytes.fromhex(row["request_key_utf8_hex"]).decode("utf-8"), KEY)
        self.assertEqual(row["token_digest_sha256"], hashlib.sha256(TOKEN.encode()).hexdigest())
        self.assertNotEqual(row["token_digest_sha256"], hashlib.sha256(bytes.fromhex(TOKEN)).hexdigest())
        self.assertEqual((row["owner_user"], row["payer_account"]), (71, 2))
        self.assertEqual((row["source_recipient_id"], row["recipient_user"], row["recipient_account"]), (0, None, None))
        self.assertEqual(row["source_row_sha256"], F.digest(args["tables"]["wallet_transfers"][0]))
        self.assertEqual((row["amount_units"], row["source_pending_units"]), (500, 500))
        self.assertTrue(row["sender_available_already_debited"])
        for field in ("wallet_debit_to_apply", "wallet_credit_to_apply", "reserved_units_to_apply"):
            self.assertEqual(row[field], 0)
        for field in ("can_apply", "economic_approval_verified", "source_unit_provenance_verified",
                      "source_namespace_authority_verified", "runtime_activation_authorized",
                      "original_bearer_activation_allowed"):
            self.assertIs(result[field], False)
        self.assertFalse(row["original_bearer_activation_allowed"])
        self.assertEqual((result["target_writes"], result["ledger_writes"]), (0, 0))
        self.assertIn("pending_transfer_claim_contract_missing", result["activation_blockers"])
        self.assertNotIn(TOKEN, F.canonical(result))
        self.assertTrue(M.verify(F, json.loads(F.canonical(result)), **args))

    def test_terminal_rows_are_history_without_reopening_funds_or_changing_times(self):
        rows = [
            transfer(status="claimed", recipient_id=72, claimed_at=1706745600),
            transfer(4, request_key="request-cancel-0001", token="cD" * 32,
                     status="cancelled", cancelled_at=1706745601),
        ]
        result = M.build(F, **inputs(rows))
        self.assertTrue(result["source_mapping_complete"])
        for row in result["records"]:
            self.assertEqual(row["source_pending_units"], 0)
            self.assertEqual(row["disposition"], "terminal-history-review-only")
            self.assertEqual(row["wallet_credit_to_apply"], 0)
            self.assertEqual(row["reserved_units_to_apply"], 0)
        claimed = result["records"][0]
        self.assertEqual((claimed["recipient_user"], claimed["recipient_account"]), (72, 9))
        self.assertEqual(claimed["claimed_at"], 1706745600)
        self.assertNotIn("pending_transfer_claim_contract_missing", result["activation_blockers"])
        self.assertFalse(result["can_apply"])

    def test_key_uniqueness_is_per_owner_and_preserves_case_and_unicode_forms(self):
        keys = ("request-CASE-0001", "request-case-0001",
                "\u00e9" * 8, "e\u0301" * 8)
        rows = [transfer(i + 1, request_key=key, token=f"{i + 1:064x}") for i, key in enumerate(keys)]
        rows.append(transfer(9, sender_id=72, request_key=keys[0], token="eF" * 32))
        result = M.build(F, **inputs(rows))
        self.assertEqual(len(result["records"]), 5)
        self.assertEqual(len({row["request_key_utf8_hex"] for row in result["records"]}), 4)
        lower = M.build(F, **inputs([transfer(token=TOKEN.lower())]))
        upper = M.build(F, **inputs())
        self.assertNotEqual(lower["records"][0]["token_digest_sha256"],
                            upper["records"][0]["token_digest_sha256"])

    def test_source_origin_is_stable_across_archives_but_not_namespaces(self):
        args = inputs()
        first = M.build(F, **args)
        args["plan"]["source_archive_sha256"] = "c" * 64
        args["source_proof"]["source_archive_sha256"] = "c" * 64
        reseal(args["plan"])
        second = M.build(F, **args)
        self.assertEqual(first["records"][0]["source_origin"], second["records"][0]["source_origin"])
        self.assertEqual(first["records"][0]["source_origin_sha256"],
                         second["records"][0]["source_origin_sha256"])
        self.assertNotEqual(first["source_mapping_sha256"], second["source_mapping_sha256"])
        args["source_namespace"] += "-other"
        third = M.build(F, **args)
        self.assertNotEqual(second["records"][0]["source_origin_sha256"],
                            third["records"][0]["source_origin_sha256"])
        self.assertFalse(third["source_namespace_authority_verified"])

    def test_duplicate_ids_owner_keys_and_tokens_reject_instead_of_deduplicating(self):
        duplicate_id = inputs([transfer(), transfer(4, request_key="request-other-001", token="cd" * 32)])
        duplicate_id["tables"]["wallet_transfers"][1]["id"] = 3
        rows = duplicate_id["tables"]["wallet_transfers"]
        evidence = duplicate_id["plan"]["source_evidence"]["wallet_transfers"]
        evidence["row_hashes"] = [F.digest(row) for row in rows]
        evidence["source_records"] = [
            {"source_primary_key": {"id": row["id"]}, "source_export_ordinal": ordinal,
             "source_row_sha256": F.digest(row)} for ordinal, row in enumerate(rows)]
        reseal(duplicate_id["plan"])
        cases = [
            (duplicate_id, "transfer_id_duplicate"),
            (inputs([transfer(), transfer(4, token="cd" * 32)]), "wallet_transfer_owner_request_key_duplicate"),
            (inputs([transfer(), transfer(4, request_key="request-other-001")]), "wallet_transfer_token_duplicate"),
        ]
        for args, code in cases:
            with self.subTest(code=code), self.assertRaisesRegex(F.MappingError, code):
                M.build(F, **args)

    def test_key_byte_limits_whitespace_nul_and_invalid_utf8_refuse_without_rewrite(self):
        for key in ("x" * 15, "\u754c" * 22, "\t" + "x" * 16,
                    "x" * 16 + "\u3000", "x" * 16 + "\0", None):
            with self.subTest(key=repr(key)), self.assertRaisesRegex(F.MappingError, "wallet_transfer_request_key_invalid"):
                M.build(F, **inputs([transfer(request_key=key)]))
        for key in ("\u754c" * 21 + "a", "\x1c" + "x" * 15):
            row = M.build(F, **inputs([transfer(request_key=key)]))["records"][0]
            self.assertEqual(row["request_key_utf8_hex"], key.encode().hex())

    def test_missing_or_invalid_token_is_never_replaced_or_disclosed(self):
        for token in ("", "a" * 63, "z" * 64, "\u754c" * 64, " " + "a" * 63, None):
            with self.subTest(kind=type(token).__name__), self.assertRaisesRegex(F.MappingError, "wallet_transfer_token_invalid"):
                M.build(F, **inputs([transfer(token=token)]))
        args = inputs()
        del args["tables"]["wallet_transfers"][0]["token"]
        with self.assertRaisesRegex(F.MappingError, "wallet_transfer_source_rows_changed"):
            M.build(F, **args)

    def test_unencodable_source_text_is_a_value_free_refusal(self):
        for field in ("request_key", "token"):
            args = inputs()
            args["tables"]["wallet_transfers"][0][field] = "\ud800"
            with self.subTest(field=field), self.assertRaisesRegex(F.MappingError, "wallet_transfer_source_utf8_invalid"):
                M.build(F, **args)

    def test_amounts_are_exact_positive_safe_integers_and_old_pending_has_no_expiry(self):
        for amount in (0, -1, True, 1.0, "500", Decimal("500"), F.JS_SAFE_MAX + 1, 2**63):
            with self.subTest(amount=repr(amount)), self.assertRaises(F.MappingError):
                M.build(F, **inputs([transfer(quota=amount)]))
        row = M.build(F, **inputs([transfer(quota=F.JS_SAFE_MAX, created_at=1)]))["records"][0]
        self.assertEqual(row["amount_units"], F.JS_SAFE_MAX)
        self.assertEqual(row["source_pending_units"], F.JS_SAFE_MAX)
        self.assertEqual(row["disposition"], "pending-right-review-only")

    def test_lifecycle_and_orphan_conflicts_block_the_whole_mapping(self):
        cases = [
            ({"status": "unknown"}, "unknown_transfer_status"),
            ({"sender_id": 999}, "orphan_transfer_sender"),
            ({"recipient_id": 999}, "orphan_transfer_recipient"),
            ({"recipient_id": 72}, "pending_transfer_terminal_fields_present"),
            ({"claimed_at": 1706745601}, "pending_transfer_terminal_fields_present"),
            ({"cancelled_at": 1706745601}, "pending_transfer_terminal_fields_present"),
            ({"status": "claimed"}, "claimed_transfer_recipient_missing"),
            ({"status": "claimed", "recipient_id": 71, "claimed_at": 1706745601}, "claimed_transfer_self_claim"),
            ({"status": "claimed", "recipient_id": 72}, "claimed_transfer_time_missing"),
            ({"status": "claimed", "recipient_id": 72, "claimed_at": 1706745601, "cancelled_at": 1},
             "claimed_transfer_cancel_time_present"),
            ({"status": "cancelled"}, "cancelled_transfer_time_missing"),
            ({"status": "cancelled", "cancelled_at": 1706745601, "recipient_id": 72},
             "cancelled_transfer_claim_fields_present"),
            ({"status": "claimed", "recipient_id": 72, "claimed_at": 1},
             "wallet_transfer_terminal_time_precedes_creation"),
        ]
        for changes, code in cases:
            args = inputs([transfer(**changes), transfer(4, request_key="request-valid-01", token="cd" * 32)])
            with self.subTest(changes=changes):
                result = M.build(F, **args)
                self.assertEqual(len(result["records"]), 2)
                self.assertFalse(result["source_mapping_complete"])
                self.assertIn(code, result["source_blockers"])
                self.assertEqual(result["records"][0]["disposition"], "rejected-source-row")
                self.assertFalse(result["can_apply"])
        for changes in ({"created_at": 0}, {"claimed_at": -1}, {"cancelled_at": F.JS_SAFE_MAX + 1}):
            with self.subTest(changes=changes), self.assertRaisesRegex(F.MappingError, "wallet_transfer_time_out_of_range"):
                M.build(F, **inputs([transfer(**changes)]))

    def test_source_coverage_catalog_and_projection_cannot_be_substituted(self):
        mutations = [
            lambda a: a["source_proof"].update(source_archive_sha256="c" * 64),
            lambda a: a["source_proof"].update(source_export_sha256=None),
            lambda a: a["source_proof"].update(source_row_baseline_exact=False),
            lambda a: a["source_proof"].update(source_column_coverage_exact=False),
            lambda a: a["source_proof"]["selected_table_counts"].update(wallet_transfers=0),
            lambda a: a["source_proof"]["source_primary_keys"].update(wallet_transfers=None),
            lambda a: a["source_proof"]["source_column_schema"].update(wallet_transfers=[]),
            lambda a: a["tables"]["wallet_transfers"][0].update(quota=501),
            lambda a: a["tables"]["wallet_transfers"].clear(),
            lambda a: a["tables"].pop("wallet_transfers"),
            lambda a: a["plan"]["wallet_transfers"][0].update(amount_units=501),
            lambda a: a["plan"]["source_evidence"]["wallet_transfers"]["source_records"][0].update(source_export_ordinal=1),
        ]
        for mutation in mutations:
            args = inputs()
            mutation(args)
            reseal(args["plan"])
            with self.subTest(mutation=mutations.index(mutation)), self.assertRaises(F.MappingError):
                M.build(F, **args)

    def test_identity_full_coverage_one_to_one_and_existing_id_contract_are_required(self):
        for change in ("omit", "account", "duplicate", "remap", "bool"):
            args = inputs()
            if change == "omit":
                args["identity_records"].pop()
            elif change == "account":
                args["identity_records"][0]["target_personal_account_id"] = 99
            elif change == "duplicate":
                args["identity_records"].append(copy.deepcopy(args["identity_records"][0]))
            elif change == "remap":
                args["identity_records"][0]["target_user_id"] = 99
            else:
                args["identity_records"][0]["target_personal_account_id"] = True
            with self.subTest(change=change), self.assertRaises(F.MappingError):
                M.build(F, **args)

    def test_explicit_empty_table_is_bound_but_never_approved(self):
        args = inputs([])
        result = M.build(F, **args)
        self.assertTrue(result["source_mapping_complete"])
        self.assertEqual(result["records"], [])
        self.assertFalse(result["can_apply"])
        self.assertTrue(M.verify(F, result, **args))
        args["source_proof"]["selected_table_counts"].pop("wallet_transfers")
        with self.assertRaises(F.MappingError):
            M.build(F, **args)

    def test_rehashed_tampering_cannot_remove_refusals_or_change_source_facts(self):
        args = inputs()
        original = M.build(F, **args)
        for field, value in (("can_apply", True), ("activation_blockers", []), ("target_writes", 1),
                             ("runtime_activation_authorized", True), ("source_mapping_complete", 1)):
            result = copy.deepcopy(original)
            result[field] = value
            result["source_mapping_sha256"] = F.digest(
                {key: item for key, item in result.items() if key != "source_mapping_sha256"})
            with self.subTest(field=field), self.assertRaisesRegex(F.MappingError, "wallet_transfer_mapping_mismatch"):
                M.verify(F, result, **args)
        for field in ("amount_units", "request_key_utf8_hex", "token_digest_sha256", "source_status"):
            result = copy.deepcopy(original)
            result["records"][0][field] = "changed"
            with self.subTest(field=field), self.assertRaises(F.MappingError):
                M.verify(F, result, **args)

    def test_mapping_does_not_change_existing_pending_opening_refusal(self):
        args = inputs()
        before = copy.deepcopy(args["plan"])
        M.build(F, **args)
        self.assertEqual(args["plan"], before)
        reservations = {"source_archive_sha256": "a" * 64, "mapping_sha256": before["mapping_sha256"],
                        "reviewed_evidence_sha256": "c" * 64,
                        "positions": [{"source_user_id": uid, "reserved_units": 500 if uid == 71 else 0}
                                      for uid in (71, 72)]}
        with self.assertRaisesRegex(F.MappingError, "pending_transfer_claim_contract_missing"):
            F.build_opening_payloads(args["plan"], migration_id="synthetic-financial",
                observation=F.source_observation({"started_utc": "20261011T174353Z"}),
                reservation_reconciliation=reservations)


if __name__ == "__main__":
    unittest.main()
