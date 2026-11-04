#!/usr/bin/env python3
"""Produce a new owner-only atomic Core55/56 SQL plan; never connect to a database.

Uses exact export/mapping bytes. All retained rows include soft deletes and
revocations. Unknown/missing values remain raw, not fabricated negative facts.
Run the plan only on its bound, stopped new target after identity/opening/keys.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path

MAX_ID = 9007199254740991
COVERAGE = 'all_retained_rows_in_frozen_snapshot_including_soft_deleted_and_revoked'
# Each supported assembly is pinned independently; an unreviewed suffix is rejected.
TARGET_CATALOGS = {
    55: '5376f90af7a78aeb501c4d976d799033a6a88346f28a57b3e5853a3e0e442207',
    56: '46cbd8e0a9e9bb6c55cb899558b40fa8c8df1affa2e366cbbde733a8527bda43',
}
TABLES = ('users', 'tokens', 'oauth_server_grants', 'developer_access_requests')
FIELDS = {
    'users': {'id', 'created_at'},
    'tokens': {'id', 'user_id', 'oauth_managed', 'created_time', 'deleted_at'},
    'oauth_server_grants': {'id', 'user_id', 'created_at_ms', 'revoked_at_ms'},
    'developer_access_requests': {'id', 'user_id', 'created_at', 'reviewed_at', 'status'},
}


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def literal(value):
    if type(value) is int:
        return str(value)
    if not isinstance(value, str) or '\x00' in value:
        raise ValueError('invalid SQL literal')
    return "E'" + value.replace("\\", "\\\\").replace("'", "''") + "'"


def integer(v, low=1, high=MAX_ID):
    if type(v) is not int or not low <= v <= high:
        raise ValueError('invalid integer identity or count')
    return v


def hexhash(v):
    if not isinstance(v, str) or re.fullmatch('[a-f0-9]{64}', v) is None:
        raise ValueError('invalid SHA256')
    return v


def object_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON object key')
        result[key] = value
    return result


def decode(raw):
    return json.loads(raw.decode('utf-8'), object_pairs_hook=object_pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite JSON')))


def plan(raw, metadata, mapping_raw, target, *, opening_migration_id,
         opening_mapping_sha256, identity_migration_id):
    doc, mapping = decode(raw), decode(mapping_raw)
    if doc.get('contract') != 'legacy.connection-source.v1' or metadata.get('contract') != 'legacy.connection-export-binding.v1':
        raise ValueError('unsupported source/binding contract')
    if doc.get('coverage') != COVERAGE or metadata.get('coverage') != COVERAGE:
        raise ValueError('explicit full retained snapshot coverage required')
    if sha(raw) != metadata.get('export_file_sha256'):
        raise ValueError('exact source file hash mismatch')
    archive = hexhash(metadata.get('source_archive_sha256'))
    if not isinstance(doc.get('source_schema'), str) or not doc['source_schema'] or metadata.get('source_schema') != doc['source_schema']:
        raise ValueError('source schema mismatch')
    snapshot_at = integer(metadata.get('snapshot_observed_at'), high=253402300799)
    if snapshot_at > integer(doc.get('observed_at'), high=253402300799):
        raise ValueError('source freeze after export')
    if not isinstance(metadata.get('source_snapshot_id'), str) or not 1 <= len(metadata['source_snapshot_id']) <= 200:
        raise ValueError('snapshot identity required')
    users = set()
    for table in TABLES:
        rows, ids = doc.get(table), set()
        if not isinstance(rows, list) or doc.get('counts', {}).get(table) != len(rows):
            raise ValueError('retained row count mismatch')
        for row in rows:
            if not isinstance(row, dict) or set(row) != FIELDS[table]:
                raise ValueError('unexpected/missing source columns')
            rid = row['id']
            if table == 'oauth_server_grants':
                if not isinstance(rid, str) or not 1 <= len(rid) <= 128:
                    raise ValueError('invalid source grant identity')
            else:
                integer(rid)
            if rid in ids:
                raise ValueError('duplicate retained source row')
            ids.add(rid)
            if table == 'users':
                users.add(rid)
            else:
                integer(row['user_id'])
            for field in ('created_at', 'created_time', 'created_at_ms', 'reviewed_at', 'revoked_at_ms'):
                if field in row and row[field] is not None:
                    integer(row[field], low=-(2**63), high=2**63-1)
            if table == 'tokens':
                if row['oauth_managed'] is not None and type(row['oauth_managed']) is not bool:
                    raise ValueError('invalid token classification')
                if row['deleted_at'] is not None and (not isinstance(row['deleted_at'], str) or len(row['deleted_at']) > 100):
                    raise ValueError('invalid source deletion marker')
            if table == 'developer_access_requests' and row['status'] is not None and not isinstance(row['status'], str):
                raise ValueError('invalid source application state')
    if not users or mapping.get('contract') != 'legacy.connection-mapping.v1':
        raise ValueError('source/mapping cohort required')
    mapped, targets = set(), set()
    for row in mapping.get('users', []):
        if set(row) != {'source_user_id', 'user_id', 'account_id', 'opening_entity_key'}:
            raise ValueError('invalid user mapping fields')
        uid, target_uid = integer(row['source_user_id']), integer(row['user_id'])
        integer(row['account_id'])
        if uid in mapped or target_uid in targets or row['opening_entity_key'] not in (f'user:{uid}', f'legacy.users/{uid}'):
            raise ValueError('duplicate or wrong opening mapping')
        mapped.add(uid)
        targets.add(target_uid)
    for row in mapping.get('omitted_users', []):
        if set(row) != {'source_user_id', 'reason'}:
            raise ValueError('invalid omitted user declaration')
        uid = integer(row['source_user_id'])
        if uid in mapped or not isinstance(row['reason'], str) or not 1 <= len(row['reason'].strip()) <= 500:
            raise ValueError('omitted source owner needs explicit reason')
        mapped.add(uid)
    if mapped != users or not isinstance(mapping.get('users'), list) or not isinstance(mapping.get('omitted_users'), list):
        raise ValueError('mapping must cover all retained source users')
    count = target.get('component_count')
    if (target.get('contract') != 'legacy.connection-target.v1' or type(count) is not int
            or count not in TARGET_CATALOGS or target.get('catalog_sha256') != TARGET_CATALOGS[count]):
        raise ValueError('reviewed target component/catalog contract required')
    nonce = hexhash(target.get('target_nonce'))
    if nonce == '0'*64:
        raise ValueError('unique target nonce required')
    integer(target.get('target_database_oid'), high=2**32-1)
    integer(target.get('native_credential_floor'), low=0)
    hexhash(target.get('catalog_sha256'))
    hexhash(opening_mapping_sha256)
    for migration in (opening_migration_id, identity_migration_id):
        if not isinstance(migration, str) or re.fullmatch('[A-Za-z0-9._:/-]{1,128}', migration) is None:
            raise ValueError('invalid original migration identity')
    if not isinstance(target.get('target_database'), str) or not target['target_database']:
        raise ValueError('target database required')
    values = [archive, sha(raw), raw.decode('utf-8'), json.dumps(metadata, ensure_ascii=False),
              mapping_raw.decode('utf-8'), sha(mapping_raw), opening_migration_id,
              opening_mapping_sha256, identity_migration_id, snapshot_at,
              target['target_database'], target['target_database_oid'], nonce,
              target['catalog_sha256'], target['native_credential_floor']]
    return ("BEGIN ISOLATION LEVEL SERIALIZABLE;\nSET LOCAL synchronous_commit=on;\nSET LOCAL lock_timeout='5s';\n"
            "LOCK TABLE core_history.connection_snapshot IN EXCLUSIVE MODE;\n"
            "LOCK TABLE core_identity.users,core_identity.accounts,core_identity.credentials,"
            "core_billing.opening_imports,core_billing.opening_import_runs,core_migration.import_runs IN SHARE MODE;\n"
            "INSERT INTO core_history.connection_snapshot(source_archive_sha256,export_file_sha256,document_utf8,metadata,"
            "mapping_utf8,mapping_sha256,opening_migration_id,opening_mapping_sha256,identity_migration_id,"
            "snapshot_observed_at,target_database,target_database_oid,target_nonce,catalog_sha256,native_credential_floor) VALUES("
            + ','.join(literal(v) for v in values) + ");\nCOMMIT;\n")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'metadata', 'mapping', 'target', 'output'):
        p.add_argument('--'+name, type=Path, required=True)
    for name in ('opening-migration-id', 'opening-mapping-sha256', 'identity-migration-id'):
        p.add_argument('--'+name, required=True)
    a = p.parse_args()
    sql = plan(a.source.read_bytes(), decode(a.metadata.read_bytes()), a.mapping.read_bytes(),
               decode(a.target.read_bytes()), opening_migration_id=a.opening_migration_id,
               opening_mapping_sha256=a.opening_mapping_sha256, identity_migration_id=a.identity_migration_id)
    with os.fdopen(os.open(a.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as f:
        f.write(sql)
    print(json.dumps({'plan_sha256': sha(sql.encode()), 'applied': False}))


if __name__ == '__main__':
    main()
