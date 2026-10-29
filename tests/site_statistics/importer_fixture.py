"""Author-only synthetic legacy data, not the independent reader oracle."""
import hashlib
import json


def old_tables(observed: int) -> dict:
    users = [{"id": uid, "quota": balance, "aff_quota": 0, "used_quota": used,
        "aff_history": 9, "request_count": 11, "status": 2 if uid == 12 else 1,
        "deleted_at": "2026-01-01" if uid == 12 else None,
        "password": "SYNTHETIC-NOT-A-REAL-PASSWORD"} for uid, balance, used in ((11, 37, 920), (12, -3, 300), (13, 0, 0))]
    def topup(oid, **changes):
        return {"id": oid, "user_id": 11, "credited_quota": 500, "refunded_quota": 0,
            "pending_credit_rebase_original_quota": 0, "pending_credit_rebase_effective_quota": 0,
            "expected_amount_micros": 9000000, "settled_amount_micros": 1200000,
            "refunded_amount_micros": 200000, "platform_amount_micros": 0,
            "money": 9999999, "amount": 23, "settlement_currency": "USD",
            "payment_method": "stripe", "payment_provider": "stripe", "provider_event_id": None,
            "provider_transaction_id": "", "trade_no": f"synthetic-{oid}", "status": "success",
            "create_time": observed - 100, "complete_time": observed - 50, "pending_credit_rebase_key": "", **changes}
    topups = [topup(1), topup(2, payment_method="ldc", payment_provider="epay", settlement_currency="LDC",
                              settled_amount_micros=700000, refunded_amount_micros=100000),
        topup(3, payment_method="gift"), topup(4, trade_no="subscription-mirror"),
        topup(5, payment_provider="unknown"), topup(6, refunded_amount_micros=1200001),
        topup(7, settled_amount_micros=0, refunded_amount_micros=0), topup(8, status="pending")]
    source_fee = {"id": 9, "user_id": 11, "charged_quota": 100, "status": "charged", "created_at": observed - 30,
                  "reversed_at": 0, "reversed_by": 0, "error_code": "abuse.synthetic"}
    plan = {"version": 1, "kind": "offline_credit_balance_rebase_preview", "migration_id": "synthetic-rebase",
        "user_ids": [11], "has_complete_history": True, "usd_credit_conversion": 500000, "divisor": "2",
        "rounding": "half-away-from-zero", "snapshot_at": observed - 20,
        "user_sources": [{"id": 11, "used_quota": 1000}], "token_sources": [],
        "other_credit_bases": [{"kind": "violation_fee_refund", "source_id": "9", "user_id": 11,
            "original_quota": 100, "rebased_quota": 50, "source": source_fee}]}
    raw_plan = json.dumps(plan, sort_keys=True, separators=(",", ":"))
    return {"users": users, "top_ups": topups, "tokens": [], "user_subscriptions": [], "subscription_plans": [],
        "options": [], "subscription_orders": [{"id": 1, "trade_no": "subscription-mirror", "status": "failed"}],
        "wallet_credit_rebases": [{"id": 1, "migration_id": "synthetic-rebase", "plan": raw_plan,
                                   "plan_sha256": hashlib.sha256(raw_plan.encode()).hexdigest()}],
        "violation_fee_records": [{**source_fee, "status": "reversed", "reversed_at": observed - 10, "reversed_by": 13}]}


def assert_review(review: dict) -> None:
    # Hand calculated: (1000 / 2 - 100 / 2) + (920 - 1000 + 100) = 470.
    assert review["raw_used_quota"] == "1220"
    assert review["normalized_used_credits"] == "770"
    assert review["source"]["user_count"] == 3
    assert review["source"]["topup_count"] == 7
    assert review["classification_counts"] == {"cash": 1, "virtual": 1, "excluded": 2, "unconfirmed": 2, "invalid": 1}
    assert [(r["classification"], r["currency"], r["gross_amount_micros"], r["refunded_amount_micros"], r["net_amount_micros"])
            for r in review["recharge_totals"]] == [
                ("cash", "USD", "1200000", "200000", "1000000"),
                ("virtual", "LDC", "700000", "100000", "600000")]
