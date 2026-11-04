#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Immutable discount prerequisites only: no Native rights or capacity writes."""
import hashlib
import json
import re
from collections import Counter

from legacy_cutover_history import query, sha, lit, canonical
from legacy_cutover_money import discount_capacity_holds, topup_can_settle
from owner_history_receipt import owner_history_import_sql
from prepare_wallet_history_snapshot import require

SCOPE = 'legacy-discount-facts.v1'
MIGRATION_ID = 'legacy-discount-facts-v1'
NAMESPACE = 'lmm_api.lmm_prod_20260802.discounts'
# Read old controller/topup_currency.go and topup_canonical_credit.go plus
# their denomination/config dependencies. Never SELECT/export all options.
OPTION_KEYS = ('QuotaPerUnit', 'LegacyPricingQuotaPerUnit', 'CreditsPerUSD',
               'PublicCreditsPerUSD', 'USDExchangeRate',
               'general_setting.quota_display_type', 'DisplayInCurrencyEnabled')


def capacity_holds(tables):
    codes = {r['id'] for r in tables['discount_codes']}
    require(len(codes) == len(tables['discount_codes']), 'discount_code_duplicate')
    orders = tables['discount_top_ups']
    require(len({r['id'] for r in orders}) == len(orders), 'discount_order_duplicate')
    by_trade = {}
    for o in orders:
        require(isinstance(o['trade_no'], str) and o['trade_no'] and
                o['trade_no'] not in by_trade, 'discount_order_trade_duplicate_or_invalid')
        require(o['discount_code_id'] in codes, 'discount_order_code_missing')
        by_trade[o['trade_no']] = o
    seen, ids, holds = set(), set(), []

    def hold(code, trade, order, reservation, reason):
        holds.append(dict(discount_code_id=code, source_order_id=order['id'] if order else None,
                          top_up_trade_no=trade,
                          source_reservation_id=reservation['id'] if reservation else None,
                          source_user_id=(order if order else reservation)['user_id'], reason=reason))

    for r in tables['discount_code_reservations']:
        trade = r['top_up_trade_no']
        require(isinstance(trade, str) and trade and trade not in seen and r['id'] not in ids,
                'discount_reservation_reference_duplicate_or_invalid')
        seen.add(trade)
        ids.add(r['id'])
        require(r['discount_code_id'] in codes, 'discount_reservation_code_missing')
        require(r['status'] in ('reserved', 'released', 'consumed'), 'discount_reservation_status_unknown')
        o = by_trade.get(trade)
        require(o is None or (o['discount_code_id'] == r['discount_code_id'] and
                              o['user_id'] == r['user_id']), 'discount_reservation_order_mismatch')
        closed = o is not None and (o['status'] == 'expired' or (
            o['status'] == 'failed' and ((o['payment_provider'] or '') != 'waffo_pancake' or (
                o['failure_reason_code'] is not None and o['failure_reason_code'] != 'checkout_timeout'))))
        if r['status'] == 'reserved' and not closed:
            hold(r['discount_code_id'], trade, o, r, 'reserved_without_confirmed_closed_order')
        elif r['status'] == 'released' and topup_can_settle(o):
            hold(r['discount_code_id'], trade, o, r, 'released_order_still_payable')
    for o in orders:
        if o['trade_no'] not in seen and topup_can_settle(o):
            hold(o['discount_code_id'], o['trade_no'], o, None, 'unreserved_order_still_payable')
    require(Counter(h['discount_code_id'] for h in holds) == discount_capacity_holds(tables),
            'discount_capacity_rule_mismatch')
    return holds


def material(args, tables, options, source_user_ids):
    require(re.fullmatch('[0-9a-f]{64}', args.archive_sha256) is not None, 'discount_archive_invalid')
    require(type(args.observed_at) is int and args.observed_at > 0, 'discount_observation_invalid')
    require(args.source_schema == 'lmm_prod_20260802', 'discount_source_namespace_mismatch')
    require(all(r['key'] in OPTION_KEYS for r in options) and
            len({r['key'] for r in options}) == len(options), 'discount_options_invalid')
    require(len(set(source_user_ids)) == len(source_user_ids), 'discount_source_users_duplicate')
    holds = capacity_holds(tables)
    redacted = []
    for row in tables['discount_codes']:
        require(isinstance(row['code'], str) and 'code_sha256' not in row, 'discount_code_invalid')
        redacted.append({**{k: v for k, v in row.items() if k != 'code'},
                         'code_sha256': hashlib.sha256(row['code'].strip().upper().encode()).hexdigest()})
    # Keep absent private owners verbatim. Source users bind the owner helper;
    # orphan references remain historical facts, never public entitlements.
    safe_tables = {**tables, 'discount_codes': redacted}
    return dict(scope=SCOPE, migration_id=MIGRATION_ID, source_namespace=NAMESPACE,
                source_archive_sha256=args.archive_sha256, observed_at=args.observed_at,
                source_user_ids=sorted(source_user_ids),
                mapping_sha256=sha([[u, u] for u in sorted(source_user_ids)]),
                tables=json.loads(canonical(safe_tables)),
                source_tables={k: dict(count=len(v), rows_sha256=sha(v)) for k, v in tables.items()},
                options=options, options_sha256=sha(options), capacity_holds=holds,
                balance_effect='none', native_discount_enabled_by_import=False,
                capacity_reserved_by_import=False)


def plan(args, src, tgt):
    require(re.fullmatch('[A-Za-z_][A-Za-z0-9_]*', args.source_schema) is not None,
            'discount_source_schema_invalid')
    schema = args.source_schema
    tables = {
        'discount_codes': query(src, 'SELECT * FROM discount_codes ORDER BY id', schema),
        'discount_code_reservations': query(src, 'SELECT * FROM discount_code_reservations ORDER BY id', schema),
        'discount_top_ups': query(src, 'SELECT id,user_id,trade_no,discount_code_id,status,payment_provider,'
                                 'failure_reason_code FROM top_ups WHERE discount_code_id>0 OR trade_no IN '
                                 '(SELECT top_up_trade_no FROM discount_code_reservations) ORDER BY id', schema),
    }
    options = query(src, 'SELECT key,value FROM options WHERE key IN (' +
                    ','.join(lit(k) for k in OPTION_KEYS) + ') ORDER BY key', schema)
    users = [r['id'] for r in query(src, 'SELECT id FROM users ORDER BY id', schema)]
    manifest = material(args, tables, options, users)
    guard = """DO $discount_facts$ BEGIN
 LOCK TABLE core_migration.import_runs IN SHARE ROW EXCLUSIVE MODE;
 IF EXISTS(SELECT FROM core_migration.import_runs WHERE manifest->>'scope'='legacy-discount-facts.v1')
 THEN RAISE EXCEPTION 'discount_facts_scope_not_empty'; END IF;
END $discount_facts$;"""
    sql = owner_history_import_sql(manifest, scope=SCOPE, error_prefix='discount_facts',
                                   include_transaction=False)
    return [guard, sql], dict(scope=SCOPE, migration_id=MIGRATION_ID,
                             tables={k: len(v) for k, v in tables.items()},
                             capacity_holds=len(manifest['capacity_holds']), balance_effect='none',
                             native_discount_enabled_by_import=False, capacity_reserved_by_import=False)
