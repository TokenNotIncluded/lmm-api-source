#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Read-only audit of legacy acquisition daily bounds against success receipts.

By default only aggregate counts leave the restored local database. An explicit
--projection writes daily bounds and user IDs to a private Backups file; it never
exports browser identifiers, request IDs or log metadata. Exit 2 means the
stored activity cache differs from the consent-scoped request reconstruction.
This does not import data or claim activity coverage is complete.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def audit_sql(schema, include_projection=False):
    if not re.fullmatch(r"[a-z_][a-z_0-9]*", schema):
        raise ValueError("invalid legacy schema")
    s = '"' + schema + '"'
    projection_sql = ""
    if include_projection:
        projection_sql = f""", 'projection',json_build_object(
          'window',(SELECT row_to_json(state) FROM state),
          'reconstructed',coalesce((SELECT json_agg(expected ORDER BY user_id,day) FROM expected),'[]'::json),
          'stored',coalesce((SELECT json_agg(saved ORDER BY user_id,day) FROM (SELECT user_id,day,first_at,last_at FROM {s}.acquisition_activities) saved),'[]'::json),
          'gaps',coalesce((SELECT json_agg(g ORDER BY from_at) FROM (SELECT from_at,through_at FROM {s}.acquisition_activity_gaps) g),'[]'::json))"""
    return f"""BEGIN READ ONLY;
WITH state AS (SELECT started_at,scanned_through FROM {s}.acquisition_activity_states WHERE id=1),
success AS (
 SELECT l.user_id,l.request_id,min(l.created_at) AS at FROM {s}.logs l CROSS JOIN state s
 WHERE l.type=2 AND l.token_id>0 AND coalesce(l.request_id,'')<>''
 AND l.created_at>=s.started_at AND l.created_at<s.scanned_through
 AND CASE WHEN pg_input_is_valid(l.other,'jsonb')
     THEN l.other::jsonb->'acquisition_success_v1'='true'::jsonb ELSE false END
 GROUP BY l.user_id,l.request_id
), expected AS (
 SELECT e.user_id,(e.at/86400)*86400 AS day,min(e.at) AS first_at,max(e.at) AS last_at
 FROM success e JOIN {s}.acquisition_accounts a ON a.user_id=e.user_id
 JOIN {s}.acquisition_consents c ON c.user_id=e.user_id JOIN {s}.users u ON u.id=e.user_id CROSS JOIN state s
 WHERE a.consent_version>=2 AND c.allowed AND c.version>=2
 AND u.created_at>=s.started_at AND u.role<10 AND u.deleted_at IS NULL
 GROUP BY e.user_id,(e.at/86400)*86400
), missing AS (
 SELECT * FROM expected EXCEPT SELECT user_id,day,first_at,last_at FROM {s}.acquisition_activities
), extra AS (
 SELECT user_id,day,first_at,last_at FROM {s}.acquisition_activities EXCEPT SELECT * FROM expected
)
SELECT json_build_object('contract','legacy-acquisition-activity-audit.v1',
 'state_rows',(SELECT count(*) FROM state),
 'state_incomplete',(SELECT incomplete FROM {s}.acquisition_activity_states WHERE id=1),
 'recorded_gaps',(SELECT count(*) FROM {s}.acquisition_activity_gaps),
 'reconstructed_rows',(SELECT count(*) FROM expected),
 'stored_rows',(SELECT count(*) FROM {s}.acquisition_activities),
 'missing_or_changed_rows',(SELECT count(*) FROM missing),
 'extra_or_changed_rows',(SELECT count(*) FROM extra){projection_sql});
ROLLBACK;
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', required=True, type=Path)
    parser.add_argument('--port', type=int, required=True)
    parser.add_argument('--database', required=True)
    parser.add_argument('--schema', required=True)
    parser.add_argument('--report', type=Path)
    parser.add_argument('--projection', type=Path, help='private daily reconstruction and preserved cache evidence; never applied automatically')
    args = parser.parse_args()
    backup = (Path.home() / 'Backups').resolve()
    socket = args.socket.resolve(strict=True)
    if not socket.is_relative_to(backup) or not socket.is_dir() or not 1 <= args.port <= 65535:
        parser.error('requires a local restored database socket under Backups')
    if not re.fullmatch(r"[a-z_][a-z_0-9]*", args.database):
        parser.error("database must be a local database name, not a connection string")
    for path in (args.report, args.projection):
        if path and (not path.resolve().is_relative_to(backup) or path.exists()):
            parser.error('output must be a new private file under Backups')
    if args.report and args.projection and args.report.resolve() == args.projection.resolve():
        parser.error('report and projection require different files')
    sql = audit_sql(args.schema, args.projection is not None)
    env = {k: v for k, v in os.environ.items() if not k.startswith('PG')}
    env['PGOPTIONS'] = '-c default_transaction_read_only=on -c statement_timeout=60000'
    result = subprocess.run(['psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '--host', str(socket), '--port', str(args.port), '--username', 'postgres', '--dbname', args.database], input=sql, text=True, capture_output=True, env=env, timeout=90)
    if result.returncode:
        raise SystemExit('local read-only activity audit failed; no report accepted')
    report = json.loads(result.stdout)
    projection = report.pop("projection", None)
    report['matches'] = report['state_rows'] == 1 and report['missing_or_changed_rows'] == 0 and report['extra_or_changed_rows'] == 0
    report['coverage_complete'] = report['matches'] and report['state_incomplete'] is False and report['recorded_gaps'] == 0
    if args.projection:
        if report['state_rows'] != 1 or projection is None:
            raise SystemExit('one activity state is required for a bounded projection')
        projection.update(contract='legacy-acquisition-activity-projection.v1', source_schema=args.schema, audit=dict(report))
        payload = json.dumps(projection, sort_keys=True, separators=(',', ':')).encode()
        digest = hashlib.sha256(payload).hexdigest()
        with os.fdopen(os.open(args.projection.resolve(), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb') as stream:
            stream.write(payload + b'\n')
        report['projection_sha256'] = digest
    output = json.dumps(report, sort_keys=True) + '\n'
    if args.report:
        path = args.report.resolve()
        if not path.is_relative_to(backup):
            parser.error('report must stay under Backups')
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as stream:
            stream.write(output)
    print(output, end='')
    raise SystemExit(0 if report['coverage_complete'] else 2)


if __name__ == '__main__':
    main()
