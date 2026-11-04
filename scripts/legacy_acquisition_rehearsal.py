#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Copy acquisition data into a fresh, local audit database only.

This is a rehearsal, not production installation or cutover. The source is read
in one repeatable-read snapshot. Original preferences, attribution and cached
activities are copied exactly. Reconstructed daily activities remain evidence;
this tool marks the imported activity state incomplete when the cache differs.
No balances, credentials, requests or Core database rows are written.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from legacy_acquisition_activity_audit import audit_sql

TABLES = ('config', 'cohort', 'links', 'visitors', 'visits', 'accounts', 'consents',
          'self_reports', 'attribution_policies', 'correction_heads', 'corrections',
          'costs', 'first_payments', 'activities', 'activity_state', 'activity_gaps')
SOURCE = {t: 'acquisition_' + t for t in TABLES}
SOURCE.update(config='acquisition_configs', activity_state='acquisition_activity_states')
SEQUENCES = ('visits', 'attribution_policies', 'corrections', 'costs')


def snapshot_sql(schema):
    audit = audit_sql(schema, True).replace('BEGIN READ ONLY;', '').replace('ROLLBACK;', '')
    expressions = []
    for table in TABLES:
        if table == 'cohort':
            select = f"SELECT id AS user_id,created_at AS registered_at,role>=10 AS staff,deleted_at IS NOT NULL AS deleted,'legacy_import'::text AS origin FROM {schema}.users"
        elif table in ('config', 'activity_state'):
            select = f"SELECT (to_jsonb(t)-'id')||jsonb_build_object('singleton',true) AS row FROM {schema}.{SOURCE[table]} t WHERE id=1"
        else:
            select = f"SELECT to_jsonb(t) AS row FROM {schema}.{SOURCE[table]} t"
        rows = f"SELECT to_jsonb(c) AS row FROM ({select}) c" if table == 'cohort' else select
        expressions.append(f"'{table}',coalesce((SELECT jsonb_agg(row) FROM ({rows}) q),'[]'::jsonb)")
    return 'BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\nSELECT jsonb_build_object(' + ','.join(expressions) + ');\n' + audit + '\nROLLBACK;\n'


def literal(value):
    return "'" + json.dumps(value, ensure_ascii=False, separators=(',', ':')).replace("'", "''") + "'::jsonb"


def import_sql(data):
    if set(data) != set(TABLES) or len(data['config']) != 1 or len(data['activity_state']) != 1:
        raise ValueError('complete acquisition snapshot with one configuration/state required')
    statements = ["BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL standard_conforming_strings=on;",
                  "CREATE TEMP TABLE acquisition_payload(doc jsonb) ON COMMIT DROP;",
                  'INSERT INTO acquisition_payload VALUES(' + literal(data) + ');',
                  "DO $acquisition$ DECLARE payload jsonb; BEGIN SELECT doc INTO payload FROM acquisition_payload;",
                  "IF (SELECT count(*) FROM lmm_acquisition.contract)<>1 OR NOT EXISTS(SELECT 1 FROM lmm_acquisition.contract WHERE singleton AND version=1) THEN RAISE EXCEPTION 'acquisition contract mismatch'; END IF;"]
    for table in TABLES:
        if table != 'config':
            statements.append(f"IF EXISTS(SELECT 1 FROM lmm_acquisition.{table}) THEN RAISE EXCEPTION 'audit target is not empty'; END IF;")
    statements.append('DELETE FROM lmm_acquisition.config;')
    for table in TABLES:
        # Reject dropped/added fields, including fields json_populate_recordset
        # would otherwise silently ignore or fill with NULL.
        statements.append(f"""IF EXISTS(SELECT 1 FROM jsonb_array_elements(payload->'{table}') r WHERE
          ARRAY(SELECT jsonb_object_keys(r) ORDER BY 1) IS DISTINCT FROM
          ARRAY(SELECT a.attname::text FROM pg_attribute a WHERE a.attrelid='lmm_acquisition.{table}'::regclass AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attname))
          THEN RAISE EXCEPTION 'acquisition row shape mismatch: {table}'; END IF;
          INSERT INTO lmm_acquisition.{table} SELECT * FROM jsonb_populate_recordset(NULL::lmm_acquisition.{table},payload->'{table}');
          IF (SELECT count(*) FROM lmm_acquisition.{table})<>jsonb_array_length(payload->'{table}') OR EXISTS(
          SELECT to_jsonb(t) FROM lmm_acquisition.{table} t EXCEPT SELECT value FROM jsonb_array_elements(payload->'{table}'))
          THEN RAISE EXCEPTION 'acquisition row readback mismatch: {table}'; END IF;""")
    statements.append('END $acquisition$;')
    for table in SEQUENCES:
        statements.append(f"DO $sequence$ DECLARE seq text; next_id bigint; BEGIN SELECT pg_get_serial_sequence('lmm_acquisition.{table}','id'),coalesce(max(id),0)+1 INTO seq,next_id FROM lmm_acquisition.{table}; EXECUTE format('ALTER SEQUENCE %s RESTART WITH %s',seq,next_id); END $sequence$;")
    statements.append('COMMIT;')
    return '\n'.join(statements)


