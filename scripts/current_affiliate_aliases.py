#!/usr/bin/env python3
"""Exact legacy invitation aliases: source preview and offline owner SQL.

No database connection, source fence, identity receipt, approval or code is
manufactured here. The final Core85 predecessor and Core86 SQL catalogue are
pinned below; official binary qualification remains separate. Existing financial helpers
are used only for their frozen pure/observational functions, never as a target
version override. No sequence is allocated and no financial row is written.
"""
from collections import Counter
from copy import deepcopy
import argparse
import hashlib
import importlib
import json
import os
from pathlib import Path
import re
import sys

SCOPE = 'current-legacy-affiliate-aliases-v1'
FORMAT = 'lmm.current-affiliate-alias-import.v1'
PREPARATION = 'lmm.current-affiliate-alias-owner-preparation.v1'
COMPONENT_COUNT = 86
COMPONENT_NAME = 'legacy-affiliate-aliases'
TARGET_BINDING_SHA256 = 'ca8dbe193b42b74d8bb01a8424565e0aa40326d504a2a4eb403313c72b791584'
TABLES = ('core_identity.legacy_affiliate_aliases', 'core_meta.legacy_affiliate_alias_imports')
HEX = re.compile(r'[a-f0-9]{64}\Z')
CODE = re.compile(r'[A-Za-z0-9]{4}\Z')
IDENTIFIER = re.compile(r'[A-Za-z0-9._:/-]{1,128}\Z')
FROZEN = {
 'current_key_lineage.py':'0835d57cb70a58ed8bdf368d5c25d588aca0139dfdb44389e2550981953c9da7',
 'migrate_current_identity.py':'c3430d1d03f201dcf254bb31678e7af281d94136cf64b6e661881907f6b3b376',
 'final81_target_guard.py':'5b928e9910fe4648b4e4d846625bb141eb880b5a5f864d0ef1e11ac662a6a193',
 'migrate_current_runtime_keys.py':'6a612f6c07737bc478bff430804f25ffe85072aba6ecfd3a398e4afe16cf9708',
 'legacy_routing_migration.py':'8744d679aa352c852b262ad30b955f4187a8a1b3f1a6b492bc38e5b9cee1ff6b',
 'migrate-legacy-financial.py':'fa21a4f97bfcd038ee2fdcc588481e1a8704964b2c59102147c4384dbcfc1742',
 'financial_import.py':'230e41a34cca558e16e73282c94d898e892822d8d1991902c9a4e3cb80e32d23',
 'migrate-legacy-snapshot.py':'abf4c0e95a0e0fc406c84a082e5c965f805a99a16deef517e8be38577f054ec2',
 'legacy_credential_import.py':'c274063a5d40d95821605a849a745fade0aafeb5e3d075105bb58a54c9f8f69a',
 'legacy_api_key_migration.py':'7ca343cbfcf6f58bb2162c4117fc51b3cf31a9be7484e183ecfa9fb0311e6707',
 'legacy_key_quota_opening.py':'61519913184922d82361739f6d0ec004d1ea4189564fd1464783f8de110e9aa3',
}


class Error(ValueError):
    pass


def need(ok, message):
    if not ok:
        raise Error(message)


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'), allow_nan=False)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def digest(value):
    """Plain UTF8 canonical JSON. Source row hashes use the frozen typed digest."""
    return sha(canonical(value).encode())


def unique_object(items):
    value = {}
    for key, item in items:
        need(key not in value, 'alias_json_duplicate_key')
        value[key] = item
    return value


def parse(raw):
    return json.loads(raw, object_pairs_hook=unique_object)


def integer(value):
    need(type(value) is int and 0 < value <= 9223372036854775807, 'alias_positive_id_required')
    return value


def file_ref(path):
    path = Path(path).resolve()
    return {'path':str(path), 'sha256':sha(path.read_bytes()), 'bytes':path.stat().st_size}


