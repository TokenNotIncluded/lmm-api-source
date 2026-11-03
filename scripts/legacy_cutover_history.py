#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Cutover phases for history, OAuth clients, MFA, passkeys, Telegram and channel openings.

Registered by legacy_cutover_import.py; each phase is one owner SERIALIZABLE
transaction guarded by an empty target table. Source reads are read-only.
Legacy user IDs are Core user and personal account IDs; legacy channel IDs are
Core channel identity IDs. Nothing here funds wallets, posts usage or issues
OAuth tokens. Items that Core cannot represent are counted in the report.

Reused reviewed semantics: legacy_history_import.captured_basis (per-log price
basis), legacy_site_statistics.normalized_usage/classify_topup (site finance),
prepare_wallet_history_snapshot.prepare (self top-up receipt) and the channel
opening rules in docs/channel-openings.md.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import tomllib
from collections import Counter, defaultdict
from decimal import Decimal
from fractions import Fraction
from pathlib import Path

PHASES = ("history", "oauth_apps", "mfa", "passkeys", "telegram", "channels_history")
GUARDS = {
    "history": "SELECT count(*) FROM core_history.sources",
    "oauth_apps": "SELECT count(*) FROM core_market_oauth.clients",
    "mfa": "SELECT count(*) FROM core_identity.mfa_factors",
    "passkeys": "SELECT count(*) FROM core_passkeys.credentials",
    "telegram": "SELECT count(*) FROM core_telegram_auth.bindings",
    "channels_history": "SELECT count(*) FROM core_channels.opening_sources",
}
MAX_TIME = 253402300799
MAX_ID = 9007199254740991
CREDITS_PER_USD = 500000
TEXT_FIELDS = (("username", "username"), ("token_name", "token_name"), ("model_name", "model_name"),
               ("group_name", "group"), ("request_id", "request_id"),
               ("upstream_request_id", "upstream_request_id"))


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


def canonical(value) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), default=str)


def sha(value) -> str:
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def jlit(value) -> str:
    return lit(canonical(value)) + "::jsonb"


