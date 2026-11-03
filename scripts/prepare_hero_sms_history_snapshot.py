#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Preserve terminal Hero SMS orders, encrypted payloads and original quota ledgers.

This is archival source data, not native orders or replayed reservations/refunds.
Encrypted payloads are retained verbatim; decrypting/displaying them and native
supplier operations still require their own implementation and acceptance.
"""
import argparse
import json
import os
from pathlib import Path
import re

from owner_history_receipt import owner_history_import_sql
from prepare_wallet_history_snapshot import canonical, digest, require
from verify_legacy_cutover_rehearsal import hero_sms_consistency

SCOPE = 'hero-sms-history.v1'
NAMESPACE = 'lmm_api.lmm_prod_20260802.hero_sms_sms'
ORDERS = 'hero_sms_sms_orders'
LEDGERS = 'hero_sms_sms_quota_ledgers'
JS_SAFE = 9007199254740991


def prepare(export, *, expected_orders, expected_ledgers, archive_sha256):
    require(re.fullmatch('[0-9a-f]{64}', archive_sha256) is not None, 'archive_hash_invalid')
    require(export.get('source_namespace') == NAMESPACE, 'source_namespace_mismatch')
    observed = export.get('observed_at')
    require(type(observed) is int and 0 < observed <= JS_SAFE, 'observation_invalid')
    users, tables = export.get('source_user_ids'), export.get('tables')
    require(type(users) is list and all(type(u) is int and 0 < u <= JS_SAFE for u in users)
            and len(users) == len(set(users)), 'source_users_invalid')
    require(type(tables) is dict and set(tables) == {ORDERS, LEDGERS}, 'source_tables_invalid')
    for table, expected in [(ORDERS, expected_orders), (LEDGERS, expected_ledgers)]:
        rows = tables[table]
        require(type(expected) is int and expected >= 0 and type(rows) is list and len(rows) == expected,
                'source_count_mismatch')
        ids = set()
        for row in rows:
            require(type(row) is dict and row.get('id') is not None and row['id'] not in ids,
                    'duplicate_or_missing_record_id')
            ids.add(row['id'])
            require(type(row.get('user_id')) is int and 0 < row['user_id'] <= JS_SAFE, 'record_user_invalid')
            require(type(row.get('created_at')) is int and 0 <= row['created_at'] <= observed, 'record_time_invalid')
    for row in tables[ORDERS]:
        require(type(row['id']) is str and bool(row['id']), 'order_id_invalid')
        require(row.get('status') in ('completed', 'cancelled', 'failed'), 'nonterminal_sms_order')
        for field in ('reserved_quota', 'refunded_quota', 'charge_quota'):
            require(type(row.get(field)) is int and 0 <= row[field] <= JS_SAFE, 'order_quota_invalid')
    for row in tables[LEDGERS]:
        require(type(row['id']) is int and 0 < row['id'] <= JS_SAFE, 'ledger_id_invalid')
        require(type(row.get('amount_quota')) is int and abs(row['amount_quota']) <= JS_SAFE, 'ledger_quota_invalid')
        require(type(row.get('order_id')) is str and type(row.get('entry_type')) is str, 'ledger_shape_invalid')
    consistency = hero_sms_consistency(tables[ORDERS], tables[LEDGERS])
    require(consistency['passed'], 'source_sms_order_ledger_mismatch')
    known = set(users)
    return {'scope': SCOPE, 'source_namespace': NAMESPACE, 'migration_id': 'hero-sms-history-v1',
            'source_archive_sha256': archive_sha256, 'source_export_sha256': digest(export),
            'observed_at': observed, 'source_user_ids': sorted(users), 'mapping_sha256': digest(sorted(users)),
            'history_complete': True, 'balance_effect': 'none', 'native_supplier_operations_enabled': False,
            'encrypted_payloads': 'original_ciphertext_not_reencrypted_or_display_verified',
            'quota_unit': 'legacy_raw_no_conversion', 'source_order_count': expected_orders,
            'source_ledger_count': expected_ledgers,
            'unmapped_order_ids': sorted(r['id'] for r in tables[ORDERS] if r['user_id'] not in known),
            'tables': {ORDERS: sorted(tables[ORDERS], key=lambda r:r['id']),
                       LEDGERS: sorted(tables[LEDGERS], key=lambda r:r['id'])}}


def import_sql(manifest):
    return owner_history_import_sql(manifest, scope=SCOPE, error_prefix='hero_sms_history')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--input', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--archive-sha256', required=True)
    p.add_argument('--expected-orders', type=int, required=True)
    p.add_argument('--expected-ledgers', type=int, required=True)
    a = p.parse_args()
    m = prepare(json.loads(a.input.read_text()), expected_orders=a.expected_orders,
                expected_ledgers=a.expected_ledgers, archive_sha256=a.archive_sha256)
    a.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    for name, value in [('manifest.private.json', canonical(m)+'\n'), ('import.private.sql', import_sql(m))]:
        with os.fdopen(os.open(a.output/name, os.O_WRONLY|os.O_CREAT|os.O_EXCL, 0o600), 'w') as f:
            f.write(value)
    print(json.dumps({'source_orders':m['source_order_count'], 'source_ledgers':m['source_ledger_count'],
                      'unmapped_orders_preserved':len(m['unmapped_order_ids']), 'balance_effect':'none'}))


if __name__ == '__main__':
    main()