def read_ref(ref, *, private=True):
    need(type(ref) is dict and set(ref) == {'path','sha256','bytes'}, 'alias_file_ref_invalid')
    path = Path(ref['path'])
    need(path.is_absolute() and path.is_file() and not path.is_symlink(), 'alias_file_missing')
    if private:
        need(path.stat().st_mode & 0o077 == 0, 'alias_private_input_permissions')
    raw = path.read_bytes()
    need(len(raw) == ref['bytes'] and sha(raw) == ref['sha256'], 'alias_file_bytes_changed')
    return raw


def frozen_helpers():
    L = importlib.import_module('current_key_lineage')
    T = importlib.import_module('final81_target_guard')
    R = importlib.import_module('legacy_routing_migration')
    for module in (L,L.ID,T,L.U,L.F,L.U.I,L.C,L.P,L.U.Q,R,R.F):
        need(sha(Path(module.__file__).read_bytes()) == FROZEN[Path(module.__file__).name],
             'alias_frozen_helper_changed')
    L.frozen_identity_module()
    return L,T,R


def schema_path():
    here = Path(__file__).resolve()
    repository = here.parent.parent / 'services/core/schema/legacy_affiliate_aliases.sql'
    if repository.is_file():
        return repository
    adjacent = here.with_name('legacy_affiliate_aliases.sql')
    need(adjacent.is_file(), 'alias_schema_file_required')
    return adjacent


def project_users(users, refs, *, synthetic=False):
    """Complete rows, including NULL codes and disabled/deleted owners."""
    _,_,R = frozen_helpers()
    need(type(users) is list and users, 'alias_complete_users_required')
    by_id = {}
    codes = set()
    rows = []
    for user in users:
        need(type(user) is dict and all(k in user for k in
             ('id','aff_code','role','status','deleted_at')), 'alias_user_columns_missing')
        uid = integer(user['id'])
        need(uid not in by_id, 'alias_duplicate_source_user')
        by_id[uid] = user
        code = user['aff_code']
        need(code is None or type(code) is str and CODE.fullmatch(code), 'alias_source_code_shape')
        if code is not None:
            need(code not in codes, 'alias_exact_code_collision')
            codes.add(code)
        rows.append({'source_user_id':uid, 'code':code, 'source_row_sha256':R.F.digest(user),
                     'source_role':user['role'], 'source_status':user['status'],
                     'source_deleted_at':user['deleted_at']})
    rows.sort(key=lambda row:row['source_user_id'])
    required = ('source_archive_sha256','source_export_sha256','source_export_proof_sha256')
    need(all(type(refs.get(key)) is str and HEX.fullmatch(refs[key]) for key in required),
         'alias_source_archive_binding_missing')
    return {'source_kind':'SYNTHETIC_ALIAS_FIXTURE' if synthetic else 'VERIFIED_FULL_CANONICAL_SOURCE',
            'source_archive_sha256':refs['source_archive_sha256'],
            'source_export_sha256':refs['source_export_sha256'],
            'source_export_proof_sha256':refs['source_export_proof_sha256'],
            'source_users_sha256':R.F.digest(users), 'source_codes_sha256':R.F.digest(rows),
            'source_digest_format':R.F.DIGEST_FORMAT,
            'user_count':len(users), 'alias_count':len(codes), 'rows':rows}


def verified_source(evidence):
    """Formal full export verification, retaining only users in memory."""
    _,_,R = frozen_helpers()
    need(type(evidence) is dict and set(evidence) == {'receipt','proof'}, 'alias_source_evidence_invalid')
    receipt = parse(read_ref(evidence['receipt']))
    read_ref(evidence['proof'])
    hashes = {}
    tables, proof, baseline = R.load_routing_export(
        receipt, Path(evidence['proof']['path']), hashes, retained_tables=('users',))
    users = tables['users']
    need(len(users) == baseline['users'] == proof['selected_table_counts']['users'],
         'alias_complete_users_baseline_mismatch')
    projection = project_users(users, {
        'source_archive_sha256':proof['source_archive_sha256'],
        'source_export_sha256':proof['source_export_sha256'],
        'source_export_proof_sha256':evidence['proof']['sha256']})
    return projection, users


