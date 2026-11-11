"""Money-adjacent cutover phases: subscriptions, promotions, bounties, wallet.

Registered by legacy_cutover_import.py. Each phase reads the legacy snapshot
read-only and emits one SERIALIZABLE owner transaction. Legacy balances were
already moved by the opening phase; they must not be charged a second time:
historical rights are imported as receipts, frozen holds or redeemable codes.
Pending wallet transfers restore the already-debited liability and reserve it
atomically, leaving the imported available balance unchanged.

Run order: subscriptions, promotions, bounties, wallet. Bounty escrow requires
the payer to have no non-opening journal, so it must precede any phase that
posts holds (wallet pending transfers).
"""
from __future__ import annotations

import hashlib
import json
import re
from collections import Counter, defaultdict
from decimal import ROUND_HALF_UP, Decimal

from legacy_sql import literal as lit
from legacy_cutover_keys import personal_account



def fail(code: str) -> None:
    raise SystemExit(f"error: {code}")


def digest(value) -> str:
    # Same canonical form as legacy_cutover_import.digest (opening mapping binding).
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True, default=str,
                                     separators=(",", ":")).encode()).hexdigest()

TOPUP_PAYABLE_SQL = "(status='pending' OR (status='failed' AND payment_provider='waffo_pancake' AND failure_reason_code='checkout_timeout'))"


def topup_can_settle(row):
    return row is not None and (row.get("status") == "pending" or (
        row.get("status") == "failed" and row.get("payment_provider") == "waffo_pancake"
        and row.get("failure_reason_code") == "checkout_timeout"))


def discount_capacity_holds(data):
    """Mirror original capacity rules; reservation TTL never proves closure."""
    reservations = data["discount_code_reservations"]
    if not data["discount_codes"] and not reservations:
        return Counter()
    if "discount_top_ups" not in data:
        fail("discount_payment_lifecycle_missing")
    orders = {}
    for row in data["discount_top_ups"]:
        trade = row["trade_no"]
        if not isinstance(trade, str) or not trade or trade in orders:
            fail("discount_topup_reference_invalid_or_duplicate")
        orders[trade] = row
    held = Counter()
    reserved_trades = set()
    for row in reservations:
        trade = row["top_up_trade_no"]
        if trade in reserved_trades:
            fail("discount_reservation_reference_duplicate")
        reserved_trades.add(trade)
        order = orders.get(trade)
        # Match SQL's nullable failure_reason_code: unknown is not proof of closure.
        closed = order is not None and (order.get("status") == "expired" or (
            order.get("status") == "failed" and (
                (order.get("payment_provider") or "") != "waffo_pancake" or (
                    order.get("failure_reason_code") is not None
                    and order["failure_reason_code"] != "checkout_timeout"))))
        if row["status"] == "reserved" and not closed:
            held[row["discount_code_id"]] += 1
        elif row["status"] == "released" and topup_can_settle(order):
            held[row["discount_code_id"]] += 1
        elif row["status"] not in ("reserved", "released", "consumed"):
            fail("discount_reservation_status_unknown")
    for trade, order in orders.items():
        if order["discount_code_id"] > 0 and trade not in reserved_trades and topup_can_settle(order):
            held[order["discount_code_id"]] += 1
    return held


PHASES = ("subscriptions", "promotions", "bounties", "wallet")
GUARDS = {
    "subscriptions": "SELECT (SELECT count(*) FROM core_subscriptions.plans)"
                     "+(SELECT count(*) FROM core_charging.subscriptions WHERE grant_reference LIKE 'legacy-sub:%')"
                     "+(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE source_table='subscription_orders')",
    "promotions": "SELECT count(*) FROM core_promotions.campaigns",
    "bounties": "SELECT (SELECT count(*) FROM core_billing.legacy_escrow_batch_runs)"
                "+(SELECT count(*) FROM core_bounties.policy)",
    "wallet": "SELECT (SELECT count(*) FROM core_wallet.transfers)"
              "+(SELECT count(*) FROM core_wallet.legacy_transfer_history)"
              "+(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE source_table='top_ups')",
}
SUBSCRIPTION_PLANS_PENDING_GUARD = (
    "SELECT (SELECT count(*) FROM core_subscriptions.plans)"
    "+(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE source_table='subscription_orders')"
)
CREDITS_PER_USD = 500000
MAX_TIME = 253402300799
MAX_UNITS = 1_000_000_000_000
DAY = 86400
PERIODS = {"never": "grant", "daily": "day", "weekly": "week", "monthly": "month", "custom": "custom"}
DURATION = {"hour": 3600, "day": DAY, "week": 7 * DAY}


def jlit(value) -> str:
    return lit(json.dumps(value, ensure_ascii=False, sort_keys=True, default=str, separators=(",", ":"))) + "::jsonb"


