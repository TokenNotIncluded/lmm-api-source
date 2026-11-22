#!/usr/bin/env python3
"""Current connection-history source projection and owner SQL preparation.

No credential, entitlement, balance, receipt or sequence is created by this
component. Only the two new history tables are inserted. The production
controller owns the reviewed exact candidate, source fence and offline roles.
This module never connects to a database or manufactures a committed receipt.
"""
from collections import Counter
from copy import deepcopy
import argparse
import hashlib
import importlib
import json
from pathlib import Path
import re
import sys
import time

SCOPE = 'current-connection-history-v1'
COMPONENT_COUNT = 84
TABLES = ('core_history.current_connection_snapshot', 'core_history.current_connection_credential_map')
SOURCE_TABLES = ('users', 'tokens', 'oauth_server_grants', 'developer_access_requests')
SOURCE_FIELDS = {
    'users': {'id', 'created_at'},
    'tokens': {'id', 'user_id', 'oauth_managed', 'created_time', 'deleted_at'},
    'oauth_server_grants': {'id', 'user_id', 'created_at_ms', 'revoked_at_ms'},
    'developer_access_requests': {'id', 'user_id', 'created_at', 'reviewed_at', 'status'},
}
COVERAGE = 'all_retained_rows_in_frozen_snapshot_including_soft_deleted_and_revoked'
FROZEN = {
    'current_runtime_keys81.py': '51313c45f0f6fc9f614b8f70deb6d3540d2d9625cca16f600029a7604e81a7e0',
    'current_key_lineage.py': '0835d57cb70a58ed8bdf368d5c25d588aca0139dfdb44389e2550981953c9da7',
    'migrate_current_identity.py': 'c3430d1d03f201dcf254bb31678e7af281d94136cf64b6e661881907f6b3b376',
}
FROZEN_KEY_TOOLS = {
    **FROZEN,
    'current_key_production.py':'b1e157a0db8b22ccc7a41263c54c0c6b6f0c4c2ba49fe825b91fc0e90a0ba8ba',
    'final81_target_guard.py':'5b928e9910fe4648b4e4d846625bb141eb880b5a5f864d0ef1e11ac662a6a193',
    'financial_import.py':'230e41a34cca558e16e73282c94d898e892822d8d1991902c9a4e3cb80e32d23',
    'legacy_admin_activation_lineage.py':'89a2c24465c6543699d8fe91dbba48cd4a02556e090e7cdc4626db3f0b2809dc',
    'legacy_api_key_migration.py':'7ca343cbfcf6f58bb2162c4117fc51b3cf31a9be7484e183ecfa9fb0311e6707',
    'legacy_credential_import.py':'c274063a5d40d95821605a849a745fade0aafeb5e3d075105bb58a54c9f8f69a',
    'legacy_financial_preflight.py':'4d9108a3138796ea51068983057c3cff1e58430205cf450369a532e233f1fcb0',
    'legacy_key_quota_opening.py':'61519913184922d82361739f6d0ec004d1ea4189564fd1464783f8de110e9aa3',
    'migrate-legacy-financial.py':'fa21a4f97bfcd038ee2fdcc588481e1a8704964b2c59102147c4384dbcfc1742',
    'migrate-legacy-snapshot.py':'abf4c0e95a0e0fc406c84a082e5c965f805a99a16deef517e8be38577f054ec2',
    'migrate_current_runtime_keys.py':'6a612f6c07737bc478bff430804f25ffe85072aba6ecfd3a398e4afe16cf9708',
    'migrate_current_wallet_reconciliation.py':'7b05d231bc08150196bd9fa577c3f53a532b78a5249c8143837498ef7e444e46',
    'services/core/src/identity.rs':'eb4d860a97d2c9d7feabc7c714d8d64d1657c1275af87a3458791c7f4fa0d80a',
    'services/core/src/identity/compat.rs':'2f3769ec89a658390990c2013ca3886f59352896e7715aeaf6d51e0dd96f9d69',
}
FROZEN_SOURCE_EXTRAS = {
    'legacy_routing_migration.py':'8744d679aa352c852b262ad30b955f4187a8a1b3f1a6b492bc38e5b9cee1ff6b',
    'legacy_cutover_import.py':'6357e13d0e711d21da2c2bf74ea725962aec297d36139089e62a8a1f77c54518',
    'legacy_cutover_money.py':'421c757893d33d48e620c05dc144adde90c683c7a199c4a4ef625e7196a8efe5',
}
HISTORY_COLLECTOR_SHA = '827f96f327f1de4dd443f0a3452609da0f4cafecd747abe26cbff135e8830da1'
HISTORY_EXPORT_SQL_SHA = 'da29734d182f1a7fab8d849581134d7937eaecc4782e57ace490c63130f4d7a0'
HEX = re.compile(r'[a-f0-9]{64}\Z')
ID = re.compile(r'[A-Za-z0-9._:/-]{1,128}\Z')
SNAPSHOT_COLUMNS = ('singleton','migration_id','source_namespace_sha256','source_archive_sha256',
    'export_file_sha256','document_utf8','metadata','mapping_utf8','mapping_sha256',
    'identity_migration_id','identity_mapping_sha256','key_migration_id','key_owner_migration_id',
    'key_mapping_sha256','key_material_sha256','snapshot_observed_at','target_database','target_database_oid',
    'target_system_identifier','target_nonce','catalog_sha256','native_credential_floor','migration_actor_user_id')
MAP_COLUMNS = ('snapshot_singleton','source_key_id','target_credential_id','user_id','account_id',
               'identity_migration_id','key_migration_id','key_owner_migration_id')


class Error(ValueError):
    pass


def need(ok, code):
    if not ok:
        raise Error(code)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':'), allow_nan=False)


def digest(value):
    return sha(canonical(value).encode())


