#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Offline observations only; no ledger, grants, credentials or payment execution.

Usage and payment rules ported from the AGPL-3.0-or-later legacy LMM API
3948a0a1ee1cefdd016a7aa45a11851545bb4b85 (QuantumNous/new-api lineage):
usage_projection.go, referral_credit_rebase.go, admin_site_statistics.go, topup.go.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from decimal import Decimal
from fractions import Fraction
from pathlib import Path
import re
import sys
from typing import Any, Literal, TypedDict
import uuid

import financial_import as F

P = F.PLANNER
I = F.IDENTITY
require = F.require
Error = F.Error
CONTRACT = "core.site-finance-source.v1"
REVISION = "3948a0a1ee1cefdd016a7aa45a11851545bb4b85"
SAFE = (1 << 53) - 1
MAX_I64 = (1 << 63) - 1
TABLES = ("users", "top_ups", "subscription_orders", "wallet_credit_rebases", "tokens", "violation_fee_records")
DESTINATIONS = tuple("core_history.finance_" + name for name in ("sources", "users", "topups"))
INTENT_SEQUENCE = "core_payments.intents_id_seq"
INTENT_UPPER_SQL = "COALESCE(pg_catalog.pg_sequence_last_value('core_payments.intents_id_seq'::regclass),0)"
REQUEST_IDS_SQL = 'ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE "C")'
COUNTER_KEYS = ("account_id", "user_id", "migration_id", "source_entity_key", "source_archive_sha256",
                "mapping_sha256", "source_row_sha256", "observed_at", "used_quota", "request_count", "aff_history")
Classification = Literal["cash", "virtual", "unconfirmed", "invalid", "excluded"]


class FinanceUser(TypedDict):
    user_id: int
    account_id: int
    source_row_sha256: str
    raw_used_quota: int
    normalized_used_credits: int


class FinanceTopup(TypedDict):
    source_order_id: int
    source_row_sha256: str
    classification: Classification
    currency: str | None
    gross_amount_micros: int | None
    refunded_amount_micros: int | None


class FinanceSource(TypedDict):
    source_key: str
    contract: str
    source_archive_sha256: str
    mapping_sha256: str
    tables_sha256: str
    normalization_sha256: str
    rows_sha256: str
    observed_at: int
    user_count: int
    topup_count: int
    credits_per_usd: int
    complete: bool


class FinancePlan(TypedDict):
    contract: str
    source: FinanceSource
    users: list[FinanceUser]
    topups: list[FinanceTopup]
    normalization: dict[str, Any]
    opening_counters: list[dict[str, Any]]
    identity_import: dict[str, Any]
    plan_sha256: str


class SourceSeal(TypedDict):
    source_key: str
    native_intent_floor: int
    native_request_ids: list[str]


def integer(value: Any, code: str, minimum: int = 0, maximum: int = MAX_I64) -> int:
    return P.integer(value, code, minimum=minimum, maximum=maximum)


def canonical_sha(value: Any) -> str:
    return F.sha(F.canonical(value).encode("utf-8"))


def request_ids(value: Any, code: str) -> list[str]:
    """Opaque PostgreSQL text IDs in byte order; no timestamp or ID format inference."""
    require(type(value) is list and all(type(item) is str for item in value), code)
    require(value == sorted(set(value), key=lambda item: item.encode("utf-8")), code)
    return value


def indexed(rows: list[dict], field: str = "id") -> dict[int, dict]:
    require(type(rows) is list, "source_rows_invalid")
    result = {}
    for row in rows:
        require(type(row) is dict, "source_row_invalid")
        key = integer(row.get(field), "source_id_invalid", 1)
        require(key not in result, "source_id_duplicate")
        result[key] = row
    return result


def go_int(row: dict, key: str) -> int:
    """Go's zero default applies only to absent audit struct fields, never source amounts."""
    value = row.get(key)
    return integer(0 if value is None else value, "audit_integer_invalid", -MAX_I64 - 1)


def validate_struct(row: dict, *, integers: tuple[str, ...] = (), strings: tuple[str, ...] = ()) -> None:
    """Reject invalid known Go struct fields even if this projection ignores them."""
    require(type(row) is dict, "invalid_migration_audit")
    for key in integers:
        go_int(row, key)
    for key in strings:
        require(row.get(key) is None or type(row[key]) is str, "invalid_migration_audit")


def legacy_case(value: str, *, upper: bool = False) -> str:
    # Go strings.TrimSpace uses Unicode White_Space, not Python's extra C0
    # separators. Go case conversion maps one rune, never expands ligatures.
    spaces = " \t\n\r\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000"
    result = []
    for char in value.strip(spaces):
        mapped = char.upper() if upper else char.lower()
        result.append(mapped if len(mapped) == 1 else ("i" if char == "\u0130" and not upper else char))
    return "".join(result)


def text(row: dict, key: str, *, coalesce: bool = False) -> str:
    require(key in row, "source_column_missing")
    value = row[key]
    if coalesce and value is None:
        return ""
    require(type(value) is str, "source_text_invalid")
    return value


def scaled(raw: int, divisor: Fraction, rounding: str) -> int:
    quotient, remainder = divmod(raw * divisor.denominator, divisor.numerator)
    return quotient + int(rounding == "half-away-from-zero" and remainder * 2 >= divisor.numerator)


@dataclass
class UsageBaseline:
    raw: int
    divisor: Fraction
    rounding: str
    migration_id: str
    reversed_raw: int = 0
    reversed_normalized: int = 0