def copy_value(value) -> str:
    if value is None:
        return r"\N"
    if isinstance(value, bool):
        return "t" if value else "f"
    text = str(value)
    return (text.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r"))


def copy_block(table: str, columns: tuple[str, ...], rows) -> str:
    lines = [f"COPY {table}({','.join(columns)}) FROM STDIN;"]
    lines += ["\t".join(copy_value(r[c]) for c in columns) for r in rows]
    lines.append("\\.")
    return "\n".join(lines)


def query(db, sql: str, schema: str | None = None):
    prefix = f"SET search_path={schema};" if schema else ""
    return json.loads(db.run(f"{prefix}SELECT COALESCE(json_agg(t),'[]') FROM ({sql}) t"), parse_float=Decimal)


def add_arguments(parser) -> None:
    parser.add_argument("--mfa-keyring", type=Path,
                        help="mfa: Core [identity.mfa] keyring TOML used to encrypt enabled legacy TOTP secrets")
    parser.add_argument("--market-issuer",
                        help="oauth_apps: Core market OAuth issuer, e.g. https://api.lmm.best/api/oauth2/market")
    parser.add_argument("--credit-rebase-boundary", type=int,
                        help="channels_history: override the legacy wallet credit rebase snapshot_at")


def plan(phase: str, args, src, target) -> tuple[list[str], dict]:
    return {
        "history": plan_history, "oauth_apps": plan_oauth_apps, "mfa": plan_mfa,
        "passkeys": plan_passkeys, "telegram": plan_telegram, "channels_history": plan_channels,
    }[phase](args, src, target)


# --------------------------------------------------------------------- history
def _text(value) -> str:
    value = "" if value is None else value
    if type(value) is not str or len(value.encode()) > 256 or "\0" in value:
        fail("history_text_unrepresentable")
    return value


def _int(value, code, lo=-(1 << 63), hi=(1 << 63) - 1, nullable=True):
    if value is None and nullable:
        return None
    if type(value) is not int or not lo <= value <= hi:
        fail(code)
    return value


def plan_history(a, src, tgt):
    import legacy_history_import as LH  # reviewed captured price-basis parser
    import legacy_site_statistics as LS
    import prepare_wallet_history_snapshot as WH

    schema, archive = a.source_schema, a.archive_sha256
    stats = Counter()
    owners = {r["id"]: r["personal_account_id"] for r in query(tgt, "SELECT id,personal_account_id FROM core_identity.users")}
    creds = {r["id"]: r for r in query(tgt, "SELECT id,user_id,owner_account_id FROM core_identity.credentials WHERE kind='api_key'")}
    rows, excluded = [], []
    out = src.run(f"SET search_path={schema};SELECT row_to_json(l)::text FROM logs l ORDER BY id")
    for line in out.splitlines():
        raw = json.loads(line)
        sid = _int(raw["id"], "history_source_id_invalid", 1, MAX_ID, False)
        uid = raw.get("user_id")
        if uid not in owners:
            excluded.append({"source_log_id": sid, "user_id": uid, "type": raw.get("type"),
                             "quota": raw.get("quota"), "row_sha256": sha(raw)})
            stats["logs_excluded_user_missing"] += 1
            continue
        key = raw.get("token_id") or 0
        cred = creds.get(key) if key else None
        token = key if cred and cred["user_id"] == uid and cred["owner_account_id"] == owners[uid] else None
        if key and token is None:
            stats["logs_token_unmapped"] += 1
        r = {"source_log_id": sid, "canonical_event_key": f"legacy.logs:{archive}:{sid}",
             "source_row_sha256": sha(raw), "user_id": uid, "owner_account_id": owners[uid], "token_id": token,
             "created_at": _int(raw.get("created_at"), "history_time_invalid", 0, MAX_TIME, False),
             "log_type": _int(raw.get("type"), "history_type_invalid", 0, 7, False),
             "quota": _int(raw.get("quota"), "history_quota_invalid"),
             "pricing_credits_per_usd": LH.captured_basis(raw.get("other")),
             "prompt_tokens": _int(raw.get("prompt_tokens"), "history_tokens_invalid"),
             "completion_tokens": _int(raw.get("completion_tokens"), "history_tokens_invalid"),
             "channel_id": _int(raw.get("channel_id") or 0, "history_channel_invalid", 0, MAX_ID, False)}
        for target_field, source_field in TEXT_FIELDS:
            r[target_field] = _text(raw.get(source_field))
        rows.append(r)
        stats["logs"] += 1
        stats[f"logs_type_{r['log_type']}"] += 1
        stats["logs_priced"] += r["pricing_credits_per_usd"] is not None
        if r["log_type"] == 2:
            stats["consume_quota_sum"] += r["quota"] or 0
    mapping = {"identities": sorted(owners.items()), "credentials": sorted(
        (k, v["user_id"]) for k, v in creds.items()), "excluded": excluded}
    source = {"contract_version": "core.history-source.v1", "source_archive_sha256": archive,
              "mapping_sha256": sha(mapping), "source_namespace": f"legacy.logs:{schema}",
              "observed_at": a.observed_at, "complete": True}
    by_scope = defaultdict(list)
    for r in rows:
        item = [r["source_log_id"], r["source_row_sha256"]]
        by_scope[(r["user_id"], None)].append(item)
        if r["token_id"] is not None:
            by_scope[(r["user_id"], r["token_id"])].append(item)
    scopes = [(uid, None) for uid in sorted(owners)] + sorted((c["user_id"], k) for k, c in creds.items())
    receipts = []
    for uid, token in scopes:
        scope = {"user_id": uid, "owner_account_id": owners[uid], "token_id": token,
                 "source_archive_sha256": archive, "mapping_sha256": source["mapping_sha256"],
                 "rows": by_scope.get((uid, token), [])}
        receipts.append({**{k: scope[k] for k in ("user_id", "owner_account_id", "token_id",
                         "source_archive_sha256", "mapping_sha256")},
                         "source_scope_sha256": sha(scope), "complete": True})
    stats["scope_receipts"] = len(receipts)
    log_cols = ("source_log_id", "canonical_event_key", "source_row_sha256", "user_id", "owner_account_id",
                "token_id", "created_at", "log_type", "quota", "pricing_credits_per_usd", "prompt_tokens",
                "completion_tokens", "username", "token_name", "model_name", "channel_id", "group_name",
                "request_id", "upstream_request_id")
    rec_cols = ("user_id", "owner_account_id", "token_id", "source_archive_sha256", "mapping_sha256",
                "source_scope_sha256", "complete")
    src_where = (f"source_archive_sha256={lit(archive)} AND mapping_sha256={lit(source['mapping_sha256'])} "
                 f"AND source_namespace={lit(source['source_namespace'])}")
    sql = [
        "INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,"
        "source_namespace,observed_at,complete) VALUES(" + ",".join(lit(source[k]) for k in (
            "contract_version", "source_archive_sha256", "mapping_sha256", "source_namespace",
            "observed_at", "complete")) + ");",
        "CREATE TEMP TABLE lmm_cutover_logs ON COMMIT DROP AS SELECT * FROM core_history.log_records WITH NO DATA;",
        "CREATE TEMP TABLE lmm_cutover_scopes ON COMMIT DROP AS SELECT * FROM core_history.scope_receipts WITH NO DATA;",
        copy_block("lmm_cutover_logs", log_cols, rows),
        copy_block("lmm_cutover_scopes", rec_cols, receipts),
        f"INSERT INTO core_history.log_records(source_id,{','.join(log_cols)}) SELECT s.source_id,"
        f"{','.join('l.' + c for c in log_cols)} FROM lmm_cutover_logs l CROSS JOIN "
        f"(SELECT source_id FROM core_history.sources WHERE {src_where}) s;",
        f"INSERT INTO core_history.scope_receipts(source_id,{','.join(rec_cols)}) SELECT s.source_id,"
        f"{','.join('r.' + c for c in rec_cols)} FROM lmm_cutover_scopes r CROSS JOIN "
        f"(SELECT source_id FROM core_history.sources WHERE {src_where}) s;",
        "DO $$BEGIN IF (SELECT count(*) FROM core_history.log_records)<>" + str(len(rows)) +
        " OR (SELECT COALESCE(sum(quota),0) FROM core_history.log_records WHERE log_type=2)<>" +
        str(stats["consume_quota_sum"]) + " OR (SELECT count(*) FROM core_history.scope_receipts)<>" +
        str(len(receipts)) + " THEN RAISE EXCEPTION 'history_readback_mismatch'; END IF; END$$;",
    ]
    quota_rows, quota_cols = plan_quota_aggregates(a, src, owners, creds, stats)
    sql.append("CREATE TEMP TABLE lmm_cutover_quota ON COMMIT DROP AS SELECT * FROM core_history.quota_aggregates WITH NO DATA;")
    sql.append(copy_block("lmm_cutover_quota", quota_cols, quota_rows))
    sql.append(f"INSERT INTO core_history.quota_aggregates(source_id,{','.join(quota_cols)}) SELECT s.source_id,"
               f"{','.join('q.' + c for c in quota_cols)} FROM lmm_cutover_quota q CROSS JOIN "
               f"(SELECT source_id FROM core_history.sources WHERE {src_where}) s;")
    sql.append("DO $$BEGIN IF (SELECT count(*) FROM core_history.quota_aggregates)<>" + str(len(quota_rows)) +
               " OR (SELECT COALESCE(sum(quota_credits),0) FROM core_history.quota_aggregates)<>" +
               str(stats["quota_aggregate_credits"]) + " THEN RAISE EXCEPTION 'quota_readback_mismatch'; END IF; END$$;")
    # Site finance observations (admin site statistics), reviewed legacy rules.
    run = query(tgt, "SELECT * FROM core_billing.opening_import_runs")
    if len(run) != 1:
        fail("history_opening_run_missing")
    run = run[0]
    counters = {r["user_id"]: r for r in query(tgt, "SELECT user_id,account_id,source_row_sha256,used_quota "
                                                    "FROM core_billing.account_opening_counters")}
    tables = {"users": query(src, "SELECT * FROM users ORDER BY id", schema),
              "wallet_credit_rebases": [{**r, "plan": r["plan"]} for r in query(
                  src, "SELECT migration_id,plan_sha256,plan::text AS plan,applied_at FROM wallet_credit_rebases", schema)],
              "tokens": query(src, "SELECT * FROM tokens ORDER BY id", schema),
              "violation_fee_records": query(src, "SELECT * FROM violation_fee_records ORDER BY id", schema)}
    normalized = LS.normalized_usage(tables)
    legacy_used = {u["id"]: u["used_quota"] for u in tables["users"]}
    if set(counters) != set(legacy_used) or set(counters) != set(owners):
        fail("history_finance_user_coverage_mismatch")
    finance_users = []
    for uid in sorted(counters):
        c = counters[uid]
        if c["used_quota"] != legacy_used[uid] or c["account_id"] != owners[uid] or c["used_quota"] < 0:
            fail("history_finance_counter_mismatch")
        finance_users.append({"user_id": uid, "account_id": c["account_id"], "source_row_sha256": c["source_row_sha256"],
                              "raw_used_quota": c["used_quota"], "normalized_used_credits": normalized[uid]})
    subscriptions = {o["trade_no"] for o in query(src, "SELECT trade_no FROM subscription_orders", schema)
                     if o["trade_no"] is not None}
    top_ups = query(src, "SELECT * FROM top_ups ORDER BY id", schema)
    topups = [LS.classify_topup(r, subscriptions) for r in top_ups if r["status"] == "success"]
    for t in topups:
        stats["topup_" + t["classification"]] += 1
        if t["currency"]:
            stats[f"topup_gross_micros_{t['classification']}_{t['currency']}"] += t["gross_amount_micros"]
    normalization = {"legacy_revision": LS.REVISION, "algorithm": "usage-projection-user-v1",
                     "native_without_audit": True, "tables": {k: {"count": len(v), "rows_sha256": sha(v)}
                                                              for k, v in tables.items()}}
    fsource = {"contract": LS.CONTRACT, "source_archive_sha256": run["source_archive_sha256"],
               "mapping_sha256": run["mapping_sha256"], "tables_sha256": sha(normalization["tables"]),
               "normalization_sha256": sha(normalization), "rows_sha256": sha({"users": finance_users, "topups": topups}),
               "observed_at": run["observed_at"], "user_count": len(finance_users), "topup_count": len(topups),
               "credits_per_usd": CREDITS_PER_USD, "complete": True}
    fsource["source_key"] = sha(fsource)
    key = fsource["source_key"]
    cols = ("source_key", "contract", "source_archive_sha256", "mapping_sha256", "tables_sha256",
            "normalization_sha256", "rows_sha256", "observed_at", "user_count", "topup_count", "credits_per_usd",
            "complete")
    sql.append(f"INSERT INTO core_history.finance_sources({','.join(cols)},native_request_ids) VALUES("
               + ",".join(lit(fsource[c]) for c in cols)
               + ",ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE \"C\"));")
    sql.append(copy_block("core_history.finance_users", ("user_id", "account_id", "source_key", "source_row_sha256",
                          "raw_used_quota", "normalized_used_credits"),
                          [{**u, "source_key": key} for u in finance_users]))
    sql.append(copy_block("core_history.finance_topups", ("source_key", "source_order_id", "source_row_sha256",
                          "classification", "currency", "gross_amount_micros", "refunded_amount_micros"),
                          [{**t, "source_key": key} for t in topups]))
    stats.update(finance_users=len(finance_users), finance_topups=len(topups),
                 raw_used_quota=sum(legacy_used.values()),
                 normalized_used_credits=sum(u["normalized_used_credits"] for u in finance_users))
    # Self top-up history receipt read by wallet_topup_snapshot (all 325 orders).
    orders = query(src, "SELECT id,user_id,amount,money::text AS money_text,trade_no,payment_method,status,"
                        "create_time,complete_time,credited_quota,platform_amount_micros,settlement_currency "
                        "FROM top_ups ORDER BY id", schema)
    # Orders of users absent from the archive have no Core owner to read them.
    orphan_orders = [o for o in orders if o["user_id"] not in legacy_used]
    if any(o["status"] == "success" for o in orphan_orders):
        fail("wallet_history_orphan_success_order")
    orders = [o for o in orders if o["user_id"] in legacy_used]
    export = {"source_namespace": WH.NAMESPACE, "observed_at": a.observed_at, "source_row_count": len(orders),
              "source_user_ids": sorted(legacy_used), "rows": orders}
    if not WH.NAMESPACE.endswith(f".{schema}.top_ups"):
        fail("wallet_history_namespace_mismatch")
    identity_map = [{"source_user_id": u, "target_user_id": u, "target_personal_account_id": owners[u]}
                    for u in sorted(legacy_used)]
    manifest = WH.prepare(export, identity_map, expected_rows=len(orders), archive_sha256=archive,
                          export_sha256=WH.digest(export))
    sql.append("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) "
               f"VALUES({lit(manifest['migration_id'])},{lit(archive)},{lit(manifest['mapping_sha256'])},"
               f"{lit(WH.canonical(manifest))}::jsonb);")
    dict.update(stats, wallet_topup_history_migration_id=manifest["migration_id"], wallet_topup_orders=len(orders),
                 wallet_topup_orders_excluded_user_missing=[o["id"] for o in orphan_orders],
                 perf_metrics_rows_not_imported=query(src, "SELECT count(*) AS n FROM perf_metrics", schema)[0]["n"])
    stats["excluded_log_ids"] = [e["source_log_id"] for e in excluded]
    return sql, dict(stats)


def plan_quota_aggregates(a, src, owners, creds, stats):
    """Legacy hourly quota_data with the reviewed 2026-10 credit rebase applied.

    The rebase divided wallet units by its exact divisor but left quota_data
    rows untouched. Buckets that ended before the rebase snapshot are scaled with
    the plan's divisor/rounding; buckets that started after it was applied are
    already current units. A bucket spanning the window has no proved unit.
    """
    import legacy_site_statistics as LS
    schema = a.source_schema
    rebases = query(src, "SELECT plan->>'divisor' AS divisor,plan->>'rounding' AS rounding,"
                         "(plan->>'snapshot_at')::bigint AS snapshot_at,"
                         "floor(extract(epoch FROM applied_at))::bigint AS applied_at,"
                         "plan->>'usd_credit_conversion' AS k FROM wallet_credit_rebases", schema)
    if len(rebases) > 1 or any(r["k"] != str(CREDITS_PER_USD) for r in rebases):
        fail("quota_rebase_unsupported")
    rebase = rebases[0] if rebases else None
    cols = ("source_row_id", "source_row_sha256", "user_id", "owner_account_id", "token_id", "username",
            "model_name", "group_name", "channel_id", "node_name", "created_at", "request_count", "token_used",
            "raw_quota", "quota_credits", "credits_basis")
    rows = []
    for raw in query(src, "SELECT * FROM quota_data ORDER BY id", schema):
        uid = raw["user_id"]
        if uid not in owners:
            stats["quota_rows_excluded_user_missing"] += 1
            continue
        at = _int(raw["created_at"], "quota_time_invalid", 0, MAX_TIME, False)
        quota = _int(raw["quota"] or 0, "quota_invalid", 0, MAX_ID, False)
        if rebase is None or at >= rebase["applied_at"]:
            credits, basis = quota, "current_units"
        elif at + 3600 <= rebase["snapshot_at"]:
            credits = LS.scaled(quota, Fraction(rebase["divisor"]), rebase["rounding"])
            basis = "rebased_legacy_units"
        else:
            fail(f"quota_aggregate_unit_ambiguous:{raw['id']}")
        key = raw.get("token_id") or 0
        cred = creds.get(key) if key else None
        rows.append({"source_row_id": raw["id"], "source_row_sha256": sha(raw), "user_id": uid,
                     "owner_account_id": owners[uid],
                     "token_id": key if cred and cred["user_id"] == uid else None,
                     "username": _text(raw.get("username")), "model_name": _text(raw.get("model_name")),
                     "group_name": _text(raw.get("use_group")),
                     "channel_id": _int(raw.get("channel_id") or 0, "quota_channel_invalid", 0, MAX_ID, False),
                     "node_name": _text(raw.get("node_name")), "created_at": at,
                     "request_count": _int(raw.get("count") or 0, "quota_count_invalid", 0, MAX_ID, False),
                     "token_used": _int(raw.get("token_used") or 0, "quota_tokens_invalid", 0, MAX_ID, False),
                     "raw_quota": quota, "quota_credits": credits, "credits_basis": basis})
        stats["quota_rows"] += 1
        stats["quota_rows_" + basis] += 1
        stats["quota_aggregate_raw"] += quota
        stats["quota_aggregate_credits"] += credits
    return rows, cols


# ------------------------------------------------------------------ oauth_apps
def market_redirect(raw: str) -> str:
    """Python port of Core market_oauth::protocol::redirect for stored templates."""
    from urllib.parse import urlsplit
    u = urlsplit(raw)
    if (len(raw) > 512 or "%" in raw or u.query or u.fragment or u.username or u.password
            or "//" in u.path or any(p in (".", "..") for p in u.path.split("/"))):
        fail("market_redirect_invalid")
    host = u.hostname or ""
    if u.scheme == "https" and host and host != "localhost":
        return raw
    if u.scheme == "http" and host in ("127.0.0.1", "::1"):
        netloc = "[::1]" if host == "::1" else host
        return f"http://{netloc}{u.path or '/'}"
    if u.scheme == "http" and host == "localhost":
        return raw
    fail("market_redirect_invalid")


def plan_oauth_apps(a, src, tgt):
    schema = a.source_schema
    clients = query(src, "SELECT * FROM oauth_server_mcp_clients ORDER BY id", schema)
    grants = query(src, "SELECT id,client_id,user_id,resource,scope,absolute_expires_at_ms,revoked_at_ms "
                        "FROM oauth_server_grants ORDER BY created_at_ms", schema)
    tokens = query(src, "SELECT kind,family_id,expires_at_ms,used_at_ms FROM oauth_server_tokens", schema)
    stats, sql = Counter(), []
    issuers = {c["issuer"] for c in clients}
    issuer = a.market_issuer or (issuers.pop().rstrip("/") + "/api/oauth2/market" if len(issuers) == 1 else None)
    if not issuer or not re.fullmatch(r"https://[a-z0-9.-]+/api/oauth2/market", issuer):
        fail("market_issuer_required")
    for c in clients:
        if not re.fullmatch(r"lmm-mcp-[a-f0-9]{64}", c["id"]):
            fail("market_client_id_invalid")
        uris = json.loads(c["redirect_uris"])
        scopes = sorted(c["scope"].split(" "))
        if not uris or len(uris) > 4 or "market:discover" not in scopes or not set(scopes) <= {
                "market:discover", "market:invoke", "market:manage"}:
            fail("market_client_invalid")
        templates = sorted(set(market_redirect(u) for u in uris))
        kind = "web" if templates[0].startswith("https://") else "native"
        if any(t.startswith("https://") != (kind == "web") for t in templates):
            fail("market_client_mixed_redirects")
        refresh = not c["refresh_disabled"]
        grant_types = ["authorization_code", "refresh_token"] if refresh else ["authorization_code"]
        metadata = {"client_name": c["name"], "application_type": kind, "redirect_uris": uris,
                    "token_endpoint_auth_method": "none", "grant_types": grant_types,
                    "response_types": ["code"], "scope": " ".join(scopes)}
        sql.append("INSERT INTO core_market_oauth.clients(id,issuer,metadata,redirect_templates,scopes,"
                   f"refresh_enabled,created_at) VALUES({lit(c['id'])},{lit(issuer)},{jlit(metadata)},"
                   f"ARRAY[{','.join(lit(t) for t in templates)}]::text[],"
                   f"ARRAY[{','.join(lit(s) for s in scopes)}]::text[],{lit(refresh)},{c['created_at_ms'] // 1000});")
        stats["market_clients"] += 1
    boundary = a.observed_at * 1000
    live_refresh = Counter(t["family_id"] for t in tokens
                           if t["kind"] == "refresh" and t["used_at_ms"] == 0 and t["expires_at_ms"] > boundary)
    for g in grants:
        live = g["revoked_at_ms"] == 0 and g["absolute_expires_at_ms"] > boundary
        market = g["client_id"].startswith("lmm-mcp-")
        stats[("market" if market else "model") + "_grants_" + ("live" if live else "inactive")] += 1
        if live:
            stats.setdefault("reauthorize_required", []).append(
                {"grant": g["id"][:6], "client_id": g["client_id"][:20], "user_id": g["user_id"],
                 "live_refresh_tokens": live_refresh.get(g["id"], 0)})
    stats["legacy_tokens_not_imported"] = len(tokens)
    stats["legacy_live_refresh_tokens"] = sum(live_refresh.values())
    stats["legacy_codes_not_imported"] = query(src, "SELECT count(*) AS n FROM oauth_server_codes", schema)[0]["n"]
    stats["legacy_billing_bindings_not_imported"] = query(
        src, "SELECT count(*) AS n FROM o_auth_billing_bindings", schema)[0]["n"]
    stats["market_issuer"] = issuer
    return sql, dict(stats)


# ------------------------------------------------------------------------- mfa
HKDF_DOMAIN = b"api.lmm.best/core/mfa/v1"
AAD_DOMAIN = b"api.lmm.best/core/mfa/secret"
LABEL = re.compile(r"[A-Za-z0-9_-]{1,64}\Z")


def _private_file(path: Path, size: int | None = None) -> bytes:
    st = os.lstat(path)
    if not path.is_absolute() or not (st.st_mode & 0o170000) == 0o100000 or st.st_mode & 0o777 not in (0o400, 0o600):
        fail("mfa_keyring_file_not_private")
    data = path.read_bytes()
    if size is not None and len(data) != size:
        fail("mfa_key_length_invalid")
    return data


def load_mfa_key(path: Path) -> tuple[str, bytes]:
    """Active (key_id, AES-256 key) derived exactly like Core MfaKeyring::new."""
    from cryptography.hazmat.primitives import hashes
    from cryptography.hazmat.primitives.kdf.hkdf import HKDF
    ring = tomllib.loads(_private_file(path).decode())
    if set(ring) != {"version", "active_key_id", "keys"} or ring["version"] != 1 or not 1 <= len(ring["keys"]) <= 8:
        fail("mfa_keyring_invalid")
    keys = {k["id"]: Path(k["key_file"]) for k in ring["keys"]}
    active = ring["active_key_id"]
    if not LABEL.match(active) or active not in keys or len(keys) != len(ring["keys"]):
        fail("mfa_keyring_invalid")
    ikm = _private_file(keys[active], 32)
    return active, HKDF(algorithm=hashes.SHA256(), length=32, salt=HKDF_DOMAIN, info=b"aes-256-gcm").derive(ikm)


def mfa_aad(user: int, revision: int, purpose: str, key_id: str) -> bytes:
    return (AAD_DOMAIN + b"\x01" + user.to_bytes(8, "big", signed=True) + revision.to_bytes(8, "big", signed=True)
            + len(purpose).to_bytes(2, "big") + purpose.encode() + len(key_id).to_bytes(2, "big") + key_id.encode())


def encrypt_totp(key_id: str, key: bytes, user: int, revision: int, secret: bytes) -> bytes:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    nonce = os.urandom(12)
    return b"\x01" + nonce + AESGCM(key).encrypt(nonce, secret, mfa_aad(user, revision, "totp-factor", key_id))


def decrypt_totp(key: bytes, key_id: str, user: int, revision: int, blob: bytes) -> bytes:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    if blob[0] != 1:
        fail("mfa_envelope_invalid")
    return AESGCM(key).decrypt(blob[1:13], blob[13:], mfa_aad(user, revision, "totp-factor", key_id))


def legacy_totp_secret(text: str) -> bytes:
    text = text.strip().upper().replace(" ", "")
    secret = base64.b32decode(text + "=" * (-len(text) % 8))
    if len(secret) != 20:
        fail("mfa_secret_length_unsupported")
    return secret


def plan_mfa(a, src, tgt):
    schema, stats, sql = a.source_schema, Counter(), []
    factors = query(src, "SELECT id,user_id,secret,is_enabled,last_used_at,deleted_at,"
                         "floor(extract(epoch FROM last_used_at))::bigint AS last_used FROM two_fas ORDER BY id", schema)
    codes = query(src, "SELECT user_id,is_used,deleted_at FROM two_fa_backup_codes", schema)
    required = {r["user_id"] for r in query(tgt, "SELECT user_id FROM core_identity.user_profiles "
                                                  "WHERE second_factor_required")}
    enabled = [f for f in factors if f["is_enabled"] and f["deleted_at"] is None]
    if {f["user_id"] for f in enabled} != required:
        fail("mfa_requirement_mismatch")
    stats["legacy_factors"] = len(factors)
    stats["legacy_factors_not_enabled"] = len(factors) - len(enabled)
    if enabled:
        if a.mfa_keyring is None:
            fail("mfa_keyring_required")
        key_id, key = load_mfa_key(a.mfa_keyring)
        for f in enabled:
            secret = legacy_totp_secret(f["secret"])
            blob = encrypt_totp(key_id, key, f["user_id"], 1, secret)
            if decrypt_totp(key, key_id, f["user_id"], 1, blob) != secret:
                fail("mfa_round_trip_failed")
            step = f["last_used"] // 30 if f["last_used"] else None
            sql.append("INSERT INTO core_identity.mfa_factors(user_id,revision,enabled,key_id,encrypted_secret,"
                       f"last_used_step) VALUES({f['user_id']},1,TRUE,{lit(key_id)},'\\x{blob.hex()}'::bytea,{lit(step)});")
            stats["factors_imported"] += 1
        stats["mfa_key_id"] = key_id
    # Legacy backup codes are bcrypt; Core recovery verifiers are PHC (Argon2).
    live_codes = [c for c in codes if not c["is_used"] and c["deleted_at"] is None]
    stats["backup_codes_not_imported"] = len(codes)
    stats["backup_codes_unused_for_enabled_factors"] = sum(
        1 for c in live_codes if c["user_id"] in {f["user_id"] for f in enabled})
    sql.append("DO $$BEGIN IF EXISTS(SELECT 1 FROM core_identity.user_profiles p LEFT JOIN core_identity.mfa_factors f "
               "ON f.user_id=p.user_id WHERE p.second_factor_required IS DISTINCT FROM COALESCE(f.enabled,FALSE)) "
               "THEN RAISE EXCEPTION 'mfa_policy_mismatch'; END IF; END$$;")
    return sql, dict(stats)


# -------------------------------------------------------------------- passkeys
COSE_ES256 = bytes.fromhex("a5010203262001215820")


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def passkey_json(row) -> tuple[bytes, dict]:
    cred_id = base64.b64decode(row["credential_id"], validate=True)
    cose = base64.b64decode(row["public_key"], validate=True)
    if not (len(cose) == 77 and cose.startswith(COSE_ES256) and cose[42:45] == bytes.fromhex("225820")):
        fail("passkey_cose_key_unsupported")
    transports = [t for t in json.loads(row["transports"] or "[]")
                  if t in ("usb", "nfc", "ble", "internal", "hybrid", "test")]
    cred = {"cred_id": b64url(cred_id),
            "cred": {"type_": "ES256", "key": {"EC_EC2": {"curve": "SECP256R1", "x": b64url(cose[10:42]),
                                                          "y": b64url(cose[45:77])}}},
            "counter": row["sign_count"] or 0, "transports": transports or None,
            "user_verified": bool(row["user_verified"]), "backup_eligible": bool(row["backup_eligible"]),
            "backup_state": bool(row["backup_state"]), "registration_policy": "required",
            "extensions": {"cred_protect": "NotRequested", "hmac_create_secret": "NotRequested",
                           "appid": "NotRequested", "cred_props": "NotRequested"},
            "attestation": {"data": "None", "metadata": "None"}, "attestation_format": "none"}
    return cred_id, {"cred": cred}


def plan_passkeys(a, src, tgt):
    rows = query(src, "SELECT * FROM passkey_credentials ORDER BY id", a.source_schema)
    users = {r["id"] for r in query(tgt, "SELECT id FROM core_identity.users")}
    stats, sql, handles = Counter(), [], set()
    for r in rows:
        if r["user_id"] not in users:
            stats["passkeys_user_missing"] += 1
            continue
        cred_id, key = passkey_json(r)
        name = (r["name"] or "").strip()[:64] or f"Passkey {r['id']}"
        handles.add(r["user_id"])
        sql.append("INSERT INTO core_passkeys.credentials(user_id,credential_id,passkey,name,created_at,last_used_at,"
                   f"revoked_at) VALUES({r['user_id']},'\\x{cred_id.hex()}'::bytea,{jlit(key)},{lit(name)},"
                   f"{lit(r['created_at'])},{lit(r['last_used_at'])},{lit(r['deleted_at'])});")
        # Legacy go-webauthn used the decimal user ID as the WebAuthn user handle.
        sql.append("INSERT INTO core_history.legacy_passkey_handles(credential_id,user_id,user_handle) VALUES("
                   f"'\\x{cred_id.hex()}'::bytea,{r['user_id']},'\\x{str(r['user_id']).encode().hex()}'::bytea);")
        stats["passkeys_revoked" if r["deleted_at"] else "passkeys_active"] += 1
    sql[:0] = [f"INSERT INTO core_passkeys.user_handles(user_id) VALUES({u}) ON CONFLICT DO NOTHING;"
               for u in sorted(handles)]
    stats["user_handles"] = len(handles)
    stats["legacy_user_handles"] = sum(1 for r in rows if r["user_id"] in users)
    return sql, dict(stats)


# -------------------------------------------------------------------- telegram
def plan_telegram(a, src, tgt):
    rows = query(src, "SELECT id,telegram_id,deleted_at FROM users WHERE COALESCE(telegram_id,'')<>'' ORDER BY id",
                 a.source_schema)
    stats, sql, seen = Counter(), [], set()
    for r in rows:
        subject = r["telegram_id"].strip()
        if not re.fullmatch(r"[1-9][0-9]{0,18}", subject) or int(subject) > (1 << 63) - 1 or subject in seen:
            stats["telegram_invalid_or_duplicate"] += 1
            continue
        seen.add(subject)
        sql.append(f"INSERT INTO core_telegram_auth.bindings(subject,user_id) VALUES({lit(subject)},{r['id']});")
        stats["telegram_bindings"] += 1
    return sql, dict(stats)


# ------------------------------------------------------------ channels_history
def plan_channels(a, src, tgt):
    """Opening display baselines for counters with a proved unit; cached balances.

    Legacy channel used_quota was not rebased by the 2026-10 wallet credit
    rebase, so a counter that started before that snapshot mixes units and has
    no proved K (docs/channel-openings.md: unknown K -> no baseline). Channels
    created after the rebase snapshot accumulated only 500000/USD units.
    """
    schema = a.source_schema
    channels = query(src, "SELECT * FROM channels ORDER BY id", schema)
    rebases = query(src, "SELECT (plan->>'snapshot_at')::bigint AS at,plan->>'usd_credit_conversion' AS k "
                         "FROM wallet_credit_rebases", schema)
    boundary = a.credit_rebase_boundary or max((r["at"] for r in rebases), default=0)
    if any(r["k"] != str(CREDITS_PER_USD) for r in rebases):
        fail("channel_rebase_unit_unexpected")
    identities = {r["id"] for r in query(tgt, "SELECT id FROM core_channels.identities WHERE retired_at IS NULL")}
    missing = [c["id"] for c in channels if c["id"] not in identities]
    if missing:
        fail("channel_identities_missing:" + ",".join(map(str, missing)))
    stats, rows, balances = Counter(), [], []
    for c in channels:
        row_sha = sha(c)
        if (c["created_time"] or 0) > boundary:
            off = Fraction(c["used_quota"] or 0, CREDITS_PER_USD)
            rows.append({"channel_id": c["id"], "source_channel_id": c["id"], "raw_used_quota": c["used_quota"] or 0,
                         "credits_per_usd": CREDITS_PER_USD, "offset_numerator": str(off.numerator),
                         "offset_denominator": str(off.denominator), "historical_coverage": False,
                         "source_row_sha256": row_sha})
            stats["baselines_post_rebase"] += 1
        else:
            stats["baselines_unit_unproven"] += 1
            stats["unit_unproven_raw_used_quota"] += c["used_quota"] or 0
        if (c["balance_updated_time"] or 0) > 0:
            balances.append({"channel_id": c["id"], "balance": str(c["balance"]),
                             "balance_updated_time": c["balance_updated_time"]})
    material = {"archive": a.archive_sha256, "schema": schema, "boundary": boundary,
                "channels": [[c["id"], sha(c)] for c in channels]}
    mapping = [[c["id"], c["id"]] for c in channels]
    evidence = {"contract": "legacy-channel-opening-v1", "native_overlap": "disjoint",
                "basis_kind": "captured_counter_unit", "rule": "created_after_credit_rebase_snapshot",
                "credits_per_usd": CREDITS_PER_USD, "boundary": boundary}
    source = {"archive_sha256": a.archive_sha256, "export_sha256": sha(material), "mapping_sha256": sha(mapping),
              "unit_evidence_sha256": sha(evidence), "source_revision": f"legacy-schema:{schema}",
              "observed_at": a.observed_at, "native_overlap": "disjoint", "rows_sha256": sha(rows)}
    source["source_key"] = sha(source)
    source["plan_sha256"] = sha({"source": source, "rows": rows, "balances": balances})
    cols = ("source_key", "archive_sha256", "export_sha256", "mapping_sha256", "unit_evidence_sha256",
            "source_revision", "observed_at", "native_overlap", "rows_sha256", "plan_sha256")
    sql = ["INSERT INTO core_channels.opening_sources(" + ",".join(cols) + ",native_request_ids,native_seal_sha256) "
           "SELECT " + ",".join(lit(source[c]) for c in cols) + ",ids,encode(sha256(convert_to(array_to_string(ids,"
           "E'\\n'),'UTF8')),'hex') FROM (SELECT ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE \"C\") ids) x;"]
    for r in rows:
        sql.append("INSERT INTO core_channels.opening_baselines(channel_id,source_key,source_channel_id,raw_used_quota,"
                   "credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind) VALUES("
                   f"{r['channel_id']},{lit(source['source_key'])},{r['source_channel_id']},{r['raw_used_quota']},"
                   f"{r['credits_per_usd']},{r['offset_numerator']},{r['offset_denominator']},FALSE,'imported');")
    for b in balances:
        sql.append("UPDATE core_channels.balance_observations SET balance=" + lit(b["balance"]) + "::float8,"
                   f"unit_label=NULL,observed_at=to_timestamp({b['balance_updated_time']}),kind='copied',"
                   "credential_id=NULL,credential_revision=NULL,source_channel_id=NULL,recorded_at=transaction_timestamp() "
                   f"WHERE channel_id={b['channel_id']} AND kind='unknown' AND balance IS NULL;")
    stats.update(channels=len(channels), rebase_boundary=boundary, balance_candidates=len(balances),
                 baseline_raw_used_quota=sum(r["raw_used_quota"] for r in rows))
    return sql, dict(stats)