def schema_path():
    """Use the repository's single schema file or the identical private bundle."""
    here=Path(__file__).resolve()
    repository=here.parent.parent/'services/core/schema/current_connection_history.sql'
    adjacent=here.with_name('current_connection_history.sql')
    if repository.is_file():
        if adjacent.is_file():
            need(repository.read_bytes()==adjacent.read_bytes(),'connection_schema_copies_disagree')
        return repository
    need(adjacent.is_file(),'connection_schema_source_missing')
    return adjacent


def integer(value, code, low=1, high=(1 << 63) - 1):
    need(type(value) is int and low <= value <= high, code)
    return value


def unique(rows, field, code):
    need(type(rows) is list and all(type(r) is dict and field in r for r in rows), code)
    indexed = {r[field]: r for r in rows}
    need(len(indexed) == len(rows), code)
    return indexed


def frozen_helpers():
    """The existing private package supplies the original dependency paths."""
    A = importlib.import_module('current_runtime_keys81')
    L = importlib.import_module('current_key_lineage')
    hashes={**FROZEN_KEY_TOOLS,**FROZEN_SOURCE_EXTRAS}
    # Check the objects Python actually loaded, not merely a same-named file
    # in a supplied directory. This also rejects sys.modules shadowing when
    # the owner controller imports more than one migration package.
    for module in (A,L,L.ID,A.U78,A.K78,A.K78.W,A.T,A.C,A.P,A.Q,A.F,A.I,
                   A.C.LINEAGE,A.F.PREFLIGHT,A.F.PLANNER,A.K78.R,A.K78.R.F,
                   A.K78.R.F.FINANCIAL_PREFLIGHT,A.K78.R.F.PAYMENT_LIFECYCLE,L.ID.O):
        need(sha(Path(module.__file__).read_bytes()) == hashes[Path(module.__file__).name],
             'connection_frozen_validator_changed')
    A.dependencies()
    need(A.tool_sources()==FROZEN_KEY_TOOLS,'connection_frozen_dependency_tree_changed')
    return A, L


def verified_source_from_evidence(evidence):
    """Full canonical verifier plus exact original four-table capture files."""
    A,_=frozen_helpers();K=A.K78
    tables,proof,parent,provenance,_=K.read_source(evidence)
    for name,value in (('parent',parent),('parent_provenance',provenance)):
        if name in evidence['source']:
            need(K.read_ref(evidence['source'][name])==value,'connection_parent_not_original_full_projection')
    history={k:K.read_ref(v,parse=k!='query') for k,v in evidence['history'].items()}
    raw,binding,acq,reader,cleanup,restore,baseline=(history[k] for k in
        ('document','binding','acquisition','reader','cleanup','restore','baseline'))
    h=evidence['history']
    need(acq['scope']=='ACTUAL_ISOLATED_RESTORED_CONNECTION_HISTORY_PREVIEW'
         and acq['collector_sha256']==HISTORY_COLLECTOR_SHA
         and acq['original_export_sql_sha256']==HISTORY_EXPORT_SQL_SHA
         and acq['actual_query_exit_code']==0 and acq['source_production_fenced'] is False
         and acq['financial_approval'] is False, 'connection_actual_capture_not_verified')
    for key,record in (('document','actual_export'),('query','actual_sql'),('reader','reader_proof'),
                       ('restore','original_restore_receipt'),('baseline','baseline')):
        # Byte-identical private copies may relocate an original acquisition
        # input under BACKUP_ROOT; paths never substitute for exact file bytes.
        need(h[key]['sha256']==acq[record]['sha256'],'connection_capture_file_binding_mismatch')
    need(binding['export_file_sha256']==h['document']['sha256']
         and binding['source_archive_sha256']==parent['source_archive_sha256']==proof['source_archive_sha256']
         and acq['archive']['sha256']==proof['source_archive_sha256']
         and restore['archive']['sha256']==proof['source_archive_sha256'], 'connection_capture_archive_mismatch')
    before,after=acq['physical_before'],acq['physical_after']
    for key in ('database','database_oid','nonce','system_identifier','data_directory','postmaster_started_at'):
        need(before[key]==after[key], 'connection_capture_physical_identity_changed')
    ri=restore['restored_instance']
    need(before['database']==ri['database']==reader['database']
         and before['nonce']==ri['nonce']==reader['nonce']
         and before['system_identifier']==ri['system_identifier']
         and before['system_identifier']!=restore['production_source_observation']['system_identifier']
         and before['data_directory']==ri['data_directory']
         and reader['database_oid']==before['database_oid']
         and reader['postmaster_started_at']==before['postmaster_started_at']
         and reader['role']==restore['connection']['role']
         and reader['schema']==binding['source_schema'], 'connection_capture_not_owned_restore')
    need(reader['read_only']=='on' and reader['isolation']=='repeatable read' and reader['row_security']=='off'
         and all(v is False for v in reader['role_flags'].values())
         and all(reader[k] in (0,False) for k in ('role_memberships','database_create','database_temp',
             'application_table_write_privileges','application_schema_create_privileges','application_security_definer_execute','rls_tables'))
         and all(before[k]==after[k]=='on' for k in ('read_only','default_read_only'))
         and before['listen_addresses']==after['listen_addresses']==''
         and before['autovacuum']==after['autovacuum']=='off', 'connection_capture_reader_not_readonly')
    baseline_rows=[r for r in baseline['tables'] if r['schema']==binding['source_schema'] and r['table'] in SOURCE_TABLES]
    need(len(baseline_rows)==len(SOURCE_TABLES)
         and len({r['table'] for r in baseline_rows})==len(SOURCE_TABLES)
         and {r['table']:r['rows'] for r in baseline_rows}==raw['counts']==acq['counts'],
         'connection_capture_baseline_changed')
    need(cleanup['status']=='EXPORTED_VERIFIED_SOURCE_ONLY'
         and cleanup['restart_exit_code']==cleanup['stop_exit_code']==0
         and cleanup['final_status_exit_code']==3 and cleanup['pid_absent'] is True and cleanup['socket_absent'] is True
         and cleanup['nonce']==before['nonce'] and cleanup['system_identifier']==before['system_identifier']
         and cleanup['archive_sha256_unchanged']==parent['source_archive_sha256']
         and cleanup['production_writes']==0 and cleanup['target_database_accessed'] is False,
         'connection_capture_cleanup_unproven')
    projection=project_source(raw,tables['users'],parent,provenance,binding,acq)
    refs=list(evidence['history'].values())+list(v for v in evidence['source'].values() if type(v) is dict and set(v)=={'path','sha256'})
    receipt=K.read_ref(evidence['source']['receipt'])
    paths=[Path(p) for p in receipt['private_metadata_files']]
    paths += [Path(receipt['archive']['path']),Path(evidence['source']['proof']['path']).with_name('source-financial-export.private.jsonl')]
    refs += [K.file_ref(p) for p in paths]
    projection['input_files']=refs
    projection['raw_history_file']=deepcopy(h['document'])
    projection['source_evidence']=deepcopy(evidence)
    verify_source_files(projection)
    return projection,parent,provenance,tables