def normalized_usage(tables: dict[str, list[dict]]) -> dict[int, int]:
    """Legacy LoadUsageProjector + User; all audits validated, even non-retained scopes.

    Absent audit tables must be established by the frozen catalog, not by omitting
    an available table. The frozen-file adapter enforces that distinction.
    """
    users = indexed(tables["users"])
    baselines: dict[int, UsageBaseline] = {}
    scopes: set[int] = set()
    token_ids: set[int] = set()
    audits = tables.get("wallet_credit_rebases", [])
    fees = indexed(tables.get("violation_fee_records", [])) if audits else {}
    for audit in audits:
        require(type(audit.get("plan")) is str, "invalid_migration_audit")
        try:
            plan = P.parse_json(audit["plan"])
        except ValueError:
            raise Error("invalid_migration_audit") from None
        require(type(plan) is dict, "invalid_migration_audit")
        divisor_text = plan.get("divisor")
        require(type(divisor_text) is str and re.fullmatch(r"[0-9]+(?:\.[0-9]{1,18})?", divisor_text),
                "unsupported_or_incomplete_migration_basis")
        divisor = Fraction(divisor_text)
        require(type(plan.get("version")) is int and plan["version"] == 1
                and plan.get("kind") == "offline_credit_balance_rebase_preview"
                and type(plan.get("migration_id")) is str and bool(plan["migration_id"])
                and plan.get("has_complete_history") is True
                and type(plan.get("usd_credit_conversion")) is int and plan["usd_credit_conversion"] == 500000
                and go_int(plan, "snapshot_at") > 0 and divisor > 1
                and plan.get("rounding") in ("half-away-from-zero", "toward-zero")
                and type(plan.get("user_sources")) is list and type(plan.get("token_sources")) is list,
                "unsupported_or_incomplete_migration_basis")
        selected: set[int] = set()
        require(type(plan.get("user_ids")) is list, "invalid_migration_scope")
        for uid in plan["user_ids"]:
            integer(uid, "invalid_migration_scope", 1)
            require(uid not in selected and uid not in scopes, "duplicate_user_migration_scope")
            selected.add(uid)
            scopes.add(uid)
        require(bool(selected), "empty_migration_scope")
        seen_users = set()
        for row in plan["user_sources"]:
            require(type(row) is dict, "invalid_user_historical_baseline")
            validate_struct(row, integers=("id", "user_id"))
            uid = integer(row.get("id"), "invalid_user_historical_baseline", 1)
            require(uid in selected and uid not in seen_users, "invalid_user_historical_baseline")
            raw = integer(row.get("used_quota"), "invalid_user_historical_baseline", 0, SAFE)
            baselines[uid] = UsageBaseline(raw, divisor, plan["rounding"], plan["migration_id"])
            seen_users.add(uid)
        require(seen_users == selected, "selected_user_historical_baseline_missing")
        for row in plan["token_sources"]:
            require(type(row) is dict, "invalid_token_historical_baseline")
            tid = integer(row.get("id"), "invalid_token_historical_baseline", 1)
            require(tid not in token_ids and row.get("user_id") in selected, "invalid_token_historical_baseline")
            integer(row.get("user_id"), "invalid_token_historical_baseline", 1)
            integer(row.get("used_quota"), "invalid_token_historical_baseline", 0, SAFE)
            token_ids.add(tid)
        require("tokens" in tables or not plan["token_sources"], "token_source_table_missing")
        # LoadUsageProjector records missing old tokens but only Token() rejects
        # them. This all-user projection never calls Token().
        bases = plan.get("other_credit_bases")
        bases = [] if bases is None else bases
        require(type(bases) is list, "invalid_historical_fee_reversal_basis")
        seen_fees: set[int] = set()
        for child in bases:
            require(type(child) is dict, "invalid_historical_fee_reversal_basis")
            validate_struct(child, integers=("user_id", "original_quota", "rebased_quota"), strings=("kind", "source_id"))
            if child.get("kind") != "violation_fee_refund":
                continue
            sid = child.get("source_id")
            require(type(sid) is str and re.fullmatch(r"[1-9][0-9]*", sid), "invalid_historical_fee_reversal_basis")
            fid = integer(int(sid), "invalid_historical_fee_reversal_basis", 1, (1 << 64) - 1)
            uid = integer(child.get("user_id"), "invalid_historical_fee_reversal_basis", 1)
            source = child.get("source")
            require(type(source) is dict and fid not in seen_fees and uid in selected,
                    "invalid_historical_fee_reversal_basis")
            original = integer(child.get("original_quota"), "invalid_historical_fee_reversal_basis", 1, SAFE)
            validate_struct(source, integers=("id", "user_id", "occurrence", "period_started_at", "period_ends_at",
                "requested_quota", "charged_quota", "created_at", "reversed_at", "reversed_by"),
                strings=("request_id", "policy_key", "group", "amount_currency", "error_code", "status"))
            for key in ("requested_amount_usd", "charged_amount_usd"):
                value = source.get(key)
                require(value is None or (type(value) in (int, Decimal) and
                        abs(value) <= Decimal("1.7976931348623157e308")), "invalid_historical_fee_reversal_basis")
            source = {**source, "error_code": source.get("error_code") or "",
                      "created_at": go_int(source, "created_at")}
            require(go_int(source, "id") == fid and go_int(source, "user_id") == uid
                    and go_int(source, "charged_quota") == original and source.get("status") == "charged"
                    and go_int(source, "reversed_at") == 0 and go_int(source, "created_at") <= plan["snapshot_at"],
                    "invalid_historical_fee_reversal_basis")
            seen_fees.add(fid)
            baseline = baselines[uid]
            rebased = go_int(child, "rebased_quota")
            require(rebased == scaled(original, divisor, baseline.rounding), "inconsistent_historical_fee_reversal_units")
            current = fees.get(fid)
            require(current is not None, "historical_fee_reversal_record_missing")
            require(all(current.get(k) == source.get(k) for k in ("user_id", "charged_quota", "error_code", "created_at")),
                    "historical_fee_reversal_source_changed")
            status = text(current, "status")
            if status == "charged":
                require(go_int(current, "reversed_at") == 0, "inconsistent_charged_historical_fee")
            elif status == "reversed":
                require(go_int(current, "reversed_at") >= plan["snapshot_at"] and go_int(current, "reversed_by") > 0,
                        "unproven_historical_fee_reversal")
                if not text(current, "error_code").startswith("moderation."):
                    baseline.reversed_raw += original
                    baseline.reversed_normalized += rebased
                    require(baseline.reversed_raw <= baseline.raw, "historical_fee_reversals_exceed_baseline")
            else:
                raise Error("unsupported_historical_fee_reversal_state")
        for fid, fee in fees.items():
            if (fee.get("user_id") in selected and go_int(fee, "created_at") <= plan["snapshot_at"]
                    and fee.get("status") == "reversed" and go_int(fee, "reversed_at") >= plan["snapshot_at"]
                    and not text(fee, "error_code").startswith("moderation.")):
                require(fid in seen_fees, "historical_fee_reversal_audit_missing")
    result = {}
    for uid, user in users.items():
        raw = integer(user.get("used_quota"), "current_usage_outside_safe_integer_domain", 0, SAFE)
        base = baselines.get(uid)
        if base is None:
            result[uid] = raw
            continue
        historical = scaled(base.raw, base.divisor, base.rounding) - base.reversed_normalized
        delta = raw - base.raw + base.reversed_raw
        require(delta >= 0, "unproven_historical_usage_counter_reduction")
        require(historical >= 0 and 0 <= historical + delta <= SAFE, "normalized_usage_outside_safe_integer_domain")
        result[uid] = historical + delta
    return result


