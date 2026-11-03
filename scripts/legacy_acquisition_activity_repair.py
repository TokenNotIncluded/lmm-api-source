#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Repair verified missing daily rows in an offline local acquisition audit DB.

Refuses changed/extra rows, changed consent and an already modified target.
Preserves coverage gaps and never marks the whole migration complete.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
from legacy_acquisition_rehearsal import literal, psql


def validate_projection(doc):
    if doc.get('contract') != 'legacy-acquisition-activity-projection.v1':
        raise ValueError('activity projection contract mismatch')
    window = doc['window']
    start, end = window['started_at'], window['scanned_through']
    if type(start) is not int or type(end) is not int or not 0 < start < end <= 253402300799:
        raise ValueError('invalid reconstruction window')
    sets = []
    for key in ('stored', 'reconstructed'):
        values, identities = set(), set()
        if len(doc[key]) > 10000:
            raise ValueError('activity projection exceeds row bound')
        for row in doc[key]:
            if set(row) != {'user_id', 'day', 'first_at', 'last_at'} or any(type(v) is not int for v in row.values()):
                raise ValueError('activity row shape mismatch')
            uid, day, first, last = (row[k] for k in ('user_id', 'day', 'first_at', 'last_at'))
            if uid <= 0 or not start <= first <= last < end or day != first // 86400 * 86400 or day != last // 86400 * 86400 or (uid, day) in identities:
                raise ValueError('invalid or duplicate activity row')
            identities.add((uid, day));values.add((uid, day, first, last))
        sets.append(values)
    if not sets[0] <= sets[1] or len(sets[0]) == len(sets[1]):
        raise ValueError('only strictly missing rows can be repaired')
    return len(sets[1] - sets[0])


def repair_sql(doc):
    missing = validate_projection(doc)
    return f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL standard_conforming_strings=on;
SET LOCAL lock_timeout='5s';
LOCK TABLE lmm_acquisition.activity_state,lmm_acquisition.accounts,lmm_acquisition.consents,lmm_acquisition.activities,lmm_acquisition.config,lmm_acquisition.cohort IN SHARE ROW EXCLUSIVE MODE;
CREATE TEMP TABLE activity_repair_payload(doc jsonb) ON COMMIT DROP;
INSERT INTO activity_repair_payload VALUES({literal(doc)});
DO $repair$ DECLARE p jsonb; BEGIN SELECT doc INTO p FROM activity_repair_payload;
IF NOT EXISTS(SELECT 1 FROM lmm_acquisition.contract WHERE singleton AND version=1)
 OR NOT EXISTS(SELECT 1 FROM lmm_acquisition.activity_state WHERE singleton AND status='legacy_mismatch' AND incomplete
 AND started_at=(p->'window'->>'started_at')::bigint AND scanned_through=(p->'window'->>'scanned_through')::bigint)
 THEN RAISE EXCEPTION 'requires unchanged legacy acquisition rehearsal'; END IF;
IF (SELECT count(*) FROM lmm_acquisition.activities)<>jsonb_array_length(p->'stored') OR EXISTS(
 SELECT to_jsonb(a) FROM lmm_acquisition.activities a EXCEPT SELECT value FROM jsonb_array_elements(p->'stored'))
 THEN RAISE EXCEPTION 'activity cache changed since projection'; END IF;
IF EXISTS(SELECT 1 FROM jsonb_to_recordset(p->'reconstructed') r(user_id bigint,day bigint,first_at bigint,last_at bigint)
 LEFT JOIN lmm_acquisition.accounts a USING(user_id) LEFT JOIN lmm_acquisition.consents c USING(user_id) LEFT JOIN lmm_acquisition.cohort u USING(user_id)
 WHERE a.user_id IS NULL OR a.consent_version<2 OR c.user_id IS NULL OR NOT c.allowed OR c.version<2
 OR u.user_id IS NULL OR u.staff OR u.deleted OR u.registered_at<(p->'window'->>'started_at')::bigint)
 THEN RAISE EXCEPTION 'activity eligibility changed since projection'; END IF;
INSERT INTO lmm_acquisition.activities SELECT r.* FROM jsonb_populate_recordset(NULL::lmm_acquisition.activities,p->'reconstructed') r
 WHERE NOT EXISTS(SELECT 1 FROM lmm_acquisition.activities a WHERE a.user_id=r.user_id AND a.day=r.day);
IF (SELECT count(*) FROM lmm_acquisition.activities)<>jsonb_array_length(p->'reconstructed') OR EXISTS(
 SELECT to_jsonb(a) FROM lmm_acquisition.activities a EXCEPT SELECT value FROM jsonb_array_elements(p->'reconstructed'))
 THEN RAISE EXCEPTION 'reconstructed activity readback mismatch'; END IF;
UPDATE lmm_acquisition.activity_state SET status='legacy_rebuilt',incomplete=true WHERE singleton;
END $repair$;
SELECT json_build_object('repaired_missing_rows',{missing},'daily_rows',(SELECT count(*) FROM lmm_acquisition.activities),
 'activity_status',(SELECT status FROM lmm_acquisition.activity_state WHERE singleton),'coverage_complete',false);
COMMIT;
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', type=Path, required=True)
    parser.add_argument('--port', type=int, required=True)
    parser.add_argument('--target', required=True)
    parser.add_argument('--projection', type=Path, required=True)
    parser.add_argument('--projection-sha256', required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    backup = (Path.home() / 'Backups').resolve()
    socket, projection, report = (path.resolve() for path in (args.socket, args.projection, args.report))
    if not socket.is_dir() or not socket.is_relative_to(backup) or not projection.is_relative_to(backup) or not report.is_relative_to(backup) or report.exists():
        parser.error('requires a local socket, private projection and new report under Backups')
    if not 1 <= args.port <= 65535 or not re.fullmatch(r'lmm_acquisition_audit_[a-z0-9_]+', args.target) or not re.fullmatch(r'[0-9a-f]{64}', args.projection_sha256):
        parser.error('requires a plain local audit database name and projection SHA256')
    raw = projection.read_bytes().removesuffix(b'\n')
    if hashlib.sha256(raw).hexdigest() != args.projection_sha256:
        parser.error('projection digest mismatch')
    doc = json.loads(raw)
    count = validate_projection(doc)
    result = {'mode': 'execute' if args.execute else 'plan', 'missing_rows': count, 'projection_sha256': args.projection_sha256, 'coverage_complete': False}
    if args.execute:
        result['target_readback'] = json.loads(psql(socket, args.port, args.target, repair_sql(doc), False))
    with os.fdopen(os.open(report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as stream:
        json.dump(result, stream, sort_keys=True);stream.write('\n')
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
