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



def fail(code: str) -> None:
    raise SystemExit(f"error: {code}")


def lit(value) -> str:
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE"
    if isinstance(value, int):
        return str(value)
    return "'" + str(value).replace("'", "''") + "'"


def digest(value) -> str:
    # Same canonical form as legacy_cutover_import.digest (opening mapping binding).
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True, default=str,
                                     separators=(",", ":")).encode()).hexdigest()

PHASES = ("subscriptions", "promotions", "bounties", "wallet")
GUARDS = {
    "subscriptions": "SELECT (SELECT count(*) FROM core_subscriptions.plans)"
                     "+(SELECT count(*) FROM core_charging.subscriptions WHERE grant_reference LIKE 'legacy-sub:%')"
                     "+(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE source_table='subscription_orders')",
    "promotions": "SELECT count(*) FROM core_promotions.campaigns",
    "bounties": "SELECT (SELECT count(*) FROM core_billing.legacy_escrow_batch_runs)"
                "+(SELECT count(*) FROM core_bounties.policy)",
    "wallet": "SELECT (SELECT count(*) FROM core_wallet.transfers)"
              "+(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE source_table='top_ups')",
}
CREDITS_PER_USD = 500000
MAX_TIME = 253402300799
MAX_UNITS = 1_000_000_000_000
DAY = 86400
MONTH_SECONDS = 30 * DAY  # Core plans have fixed durations; see report "month_as_30_days".
PERIODS = {"never": "grant", "daily": "day", "weekly": "week", "monthly": "month", "custom": "custom"}
DURATION = {"hour": 3600, "day": DAY, "week": 7 * DAY, "month": MONTH_SECONDS, "year": 365 * DAY}


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
        "top_ups_pending": q("SELECT * FROM top_ups WHERE status='pending' ORDER BY id"),
        "discount_codes": q("SELECT id,status,used_count,max_uses,expired_time,deleted_at FROM discount_codes"),
        "discount_code_reservations": q("SELECT id,status FROM discount_code_reservations"),
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
    # Must equal plan_opening's mapping so owner functions bind the same run.
    return digest([[u["id"], u["id"]] for u in data["users"]])


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
        "migration_id": migration_id, "source_archive_sha256": archive_sha,
        "mapping_sha256": opening_mapping(data), "source_table": table, "source_row_key": str(row["id"]),
        "source_row_sha256": digest(row), "source_parent_plan_sha256": None,
        "owner_user_id": uid, "owner_account_id": uid, "status": "pending", "settlement_state": "unknown",
        "payload_json": row, "amount_raw_json": {k: row.get(k) for k in amount_fields},
        "currency_raw_json": {k: row.get(k) for k in currency_fields},
        "spendable": False, "spendable_units": 0, "observed_at": observed_at,
        "request": {"importer": "legacy_cutover_money", "source_status": "pending"},
        "result": {"provider_status": "unobserved"}}) + ");"


# ---------------------------------------------------------------- subscriptions
def plan_terms(p: dict, options: dict) -> tuple[dict, bool]:
    unit = p["duration_unit"]
    if unit == "custom":
        duration = p["custom_seconds"]
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
        "reset_period": period,
        "reset_seconds": p["quota_reset_custom_seconds"] if period == "custom" else 0,
        "limit_units": str(p["total_amount"]),
        "models": ["*"], "groups": ["*"],
        "max_purchases_per_user": p["max_purchase_per_user"],
        "enabled": bool(p["enabled"]) and not p["archived_at"],
    }
    if not (1 <= int(terms["price_units"]) <= MAX_UNITS and 1 <= p["total_amount"] <= MAX_UNITS
            and 60 <= duration <= 315_576_000 and 0 <= terms["max_purchases_per_user"] <= 10_000
            and (period != "custom" or 1 <= terms["reset_seconds"] <= 315_576_000)):
        fail("plan_terms_invalid")
    return terms, bool(p["archived_at"])