def sha_hex(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def load_source(src, schema: str) -> dict:
    def q(sql: str) -> list[dict]:
        out = src.run(f"SET search_path={schema};SELECT COALESCE(json_agg(t),'[]') FROM ({sql}) t")
        return json.loads(out, parse_float=Decimal)
    return {
        "subscription_plans": q("SELECT * FROM subscription_plans ORDER BY id"),
        "user_subscriptions": q("SELECT * FROM user_subscriptions ORDER BY id"),
        "subscription_orders": q("SELECT * FROM subscription_orders ORDER BY id"),
        "redemptions": q("SELECT id,user_id,key,status,name,quota,created_time,redeemed_time,used_user_id,"
                         "deleted_at,expired_time,reward_type,reset_plan_id FROM redemptions ORDER BY id"),
        "checkin_recent": q("SELECT count(*) AS n, COALESCE(sum(quota_awarded),0) AS total FROM checkins "
                            "WHERE created_at >= (SELECT max(created_at) FROM checkins) - 30*86400"),
        "checkin_total": q("SELECT count(*) AS n, count(DISTINCT user_id) AS users FROM checkins"),
        "bounty_projects": q("SELECT * FROM open_source_bounty_projects ORDER BY id"),
        "bounty_challenges": q("SELECT * FROM open_source_bounty_challenges ORDER BY id"),
        "bounty_disputes": q("SELECT * FROM open_source_bounty_disputes ORDER BY id"),
        "wallet_transfers": q("SELECT * FROM wallet_transfers ORDER BY id"),
        "top_ups_payable": q("SELECT * FROM top_ups WHERE " + TOPUP_PAYABLE_SQL + " ORDER BY id"),
        "discount_top_ups": q("SELECT trade_no,discount_code_id,status,payment_provider,failure_reason_code FROM top_ups "
                              "WHERE discount_code_id>0 OR trade_no IN (SELECT top_up_trade_no FROM discount_code_reservations)"),
        "discount_codes": q("SELECT id,status,used_count,max_uses,starts_time,expired_time,deleted_at,owner_user_id FROM discount_codes"),
        "discount_code_reservations": q("SELECT id,status,discount_code_id,expires_time,top_up_trade_no FROM discount_code_reservations"),
        "red_packets": q("SELECT id,enabled,end_at,deleted_at FROM red_packets"),
        "red_packet_items": q("SELECT id,packet_id,item_type,claimed_by FROM red_packet_items"),
        "red_packet_claims": q("SELECT count(*) AS n FROM red_packet_claims"),
        "referral_rewards": q("SELECT status,stage,count(*) AS n FROM referral_rewards GROUP BY 1,2"),
        "other_counts": q(
            "SELECT 'subscription_pre_consume_inflight' AS k, count(*) AS n FROM subscription_pre_consume_records "
            "WHERE status NOT IN ('settled','refunded') UNION ALL "
            "SELECT 'subscription_reset_vouchers', count(*) FROM subscription_reset_vouchers UNION ALL "
            "SELECT 'bounty_ledgers', count(*) FROM open_source_bounty_ledgers"),
    }


def superadmin(data: dict) -> int:
    ids = [u["id"] for u in data["users"] if u["role"] == 100 and u["deleted_at"] is None]
    if not ids:
        fail("superadmin_missing")
    return min(ids)


def opening_mapping(data: dict) -> str:
    return opening_run(data)["mapping_sha256"]


def opening_run(data: dict) -> dict:
    run = data.get("opening_binding", {}).get("run")
    if not isinstance(run, dict):
        fail("committed_opening_binding_missing")
    return run


def bind_opening(data: dict, target, archive_sha: str, observed_at: int) -> None:
    """Use the actual formal opening run, including its financial plan hash."""
    actual = json.loads(target.run("SELECT json_build_object('runs',"
        "(SELECT coalesce(json_agg(to_jsonb(r)),'[]') FROM core_billing.opening_import_runs r),"
        "'imports',(SELECT coalesce(json_agg(json_build_object('migration_id',migration_id,"
        "'account_id',account_id)),'[]') FROM core_billing.opening_imports),"
        "'counters',(SELECT coalesce(json_agg(json_build_object('migration_id',migration_id,"
        "'user_id',user_id,'account_id',account_id)),'[]') FROM core_billing.account_opening_counters))"))
    if len(actual["runs"]) != 1:
        fail("committed_opening_run_missing_or_ambiguous")
    run = actual["runs"][0]
    if run["source_archive_sha256"] != archive_sha or run["observed_at"] != observed_at:
        fail("committed_opening_source_mismatch")
    accounts = data["identity_binding"]["accounts"]
    if (len(actual["imports"]) != len(accounts)
            or {row["account_id"] for row in actual["imports"]} != set(accounts.values())
            or len(actual["counters"]) != len(accounts)
            or {row["user_id"]: row["account_id"] for row in actual["counters"]} != accounts
            or any(row["migration_id"] != run["migration_id"]
                   for row in actual["imports"] + actual["counters"])):
        fail("committed_opening_account_map_mismatch")
    data["opening_binding"] = {"run": run}


def opening_guard_sql(data: dict) -> str:
    run = opening_run(data)
    rows = [{"user_id": uid, "account_id": aid}
            for uid, aid in sorted(data["identity_binding"]["accounts"].items())]
    return f"""DO $opening_binding$
BEGIN
 LOCK TABLE core_billing.opening_import_runs,core_billing.opening_imports,
  core_billing.account_opening_counters IN SHARE MODE;
 IF (SELECT count(*) FROM core_billing.opening_import_runs)<>1
 OR NOT EXISTS(SELECT 1 FROM core_billing.opening_import_runs r WHERE to_jsonb(r)={jlit(run)})
 OR (SELECT count(*) FROM core_billing.opening_imports)<>{len(rows)}
 OR (SELECT count(*) FROM core_billing.account_opening_counters)<>{len(rows)}
 OR EXISTS(SELECT 1 FROM jsonb_to_recordset({jlit(rows)}) e(user_id BIGINT,account_id BIGINT)
  WHERE NOT EXISTS(SELECT 1 FROM core_billing.opening_imports i
   WHERE i.account_id=e.account_id AND i.migration_id={lit(run['migration_id'])})
  OR NOT EXISTS(SELECT 1 FROM core_billing.account_opening_counters c WHERE c.user_id=e.user_id
   AND c.account_id=e.account_id AND c.migration_id={lit(run['migration_id'])})) THEN
  RAISE EXCEPTION 'committed_opening_binding_changed';
 END IF;
END $opening_binding$;"""


def usd_units(amount: Decimal, currency: str, options: dict) -> int:
    if currency == "USD":
        value = amount * CREDITS_PER_USD
    elif currency == "CNY":
        value = amount / Decimal(options["USDExchangeRate"]) * CREDITS_PER_USD
    else:
        fail("plan_currency_unsupported")
    return int(value.quantize(Decimal(1), rounding=ROUND_HALF_UP))


def pending_liability(data, table, row, *, archive_sha, migration_id, observed_at, amount_fields, currency_fields):
    uid = row["user_id"]
    return "SELECT core_billing.import_legacy_pending_liability(" + jlit({
        "migration_id": opening_run(data)["migration_id"], "source_archive_sha256": archive_sha,
        "mapping_sha256": opening_mapping(data), "source_table": table, "source_row_key": str(row["id"]),
        "source_row_sha256": digest(row), "source_parent_plan_sha256": None,
        "owner_user_id": uid, "owner_account_id": personal_account(data, uid),
        "status": row["status"], "settlement_state": "unknown",
        "payload_json": row, "amount_raw_json": {k: row.get(k) for k in amount_fields},
        "currency_raw_json": {k: row.get(k) for k in currency_fields},
        "spendable": False, "spendable_units": 0, "observed_at": observed_at,
        "request": {"importer": "legacy_cutover_money", "source_status": row["status"]},
        "result": {"provider_status": "unobserved"}}) + ");"


# ---------------------------------------------------------------- subscriptions
def plan_terms(p: dict, options: dict) -> tuple[dict, bool]:
    unit = p["duration_unit"]
    months = p["duration_value"] * (12 if unit == "year" else 1) if unit in ("month", "year") else 0
    if unit == "custom":
        duration = p["custom_seconds"]
    elif months:
        duration = 0
    elif unit in DURATION:
        duration = DURATION[unit] * p["duration_value"]
    else:
        fail("plan_duration_unit_unsupported")
    period = PERIODS.get(p["quota_reset_period"]) or fail("plan_reset_period_unsupported")
    title = " ".join((p["title"] or "").split())[:160] or f"Plan {p['id']}"
    terms = {
        "title": title,
        "price_units": str(usd_units(Decimal(p["price_amount"]), p["currency"], options)),
        "duration_seconds": duration,
        "duration_months": months,
        "reset_period": period,
        "reset_seconds": p["quota_reset_custom_seconds"] if period == "custom" else 0,
        "limit_units": str(p["total_amount"]),
        "models": ["*"], "groups": ["*"],
        "max_purchases_per_user": p["max_purchase_per_user"],
        "enabled": bool(p["enabled"]) and not p["archived_at"],
    }
    if not (1 <= int(terms["price_units"]) <= MAX_UNITS and 1 <= p["total_amount"] <= MAX_UNITS
            and ((months == 0 and 60 <= duration <= 315_576_000)
                 or (1 <= months <= 120 and duration == 0))
            and 0 <= terms["max_purchases_per_user"] <= 10_000
            and (period != "custom" or 1 <= terms["reset_seconds"] <= 315_576_000)):
        fail("plan_terms_invalid")
    return terms, bool(p["archived_at"])


def subscription_record(s: dict, plan: dict, observed_at: int, migration_id: str, *, account_id: int) -> dict:
    period = PERIODS.get(plan["quota_reset_period"]) or fail("subscription_period_unsupported")
    # allow_wallet_overflow maps to the account billing preference in Core, not the grant.
    if any(s.get(k) for k in ("upgrade_group", "prev_user_group", "downgrade_group")):
        fail("subscription_group_transition_unrepresentable")
    start, end, total, used = s["start_time"], s["end_time"], s["amount_total"], s["amount_used"]
    if not (0 <= start < end <= MAX_TIME and total >= 0 and used >= 0):
        fail("subscription_window_invalid")
    unlimited = not (total > 0 or s["reset_amount"] is not None or s["renewal_amount"] is not None)
    seconds, reset_basis, reset_limit, reset_unlimited = 0, "same", 0, False
    if period == "grant":
        o_start, o_end = start, end
    else:
        o_start = max(start, s["last_reset_time"] or 0)
        o_end = min(end, s["next_reset_time"] or end)
        if not start <= o_start < o_end <= end:
            fail("subscription_opening_window_invalid")
        reset_unlimited = unlimited and s["reset_amount"] is None
        reset_limit = 0 if reset_unlimited else (s["reset_amount"] if s["reset_amount"] is not None else fail("reset_basis_unproven"))
        reset_basis = "explicit"
        if period == "custom":
            seconds = plan["quota_reset_custom_seconds"]
    return {"source_id": s["id"], "account_id": account_id, "period": period, "anchor": start,
            "seconds": seconds, "ends_at": end, "limit_credits": 0 if unlimited else total,
            "unlimited": unlimited, "reset_basis": reset_basis, "reset_limit_credits": reset_limit,
            "reset_unlimited": reset_unlimited,
            "active": s["status"] == "active" and end > observed_at,
            "opening_start": o_start, "opening_end": o_end, "opening_used": used,
            "remaining": None if unlimited else total - used,
            "grant_reference": f"legacy-sub:{migration_id}:{s['id']}"}


def subscription_order_links(data):
    """Preserve explicit source links; timestamp matches are evidence, not authority."""
    grants = {s["id"]: s for s in data["user_subscriptions"]}
    results = []
    for order in data["subscription_orders"]:
        if order["status"] != "success":
            continue
        source_id = order.get("user_subscription_id") or 0
        grant = grants.get(source_id)
        if grant is not None:
            state = "matched" if (grant["user_id"], grant["plan_id"]) == (order["user_id"], order["plan_id"]) else "ownership_mismatch"
        else:
            state = "missing_source_link" if source_id == 0 else "missing_source_grant"
        candidates = [s["id"] for s in grants.values()
                      if (s["user_id"], s["plan_id"], s["start_time"]) ==
                      (order["user_id"], order["plan_id"], order["complete_time"])]
        results.append({"source_order_id": order["id"], "source_subscription_id": source_id,
                        "state": state, "timestamp_candidate_ids": candidates})
    return results


def plan_subscriptions(data, archive_sha, migration_id, observed_at, *, plans_pending_only=False):
    sql, stats = [], Counter()
    actor = superadmin(data)
    plans = {p["id"]: p for p in data["subscription_plans"]}
    for p in data["subscription_plans"]:
        terms, archived = plan_terms(p, data["options"])
        sql.append("INSERT INTO core_subscriptions.plans(id,revision,archived) OVERRIDING SYSTEM VALUE "
                   f"VALUES({p['id']},1,{lit(archived)});")
        sql.append("INSERT INTO core_subscriptions.plan_revisions(plan_id,revision,terms,actor_user_id,created_at) "
                   f"VALUES({p['id']},1,{jlit(terms)},{actor},{p['updated_at'] or p['created_at']});")
        stats["plans"] += 1
        stats["plans_purchasable"] += terms["enabled"] and not archived
        stats["plans_calendar_duration"] += p["duration_unit"] in ("month", "year")
    if plans:
        sql.append(f"SELECT setval(pg_get_serial_sequence('core_subscriptions.plans','id'),{max(plans)});")
    users = {u["id"] for u in data["users"]}
    # Formal opening owns charging grants and their opening consumption when selected.
    subscriptions = [] if plans_pending_only else data["user_subscriptions"]
    if plans_pending_only:
        stats["user_subscriptions_deferred_to_formal_opening"] = len(data["user_subscriptions"])
    for s in subscriptions:
        if s["user_id"] not in users or s["plan_id"] not in plans:
            fail("subscription_orphan")
        r = subscription_record(s, plans[s["plan_id"]], observed_at, migration_id,
                                account_id=personal_account(data, s["user_id"]))
        sql.append("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,timezone,anchor,seconds,"
                   "ends_at,limit_credits,unlimited,reset_basis,reset_limit_credits,reset_unlimited,models,groups,active) "
                   f"VALUES({r['account_id']},{lit(r['grant_reference'])},{lit(r['period'])},'UTC',{r['anchor']},"
                   f"{r['seconds']},{r['ends_at']},{r['limit_credits']},{lit(r['unlimited'])},{lit(r['reset_basis'])},"
                   f"{r['reset_limit_credits']},{lit(r['reset_unlimited'])},ARRAY['*'],ARRAY['*'],{lit(r['active'])});")
        sql.append("INSERT INTO core_charging.subscription_opening_consumption(subscription_id,start_at,end_at,"
                   f"consumed_credits,import_reference) SELECT id,{r['opening_start']},{r['opening_end']},"
                   f"{r['opening_used']},grant_reference FROM core_charging.subscriptions "
                   f"WHERE grant_reference={lit(r['grant_reference'])};")
        stats["user_subscriptions"] += 1
        stats["user_subscriptions_active"] += r["active"]
        if r["active"] and r["remaining"] is not None:
            stats["active_remaining_units"] += max(r["remaining"], 0)
    for link in subscription_order_links(data):
        stats["successful_order_link_" + link["state"]] += 1
    for o in data["subscription_orders"]:
        stats["orders_" + str(o["status"])] += 1
        if o["status"] != "pending":
            continue
        sql.append(pending_liability(data, "subscription_orders", o, archive_sha=archive_sha,
                                     migration_id=migration_id, observed_at=observed_at,
                                     amount_fields=("money", "expected_amount_micros", "refunded_amount_micros",
                                                    "refunded_quota", "charged_quota"),
                                     currency_fields=("plan_currency", "settlement_currency")))
        stats["pending_order_liabilities"] += 1
    return sql, dict(stats)


# ------------------------------------------------------------------ promotions
def discount_available(code, reserved_slots, observed_at):
    """Old reservations retain capacity even past TTL while payment can settle."""
    return (code["status"] == 1 and code["deleted_at"] is None
            and (not code["starts_time"] or code["starts_time"] <= observed_at)
            and (not code["expired_time"] or code["expired_time"] >= observed_at)
            and (not code["max_uses"] or code["used_count"] + reserved_slots < code["max_uses"]))


def plan_promotions(data, archive_sha, migration_id, observed_at):
    sql, stats = [], Counter()
    admin = superadmin(data)
    users = {u["id"] for u in data["users"]}
    groups = defaultdict(list)
    for r in data["redemptions"]:
        key = (r["key"] or "").strip()
        if len(key) != 32 or any(c not in "0123456789abcdef" for c in key):
            fail("redemption_key_format_unsupported")
        if r["reward_type"] != "quota":
            stats["skipped_reward_" + r["reward_type"]] += 1
            stats["skipped_reward_" + r["reward_type"] + "_redeemable"] += (
                r["status"] == 1 and r["deleted_at"] is None
                and (not r["expired_time"] or r["expired_time"] > observed_at))
            continue
        if not 1 <= r["quota"] <= MAX_UNITS:
            fail("redemption_quota_invalid")
        groups[(r["name"] or "", r["quota"], r["expired_time"] or 0)].append(r)
    for (name, quota, expires), rows in sorted(groups.items(), key=lambda kv: min(r["id"] for r in kv[1])):
        cid = sha_hex(json.dumps(["legacy-redemption", migration_id, name, quota, expires], ensure_ascii=False))
        redeemable = [r for r in rows if r["status"] == 1 and r["deleted_at"] is None
                      and (not expires or expires > observed_at)]
        used = [r for r in rows if r["status"] == 3]
        max_claims = max(1, len(used) + len(redeemable))
        title = " ".join(name.split())[:150] or "redemption"
        starts = min(r["created_time"] or 0 for r in rows)
        ends = expires if expires else MAX_TIME
        if ends <= starts:
            fail("redemption_window_invalid")
        sql.append("INSERT INTO core_promotions.campaigns(id,revision,kind,title,award_units,budget_units,spent_units,"
                   "reserved_units,max_claims,claims,reserved_claims,max_claims_per_user,starts_at,ends_at,enabled) "
                   f"VALUES({lit(cid)},1,'redemption',{lit('Legacy ' + title)},{quota},{quota * max_claims},"
                   f"{quota * len(used)},{quota * len(redeemable)},{max_claims},{len(used)},{len(redeemable)},"
                   f"{min(10000, max_claims)},{starts},{ends},{lit(bool(redeemable))});")
        redeemable_ids = {r["id"] for r in redeemable}
        by_creator = defaultdict(list)
        for r in sorted(rows, key=lambda r: r["id"]):
            by_creator[r["user_id"] if r["user_id"] in users else admin].append(r)
        for creator, items in sorted(by_creator.items()):
            for chunk_start in range(0, len(items), 100):
                chunk = items[chunk_start:chunk_start + 100]
                iid = sha_hex(json.dumps(["legacy-redemption-issue", cid, creator, chunk_start]))
                sql.append("INSERT INTO core_promotions.issues(id,campaign_id,actor_user_id,quantity,created_at) "
                           f"VALUES({lit(iid)},{lit(cid)},{creator},{len(chunk)},"
                           f"{min(r['created_time'] or 0 for r in chunk)});")
                for pos, r in enumerate(chunk):
                    sql.append("INSERT INTO core_promotions.codes(digest,issue_id,position,campaign_id,revoked) "
                               f"VALUES(decode({lit(sha_hex(r['key'].strip()))},'hex'),{lit(iid)},{pos},{lit(cid)},"
                               f"{lit(r['id'] not in redeemable_ids)});")
        stats["redemption_campaigns"] += 1
        stats["codes"] += len(rows)
        stats["codes_redeemable"] += len(redeemable)
        stats["codes_redeemable_units"] += quota * len(redeemable)
        stats["codes_used_recorded"] += len(used)
        stats["codes_revoked_other"] += len(rows) - len(used) - len(redeemable)
    # Legacy check-in awarded a random, trust-scaled amount per day.
    # Historical averages are not the configured reward contract. Legacy
    # check-ins use trust-level-dependent random ranges and no inferred budget.
    # Preserve the gap explicitly until that contract can be imported exactly.
    stats["unrepresented_checkin_enabled"] = data["options"].get("checkin_setting.enabled") == "true"
    stats["legacy_checkins_history_not_imported"] = data["checkin_total"][0]["n"]
    live = lambda expires: not expires or expires > observed_at  # noqa: E731
    reserved = discount_capacity_holds(data)
    stats["unrepresented_discount_codes_usable"] = sum(
        discount_available(d, reserved[d["id"]], observed_at) for d in data["discount_codes"])
    stats["unrepresented_discount_code_reservations"] = len(data["discount_code_reservations"])
    stats["unrepresented_discount_slots_reserved"] = sum(reserved.values())
    active = {p["id"] for p in data["red_packets"] if p["enabled"] and p["deleted_at"] is None and live(p["end_at"])}
    stats["unrepresented_red_packets_active"] = len(active)
    for i in data["red_packet_items"]:
        if i["packet_id"] in active and not i["claimed_by"]:
            stats["unrepresented_red_packet_unclaimed_" + i["item_type"]] += 1
    stats["unrepresented_red_packet_claims_history"] = data["red_packet_claims"][0]["n"]
    for r in data["referral_rewards"]:
        if r["status"] == "earned" and r["stage"] == "awaiting_first_top_up":
            stats["unrepresented_referral_rewards_awaiting_first_top_up"] += r["n"]
    for row in data["other_counts"]:
        stats["unrepresented_" + row["k"]] = row["n"]
    return sql, dict(stats)


# -------------------------------------------------------------------- bounties
def plan_bounties(data, archive_sha, migration_id, observed_at):
    stats = Counter()
    users = {u["id"] for u in data["users"]}
    challenges = defaultdict(list)
    for c in data["bounty_challenges"]:
        challenges[c["project_id"]].append(c)
    disputes = defaultdict(list)
    for d in data["bounty_disputes"]:
        disputes[d["project_id"]].append(d)
    projects = []
    for p in data["bounty_projects"]:
        stats["projects_" + p["status"]] += 1
        if not p["escrow_quota"]:
            continue
        if p["status"] not in ("published", "paused") or p["owner_user_id"] not in users:
            fail("bounty_funded_project_unsupported")
        claims, owed = [], 0
        for c in challenges[p["id"]]:
            if c["paid_at"] or c["status"] not in ("accepted", "submitted", "rejected"):
                continue
            guard_only = c["status"] == "rejected" and c["rejected_at"] <= observed_at - 604800
            if c["status"] == "rejected" and not guard_only:
                fail("bounty_rejection_in_appeal_window_needs_review")
            if c["participant_user_id"] not in users or c["participant_user_id"] == p["owner_user_id"]:
                fail("bounty_claim_participant_invalid")
            claims.append({"source_challenge_id": c["id"], "participant_user_id": c["participant_user_id"],
                           "recipient_account_id": personal_account(data, c["participant_user_id"]),
                           "source_row_sha256": digest(c),
                           "rights_status": "historical_rejection_guard_only" if guard_only else "active_future_reward",
                           "source_reward_quota": c["reward_quota"],
                           "payout_units": None if guard_only else c["reward_quota"],
                           "status": c["status"], "rejected_at": c["rejected_at"], "paid_at": 0})
            owed += 0 if guard_only else c["reward_quota"]
            stats["claims_" + ("guard_only" if guard_only else "active")] += 1
        if owed > p["escrow_quota"]:
            fail("bounty_escrow_short")
        projects.append({"source_project_id": p["id"], "owner_user_id": p["owner_user_id"],
                         "payer_account_id": personal_account(data, p["owner_user_id"]), "source_row_sha256": digest(p),
                         "source_parent_plan_sha256": archive_sha, "escrow_units": p["escrow_quota"],
                         "status": p["status"], "claims": claims,
                         "disputes": [{"source_dispute_id": d["id"], "source_challenge_id": d["challenge_id"],
                                       "source_row_sha256": digest(d), "status": d["status"],
                                       "reward_quota_snapshot": d["reward_quota_snapshot"],
                                       "project_escrow_quota_snapshot": d["project_escrow_quota_snapshot"],
                                       "tip_quota_snapshot": d["tip_quota_snapshot"]} for d in disputes[p["id"]]]})
        stats["escrow_projects"] += 1
        stats["escrow_units"] += p["escrow_quota"]
        stats["escrow_owed_active_units"] += owed
    sql = []
    if projects:
        batch = {"migration_id": opening_run(data)["migration_id"], "source_archive_sha256": archive_sha,
                 "mapping_sha256": opening_mapping(data), "review_proof_sha256": digest(projects),
                 "unit": "credit_500k_usd", "observed_at": observed_at, "projects": projects}
        sql.append("SELECT core_billing.import_legacy_escrow_batch(" + jlit(batch) + ");")
    fee_bps = int(Decimal(data["options"].get("OpenSourceBountyFeeRate") or "0") * 100)
    fee_account = personal_account(data, superadmin(data))
    sql.append(f"INSERT INTO core_bounties.policy(id,fee_bps,fee_account_id) VALUES(TRUE,{fee_bps},{fee_account});")
    stats.update(fee_bps=fee_bps, fee_account_id=fee_account, disputes=len(data["bounty_disputes"]))
    return sql, dict(stats)


# ---------------------------------------------------------------------- wallet
def plan_wallet(data, archive_sha, migration_id, observed_at):
    """Restore each old pending liability, then reserve through the normal ledger.

    The two postings are atomic and net to zero in the sender wallet. Their
    payer/payee metadata exactly matches their entries; no synthetic reserve
    receipt may pretend clearing funds came from the wallet.
    """
    sql, stats = [], Counter()
    users = {u["id"] for u in data["users"]}
    mapping = opening_mapping(data)
    top = 0
    for t in data["wallet_transfers"]:
        if t["status"] not in ("pending", "claimed", "cancelled"):
            fail("wallet_transfer_status_unsupported")
        stats["transfers_" + t["status"]] += 1
        if t["status"] != "pending":
            finished = t["claimed_at"] if t["status"] == "claimed" else t["cancelled_at"]
            recipient = t["recipient_id"] if t["status"] == "claimed" else None
            if t["sender_id"] not in users or (recipient is not None and recipient not in users):
                fail("wallet_transfer_history_orphan")
            sql.append("INSERT INTO core_wallet.legacy_transfer_history(id,owner_user,recipient_user,amount,state,created_at,finished_at,migration_id,source_row_sha256) "
                       f"VALUES({t['id']},{t['sender_id']},{lit(recipient)},{t['quota']},{lit(t['status'])},{t['created_at']},{finished},{lit(migration_id)},{lit(digest(t))});")
            top = max(top, t["id"])
            stats["terminal_transfer_history"] += 1
            continue
        tid, sender, amount = t["id"], t["sender_id"], t["quota"]
        if (type(tid) is not int or tid <= 0 or sender not in users
                or type(amount) is not int or not 0 < amount <= MAX_UNITS
                or re.fullmatch(r"[0-9a-fA-F]{64}", t["token"]) is None
                or re.fullmatch(r"[A-Za-z0-9_-]{16,64}", t["request_key"]) is None
                or not 0 < t["created_at"] <= observed_at
                or t.get("recipient_id", 0) or t.get("claimed_at", 0) or t.get("cancelled_at", 0)):
            fail("wallet_transfer_invalid")
        payer = personal_account(data, sender)
        request = {"importer": "legacy_cutover_money", "migration_id": migration_id,
                   "source_archive_sha256": archive_sha, "mapping_sha256": mapping,
                   "observed_at": observed_at, "source_table": "wallet_transfers",
                   "source_id": tid, "source_row_sha256": digest(t),
                   "funding": "legacy already-debited pending liability"}
        hold = {"scope": "core-wallet-transfer", "operation_key": f"hold-{tid}",
                "actor_user_id": sender, "reason": "Imported pending personal wallet transfer",
                "action": {"kind": "reserve", "account_id": payer, "amount_units": amount}}
        sql.append(f"""DO $wt$DECLARE w BIGINT; c BIGINT; j BIGINT; h JSONB; before_units BIGINT; BEGIN
 IF current_user<>pg_get_userbyid((SELECT nspowner FROM pg_namespace WHERE nspname='core_billing'))
   OR current_setting('transaction_isolation')<>'serializable' OR current_setting('synchronous_commit')<>'on' THEN
  RAISE EXCEPTION 'wallet import requires durable offline owner transaction';
 END IF;
 IF NOT EXISTS(SELECT 1 FROM core_billing.opening_import_runs r JOIN core_billing.opening_imports i USING(migration_id)
   WHERE r.migration_id={lit(opening_run(data)['migration_id'])} AND r.source_archive_sha256={lit(archive_sha)}
   AND r.mapping_sha256={lit(mapping)} AND r.observed_at={observed_at} AND i.account_id={payer}) THEN
  RAISE EXCEPTION 'wallet import opening provenance mismatch';
 END IF;
 INSERT INTO core_wallet.transfers(id,owner_user,payer_account,request_key,amount,token_digest,created_at) OVERRIDING SYSTEM VALUE
  VALUES({tid},{sender},{payer},{lit(t['request_key'])},{amount},sha256(convert_to({lit(t['token'])},'UTF8')),{t['created_at']});
 SELECT id INTO STRICT w FROM core_billing.ledger_accounts WHERE owner_account_id={payer} AND bucket='wallet';
 SELECT id INTO STRICT c FROM core_billing.ledger_accounts WHERE bucket='clearing';
 PERFORM ledger_account_id FROM core_billing.balance_state WHERE ledger_account_id IN(w,c) ORDER BY ledger_account_id FOR NO KEY UPDATE;
 SELECT balance_units INTO STRICT before_units FROM core_billing.balance_state WHERE ledger_account_id=w;
 INSERT INTO core_billing.ledger_journals(scope,operation_key,kind,payer_id,payee_id,amount_units,actor_user_id,reason)
  VALUES('legacy-wallet-transfer-opening','transfer-{tid}','opening',c,w,{amount},{sender},'Legacy already-debited pending transfer liability') RETURNING id INTO j;
 INSERT INTO core_billing.ledger_entries(journal_id,ledger_account_id,delta_units)
  SELECT j,id,units FROM (VALUES(c,{-amount}::BIGINT),(w,{amount}::BIGINT)) v(id,units) ORDER BY id;
 INSERT INTO core_billing.ledger_operations(scope,operation_key,request,result)
  VALUES('legacy-wallet-transfer-opening','transfer-{tid}',{jlit(request)},jsonb_build_object('status','posted','journal_id',j));
 h := core_billing.post_ledger({jlit(hold)});
 IF h->>'status' IS DISTINCT FROM 'posted' THEN RAISE EXCEPTION 'imported transfer reserve failed'; END IF;
 IF (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=w)<>before_units THEN
  RAISE EXCEPTION 'imported transfer changed available balance';
 END IF;
 UPDATE core_wallet.transfers SET state='pending',hold_journal=(h->>'journal_id')::BIGINT,confirmed_at={t['created_at']} WHERE id={tid};
END$wt$;""")
        top = max(top, tid)
        stats["pending_transfers"] += 1
        stats["pending_units"] += amount
    if top:
        sql.append(f"SELECT setval(pg_get_serial_sequence('core_wallet.transfers','id'),{top});")
    if "top_ups_payable" not in data:
        fail("topup_payable_lifecycle_missing")
    for row in data["top_ups_payable"]:
        if not topup_can_settle(row):
            fail("topup_not_payable")
        if row["user_id"] not in users:
            # Keep the complete row in the private source export; never assign a
            # deleted user's payment to a new account or mark it settled.
            stats["unrepresented_payable_topup_orphan_rows"] += 1
            continue
        sql.append(pending_liability(data, "top_ups", row, archive_sha=archive_sha, migration_id=migration_id,
                                     observed_at=observed_at,
                                     amount_fields=("amount", "money", "expected_amount_micros", "settled_amount_micros",
                                                    "credited_quota", "refunded_amount_micros", "refunded_quota",
                                                    "platform_amount_micros"),
                                     currency_fields=("settlement_currency", "discount_qualifying_unit")))
        stats["pending_topup_liabilities"] += 1
        stats["late_payable_timeout_topup_liabilities"] += row["status"] == "failed"
    return sql, dict(stats)


def plan(phase, data, archive_sha, migration_id, observed_at):
    return {"subscriptions": plan_subscriptions, "promotions": plan_promotions,
            "bounties": plan_bounties, "wallet": plan_wallet}[phase](data, archive_sha, migration_id, observed_at)