def verify_source_files(projection):
    if projection['source_kind']=='SYNTHETIC_HISTORY_FIXTURE':
        return
    A,_=frozen_helpers()
    need(projection['source_kind']=='VERIFIED_FULL_ARCHIVE_HISTORY'
         and projection.get('input_files') and projection.get('raw_history_file'), 'connection_verified_source_files_required')
    for ref in projection['input_files']:
        path=A.I.private_backup_path(Path(ref['path']))
        need(A.I.sha256_file(path)==ref['sha256'],'connection_source_input_bytes_changed')
    bind_original_history(projection['document'],A.K78.read_ref(projection['raw_history_file']))


def bind_original_history(current_document, original_document):
    """All four arrays retain their actual nonfinancial capture, not only its SHA label."""
    validate_retained_history(original_document)
    selected=deepcopy(current_document);selected['contract']='legacy.connection-source.v1'
    selected['users']=[{k:r[k] for k in SOURCE_FIELDS['users']} for r in selected['users']]
    selected['tokens']=[{k:r[k] for k in SOURCE_FIELDS['tokens']} for r in selected['tokens']]
    need(selected==original_document,'connection_projected_history_not_original_capture')


def validate_retained_history(document):
    need(type(document) is dict and document.get('contract') == 'legacy.connection-source.v1'
         and document.get('coverage') == COVERAGE and type(document.get('source_schema')) is str
         and document['source_schema'], 'connection_history_source_contract_invalid')
    integer(document.get('observed_at'), 'connection_history_observation_invalid', high=253402300799)
    for table in SOURCE_TABLES:
        rows = document.get(table)
        by_id = unique(rows, 'id', 'connection_history_source_ids_not_unique')
        count=document.get('counts',{}).get(table)
        need(type(count) is int and count == len(rows), 'connection_history_source_count_mismatch')
        for row in by_id.values():
            need(set(row) == SOURCE_FIELDS[table], 'connection_history_source_columns_changed')
            if table == 'oauth_server_grants':
                need(type(row['id']) is str and 1 <= len(row['id']) <= 128, 'connection_grant_identity_invalid')
            else:
                integer(row['id'], 'connection_source_identity_invalid')
            if table != 'users':
                integer(row['user_id'], 'connection_source_owner_invalid')
            for key in ('created_at','created_time','created_at_ms','revoked_at_ms','reviewed_at'):
                if key in row and row[key] is not None:
                    integer(row[key], 'connection_source_time_invalid', low=-(1 << 63))
            if table == 'tokens':
                need(row['oauth_managed'] is None or type(row['oauth_managed']) is bool,
                     'connection_source_key_kind_invalid')
                need(row['deleted_at'] is None or type(row['deleted_at']) is str,
                     'connection_source_deletion_marker_invalid')
            if table == 'developer_access_requests':
                need(row['status'] is None or type(row['status']) is str, 'connection_source_application_state_invalid')
    return document


def project_source(history, full_users, parent, provenance, history_binding, acquisition, *, synthetic=False):
    """Join a complete history export to the already-verified full canonical rows.

    Production callers first use the unchanged K78.read_source full verifier
    and verify the acquisition's exact files. Synthetic fixture callers retain
    their separate explicit label. Neither result can authorize a write.
    """
    A, _ = frozen_helpers()
    F, P = A.F, A.P
    validate_retained_history(history)
    raw_users = unique(full_users, 'id', 'connection_canonical_user_ids_not_unique')
    raw_tokens = unique(parent['tokens'], 'id', 'connection_canonical_key_ids_not_unique')
    need(parent['source_projection_sha256'] == P.sha256(provenance)
         and provenance['source_row_counts']['users'] == len(full_users)
         and provenance['source_row_counts']['tokens'] == len(raw_tokens), 'connection_parent_source_binding_invalid')
    need({r['id'] for r in history['users']} == set(raw_users)
         and {r['id'] for r in history['tokens']} == set(raw_tokens), 'connection_history_not_full_canonical_cohort')
    need(history_binding['source_archive_sha256'] == parent['source_archive_sha256']
         and history_binding['source_schema'] == history['source_schema']
         and history_binding['snapshot_observed_at'] == parent['snapshot_at']
         and history_binding['coverage'] == COVERAGE,
         'connection_history_archive_binding_invalid')
    need(acquisition['counts'] == history['counts'] and acquisition['source_archive_sha256'] == parent['source_archive_sha256']
         and acquisition['production_writes'] == 0 and acquisition['target_database_accessed'] is False,
         'connection_history_acquisition_scope_invalid')
    result = deepcopy(history)
    result['contract'] = 'current.connection-source.v1'
    for row in result['users']:
        full = raw_users[row['id']]
        need(row == {k: full[k] for k in SOURCE_FIELDS['users']}, 'connection_registration_source_drift')
        row['source_canonical_row_sha256'] = F.PLANNER.digest(full)
    for row in result['tokens']:
        full = raw_tokens[row['id']]
        need(row == {k: full[k] for k in SOURCE_FIELDS['tokens']}, 'connection_token_source_drift')
        # The old parser also rejects an unclassified oauth_managed_ wire.
        need(not full['key'].startswith('oauth_managed_') or full['oauth_managed'] is True,
             'connection_unknown_internal_key_prefix')
        row['creation_source'] = full['creation_source']
        row['source_row_sha256'] = F.digest(full)
    ordinary = [deepcopy(r) for r in parent['tokens'] if not
                (r['oauth_managed'] or r['creation_source'] == 'assistant_runtime' or r['key'].startswith('oauth_managed_'))]
    return {'scope':SCOPE,'source_kind':'SYNTHETIC_HISTORY_FIXTURE' if synthetic else 'VERIFIED_FULL_ARCHIVE_HISTORY',
        'source_archive_sha256':parent['source_archive_sha256'],'source_namespace':None,
        'document':result,'history_binding':deepcopy(history_binding),'acquisition':deepcopy(acquisition),
        'source_canonical_export_sha256':parent['source_export_sha256'],
        'source_canonical_export_proof_sha256':provenance['source_export_proof_sha256'],
        'source_canonical_receipt_sha256':provenance['source_receipt_sha256'],
        'full_parent_source_sha256':P.sha256(parent),'ordinary_source_token_ids':[r['id'] for r in ordinary],
        'full_parent_tokens_sha256':F.digest(parent['tokens']), 'projection_authorizes_write':False}


