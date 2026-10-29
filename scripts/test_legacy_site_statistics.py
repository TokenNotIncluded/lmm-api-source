"""Synthetic source files and pure behavior tests. No PostgreSQL or network.

Driver transport mocks test orchestration only; they do not prove PostgreSQL
accepts the SQL or that rollback, owner grants and deferred FK behavior work.
"""
import copy
from decimal import Decimal
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import legacy_site_statistics as S


def user(uid=1, **changes):
    return {"id": uid, "quota": 37, "aff_quota": 0, "used_quota": 1000, "request_count": 11,
            "aff_history": 9, "status": 1, "deleted_at": None, "password": "SYNTHETIC-PRIVATE-PASSWORD", **changes}


def topup(oid=1, **changes):
    return {"id": oid, "trade_no": f"synthetic-{oid}", "status": "success", "payment_method": "stripe",
            "payment_provider": "stripe", "settlement_currency": "USD", "settled_amount_micros": 1234567,
            "refunded_amount_micros": 234567, "expected_amount_micros": 9999999, "credited_quota": 10,
            "money": Decimal("99999999.1234"), **changes}


def audit(**changes):
    return {"version": 1, "kind": "offline_credit_balance_rebase_preview", "migration_id": "synthetic-rebase",
            "user_ids": [1], "has_complete_history": True, "usd_credit_conversion": 500000, "divisor": "2",
            "rounding": "half-away-from-zero", "snapshot_at": 100,
            "user_sources": [{"id": 1, "used_quota": 1000}], "token_sources": [], "other_credit_bases": [], **changes}


def usage_table(raw=1000, plan=None, **extra):
    return {"users": [user(used_quota=raw)], "wallet_credit_rebases": [{"plan": json.dumps(plan or audit())}], **extra}


def reversal_fixture(*, code="abuse", current=920, rounding="half-away-from-zero", original=100, baseline=1000):
    source = {"id": 9, "user_id": 1, "charged_quota": original, "status": "charged", "created_at": 90,
              "reversed_at": 0, "error_code": code}
    # Explicit oracle values: divisor 2, original 100 -> 50; original 1 -> 1/0.
    rebased = 50 if original == 100 else (1 if rounding == "half-away-from-zero" else 0)
    child = {"kind": "violation_fee_refund", "source_id": "9", "user_id": 1, "original_quota": original,
             "rebased_quota": rebased, "source": source}
    plan = audit(rounding=rounding, user_sources=[{"id": 1, "used_quota": baseline}], other_credit_bases=[child])
    fee = {**source, "status": "reversed", "reversed_at": 100, "reversed_by": 7}
    return usage_table(current, plan, violation_fee_records=[fee])


def fixture(tables=None):
    tables = copy.deepcopy(tables if tables is not None else {"users": [user()], "top_ups": [], "subscription_orders": []})
    identities = [{"source_user_id": u["id"], "target_user_id": u["id"], "target_personal_account_id": 100 + u["id"]}
                  for u in tables["users"]]
    opening = {"source_archive_sha256": "a" * 64, "mapping_sha256": "b" * 64,
               "observation": {"observed_at": 200}, "identity_import": {"migration_id": "synthetic-identity"}, "counters": []}
    for u in tables["users"]:
        opening["counters"].append({"account_id": 100 + u["id"], "user_id": u["id"], "migration_id": "synthetic-opening",
            "source_entity_key": f"legacy.users/{u['id']}", "source_archive_sha256": "a" * 64, "mapping_sha256": "b" * 64,
            "source_row_sha256": S.F.digest(u), "observed_at": 200, "used_quota": u["used_quota"],
            "request_count": u["request_count"], "aff_history": u["aff_history"]})
    args = {"tables": tables, "identity_records": identities, "opening": opening,
            "source_table_counts": {k: len(v) for k, v in tables.items()}}
    return args


def snapshot(plan):
    inventory = {"tables": sorted(set(S.F.CHANGED_TABLES) | set(S.F.FRESH_EMPTY_TABLES) | set(S.DESTINATIONS)
        | {"core_migration.import_runs", "core_identity.users", "core_trust_pricing.policies", "core_meta.site_finance_upgrades", "core_charging.requests"}),
        "sequences": sorted((*S.F.CHANGED_SEQUENCES, S.INTENT_SEQUENCE))}
    return {"native_users": [{"id": u["user_id"], "personal_account_id": u["account_id"]} for u in plan["users"]],
        "opening_counters": [{**c, "request": copy.deepcopy(c)} for c in plan["opening_counters"]],
        "import_runs": [plan["identity_import"]], "finance": {"sources": [], "users": [], "topups": []},
        "source_namespace": {"source_count": 0, "source_keys": [], "archive_sha256s": [], "user_rows": 0, "topup_rows": 0},
        "source_seals": [], "native_intent_upper": 0, "native_intent_max_id": 0, "native_request_ids": [],
        "instance": {"database": "synthetic_finance", "role": "offline_owner", "session_user": "offline_operator", "port": 5432},
        "ownership_sha256": "c" * 64, "catalog_sha256": "d" * 64,
        "state": {"inventory": inventory, "tables": {n: {"rows": 0, "sha256": "e" * 64} for n in inventory["tables"]},
                  "sequences": {n: {"last_value": 1, "is_called": False, "increment": 1, "cycle": False} for n in inventory["sequences"]}}}


def sync_namespace(target):
    actual = target["finance"]
    target["source_namespace"] = {"source_count": len(actual["sources"]),
        "source_keys": [r["source_key"] for r in actual["sources"]],
        "archive_sha256s": [r["source_archive_sha256"] for r in actual["sources"]],
        "user_rows": len(actual["users"]), "topup_rows": len(actual["topups"])}
    prior = {row["source_key"]: row for row in target["source_seals"]}
    target["source_seals"] = [prior.get(row["source_key"], {"source_key": row["source_key"],
        "native_intent_floor": target["native_intent_upper"], "native_request_ids": list(target["native_request_ids"])})
        for row in actual["sources"]]