def source_summary(projection):
    rows = projection['rows']
    folded = Counter(row['code'].lower() for row in rows if row['code'] is not None)
    return {'scope':SCOPE, 'status':'SOURCE_PREVIEW_ONLY', 'user_count':len(rows),
            'aliases':projection['alias_count'], 'null_codes':sum(row['code'] is None for row in rows),
            'casefold_collision_groups':sum(n > 1 for n in folded.values()),
            'owners_by_role_status_deleted':dict(sorted(Counter(
                str(row['source_role'])+'/'+str(row['source_status'])+'/'+str(row['source_deleted_at'] is not None)
                for row in rows if row['code'] is not None).items())),
            'source_users_sha256':projection['source_users_sha256'],
            'source_codes_sha256':projection['source_codes_sha256'],
            'target_binding':'PENDING_ACTUAL_COMMITTED_C343_IDENTITY',
            'target_contract_sha256':TARGET_BINDING_SHA256, 'source_fence_observed':False,
            'production_authorization':False, 'target_writes':0, 'financial_writes':0}


def contract_from_manifest(raw, binding, *, synthetic=False):
    """A SQL-capture binding and official binary evidence remain separate."""
    if not synthetic:
        need(TARGET_BINDING_SHA256 is not None, 'alias_final86_target_UNBOUND')
        need(digest(binding) == TARGET_BINDING_SHA256, 'alias_reviewed_target_binding_changed')
    manifest = parse(raw)
    need(manifest.get('scope') == 'compiled-core-schema-contract' and manifest.get('format_version') == 1,
         'alias_manifest_format_invalid')
    components = manifest['components']
    need(type(components) is list and len(components) == COMPONENT_COUNT, 'alias_component_count_not_86')
    names = {r['name']:r['raw_schema_sha256'] for r in components}
    need(len(names) == COMPONENT_COUNT and all(HEX.fullmatch(value) for value in names.values())
         and components[-1]['name'] == COMPONENT_NAME
         and names[COMPONENT_NAME] == sha(schema_path().read_bytes()), 'alias_component_assembly_changed')
    catalog = manifest['catalog']
    need(catalog['query_sha256'] == sha(catalog['query_utf8'].encode())
         and catalog['query_hash_domain'] == 'sha256-raw-utf8-v1'
         and catalog['fingerprint_hash_domain'] == 'sha256-u64be-byte-length-prefixed-utf8-v1'
         and catalog['definition_order'] == 'postgresql-c-collation', 'alias_catalog_algorithm_changed')
    need(binding['component_count'] == COMPONENT_COUNT and binding['manifest_sha256'] == sha(raw)
         and HEX.fullmatch(binding['catalog_sha256']), 'alias_target_binding_invalid')
    return {'component_count':COMPONENT_COUNT, 'manifest_sha256':sha(raw),
            'catalog_fingerprint_sha256':binding['catalog_sha256'],
            'catalog_query':catalog['query_utf8'], 'component_fingerprints':names,
            'identity_version':manifest['identity']['version'],
            'identity_fingerprint_sha256':manifest['identity']['fingerprint_sha256'],
            'binding':deepcopy(binding), 'synthetic_only':synthetic}


def alias_fence_sql(namespace):
    L,_,_ = frozen_helpers()
    L.U.namespace_key(namespace)
    relation = L.F.relation(namespace['schema']+'.users')
    return """BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC'; SET LOCAL row_security=off; SET LOCAL statement_timeout='60s';
SELECT jsonb_build_object('scope','current-source-affiliate-alias-fence-v1',
 'source_namespace',jsonb_build_object('system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
  'database',current_database(),'schema',""" + L.U.literal(namespace['schema']) + """),
 'observed_at',to_char(clock_timestamp(),'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
 'local_snapshot',pg_current_snapshot()::text,'transaction_read_only',current_setting('transaction_read_only'),
 'database_allows_connections',(SELECT datallowconn FROM pg_database WHERE datname=current_database()),
 'other_database_backends',(SELECT count(*) FROM pg_stat_activity
  WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid()
    AND backend_type IS DISTINCT FROM 'autovacuum worker'),
 'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()),
 'rows',(SELECT coalesce(jsonb_agg(jsonb_build_object('source_user_id',id,'code',aff_code) ORDER BY id),'[]'::jsonb)
  FROM """ + relation + """));
COMMIT;
"""