def classify_topup(row: dict, subscriptions: set[str]) -> FinanceTopup:
    """Exact legacy branch ordering, including excluded orders with bad amounts."""
    result: FinanceTopup = {"source_order_id": integer(row.get("id"), "topup_id_invalid", 1),
        "source_row_sha256": F.digest(row), "classification": "excluded", "currency": None,
        "gross_amount_micros": None, "refunded_amount_micros": None}
    require(text(row, "status") == "success", "topup_not_successful")
    require("trade_no" in row, "topup_trade_missing")
    trade = row.get("trade_no")
    require(trade is None or type(trade) is str, "topup_trade_invalid")
    if trade is not None and trade in subscriptions:
        return result
    method = legacy_case(text(row, "payment_method", coalesce=True))
    provider = legacy_case(text(row, "payment_provider", coalesce=True))
    currency = legacy_case(text(row, "settlement_currency", coalesce=True), upper=True)
    # Missing micros fail closed; they cannot become a fabricated zero.
    amounts = {k: integer(row.get(k), "topup_amount_missing_or_invalid", -MAX_I64 - 1)
               for k in ("settled_amount_micros", "refunded_amount_micros", "expected_amount_micros", "credited_quota")}
    gross, refunded = amounts["settled_amount_micros"], amounts["refunded_amount_micros"]
    expected, credited = amounts["expected_amount_micros"], amounts["credited_quota"]
    ldc_aliases = {"ldc", "linuxdo", "linux_do", "linuxdo_credit"}
    legacy_ldc = provider == "epay" and (method in ldc_aliases or
        (method == "epay" and (currency != "CNY" or (expected <= 0 and gross <= 0))))
    virtual = currency == "LDC" and legacy_ldc
    excluded = {"gift", "bonus", "checkin", "invite", "bounty", "linuxdo", "linux_do", "linuxdo_credit", "internal", "admin"}
    financial = not (method == "balance" or provider == "balance" or
        (provider == "epay" and method in ldc_aliases) or method in excluded or provider in excluded)
    if not virtual and not financial:
        return result
    gateways = {"epay", "stripe", "creem", "waffo", "waffo_pancake"}
    known_external = not legacy_ldc and (provider in gateways or
        (provider == "" and method in {"stripe", "creem", "waffo", "waffo_pancake", "alipay", "wxpay"}))
    if ((virtual and not (expected > 0 and credited > 0 and currency != "")) or
            (not virtual and (currency == "LDC" or not known_external))):
        result["classification"] = "unconfirmed"
    elif gross < 0 or refunded < 0 or refunded > gross:
        result["classification"] = "invalid"
    elif re.fullmatch(r"[A-Z]{3}", currency) is None or gross == 0:
        result["classification"] = "unconfirmed"
    else:
        result.update(classification="virtual" if virtual else "cash", currency=currency,
                      gross_amount_micros=gross, refunded_amount_micros=refunded)
    return result


def build_plan(*, tables: dict[str, list[dict]], identity_records: list[dict], opening: dict,
               source_table_counts: dict[str, int]) -> FinancePlan:
    """Pure extraction. Production entry is prepare(): it verifies raw files first."""
    require({"users", "top_ups", "subscription_orders"} <= tables.keys(), "required_source_table_missing")
    for name in TABLES:
        require((name in tables) == (name in source_table_counts), "source_table_coverage_mismatch")
        if name in tables:
            require(type(source_table_counts[name]) is int and len(tables[name]) == source_table_counts[name],
                    "source_table_count_mismatch")
    identities = P.identity_index(identity_records)
    users = indexed(tables["users"])
    require(users.keys() == identities.keys(), "retained_user_coverage_mismatch")
    archive = F.checked_hash(opening.get("source_archive_sha256"))
    mapping = F.checked_hash(opening.get("mapping_sha256"))
    observed = integer(opening.get("observation", {}).get("observed_at"), "cutoff_invalid", 1, 253402300799)
    normalized = normalized_usage(tables)
    counters = indexed(opening["counters"], "user_id")
    require(counters.keys() == users.keys(), "opening_counter_coverage_mismatch")
    finance_users = []
    for uid, user in sorted(users.items()):
        counter = counters[uid]
        require(set(counter) == set(COUNTER_KEYS), "opening_counter_shape_invalid")
        expected = {"user_id": uid, "account_id": identities[uid], "source_archive_sha256": archive,
            "mapping_sha256": mapping, "source_entity_key": f"legacy.users/{uid}", "observed_at": observed,
            "source_row_sha256": F.digest(user), "used_quota": user["used_quota"],
            "request_count": user.get("request_count"), "aff_history": user.get("aff_history")}
        require(all(counter.get(k) == v and type(counter.get(k)) is type(v) for k, v in expected.items()),
                "opening_source_binding_mismatch")
        for key in ("used_quota", "request_count", "aff_history"):
            integer(counter[key], "opening_counter_invalid", -SAFE, SAFE)
        require(type(counter["migration_id"]) is str and bool(counter["migration_id"]), "opening_migration_id_missing")
        finance_users.append(FinanceUser(user_id=uid, account_id=identities[uid], source_row_sha256=F.digest(user),
            raw_used_quota=user["used_quota"], normalized_used_credits=normalized[uid]))
    subscription_trades = set()
    for order in tables["subscription_orders"]:
        require("trade_no" in order and (order["trade_no"] is None or type(order["trade_no"]) is str),
                "subscription_trade_missing")
        if order["trade_no"] is not None:
            subscription_trades.add(order["trade_no"])
    topups = []
    for _, row in sorted(indexed(tables["top_ups"]).items()):
        require("status" in row and (row["status"] is None or type(row["status"]) is str), "topup_status_missing_or_invalid")
        if row["status"] == "success":
            topups.append(classify_topup(row, subscription_trades))
    table_evidence = {name: {"count": len(tables[name]), "rows_sha256": F.digest(tables[name])}
                      for name in TABLES if name in tables}
    normalization = {"legacy_revision": REVISION, "algorithm": "usage-projection-user-v1",
        "native_without_audit": True, "tables": {name: table_evidence.get(name) for name in
            ("users", "wallet_credit_rebases", "tokens", "violation_fee_records")}}
    source = {"contract": CONTRACT, "source_archive_sha256": archive, "mapping_sha256": mapping,
        "tables_sha256": F.digest(table_evidence), "normalization_sha256": F.digest(normalization),
        "rows_sha256": F.digest({"users": finance_users, "topups": topups}), "observed_at": observed,
        "user_count": len(finance_users), "topup_count": len(topups), "credits_per_usd": 500000, "complete": True}
    source["source_key"] = canonical_sha(source)
    plan: FinancePlan = {"contract": CONTRACT, "source": source, "users": finance_users, "topups": topups,
        "normalization": normalization, "opening_counters": [counters[k] for k in sorted(counters)],
        "identity_import": opening["identity_import"], "plan_sha256": ""}
    plan["plan_sha256"] = canonical_sha({k: v for k, v in plan.items() if k != "plan_sha256"})
    validate_plan(plan)
    return plan