def map_committed_rows(projection, identity_run, key_run, key_owner_run, native_credentials, native_accounts):
    """Pure, exact cross-joins used after the frozen whole-material validators."""
    need(identity_run.get('manifest',{}).get('scope') == 'current-identity-append-completion-v1'
         and key_run.get('manifest',{}).get('scope') == 'current-core81-runtime-key-append-v1'
         and key_owner_run.get('manifest',{}).get('scope') == 'current-core81-rights-append-v1',
         'connection_committed_receipts_required')
    im, km, om = (r['manifest'].get('material', {}) for r in (identity_run,key_run,key_owner_run))
    archive = projection['source_archive_sha256']
    need(all(r['source_archive_sha256'] == archive for r in (identity_run,key_run,key_owner_run)),
         'connection_receipt_archive_mismatch')
    need(identity_run['manifest']['mapping'] == im['plan']['mapping']
         and identity_run['mapping_sha256'] == im['mapping_sha256']
         and key_run['manifest']['mapping'] == km['credential_receipt']['records']
         and key_run['mapping_sha256'] == km['mapping_sha256'] == key_owner_run['mapping_sha256']
         and om['component_material'] == km and key_owner_run['manifest']['component_scope'] == km['scope'],
         'connection_receipt_material_mismatch')
    namespaces = [im['source_namespace_sha256'],km['source_namespace_sha256'],
                  identity_run['manifest']['source_namespace_sha256'],key_run['manifest']['source_namespace_sha256']]
    need(len(set(namespaces)) == 1, 'connection_receipt_namespace_mismatch')
    users = unique(projection['document']['users'],'id','connection_user_projection_duplicate')
    tokens = unique(projection['document']['tokens'],'id','connection_token_projection_duplicate')
    identities = unique(im['plan']['mapping'],'source_user_id','connection_identity_map_duplicate')
    lineage = unique(im['plan']['auth_version_lineage'],'source_user_id','connection_identity_lineage_duplicate')
    keys = unique(key_run['manifest']['mapping'],'source_key_id','connection_key_map_duplicate')
    records = unique(km['records'],'source_token_id','connection_key_record_duplicate')
    credentials = unique(native_credentials,'id','connection_native_credential_duplicate')
    accounts = unique(native_accounts,'id','connection_native_account_duplicate')
    need(set(users) == set(identities) == set(lineage), 'connection_owner_map_not_complete')
    need(len({r['target_user_id'] for r in identities.values()}) == len(identities)
         and len({r['target_personal_account_id'] for r in identities.values()}) == len(identities),
         'connection_owner_map_not_injective')
    ordinary_ids = {r['id'] for r in tokens.values() if r['oauth_managed'] is False and r['creation_source'] != 'assistant_runtime'}
    need(len(set(projection['ordinary_source_token_ids']))==len(projection['ordinary_source_token_ids'])
         and set(projection['ordinary_source_token_ids']) == ordinary_ids == set(keys) == set(records),
         'connection_ordinary_key_map_not_complete')
    need(len({r['target_credential_id'] for r in keys.values()}) == len(keys), 'connection_key_map_not_injective')
    user_map = []
    for sid, mp in sorted(identities.items()):
        tid, aid = mp['target_user_id'], mp['target_personal_account_id']
        integer(tid,'connection_target_user_invalid');integer(aid,'connection_target_account_invalid')
        need(lineage[sid]['source_canonical_row_sha256'] == users[sid]['source_canonical_row_sha256'],
             'connection_canonical_owner_lineage_changed')
        need(aid in accounts and accounts[aid]['kind'] == 'personal', 'connection_actual_account_missing')
        registered = users[sid]['created_at']
        need(registered is None or registered <= 0 or accounts[aid]['created_time'] == registered,
             'connection_native_registration_changed')
        user_map.append({'source_user_id':sid,'user_id':tid,'account_id':aid})
    credential_map = []
    for sid, row in sorted(keys.items()):
        source, record = tokens[sid], records[sid]
        cid, uid, aid = row['target_credential_id'],row['target_user_id'],row['target_account_id']
        need(source['user_id'] in identities, 'connection_key_owner_unmapped')
        owner = identities[source['user_id']]
        need((owner['target_user_id'],owner['target_personal_account_id']) == (uid,aid), 'connection_key_owner_mismatch')
        need(cid in credentials, 'connection_actual_credential_missing')
        native = credentials[cid]
        need(native['kind'] == 'api_key' and (native['user_id'],native['owner_account_id']) == (uid,aid)
             and (record['target_credential_id'],record['target_user_id'],record['target_personal_account_id']) == (cid,uid,aid)
             and native['digest_sha256'] == row['source_secret_sha256'] == record['digest_sha256']
             and native['user_version'] == record['target_auth_version']
             and native['created_time'] == record['created_time'] == source['created_time']
             and record['source_row_sha256'] == source['source_row_sha256']
             and record['source_deleted_at'] == source['deleted_at'], 'connection_actual_credential_binding_changed')
        credential_map.append({'snapshot_singleton':True,'source_key_id':sid,'target_credential_id':cid,
            'user_id':uid,'account_id':aid,'identity_migration_id':identity_run['migration_id'],
            'key_migration_id':key_run['migration_id'],'key_owner_migration_id':key_owner_run['migration_id']})
    return {'contract':'current.connection-mapping.v1','users':user_map,'omitted_users':[]},credential_map