class UsageTests(unittest.TestCase):
    def test_native_counters_no_audit_are_not_divided_by_current_k(self):
        self.assertEqual(S.normalized_usage({"users": [user(used_quota=987654321)]}), {1: 987654321})

    def test_empty_audit_table_preserves_native(self):
        self.assertEqual(S.normalized_usage({"users": [user()], "wallet_credit_rebases": []}), {1: 1000})
        self.assertEqual(S.normalized_usage({"users": [user()], "violation_fee_records": [{"id": 0}]}), {1: 1000})

    def test_migration_scales_only_baseline_not_post_migration_delta(self):
        self.assertEqual(S.normalized_usage(usage_table(1100)), {1: 600})

    def test_rounding_ties_and_fractional_divisor_are_exact(self):
        cases = [("2", "half-away-from-zero", 5, 3), ("2", "toward-zero", 5, 2),
                 ("2.5", "half-away-from-zero", 4, 2), ("2.5", "toward-zero", 4, 1),
                 ("1.000000000000000001", "toward-zero", S.SAFE, S.SAFE - 1)]
        for divisor, rounding, raw, expected in cases:
            with self.subTest(divisor=divisor, rounding=rounding):
                plan = audit(divisor=divisor, rounding=rounding, user_sources=[{"id": 1, "used_quota": raw}])
                self.assertEqual(S.normalized_usage(usage_table(raw, plan)), {1: expected})

    def test_fee_reversal_restores_raw_delta_and_removes_scaled_history(self):
        self.assertEqual(S.normalized_usage(reversal_fixture()), {1: 470})

    def test_reversal_rounds_each_fee_as_the_original_did(self):
        for rounding in ("half-away-from-zero", "toward-zero"):
            self.assertEqual(S.normalized_usage(reversal_fixture(current=4, original=1, baseline=5, rounding=rounding)), {1: 2})

    def test_moderation_reversal_does_not_adjust_usage(self):
        self.assertEqual(S.normalized_usage(reversal_fixture(code="moderation.text", current=1000)), {1: 500})
        with self.assertRaises(S.Error):
            S.normalized_usage(reversal_fixture(code="moderation.text", current=900))

    def test_charged_unreversed_fee_keeps_baseline(self):
        tables = reversal_fixture(current=1000)
        tables["violation_fee_records"][0].update(status="charged", reversed_at=0)
        self.assertEqual(S.normalized_usage(tables), {1: 500})

    def test_unaudited_post_snapshot_reversal_rejected_but_moderation_ignored(self):
        tables = reversal_fixture(current=1000)
        tables["wallet_credit_rebases"][0]["plan"] = json.dumps(audit())
        with self.assertRaisesRegex(S.Error, "audit_missing"):
            S.normalized_usage(tables)
        tables["violation_fee_records"][0]["error_code"] = "moderation.text"
        self.assertEqual(S.normalized_usage(tables), {1: 500})

    def test_unproven_counter_reduction_and_out_of_range_rejected(self):
        for raw in (-1, S.SAFE + 1, None, True, 1.0):
            with self.subTest(raw=raw), self.assertRaises(S.Error):
                S.normalized_usage({"users": [user(used_quota=raw)]})
        with self.assertRaisesRegex(S.Error, "unproven"):
            S.normalized_usage(usage_table(999))

    def test_invalid_or_incomplete_audits_rejected(self):
        changes = [("version", 2), ("version", True), ("kind", "unknown"), ("migration_id", ""),
            ("has_complete_history", False), ("usd_credit_conversion", 1000000), ("snapshot_at", 0),
            ("divisor", "1"), ("divisor", "1e3"), ("divisor", "2.1234567890123456789"),
            ("rounding", "bankers"), ("user_sources", None), ("token_sources", None),
            ("user_ids", []), ("user_ids", [1, 1]), ("user_ids", [0]), ("user_sources", []),
            ("user_sources", [{"id": 1, "used_quota": None}]),
            ("user_sources", [{"id": 1, "used_quota": 1000}] * 2), ("other_credit_bases", 0)]
        for key, value in changes:
            with self.subTest(key=key, value=value), self.assertRaises(S.Error):
                S.normalized_usage(usage_table(plan=audit(**{key: value})))
        with self.assertRaises(S.Error):
            S.normalized_usage({"users": [user()], "wallet_credit_rebases": [{"plan": "not-json"}]})

    def test_duplicate_scope_and_token_ids_rejected(self):
        tables = usage_table()
        tables["wallet_credit_rebases"] *= 2
        with self.assertRaises(S.Error):
            S.normalized_usage(tables)
        for tokens in ([{"id": 3, "user_id": 2, "used_quota": 1}],
                       [{"id": 3, "user_id": 1, "used_quota": 1}] * 2):
            with self.assertRaises(S.Error):
                S.normalized_usage(usage_table(plan=audit(token_sources=tokens), tokens=[]))

    def test_missing_token_table_rejected_only_when_audit_has_tokens(self):
        plan = audit(token_sources=[{"id": 3, "user_id": 1, "used_quota": 1}])
        with self.assertRaises(S.Error):
            S.normalized_usage(usage_table(plan=plan))
        self.assertEqual(S.normalized_usage(usage_table(plan=plan, tokens=[])), {1: 500})
        # Missing pre-snapshot token baselines affect Token(), not User().
        self.assertEqual(S.normalized_usage(usage_table(tokens=[{"id": 77, "user_id": 1, "created_time": 1}])), {1: 500})

    def test_reversal_evidence_corruption_rejected(self):
        for key, value in (("user_id", 2), ("charged_quota", 99), ("error_code", "changed"),
                           ("created_at", 91), ("reversed_at", 99), ("reversed_by", 0), ("status", "pending")):
            tables = reversal_fixture()
            tables["violation_fee_records"][0][key] = value
            with self.subTest(key=key), self.assertRaises(S.Error):
                S.normalized_usage(tables)
        tables = reversal_fixture()
        tables["violation_fee_records"] = []
        with self.assertRaises(S.Error):
            S.normalized_usage(tables)

    def test_reversal_source_id_scaled_amount_and_duplicate_rejected(self):
        for key, value in (("source_id", "09"), ("source_id", str(1 << 64)), ("rebased_quota", 49), ("user_id", 2)):
            tables = reversal_fixture()
            plan = json.loads(tables["wallet_credit_rebases"][0]["plan"])
            plan["other_credit_bases"][0][key] = value
            tables["wallet_credit_rebases"][0]["plan"] = json.dumps(plan)
            with self.subTest(key=key), self.assertRaises(S.Error):
                S.normalized_usage(tables)
        tables = reversal_fixture()
        plan = json.loads(tables["wallet_credit_rebases"][0]["plan"])
        plan["other_credit_bases"] *= 2
        tables["wallet_credit_rebases"][0]["plan"] = json.dumps(plan)
        with self.assertRaises(S.Error):
            S.normalized_usage(tables)

    def test_malformed_ignored_go_struct_fields_still_rejected(self):
        plans = [audit(user_sources=[{"id": 1, "user_id": "1", "used_quota": 1000}]),
                 audit(other_credit_bases=[{"kind": "ignored", "original_quota": "1"}])]
        for plan in plans:
            with self.assertRaises(S.Error):
                S.normalized_usage(usage_table(plan=plan))
        tables = reversal_fixture()
        plan = json.loads(tables["wallet_credit_rebases"][0]["plan"])
        plan["other_credit_bases"][0]["source"]["requested_amount_usd"] = "not-a-number"
        tables["wallet_credit_rebases"][0]["plan"] = json.dumps(plan)
        with self.assertRaises(S.Error):
            S.normalized_usage(tables)

    def test_fee_reversals_cannot_exceed_historical_baseline(self):
        with self.assertRaisesRegex(S.Error, "exceed_baseline"):
            S.normalized_usage(reversal_fixture(current=0, baseline=50))