def validate_plan(plan: FinancePlan) -> None:
    require(plan.get("contract") == CONTRACT and plan.get("plan_sha256") ==
            canonical_sha({k: v for k, v in plan.items() if k != "plan_sha256"}), "plan_hash_mismatch")
    source = plan["source"]
    require(set(source) == set(FinanceSource.__annotations__), "source_material_fields_invalid")
    require(source["source_key"] == canonical_sha({k: v for k, v in source.items() if k != "source_key"}),
            "source_key_mismatch")
    require(source["rows_sha256"] == F.digest({"users": plan["users"], "topups": plan["topups"]})
            and source["normalization_sha256"] == F.digest(plan["normalization"])
            and source["user_count"] == len(plan["users"]) and source["topup_count"] == len(plan["topups"]),
            "source_rows_hash_mismatch")


def load_source(opening_path: Path, supplement: dict | None = None) -> tuple[dict, dict, list[dict], dict]:
    """Reuse frozen financial material; never connect to source or execute opening."""
    opening = P.parse_json(F.read_private(opening_path))
    F.verify_source_files(opening)
    files = opening["source_files"]
    load = lambda key: P.parse_json(F.read_private(Path(files[key]["path"])))
    receipt, proof = load("source_receipt"), load("source_export_proof")
    columns, counts, _, _, _ = P.metadata(receipt)
    require(receipt.get("status") == "snapshot-restored-baseline-verified-frozen-and-classified", "source_not_frozen")
    require(receipt["archive"]["sha256"] == opening["source_archive_sha256"] and
            I.sha256_file(I.private_backup_path(Path(receipt["archive"]["path"]))) == opening["source_archive_sha256"],
            "receipt_archive_mismatch")
    export_path = Path(files["source_export"]["path"])
    if supplement is not None:
        require(set(supplement) == {"export", "proof"}, "supplement_shape_invalid")
        for item in supplement.values():
            require(I.sha256_file(I.private_backup_path(Path(item["path"]))) == item["sha256"], "supplement_file_changed")
        export_path = Path(supplement["export"]["path"])
        proof = P.parse_json(F.read_private(Path(supplement["proof"]["path"])))
        require(proof["source_archive_sha256"] == opening["source_archive_sha256"] and
                proof["source_export_sha256"] == supplement["export"]["sha256"], "supplement_archive_mismatch")
        require(proof["source_instance"] == load("source_export_proof")["source_instance"], "supplement_instance_mismatch")
    require(proof.get("source_column_coverage_exact") is True and proof.get("source_row_baseline_exact") is True,
            "source_export_baseline_unproven")
    require(proof.get("metadata_hashes") == P.metadata(receipt)[2], "source_export_metadata_mismatch")
    selected = sorted(proof["selected_table_counts"])
    require(proof["selected_table_counts"] == {name: counts[name] for name in selected}, "export_counts_mismatch")
    require(all(name not in columns or name in selected or counts[name] == 0 for name in TABLES), "required_table_not_exported")
    tables = P.read_frozen_export(export_path, receipt, columns, counts,
                                  selected, proof["source_instance"])
    for name in TABLES:
        if name in columns and counts[name] == 0:
            tables.setdefault(name, [])
    for name in TABLES:
        evidence = opening["source_plan"].get("source_evidence", {}).get(name)
        if name in tables and evidence is not None:
            require(sorted(F.digest(row) for row in tables[name]) == sorted(evidence["row_hashes"]),
                    "opening_table_material_changed")
    records = [P.parse_json(line) for line in F.read_private(Path(files["identity_map"]["path"])).splitlines() if line]
    require(files["archive"]["sha256"] == opening["source_archive_sha256"], "archive_binding_mismatch")
    P.verify_plan_mapping(opening["source_plan"])
    require(opening["mapping_sha256"] == opening["source_plan"]["mapping_sha256"], "opening_mapping_mismatch")
    P.verify_target_identity_binding(opening["source_plan"], P.identity_index(records),
        import_run=opening["identity_import"], native_users=opening["native_users"])
    rebuilt = P.build_opening_payloads(opening["source_plan"], migration_id=opening["migration_id"],
        observation=opening["observation"], reservation_reconciliation=load("reservations"))
    require(P.build_opening_counter_payloads(opening["source_plan"], rebuilt)["payloads"] == opening["counters"],
            "opening_counters_material_mismatch")
    return tables, opening, records, counts