def validate_source_fence(projection, identity_run, evidence):
    L,_,_ = frozen_helpers()
    need(set(evidence) == {'identity_fence','alias_fence'}, 'alias_final_source_fence_required')
    identity_fence = parse(read_ref(evidence['identity_fence']))
    identity_material = identity_run['manifest']['material']
    L.ID.validate_final_fence(identity_material, identity_fence)
    alias_fence = parse(read_ref(evidence['alias_fence']))
    need(set(alias_fence) == {'scope','verification_actual_exit','observation','observation_sql',
                              'verification_stderr','identity_fence_sha256'}
         and alias_fence['scope'] == 'current-source-affiliate-alias-fence-v1'
         and type(alias_fence['verification_actual_exit']) is int and alias_fence['verification_actual_exit'] == 0
         and alias_fence['identity_fence_sha256'] == evidence['identity_fence']['sha256'],
         'alias_source_fence_binding_invalid')
    need(read_ref(alias_fence['observation_sql']) == alias_fence_sql(identity_material['source_namespace']).encode(),
         'alias_source_fence_sql_changed')
    read_ref(alias_fence['verification_stderr'])
    observation = parse(read_ref(alias_fence['observation']))
    identity_observation = parse(L.ID.private_bytes(identity_fence['observation']['path']))
    need(observation['scope'] == alias_fence['scope']
         and observation['source_namespace'] == identity_material['source_namespace']
         and observation['transaction_read_only'] == 'on' and observation['database_allows_connections'] is False
         and type(observation['other_database_backends']) is int and observation['other_database_backends'] == 0
         and type(observation['prepared_transactions']) is int and observation['prepared_transactions'] == 0
         and type(observation['local_snapshot']) is str and observation['local_snapshot']
         and L.ID.utc_time(observation['observed_at']) >= L.ID.utc_time(identity_observation['observed_at']),
         'alias_real_source_barrier_missing')
    expected = [{'source_user_id':row['source_user_id'],'code':row['code']} for row in projection['rows']]
    need(observation['rows'] == expected, 'alias_source_codes_changed_after_snapshot')
    return digest(evidence)


def actual_mapping(identity_run, authority, users, projection):
    L,_,_ = frozen_helpers()
    need(type(identity_run) is dict and type(identity_run.get('manifest')) is dict,
         'alias_actual_committed_identity_required')
    im = identity_run['manifest']['material']
    mapping = [{key:row[key] for key in L.MAP_FIELDS} for row in im['plan']['mapping']]
    target = {'source_target_id_map':mapping,
              'users':[{'user_id':row['target_user_id'], 'personal_account_id':row['target_personal_account_id'],
                        'auth_version':row['auth_version'], 'active':row['active'] and row['account_active'],
                        'developer_access_granted':row['developer_granted'] is True} for row in authority]}
    source = {'source_archive_sha256':projection['source_archive_sha256'],
              'source_export_sha256':projection['source_export_sha256'], 'users':users}
    L.validate(users,source,target,im['source_namespace'],identity_run,authority)
    need(im['source_canonical_export_proof_sha256'] == projection['source_export_proof_sha256'],
         'alias_identity_export_proof_changed')
    need(mapping == sorted(mapping,key=lambda row:row['source_user_id']), 'alias_identity_map_order_invalid')
    return mapping