class TopupTests(unittest.TestCase):
    def classify(self, **changes):
        return S.classify_topup(topup(**changes), set())

    def test_cash_uses_settled_refunded_only_and_retains_row_hash(self):
        row = topup()
        result = S.classify_topup(row, set())
        self.assertEqual((result["classification"], result["currency"], result["gross_amount_micros"], result["refunded_amount_micros"]),
                         ("cash", "USD", 1234567, 234567))
        self.assertEqual(result["source_row_sha256"], S.F.digest(row))

    def test_currency_and_source_whitespace_case_normalization(self):
        self.assertEqual(self.classify(payment_provider=" STRIPE\t", settlement_currency=" usd ")["classification"], "cash")

    def test_currency_unicode_expansion_cannot_forge_ascii_iso_code(self):
        self.assertEqual(self.classify(settlement_currency="uﬀ")["classification"], "unconfirmed")
        self.assertEqual(self.classify(settlement_currency="\x1cUSD")["classification"], "unconfirmed")
        self.assertEqual(self.classify(payment_provider="STRİPE")["classification"], "cash")

    def test_every_ldc_alias_and_legacy_epay_method_virtual(self):
        for method in ("ldc", "linuxdo", "linux_do", "linuxdo_credit", "epay"):
            with self.subTest(method=method):
                result = self.classify(payment_method=method, payment_provider="epay", settlement_currency="LDC")
                self.assertEqual(result["classification"], "virtual")
                self.assertEqual(result["gross_amount_micros"], 1234567)

    def test_virtual_requires_immutable_snapshot_before_invalid_amount_branch(self):
        for key in ("expected_amount_micros", "credited_quota"):
            self.assertEqual(self.classify(payment_method="ldc", payment_provider="epay", settlement_currency="LDC",
                settled_amount_micros=-1, **{key: 0})["classification"], "unconfirmed")

    def test_non_epay_ldc_never_becomes_cash(self):
        self.assertEqual(self.classify(settlement_currency="LDC")["classification"], "unconfirmed")

    def test_subscription_any_status_excluded_and_sql_null_does_not_match_null(self):
        for status in ("pending", "success", "failed", "refunded", ""):
            data = fixture({"users": [user()], "top_ups": [topup()], "subscription_orders": [{"trade_no": "synthetic-1", "status": status}]})
            self.assertEqual(S.build_plan(**data)["topups"][0]["classification"], "excluded")
        result = S.classify_topup(topup(trade_no=None), set())
        self.assertEqual(result["classification"], "cash")
        self.assertEqual(S.classify_topup(topup(trade_no=""), {""})["classification"], "excluded")

    def test_internal_sources_excluded_even_with_negative_amount(self):
        for name in ("balance", "gift", "bonus", "checkin", "invite", "bounty", "linuxdo", "linux_do", "linuxdo_credit", "internal", "admin"):
            for field in ("payment_method", "payment_provider"):
                with self.subTest(name=name, field=field):
                    self.assertEqual(self.classify(**{field: name, "settled_amount_micros": -9})["classification"], "excluded")

    def test_known_providers_and_providerless_payment_methods(self):
        for provider in ("epay", "stripe", "creem", "waffo", "waffo_pancake"):
            self.assertEqual(self.classify(payment_provider=provider, payment_method="alipay", settlement_currency="CNY")["classification"], "cash")
        for method in ("stripe", "creem", "waffo", "waffo_pancake", "alipay", "wxpay"):
            self.assertEqual(self.classify(payment_provider="", payment_method=method)["classification"], "cash")
        self.assertEqual(self.classify(payment_provider="unknown", settled_amount_micros=-1)["classification"], "unconfirmed")
        self.assertEqual(self.classify(payment_provider="", payment_method="epay")["classification"], "unconfirmed")

    def test_legacy_epay_non_cny_is_not_external_cash(self):
        self.assertEqual(self.classify(payment_provider="epay", payment_method="epay", settlement_currency="USD")["classification"], "unconfirmed")
        self.assertEqual(self.classify(payment_provider="epay", payment_method="epay", settlement_currency="CNY")["classification"], "cash")

    def test_invalid_and_unconfirmed_boundaries(self):
        for values in ({"settled_amount_micros": -1}, {"refunded_amount_micros": -1}, {"refunded_amount_micros": 1234568}):
            self.assertEqual(self.classify(**values)["classification"], "invalid")
        for values in ({"settled_amount_micros": 0, "refunded_amount_micros": 0}, {"settlement_currency": ""}, {"settlement_currency": "USDT"}):
            self.assertEqual(self.classify(**values)["classification"], "unconfirmed")
        self.assertEqual(self.classify(refunded_amount_micros=1234567)["classification"], "cash")

    def test_missing_float_boolean_amounts_rejected_not_filled_from_expected(self):
        for value in (None, True, 1.25, Decimal("1.0"), S.MAX_I64 + 1):
            with self.subTest(value=value), self.assertRaises(S.Error):
                self.classify(settled_amount_micros=value)

    def test_only_successful_orders_are_included_in_complete_count(self):
        data = fixture({"users": [user()], "subscription_orders": [], "top_ups": [topup(1), topup(2, payment_method="gift"),
            topup(3, payment_provider="unknown"), topup(4, settled_amount_micros=-1), topup(5, status="pending")]})
        plan = S.build_plan(**data)
        self.assertEqual(plan["source"]["topup_count"], 4)
        self.assertEqual([r["classification"] for r in plan["topups"]], ["cash", "excluded", "unconfirmed", "invalid"])
        for row in plan["topups"][1:]:
            self.assertIsNone(row["currency"])
            self.assertIsNone(row["gross_amount_micros"])
            self.assertIsNone(row["refunded_amount_micros"])

    def test_null_status_is_not_a_success_and_missing_status_is_unproven(self):
        args = fixture({"users": [user()], "top_ups": [topup(status=None)], "subscription_orders": []})
        self.assertEqual(S.build_plan(**args)["source"]["topup_count"], 0)
        del args["tables"]["top_ups"][0]["status"]
        with self.assertRaises(S.Error):
            S.build_plan(**args)