def projected_rows(plan: FinancePlan) -> dict[str, list[dict]]:
    return {"sources": [plan["source"]], "users": [{**r, "source_key": plan["source"]["source_key"]} for r in plan["users"]],
            "topups": [{**r, "source_key": plan["source"]["source_key"]} for r in plan["topups"]]}


def review_summary(plan: FinancePlan) -> dict:
    validate_plan(plan)
    counts = {kind: 0 for kind in ("cash", "virtual", "unconfirmed", "invalid", "excluded")}
    groups: dict[tuple[str, str], dict] = {}
    for row in plan["topups"]:
        kind = row["classification"]
        counts[kind] += 1
        if kind in ("cash", "virtual"):
            key = (kind, row["currency"])
            group = groups.setdefault(key, {"classification": kind, "currency": row["currency"],
                "orders": 0, "gross_amount_micros": 0, "refunded_amount_micros": 0})
            group["orders"] += 1
            group["gross_amount_micros"] += row["gross_amount_micros"]
            group["refunded_amount_micros"] += row["refunded_amount_micros"]
    totals = []
    for _, group in sorted(groups.items()):
        group["net_amount_micros"] = group["gross_amount_micros"] - group["refunded_amount_micros"]
        totals.append({key: str(value) if key.endswith("_micros") else value for key, value in group.items()})
    return {"source": plan["source"], "plan_sha256": plan["plan_sha256"], "classification_counts": counts,
            "raw_used_quota": str(sum(row["raw_used_quota"] for row in plan["users"])),
            "normalized_used_credits": str(sum(row["normalized_used_credits"] for row in plan["users"])),
            "recharge_totals": totals}


def reconcile(plan: FinancePlan, snapshot: dict) -> dict:
    """Compare independently read target identity/opening/projection against source."""
    validate_plan(plan)
    actual = snapshot["finance"]
    namespace = snapshot["source_namespace"]
    observed = {"source_count": len(actual["sources"]),
        "source_keys": [row.get("source_key") for row in actual["sources"]],
        "archive_sha256s": [row.get("source_archive_sha256") for row in actual["sources"]],
        "user_rows": len(actual["users"]), "topup_rows": len(actual["topups"])}
    require(F.digest(namespace) == F.digest(observed), "target_source_namespace_mismatch")
    require(namespace["source_count"] <= 1 and namespace["source_keys"] in ([], [plan["source"]["source_key"]]),
            "target_source_key_conflict")
    require(namespace["source_count"] != 0 or namespace["user_rows"] == namespace["topup_rows"] == 0,
            "target_source_incomplete")
    upper = integer(snapshot.get("native_intent_upper"), "target_native_intent_upper_invalid")
    maximum = integer(snapshot.get("native_intent_max_id"), "target_native_intent_max_id_invalid")
    sequence = snapshot["state"]["sequences"].get(INTENT_SEQUENCE, {})
    require(type(sequence.get("increment")) is int and sequence["increment"] == 1 and sequence.get("cycle") is False
            and upper >= maximum, "target_native_intent_sequence_invalid")
    require(type(sequence.get("is_called")) is bool, "target_native_intent_sequence_invalid")
    sequence_upper = integer(sequence.get("last_value"), "target_native_intent_sequence_invalid") if sequence["is_called"] else 0
    require(sequence_upper == upper, "target_native_intent_upper_mismatch")
    current_requests = request_ids(snapshot.get("native_request_ids"), "target_native_request_ids_invalid")
    request_count = snapshot["state"]["tables"].get("core_charging.requests", {}).get("rows")
    require(type(request_count) is int and request_count == len(current_requests), "target_native_request_count_mismatch")
    seals = snapshot.get("source_seals")
    require(type(seals) is list, "target_source_seals_missing")
    for seal in seals:
        require(type(seal) is dict and set(seal) == set(SourceSeal.__annotations__), "target_source_seal_invalid")
        integer(seal["native_intent_floor"], "target_source_floor_invalid", 0, upper)
        sealed_requests = request_ids(seal["native_request_ids"], "target_source_request_ids_invalid")
        require(set(sealed_requests) <= set(current_requests), "target_source_request_missing")
    require([seal["source_key"] for seal in seals] == namespace["source_keys"], "target_source_seals_mismatch")
    native = indexed(snapshot["native_users"])
    counters = indexed(snapshot["opening_counters"], "user_id")
    require(len(counters) == len(plan["opening_counters"]), "target_opening_coverage_mismatch")
    for counter in plan["opening_counters"]:
        uid = counter["user_id"]
        require(uid in native and native[uid]["personal_account_id"] == counter["account_id"], "target_identity_mismatch")
        require(F.digest(counters.get(uid)) == F.digest({**counter, "request": counter}), "target_opening_mismatch")
    require(any(F.digest(plan["identity_import"]) == F.digest(row) for row in snapshot["import_runs"]), "target_identity_import_mismatch")
    if F.digest(actual) == F.digest(projected_rows(plan)):
        status = "NOOP"
    else:
        require(actual == {"sources": [], "users": [], "topups": []}, "target_source_conflict")
        status = "READY"
    return {"status": status, "source_key": plan["source"]["source_key"], "plan_sha256": plan["plan_sha256"],
            "matched_user_count": len(plan["users"]), "target_writes": 0,
            "source_seals": seals, "native_intent_upper": upper, "native_request_ids": current_requests}


def finance_sql() -> str:
    return "SELECT jsonb_build_object(" + ",".join(
        F.literal(name) + ", (SELECT coalesce(jsonb_agg(" +
        ("to_jsonb(r)-'native_intent_floor'-'native_request_ids'" if name == "sources" else "to_jsonb(r)") +
        " ORDER BY " + order + "),'[]'::jsonb) FROM core_history.finance_" + name + " r)"
        for name, order in (("sources", "source_key"), ("users", "user_id"), ("topups", "source_key,source_order_id"))) + ")"