def subscription_record(s: dict, plan: dict, observed_at: int, migration_id: str) -> dict:
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
    return {"source_id": s["id"], "account_id": s["user_id"], "period": period, "anchor": start,
            "seconds": seconds, "ends_at": end, "limit_credits": 0 if unlimited else total,
            "unlimited": unlimited, "reset_basis": reset_basis, "reset_limit_credits": reset_limit,
            "reset_unlimited": reset_unlimited,
            "active": s["status"] == "active" and end > observed_at,
            "opening_start": o_start, "opening_end": o_end, "opening_used": used,
            "remaining": None if unlimited else total - used,
            "grant_reference": f"legacy-sub:{migration_id}:{s['id']}"}


def plan_subscriptions(data, archive_sha, migration_id, observed_at):
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
        stats["plans_month_as_30_days"] += p["duration_unit"] == "month"
    if plans:
        sql.append(f"SELECT setval(pg_get_serial_sequence('core_subscriptions.plans','id'),{max(plans)});")
    users = {u["id"] for u in data["users"]}
    for s in data["user_subscriptions"]:
        if s["user_id"] not in users or s["plan_id"] not in plans:
            fail("subscription_orphan")
        r = subscription_record(s, plans[s["plan_id"]], observed_at, migration_id)
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
    # Legacy check-in awarded a random, trust-scaled amount per day. Core awards
    # a fixed amount: use the legacy mean of the final 30 days (owner may revise).
    recent = data["checkin_recent"][0]
    if data["options"].get("checkin_setting.enabled") == "true" and recent["n"]:
        award = int((Decimal(recent["total"]) / recent["n"]).quantize(Decimal(1), rounding=ROUND_HALF_UP))
        max_claims = min(1_000_000, recent["n"] * 13)  # ~one year at the legacy daily rate
        cid = sha_hex(json.dumps(["legacy-checkin", migration_id]))
        sql.append("INSERT INTO core_promotions.campaigns(id,revision,kind,title,award_units,budget_units,max_claims,"
                   "max_claims_per_user,starts_at,ends_at,enabled) "
                   f"VALUES({lit(cid)},1,'checkin','Daily check-in',{award},{award * max_claims},{max_claims},"
                   f"366,{observed_at},{observed_at + 365 * DAY},TRUE);")
        stats.update(checkin_campaigns=1, checkin_award_units=award, checkin_max_claims=max_claims)
    stats["legacy_checkins_history_not_imported"] = data["checkin_total"][0]["n"]
    live = lambda expires: not expires or expires > observed_at  # noqa: E731
    stats["unrepresented_discount_codes_usable"] = sum(
        d["status"] == 1 and d["deleted_at"] is None and live(d["expired_time"])
        and (not d["max_uses"] or d["used_count"] < d["max_uses"]) for d in data["discount_codes"])
    stats["unrepresented_discount_code_reservations"] = len(data["discount_code_reservations"])
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
                           "recipient_account_id": c["participant_user_id"], "source_row_sha256": digest(c),
                           "rights_status": "historical_rejection_guard_only" if guard_only else "active_future_reward",
                           "source_reward_quota": c["reward_quota"],
                           "payout_units": None if guard_only else c["reward_quota"],
                           "status": c["status"], "rejected_at": c["rejected_at"], "paid_at": 0})
            owed += 0 if guard_only else c["reward_quota"]
            stats["claims_" + ("guard_only" if guard_only else "active")] += 1
        if owed > p["escrow_quota"]:
            fail("bounty_escrow_short")
        projects.append({"source_project_id": p["id"], "owner_user_id": p["owner_user_id"],
                         "payer_account_id": p["owner_user_id"], "source_row_sha256": digest(p),
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
        batch = {"migration_id": migration_id, "source_archive_sha256": archive_sha,
                 "mapping_sha256": opening_mapping(data), "review_proof_sha256": digest(projects),
                 "unit": "credit_500k_usd", "observed_at": observed_at, "projects": projects}
        sql.append("SELECT core_billing.import_legacy_escrow_batch(" + jlit(batch) + ");")
    fee_bps = int(Decimal(data["options"].get("OpenSourceBountyFeeRate") or "0") * 100)
    sql.append(f"INSERT INTO core_bounties.policy(id,fee_bps,fee_account_id) VALUES(TRUE,{fee_bps},{superadmin(data)});")
    stats.update(fee_bps=fee_bps, fee_account_id=superadmin(data), disputes=len(data["bounty_disputes"]))
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
            continue
        tid, sender, amount = t["id"], t["sender_id"], t["quota"]
        if (type(tid) is not int or tid <= 0 or sender not in users
                or type(amount) is not int or not 0 < amount <= MAX_UNITS
                or re.fullmatch(r"[0-9a-fA-F]{64}", t["token"]) is None
                or re.fullmatch(r"[A-Za-z0-9_-]{16,64}", t["request_key"]) is None
                or not 0 < t["created_at"] <= observed_at
                or t.get("recipient_id", 0) or t.get("claimed_at", 0) or t.get("cancelled_at", 0)):
            fail("wallet_transfer_invalid")
        request = {"importer": "legacy_cutover_money", "migration_id": migration_id,
                   "source_archive_sha256": archive_sha, "mapping_sha256": mapping,
                   "observed_at": observed_at, "source_table": "wallet_transfers",
                   "source_id": tid, "source_row_sha256": digest(t),
                   "funding": "legacy already-debited pending liability"}
        hold = {"scope": "core-wallet-transfer", "operation_key": f"hold-{tid}",
                "actor_user_id": sender, "reason": "Imported pending personal wallet transfer",
                "action": {"kind": "reserve", "account_id": sender, "amount_units": amount}}
        sql.append(f"""DO $wt$DECLARE w BIGINT; c BIGINT; j BIGINT; h JSONB; before_units BIGINT; BEGIN
 IF current_user<>pg_get_userbyid((SELECT nspowner FROM pg_namespace WHERE nspname='core_billing'))
   OR current_setting('transaction_isolation')<>'serializable' OR current_setting('synchronous_commit')<>'on' THEN
  RAISE EXCEPTION 'wallet import requires durable offline owner transaction';
 END IF;
 IF NOT EXISTS(SELECT 1 FROM core_billing.opening_import_runs r JOIN core_billing.opening_imports i USING(migration_id)
   WHERE r.migration_id={lit(migration_id)} AND r.source_archive_sha256={lit(archive_sha)}
   AND r.mapping_sha256={lit(mapping)} AND r.observed_at={observed_at} AND i.account_id={sender}) THEN
  RAISE EXCEPTION 'wallet import opening provenance mismatch';
 END IF;
 INSERT INTO core_wallet.transfers(id,owner_user,payer_account,request_key,amount,token_digest,created_at) OVERRIDING SYSTEM VALUE
  VALUES({tid},{sender},{sender},{lit(t['request_key'])},{amount},sha256(convert_to({lit(t['token'])},'UTF8')),{t['created_at']});
 SELECT id INTO STRICT w FROM core_billing.ledger_accounts WHERE owner_account_id={sender} AND bucket='wallet';
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
    for row in data["top_ups_pending"]:
        sql.append(pending_liability(data, "top_ups", row, archive_sha=archive_sha, migration_id=migration_id,
                                     observed_at=observed_at,
                                     amount_fields=("amount", "money", "expected_amount_micros", "settled_amount_micros",
                                                    "credited_quota", "refunded_amount_micros", "refunded_quota",
                                                    "platform_amount_micros"),
                                     currency_fields=("settlement_currency", "discount_qualifying_unit")))
        stats["pending_topup_liabilities"] += 1
    return sql, dict(stats)


def plan(phase, data, archive_sha, migration_id, observed_at):
    return {"subscriptions": plan_subscriptions, "promotions": plan_promotions,
            "bounties": plan_bounties, "wallet": plan_wallet}[phase](data, archive_sha, migration_id, observed_at)