class BindingTests(unittest.TestCase):
    def test_retains_disabled_deleted_users_and_does_not_output_secrets(self):
        args = fixture({"users": [user(2, status=2, deleted_at="2026-01-01"), user(1)], "top_ups": [], "subscription_orders": []})
        plan = S.build_plan(**args)
        self.assertEqual([r["user_id"] for r in plan["users"]], [1, 2])
        self.assertNotIn("SYNTHETIC-PRIVATE-PASSWORD", S.F.canonical(plan))
        self.assertEqual(plan["users"][0]["source_row_sha256"], S.F.digest(args["tables"]["users"][1]))

    def test_all_opening_source_fields_are_bound(self):
        for field in ("account_id", "user_id", "source_entity_key", "source_archive_sha256", "mapping_sha256",
                      "source_row_sha256", "observed_at", "used_quota", "request_count", "aff_history"):
            args = fixture()
            row = args["opening"]["counters"][0]
            row[field] = row[field] + 1 if type(row[field]) is int else "bad"
            with self.subTest(field=field), self.assertRaises(S.Error):
                S.build_plan(**args)

    def test_missing_user_table_rows_mapping_or_counter_rejected(self):
        for area in ("tables", "identity_records", "counters", "count"):
            args = fixture()
            if area == "tables":
                args["tables"]["users"] = []
            elif area == "identity_records":
                args[area] = []
            elif area == "counters":
                args["opening"][area] = []
            else:
                args["source_table_counts"]["users"] = 2
            with self.subTest(area=area), self.assertRaises(S.Error):
                S.build_plan(**args)

    def test_missing_known_audit_table_is_not_treated_as_absent(self):
        args = fixture()
        args["source_table_counts"]["wallet_credit_rebases"] = 1
        with self.assertRaisesRegex(S.Error, "coverage"):
            S.build_plan(**args)

    def test_duplicate_source_keys_and_mapping_rejected(self):
        args = fixture()
        args["tables"]["users"] *= 2
        args["source_table_counts"]["users"] = 2
        with self.assertRaises(S.Error):
            S.build_plan(**args)
        args = fixture()
        args["identity_records"] *= 2
        with self.assertRaises(S.Error):
            S.build_plan(**args)

    def test_source_key_is_standard_canonical_sha_rows_use_opening_digest(self):
        plan = S.build_plan(**fixture())
        self.assertEqual(plan["source"]["source_key"], S.F.sha(json.dumps(
            {k: v for k, v in plan["source"].items() if k != "source_key"},
            sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode()))
        self.assertEqual(plan["source"]["rows_sha256"], S.F.digest({"users": plan["users"], "topups": plan["topups"]}))

    def test_review_totals_keep_cash_virtual_and_currencies_separate(self):
        args = fixture({"users": [user(1, used_quota=S.SAFE), user(2, used_quota=S.SAFE)], "subscription_orders": [],
            "top_ups": [topup(1), topup(2, settlement_currency="EUR"),
                        topup(3, payment_method="ldc", payment_provider="epay", settlement_currency="LDC"),
                        topup(4, payment_method="gift")]})
        review = S.review_summary(S.build_plan(**args))
        self.assertEqual(review["normalized_used_credits"], "18014398509481982")
        self.assertEqual(review["classification_counts"], {"cash": 2, "virtual": 1, "excluded": 1, "unconfirmed": 0, "invalid": 0})
        self.assertEqual([(r["classification"], r["currency"], r["net_amount_micros"]) for r in review["recharge_totals"]],
                         [("cash", "EUR", "1000000"), ("cash", "USD", "1000000"), ("virtual", "LDC", "1000000")])

    def test_mutated_plan_hashes_rejected(self):
        for field in ("users", "topups", "source", "normalization"):
            plan = S.build_plan(**fixture())
            if field == "users":
                plan[field][0]["normalized_used_credits"] = 7
            elif field == "topups":
                plan[field].append(S.classify_topup(topup(), set()))
            else:
                plan[field]["unexpected"] = True
            with self.subTest(field=field), self.assertRaises(S.Error):
                S.validate_plan(plan)

    def test_empty_complete_source_has_real_hashes_and_completion_record(self):
        plan = S.build_plan(**fixture({"users": [], "top_ups": [], "subscription_orders": []}))
        self.assertEqual((plan["source"]["user_count"], plan["source"]["topup_count"], plan["source"]["complete"]), (0, 0, True))
        self.assertEqual(S.reconcile(plan, snapshot(plan))["status"], "READY")
        self.assertEqual(len(S.projected_rows(plan)["sources"]), 1)
        self.assertNotEqual(plan["source"]["rows_sha256"], "0" * 64)

    def test_three_way_reconcile_ready_and_exact_replay_noop(self):
        plan = S.build_plan(**fixture())
        target = snapshot(plan)
        before = copy.deepcopy(target)
        self.assertEqual(S.reconcile(plan, target)["status"], "READY")
        self.assertEqual(before, target)
        target["finance"] = S.projected_rows(plan)
        sync_namespace(target)
        self.assertEqual(S.reconcile(plan, target)["status"], "NOOP")

    def test_target_user_account_raw_hash_and_request_conflicts_rejected(self):
        plan = S.build_plan(**fixture())
        for area, field, value in (("native_users", "personal_account_id", 999),
            ("opening_counters", "used_quota", 99), ("opening_counters", "source_row_sha256", "f" * 64),
            ("opening_counters", "request", {})):
            target = snapshot(plan)
            target[area][0][field] = value
            with self.subTest(area=area, field=field), self.assertRaises(S.Error):
                S.reconcile(plan, target)
        target = snapshot(plan)
        target["import_runs"] = []
        with self.assertRaises(S.Error):
            S.reconcile(plan, target)

    def test_partial_completed_other_key_or_different_rows_rejected(self):
        plan = S.build_plan(**fixture())
        for name in ("sources", "users", "topups"):
            target = snapshot(plan)
            target["finance"][name] = [{"different": True}]
            with self.subTest(name=name), self.assertRaises(S.Error):
                S.reconcile(plan, target)
        target = snapshot(plan)
        target["finance"] = S.projected_rows(plan)
        sync_namespace(target)
        target["finance"]["users"][0]["normalized_used_credits"] += 1
        with self.assertRaisesRegex(S.Error, "source_conflict"):
            S.reconcile(plan, target)

    def test_other_archive_never_looks_like_empty_target_namespace(self):
        plan = S.build_plan(**fixture())
        target = snapshot(plan)
        target["finance"]["sources"] = [{**plan["source"], "source_key": "f" * 64, "source_archive_sha256": "e" * 64}]
        sync_namespace(target)
        with self.assertRaisesRegex(S.Error, "target_source_key_conflict"):
            S.reconcile(plan, target)
        target["finance"]["sources"].append(plan["source"])
        sync_namespace(target)
        with self.assertRaisesRegex(S.Error, "target_source_key_conflict"):
            S.reconcile(plan, target)

    def test_leaf_without_completion_is_not_empty_source(self):
        plan = S.build_plan(**fixture())
        target = snapshot(plan)
        target["finance"]["users"] = S.projected_rows(plan)["users"]
        sync_namespace(target)
        with self.assertRaisesRegex(S.Error, "target_source_incomplete"):
            S.reconcile(plan, target)

    def test_target_boolean_does_not_equal_original_integer_counter(self):
        plan = S.build_plan(**fixture({"users": [user(used_quota=1)], "top_ups": [], "subscription_orders": []}))
        target = snapshot(plan)
        target["opening_counters"][0]["used_quota"] = True
        with self.assertRaisesRegex(S.Error, "target_opening_mismatch"):
            S.reconcile(plan, target)


class SqlContractTests(unittest.TestCase):
    def setUp(self):
        self.plan = S.build_plan(**fixture())
        self.target = snapshot(self.plan)
        self.sql = S.build_apply_sql(self.plan, self.target, ["app_runtime"])

    def test_only_three_inserts_completion_last_no_ddl_grants_ledger_calls(self):
        self.assertEqual(re.findall(r"INSERT INTO ([a-z_.]+)", self.sql),
            ["core_history.finance_users", "core_history.finance_topups", "core_history.finance_sources"])
        for pattern in (r"\bCREATE\s", r"\bALTER\s", r"\bDROP\s", r"\bGRANT\s", r"\bREVOKE\s", r"\bUPDATE\s", r"\bDELETE FROM\s", r"\bpost_ledger\(", r"\bimport_opening_balance\("):
            self.assertNotRegex(self.sql, pattern)
        self.assertLess(self.sql.index("INSERT INTO core_history.finance_sources"), self.sql.index("SET CONSTRAINTS ALL IMMEDIATE"))

    def test_serializable_durable_fence_precedes_snapshot_and_no_retry(self):
        self.assertLess(self.sql.index("pg_try_advisory_lock"), self.sql.index("BEGIN ISOLATION LEVEL SERIALIZABLE"))
        self.assertIn("SET LOCAL synchronous_commit=on", self.sql)
        self.assertIn("LOCK TABLE core_identity.users", self.sql)
        self.assertIn("site_finance_target_binding_changed", self.sql)
        self.assertIn("site_finance_target_state_changed", self.sql)
        self.assertIn("site_finance_unrelated_state_changed", self.sql)
        self.assertEqual(self.sql.count("COMMIT;"), 1)

    def test_runtime_owner_current_identity_and_offline_guards_present(self):
        for token in ("current_user", "session_user", "NOT rolcanlogin", "pg_get_userbyid(relowner)",
                      "has_any_column_privilege", "site_finance_runtime_writes_forbidden", "inet_server_addr()", "listen_addresses"):
            self.assertIn(token, self.sql)

    def test_readonly_snapshot_and_upgrade_receipt_are_preserved(self):
        sql = S.snapshot_sql(self.target["state"]["inventory"], ["app_runtime"])
        self.assertIn("REPEATABLE READ READ ONLY", sql)
        self.assertNotIn("INSERT INTO", sql)
        state = S.protected_state(self.target)
        self.assertIn("core_meta.site_finance_upgrades", state["tables"])
        self.assertNotIn("core_history.finance_sources", state["tables"])

    def test_runtime_role_sql_injection_rejected(self):
        with self.assertRaises(S.Error):
            S.build_apply_sql(self.plan, self.target, ["x'); END $$; DROP TABLE x;--"])


class SourceSealTests(unittest.TestCase):
    def setUp(self):
        self.plan = S.build_plan(**fixture())
        self.target = snapshot(self.plan)

    def intents(self, upper, maximum):
        self.target.update(native_intent_upper=upper, native_intent_max_id=maximum)
        self.target["state"]["sequences"][S.INTENT_SEQUENCE].update(last_value=upper, is_called=True)

    def test_uncalled_empty_sequence_is_zero_only_when_no_native_ids(self):
        self.assertEqual(S.reconcile(self.plan, self.target)["native_intent_upper"], 0)
        self.target["native_intent_max_id"] = 8
        with self.assertRaisesRegex(S.Error, "sequence_invalid"):
            S.reconcile(self.plan, self.target)

    def test_source_material_cannot_supply_server_floor_even_when_rehashed(self):
        for key, value in (("native_intent_floor", 99), ("native_request_ids", ["caller-supplied"])):
            plan = copy.deepcopy(self.plan)
            plan["source"][key] = value
            plan["source"]["source_key"] = S.canonical_sha({k: v for k, v in plan["source"].items() if k != "source_key"})
            plan["plan_sha256"] = S.canonical_sha({k: v for k, v in plan.items() if k != "plan_sha256"})
            with self.subTest(key=key), self.assertRaisesRegex(S.Error, "source_material_fields_invalid"):
                S.validate_plan(plan)

    def test_first_insert_omits_default_floor_and_binds_actual_high_water(self):
        self.intents(32, 17)
        sql = S.build_apply_sql(self.plan, self.target, ["app_runtime"])
        insert = re.search(r"INSERT INTO core_history.finance_sources \(([^)]+)\) SELECT ([^;]+);", sql).group(0)
        self.assertNotIn("native_intent_floor", insert)
        self.assertNotIn("SELECT *", insert)
        self.assertIn('ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE "C")', insert)
        self.assertNotIn("r.native_request_ids", insert)
        self.assertIn('"native_intent_floor":32', sql)
        self.assertIn("site_finance_source_seal_mismatch", sql)
        self.assertNotRegex(sql, r"\b(?:nextval|setval)\s*\(")
        self.assertIn("to_jsonb(r)-'native_intent_floor'", S.finance_sql())
        self.assertEqual(len(self.plan["source"]), 12)

    def test_noop_preserves_floor_after_new_native_ids(self):
        self.intents(17, 17)
        self.target["finance"] = S.projected_rows(self.plan)
        sync_namespace(self.target)
        original_key = self.plan["source"]["source_key"]
        self.intents(23, 23)
        result = S.reconcile(self.plan, self.target)
        self.assertEqual(result["status"], "NOOP")
        self.assertEqual(result["source_seals"], [{"source_key": original_key, "native_intent_floor": 17, "native_request_ids": []}])
        self.assertEqual(result["native_intent_upper"], 23)
        sql = S.build_apply_sql(self.plan, self.target, ["app_runtime"])
        self.assertIn('"native_intent_floor":17', sql)
        self.assertNotIn('"native_intent_floor":23', sql)
        self.assertEqual(self.plan["source"]["source_key"], original_key)

    def test_invalid_or_missing_seal_proof_is_rejected(self):
        self.intents(17, 17)
        self.target["finance"] = S.projected_rows(self.plan)
        sync_namespace(self.target)
        for floor in (None, True, -1, 18, "17"):
            target = copy.deepcopy(self.target)
            target["source_seals"][0]["native_intent_floor"] = floor
            with self.subTest(floor=floor), self.assertRaises(S.Error):
                S.reconcile(self.plan, target)
        self.target["source_seals"] = []
        with self.assertRaisesRegex(S.Error, "seals_mismatch"):
            S.reconcile(self.plan, self.target)

    def test_nonunit_cycle_rewind_and_manual_high_id_are_rejected(self):
        for key, value in (("increment", 2), ("increment", True), ("cycle", True)):
            target = copy.deepcopy(self.target)
            target["state"]["sequences"][S.INTENT_SEQUENCE][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(S.Error, "sequence_invalid"):
                S.reconcile(self.plan, target)
        self.intents(5, 7)
        with self.assertRaisesRegex(S.Error, "sequence_invalid"):
            S.reconcile(self.plan, self.target)
        self.intents(7, 7)
        self.target["finance"] = S.projected_rows(self.plan)
        sync_namespace(self.target)
        self.intents(5, 4)
        with self.assertRaisesRegex(S.Error, "source_floor_invalid"):
            S.reconcile(self.plan, self.target)

    def test_reported_sequence_upper_must_match_sequence_state(self):
        self.target["native_intent_upper"] = 99
        with self.assertRaisesRegex(S.Error, "upper_mismatch"):
            S.reconcile(self.plan, self.target)

    def test_owner_guard_runs_sequence_checks_for_first_apply_and_replay(self):
        guard = S._owner_guard(["app_runtime"])
        for text in ("seqincrement=1", "NOT seqcycle", "COALESCE(max(id),0)", "native_intent_floor >",
                     "site_finance_native_intent_sequence_invalid", "has_sequence_privilege",
                     "site_finance_native_intent_sequence_access_denied"):
            self.assertIn(text, guard)
        sql = S.build_apply_sql(self.plan, self.target, ["app_runtime"])
        self.assertLess(sql.index("site_finance_native_intent_sequence_invalid"), sql.index("replay :="))

    def test_drain_guard_uses_public_pid_not_masked_backend_type(self):
        guard = S._owner_guard(["app_runtime"])
        self.assertNotIn("backend_type", guard)
        self.assertNotIn("pg_read_all_stats", guard)
        self.assertIn("datid=(SELECT oid FROM pg_database WHERE datname=current_database())", guard)
        self.assertIn("pid IS NOT NULL AND pid<>pg_backend_pid()", guard)

    def test_opaque_request_ids_are_preserved_in_byte_order_and_not_recaptured(self):
        old_ids = ["A:pending", "z/opaque", "北.request"]
        self.target["native_request_ids"] = list(old_ids)
        self.target["state"]["tables"]["core_charging.requests"]["rows"] = 3
        self.target["finance"] = S.projected_rows(self.plan)
        sync_namespace(self.target)
        self.target["native_request_ids"] = sorted(old_ids + ["0:new-same-second"], key=lambda x: x.encode())
        self.target["state"]["tables"]["core_charging.requests"]["rows"] = 4
        result = S.reconcile(self.plan, self.target)
        self.assertEqual(result["status"], "NOOP")
        self.assertEqual(result["source_seals"][0]["native_request_ids"], old_ids)
        sql = S.build_apply_sql(self.plan, self.target, ["app_runtime"])
        self.assertIn('"native_request_ids":["A:pending","z/opaque","北.request"]', sql)
        self.assertIn("core_charging.requests IN SHARE MODE", sql)

    def test_sealed_request_missing_from_current_target_is_refused(self):
        self.target["native_request_ids"] = ["pending-later-settled"]
        self.target["state"]["tables"]["core_charging.requests"]["rows"] = 1
        self.target["finance"] = S.projected_rows(self.plan)
        sync_namespace(self.target)
        self.target["native_request_ids"] = []
        self.target["state"]["tables"]["core_charging.requests"]["rows"] = 0
        with self.assertRaisesRegex(S.Error, "target_source_request_missing"):
            S.reconcile(self.plan, self.target)
        self.assertIn("site_finance_source_request_missing", S._owner_guard(["app_runtime"]))

    def test_request_inventory_and_seal_cannot_contain_null_duplicates_or_wrong_order(self):
        for ids in (None, [None], [True], ["x", "x"], ["z", "A"]):
            target = copy.deepcopy(self.target)
            target["native_request_ids"] = ids
            with self.subTest(ids=ids), self.assertRaisesRegex(S.Error, "request_ids_invalid"):
                S.reconcile(self.plan, target)
        self.target["native_request_ids"] = ["x"]
        with self.assertRaisesRegex(S.Error, "request_count_mismatch"):
            S.reconcile(self.plan, self.target)

    def test_finance_projection_keeps_material_and_exposes_both_seal_proofs(self):
        sql = S.target_sql(self.target["state"]["inventory"])
        self.assertIn("to_jsonb(r)-'native_intent_floor'-'native_request_ids'", sql)
        self.assertIn("'native_request_ids',native_request_ids", sql)
        self.assertIn("'source_seals'", sql)


class PrivateFilesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.backup_patch = patch.object(S.I, "BACKUP_ROOT", self.root)
        self.backup_patch.start()
        self.addCleanup(self.backup_patch.stop)
        self.input = self.root / "source"
        self.input.mkdir(mode=0o700)
        self.tables = {name: [] for name in S.P.REQUIRED_TABLES}
        self.tables.update(users=[user()], subscription_orders=[], violation_fee_records=[{"id": 9, "user_id": 1,
            "charged_quota": 100, "status": "charged", "created_at": 90, "reversed_at": 0, "reversed_by": 0, "error_code": "abuse"}])
        self.files = {}
        self.write("archive", "SYNTHETIC frozen archive fixture, no database dump")
        archive = self.files["archive"]["sha256"]
        self.records = [{"source_user_id": 1, "target_user_id": 1, "target_personal_account_id": 101}]
        self.write("identity_map", S.F.canonical(self.records[0]) + "\n")
        manifest = {"format_version": 1, "scope": "identity-only", "source_archive_sha256": archive,
            "mapping_sha256": "b" * 64, "source_target_id_map_sha256": self.files["identity_map"]["sha256"]}
        self.write("identity_manifest", manifest)
        provenance = S.P.bind_identity_provenance(manifest, map_file_sha256=self.files["identity_map"]["sha256"], source_archive_sha256=archive)
        financial = S.P.plan_financial(self.tables, {1: 101}, source_sha256=archive, identity_provenance=provenance)
        self.write("financial_plan", financial)
        self.write("source_review", {"synthetic_review": True})
        observation = S.P.source_observation({"started_utc": "20261001T000000Z"})
        reservations = {"source_archive_sha256": archive, "mapping_sha256": financial["mapping_sha256"],
                        "reviewed_evidence_sha256": "d" * 64, "positions": [{"source_user_id": 1, "reserved_units": 0}]}
        self.write("reservations", reservations)
        opening = S.P.build_opening_payloads(financial, migration_id="synthetic-opening", observation=observation, reservation_reconciliation=reservations)
        self.opening = {"scope": S.F.SCOPE, "migration_id": "synthetic-opening", "source_archive_sha256": archive,
            "mapping_sha256": financial["mapping_sha256"], "source_plan": financial, "source_files": self.files,
            "identity_map_sha256": S.F.digest({1: 101}), "identity_import": {"migration_id": "synthetic-identity",
                "source_archive_sha256": archive, "mapping_sha256": manifest["mapping_sha256"], "manifest": manifest},
            "native_users": [{"id": 1, "personal_account_id": 101}], "observation": observation,
            "opening": opening["payloads"], "counters": S.P.build_opening_counter_payloads(financial, opening)["payloads"],
            "source_review": {"synthetic_review": True}}
        self.instance = {"database": "synthetic_source", "role": "source_reader", "database_oid": "99",
            "postmaster_started_at": "2026-09-30 00:00:00+00", "server_version_num": "180000", "port": 5433,
            "tcp_connection": False, "read_only": "on", "schema_owner": "source_owner"}
        columns = {name: sorted(rows[0]) if rows else ["id"] for name, rows in self.tables.items()}
        column_rows = [{"schema": "public", "table_name": name, "column_name": col} for name in sorted(columns) for col in columns[name]]
        constraints = [{"schema": "public", "table_name": name, "contype": "p", "definition": "PRIMARY KEY (id)"} for name in sorted(columns)]
        catalog = S.F.write_private(self.input, "native-catalog-metadata.jsonl", "\n".join(S.F.canonical(v) for v in ([], [], column_rows, constraints, [], [], [])) + "\n")
        counts = S.F.write_private(self.input, "native-table-row-count-baseline.jsonl", "\n".join(S.F.canonical({"table": n, "row_count": len(r)}) for n, r in self.tables.items()) + "\n")
        self.metadata = {p.name: S.I.sha256_file(p) for p in (catalog, counts)}
        receipt = {"status": "snapshot-restored-baseline-verified-frozen-and-classified", "started_utc": "20261001T000000Z",
            "archive": self.files["archive"], "private_metadata_files": [str(catalog), str(counts)], "source": {"schema": "public"},
            "readonly_reader": {"database": "synthetic_source", "role": "source_reader", "port": 5433},
            "source_instance_identity": {**self.instance, "listen_addresses": ""}}
        self.write("source_receipt", receipt)
        self.write_export(self.tables)
        self.path = S.F.write_private(self.input, "opening.private.json", S.F.canonical(self.opening))

    def write(self, name, value):
        content = value if type(value) is str else S.F.canonical(value) + "\n"
        path = self.input / (name + ".private")
        if path.exists():
            path.unlink()  # Own disposable synthetic fixture only.
        S.F.write_private(self.input, path.name, content)
        self.files[name] = {"path": str(path), "sha256": S.F.sha(content.encode())}
        return path

    def write_export(self, selected, prefix="source"):
        raw = [{"kind": "identity", "row": self.instance}, {"kind": "schema", "row": {"schema": "public", "schema_owner": "source_owner"}}]
        raw += [{"kind": "row", "table": n, "row": row} for n in sorted(selected) for row in selected[n]]
        self.write(prefix + "_export", "\n".join(S.F.canonical(r) for r in raw) + "\n")
        self.write(prefix + "_export_proof", {"source_archive_sha256": self.files["archive"]["sha256"],
            "source_export_sha256": self.files[prefix + "_export"]["sha256"], "metadata_hashes": self.metadata,
            "source_column_coverage_exact": True, "source_row_baseline_exact": True,
            "selected_table_counts": {n: len(r) for n, r in selected.items()}, "source_instance": self.instance})

    def rewrite_opening(self):
        self.path.write_text(S.F.canonical(self.opening))

    def material(self):
        tables, opening, records, counts = S.load_source(self.path)
        plan = S.build_plan(tables=tables, opening=opening, identity_records=records, source_table_counts=counts)
        return S.prepare(self.path, snapshot(plan), ["app_runtime"])

    def output(self, name="attempt"):
        path = self.root / name
        path.mkdir(mode=0o700)
        return path

    def test_real_files_parse_hash_and_prepare_without_database(self):
        with patch.object(S.F, "run_sql") as run:
            material = self.material()
            self.assertEqual(S.verify_inputs(material), material["plan"])
            run.assert_not_called()
        self.assertEqual(material["plan"]["source"]["user_count"], 1)
        self.assertNotIn("SYNTHETIC-PRIVATE-PASSWORD", S.F.canonical(material))

    def test_every_raw_source_file_hash_is_rechecked(self):
        for name in tuple(self.files):
            path = Path(self.files[name]["path"])
            before = path.read_bytes()
            path.write_bytes(before + b" ")
            with self.subTest(name=name), self.assertRaisesRegex(S.Error, "source_file_changed"):
                S.load_source(self.path)
            path.write_bytes(before)

    def test_private_permissions_and_symlink_rejected(self):
        path = Path(self.files["source_export"]["path"])
        path.chmod(0o644)
        with self.assertRaises(S.Error):
            S.load_source(self.path)
        path.chmod(0o600)
        link = self.input / "linked-export"
        link.symlink_to(path)
        self.files["source_export"]["path"] = str(link)
        self.rewrite_opening()
        with self.assertRaises(S.Error):
            S.load_source(self.path)

    def test_known_nonempty_fee_table_requires_supplement(self):
        selected = {k: v for k, v in self.tables.items() if k != "violation_fee_records"}
        self.write_export(selected)
        self.rewrite_opening()
        with self.assertRaisesRegex(S.Error, "required_table_not_exported"):
            S.load_source(self.path)
        self.write_export(self.tables, "site")
        supplement = {"export": self.files.pop("site_export"), "proof": self.files.pop("site_export_proof")}
        tables, _, _, _ = S.load_source(self.path, supplement)
        self.assertEqual(tables["violation_fee_records"], self.tables["violation_fee_records"])
        Path(supplement["export"]["path"]).write_text("changed")
        with self.assertRaisesRegex(S.Error, "supplement_file_changed"):
            S.load_source(self.path, supplement)

    def test_rehashed_incomplete_export_cannot_forge_full_counts(self):
        self.write_export({**self.tables, "users": []})
        self.rewrite_opening()
        with self.assertRaisesRegex(S.Error, "export_counts_mismatch"):
            S.load_source(self.path)

    def test_rehashed_original_user_cannot_change_opening_source_hash(self):
        changed = copy.deepcopy(self.tables)
        changed["users"][0]["used_quota"] += 1
        self.write_export(changed)
        self.rewrite_opening()
        with self.assertRaisesRegex(S.Error, "opening_table_material_changed"):
            self.material()

    def test_material_source_byte_change_rejected_before_transport(self):
        material = self.material()
        self.path.write_text(self.path.read_text() + " ")
        with patch.object(S.F, "run_sql") as run, self.assertRaisesRegex(S.Error, "opening_material_changed"):
            S.apply_offline(material, approval={}, connection={}, output=self.output())
        run.assert_not_called()

    def test_explicit_approval_bound_to_exact_sql_and_material(self):
        material = self.material()
        with patch.object(S.F, "run_sql") as run, self.assertRaisesRegex(S.Error, "owner_approval_mismatch"):
            S.apply_offline(material, approval={}, connection={}, output=self.output())
        run.assert_not_called()

    def test_changed_tool_contract_rejected_before_transport(self):
        material = self.material()
        material["tool_hashes"]["site_statistics_writer"] = "0" * 64
        with patch.object(S.F, "run_sql") as run, self.assertRaisesRegex(S.Error, "tool_source_changed"):
            S.apply_offline(material, approval={}, connection={}, output=self.output())
        run.assert_not_called()

    def test_cli_prepare_emits_review_and_non_executable_approval_request(self):
        material = self.material()
        snap = S.F.write_private(self.input, "target.private.json", S.F.canonical(material["snapshot"]))
        output = self.output()
        arguments = ["legacy_site_statistics.py", "--mode", "prepare", "--opening-material", str(self.path),
                     "--snapshot", str(snap), "--runtime-role", "app_runtime", "--output", str(output)]
        with patch.object(sys, "argv", arguments), patch.object(sys, "stdout", new_callable=io.StringIO) as stdout, \
             patch.object(S.F, "run_sql") as run:
            self.assertEqual(S.main(), 0)
            self.assertEqual(json.loads(stdout.getvalue())["status"], "READY")
            run.assert_not_called()
        request = json.loads((output / "approval-request.private.json").read_text())
        self.assertEqual(request["decision"], "REVIEW_REQUIRED")
        self.assertEqual(request["sql_sha256"], S.I.sha256_file(output / "review.private.sql"))
        self.assertEqual(json.loads((output / "review.private.json").read_text())["normalized_used_credits"], "1000")

    def drive(self, material, output):
        sql = S.build_apply_sql(material["plan"], material["snapshot"], material["runtime_roles"])
        approval = {"decision": "approve-offline-site-finance", "material_sha256": S.canonical_sha(material),
            "sql_sha256": S.F.sha(sql.encode()), "source_key": material["plan"]["source"]["source_key"]}
        connection = {"role": "offline_owner", "login_role": "offline_operator", "set_role": "offline_owner",
                      "database": "synthetic_finance", "port": 5432}
        return S.apply_offline(material, approval=approval, connection=connection, output=output)

    def test_driver_writes_durable_private_attempt_before_one_transport_call(self):
        material = self.material()
        output = self.output()
        def transport(*args, **kwargs):
            for name in ("attempt.private.json", "executed.private.sql"):
                self.assertTrue((output / name).is_file())
                self.assertEqual((output / name).stat().st_mode & 0o777, 0o600)
            return 0, S.F.canonical({"status": "COMMITTED", "source_key": material["plan"]["source"]["source_key"],
                                     "plan_sha256": material["plan"]["plan_sha256"]})
        with patch.object(S.I, "socket_connection", return_value={}), patch.object(S.F, "run_sql", side_effect=transport) as run:
            self.assertEqual(self.drive(material, output)["status"], "COMMITTED")
        run.assert_called_once()

    def test_driver_uncertain_transport_never_retries_or_claims_rollback(self):
        material = self.material()
        for index, result in enumerate(((1, ""), (0, "garbage"), (0, "{}"), OSError("synthetic"), subprocess.TimeoutExpired("synthetic", 1))):
            with self.subTest(index=index), patch.object(S.I, "socket_connection", return_value={}), \
                 patch.object(S.F, "run_sql", **({"side_effect": result} if isinstance(result, Exception) else {"return_value": result})) as run:
                self.assertEqual(self.drive(material, self.output(str(index)))["status"], "UNKNOWN")
                run.assert_called_once()

    def test_driver_post_transport_input_change_is_unknown(self):
        material = self.material()
        def transport(*args, **kwargs):
            Path(self.files["archive"]["path"]).write_text("changed after submission")
            return 0, S.F.canonical({"status": "COMMITTED", "source_key": material["plan"]["source"]["source_key"],
                                     "plan_sha256": material["plan"]["plan_sha256"]})
        with patch.object(S.I, "socket_connection", return_value={}), patch.object(S.F, "run_sql", side_effect=transport):
            self.assertEqual(self.drive(material, self.output())["status"], "UNKNOWN")

    def test_driver_lost_result_file_after_success_is_unknown(self):
        material = self.material()
        receipt = {"status": "NOOP", "source_key": material["plan"]["source"]["source_key"], "plan_sha256": material["plan"]["plan_sha256"]}
        write = S.F.write_private
        def failing_write(directory, name, content):
            if name == "result.private.json":
                raise OSError("synthetic full disk")
            return write(directory, name, content)
        with patch.object(S.I, "socket_connection", return_value={}), \
             patch.object(S.F, "run_sql", return_value=(0, S.F.canonical(receipt))) as run, \
             patch.object(S.F, "write_private", side_effect=failing_write):
            self.assertEqual(self.drive(material, self.output())["status"], "UNKNOWN")
            run.assert_called_once()


if __name__ == "__main__":
    unittest.main()
