#!/usr/bin/env python3
"""Import legacy red packets into Core and inventory other money-adjacent features.

Reads the restored legacy schema read-only and writes one owner-only SERIALIZABLE
transaction into a fresh Core that already contains:

* identities with legacy user IDs (legacy_cutover_import.py phase ``identity``);
* the legacy redemption codes (phase ``promotions``), stored as SHA-256 of the
  original 32-hex key with their original budget reservation.

Legacy red packets were administrator share links over existing redemption and
discount codes; creating a packet never debited anyone. Here a packet item binds
the already-imported Core code, so the remaining value stays claimable exactly
once (either through the packet or by redeeming the original key, never both)
and no wallet is debited or credited by this import. Discount codes, reset
vouchers, revoked/expired/used codes and items of deleted packets are not bound
and are reported. Every legacy claim is kept as a read-only reward snapshot so
recipients keep seeing what they received.

Hero SMS orders and public relay contributions are only inventoried: the report
proves no order holds funds (all terminal) and no relay tip balance exists.

Connections use local Unix sockets only (libpq environment), never DSNs in argv.
The private SQL plan and report are written under --output (0700).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from collections import Counter
from pathlib import Path


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


def jlit(value) -> str:
    return lit(json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))) + "::jsonb"


class Db:
    def __init__(self, socket: Path, port: int, database: str, user: str, *, role: str | None = None,
                 readonly: bool):
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        self.env.update(PGHOST=str(socket), PGPORT=str(port), PGDATABASE=database, PGUSER=user,
                        PGAPPNAME="lmm-legacy-money-features-import", PGCONNECT_TIMEOUT="5")
        options = "-c default_transaction_read_only=" + ("on" if readonly else "off")
        if role:
            options += " -c role=" + role
        self.env["PGOPTIONS"] = options

    def run(self, sql: str) -> str:
        result = subprocess.run(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
                                input=sql, text=True, capture_output=True, env=self.env, check=False)
        if result.returncode:
            sys.stderr.write(result.stderr[-4000:])
            fail("psql_failed")
        return result.stdout

    def rows(self, sql: str) -> list[dict]:
        return json.loads(self.run(f"SELECT COALESCE(json_agg(t),'[]') FROM ({sql}) t"))


def load_source(src: Db, schema: str) -> dict:
    def q(sql: str) -> list[dict]:
        return json.loads(src.run(f"SET search_path={schema};SELECT COALESCE(json_agg(t),'[]') FROM ({sql}) t"))
    return {
        "packets": q("SELECT id,slug,title,description,coalesce(cover_image,'') AS cover_image,"
                     "cover_prompt,draw_mode,per_user_limit,start_at,end_at,enabled,created_by,created_at,"
                     "updated_at,extract(epoch FROM deleted_at)::bigint AS deleted_at FROM red_packets ORDER BY id"),
        "items": q("SELECT * FROM red_packet_items ORDER BY id"),
        "claims": q("SELECT * FROM red_packet_claims ORDER BY id"),
        "redemptions": q("SELECT r.id,r.key,r.name,r.quota,r.reward_type,r.reset_plan_id,r.reset_voucher_expires_at,"
                         "r.status,r.expired_time,r.deleted_at IS NOT NULL AS deleted FROM redemptions r "
                         "WHERE r.id IN (SELECT source_id FROM red_packet_items WHERE item_type='redemption')"),
        "discounts": q("SELECT d.id,d.code,d.name,d.discount_percent,d.min_amount,d.expired_time FROM discount_codes d "
                       "WHERE d.id IN (SELECT source_id FROM red_packet_items WHERE item_type='discount')"),
        "hero_sms_orders": q("SELECT status,coalesce(cancel_final_status,'') AS cancel_final_status,count(*) AS n,"
                             "sum(reserved_quota)::bigint AS reserved,sum(charge_quota)::bigint AS charged,"
                             "sum(refunded_quota)::bigint AS refunded FROM hero_sms_sms_orders GROUP BY 1,2 ORDER BY 1,2"),
        "hero_sms_ledgers": q("SELECT entry_type,count(*) AS n,sum(amount_quota)::bigint AS amount "
                              "FROM hero_sms_sms_quota_ledgers GROUP BY 1 ORDER BY 1"),
        "hero_sms_email": q("SELECT (SELECT count(*) FROM hero_sms_email_orders) AS orders,"
                            "(SELECT count(*) FROM hero_sms_email_activations) AS activations,"
                            "(SELECT count(*) FROM hero_sms_email_quota_ledgers) AS ledgers"),
        "relay": q("SELECT (SELECT count(*) FROM public_relay_contributions) AS contributions,"
                   "(SELECT coalesce(sum(tip_quota),0)::bigint FROM public_relay_contributions) AS tip_quota,"
                   "(SELECT coalesce(sum(withdrawn_quota),0)::bigint FROM public_relay_contributions) AS withdrawn_quota,"
                   "(SELECT count(*) FROM public_relay_tips) AS tips,"
                   "(SELECT count(*) FROM public_relay_preferences) AS preferences"),
    }


def reward_snapshot(claim: dict, redemptions: dict, discounts: dict) -> dict | None:
    """Same fields as the legacy RedPacketReward (Go omitempty preserved)."""
    out = {"item_type": claim["item_type"], "legacy": True}
    if claim["item_type"] == "redemption":
        row = redemptions.get(claim["source_id"])
        if row is None:
            return None
        out.update(code=(row["key"] or "").strip(), name=row["name"] or "",
                   reward_type=row["reward_type"] or "quota")
        for field in ("quota", "reset_plan_id", "reset_voucher_expires_at"):
            if row[field]:
                out["quota" if field == "quota" else field] = row[field]
    elif claim["item_type"] == "discount":
        row = discounts.get(claim["source_id"])
        if row is None:
            return None
        out.update(code=row["code"] or "", name=row["name"] or "")
        for field, name in (("discount_percent", "discount_percent"), ("min_amount", "min_amount"),
                            ("expired_time", "expires_at")):
            if row[field]:
                out[name] = row[field]
    else:
        return None
    return out


def plan(data: dict, target: Db, observed_at: int) -> tuple[list[str], dict]:
    if int(target.run("SELECT count(*) FROM core_promotions.red_packets")) != 0:
        fail("red_packets_target_not_empty")
    if int(target.run("SELECT count(*) FROM core_promotions.codes")) == 0:
        fail("promotions_phase_missing")
    users = {r["id"] for r in target.rows("SELECT id FROM core_identity.users")}
    roots = target.rows("SELECT id FROM core_identity.users WHERE platform_role='superadmin' ORDER BY id")
    if len(roots) != 1:
        fail("superadmin_ambiguous")
    root = roots[0]["id"]
    codes = {r["id"]: r for r in target.rows(
        "SELECT encode(d.digest,'hex') AS id,d.revoked,c.kind,c.enabled,c.starts_at,c.ends_at,"
        "EXISTS(SELECT 1 FROM core_promotions.claims l WHERE l.code_digest=d.digest) AS claimed "
        "FROM core_promotions.codes d JOIN core_promotions.campaigns c ON c.id=d.campaign_id")}
    redemptions = {r["id"]: r for r in data["redemptions"]}
    discounts = {r["id"]: r for r in data["discounts"]}
    stats: Counter = Counter()
    sql = ["BEGIN ISOLATION LEVEL SERIALIZABLE;", "SET LOCAL synchronous_commit=on;",
           "DO $$BEGIN IF (SELECT count(*) FROM core_promotions.red_packets)<>0 "
           "THEN RAISE EXCEPTION 'phase target not empty'; END IF; END$$;"]
    deleted = set()
    for p in data["packets"]:
        creator = p["created_by"] if p["created_by"] in users else root
        stats["packets_creator_remapped"] += creator != p["created_by"]
        title = " ".join((p["title"] or "").split())[:80] or "红包"
        mode = p["draw_mode"] if p["draw_mode"] in ("random", "weighted", "sequence") else "random"
        if p["deleted_at"] is not None:
            deleted.add(p["id"])
        sql.append(
            "INSERT INTO core_promotions.red_packets(id,slug,title,description,cover_image,cover_prompt,draw_mode,"
            "per_user_limit,start_at,end_at,enabled,created_by,created_at,updated_at,deleted_at) VALUES("
            f"{p['id']},{lit(p['slug'].strip())},{lit(title)},{lit((p['description'] or '')[:500])},"
            f"{lit(p['cover_image'])},{lit((p['cover_prompt'] or '')[:1000])},{lit(mode)},"
            f"{min(100, max(1, p['per_user_limit']))},{p['start_at']},{p['end_at']},"
            f"{lit(bool(p['enabled']) and p['deleted_at'] is None)},{creator},{p['created_at']},"
            f"{p['updated_at']},{lit(p['deleted_at'])});")
        stats["packets"] += 1
        stats["packets_deleted"] += p["id"] in deleted
        live = p["enabled"] and p["id"] not in deleted and (not p["end_at"] or p["end_at"] > observed_at)
        stats["packets_claimable_at_boundary"] += bool(live)
    for i in data["items"]:
        if i["claimed_by"]:
            continue
        reason = None
        if i["packet_id"] in deleted:
            reason = "packet_deleted"
        elif i["item_type"] != "redemption":
            reason = "discount_no_core_authority"
        else:
            row = redemptions.get(i["source_id"])
            if row is None or row["deleted"]:
                reason = "redemption_missing"
            elif (row["reward_type"] or "quota") != "quota":
                reason = "reset_voucher_no_core_authority"
            else:
                digest = hashlib.sha256((row["key"] or "").strip().encode()).hexdigest()
                code = codes.get(digest)
                if code is None:
                    reason = "core_code_missing"
                elif code["revoked"] or code["claimed"] or code["kind"] != "redemption":
                    reason = "core_code_not_redeemable"
                elif not code["enabled"] or code["ends_at"] <= observed_at:
                    reason = "core_code_expired"
        if reason:
            stats["unclaimed_items_not_bound_" + reason] += 1
            continue
        sql.append("INSERT INTO core_promotions.red_packet_items(id,packet_id,code_digest,weight,created_at) "
                   f"VALUES({i['id']},{i['packet_id']},decode({lit(digest)},'hex'),{min(1000000, max(1, i['weight']))},"
                   f"{i['created_at']});")
        stats["unclaimed_items_bound"] += 1
        stats["unclaimed_items_bound_units"] += row["quota"]
    for c in data["claims"]:
        if c["user_id"] not in users:
            stats["claims_skipped_user_missing"] += 1
            continue
        snapshot = reward_snapshot(c, redemptions, discounts)
        if snapshot is None:
            stats["claims_skipped_source_missing"] += 1
            continue
        sql.append("INSERT INTO core_promotions.red_packet_claims(id,packet_id,user_id,claim_index,reward,created_at) "
                   f"VALUES({c['id']},{c['packet_id']},{c['user_id']},{c['claim_index']},{jlit(snapshot)},"
                   f"{c['created_at']});")
        stats["claims_history"] += 1
        stats["claims_history_" + c["item_type"]] += 1
    for table in ("red_packets", "red_packet_items", "red_packet_claims"):
        sql.append(f"SELECT setval(pg_get_serial_sequence('core_promotions.{table}','id'),"
                   f"greatest(1,(SELECT coalesce(max(id),0) FROM core_promotions.{table})));")
    sql.append("COMMIT;")
    return sql, dict(stats)


def inventory(data: dict) -> dict:
    orders = data["hero_sms_orders"]
    terminal = {"completed", "cancelled", "failed", "refunded", "expired"}
    open_orders = [o for o in orders if o["status"] not in terminal]
    ledgers = {r["entry_type"]: r for r in data["hero_sms_ledgers"]}
    net = sum(r["amount"] for r in data["hero_sms_ledgers"])
    charged_net = sum(o["reserved"] - o["refunded"] for o in orders)
    return {
        "hero_sms": {
            "sms_orders_by_status": orders,
            "sms_ledgers": data["hero_sms_ledgers"],
            "email": data["hero_sms_email"][0],
            "non_terminal_orders": sum(o["n"] for o in open_orders),
            "ledger_net_debit_units": -net,
            "orders_net_charged_units": charged_net,
            "ledger_matches_orders": -net == charged_net,
            "reserve_entries": ledgers.get("reserve", {}).get("n", 0),
            "import": "not_imported_no_core_or_extension_destination",
        },
        "public_relay": {**data["relay"][0], "import": "not_imported_no_monetary_state"},
    }


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--socket", type=Path, required=True)
    p.add_argument("--port", type=int, required=True)
    p.add_argument("--source-db", required=True)
    p.add_argument("--source-user", required=True)
    p.add_argument("--source-schema", required=True)
    p.add_argument("--target-db", required=True)
    p.add_argument("--target-login", required=True)
    p.add_argument("--target-owner", required=True)
    p.add_argument("--observed-at", type=int, required=True, help="Unix seconds of the source boundary")
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--apply", action="store_true")
    a = p.parse_args()
    a.output.mkdir(mode=0o700, parents=False, exist_ok=True)
    src = Db(a.socket, a.port, a.source_db, a.source_user, readonly=True)
    data = load_source(src, a.source_schema)
    target = Db(a.socket, a.port, a.target_db, a.target_login, role=a.target_owner, readonly=True)
    sql, stats = plan(data, target, a.observed_at)
    report = {"phase": "red_packets", "observed_at": a.observed_at, "statements": len(sql),
              "plan_sha256": hashlib.sha256("\n".join(sql).encode()).hexdigest(), "stats": stats,
              "inventory": inventory(data), "applied": False}
    plan_path = a.output / "red-packets.sql"
    fd = os.open(plan_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write("\n".join(sql) + "\n")
    if a.apply:
        Db(a.socket, a.port, a.target_db, a.target_login, role=a.target_owner, readonly=False).run("\n".join(sql))
        check = target.rows(
            "SELECT (SELECT count(*) FROM core_promotions.red_packets) AS packets,"
            "(SELECT count(*) FROM core_promotions.red_packet_items) AS items,"
            "(SELECT count(*) FROM core_promotions.red_packet_claims) AS claims,"
            "(SELECT count(*) FROM core_billing.ledger_journals) AS journals")[0]
        if (check["packets"], check["items"], check["claims"]) != (
                stats.get("packets", 0), stats.get("unclaimed_items_bound", 0), stats.get("claims_history", 0)):
            fail("readback_mismatch")
        report.update(applied=True, readback=check)
    (a.output / "red-packets-report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