def validate_snapshot(snapshot, contract, actor, *, synthetic):
    L,T,_ = frozen_helpers()
    instance = snapshot['instance']
    L.C.alias_target(instance)
    need(contract['component_count'] == COMPONENT_COUNT
         and len(contract['component_fingerprints']) == COMPONENT_COUNT
         and contract['component_fingerprints'][COMPONENT_NAME] == sha(schema_path().read_bytes())
         and snapshot['component_fingerprints'] == contract['component_fingerprints']
         and snapshot['runtime_catalog'] == snapshot['catalog_fingerprint_sha256']
             == contract['catalog_fingerprint_sha256'], 'alias_exact86_catalog_required')
    need(instance['database_owner'] == instance['role'] != instance['session_user']
         and instance['effective_role_can_login'] is False and instance['tcp_connection'] is False
         and HEX.fullmatch(instance['server_instance_nonce']), 'alias_offline_owner_instance_required')
    boundary = snapshot['role_boundary']
    need(boundary['other_connections'] == boundary['prepared_transactions'] == 0
         and boundary['unsafe_runtime'] == [] and boundary['connectors'] == [instance['session_user']],
         'alias_runtime_not_fenced')
    roles = {row['rolname']:row for row in boundary['roles']}
    need(len(roles) == len(boundary['roles']) and instance['role'] in roles and instance['session_user'] in roles,
         'alias_owner_role_coverage_invalid')
    for name, role in roles.items():
        need(not any(role[key] for key in ('rolsuper','rolcreaterole','rolcreatedb','rolreplication','rolbypassrls'))
             and role['rolcanlogin'] == (name == instance['session_user']), 'alias_offline_role_invalid')
    need(roles[instance['session_user']]['rolinherit'] is False
         and boundary['database_nonce'] == 'lmm.migration_instance_nonce='+instance['server_instance_nonce']
         and any(row['role'] == instance['role'] and row['member'] == instance['session_user']
                 and row['set'] is True and row['inherit'] is False and row['admin'] is False
                 for row in boundary['memberships'])
         and HEX.fullmatch(snapshot['ownership_sha256']), 'alias_installer_boundary_invalid')
    actual = snapshot['migration_actor']
    need(actual['user_id'] == integer(actor) and actual['active'] is True
         and actual['account_active'] is True and actual['platform_role'] == 'superadmin',
         'alias_enabled_actor_required')
    T.validate_inventory(snapshot['state']['inventory'])
    need(set(snapshot['state']['tables']) == set(snapshot['state']['inventory']['tables'])
         and set(snapshot['state']['sequences']) == set(snapshot['state']['inventory']['sequences'])
         and set(TABLES) <= set(snapshot['state']['tables']), 'alias_inventory_incomplete')
    need(snapshot['alias_count'] == snapshot['import_count'] == 0, 'alias_existing_import_requires_readonly_resolution')
    if synthetic:
        need(contract['synthetic_only'] is True
             and re.fullmatch(r'lmm_runtime_keys_test_[a-z0-9_]{1,30}', instance['database'])
             and snapshot['database_marker'] == 'lmm-runtime-keys-synthetic:'+snapshot['fixture_nonce'],
             'alias_synthetic_database_marker_required')
    else:
        need(contract['synthetic_only'] is False and TARGET_BINDING_SHA256 is not None
             and digest(contract['binding']) == TARGET_BINDING_SHA256, 'alias_final86_target_UNBOUND')


def prepare_owner(projection, users, snapshot, contract, *, migration_id, actor,
                  source_evidence=None, fence_evidence=None, synthetic=False):
    L,_,_ = frozen_helpers()
    need(type(migration_id) is str and IDENTIFIER.fullmatch(migration_id), 'alias_migration_id_invalid')
    if synthetic:
        need(projection['source_kind'] == 'SYNTHETIC_ALIAS_FIXTURE'
             and snapshot['identity_run']['manifest']['material']['source_kind'] == 'synthetic-test-facts',
             'alias_actual_synthetic_identity_required')
        need(projection == project_users(users,projection,synthetic=True), 'alias_synthetic_source_changed')
        source_fence_sha = None
    else:
        need(projection['source_kind'] == 'VERIFIED_FULL_CANONICAL_SOURCE', 'alias_production_source_required')
        verified, verified_users = verified_source(source_evidence)
        need(verified == projection and verified_users == users, 'alias_canonical_source_changed')
        source_fence_sha = validate_source_fence(projection,snapshot['identity_run'],fence_evidence)
    validate_snapshot(snapshot,contract,actor,synthetic=synthetic)
    mapping = actual_mapping(snapshot['identity_run'],snapshot['identity_authority'],users,projection)
    ir = snapshot['identity_run'];im=ir['manifest']['material'];instance=snapshot['instance']
    target = {'component_count':COMPONENT_COUNT,'manifest_sha256':contract['manifest_sha256'],
              'catalog_sha256':contract['catalog_fingerprint_sha256'],'database':instance['database'],
              'database_oid':instance['database_oid'],'system_identifier':instance['system_identifier'],
              'server_instance_nonce':instance['server_instance_nonce']}
    material = {'format':FORMAT,'scope':SCOPE,'migration_id':migration_id,'source_kind':projection['source_kind'],
                'source_namespace':im['source_namespace'],'source_namespace_sha256':im['source_namespace_sha256'],
                'source':deepcopy(projection),'mapping':mapping,'migration_actor_user_id':actor,
                'source_fence_sha256':source_fence_sha,'target':target,
                'identity':{'migration_id':ir['migration_id'],'mapping_sha256':ir['mapping_sha256'],
                            'manifest_sha256':snapshot['identity_manifest_sha256'],
                            'actual_authority':snapshot['identity_authority']}}
    if synthetic:
        material['fixture_nonce'] = snapshot['fixture_nonce']
    return {'format':PREPARATION,'material':material,'contract':deepcopy(contract),
            'snapshot':deepcopy(snapshot),'source_evidence':source_evidence,
            'fence_evidence':fence_evidence,'synthetic_users':deepcopy(users) if synthetic else None}