def target_summary_sql():
    counts = ','.join(f"'{table}',(SELECT count(*) FROM lmm_acquisition.{table})" for table in TABLES)
    return "SELECT json_build_object('counts',json_build_object(" + counts + "),'activity_status',(SELECT status FROM lmm_acquisition.activity_state WHERE singleton),'activity_incomplete',(SELECT incomplete FROM lmm_acquisition.activity_state WHERE singleton));"


def psql(socket, port, database, sql, readonly):
    env = {k: v for k, v in os.environ.items() if not k.startswith('PG')}
    env['PGOPTIONS'] = '-c statement_timeout=60000' + (' -c default_transaction_read_only=on' if readonly else '')
    result = subprocess.run(['psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate', '--host', str(socket), '--port', str(port), '--username', 'postgres', '--dbname', database], input=sql, text=True, capture_output=True, env=env, timeout=90)
    if result.returncode:
        state = re.search(r'ERROR:\s+([A-Z0-9]{5})', result.stderr)
        raise RuntimeError('local acquisition rehearsal failed; SQLSTATE=' + (state[1] if state else 'unknown'))
    return result.stdout


def decode_snapshot_documents(exported):
    decoder = json.JSONDecoder()
    data, offset = decoder.raw_decode(exported.lstrip())
    remainder = exported.lstrip()[offset:].lstrip()
    audit, offset = decoder.raw_decode(remainder)
    if remainder[offset:].strip():
        raise ValueError('unexpected source result after the two snapshot documents')
    return data, audit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', required=True, type=Path)
    parser.add_argument('--port', required=True, type=int)
    parser.add_argument('--source', default='legacy')
    parser.add_argument('--schema', required=True)
    parser.add_argument('--target', required=True)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    backup = (Path.home() / '.local/state/lmm-api').resolve()
    socket = args.socket.resolve(strict=True)
    if not socket.is_relative_to(backup) or not socket.is_dir() or not 1 <= args.port <= 65535:
        parser.error('local restored database socket under private evidence directory required')
    if not re.fullmatch(r'[a-z_][a-z_0-9]*', args.source) or not re.fullmatch(r'lmm_acquisition_audit_[a-z0-9_]+', args.target) or args.source == args.target:
        parser.error('requires a separate lmm_acquisition_audit_* target and plain database names')
    report_path = args.report.resolve()
    if not report_path.is_relative_to(backup) or report_path.exists():
        parser.error('report must be a new private file under private evidence directory')
    exported = psql(socket, args.port, args.source, snapshot_sql(args.schema), True)
    data, audit = decode_snapshot_documents(exported)
    projection = audit.pop('projection')
    report = {'mode': 'execute' if args.execute else 'plan', 'source_counts': {t: len(data[t]) for t in TABLES}, 'activity_audit': audit,
              'snapshot_sha256': hashlib.sha256(json.dumps(data, sort_keys=True, separators=(',', ':')).encode()).hexdigest()}
    if args.execute:
        # Exact-copy readback runs before recording the known cache gap; source
        # rows remain unchanged and the private audit preserves the discrepancy.
        sql = import_sql(data).removesuffix('COMMIT;')
        if audit['missing_or_changed_rows'] or audit['extra_or_changed_rows']:
            window = projection['window']
            sql += f"\nUPDATE lmm_acquisition.activity_state SET incomplete=true,status='legacy_mismatch',gap_from={int(window['started_at'])},gap_through={int(window['scanned_through'])} WHERE singleton;\n"
            sql += f"INSERT INTO lmm_acquisition.activity_gaps(from_at,through_at) VALUES({int(window['started_at'])},{int(window['scanned_through'])}) ON CONFLICT(from_at) DO UPDATE SET through_at=greatest(lmm_acquisition.activity_gaps.through_at,EXCLUDED.through_at);\n"
        result = psql(socket, args.port, args.target, sql + '\n' + target_summary_sql() + '\nCOMMIT;', False)
        report['target_readback'] = json.loads(result)
        report['imported'] = True
    with os.fdopen(os.open(report_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as stream:
        json.dump(report, stream, sort_keys=True);stream.write('\n')
    print(json.dumps(report, sort_keys=True))


if __name__ == '__main__':
    main()