def prepare_owner(projection, parent, ordinary, full_users, snapshot, contract, *, migration_id, actor):
    A,L = frozen_helpers()
    need(ID.fullmatch(migration_id) is not None, 'connection_migration_id_invalid')
    need(projection['projection_authorizes_write'] is False and projection['scope'] == SCOPE,
         'connection_source_projection_required')
    verify_source_files(projection)
    need(A.P.sha256(parent) == projection['full_parent_source_sha256']
         and A.F.digest(parent['tokens']) == projection['full_parent_tokens_sha256'], 'connection_full_parent_changed')
    need({k:v for k,v in ordinary.items() if k not in ('tokens','source_projection_sha256')} ==
         {k:v for k,v in parent.items() if k not in ('tokens','source_projection_sha256')}
         and ordinary['source_projection_sha256'] != parent['source_projection_sha256']
         and Counter(A.F.digest(r) for r in ordinary['tokens']) == Counter(A.F.digest(r) for r in parent['tokens']
             if r['id'] in projection['ordinary_source_token_ids']), 'connection_ordinary_view_not_exact_partition')
    ir,kr,orr = (snapshot[k] for k in ('identity_run','key_run','key_owner_run'))
    km = kr['manifest']['material'];om = orr['manifest']['material']
    A.validate_mechanics(km);A.T.validate_material(om)
    need(km['source_input_sha256'] == A.P.sha256(ordinary)
         and orr['manifest']['component_material_sha256'] == A.digest(km)
         and om['component_material'] == km,'connection_key_owner_material_binding_changed')
    bindings = L.validate(full_users,parent,km['target_projection'],km['source_namespace'],ir,snapshot['identity_authority'])
    need(bindings == km['current_identity_bindings'], 'connection_current_identity_binding_changed')
    for field in ('source_canonical_export_sha256','source_canonical_export_proof_sha256','source_canonical_receipt_sha256'):
        need(ir['manifest']['material'][field] == projection[field], 'connection_full_canonical_source_mismatch')
    user_map,key_map = map_committed_rows(projection,ir,kr,orr,snapshot['credentials'],snapshot['accounts'])
    validate_owner_snapshot(snapshot,contract,actor,projection['source_kind'])
    row=owner_snapshot_row(projection,user_map,snapshot,contract,migration_id,actor)
    return {'scope':SCOPE,'format_version':1,'snapshot_row':row,'credential_map':key_map,'target_snapshot':snapshot,
        'source_projection':deepcopy(projection),
        'contract':contract,'source_projection_sha256':digest(projection),'source_kind':projection['source_kind'],
        'ddl_sha256':sha(schema_path().read_bytes()),
        'module_sha256':sha(Path(__file__).read_bytes()),'frozen_helpers':FROZEN,
        'automatic_retry_allowed':False,'credential_ids_allocated':0,'approval_created':False}


def validate_owner_snapshot(snapshot,contract,actor,source_kind):
    A,_=frozen_helpers();om=snapshot['key_owner_run']['manifest']['material']
    instance = snapshot['instance']
    A.C.alias_target(instance)
    need(len(contract['component_fingerprints']) == COMPONENT_COUNT
         and contract['component_fingerprints'].get('current-connection-history')==sha(schema_path().read_bytes())
         and snapshot['component_fingerprints'] == contract['component_fingerprints']
         and snapshot['runtime_catalog'] == snapshot['catalog_fingerprint_sha256'] == contract['catalog_fingerprint_sha256'],
         'connection_exact_current_contract_required')
    need(instance['database_owner'] == instance['role'] != instance['session_user']
         and instance['effective_role_can_login'] is False and instance['tcp_connection'] is False
         and type(instance['server_instance_nonce']) is str and HEX.fullmatch(instance['server_instance_nonce']),
         'connection_offline_owner_instance_invalid')
    boundary = snapshot['role_boundary']
    need(boundary['other_connections'] == boundary['prepared_transactions'] == 0
         and boundary['unsafe_runtime'] == [] and boundary['connectors'] == [instance['session_user']],
         'connection_target_not_fenced')
    roles={r['rolname']:r for r in boundary['roles']}
    need(len(roles)==len(boundary['roles']) and instance['role'] in roles and instance['session_user'] in roles,
         'connection_role_coverage_invalid')
    for name,role in roles.items():
        need(not any(role[k] for k in ('rolsuper','rolcreaterole','rolcreatedb','rolreplication','rolbypassrls'))
             and role['rolcanlogin']==(name==instance['session_user']), 'connection_offline_role_invalid')
    need(roles[instance['session_user']]['rolinherit'] is False
         and boundary['database_nonce']=='lmm.migration_instance_nonce='+instance['server_instance_nonce']
         and any(r['role']==instance['role'] and r['member']==instance['session_user'] and r['set'] is True
                 and r['inherit'] is False and r['admin'] is False for r in boundary['memberships'])
         and type(snapshot['ownership_sha256']) is str and HEX.fullmatch(snapshot['ownership_sha256']),
         'connection_owner_role_or_permission_binding_invalid')
    actual_actor = snapshot['migration_actor']
    need(actual_actor['user_id'] == integer(actor,'connection_actor_invalid') and actual_actor['active'] is True
         and actual_actor['account_active'] is True and actual_actor['platform_role'] == 'superadmin',
         'connection_actual_actor_required')
    A.T.validate_inventory(snapshot['state']['inventory'])
    need(set(snapshot['state']['tables'])==set(snapshot['state']['inventory']['tables'])
         and set(snapshot['state']['sequences'])==set(snapshot['state']['inventory']['sequences'])
         and set(TABLES)<=set(snapshot['state']['tables']), 'connection_target_inventory_incomplete')
    need(snapshot['legacy_snapshot_count'] == snapshot['current_snapshot_count'] == snapshot['current_map_count'] == 0,
         'connection_existing_snapshot_requires_resolution')
    if source_kind == 'SYNTHETIC_HISTORY_FIXTURE':
        need(om['synthetic_only'] is True and re.fullmatch(r'lmm_runtime_keys_test_[a-z0-9_]{1,30}',instance['database'])
             and snapshot['database_marker'] == 'lmm-runtime-keys-synthetic:' + om['fixture_nonce'],
             'connection_synthetic_fixture_not_marked')
    else:
        need(source_kind == 'VERIFIED_FULL_ARCHIVE_HISTORY' and om['synthetic_only'] is False,
             'connection_production_cannot_use_synthetic_receipt')