def catalog_contract() -> dict[str, str]:
    # Reuse the reviewed catalog query without rerunning the old fresh-opening
    # installer contract. Core's new schema/upgrade is owned by its installer.
    source = (I.ROOT / "services/core/src/core_schema.rs").read_text()
    queries = re.findall(r'const CATALOG:\s*&str\s*=\s*r#"(.*?)"#;', source, re.S)
    # This new importer follows the current frozen Core query. The old opening
    # helper's query constant intentionally remains at its reviewed predecessor.
    # tool_hashes binds the full Core source before approval and execution.
    require(len(queries) == 1, "catalog_source_fingerprint_mismatch")
    return {"catalog_query": queries[0]}


def target_sql(inventory: dict) -> str:
    return "SELECT jsonb_build_object('instance',(" + F.instance_sql() + "),'ownership_sha256',(" + F.ownership_sql() + ")," + \
        "'catalog_sha256',(" + I.catalog_hash_sql(catalog_contract()) + ")," + \
        "'state',(" + F.state_sql(inventory, "site-finance-observation-only") + ")," + \
        "'native_users',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'personal_account_id',personal_account_id) ORDER BY id),'[]'::jsonb) FROM core_identity.users)," + \
        "'opening_counters',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY user_id),'[]'::jsonb) FROM core_billing.account_opening_counters c)," + \
        "'import_runs',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY migration_id),'[]'::jsonb) FROM core_migration.import_runs r)," + \
        "'native_intent_upper'," + INTENT_UPPER_SQL + "," + \
        "'native_intent_max_id',(SELECT COALESCE(max(id),0) FROM core_payments.intents)," + \
        "'native_request_ids',to_jsonb(" + REQUEST_IDS_SQL + ")," + \
        "'source_seals',(SELECT coalesce(jsonb_agg(jsonb_build_object('source_key',source_key,'native_intent_floor',native_intent_floor,'native_request_ids',native_request_ids) ORDER BY source_key),'[]'::jsonb) FROM core_history.finance_sources)," + \
        "'source_namespace',jsonb_build_object(" + \
        "'source_count',(SELECT count(*) FROM core_history.finance_sources)," + \
        "'source_keys',(SELECT coalesce(jsonb_agg(source_key ORDER BY source_key),'[]'::jsonb) FROM core_history.finance_sources)," + \
        "'archive_sha256s',(SELECT coalesce(jsonb_agg(source_archive_sha256 ORDER BY source_key),'[]'::jsonb) FROM core_history.finance_sources)," + \
        "'user_rows',(SELECT count(*) FROM core_history.finance_users),'topup_rows',(SELECT count(*) FROM core_history.finance_topups))," + \
        "'finance',(" + finance_sql() + "))"


def protected_state(snapshot: dict) -> dict:
    state = snapshot["state"]
    return {**state, "tables": {k: v for k, v in state["tables"].items() if k not in DESTINATIONS}}


def _owner_guard(runtime_roles: list[str]) -> str:
    require(type(runtime_roles) is list and bool(runtime_roles) and len(set(runtime_roles)) == len(runtime_roles), "runtime_roles_missing")
    for role in runtime_roles:
        require(type(role) is str and I.IDENTIFIER.fullmatch(role), "runtime_role_invalid")
    return f"""
 IF inet_server_addr() IS NOT NULL OR current_setting('listen_addresses')<>'' THEN
  RAISE EXCEPTION 'site_finance_offline_socket_required'; END IF;
 IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin AND NOT rolsuper
    AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls)
 OR NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=session_user AND rolcanlogin AND NOT rolsuper
    AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls)
 OR NOT pg_has_role(session_user,current_user,'MEMBER') THEN RAISE EXCEPTION 'site_finance_offline_owner_required'; END IF;
 IF EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='core_history' AND pg_get_userbyid(nspowner)<>current_user)
 OR EXISTS(SELECT 1 FROM pg_class WHERE oid IN ({','.join(F.literal(n)+'::regclass' for n in DESTINATIONS)})
   AND pg_get_userbyid(relowner)<>current_user) THEN RAISE EXCEPTION 'site_finance_owner_mismatch'; END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
   AND pid IS NOT NULL AND pid<>pg_backend_pid()) THEN RAISE EXCEPTION 'site_finance_other_clients'; END IF;
 IF EXISTS(SELECT 1 FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
   AND has_database_privilege(oid,current_database(),'CONNECT')) THEN RAISE EXCEPTION 'site_finance_offline_fence_missing'; END IF;
 IF (SELECT count(*) FROM pg_roles WHERE rolname IN ({','.join(F.literal(r) for r in runtime_roles)})
   AND NOT rolcanlogin AND NOT rolsuper AND NOT rolbypassrls)<>{len(runtime_roles)}
 OR EXISTS(SELECT 1 FROM pg_roles r CROSS JOIN unnest(ARRAY[{','.join(F.literal(n) for n in DESTINATIONS)}]) AS t(name)
   WHERE r.rolname IN ({','.join(F.literal(r) for r in runtime_roles)}) AND
    (has_table_privilege(r.oid,t.name,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')
     OR has_any_column_privilege(r.oid,t.name,'INSERT,UPDATE,REFERENCES')))
 THEN RAISE EXCEPTION 'site_finance_runtime_writes_forbidden'; END IF;
 IF NOT has_sequence_privilege(current_user,'{INTENT_SEQUENCE}'::regclass,'SELECT') THEN
  RAISE EXCEPTION 'site_finance_native_intent_sequence_access_denied'; END IF;
 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_sequence WHERE seqrelid='{INTENT_SEQUENCE}'::regclass
   AND seqincrement=1 AND NOT seqcycle)
 OR {INTENT_UPPER_SQL} < (SELECT COALESCE(max(id),0) FROM core_payments.intents)
 OR {INTENT_UPPER_SQL} < 0
 OR EXISTS(SELECT 1 FROM core_history.finance_sources WHERE native_intent_floor > {INTENT_UPPER_SQL})
 THEN RAISE EXCEPTION 'site_finance_native_intent_sequence_invalid'; END IF;
 IF EXISTS(SELECT 1 FROM core_history.finance_sources s
   CROSS JOIN LATERAL unnest(s.native_request_ids) sealed(request_id)
   LEFT JOIN core_charging.requests r ON r.id=sealed.request_id WHERE r.id IS NULL)
 THEN RAISE EXCEPTION 'site_finance_source_request_missing'; END IF;
"""