def observation_sql(mapping, identity_id, *, inventory, contract, runtime_roles, actor):
    L,T,_ = frozen_helpers()
    need(IDENTIFIER.fullmatch(identity_id), 'alias_identity_id_invalid')
    F,I=L.F,L.U.I
    lit=L.U.literal
    return ("SELECT jsonb_build_object('instance',("+F.instance_sql()+"),'state',("+T.state_sql(inventory)+"),"
      "'ownership_sha256',("+F.ownership_sql()+"),"
      "'component_fingerprints',(SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract),"
      "'runtime_catalog',(SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton),"
      "'catalog_fingerprint_sha256',("+I.catalog_hash_sql(contract)+"),"
      "'role_boundary',("+T.role_boundary_sql(runtime_roles)+"),'migration_actor',("+T.actor_sql(actor)+"),"
      "'identity_run',(SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE r.migration_id="+lit(identity_id)+"),"
      "'identity_manifest_sha256',(SELECT encode(sha256(convert_to(r.manifest::text,'UTF8')),'hex') "
      "FROM core_migration.import_runs r WHERE r.migration_id="+lit(identity_id)+"),"
      "'identity_authority',("+L.identity_authority_sql(mapping)+"),"
      "'alias_count',(SELECT count(*) FROM core_identity.legacy_affiliate_aliases),"
      "'import_count',(SELECT count(*) FROM core_meta.legacy_affiliate_alias_imports),"
      "'database_marker',shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database'))")


def validate_preparation(prepared):
    need(prepared['format'] == PREPARATION, 'alias_preparation_format')
    m = prepared['material']
    synthetic = m['source_kind'] == 'SYNTHETIC_ALIAS_FIXTURE'
    if synthetic:
        users = prepared['synthetic_users']
        projection = project_users(users,m['source'],synthetic=True)
    else:
        projection,users = verified_source(prepared['source_evidence'])
    rebuilt = prepare_owner(projection,users,prepared['snapshot'],prepared['contract'],
                migration_id=m['migration_id'],actor=m['migration_actor_user_id'],
                source_evidence=prepared['source_evidence'],fence_evidence=prepared['fence_evidence'],
                synthetic=synthetic)
    need(rebuilt == prepared, 'alias_saved_preparation_changed')


def reviewed(prepared, review):
    m = prepared['material']
    if m['source_kind'] == 'SYNTHETIC_ALIAS_FIXTURE':
        expected = {'scope':'synthetic-alias-test-only','material_sha256':digest(m),'production_authorization':False}
    else:
        expected = {'scope':'reviewed-current-legacy-affiliate-aliases-v1','material_sha256':digest(m),
                    'migration_actor_user_id':m['migration_actor_user_id'],
                    'source_fence_sha256':m['source_fence_sha256'],'target':m['target']}
    need(review == expected, 'alias_exact_review_document_required')
    return canonical(review)


