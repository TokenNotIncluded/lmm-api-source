#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Preserve exact legacy check-in history in an offline owner-only receipt.

No reward is replayed: opening balances already include historical awards.
Deleted-user rows remain archival facts, never mapped to another account.
This receipt does not enable native check-ins or establish a reward timezone.
"""
import argparse
from datetime import date
import hashlib
import json
import os
from pathlib import Path
import re

from prepare_wallet_history_snapshot import canonical, digest, require
from owner_history_receipt import owner_history_import_sql

SCOPE = 'checkin-history.v1'
NAMESPACE = 'lmm_api.lmm_prod_20260802.checkins'
FIELDS = {'id', 'user_id', 'checkin_date', 'quota_awarded', 'created_at'}
JS_SAFE = 9007199254740991


def prepare(export, *, expected_rows, archive_sha256):
    require(re.fullmatch('[0-9a-f]{64}', archive_sha256) is not None, 'archive_hash_invalid')
    require(export.get('source_namespace') == NAMESPACE, 'source_namespace_mismatch')
    require(type(expected_rows) is int and expected_rows >= 0, 'expected_count_invalid')
    rows, users = export.get('rows'), export.get('source_user_ids')
    require(type(rows) is list and len(rows) == expected_rows == export.get('source_row_count'), 'source_row_count_mismatch')
    require(type(users) is list and all(type(u) is int and 0 < u <= JS_SAFE for u in users)
            and len(users) == len(set(users)), 'source_users_invalid')
    observed = export.get('observed_at')
    require(type(observed) is int and 0 < observed <= JS_SAFE, 'observation_invalid')
    settings = export.get('settings')
    keys = {'checkin_setting.' + k for k in ('enabled', 'min_quota', 'max_quota', 'level_multipliers')}
    require(type(settings) is dict and set(settings) == keys
            and all(type(v) is str for v in settings.values()), 'reward_settings_incomplete')
    # Preserve exact option text; do not replace a random reward range with an average.
    known_users, ids, days, records = set(users), set(), set(), []
    for row in rows:
        require(type(row) is dict and set(row) == FIELDS, 'record_shape_invalid')
        require(all(type(row[k]) is int and 0 < row[k] <= JS_SAFE for k in ('id', 'user_id')), 'record_id_invalid')
        require(type(row['quota_awarded']) is int and 0 <= row['quota_awarded'] <= JS_SAFE, 'record_award_invalid')
        require(type(row['created_at']) is int and 0 <= row['created_at'] <= observed, 'record_time_invalid')
        day = row['checkin_date']
        require(type(day) is str and re.fullmatch(r'\d{4}-\d{2}-\d{2}', day) is not None, 'record_date_invalid')
        try:
            date.fromisoformat(day)
        except ValueError:
            raise ValueError('record_date_invalid') from None
        require(row['id'] not in ids and (row['user_id'], day) not in days, 'duplicate_checkin')
        ids.add(row['id']); days.add((row['user_id'], day))
        records.append({'record': row, 'source_row_sha256': digest(row),
                        'source_user_present': row['user_id'] in known_users})
    manifest = {'scope': SCOPE, 'source_namespace': NAMESPACE,
                'migration_id': 'checkin-history-v1-' + hashlib.sha256(NAMESPACE.encode()).hexdigest()[:32],
                'source_archive_sha256': archive_sha256, 'source_export_sha256': digest(export),
                'observed_at': observed, 'source_row_count': expected_rows,
                'source_user_ids': sorted(users), 'mapping_sha256': digest(sorted(users)),
                'settings': settings, 'rows': sorted(records, key=lambda r: r['record']['id']),
                'orphan_rows': sum(not r['source_user_present'] for r in records),
                'award_unit': 'legacy_raw_no_conversion', 'balance_effect': 'none',
                'history_complete': True, 'native_checkin_enabled_by_import': False,
                'legacy_calendar_timezone': None}
    return manifest


def import_sql(manifest):
    return owner_history_import_sql(manifest, scope=SCOPE, error_prefix='checkin_history')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--input', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--archive-sha256', required=True)
    p.add_argument('--expected-rows', type=int, required=True)
    args = p.parse_args()
    manifest = prepare(json.loads(args.input.read_text()), expected_rows=args.expected_rows, archive_sha256=args.archive_sha256)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    for name, value in [('manifest.private.json', canonical(manifest) + '\n'), ('import.private.sql', import_sql(manifest))]:
        with os.fdopen(os.open(args.output / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as f:
            f.write(value)
    print(json.dumps({'source_rows': manifest['source_row_count'], 'orphan_rows_preserved': manifest['orphan_rows'], 'balance_effect': 'none', 'native_checkin_enabled_by_import': False}))


if __name__ == '__main__':
    main()