def snapshot_sql(inventory: dict, runtime_roles: list[str]) -> str:
    return "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;\n" + \
        "DO $site_finance$ BEGIN " + _owner_guard(runtime_roles) + " END $site_finance$;\n" + target_sql(inventory) + ";\nCOMMIT;\n"


def build_apply_sql(plan: FinancePlan, snapshot: dict, runtime_roles: list[str]) -> str:
    reconciliation = reconcile(plan, snapshot)
    inventory = snapshot["state"]["inventory"]
    expected = projected_rows(plan)
    bound = {k: snapshot[k] for k in ("instance", "ownership_sha256", "catalog_sha256", "native_users", "opening_counters", "import_runs",
                                     "native_intent_upper", "native_intent_max_id", "native_request_ids")}
    expected_seals = snapshot["source_seals"] if reconciliation["status"] == "NOOP" else [
        {"source_key": plan["source"]["source_key"], "native_intent_floor": snapshot["native_intent_upper"],
         "native_request_ids": snapshot["native_request_ids"]}]
    state = protected_state(snapshot)
    updates = []
    for name in ("users", "topups", "sources"):
        table = "core_history.finance_" + name
        fields = list({"users": FinanceUser, "topups": FinanceTopup, "sources": FinanceSource}[name].__annotations__)
        if name != "sources":
            fields.append("source_key")
        selected = ['r.' + field for field in fields]
        if name == "sources":
            fields.append("native_request_ids")
            selected.append(REQUEST_IDS_SQL)
        updates.append(f"INSERT INTO {table} ({','.join(fields)}) SELECT {','.join(selected)} "
                       f"FROM jsonb_populate_recordset(NULL::{table},{F.literal(expected[name])}::jsonb) AS r;")
    body = f"""
DECLARE actual jsonb; before_protected jsonb; after_protected jsonb; replay boolean;
BEGIN
{_owner_guard(runtime_roles)}
 actual := ({target_sql(inventory)});
 IF (actual->'source_namespace'->>'source_count')::bigint > 1 OR
  (actual->'source_namespace'->'source_keys' <> '[]'::jsonb AND
   actual->'source_namespace'->'source_keys' <> {F.literal([plan['source']['source_key']])}::jsonb) THEN
  RAISE EXCEPTION 'site_finance_source_key_conflict'; END IF;
 IF (actual - 'state' - 'finance' - 'source_namespace' - 'source_seals') IS DISTINCT FROM {F.literal(bound)}::jsonb THEN
  RAISE EXCEPTION 'site_finance_target_binding_changed'; END IF;
 before_protected := jsonb_set(actual->'state','{{tables}}',(actual->'state'->'tables') - ARRAY[{','.join(F.literal(n) for n in DESTINATIONS)}]);
 IF before_protected IS DISTINCT FROM {F.literal(state)}::jsonb THEN RAISE EXCEPTION 'site_finance_target_state_changed'; END IF;
 replay := actual->'finance' = {F.literal(expected)}::jsonb;
 IF replay AND actual->'source_seals' IS DISTINCT FROM {F.literal(expected_seals)}::jsonb THEN
  RAISE EXCEPTION 'site_finance_source_seal_mismatch'; END IF;
 IF NOT replay THEN
  IF actual->'finance' IS DISTINCT FROM '{{"sources":[],"users":[],"topups":[]}}'::jsonb THEN
   RAISE EXCEPTION 'site_finance_source_conflict'; END IF;
  {chr(10).join(updates)}
 END IF;
 IF ({finance_sql()}) IS DISTINCT FROM {F.literal(expected)}::jsonb THEN RAISE EXCEPTION 'site_finance_readback_mismatch'; END IF;
 actual := ({target_sql(inventory)});
 IF actual->'source_seals' IS DISTINCT FROM {F.literal(expected_seals)}::jsonb THEN
  RAISE EXCEPTION 'site_finance_source_seal_mismatch'; END IF;
 after_protected := jsonb_set(actual->'state','{{tables}}',(actual->'state'->'tables') - ARRAY[{','.join(F.literal(n) for n in DESTINATIONS)}]);
 IF after_protected IS DISTINCT FROM before_protected OR (actual - 'state' - 'finance' - 'source_namespace' - 'source_seals') IS DISTINCT FROM {F.literal(bound)}::jsonb
 THEN RAISE EXCEPTION 'site_finance_unrelated_state_changed'; END IF;
 PERFORM set_config('lmm.site_finance_result',CASE WHEN replay THEN 'NOOP' ELSE 'COMMITTED' END,true);
END
"""
    tag = "$site_finance_" + F.sha(body.encode())[:24] + "$"
    require(tag not in body, "sql_delimiter_collision")
    return f"""SET standard_conforming_strings=on;
SET timezone='UTC'; SET lock_timeout='5s'; SET statement_timeout='120s';
DO $site_finance_lock$ BEGIN IF NOT pg_try_advisory_lock({F.FENCE}) THEN RAISE EXCEPTION 'site_finance_busy'; END IF; END $site_finance_lock$;
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on; SET LOCAL row_security=off; SET LOCAL search_path=pg_catalog,public;
LOCK TABLE core_identity.users,core_billing.account_opening_counters,core_migration.import_runs,core_payments.intents,core_charging.requests IN SHARE MODE;
LOCK TABLE {','.join(DESTINATIONS)} IN EXCLUSIVE MODE;
DO {tag}{body}{tag};
SET CONSTRAINTS ALL IMMEDIATE;
SELECT jsonb_build_object('status',current_setting('lmm.site_finance_result'),'source_key',{F.literal(plan['source']['source_key'])},'plan_sha256',{F.literal(plan['plan_sha256'])});
COMMIT;
"""


def tool_hashes() -> dict[str, str]:
    return {**F.source_hashes(escrow=True), "site_statistics_writer": I.sha256_file(Path(__file__)),
            **{name: I.sha256_file(I.ROOT / name) for name in
               ("services/core/src/core_schema.rs", "services/core/schema/site_statistics.sql")}}