def readback_expression(prepared, review):
    L,_,_ = frozen_helpers()
    m=prepared['material'];lit=L.U.literal
    expected=[{'code':row['code'],'user_id':mp['target_user_id'],'source_user_id':row['source_user_id'],
               'source_row_sha256':row['source_row_sha256'],'import_id':m['migration_id']}
              for row,mp in zip(m['source']['rows'],m['mapping'],strict=True) if row['code'] is not None]
    return ("(SELECT count(*)=1 AND bool_and(r.import_id="+lit(m['migration_id'])+
      " AND r.material_utf8="+lit(canonical(m))+" AND r.material_sha256="+lit(digest(m))+
      " AND r.review_utf8="+lit(canonical(review))+" AND r.review_sha256="+lit(digest(review))+")"
      " FROM core_meta.legacy_affiliate_alias_imports r)"
      " AND (SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY a.source_user_id),'[]'::jsonb)"
      " FROM core_identity.legacy_affiliate_aliases a)="+lit(expected)+"::jsonb")


def transaction_sql(prepared, review):
    validate_preparation(prepared)
    review_raw=reviewed(prepared,review)
    L,T,_=frozen_helpers();F=L.F;lit=L.U.literal;m=prepared['material'];s=prepared['snapshot'];c=prepared['contract']
    inventory=s['state']['inventory'];state="("+T.state_sql(inventory)+")";before=lit(s['state'])+"::jsonb"
    roles=sorted(row['rolname'] for row in s['role_boundary']['roles']
                 if row['rolname'] not in (s['instance']['role'],s['instance']['session_user']))
    guard=("IF ("+F.instance_sql()+") IS DISTINCT FROM "+lit(s['instance'])+"::jsonb"
      " OR ("+L.U.I.catalog_hash_sql(c)+")<>"+lit(c['catalog_fingerprint_sha256'])+
      " OR ("+T.role_boundary_sql(roles)+") IS DISTINCT FROM "+lit(s['role_boundary'])+"::jsonb"
      " OR ("+F.ownership_sql()+")<>"+lit(s['ownership_sha256'])+
      " THEN RAISE EXCEPTION 'affiliate_alias_target_or_owner_fence_changed'; END IF;")
    exact=readback_expression(prepared,review)
    excluded="ARRAY["+','.join(lit(t) for t in TABLES)+"]"
    body="""DECLARE after_state jsonb; BEGIN
 IF NOT pg_try_advisory_xact_lock("""+str(L.U.FENCE)+""") THEN RAISE EXCEPTION 'affiliate_alias_import_busy'; END IF;
 """+guard+"""
 LOCK TABLE """+','.join(F.relation(t) for t in inventory['tables'])+""" IN ACCESS EXCLUSIVE MODE;
 """+guard+"""
 IF EXISTS(SELECT FROM core_meta.legacy_affiliate_alias_imports) THEN
   IF coalesce(("""+exact+"""),false) IS NOT TRUE THEN RAISE EXCEPTION 'affiliate_alias_replay_conflict'; END IF;
 ELSE
   IF """+state+""" IS DISTINCT FROM """+before+""" THEN RAISE EXCEPTION 'affiliate_alias_before_state_changed'; END IF;
   """+L.ID.readback_sql(s['identity_run']['manifest']['material'])+"""
   INSERT INTO core_meta.legacy_affiliate_alias_imports(import_id,material_utf8,material_sha256,review_utf8,review_sha256)
   VALUES("""+','.join(lit(v) for v in (m['migration_id'],canonical(m),digest(m),review_raw,sha(review_raw.encode())))+""");
   INSERT INTO core_identity.legacy_affiliate_aliases(code,user_id,source_user_id,source_row_sha256,import_id)
   SELECT s->>'code',(mp->>'target_user_id')::bigint,(s->>'source_user_id')::bigint,
     s->>'source_row_sha256',"""+lit(m['migration_id'])+"""
   FROM jsonb_array_elements("""+lit(m['source']['rows'])+"""::jsonb) s
   JOIN jsonb_array_elements("""+lit(m['mapping'])+"""::jsonb) mp ON mp->>'source_user_id'=s->>'source_user_id'
   WHERE s->>'code' IS NOT NULL;
   SET CONSTRAINTS ALL IMMEDIATE;
   IF coalesce(("""+exact+"""),false) IS NOT TRUE THEN RAISE EXCEPTION 'affiliate_alias_readback_mismatch'; END IF;
   SELECT """+state+""" INTO after_state;
   IF after_state->'sequences' IS DISTINCT FROM """+before+"""->'sequences'
    OR (after_state->'tables')-"""+excluded+""" IS DISTINCT FROM ("""+before+"""->'tables')-"""+excluded+"""
    OR after_state->'inventory' IS DISTINCT FROM """+before+"""->'inventory'
   THEN RAISE EXCEPTION 'affiliate_alias_existing_state_changed'; END IF;
 END IF;
 """+guard+"""
END"""
    tag='$alias_import_'+sha(body.encode())[:24]+'$'
    need(tag not in body,'alias_sql_delimiter_collision')
    return ("BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;"
      " SET LOCAL synchronous_commit=on; SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='180s';\n"
      "DO "+tag+body+tag+"; COMMIT;\n"
      "SELECT jsonb_build_object('status','COMMITTED','migration_id',"+lit(m['migration_id'])+
      ",'automatic_retry_allowed',false);\n")