def owner_snapshot_row(projection,user_map,snapshot,contract,migration_id,actor):
    """One row builder, also used to reject altered saved owner parameters."""
    A,_=frozen_helpers();ir,kr,orr=(snapshot[k] for k in ('identity_run','key_run','key_owner_run'))
    km=kr['manifest']['material'];instance=snapshot['instance']
    doc_raw,mapping_raw = canonical(projection['document']),canonical(user_map)
    metadata = {**projection['history_binding'],'contract':'current.connection-export-binding.v1',
        'export_file_sha256':sha(doc_raw.encode()),'ordinary_key_source_sha256':km['source_input_sha256'],
        **{k:projection[k] for k in ('source_canonical_export_sha256','source_canonical_export_proof_sha256',
            'source_canonical_receipt_sha256','full_parent_source_sha256')},
        'raw_history_export_sha256':projection['history_binding']['export_file_sha256'],
        'acquisition_sha256':digest(projection['acquisition'])}
    return {'singleton':True,'migration_id':migration_id,'source_namespace_sha256':km['source_namespace_sha256'],
        'source_archive_sha256':projection['source_archive_sha256'],'export_file_sha256':sha(doc_raw.encode()),
        'document_utf8':doc_raw,'metadata':metadata,'mapping_utf8':mapping_raw,'mapping_sha256':sha(mapping_raw.encode()),
        'identity_migration_id':ir['migration_id'],'identity_mapping_sha256':ir['mapping_sha256'],
        'key_migration_id':kr['migration_id'],'key_owner_migration_id':orr['migration_id'],
        'key_mapping_sha256':kr['mapping_sha256'],'key_material_sha256':A.digest(km),
        'snapshot_observed_at':projection['history_binding']['snapshot_observed_at'],'target_database':instance['database'],
        'target_database_oid':int(instance['database_oid']),'target_system_identifier':instance['system_identifier'],
        'target_nonce':instance['server_instance_nonce'],'catalog_sha256':contract['catalog_fingerprint_sha256'],
        'native_credential_floor':max((r['id'] for r in snapshot['credentials']),default=0),'migration_actor_user_id':actor}


def validate_material(material):
    A,L=frozen_helpers()
    need(material['scope'] == SCOPE and material['format_version'] == 1
         and material['module_sha256'] == sha(Path(__file__).read_bytes())
         and material['ddl_sha256'] == sha(schema_path().read_bytes())
         and material['frozen_helpers'] == FROZEN and material['automatic_retry_allowed'] is False
         and material['credential_ids_allocated'] == 0 and material['approval_created'] is False,
         'connection_saved_material_or_tool_changed')
    need(set(material['snapshot_row']) == set(SNAPSHOT_COLUMNS)
         and all(set(r) == set(MAP_COLUMNS) for r in material['credential_map']), 'connection_saved_row_shape_changed')
    projection=material['source_projection'];snap=material['target_snapshot'];row=material['snapshot_row']
    if projection['source_kind']=='VERIFIED_FULL_ARCHIVE_HISTORY':
        rebuilt,parent,_,tables=verified_source_from_evidence(projection['source_evidence'])
        need(rebuilt==projection,'connection_saved_source_not_exact_rebuild')
        km=snap['key_run']['manifest']['material']
        bindings=L.validate(tables['users'],parent,km['target_projection'],km['source_namespace'],
                            snap['identity_run'],snap['identity_authority'])
        need(bindings==km['current_identity_bindings'],'connection_current_identity_binding_changed')
    else:
        verify_source_files(projection)
    A.validate_mechanics(snap['key_run']['manifest']['material'])
    A.T.validate_material(snap['key_owner_run']['manifest']['material'])
    L.ID.validate_material(snap['identity_run']['manifest']['material'])
    need(digest(projection)==material['source_projection_sha256']
         and canonical(projection['document'])==row['document_utf8']
         and sha(row['document_utf8'].encode())==row['export_file_sha256']
         and sha(row['mapping_utf8'].encode())==row['mapping_sha256'], 'connection_saved_source_projection_changed')
    user_map,key_map=map_committed_rows(projection,snap['identity_run'],snap['key_run'],snap['key_owner_run'],
                                     snap['credentials'],snap['accounts'])
    need(canonical(user_map)==row['mapping_utf8'] and key_map==material['credential_map'],
         'connection_saved_committed_map_changed')
    expected_row=owner_snapshot_row(projection,user_map,snap,material['contract'],row['migration_id'],row['migration_actor_user_id'])
    need(row==expected_row and material['source_kind']==projection['source_kind']
         and ID.fullmatch(row['migration_id']) is not None
         and snap['migration_actor']['user_id']==row['migration_actor_user_id'],
         'connection_saved_owner_row_changed')
    validate_owner_snapshot(snap,material['contract'],row['migration_actor_user_id'],projection['source_kind'])


