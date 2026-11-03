#!/usr/bin/env python3
"""Offline import of legacy extension-owned data into per-module Go databases.

Reads the restored legacy schema read-only and writes one owner-only transaction
per module database (one phase per Go module). Legacy user IDs are kept as Core
user and personal account IDs. Go modules never hold balances: no phase writes
quota, wallet or ledger state; money facts stay with the Core cutover phases.

Each phase refuses a non-empty target, so it cannot run twice. Rows that the new
architecture cannot represent are reported as archived with a reason; the
restored snapshot and its dump remain the archival evidence.

Rows that only Core can own are never written here. Where a phase needs them it
emits a separate SQL file (core-<phase>.sql) for the coordinator's Core phase.

Connections use local Unix sockets only (libpq environment). Plans and reports
are written below --output (0700, under ~/Backups). Dry-run is the default.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

BACKUP_ROOT = Path.home() / "Backups"


def fail(code: str) -> None:
    raise SystemExit(f"error: {code}")


def lit(value: Any) -> str:
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE"
    if isinstance(value, int):
        return str(value)
    return "'" + str(value).replace("'", "''") + "'"


def jdump(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def jlit(value: Any) -> str:
    return lit(jdump(value)) + "::jsonb"


def sha(*parts: Any) -> str:
    """Go store/support ID(): sha256 of the JSON array of parts (Go encoding)."""
    raw = json.dumps(list(parts), ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(raw.encode()).hexdigest()


class Db:
    def __init__(self, socket: str, port: int, database: str, user: str, *, role: str | None, readonly: bool):
        self.database = database
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        self.env.update(PGHOST=socket, PGPORT=str(port), PGDATABASE=database, PGUSER=user,
                        PGAPPNAME="lmm-legacy-extensions-import", PGCONNECT_TIMEOUT="5")
        options = "-c default_transaction_read_only=" + ("on" if readonly else "off")
        if role:
            options += " -c role=" + role
        self.env["PGOPTIONS"] = options

    def run(self, sql: str) -> str:
        r = subprocess.run(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
                           input=sql, text=True, capture_output=True, env=self.env, check=False)
        if r.returncode:
            sys.stderr.write(r.stderr[-4000:])
            fail(f"psql_failed:{self.database}")
        return r.stdout

    def scalar(self, sql: str) -> str:
        return self.run(sql).strip()


@dataclass
class Plan:
    """Result of one phase. `sql` runs in one transaction in the module DB."""
    sql: list[str] = field(default_factory=list)
    # target table -> expected row count after import (reconciled after apply)
    expect: dict[str, int] = field(default_factory=dict)
    # legacy table -> (target or "archived"/"missing_backend", count, note)
    mapping: dict[str, tuple[str, int, str]] = field(default_factory=dict)
    # SQL the coordinator must run in the Core phase order (never executed here)
    core_sql: list[str] = field(default_factory=list)
    stats: Counter = field(default_factory=Counter)
    # optional post-apply hook (e.g. writing an encrypted snapshot through a binary)
    after: Callable[["Ctx", "Db"], None] | None = None


class Ctx:
    def __init__(self, src: Db, schema: str, args: argparse.Namespace):
        self.src, self.schema, self.args = src, schema, args
        self._cache: dict[str, list[dict]] = {}

    def q(self, sql: str) -> list[dict]:
        return json.loads(self.src.run(
            f"SET search_path={self.schema};SELECT COALESCE(json_agg(t),'[]') FROM ({sql}) t"))

    def rows(self, table: str, order: str = "1") -> list[dict]:
        if table not in self._cache:
            self._cache[table] = self.q(f"SELECT * FROM {table} ORDER BY {order}")
        return self._cache[table]

    def count(self, table: str) -> int:
        return int(self.src.scalar(f"SELECT count(*) FROM {self.schema}.{table}"))


def archived(plan: Plan, ctx: Ctx, table: str, reason: str, status: str = "archived") -> None:
    plan.mapping[table] = (status, ctx.count(table), reason)


# ---------------------------------------------------------------------------
# phases (one per Go module database)
# ---------------------------------------------------------------------------

PHASES: dict[str, tuple[str, Callable[[Ctx], Plan]]] = {}
# phase -> default target database name (lmm_<x>; roles lmm_<x>_installer / lmm_<x>_owner)


def phase(name: str, database: str):
    def deco(fn):
        PHASES[name] = (database, fn)
        return fn
    return deco


# === PHASE: store ===
# Store/support records are JSON bodies keyed by Go store.ID(). The legacy-web
# shop of a personal seller is ID("legacy-web-shop-v1", {"kind":"personal","id":N}).
# Legacy secrets (stock items, delivered contents, support messages) used
# AES-256-GCM with key sha256("merchant-store:<kind>:<id>:" + secret), where the
# secret is MERCHANT_STORE_ENCRYPTION_KEY (fallback CRYPTO_SECRET). Without that
# secret (--store-key-file), ciphertext rows are archived, never fabricated.

def go_json(value: Any) -> str:
    """encoding/json bytes for ID() hashing (HTML-safe escapes, insertion order)."""
    raw = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    return (raw.replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
            .replace("\u2028", "\\u2028").replace("\u2029", "\\u2029"))


def go_id(*parts: Any) -> str:
    return hashlib.sha256(go_json(list(parts)).encode()).hexdigest()


def go_time(unix: int) -> str:
    from datetime import datetime, timezone
    return datetime.fromtimestamp(unix, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def personal(uid: int) -> dict:
    return {"kind": "personal", "id": uid}


def legacy_shop_id(uid: int) -> str:
    return go_id("legacy-web-shop-v1", personal(uid))


def legacy_secret(args, *names: str) -> str | None:
    """First non-empty KEY from --legacy-secrets-env (KEY=VALUE lines, 0600)."""
    path = getattr(args, "legacy_secrets_env", None)
    if not path:
        return None
    if os.stat(path).st_mode & 0o077:
        fail("legacy_secrets_env_not_private")
    values = {}
    for line in Path(path).read_text().splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            k, v = line.split("=", 1)
            values[k.strip()] = v.strip().strip('"').strip("'")
    return next((values[n] for n in names if values.get(n)), None)


def store_secret(args) -> str | None:
    path = getattr(args, "store_key_file", None)
    if not path:
        return legacy_secret(args, "MERCHANT_STORE_ENCRYPTION_KEY", "CRYPTO_SECRET")
    secret = Path(path).read_text().strip()
    if len(secret) < 32:
        fail("store_key_too_short")
    return secret


def store_decrypt(secret: str, kind: str, ident: str, value: str) -> str:
    import base64
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    value = (value or "").strip()
    if not value:
        return ""
    if not value.startswith("v1:"):
        fail("store_cipher_envelope")
    raw = base64.urlsafe_b64decode(value[3:] + "=" * (-len(value[3:]) % 4))
    key = hashlib.sha256(f"merchant-store:{kind}:{ident}:{secret}".encode()).digest()
    return AESGCM(key).decrypt(raw[:12], raw[12:], None).decode()


def jlist(text: str | None) -> list:
    value = json.loads(text) if text else []
    return value if isinstance(value, list) else []


def legacy_variant_view(v: dict, default_id: str) -> dict:
    # store.legacyVariantView with ready=false (snapshot form)
    m = v["legacy"]
    unlimited = not v["track_stock"]
    available = -1 if unlimited else v["stock"] - v["reserved"]
    sale = available
    if v["quota"] >= 0 and (sale < 0 or sale > v["quota"] - v["claimed"]):
        sale = v["quota"] - v["claimed"]
    if sale < 0 and v["quota"] >= 0:
        sale = 0
    return {"id": v["id"], "product_id": v["product_id"], "name": v["title"], "price_quota": v["price"]["amount_credits"],
            "template": m["template"], "enabled": m["enabled"], "created_at": m["created_at"], "updated_at": m["updated_at"],
            "is_default": v["id"] == default_id, "unlimited_supply": unlimited, "inventory_total": v["stock"],
            "inventory_available": available, "reserved_stock": v["reserved"], "sale_available": sale, "trading_paused": True}


def legacy_public_snapshot(product: dict, owner: int, variants: list[dict]) -> dict:
    # store.snapshotLegacy: legacyProductView(public=false, ready=false), review fields cleared
    m = product["legacy"]
    price, lo, hi, stock, total, consumed, unlimited = 0, -1, 0, 0, 0, 0, False
    views = []
    for v in variants:
        amount = v["price"]["amount_credits"]
        consumed += v["claimed"]
        if v["id"] == m["default_variant_id"]:
            price = amount
        if v["legacy"]["enabled"]:
            lo = amount if lo < 0 or amount < lo else lo
            hi = max(hi, amount)
            unlimited = unlimited or not v["track_stock"]
        if v["track_stock"]:
            stock += v["stock"] - v["reserved"]
            total += v["stock"]
        views.append(legacy_variant_view(v, m["default_variant_id"]))
    lo = price if lo < 0 else lo
    sale = -1 if unlimited else stock
    if m.get("sale_limit") is not None:
        remaining = max(0, m["sale_limit"] - consumed)
        if sale < 0 or sale > remaining:
            sale = remaining
    tags = ["trading_paused"] if (unlimited or stock > 0) else ["out_of_stock"]
    return {"id": product["id"], "seller_id": owner, "category_id": m.get("category_id", ""), "title": product["title"],
            "description": m["description"], "image_urls": m["image_urls"], "contact": m["contact"], "links": m["links"],
            "price_quota": price, "test_mode": m["test_mode"], "visibility": m["visibility"],
            "purchase_login_required": m["purchase_login_required"], "sale_limit": m.get("sale_limit"),
            "max_quantity_per_order": m["max_quantity_per_order"], "max_quantity_per_buyer": m["max_quantity_per_buyer"],
            "template": m["template"], "delivery_strategy": m["delivery_strategy"], "payment_methods": [],
            "pickup_login_required": m["pickup_login_required"], "pickup_code_required": m["pickup_code_required"],
            "email_pickup_link": m["email_pickup_link"], "status": m["status"], "review_note": "", "reviewed_by": 0,
            "reviewed_at": m["reviewed_at"], "promotion_expires_at": 0, "created_at": m["created_at"],
            "updated_at": m["updated_at"], "official": False, "unlimited_supply": unlimited, "available_stock": stock,
            "paid_quantity": 0, "reserved_quantity": 0, "sale_available": sale, "trading_paused": True,
            "default_variant_id": m["default_variant_id"], "variants": views, "inventory_total": total,
            "inventory_available": stock, "price_min_quota": lo, "price_max_quota": hi, "display_tags": tags}


def put(table: str, shop: str, ident: str, body: dict) -> str:
    return f"INSERT INTO {table}(shop_id,id,body) VALUES({lit(shop)},{lit(ident)},{jlit(body)});"


@phase("store", "lmm_store")
def phase_store(ctx: Ctx) -> Plan:
    plan = Plan()
    secret = store_secret(ctx.args)
    products = ctx.rows("merchant_store_products", "created_at,id")
    meta = {r["product_id"]: r for r in ctx.rows("merchant_store_catalogue_metadata", "product_id")}
    legacy_variants = ctx.rows("merchant_store_variants", "created_at,id")
    orders = ctx.rows("merchant_store_orders", "created_at,id")
    refunds = {r["order_id"]: r for r in ctx.rows("merchant_store_refunds", "created_at,id")}
    stocks = ctx.rows("merchant_store_stocks", "created_at,id")
    if len(refunds) != ctx.count("merchant_store_refunds"):
        fail("store_multiple_refunds_per_order")
    by_product = {p["id"]: p for p in products}
    shops: dict[int, str] = {}
    variants: dict[str, dict] = {}
    product_bodies: dict[str, dict] = {}
    for p in products:
        uid = p["seller_id"]
        shops.setdefault(uid, legacy_shop_id(uid))
        default_id = (meta.get(p["id"]) or {}).get("default_variant_id")
        if not default_id:
            fail(f"store_default_variant_missing:{p['id']}")
        visibility = p["visibility"] or "public"
        m = {"description": p["description"] or "", "image_urls": jlist(p["image_urls"]), "contact": p["contact"] or "",
             "links": [{"title": l.get("title", ""), "url": l.get("url", ""), "description": l.get("description", "")}
                       for l in jlist(p["links"])],
             "test_mode": bool(p["test_mode"]), "visibility": visibility,
             "purchase_login_required": True if p["purchase_login_required"] is None else p["purchase_login_required"],
             "max_quantity_per_order": p["max_quantity_per_order"], "max_quantity_per_buyer": p["max_quantity_per_buyer"],
             "template": p["template"], "delivery_strategy": p["delivery_strategy"] or "sequential",
             "payment_methods": jlist(p["payment_methods"]), "pickup_login_required": bool(p["pickup_login_required"]),
             "pickup_code_required": bool(p["pickup_code_required"]), "email_pickup_link": bool(p["email_pickup_link"]),
             "status": p["status"], "review_note": p["review_note"] or "", "reviewed_by": p["reviewed_by"] or 0,
             "reviewed_at": p["reviewed_at"] or 0, "approved": p["status"] == "published",
             "created_at": p["created_at"], "updated_at": p["updated_at"], "default_variant_id": default_id,
             "sale_limit": p["sale_limit"]}
        if p["category_id"]:
            m["category_id"] = p["category_id"]
        product_bodies[p["id"]] = {"id": p["id"], "title": p["title"], "published": p["status"] == "published", "legacy": m}
        own = [v for v in legacy_variants if v["product_id"] == p["id"]]
        if not any(v["id"] == default_id for v in own):
            # Pre-variant products had one implicit default variant.
            own.insert(0, {"id": default_id, "product_id": p["id"], "name": "Default", "price_quota": p["price_quota"],
                           "template": p["template"], "enabled": True, "created_at": p["created_at"],
                           "updated_at": p["updated_at"]})
            plan.stats["implicit_default_variants"] += 1
        for v in own:
            # Per-unit legacy stock has no new backend (only fixed content);
            # tracked stock 0 keeps these card-key variants sold out.
            variants[v["id"]] = {"id": v["id"], "product_id": p["id"], "title": v["name"],
                                 "price": {"amount_credits": v["price_quota"], "unit": "credits"},
                                 "track_stock": True, "stock": 0, "reserved": 0, "quota": -1, "claimed": 0,
                                 "legacy": {"template": v["template"], "enabled": v["enabled"],
                                            "created_at": v["created_at"], "updated_at": v["updated_at"]}}
    order_bodies = []
    for o in orders:
        product = by_product[o["product_id"]]
        variant_id = o["variant_id"] or product_bodies[o["product_id"]]["legacy"]["default_variant_id"]
        v = variants[variant_id]
        if o["price_quota"] != o["unit_price_quota"] * o["quantity"] or o["payment_method"] != "balance" or o["amount_minor"]:
            fail(f"store_order_shape:{o['id']}")
        delivered = (o["claimed_at"] or 0) > 0
        if o["status"] == "paid":
            state = "settled"  # legacy balance payment credited the seller at purchase
        elif o["status"] == "refunded":
            state = "refunded"
        else:
            fail(f"store_order_status:{o['status']}")
        if delivered:
            v["claimed"] += o["quantity"]
        legacy = {"product_id": o["product_id"], "payment_method": "balance", "created_at": o["created_at"],
                  "paid_at": o["paid_at"] or 0, "delivery_template": o["delivery_template"] or v["legacy"]["template"],
                  "pickup_code_required": bool(o["pickup_code_required"])}
        if o["seller_terms_version"]:
            legacy.update(seller_terms_version=o["seller_terms_version"], seller_terms_content=o["seller_terms_content"],
                          seller_terms_accepted_at=o["seller_terms_accepted_at"])
        if o["promotion_code"]:
            legacy.update(promotion_id=o["promotion_id"], promotion_code=o["promotion_code"],
                          discount_bps=o["discount_bps"], original_unit_price=o["original_price_quota"])
        body = {"id": o["id"], "buyer": personal(o["buyer_id"]), "seller": personal(o["seller_id"]),
                "created_by": o["buyer_id"], "variant_id": variant_id, "title": o["variant_name"] or v["title"],
                "product_title": o["product_title"], "quantity": o["quantity"],
                "unit_price": {"amount_credits": o["unit_price_quota"], "unit": "credits"},
                "total": {"amount_credits": o["price_quota"], "unit": "credits"}, "state": state,
                "request_hash": "legacy:" + (o["input_digest"] or o["id"]), "delivered": delivered, "legacy": legacy}
        if delivered and secret:
            items = [s for s in stocks if s["order_id"] == o["id"]]
            body["delivery"] = "\n".join(store_decrypt(secret, "stock", f"{s['product_id']}:{s['id']}", s["ciphertext"])
                                         for s in items)
            plan.stats["delivery_contents_decrypted"] += 1
        elif delivered:
            plan.stats["delivery_contents_not_decrypted"] += 1
        r = refunds.get(o["id"])
        if r:
            if r["status"] != "completed" or state != "refunded":
                fail(f"store_refund_state:{r['id']}")
            body.update(refund_state="completed", refund_reason=r["reason"] or "")
            body["legacy_refund"] = {"id": r["id"], "request_key": r["request_key"], "input_hash": r["input_digest"] or "",
                                     "reason": r["reason"] or "", "requested_by": r["requested_by"],
                                     "created_at": r["created_at"], "decision": "approve",
                                     "decision_by": r["decision_by"] or 0, "decision_reason": r["decision_reason"] or "",
                                     "decided_at": r["decided_at"] or 0, "completed_at": r["completed_at"] or 0}
        order_bodies.append((legacy_shop_id(o["seller_id"]), body))
    for uid, shop in shops.items():
        plan.sql.append(put("lmm_store.shops", shop, shop, {"id": shop, "owner": personal(uid), "title": "Merchant store"}))
    for pid, body in product_bodies.items():
        uid = by_product[pid]["seller_id"]
        own = sorted((v for v in variants.values() if v["product_id"] == pid), key=lambda v: v["id"])
        body["legacy_public"] = legacy_public_snapshot(body, uid, own)
        body["legacy_public_version"] = 1
        plan.sql.append(put("lmm_store.products", shops[uid], pid, body))
        for v in own:
            plan.sql.append(put("lmm_store.variants", shops[uid], v["id"], v))
    for shop, body in order_bodies:
        plan.sql.append(put("lmm_store.orders", shop, body["id"], body))
    commands = 0
    for t in ctx.rows("merchant_store_seller_terms", "seller_id"):
        if t["seller_id"] not in shops:
            shops[t["seller_id"]] = legacy_shop_id(t["seller_id"])
            plan.sql.append(put("lmm_store.shops", shops[t["seller_id"]], shops[t["seller_id"]],
                                {"id": shops[t["seller_id"]], "owner": personal(t["seller_id"]), "title": "Merchant store"}))
        plan.sql.append(put("lmm_store.commands", shops[t["seller_id"]], "@web:seller-terms",
                            {"version": t["version"], "content": t["content"].strip(), "updated_at": t["updated_at"]}))
        commands += 1
    for a in ctx.rows("merchant_store_terms_acceptances", "accepted_at,id"):
        if a["kind"] != "seller" or not a["subject"].startswith("user:"):
            plan.stats["terms_acceptance_unsupported_subject"] += 1
            continue
        uid = int(a["subject"][5:])
        plan.sql.append(put("lmm_store.commands", shops[a["seller_id"]], go_id("legacy-seller-terms-v1", uid, a["version"]),
                            {"version": a["version"], "user_id": uid, "accepted_at": a["accepted_at"]}))
        commands += 1
    plan.expect = {"lmm_store.shops": len(shops), "lmm_store.products": len(product_bodies),
                   "lmm_store.variants": len(variants), "lmm_store.orders": len(order_bodies),
                   "lmm_store.commands": commands}
    plan.stats.update(shops=len(shops), products=len(product_bodies), variants=len(variants), orders=len(order_bodies))
    M = plan.mapping
    M["merchant_store_products"] = ("lmm_store.products", len(products), "legacy metadata + legacy_public snapshot; deleted kept as status=deleted")
    M["merchant_store_variants"] = ("lmm_store.variants", len(legacy_variants), f"plus {plan.stats['implicit_default_variants']} implicit default variants from catalogue_metadata; credits prices")
    M["merchant_store_catalogue_metadata"] = ("lmm_store.products legacy.default_variant_id", len(meta), "custom_tags all empty; auto_delivery/ai_processing all false (no filter loss)")
    M["merchant_store_orders"] = ("lmm_store.orders", len(orders), "paid->settled, refunded->refunded; read-only history, no money_operations")
    M["merchant_store_refunds"] = ("lmm_store.orders legacy_refund", len(refunds), "one completed full refund per order")
    archived(plan, ctx, "merchant_store_refund_items", "stock-level refund evidence; new store refunds whole orders, items stay in snapshot")
    archived(plan, ctx, "merchant_store_stocks",
             "per-unit card-key stock has no new backend (fixed content only); available units all belong to deleted products; "
             + ("delivered contents copied into order.delivery" if secret else "ciphertext needs MERCHANT_STORE_ENCRYPTION_KEY/CRYPTO_SECRET (not available locally)"),
             "missing_backend")
    M["merchant_store_seller_terms"] = ("lmm_store.commands @web:seller-terms", ctx.count("merchant_store_seller_terms"), "")
    M["merchant_store_terms_acceptances"] = ("lmm_store.commands legacy-seller-terms-v1", ctx.count("merchant_store_terms_acceptances"), "")
    archived(plan, ctx, "merchant_store_promotions", "paid placement purchases (months/quota); new store has no paid placement (promotion_quota=0); charges are Core money history", "missing_backend")
    archived(plan, ctx, "merchant_store_product_likes", "store_likes_supported=false in new store", "missing_backend")
    archived(plan, ctx, "merchant_store_product_traffic_days", "analytics; no traffic backend in new store", "missing_backend")
    archived(plan, ctx, "merchant_store_product_traffic_receipts", "analytics dedupe receipts; no backend", "missing_backend")
    archived(plan, ctx, "merchant_store_events", "legacy audit log; kept in snapshot")
    archived(plan, ctx, "merchant_store_transfers", "seller money transfers = Core money history (legacy_cutover money phases), Go never holds balances")
    archived(plan, ctx, "merchant_store_gateways", "merchant external payment gateways (encrypted provider configs) unsupported: platform_payment_methods=[]", "missing_backend")
    archived(plan, ctx, "merchant_store_configs", "global fee/promotion settings; new store PublicConfig is compiled (fee_bps 0, min 500000)")
    archived(plan, ctx, "merchant_store_email_deliveries", "sent pickup e-mail log; email pickup unsupported")
    return plan
# === END PHASE: store ===

# === PHASE: support ===
@phase("support", "lmm_support")
def phase_support(ctx: Ctx) -> Plan:
    plan = Plan()
    secret = store_secret(ctx.args)
    orders = {o["id"]: o for o in ctx.rows("merchant_store_orders", "id")}
    messages = ctx.rows("merchant_store_support_messages", "id")
    customers: dict[tuple[str, int], dict] = {}
    conversations, migrated_messages = 0, 0
    for c in ctx.rows("merchant_store_support_conversations", "created_at,id"):
        shop = legacy_shop_id(c["seller_id"])
        buyer = personal(c["buyer_id"])
        if not c["order_id"] or c["order_id"] not in orders:
            plan.stats["conversation_without_order_skipped"] += 1  # none in this snapshot
            continue
        o = orders[c["order_id"]]
        # Same identity the web compatibility layer derives for an order chat,
        # so reopening from the order reaches this conversation.
        key = go_id("web-order-support", c["order_id"])
        request = {"shop_id": shop, "buyer": buyer, "order_id": c["order_id"], "subject": o["product_title"], "key": key}
        cid = go_id("conversation", c["buyer_id"], buyer, key)
        customer_id = go_id("customer", buyer)
        customers.setdefault((shop, c["buyer_id"]), {"id": customer_id, "buyer": buyer, "notes": "", "tags": [],
                                                     "created_at": go_time(c["created_at"]), "revision": 1})
        own = [m for m in messages if m["conversation_id"] == c["id"]]
        sequence = 0
        for m in own if secret else []:
            sequence += 1
            text = store_decrypt(secret, "support-message", f"{c['id']}:{m['request_key']}:{m['sender_id']}", m["ciphertext"])
            party = personal(c["seller_id"]) if m["sender_id"] == c["seller_id"] else buyer
            plan.sql.append(put("lmm_support.messages", shop, f"{cid}:{sequence:020d}",
                                {"numeric_id": sequence, "id": f"{cid}:{sequence:020d}", "conversation_id": cid,
                                 "author_user_id": m["sender_id"], "author_account": party, "body": text,
                                 "created_at": go_time(m["created_at"])}))
            migrated_messages += 1
        body = {"shop_id": shop, "product_id": c["product_id"], "id": cid, "buyer": buyer,
                "seller": personal(c["seller_id"]), "customer_id": customer_id, "order_id": c["order_id"],
                "subject": o["product_title"], "closed": c["status"] != "open", "sequence": sequence,
                "request_hash": go_id(request), "created_at": go_time(c["created_at"]),
                "updated_at": go_time(c["updated_at"])}
        if sequence:
            body.update(last_message_id=sequence, buyer_read_id=min(c["buyer_read_id"], sequence),
                        seller_read_id=min(c["seller_read_id"], sequence))
        plan.sql.append(put("lmm_support.conversations", shop, cid, body))
        conversations += 1
    for (shop, _), customer in customers.items():
        plan.sql.append(put("lmm_support.customers", shop, customer["id"], customer))
    plan.expect = {"lmm_support.customers": len(customers), "lmm_support.conversations": conversations,
                   "lmm_support.messages": migrated_messages}
    plan.stats.update(conversations=conversations, messages=migrated_messages, customers=len(customers))
    plan.mapping["merchant_store_support_conversations"] = ("lmm_support.conversations (+customers)", conversations,
                                                            "id re-derived as web order-support conversation")
    if secret:
        plan.mapping["merchant_store_support_messages"] = ("lmm_support.messages", migrated_messages, "decrypted")
    else:
        archived(plan, ctx, "merchant_store_support_messages",
                 "body ciphertext needs MERCHANT_STORE_ENCRYPTION_KEY/CRYPTO_SECRET (rerun with --store-key-file)")
    archived(plan, ctx, "merchant_store_customers", "seller customer notes (0 rows)")
    return plan
# === END PHASE: support ===

# === PHASE: operations ===
@phase("operations", "")
def phase_operations(ctx: Ctx) -> Plan:
    # lmm_operations owns release notes and their read receipts only.
    plan = Plan()
    for table in ("release_notes", "release_note_reads"):
        n = ctx.count(table)
        if n:
            fail(f"operations_rows_need_mapping:{table}")
        plan.mapping[table] = ("lmm_operations.publications" if table == "release_notes" else "lmm_operations.reads", 0,
                               "empty in legacy; nothing to import")
    return plan
# === END PHASE: operations ===

# === PHASE: toolmarket ===
# Target: the single AES-256-GCM sealed JSON row lmm_toolmarket.snapshot (Go
# toolmarket/postgres.go encryptSnapshot: nonce(12)||Seal(json, AAD
# "lmm-toolmarket-pg-v1")). install-db already wrote the pristine version=1
# empty record; this phase replaces exactly that record once (version 1 -> 2),
# so "imported" is counted as snapshot rows with version>1.
TM_AAD = b"lmm-toolmarket-pg-v1"
TM_IMPORTED = "lmm_toolmarket.snapshot WHERE version>1"
TM_ID = __import__("re").compile(r"^[A-Za-z0-9_-]{1,80}$")
TM_CATALOG = (Path(__file__).resolve().parent.parent
              / "services/extensions/internal/modules/toolmarket/builtin_catalog.json")
# Optional: file holding the legacy TOOL_MARKET_ENCRYPTION_KEY (or CRYPTO_SECRET
# fallback) env value of the old api-go process. Without it credentials stay out.
TM_LEGACY_SECRET_ENV = "LMM_LEGACY_TOOL_MARKET_SECRET_FILE"


def _tm_go_json(value: Any) -> str:
    """encoding/json.Marshal spelling for the plain values used in Go digest()."""
    s = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    for a, b in (("<", "\\u003c"), (">", "\\u003e"), ("&", "\\u0026"),
                 (" ", "\\u2028"), (" ", "\\u2029")):
        s = s.replace(a, b)
    return s


def _tm_digest(value: Any) -> str:
    return hashlib.sha256(_tm_go_json(value).encode()).hexdigest()


def _tm_private_bytes(path: str, size: int | None = None) -> bytes:
    p = Path(path)
    st = p.lstat()
    if not p.is_file() or p.is_symlink() or st.st_mode & 0o077:
        fail("toolmarket_private_file_mode")
    raw = p.read_bytes()
    if size is not None and len(raw) != size:
        fail("toolmarket_key_not_32_bytes")
    return raw


def _tm_aead(ctx: Ctx):
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    if not ctx.args.toolmarket_key_file:
        fail("toolmarket_key_file_required")
    return AESGCM(_tm_private_bytes(ctx.args.toolmarket_key_file, 32))


def _tm_legacy_decrypt(secret: str, row: dict) -> str:
    """Legacy common.DecryptPersistentString: v1:base64url(nonce||ct), key =
    sha256(purpose + ":" + env secret), purpose = marketCredentialPurpose(row)."""
    import base64
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    purpose = "tool_market.credential:%s:%d:%s:%s:%s:%s" % (
        row["id"], row["owner_id"], row["service_id"], row["version_id"], row["endpoint_digest"], row["mode"])
    key = hashlib.sha256((purpose.strip() + ":" + secret).encode()).digest()
    text = row["ciphertext"].strip()
    if not text.startswith("v1:"):
        fail("toolmarket_legacy_credential_envelope")
    raw = base64.urlsafe_b64decode(text[3:] + "=" * (-len(text[3:]) % 4))
    return AESGCM(key).decrypt(raw[:12], raw[12:], None).decode()


def _tm_valid_secret(mode: str, secret: str) -> bool:
    """toolmarket validWebCredential (non-'none' modes)."""
    import unicodedata
    if mode not in ("bearer", "api_key") or not secret or len(secret.encode()) > 4096 or not secret.strip():
        return False
    for c in secret:
        if unicodedata.category(c) == "Cc" or (mode == "bearer" and c.isspace()):
            return False
    return not secret.strip().startswith(("lmm_at_", "lmm_rt_", "lmm_market_", "lmm_mcp_", "lmm_drawing_mcp_",
                                          "lmm_bounty_mcp_", "lmms_", "lmmk_", "lmmt_"))


def _tm_json_list(raw: Any) -> list:
    if raw in (None, "", "null"):
        return []
    value = json.loads(raw)
    if not isinstance(value, list):
        fail("toolmarket_legacy_json_list")
    return value


@phase("toolmarket", "lmm_toolmarket")
def phase_toolmarket(ctx: Ctx) -> Plan:
    plan = Plan()
    aead = _tm_aead(ctx)
    catalog = json.loads(TM_CATALOG.read_text())
    compiled_services = {d["service_id"] for d in catalog}

    services = ctx.rows("tool_market_services", "created_at, id")
    versions = ctx.rows("tool_market_versions", "created_at, id")
    tools = {t["id"]: t for t in ctx.rows("tool_market_tools", "id")}
    tool_versions = ctx.rows("tool_market_tool_versions", "version_id, tool_id")

    # Owner 0 rows are the legacy compiled builtins. The new module overlays the
    # same service IDs from its embedded catalog (new immutable version IDs and
    # schemas), so the legacy builtin services/versions are not stored.
    builtin_services = {s["id"] for s in services if s["owner_id"] == 0}
    if not builtin_services <= compiled_services:
        fail("toolmarket_unknown_builtin_service")
    version_service = {v["id"]: v["service_id"] for v in versions}
    builtin_versions = {v["id"] for v in versions if v["service_id"] in builtin_services}

    web_services: dict[str, dict] = {}
    for s in services:
        if s["id"] in builtin_services:
            continue
        for key in ("id", "live_version_id", "draft_version_id"):
            if s[key] and not TM_ID.match(s[key]):
                fail(f"toolmarket_bad_id:{s[key]}")
        if s["owner_id"] <= 0:
            fail("toolmarket_service_owner")
        web_services[s["id"]] = {
            "id": s["id"], "owner_id": s["owner_id"], "live_version_id": s["live_version_id"] or "",
            "draft_version_id": s["draft_version_id"] or "", "status": s["status"],
            "created_at": s["created_at"], "updated_at": s["updated_at"]}
        if s["status"] != "deleted" and s["draft_version_id"] and s["draft_version_id"] not in version_service:
            fail(f"toolmarket_missing_draft:{s['id']}")
        if s["draft_version_id"] and s["draft_version_id"] not in version_service:
            plan.stats["deleted_service_dangling_draft_ref"] += 1

    by_version: dict[str, list[dict]] = {}
    for tv in tool_versions:
        by_version.setdefault(tv["version_id"], []).append(tv)

    web_versions: dict[str, dict] = {}
    for v in versions:
        if v["id"] in builtin_versions:
            continue
        if v["service_id"] not in web_services or not TM_ID.match(v["id"]):
            fail(f"toolmarket_orphan_version:{v['id']}")
        wtools, remote = [], {}
        for tv in by_version.get(v["id"], []):
            t = tools.get(tv["tool_id"])
            if t is None or t["service_id"] != v["service_id"] or t["name"] != tv["name"] or not TM_ID.match(tv["tool_id"]):
                fail(f"toolmarket_tool_mismatch:{tv['tool_id']}")
            wt: dict[str, Any] = {
                "tool_id": tv["tool_id"], "version_id": v["id"], "name": tv["name"],
                "description": tv["description"] or "", "input_schema": tv["input_schema"] or "",
                "output_schema": "" if tv["output_schema"] in (None, "null") else tv["output_schema"],
                "permissions": tv["permissions"] or "[]", "price_quota": tv["price_quota"] or 0}
            if tv["billing_mode"]:
                wt["billing_mode"] = tv["billing_mode"]
            if tv["input_token_price_quota"]:
                wt["input_token_price_quota"] = tv["input_token_price_quota"]
            if tv["max_input_tokens"]:
                wt["max_input_tokens"] = tv["max_input_tokens"]
            rules = _tm_json_list(tv["billing_rules"])
            if rules:
                wt["billing_rules"] = rules
            if tv["provider_pricing"] not in (None, "", "null"):
                pp = json.loads(tv["provider_pricing"])
                if set(pp) != {"provider", "multiplier"}:
                    fail("toolmarket_provider_pricing_shape")
                wt["provider_pricing"] = pp
                plan.stats["provider_priced_tools"] += 1
            if tv["remote_digest"]:
                remote[tv["tool_id"]] = tv["remote_digest"]
            wtools.append(wt)
        allowed = [int(x) for x in _tm_json_list(v["allowed_users"])]
        snap: dict[str, Any] = {
            "version": {"id": v["id"], "service_id": v["service_id"], "status": v["status"], "name": v["name"],
                        "description": v["description"] or "", "execution_type": v["execution_type"],
                        "visibility": v["visibility"], "endpoint": v["endpoint"] or "", "digest": v["digest"] or "",
                        "reviewed_by": v["reviewed_by"] or 0, "review_note": v["review_note"] or "",
                        "created_at": v["created_at"], "published_at": v["published_at"] or 0},
            "tools": wtools, "allowed_users": allowed,
            # Legacy digests are carried verbatim: Go only compares them for equality.
            "digest": v["digest"] or "", "validation_digest": v["validation_digest"] or ""}
        if remote:
            snap["remote_digests"] = remote
        web_versions[v["id"]] = snap
        plan.stats["tool_versions_imported"] += len(wtools)
        if v["ai_review_token"]:
            plan.stats["ai_review_token_dropped"] += 1
    for s in web_services.values():
        if s["status"] != "deleted" and s["live_version_id"] and s["live_version_id"] not in web_versions:
            fail(f"toolmarket_missing_live:{s['id']}")

    installations: dict[str, dict] = {}
    for i in ctx.rows("tool_market_installations", "created_at, user_id, client_id, tool_id"):
        if i["version_id"] in builtin_versions:
            plan.stats["installations_builtin_archived"] += 1
            continue
        snap = web_versions.get(i["version_id"])
        if snap is None or i["tool_id"] not in {t["tool_id"] for t in snap["tools"]}:
            fail(f"toolmarket_installation_ref:{i['tool_id']}")
        installations[_tm_digest([i["user_id"], i["client_id"], i["tool_id"]])] = {
            "user_id": i["user_id"], "client_id": i["client_id"], "tool_id": i["tool_id"],
            "version_id": i["version_id"], "created_at": i["created_at"]}

    credentials: dict[str, dict] = {}
    cred_rows = ctx.rows("tool_market_credentials", "updated_at, id")
    secret_file = os.environ.get(TM_LEGACY_SECRET_ENV)
    legacy_key = (legacy_secret(ctx.args, "TOOL_MARKET_ENCRYPTION_KEY", "CRYPTO_SECRET")
                  or (_tm_private_bytes(secret_file).decode().strip() if secret_file else ""))
    for c in cred_rows:
        snap = web_versions.get(c["version_id"])
        if snap is None or snap["version"]["service_id"] != c["service_id"]:
            fail(f"toolmarket_credential_ref:{c['id']}")
        if _tm_digest(snap["version"]["endpoint"]) == c["endpoint_digest"]:
            plan.stats["credential_endpoint_digest_go_compatible"] += 1
        if not legacy_key:
            continue
        secret = _tm_legacy_decrypt(legacy_key, c)
        if not _tm_valid_secret(c["mode"], secret):
            fail(f"toolmarket_credential_invalid:{c['id']}")
        credentials[c["version_id"]] = {
            "owner_id": c["owner_id"], "service_id": c["service_id"], "version_id": c["version_id"],
            "endpoint_digest": _tm_digest(snap["version"]["endpoint"]), "mode": c["mode"],
            "secret": secret, "updated_at": c["updated_at"]}

    web: dict[str, Any] = {"services": web_services, "versions": web_versions,
                           "installations": installations, "favorites": {}}
    if credentials:
        web["credentials"] = credentials
    snapshot = {"version": 1, "servers": {}, "releases": {}, "installs": {}, "secrets": {}, "states": {},
                "executions": {}, "audit": [], "web_compat": web}
    plain = json.dumps(snapshot, ensure_ascii=False, separators=(",", ":")).encode()
    if len(plain) > 63 << 20:
        fail("toolmarket_snapshot_too_large")
    nonce = os.urandom(12)
    sealed = nonce + aead.encrypt(nonce, plain, TM_AAD)
    plan.sql = [
        "DO $$BEGIN IF (SELECT count(*) FROM lmm_toolmarket.snapshot)<>1 OR "
        "(SELECT count(*) FROM lmm_toolmarket.snapshot WHERE id=1 AND version=1)<>1 THEN "
        "RAISE EXCEPTION 'toolmarket snapshot is not the pristine install record'; END IF; END$$;",
        f"UPDATE lmm_toolmarket.snapshot SET version=2,ciphertext={lit(chr(92) + 'x' + sealed.hex())}::bytea "
        "WHERE id=1 AND version=1;",
    ]
    plan.expect = {TM_IMPORTED: 1}
    want = {"services": len(web_services), "versions": len(web_versions), "installations": len(installations),
            "credentials": len(credentials), "tools": sum(len(v["tools"]) for v in web_versions.values())}
    plan.stats.update({f"snapshot_{k}": n for k, n in want.items()})
    plan.stats["snapshot_plain_bytes"] = len(plain)

    def verify(_ctx: Ctx, db: Db) -> None:
        raw = bytes.fromhex(db.scalar("SELECT encode(ciphertext,'hex') FROM lmm_toolmarket.snapshot WHERE id=1"))
        got = json.loads(aead.decrypt(raw[:12], raw[12:], TM_AAD))
        w = got["web_compat"]
        have = {"services": len(w["services"]), "versions": len(w["versions"]),
                "installations": len(w["installations"]), "credentials": len(w.get("credentials", {})),
                "tools": sum(len(v["tools"]) for v in w["versions"].values())}
        if got != snapshot or have != want:
            fail(f"toolmarket_snapshot_verify:{have}")
        print(f"toolmarket: decrypted snapshot verified {have}")
    plan.after = verify

    n_cred = len(cred_rows)
    plan.mapping.update({
        "tool_market_services": ("lmm_toolmarket.snapshot web_compat.services", ctx.count("tool_market_services"),
                                 f"{len(web_services)} imported; {len(builtin_services)} owner-0 builtin services "
                                 "not stored (Go embedded builtin catalog overlays the same service IDs)"),
        "tool_market_versions": ("lmm_toolmarket.snapshot web_compat.versions", ctx.count("tool_market_versions"),
                                 f"{len(web_versions)} imported with legacy digest/validation_digest; "
                                 f"{len(builtin_versions)} builtin versions superseded by compiled catalog; "
                                 "ai_review_token dropped"),
        "tool_market_tools": ("lmm_toolmarket.snapshot web_compat.versions[].tools[].tool_id", ctx.count("tool_market_tools"),
                              "tool identity folded into each version's tool list (same tool_id)"),
        "tool_market_tool_versions": ("lmm_toolmarket.snapshot web_compat.versions[].tools", ctx.count("tool_market_tool_versions"),
                                      f"{want['tools']} imported (remote_digest -> remote_digests); "
                                      f"{ctx.count('tool_market_tool_versions') - want['tools']} builtin rows superseded"),
        "tool_market_installations": ("lmm_toolmarket.snapshot web_compat.installations", ctx.count("tool_market_installations"),
                                      f"{len(installations)} imported; {plan.stats['installations_builtin_archived']} "
                                      "builtin installs archived (pinned to superseded builtin versions; no silent upgrade)"),
        "tool_market_credentials": (("lmm_toolmarket.snapshot web_compat.credentials" if credentials else "not_migrated"),
                                    n_cred, ("decrypted and re-sealed in snapshot" if credentials else
                                             "legacy AES-GCM key = sha256(purpose:TOOL_MARKET_ENCRYPTION_KEY|CRYPTO_SECRET) "
                                             f"of the old api-go env not supplied; pass --legacy-secrets-env or {TM_LEGACY_SECRET_ENV}")),
    })
    archived(plan, ctx, "tool_market_grants",
             "Core-owned consents (core_tool_market.credit_consents need Core quote IDs that legacy rows lack); "
             "all legacy grants are revoked or expired")
    archived(plan, ctx, "tool_market_tokens", "Core-owned market tokens (credit_tokens); all 4 legacy tokens revoked")
    archived(plan, ctx, "tool_market_budgets",
             "Core-owned spending ceiling (credit_budgets); spent counter belongs to archived legacy calls; owner re-sets via Core")
    archived(plan, ctx, "tool_market_calls", "financial call history: Core credit_calls need Core consent/quote/journal "
             "rows; legacy quota effects are already in the Core opening balances")
    archived(plan, ctx, "tool_market_transfers", "financial history (all user 1 -> user 1); covered by Core opening balances")
    archived(plan, ctx, "tool_market_events", "legacy audit log (details JSON); Go audit has no details field")
    plan.mapping["tool_market_configs"] = ("core_sql: core_tool_market.credit_policy", ctx.count("tool_market_configs"),
                                           "fee/recipient emitted for the coordinator; enabled kept FALSE")
    for t in ("tool_market_accesses", "tool_market_favorites", "tool_market_results", "tool_market_reports",
              "tool_market_builtin_continuations"):
        plan.mapping[t] = ("empty", ctx.count(t), "no rows")

    for cfg in ctx.rows("tool_market_configs", "id"):
        plan.core_sql += [
            f"-- legacy tool_market_configs id={cfg['id']}: enabled={cfg['enabled']} fee_bps={cfg['fee_bps']} "
            f"recipient_id={cfg['recipient_id']} updated_by={cfg['updated_by']} updated_at={cfg['updated_at']}",
            "-- Run after identity import (users/personal accounts keep legacy IDs) and AFTER the Core first-owner",
            "-- bootstrap check (it requires a pristine credit_policy). Paid tool calls stay disabled: enabling is",
            "-- an explicit operator decision through the Core tool-market config action.",
            "UPDATE core_tool_market.credit_policy SET version=version+1,enabled=FALSE,"
            f"platform_user_id={int(cfg['recipient_id'])},platform_account_id={int(cfg['recipient_id'])},"
            f"fee_bps={int(cfg['fee_bps'])},updated_by={int(cfg['updated_by'])},updated_at={int(cfg['updated_at'])} "
            "WHERE singleton AND version=1 AND platform_user_id IS NULL;",
        ]
    return plan
# === END PHASE: toolmarket ===

# === PHASE: assistant ===
# Target: lmm_assistant (assistant schema v2 + delegation + agent + services v1).
# Legacy numeric IDs are kept (OVERRIDING SYSTEM VALUE) so cursors, receipts and
# support links stay stable; identity sequences are advanced past them.

ASSISTANT_PRESETS = ("ai_recommendation", "getting_started", "new_user_gift", "weekly_discount")


def _values(table: str, cols: str, rows: list[str], overriding: bool = False, chunk: int = 500) -> list[str]:
    ov = " OVERRIDING SYSTEM VALUE" if overriding else ""
    return [f"INSERT INTO lmm_assistant.{table}({cols}){ov} VALUES\n" + ",\n".join(rows[i:i + chunk]) + ";"
            for i in range(0, len(rows), chunk)]


def _hexlit(h: str) -> str:
    if len(h) != 64 or any(c not in "0123456789abcdef" for c in h):
        fail("assistant_bad_digest")
    return f"decode('{h}','hex')"


@phase("assistant", "lmm_assistant")
def phase_assistant(ctx: Ctx) -> Plan:
    plan = Plan()
    sql = plan.sql
    convs = ctx.rows("assistant_conversations", "id")
    msgs = ctx.rows("assistant_history_messages", "id")
    receipts = ctx.rows("assistant_turn_receipts", "turn_key")
    supports = ctx.rows("assistant_support_requests", "id")
    conv_owner = {c["id"]: c["user_id"] for c in convs}
    by_id = {m["id"]: m for m in msgs}
    by_seq = {(m["conversation_id"], m["sequence"]): m for m in msgs}

    # conversations: the legacy list (traditional ListAssistantConversationsPage)
    # only showed conversations with at least one history message, and v2 Go
    # never creates message-less shells for completed turns. Message-less legacy
    # shells (mostly empty "人工技术支持" handoff stubs) are therefore not
    # imported; their support requests keep a text reference only.
    with_messages = {m["conversation_id"] for m in msgs}
    rows = []
    for c in convs:
        if c["id"] not in with_messages:
            plan.stats["conversation_empty_not_imported"] += 1
            continue
        title, preview = c["title"], c["last_message_preview"] or ""
        if not (1 <= len(title) <= 120) or len(preview) > 120 or c["user_id"] <= 0 or c["restriction_reason"]:
            fail(f"assistant_conversation_shape:{c['id']}")
        if preview == "":
            preview = title
            plan.stats["conversation_preview_from_title"] += 1
        rows.append(f"({c['id']},{c['user_id']},{lit(title)},{lit(preview)},{c['created_at']},{c['updated_at']},"
                    f"{c['archived_at'] or 0},{c['restricted_at'] or 0})")
    sql += _values("conversations", "id,user_id,title,last_message_preview,created_at,updated_at,archived_at,restricted_at",
                   rows, overriding=True)
    plan.expect["lmm_assistant.conversations"] = len(rows)

    # history: v2 transcript only has user/assistant turns. Human-agent replies
    # move to platform_support_messages; the single empty secure_card marker
    # (its ciphertext used the legacy card key) is archived.
    rows = []
    for m in msgs:
        if m["role"] in ("user", "assistant"):
            if not (1 <= len(m["content"]) <= 4000):
                fail(f"assistant_message_shape:{m['id']}")
            rows.append(f"({m['id']},{m['conversation_id']},{m['sequence']},{lit(m['role'])},{lit(m['content'])},{m['created_at']})")
        else:
            plan.stats[f"history_role_{m['role']}"] += 1
    sql += _values("history_messages", "id,conversation_id,sequence,role,content,created_at", rows, overriding=True)
    plan.expect["lmm_assistant.history_messages"] = len(rows)

    # turn receipts: legacy turn_key = "<user_id>:<client_turn_id>"; input_digest
    # is sha256(user content). Receipts that point at an assistant answer become
    # v2 replay receipts; those pointing at a user message were human-support
    # sends and become platform_support_messages idempotency keys below.
    rows, support_user_msgs = [], []
    for r in receipts:
        user_s, _, turn = r["turn_key"].partition(":")
        user = int(user_s)
        m = by_id[r["message_id"]]
        if conv_owner[r["conversation_id"]] != user or m["conversation_id"] != r["conversation_id"] or not (1 <= len(turn) <= 64):
            fail(f"assistant_receipt_shape:{r['turn_key']}")
        if m["role"] == "assistant":
            u = by_seq.get((m["conversation_id"], m["sequence"] - 1))
            if not u or u["role"] != "user" or hashlib.sha256(u["content"].encode()).hexdigest() != r["input_digest"]:
                fail(f"assistant_receipt_digest:{r['turn_key']}")
            answer = hashlib.sha256(m["content"].encode()).hexdigest()
            rows.append(f"({user},{lit(turn)},{r['conversation_id']},{_hexlit(r['input_digest'])},{_hexlit(answer)})")
        elif m["role"] == "user" and r["support_request_id"]:
            support_user_msgs.append((r["support_request_id"], m, turn))
        else:
            fail(f"assistant_receipt_target:{r['turn_key']}")
    sql += _values("turn_receipts", "user_id,client_turn_id,conversation_id,input_hash,answer_hash", rows)
    plan.expect["lmm_assistant.turn_receipts"] = len(rows)

    # platform support: legacy handoffs bound to a local legacy conversation, not
    # a Core conversation (lmm_cv_...). core_conversation_id keeps an explicit
    # non-Core reference so no current Core conversation can match it.
    by_conv: dict[int, list[dict]] = {}
    for s in supports:
        by_conv.setdefault(s["conversation_id"], []).append(s)
    smsgs: dict[int, list[tuple[int, int, str, str, str, int]]] = {}
    for m in msgs:
        if m["role"] != "human":
            continue
        cands = by_conv.get(m["conversation_id"]) or fail(f"assistant_human_without_support:{m['id']}")
        prior = [s for s in cands if s["created_at"] <= m["created_at"]]
        s = max(prior, key=lambda s: (s["created_at"], s["id"])) if prior else min(cands, key=lambda s: s["id"])
        if not m["actor_user_id"] or not (1 <= len(m["content"]) <= 4000):
            fail(f"assistant_human_shape:{m['id']}")
        smsgs.setdefault(s["id"], []).append((m["id"], m["actor_user_id"], "human", m["content"], f"legacy-history-{m['id']}", m["created_at"]))
    for sid, m, turn in support_user_msgs:
        smsgs.setdefault(sid, []).append((m["id"], conv_owner[m["conversation_id"]], "user", m["content"], turn, m["created_at"]))
    rows, todo_rows, msg_rows = [], [], []
    todo_state = {"pending": "pending", "accepted": "assigned", "completed": "closed", "cancelled": "closed"}
    for s in supports:
        if s["kind"] not in ("handoff", "appointment") or s["status"] not in todo_state or conv_owner[s["conversation_id"]] != s["user_id"]:
            fail(f"assistant_support_shape:{s['id']}")
        closed = s["closed_at"] or None
        if (s["status"] in ("completed", "cancelled")) != (closed is not None):
            fail(f"assistant_support_closed:{s['id']}")
        mine = {x[0] for x in smsgs.get(s["id"], [])}
        closed_msg = s["closed_history_message_id"] if s["closed_history_message_id"] in mine else None
        admin = s["assigned_admin_id"] or None
        rows.append(f"({s['id']},{s['user_id']},{lit('legacy_assistant_conversation:' + str(s['conversation_id']))},"
                    f"{lit(s['kind'])},{lit(s['status'])},{lit(s['topic'] or '')},{lit(s['preferred_time'] or '')},{s['scheduled_at'] or 0},"
                    f"{lit(admin)},{lit(s['assigned_admin_name'] or '')},{s['created_at']},{s['updated_at']},{lit(s['accepted_at'] or None)},"
                    f"{lit(closed)},{lit(closed_msg)})")
        todo_rows.append(f"({s['id']},{lit(todo_state[s['status']])},{lit(admin if s['status'] != 'cancelled' else None)},{s['updated_at']})")
        for mid, author, kind, content, turn, at in sorted(smsgs.get(s["id"], [])):
            msg_rows.append(f"({mid},{s['id']},{author},{lit(kind)},{lit(content)},{lit(turn)},{at})")
    sql += _values("platform_support", "id,user_id,core_conversation_id,kind,status,topic,preferred_time,scheduled_at,"
                   "assigned_admin_id,assigned_admin_name,created_at,updated_at,accepted_at,closed_at,closed_message_id", rows, overriding=True)
    sql += _values("platform_support_todos", "support_id,state,assigned_admin_id,updated_at", todo_rows)
    sql += _values("platform_support_messages", "id,support_id,author_id,author_kind,content,client_turn_id,created_at", msg_rows, overriding=True)
    plan.expect["lmm_assistant.platform_support"] = len(rows)
    plan.expect["lmm_assistant.platform_support_todos"] = len(todo_rows)
    plan.expect["lmm_assistant.platform_support_messages"] = len(msg_rows)

    # memories / profiles (assistant_services). Go normalizes on read; tags kept.
    rows = []
    for m in ctx.rows("assistant_memories", "id"):
        tags = json.loads(m["tags_json"] or "[]")
        if not isinstance(tags, list) or len(tags) > 8 or m["source"] not in ("assistant", "administrator") or not (1 <= len(m["title"]) <= 80) or not (1 <= len(m["content"]) <= 800):
            fail(f"assistant_memory_shape:{m['id']}")
        rows.append(f"({m['id']},{m['user_id']},{lit(m['title'])},{lit(m['content'])},{jlit(tags)},{lit(m['source'])},{lit(m['enabled'])},"
                    f"{m['updated_by'] or m['user_id']},{m['created_at']},{m['updated_at']})")
    sql += _values("memories", "id,user_id,title,content,tags,source,enabled,updated_by,created_at,updated_at", rows, overriding=True)
    plan.expect["lmm_assistant.memories"] = len(rows)
    rows = []
    for p in ctx.rows("assistant_user_profiles", "user_id"):
        tags = json.loads(p["tags_json"] or "[]")
        if not isinstance(tags, list) or p["source"] not in ("assistant", "administrator") or len(p["strategy"]) > 4000:
            fail(f"assistant_profile_shape:{p['user_id']}")
        rows.append(f"({p['user_id']},{lit(p['profile_key'])},{jlit(tags)},{lit(p['strategy'])},{lit(p['source'])},{lit(p['enabled'])},"
                    f"{p['updated_by'] or p['user_id']},{p['created_at']},{p['updated_at']})")
    sql += _values("profiles", "user_id,profile_key,tags,strategy,source,enabled,updated_by,created_at,updated_at", rows)
    plan.expect["lmm_assistant.profiles"] = len(rows)

    # preset_stats is seeded with the four reviewed presets by schema.sql, so it
    # is reconciled by the after hook rather than `expect` (non-empty target).
    clicks = {r["preset_id"]: int(r["clicks"]) for r in ctx.q(
        "SELECT preset_id, sum(click_count)::bigint AS clicks FROM assistant_pre_conversation_preset_stats GROUP BY 1")}
    for pid in ASSISTANT_PRESETS:
        sql.append(f"UPDATE lmm_assistant.preset_stats SET clicks={clicks.get(pid, 0)} WHERE preset_id={lit(pid)};")
    dropped = {k: v for k, v in clicks.items() if k not in ASSISTANT_PRESETS}
    plan.stats["preset_clicks_imported"] = sum(clicks.get(p, 0) for p in ASSISTANT_PRESETS)
    plan.stats["preset_clicks_unrepresented"] = sum(dropped.values())

    for table in ("conversations", "history_messages", "platform_support", "platform_support_messages", "memories"):
        sql.append(f"SELECT setval(pg_get_serial_sequence('lmm_assistant.{table}','id'),"
                   f"GREATEST((SELECT max(id) FROM lmm_assistant.{table}),1));")

    def verify(_ctx: Ctx, db: Db) -> None:
        got = dict(line.split("|") for line in db.run("SELECT preset_id,clicks FROM lmm_assistant.preset_stats").split())
        if {k: int(v) for k, v in got.items()} != {p: clicks.get(p, 0) for p in ASSISTANT_PRESETS}:
            fail("assistant_preset_stats_mismatch")
        bad = db.scalar("SELECT count(*) FROM lmm_assistant.turn_receipts r LEFT JOIN lmm_assistant.conversations c "
                        "ON c.id=r.conversation_id AND c.user_id=r.user_id WHERE c.id IS NULL")
        if bad != "0":
            fail("assistant_receipt_owner_mismatch")
    plan.after = verify

    n = Counter(m["role"] for m in msgs)
    sup_user = len(support_user_msgs)
    plan.mapping.update({
        "assistant_conversations": ("lmm_assistant.conversations", len(convs),
                                    f"ids kept; {plan.stats['conversation_empty_not_imported']} message-less shells not imported "
                                    f"(hidden by the legacy list); {plan.stats['conversation_preview_from_title']} empty previews set to title"),
        "assistant_history_messages": ("lmm_assistant.history_messages", len(msgs),
                                       f"user {n['user']} + assistant {n['assistant']} kept with ids/sequence; human {n['human']} -> "
                                       f"platform_support_messages; secure_card {n['secure_card']} archived (empty legacy card marker)"),
        "assistant_turn_receipts": ("lmm_assistant.turn_receipts", len(receipts),
                                    f"{plan.expect['lmm_assistant.turn_receipts']} answer receipts (input digest verified, answer_hash=sha256 "
                                    f"of stored answer); {sup_user} support sends -> platform_support_messages.client_turn_id"),
        "assistant_support_requests": ("lmm_assistant.platform_support(+_todos)", len(supports),
                                       "ids kept; core_conversation_id='legacy_assistant_conversation:<id>' (no Core conversation exists); "
                                       "todos derived: accepted->assigned, completed/cancelled->closed"),
        "assistant_memories": ("lmm_assistant.memories", ctx.count("assistant_memories"), "ids kept; tags_json -> jsonb"),
        "assistant_user_profiles": ("lmm_assistant.profiles", ctx.count("assistant_user_profiles"),
                                    "legacy strategy text and tags kept (Go only reads `source`; some legacy tags outside the current write vocabulary)"),
        "assistant_pre_conversation_preset_stats": ("lmm_assistant.preset_stats", ctx.count("assistant_pre_conversation_preset_stats"),
                                                    f"aggregated per-bucket click_count into 4 builtin counters; non-builtin presets "
                                                    f"{dropped} and conversation/recommendation/approval counts archived"),
    })
    for t, target in {
        "assistant_overview_preferences": "lmm_assistant.records(preferences)",
        "assistant_site_issues": "lmm_assistant.site_issues",
        "assistant_site_issue_events": "lmm_assistant.site_issue_events",
    }.items():
        if ctx.count(t):
            fail(f"assistant_unexpected_rows:{t}")
        plan.mapping[t] = (target, 0, "no legacy rows")
    for t, why in {
        "assistant_leads": "chat intent observations (all status=observed); no lead/CRM store in the new architecture",
        "assistant_first_question_stats": "bucketed first-question text counters; no backend",
        "assistant_profile_buckets": "bucketed profile counters; no backend (profiles themselves imported)",
        "assistant_user_profile_audits": "profile change audit (hashes only); no audit table in assistant_services",
        "assistant_pre_conversation_preset_cache": "generated/custom presets; Go serves only the reviewed builtin seed set",
        "assistant_pre_conversation_conversation_attributions": "preset->conversation attribution; no backend",
        "assistant_pre_conversation_conversion_attributions": "0 rows; no backend",
        "assistant_weekly_discounts": "discount offers/claims touch pricing; no Go or Core weekly-discount backend",
        "assistant_weekly_usages": "0 rows; no backend",
        "assistant_registration_cases": "0 rows; registration review is Core abuse control",
        "assistant_registration_events": "0 rows; registration review is Core abuse control",
        "assistant_request_reviews": "0 rows; no backend",
        "assistant_review_resets": "0 rows; no backend",
        "assistant_security_incidents": "0 rows; no backend",
        "assistant_security_review_notices": "admin security digest notice; no backend",
    }.items():
        archived(plan, ctx, t, why, "missing_backend")
    for t, why in {
        "assistant_new_user_gifts": "money: Core core_gift.decisions needs a Core conversation, model request and ledger journal; "
                                    "granted credits already live in Core cutover ledger history; Go only reads Core's GiftPort",
        "assistant_gift_risk_keys": "legacy HMAC secret; never copied (Core core_gift.risk_key is freshly generated)",
        "assistant_gift_risk_memories": "windowed counters keyed by the legacy secret; incompatible with Core's key and expired",
        "assistant_registration_profiles": "Core core_gift.registration_profiles only honours a 900 s observation window; all expired",
        "assistant_registration_fingerprints": "Core core_gift.fingerprints uses a 7-day window and Core's own key; expired/incompatible",
        "assistant_secure_cards": "one expired, unrevealed API-key card encrypted with the legacy key; no backend",
    }.items():
        archived(plan, ctx, t, why)
    listed = {r["relname"] for r in ctx.q("SELECT relname FROM pg_stat_user_tables WHERE schemaname="
                                          f"{lit(ctx.schema)} AND relname LIKE 'assistant%'")}
    if listed != set(plan.mapping):
        fail(f"assistant_unmapped:{sorted(listed ^ set(plan.mapping))}")
    return plan
# === END PHASE: assistant ===

# === PHASE: catalog ===
# Legacy model catalog (vendors, models) -> Core channel release bindings
# (document.vendors / document.models). Core keys model metadata by routing
# route id, and bindings belong to a published routing version, so this phase
# writes no SQL: it emits catalog-metadata.json, which the operator publishes
# with scripts/legacy_catalog_publish.py through the channel configuration API
# once the routing release from legacy_routing_migration is active.

CATALOG_NOT_REPRESENTABLE = {
    "vendors": ["icon", "status", "created_time", "updated_time"],
    "models": ["icon", "tags", "endpoints", "status", "sync_official", "name_rule", "operational_status",
               "operational_notice", "operational_until", "created_time", "updated_time"],
}


def catalog_label(value: Any, code: str) -> str:
    if not isinstance(value, str) or not value.strip() or len(value.encode()) > 128 \
            or any(ord(c) < 32 or ord(c) == 127 for c in value):
        fail(code)
    return value


def catalog_description(value: Any, code: str) -> str:
    text = (value or "").replace("\r\n", "\n")
    if len(text.encode()) > 4096 or any((ord(c) < 32 and c not in "\n\t") or ord(c) == 127 for c in text):
        fail(code)
    return text


@phase("catalog", "")
def plan_catalog(ctx: Ctx) -> Plan:
    plan = Plan()
    vendors, vendor_ids, names = [], set(), set()
    for v in ctx.rows("vendors", "id"):
        if v["deleted_at"] or v["status"] != 1:
            plan.stats["vendor_skipped_deleted_or_disabled"] += 1
            continue
        name = catalog_label(v["name"], "catalog_vendor_name_invalid")
        if name.strip().lower() in names:
            fail("catalog_vendor_name_duplicate")
        names.add(name.strip().lower())
        vendor_ids.add(v["id"])
        vendors.append({"id": v["id"], "name": name,
                        "description": catalog_description(v["description"], "catalog_vendor_description_invalid"),
                        "legacy": {"icon": v["icon"] or ""}})
    models = []
    for m in ctx.rows("models", "id"):
        if m["deleted_at"] or m["status"] != 1:
            plan.stats["model_skipped_deleted_or_disabled"] += 1
            continue
        if m["name_rule"] != 0:
            fail("catalog_model_name_rule_unsupported")  # Core metadata is per exact route
        vendor_id = m["vendor_id"] or None
        if vendor_id is not None and vendor_id not in vendor_ids:
            fail("catalog_model_vendor_missing")
        name = catalog_label(m["model_name"], "catalog_model_name_invalid")
        models.append({"model_name": name, "display_name": name, "vendor_id": vendor_id,
                       "description": catalog_description(m["description"], "catalog_model_description_invalid"),
                       "legacy": {"id": m["id"], "icon": m["icon"] or "", "tags": m["tags"] or "",
                                  "endpoints": m["endpoints"] or "", "sync_official": m["sync_official"],
                                  "operational_status": m["operational_status"]}})
        plan.stats["model_with_description"] += bool(models[-1]["description"])
        plan.stats["model_with_vendor"] += vendor_id is not None
    plan.stats["vendor"], plan.stats["model"] = len(vendors), len(models)
    out = ctx.args.output.expanduser().resolve() / "catalog-metadata.json"
    out.write_text(json.dumps({"format_version": 1, "scope": "legacy-model-catalog-metadata",
                               "not_representable": CATALOG_NOT_REPRESENTABLE,
                               "vendors": vendors, "models": models}, ensure_ascii=False, indent=1) + "\n")
    note = ("catalog-metadata.json; publish with scripts/legacy_catalog_publish.py after the first routing "
            "release; not representable: ")
    plan.mapping["vendors"] = ("core:channel_release_bindings.document.vendors", len(vendors),
                               note + ",".join(CATALOG_NOT_REPRESENTABLE["vendors"]))
    plan.mapping["models"] = ("core:channel_release_bindings.document.models[route_id]", len(models),
                              note + ",".join(CATALOG_NOT_REPRESENTABLE["models"])
                              + "; a model without a route of the same exact name is not published")
    return plan
# === END PHASE: catalog ===

# === PHASE: other ===
# No Go module owns these domains, so this phase writes no module database
# (database ""). It only records dispositions and emits Core-owned SQL for the
# two domains whose Core table exists and no legacy_cutover_* phase covers.

OTHER_ARCHIVED = {
    # acquisition: the Web UI and Core permission resource exist, but no Core or
    # Go endpoint/table serves /api/acquisition or /api/admin/acquisition.
    **{t: ("missing_backend", "acquisition analytics: Web UI and Core permission resource only; "
           "no Core/Go endpoint or table") for t in (
        "acquisition_accounts", "acquisition_visits", "acquisition_visitors", "acquisition_consents",
        "acquisition_links", "acquisition_activities", "acquisition_activity_states",
        "acquisition_self_reports", "acquisition_attribution_policies", "acquisition_configs")},
    "acquisition_first_payments": ("missing_backend", "acquisition attribution snapshot; no backend, and "
                                   "migrate-legacy-financial lists it as reward-claim evidence (no balance)"),
    "developer_access_requests": ("archived", "current decision imported by legacy_cutover_import into "
                                  "core_identity.developer_access (derived from users); every user whose last "
                                  "request was approved is granted; Core has no request/review history table"),
    "developer_access_recommendation_archives": ("archived", "AI recommendation text for closed requests; "
                                                 "no Core owner"),
    "moderation_jobs": ("archived", "all terminal market_product/market_tool review jobs, charged_quota=0; "
                        "payload holds private seller input; no Core/Go moderation queue"),
    "ratio_notifications": ("archived", "core_notifications.price_events requires FK to routing price/config "
                            "versions and is generated only by routing publication"),
    "system_tasks": ("archived", "legacy scheduler runs, all succeeded/failed (terminal); runtime state"),
    "system_instances": ("archived", "legacy node heartbeats; core_operations.instances is written by the "
                         "running Core (boot_id)"),
    "finance_payment_methods": ("archived", "legacy finance-report labels; Core payment gateways are TOML "
                                "configured (epay/waffo_pancake were the enabled ones)"),
    # models / vendors: mapped by the catalog phase (catalog-metadata.json).
    "abilities": ("core:legacy_routing_migration", "input of scripts/legacy_routing_migration.py "
                  "(routing groups/routes)"),
}


@phase("other", "")
def plan_other(ctx: Ctx) -> Plan:
    plan = Plan()
    for table, (status, note) in OTHER_ARCHIVED.items():
        archived(plan, ctx, table, note, status)
    for table in ("moderation_notices", "ratio_deliveries", "system_task_locks", "company_billing_profiles"):
        n = ctx.count(table)
        plan.mapping[table] = ("empty" if n == 0 else "archived", n, "no rows" if n == 0 else "unreviewed rows")

    # Account appeals -> core_account_appeals.requests (ids kept). Reviewed
    # requests carry revision 2, like a Core review. user_version is the
    # imported auth_version; the generation of the old suspension is unknown,
    # and only pending requests depend on it (none are pending).
    appeals = ctx.rows("account_action_requests", "id")
    if any(r["kind"] != "appeal" or r["status"] not in ("approved", "rejected")
           or r["target_user_id"] != r["requested_by_user_id"] or not r["admin_user_id"]
           or not r["reviewed_at"] for r in appeals):
        fail("other_account_action_requests_unsupported")
    core = ["DO $$BEGIN IF EXISTS(SELECT 1 FROM core_account_appeals.requests) "
            "OR EXISTS(SELECT 1 FROM core_telegram_auth.bindings) "
            "THEN RAISE EXCEPTION 'other_core_target_not_empty'; END IF; END$$;"]
    for r in appeals:
        core.append(
            "INSERT INTO core_account_appeals.requests(id,user_id,user_version,reason,status,admin_note,"
            "reviewed_by,created_at,reviewed_at,revision) OVERRIDING SYSTEM VALUE "
            f"VALUES({r['id']},{r['target_user_id']},(SELECT auth_version FROM core_identity.users "
            f"WHERE id={r['target_user_id']}),{lit(r['reason'])},{lit(r['status'])},"
            f"{lit((r['admin_note'] or '')[:2000])},{r['admin_user_id']},to_timestamp({r['created_at']}),"
            f"to_timestamp({r['reviewed_at']}),2);")
        plan.stats["appeal_" + r["status"]] += 1
    if appeals:
        core.append("SELECT setval(pg_get_serial_sequence('core_account_appeals.requests','id'),"
                    f"{max(r['id'] for r in appeals)});")
    plan.mapping["account_action_requests"] = ("core:core_account_appeals.requests", len(appeals),
                                               "reviewed appeals, ids kept; Core-owned SQL in core-other.sql")

    # External identity claims: GitHub subjects equal users.github_id and are
    # imported by legacy_cutover_import (oauth_identities). Telegram is not.
    claims = ctx.q("SELECT c.provider,c.subject,c.user_id,c.created_at,u.telegram_id,u.github_id "
                   "FROM external_identity_claims c JOIN users u ON u.id=c.user_id ORDER BY c.id")
    if len(claims) != ctx.count("external_identity_claims"):
        fail("other_external_identity_orphan")
    for c in claims:
        if c["provider"] == "github" and c["subject"] == c["github_id"]:
            plan.stats["claim_github_covered_by_cutover"] += 1
        elif c["provider"] == "telegram" and c["subject"] == c["telegram_id"] and c["subject"].isdigit():
            created = f"{lit(c['created_at'])}::timestamptz" if c["created_at"] else "statement_timestamp()"
            core.append("INSERT INTO core_telegram_auth.bindings(subject,user_id,created_at) "
                        f"VALUES({lit(c['subject'])},{c['user_id']},{created});")
            plan.stats["claim_telegram"] += 1
        else:
            fail("other_external_identity_unsupported")
    plan.mapping["external_identity_claims"] = (
        "core:core_identity.oauth_identities+core_telegram_auth.bindings", len(claims),
        f"github {plan.stats['claim_github_covered_by_cutover']} via legacy_cutover_import; "
        f"telegram {plan.stats['claim_telegram']} in core-other.sql")
    core.append("SELECT 'other_core_check', (SELECT count(*) FROM core_account_appeals.requests), "
                "(SELECT count(*) FROM core_telegram_auth.bindings);")
    plan.core_sql = core
    return plan
# === END PHASE: other ===

# ---------------------------------------------------------------------------
# runner
# ---------------------------------------------------------------------------

def target_db(args, name: str, database: str) -> Db:
    override = dict(x.split("=", 1) for x in args.target or [])
    database = override.get(name, database)
    base = database.split("_")[0] + "_" + database.split("_")[1]
    return Db(args.socket, args.port, database, args.login or f"{base}_installer",
              role=args.owner_role or f"{base}_owner", readonly=False)


def table_counts(db: Db, tables) -> dict[str, int]:
    out = {}
    for t in tables:
        out[t] = int(db.scalar(f"SELECT count(*) FROM {t}"))
    return out


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--socket", required=True, help="PostgreSQL socket directory")
    p.add_argument("--port", type=int, default=5432)
    p.add_argument("--source-db", default="legacy")
    p.add_argument("--source-user", default="postgres")
    p.add_argument("--schema", required=True, help="legacy schema, e.g. lmm_prod_20260802")
    p.add_argument("--output", required=True, type=Path, help="private plan/report dir under ~/Backups")
    p.add_argument("--phase", action="append", choices=sorted(PHASES), help="default: all")
    p.add_argument("--target", action="append", help="PHASE=DATABASE override")
    p.add_argument("--login", help="login role for module DBs (default lmm_<x>_installer)")
    p.add_argument("--owner-role", help="SET ROLE owner (default lmm_<x>_owner)")
    p.add_argument("--toolmarket-key-file", help="raw 32-byte toolmarket snapshot key")
    p.add_argument("--extensions-bin", help="built lmm-extensions binary (toolmarket snapshot seal)")
    p.add_argument("--legacy-secrets-env", help="legacy KEY=VALUE secrets file (0600); values never printed")
    p.add_argument("--store-key-file", help="legacy MERCHANT_STORE_ENCRYPTION_KEY (or CRYPTO_SECRET) text file")
    p.add_argument("--apply", action="store_true")
    args = p.parse_args()
    out = args.output.expanduser().resolve()
    if BACKUP_ROOT.resolve() not in out.parents:
        fail("output_outside_backups")
    out.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(out, 0o700)
    src = Db(args.socket, args.port, args.source_db, args.source_user, role=None, readonly=True)
    ctx = Ctx(src, args.schema, args)
    previous = out / "report.json"
    report: dict[str, Any] = json.loads(previous.read_text()) if previous.exists() else {}
    for name in args.phase or list(PHASES):
        database, fn = PHASES[name]
        plan = fn(ctx)
        body = "\n".join(plan.sql)
        (out / f"{name}.sql").write_text("BEGIN;\nSET TRANSACTION ISOLATION LEVEL SERIALIZABLE;\n" + body + "\nCOMMIT;\n")
        if plan.core_sql:
            (out / f"core-{name}.sql").write_text("-- run in Core phase order by the coordinator\n"
                                                    + "\n".join(plan.core_sql) + "\n")
        entry: dict[str, Any] = {"database": database, "expect": plan.expect, "stats": dict(plan.stats),
                                 "mapping": {k: list(v) for k, v in plan.mapping.items()},
                                 "core_sql_statements": len(plan.core_sql)}
        if args.apply and database == "":
            if plan.sql or plan.expect:
                fail(f"phase_without_database_has_rows:{name}")
            entry["reconciled"] = True
        elif args.apply:
            db = target_db(args, name, database)
            before = table_counts(db, plan.expect)
            if any(before.values()):
                fail(f"target_not_empty:{name}:{before}")
            db.run("BEGIN;\nSET TRANSACTION ISOLATION LEVEL SERIALIZABLE;\n" + body + "\nCOMMIT;\n")
            if plan.after:
                plan.after(ctx, db)
            after = table_counts(db, plan.expect)
            entry["actual"] = after
            entry["reconciled"] = after == plan.expect
            if after != plan.expect:
                report[name] = entry
                (out / "report.json").write_text(json.dumps(report, indent=1, ensure_ascii=False))
                fail(f"reconcile_mismatch:{name}")
        report[name] = entry
        print(f"{name}: {'applied' if args.apply else 'planned'} {plan.expect} {dict(plan.stats)}")
    (out / "report.json").write_text(json.dumps(report, indent=1, ensure_ascii=False))
    for f in out.iterdir():
        os.chmod(f, 0o600)


if __name__ == "__main__":
    main()