def resolve_sql(prepared, review, application_name):
    validate_preparation(prepared);reviewed(prepared,review)
    L,_,_=frozen_helpers();lit=L.U.literal;m=prepared['material'];t=m['target']
    need(re.fullmatch(r'lmm-affiliate-alias-[a-f0-9]{32}',application_name), 'alias_attempt_id_invalid')
    check=readback_expression(prepared,review)
    return ("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC';"
      " DO $alias_resolve$ BEGIN IF current_database()<>"+lit(t['database'])+
      " OR (SELECT oid FROM pg_database WHERE datname=current_database())<>"+str(t['database_oid'])+
      " OR (SELECT system_identifier::text FROM pg_control_system())<>"+lit(t['system_identifier'])+
      " OR current_setting('lmm.migration_instance_nonce',true) IS DISTINCT FROM "+lit(t['server_instance_nonce'])+
      " THEN RAISE EXCEPTION 'affiliate_alias_resolution_target_changed'; END IF;"
      " IF EXISTS(SELECT FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())"
      " AND application_name="+lit(application_name)+" AND pid<>pg_backend_pid())"
      " THEN RAISE EXCEPTION 'affiliate_alias_original_writer_active'; END IF;"
      " IF EXISTS(SELECT FROM core_meta.legacy_affiliate_alias_imports) AND coalesce(("+check+"),false) IS NOT TRUE"
      " THEN RAISE EXCEPTION 'affiliate_alias_resolution_conflict'; END IF; END $alias_resolve$;"
      " SELECT jsonb_build_object('status',CASE WHEN coalesce(("+check+"),false) THEN 'COMMITTED' ELSE 'UNKNOWN' END,"
      "'automatic_retry_allowed',false); COMMIT;\n")


def write_private(path, value):
    path=Path(path);path.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
    with path.open('x') as out:
        out.write(json.dumps(value,ensure_ascii=False,sort_keys=True,indent=2)+'\n')
    path.chmod(0o600)
    return file_ref(path)


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='command',required=True)
    preview=sub.add_parser('source-preview')
    preview.add_argument('--support',type=Path,required=True,help='Directory containing the reviewed frozen helper files')
    preview.add_argument('--source-receipt',type=Path,required=True)
    preview.add_argument('--source-proof',type=Path,required=True)
    preview.add_argument('--output',type=Path,required=True)
    args=parser.parse_args(argv);os.umask(0o077)
    sys.path.insert(1,str(args.support.resolve()))
    evidence={'receipt':file_ref(args.source_receipt),'proof':file_ref(args.source_proof)}
    projection,_=verified_source(evidence)
    saved=write_private(args.output,{'scope':SCOPE,'projection':projection,'source_evidence':evidence,
                                   'production_authorization':False,'target_contract_sha256':TARGET_BINDING_SHA256})
    print(json.dumps({'result':saved,'summary':source_summary(projection)},sort_keys=True))
    return 0


if __name__=='__main__':
    raise SystemExit(main())