def observation_sql(mapping, identity_id, key_id, outer_id, *, inventory, contract, runtime_roles, actor):
    A,L = frozen_helpers();F,I,T = A.F,A.I,A.T
    need(all(ID.fullmatch(x) for x in (identity_id,key_id,outer_id)), 'connection_receipt_id_invalid')
    run = lambda mid: '(SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE r.migration_id=' + A.literal(mid) + ')'
    return "SELECT jsonb_build_object('instance',(" + F.instance_sql() + "),'state',(" + T.state_sql(inventory) + ")," \
        "'ownership_sha256',("+F.ownership_sql()+")," \
        "'component_fingerprints',(SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract)," \
        "'runtime_catalog',(SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton)," \
        "'catalog_fingerprint_sha256',(" + I.catalog_hash_sql(contract) + ")," \
        "'role_boundary',(" + T.role_boundary_sql(runtime_roles) + "),'migration_actor',(" + T.actor_sql(actor) + ")," \
        "'identity_run'," + run(identity_id) + ",'key_run'," + run(key_id) + ",'key_owner_run'," + run(outer_id) + "," \
        "'identity_authority',(" + L.identity_authority_sql(mapping) + ")," \
        "'credentials',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',c.id,'kind',c.kind,'user_id',c.user_id," \
        "'owner_account_id',c.owner_account_id,'user_version',c.user_version,'digest_sha256',encode(c.digest,'hex')," \
        "'created_time',floor(extract(epoch FROM c.created_at))::bigint) ORDER BY c.id),'[]'::jsonb) FROM core_identity.credentials c)," \
        "'accounts',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',a.id,'kind',a.kind," \
        "'created_time',floor(extract(epoch FROM a.created_at))::bigint) ORDER BY a.id),'[]'::jsonb) FROM core_identity.accounts a)," \
        "'legacy_snapshot_count',(SELECT count(*) FROM core_history.connection_snapshot)," \
        "'current_snapshot_count',(SELECT count(*) FROM core_history.current_connection_snapshot)," \
        "'current_map_count',(SELECT count(*) FROM core_history.current_connection_credential_map)," \
        "'database_marker',shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database'))"


def readback_sql(material):
    validate_material(material)
    A,_ = frozen_helpers()
    return _readback_expression(material,A)


def _readback_expression(material,A):
    row=material['snapshot_row']
    return "(SELECT (to_jsonb(s)-ARRAY['document','mapping','imported_at'])=" + A.literal(row) + "::jsonb" \
        " AND (SELECT coalesce(jsonb_agg(to_jsonb(cm) ORDER BY cm.source_key_id),'[]'::jsonb)" \
        " FROM core_history.current_connection_credential_map cm)=" + A.literal(material['credential_map']) + "::jsonb" \
        " FROM core_history.current_connection_snapshot s WHERE s.singleton)"


def transaction_sql(material):
    validate_material(material)
    A,L = frozen_helpers();F,T=A.F,A.T
    row=material['snapshot_row'];snap=material['target_snapshot'];inv=snap['state']['inventory']
    locks=','.join(F.relation(t) for t in inv['tables'])
    state='('+T.state_sql(inv)+')'
    expected=A.literal(snap['state'])+'::jsonb'
    names='ARRAY['+','.join(A.literal(t) for t in TABLES)+']'
    guard="IF ("+F.instance_sql()+") IS DISTINCT FROM "+A.literal(snap['instance'])+"::jsonb" \
        " OR ("+A.I.catalog_hash_sql(material['contract'])+")<>"+A.literal(row['catalog_sha256']) \
        +" THEN RAISE EXCEPTION 'current_connection_target_changed'; END IF;"
    runtime_roles=sorted(r['rolname'] for r in snap['role_boundary']['roles'] if r['rolname'] not in
                         (snap['instance']['role'],snap['instance']['session_user']))
    guard+=" IF ("+T.role_boundary_sql(runtime_roles)+") IS DISTINCT FROM "+A.literal(snap['role_boundary'])+"::jsonb" \
        " OR ("+F.ownership_sql()+")<>"+A.literal(snap['ownership_sha256']) \
        +" THEN RAISE EXCEPTION 'current_connection_role_or_permissions_changed'; END IF;"
    checks=A.U78._readback_sql(snap['key_run']['manifest']['material'])
    checks+=L.ID.readback_sql(snap['identity_run']['manifest']['material'])
    values=[]
    for name in SNAPSHOT_COLUMNS:
        value=row[name]
        values.append(A.literal(value)+('::jsonb' if name=='metadata' else ''))
    body=f"""DECLARE after_ jsonb; BEGIN
 {guard}
 LOCK TABLE {locks} IN ACCESS EXCLUSIVE MODE;
 {guard}
 IF EXISTS(SELECT FROM core_history.current_connection_snapshot) THEN
  IF coalesce({_readback_expression(material,A)},FALSE) IS NOT TRUE THEN
   RAISE EXCEPTION 'current_connection_committed_history_conflict'; END IF;
 ELSE
  IF {state} IS DISTINCT FROM {expected} THEN RAISE EXCEPTION 'current_connection_before_state_changed'; END IF;
  {checks}
  INSERT INTO core_history.current_connection_credential_map({','.join(MAP_COLUMNS)})
  SELECT snapshot_singleton,source_key_id,target_credential_id,user_id,account_id,
   identity_migration_id,key_migration_id,key_owner_migration_id
  FROM jsonb_to_recordset({A.literal(material['credential_map'])}::jsonb) AS r(
   snapshot_singleton boolean,source_key_id bigint,target_credential_id bigint,user_id bigint,account_id bigint,
   identity_migration_id text,key_migration_id text,key_owner_migration_id text);
  INSERT INTO core_history.current_connection_snapshot({','.join(SNAPSHOT_COLUMNS)}) VALUES({','.join(values)});
  SET CONSTRAINTS ALL IMMEDIATE;
  IF coalesce({_readback_expression(material,A)},FALSE) IS NOT TRUE THEN
   RAISE EXCEPTION 'current_connection_actual_history_readback_mismatch'; END IF;
  SELECT {state} INTO after_;
  IF after_->'sequences' IS DISTINCT FROM {expected}->'sequences'
   OR (after_->'tables')-{names} IS DISTINCT FROM ({expected}->'tables')-{names}
   OR after_->'inventory' IS DISTINCT FROM {expected}->'inventory'
  THEN RAISE EXCEPTION 'current_connection_unrelated_state_changed'; END IF;
 END IF;
END"""
    tag='$current_connection_'+sha(body.encode())[:24]+'$'
    need(tag not in body,'connection_sql_delimiter_collision')
    return "BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL timezone='UTC'; SET LOCAL row_security=off; SET LOCAL synchronous_commit=on;" \
        " SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='180s';\nDO "+tag+body+tag+"; COMMIT;\n" \
        "SELECT jsonb_build_object('status','COMMITTED','migration_id',"+A.literal(row['migration_id'])+",'automatic_retry_allowed',false);\n"