def prepare(opening_path: Path, snapshot: dict, runtime_roles: list[str], supplement: dict | None = None) -> dict:
    tables, opening, identities, counts = load_source(opening_path, supplement)
    plan = build_plan(tables=tables, identity_records=identities, opening=opening, source_table_counts=counts)
    reconciliation = reconcile(plan, snapshot)
    return {"plan": plan, "snapshot": snapshot, "runtime_roles": runtime_roles,
            "opening_material": {"path": str(opening_path), "sha256": I.sha256_file(opening_path)},
            "tool_hashes": tool_hashes(), "reconciliation": reconciliation, "supplement": supplement}


def verify_inputs(material: dict) -> FinancePlan:
    ref = material["opening_material"]
    path = I.private_backup_path(Path(ref["path"]))
    require(I.sha256_file(path) == ref["sha256"], "opening_material_changed")
    require(tool_hashes() == material["tool_hashes"], "tool_source_changed")
    rebuilt = prepare(path, material["snapshot"], material["runtime_roles"], material["supplement"])
    require(rebuilt == material, "site_finance_material_changed")
    return rebuilt["plan"]


def apply_offline(material: dict, *, approval: dict, connection: dict, output: Path) -> dict:
    """One explicit owner attempt. Any uncertainty after submission stays UNKNOWN."""
    I.private_backup_path(output, directory=True)
    plan = verify_inputs(material)
    sql = build_apply_sql(plan, material["snapshot"], material["runtime_roles"])
    require(approval == {"decision": "approve-offline-site-finance", "material_sha256": canonical_sha(material),
                        "sql_sha256": F.sha(sql.encode()), "source_key": plan["source"]["source_key"]}, "owner_approval_mismatch")
    owner, login = I.connection_roles(connection)
    instance = material["snapshot"]["instance"]
    require(owner == instance["role"] and login == instance["session_user"]
            and connection.get("database") == instance["database"] and connection.get("port") == instance["port"],
            "target_connection_binding_mismatch")
    I.socket_connection(connection)
    application_name = "lmm-site-finance-" + uuid.uuid4().hex
    F.write_private(output, "attempt.private.json", F.canonical({"application_name": application_name,
        "material_sha256": canonical_sha(material), "source_key": plan["source"]["source_key"]}) + "\n")
    F.write_private(output, "executed.private.sql", sql)
    result = {"status": "UNKNOWN", "source_key": plan["source"]["source_key"], "plan_sha256": plan["plan_sha256"]}
    try:
        code, stdout = F.run_sql(connection, sql, output, application_name, label="site-finance")
        if code == 0:
            verify_inputs(material)
            receipt = P.parse_json(stdout)
            require(receipt in [{**result, "status": status} for status in ("COMMITTED", "NOOP")], "receipt_mismatch")
            result = receipt
    except Exception:
        pass  # Private transport evidence is retained by run_sql; never claim rollback.
    try:
        F.write_private(output, "result.private.json", F.canonical(result) + "\n")
    except Exception:
        result = {**result, "status": "UNKNOWN"}
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("prepare", "snapshot", "apply"), required=True)
    parser.add_argument("--opening-material", type=Path)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--site-export", type=Path)
    parser.add_argument("--site-export-proof", type=Path)
    parser.add_argument("--runtime-role", action="append")
    parser.add_argument("--material", type=Path)
    parser.add_argument("--approval", type=Path)
    parser.add_argument("--connection", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    load = lambda path: P.parse_json(F.read_private(path))
    try:
        I.private_backup_path(args.output, directory=True)
        if args.mode == "prepare":
            require(args.opening_material and args.snapshot and args.runtime_role, "prepare_arguments_missing")
            require(bool(args.site_export) == bool(args.site_export_proof), "supplement_pair_required")
            supplement = {name: {"path": str(I.private_backup_path(path)), "sha256": I.sha256_file(path)}
                for name, path in (("export", args.site_export), ("proof", args.site_export_proof))} if args.site_export else None
            material = prepare(args.opening_material, load(args.snapshot), args.runtime_role, supplement)
            F.write_private(args.output, "material.private.json", F.canonical(material) + "\n")
            sql = build_apply_sql(material["plan"], material["snapshot"], args.runtime_role)
            F.write_private(args.output, "review.private.sql", sql)
            F.write_private(args.output, "review.private.json", F.canonical(review_summary(material["plan"])) + "\n")
            F.write_private(args.output, "approval-request.private.json", F.canonical({"decision": "REVIEW_REQUIRED",
                "material_sha256": canonical_sha(material), "sql_sha256": F.sha(sql.encode()),
                "source_key": material["plan"]["source"]["source_key"]}) + "\n")
            print(F.canonical(material["reconciliation"]))
        elif args.mode == "snapshot":
            require(args.connection and args.runtime_role, "snapshot_arguments_missing")
            connection = load(args.connection)
            code, stdout = F.run_sql(connection, "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n" + F.inventory_sql() + ";\nCOMMIT;",
                                     args.output, "lmm-site-finance-inventory", readonly=True, label="inventory")
            require(code == 0, "inventory_failed")
            inventory = P.parse_json(stdout)
            require(set(DESTINATIONS) <= set(inventory["tables"]), "site_finance_schema_missing")
            code, stdout = F.run_sql(connection, snapshot_sql(inventory, args.runtime_role), args.output,
                                     "lmm-site-finance-snapshot", readonly=True, label="snapshot")
            require(code == 0, "snapshot_failed")
            F.write_private(args.output, "target.private.json", F.canonical(P.parse_json(stdout)) + "\n")
            print('{"status":"READ_ONLY_SNAPSHOT","target_writes":0}')
        else:
            require(args.material and args.approval and args.connection, "apply_arguments_missing")
            result = apply_offline(load(args.material), approval=load(args.approval), connection=load(args.connection), output=args.output)
            print(F.canonical(result))
            return 0 if result["status"] in ("COMMITTED", "NOOP") else 2
    except (Error, OSError, ValueError, KeyError, TypeError):
        # Never print a source row, path, connection or exception containing data.
        print('{"status":"REFUSED","reason":"invalid_or_conflicting_site_finance_material"}')
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