def resolve_sql(material, original_application_name):
    validate_material(material)
    A,_=frozen_helpers();row=material['snapshot_row']
    need(re.fullmatch(r'lmm-current-connection-[0-9a-f]{32}',original_application_name), 'connection_attempt_identity_invalid')
    return "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\nDO $current_connection_resolution$ BEGIN " \
        "IF current_database()<>"+A.literal(row['target_database']) \
        +" OR (SELECT oid FROM pg_database WHERE datname=current_database())<>"+str(row['target_database_oid']) \
        +" OR (SELECT system_identifier::text FROM pg_control_system())<>"+A.literal(row['target_system_identifier']) \
        +" OR NOT EXISTS(SELECT FROM pg_db_role_setting s CROSS JOIN LATERAL unnest(s.setconfig) c" \
        " WHERE s.setdatabase=(SELECT oid FROM pg_database WHERE datname=current_database()) AND s.setrole=0" \
        " AND c="+A.literal('lmm.migration_instance_nonce='+row['target_nonce'])+")" \
        +" THEN RAISE EXCEPTION 'current_connection_resolution_target_changed'; END IF;" \
        " IF EXISTS(SELECT FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())" \
        " AND application_name="+A.literal(original_application_name)+" AND pid<>pg_backend_pid()) THEN " \
        "RAISE EXCEPTION 'current_connection_original_writer_still_active'; END IF;" \
        " IF EXISTS(SELECT FROM core_history.current_connection_snapshot) AND coalesce("+_readback_expression(material,A)+",FALSE) IS NOT TRUE THEN " \
        "RAISE EXCEPTION 'current_connection_resolution_receipt_conflict'; END IF; END $current_connection_resolution$;\n" \
        "SELECT jsonb_build_object('status',CASE WHEN coalesce("+_readback_expression(material,A)+",FALSE) THEN 'COMMITTED' ELSE 'UNKNOWN' END," \
        "'automatic_retry_allowed',false); COMMIT;\n"


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['source-preview'])
    parser.add_argument('--support-scripts',type=Path,action='append',required=True,
        help='Repeat in order for frozen A513 scripts, reviewed full81 support (c343/L0835/U6a612/W7b/T5b), '
             'and complete original fa57 scripts. Loaded module bytes are pinned; current same-named helpers are rejected.')
    parser.add_argument('--evidence-file',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args(argv)
    try:
        for path in reversed(args.support_scripts):
            need(path.is_dir(),'connection_support_directory_missing')
            sys.path.insert(0,str(path.resolve()))
        A,_=frozen_helpers();evidence_ref=A.K78.file_ref(args.evidence_file)
        evidence=A.K78.read_ref(evidence_ref)
        start=time.monotonic()
        projection,parent,_,_=verified_source_from_evidence(evidence)
        elapsed=time.monotonic()-start
        A.I.private_backup_path(args.output.parent,directory=True)
        args.output.mkdir(mode=0o700)
        A.I.write_private(args.output,'source-projection.private.json',canonical(projection)+'\n')
        result={'scope':SCOPE,'status':'SOURCE_VERIFIED_PENDING_ACTUAL_COMMITTED_IDENTITY_AND_KEYS',
            'source_kind':projection['source_kind'],'counts':projection['document']['counts'],
            'ordinary_key_count':len(projection['ordinary_source_token_ids']),
            'internal_key_count':len(parent['tokens'])-len(projection['ordinary_source_token_ids']),
            'full_canonical_verified':True,'all_four_retained_history_arrays_bound':True,
            'raw_history_file_sha256':projection['raw_history_file']['sha256'],
            'projection_sha256':digest(projection),'evidence_file':evidence_ref,
            'module_sha256':sha(Path(__file__).read_bytes()),
            'ddl_sha256':sha(schema_path().read_bytes()),
            'source_verification_elapsed_seconds':round(elapsed,6),
            'source_queries':0,'target_queries':0,'production_writes':0,'credential_ids_allocated':0,
            'source_production_fenced':False,'financial_approval':False,'apply_supported_by_cli':False}
        A.I.write_private(args.output,'result.public.json',canonical(result)+'\n')
        print(canonical(result))
        return 0
    except Exception as exc:
        # Inputs may contain raw legacy secrets. Never echo exception context,
        # rows, SQL or parser text to a terminal response.
        reason=str(exc) if isinstance(exc,Error) else type(exc).__name__
        print(canonical({'status':'REJECTED','reason':reason,'apply_supported_by_cli':False}),file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
