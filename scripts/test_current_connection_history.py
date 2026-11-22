"""Pure source/map cross-join tests; invented facts are not commit receipts.

The real-PG helper must use c343/A513 actual transaction results. These tests
exercise the independent row correspondence, not a fabricated approval gate.
"""
from copy import deepcopy
import hashlib
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import current_connection_history as H

# Boundary tests call the unchanged helper validators. Run with the frozen
# A513/L0835/c343/T5b928/fa57 support directories on PYTHONPATH.


def boundary_fixture():
    nonce=sha('boundary synthetic nonce');owner='test_owner';installer='test_installer'
    def role(name,login):
        return dict(rolname=name,rolsuper=False,rolcreaterole=False,rolcreatedb=False,
                    rolreplication=False,rolbypassrls=False,rolcanlogin=login,rolinherit=False)
    inventory={'tables':sorted({'core_migration.import_runs','core_identity.users','core_identity.accounts',
        'core_meta.component_contract','core_meta.schema_contract','core_meta.runtime_contract',
        'core_gift.legacy_imports','core_subscriptions.reset_codes',*H.TABLES}),'sequences':[]}
    contract={'component_fingerprints':{'synthetic_component_'+str(i):sha(str(i)) for i in range(83)}
              | {'current-connection-history':H.sha(H.schema_path().read_bytes())},
              'catalog_fingerprint_sha256':sha('synthetic boundary catalog')}
    snap={'instance':{'database':'lmm_runtime_keys_test_boundary','database_oid':'123',
        'database_owner':owner,'role':owner,'session_user':installer,'effective_role_can_login':False,
        'tcp_connection':False,'server_instance_nonce':nonce},
        'component_fingerprints':deepcopy(contract['component_fingerprints']),
        'runtime_catalog':contract['catalog_fingerprint_sha256'],
        'catalog_fingerprint_sha256':contract['catalog_fingerprint_sha256'],
        'role_boundary':{'other_connections':0,'prepared_transactions':0,'unsafe_runtime':[],
          'connectors':[installer],'roles':[role(owner,False),role(installer,True),role('test_runtime',False)],
          'memberships':[{'role':owner,'member':installer,'set':True,'inherit':False,'admin':False}],
          'database_nonce':'lmm.migration_instance_nonce='+nonce},
        'ownership_sha256':sha('synthetic ownership'),
        'migration_actor':{'user_id':1,'active':True,'account_active':True,'platform_role':'superadmin'},
        'state':{'inventory':inventory,'tables':{n:{'rows':0,'sha256':sha('')} for n in inventory['tables']},'sequences':{}},
        'legacy_snapshot_count':0,'current_snapshot_count':0,'current_map_count':0,
        'key_owner_run':{'manifest':{'material':{'synthetic_only':True,'fixture_nonce':nonce}}},
        'database_marker':'lmm-runtime-keys-synthetic:'+nonce}
    return snap,contract


class OwnerBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.snapshot,self.contract=boundary_fixture()

    def validate(self):
        return H.validate_owner_snapshot(self.snapshot,self.contract,1,'SYNTHETIC_HISTORY_FIXTURE')

    def test_marked_synthetic_offline_boundary(self):
        self.assertIsNone(self.validate())

    def test_runtime_login_is_rejected(self):
        self.snapshot['role_boundary']['roles'][2]['rolcanlogin']=True
        with self.assertRaisesRegex(H.Error,'offline_role_invalid'):self.validate()

    def test_owner_login_is_rejected(self):
        self.snapshot['instance']['effective_role_can_login']=True
        with self.assertRaisesRegex(H.Error,'offline_owner_instance_invalid'):self.validate()

    def test_extra_database_connector_is_rejected(self):
        self.snapshot['role_boundary']['connectors'].append('other')
        with self.assertRaisesRegex(H.Error,'target_not_fenced'):self.validate()

    def test_active_other_backend_is_rejected(self):
        self.snapshot['role_boundary']['other_connections']=1
        with self.assertRaisesRegex(H.Error,'target_not_fenced'):self.validate()

    def test_prepared_transaction_is_rejected(self):
        self.snapshot['role_boundary']['prepared_transactions']=1
        with self.assertRaisesRegex(H.Error,'target_not_fenced'):self.validate()

    def test_inherited_installer_owner_authority_is_rejected(self):
        self.snapshot['role_boundary']['memberships'][0]['inherit']=True
        with self.assertRaisesRegex(H.Error,'owner_role_or_permission_binding_invalid'):self.validate()

    def test_inactive_real_actor_is_rejected(self):
        self.snapshot['migration_actor']['active']=False
        with self.assertRaisesRegex(H.Error,'actual_actor_required'):self.validate()

    def test_missing_state_table_is_rejected(self):
        self.snapshot['state']['tables'].pop(H.TABLES[1])
        with self.assertRaisesRegex(H.Error,'target_inventory_incomplete'):self.validate()

    def test_synthetic_context_cannot_use_production_receipt_label(self):
        self.snapshot['key_owner_run']['manifest']['material']['synthetic_only']=False
        with self.assertRaisesRegex(H.Error,'synthetic_fixture_not_marked'):self.validate()


class InstallationTests(unittest.TestCase):
    def test_repository_schema_is_the_single_authoritative_file(self):
        with TemporaryDirectory() as directory:
            root=Path(directory);script=root/'scripts/current_connection_history.py'
            schema=root/'services/core/schema/current_connection_history.sql'
            schema.parent.mkdir(parents=True);schema.write_text('-- synthetic path test only\n')
            with patch.object(H,'__file__',str(script)):
                self.assertEqual(H.schema_path(),schema)

    def test_divergent_adjacent_copy_is_rejected(self):
        with TemporaryDirectory() as directory:
            root=Path(directory);script=root/'scripts/current_connection_history.py'
            script.parent.mkdir();script.with_suffix('.sql').write_text('-- divergent copy\n')
            schema=root/'services/core/schema/current_connection_history.sql'
            schema.parent.mkdir(parents=True);schema.write_text('-- canonical path\n')
            with patch.object(H,'__file__',str(script)):
                with self.assertRaisesRegex(H.Error,'schema_copies_disagree'):H.schema_path()

    def test_actually_loaded_unfrozen_helper_is_rejected(self):
        A,_=H.frozen_helpers()
        with TemporaryDirectory() as directory:
            path=Path(directory)/'legacy_api_key_migration.py';path.write_text('# unreviewed helper\n')
            with patch.object(A,'P',SimpleNamespace(__file__=str(path))):
                with self.assertRaisesRegex(H.Error,'frozen_validator_changed'):H.frozen_helpers()


def sha(value):
    return hashlib.sha256(value.encode()).hexdigest()


def fixture():
    users=[{'id':7,'created_at':10,'source_canonical_row_sha256':sha('full-user7')},
           {'id':8,'created_at':20,'source_canonical_row_sha256':sha('full-user8')}]
    tokens=[{'id':1,'user_id':7,'created_time':30,'oauth_managed':False,'creation_source':'manual','deleted_at':None,'source_row_sha256':sha('key1')},
            {'id':2,'user_id':8,'created_time':40,'oauth_managed':False,'creation_source':'drawing_mcp','deleted_at':'2026-01-01T00:00:00Z','source_row_sha256':sha('key2')},
            {'id':3,'user_id':7,'created_time':25,'oauth_managed':True,'creation_source':'manual','deleted_at':None,'source_row_sha256':sha('internal3')},
            {'id':4,'user_id':7,'created_time':24,'oauth_managed':False,'creation_source':'assistant_runtime','deleted_at':None,'source_row_sha256':sha('internal4')}]
    projection={'scope':H.SCOPE,'source_kind':'SYNTHETIC_HISTORY_FIXTURE','source_archive_sha256':sha('synthetic archive'),
                'ordinary_source_token_ids':[1,2],'document':{'users':users,'tokens':tokens}}
    mapping=[{'source_user_id':7,'target_user_id':109,'target_personal_account_id':110,'cohort':'history'},
             {'source_user_id':8,'target_user_id':202,'target_personal_account_id':205,'cohort':'new'}]
    im={'mapping_sha256':sha('identity map'),'source_namespace_sha256':sha('namespace'),
        'plan':{'mapping':mapping,'auth_version_lineage':[{'source_user_id':r['id'],
            'source_canonical_row_sha256':r['source_canonical_row_sha256']} for r in users]}}
    identities={'migration_id':'synthetic-identity-commit','source_archive_sha256':projection['source_archive_sha256'],
        'mapping_sha256':im['mapping_sha256'],'manifest':{'scope':'current-identity-append-completion-v1',
        'source_namespace_sha256':im['source_namespace_sha256'],'mapping':deepcopy(mapping),'material':im}}
    key_map=[{'source_key_id':1,'target_credential_id':17,'target_user_id':109,'target_account_id':110,'source_secret_sha256':sha('digest1')},
             {'source_key_id':2,'target_credential_id':18,'target_user_id':202,'target_account_id':205,'source_secret_sha256':sha('digest2')}]
    records=[{'source_token_id':r['source_key_id'],'target_credential_id':r['target_credential_id'],
        'target_user_id':r['target_user_id'],'target_personal_account_id':r['target_account_id'],
        'digest_sha256':r['source_secret_sha256'],'target_auth_version':3 if i==0 else 1,
        'created_time':tokens[i]['created_time'],'source_row_sha256':tokens[i]['source_row_sha256'],
        'source_deleted_at':tokens[i]['deleted_at']} for i,r in enumerate(key_map)]
    km={'scope':'current-core81-runtime-key-append-v1','source_namespace_sha256':im['source_namespace_sha256'],
        'mapping_sha256':sha('key map'),'records':records,'credential_receipt':{'records':deepcopy(key_map)}}
    keys={'migration_id':'synthetic-key-commit','source_archive_sha256':projection['source_archive_sha256'],
        'mapping_sha256':km['mapping_sha256'],'manifest':{'scope':km['scope'],
        'source_namespace_sha256':km['source_namespace_sha256'],'mapping':deepcopy(key_map),'material':km}}
    outer={'migration_id':'synthetic-key-owner-commit','source_archive_sha256':projection['source_archive_sha256'],
        'mapping_sha256':km['mapping_sha256'],'manifest':{'scope':'current-core81-rights-append-v1',
        'component_scope':km['scope'],'material':{'component_material':deepcopy(km)}}}
    credentials=[{'id':r['target_credential_id'],'kind':'api_key','user_id':r['target_user_id'],
        'owner_account_id':r['target_personal_account_id'],'user_version':r['target_auth_version'],
        'digest_sha256':r['digest_sha256'],'created_time':r['created_time']} for r in records]
    # The source ID overlaps an already-live Session. A preexisting native API
    # key has a lower ID and an earlier timestamp than the snapshot boundary.
    credentials += [{'id':1,'kind':'session','user_id':109,'owner_account_id':110,'user_version':3,
        'digest_sha256':sha('session'),'created_time':50}, {'id':2,'kind':'api_key','user_id':109,
        'owner_account_id':110,'user_version':3,'digest_sha256':sha('native'),'created_time':22}]
    accounts=[{'id':110,'kind':'personal','created_time':10},{'id':205,'kind':'personal','created_time':20}]
    return projection,identities,keys,outer,credentials,accounts


class MappingTests(unittest.TestCase):
    def setUp(self):
        self.f=fixture()

    def mapped(self):
        return H.map_committed_rows(*self.f)

    def test_safe_reallocated_ids_preserve_sessions_and_existing_native_keys(self):
        before=deepcopy(self.f);users,keys=self.mapped()
        self.assertEqual([r['target_credential_id'] for r in keys],[17,18])
        self.assertEqual(users['users'][0],{'source_user_id':7,'user_id':109,'account_id':110})
        imported={r['target_credential_id'] for r in keys}
        self.assertEqual([r['id'] for r in self.f[4] if r['kind']=='api_key' and r['id'] not in imported],[2])
        self.assertEqual(self.f,before)

    def test_missing_committed_identity_is_rejected(self):
        self.f[1]['manifest'].pop('scope')
        with self.assertRaisesRegex(H.Error,'committed_receipts_required'):self.mapped()

    def test_missing_actual_key_owner_receipt_is_rejected(self):
        self.f[3]['manifest']['scope']='approved'
        with self.assertRaisesRegex(H.Error,'committed_receipts_required'):self.mapped()

    def test_wrong_source_archive_is_rejected(self):
        self.f[3]['source_archive_sha256']=sha('other archive')
        with self.assertRaisesRegex(H.Error,'archive_mismatch'):self.mapped()

    def test_wrong_namespace_is_rejected(self):
        self.f[2]['manifest']['source_namespace_sha256']=sha('other namespace')
        with self.assertRaisesRegex(H.Error,'namespace_mismatch'):self.mapped()

    def test_outer_receipt_for_different_key_material_is_rejected(self):
        self.f[3]['manifest']['material']['component_material']['records'][0]['target_credential_id']=99
        with self.assertRaisesRegex(H.Error,'material_mismatch'):self.mapped()

    def test_mapped_source_token_cannot_target_colliding_session(self):
        for row in (self.f[2]['manifest']['mapping'][0],self.f[2]['manifest']['material']['credential_receipt']['records'][0]):
            row['target_credential_id']=1
        self.f[2]['manifest']['material']['records'][0]['target_credential_id']=1
        self.f[3]['manifest']['material']['component_material']=deepcopy(self.f[2]['manifest']['material'])
        with self.assertRaisesRegex(H.Error,'actual_credential_binding_changed'):self.mapped()

    def test_wrong_native_owner_is_rejected(self):
        self.f[4][0]['owner_account_id']=205
        with self.assertRaisesRegex(H.Error,'actual_credential_binding_changed'):self.mapped()

    def test_wrong_native_bearer_digest_is_rejected(self):
        self.f[4][0]['digest_sha256']=sha('different digest')
        with self.assertRaisesRegex(H.Error,'actual_credential_binding_changed'):self.mapped()

    def test_missing_ordinary_key_is_rejected(self):
        for obj in (self.f[2]['manifest']['mapping'],self.f[2]['manifest']['material']['credential_receipt']['records'],
                    self.f[2]['manifest']['material']['records']):obj.pop()
        self.f[3]['manifest']['material']['component_material']=deepcopy(self.f[2]['manifest']['material'])
        with self.assertRaisesRegex(H.Error,'ordinary_key_map_not_complete'):self.mapped()

    def test_internal_billing_key_must_not_be_ordinary(self):
        self.f[0]['ordinary_source_token_ids'].append(3)
        with self.assertRaisesRegex(H.Error,'ordinary_key_map_not_complete'):self.mapped()

    def test_assistant_runtime_key_must_not_be_ordinary(self):
        self.f[0]['ordinary_source_token_ids'].append(4)
        with self.assertRaisesRegex(H.Error,'ordinary_key_map_not_complete'):self.mapped()

    def test_duplicate_partition_ids_are_rejected(self):
        self.f[0]['ordinary_source_token_ids'].append(1)
        with self.assertRaisesRegex(H.Error,'ordinary_key_map_not_complete'):self.mapped()

    def test_missing_current_owner_is_rejected(self):
        self.f[0]['document']['users'].pop()
        with self.assertRaisesRegex(H.Error,'owner_map_not_complete'):self.mapped()

    def test_canonical_owner_digest_change_is_rejected(self):
        self.f[0]['document']['users'][0]['source_canonical_row_sha256']=sha('changed owner')
        with self.assertRaisesRegex(H.Error,'canonical_owner_lineage_changed'):self.mapped()

    def test_current_identity_keeps_original_registration(self):
        self.f[5][0]['created_time']=11
        with self.assertRaisesRegex(H.Error,'native_registration_changed'):self.mapped()

    def test_deleted_key_original_marker_is_not_inferred_from_status(self):
        self.f[0]['document']['tokens'][1]['deleted_at']=None
        with self.assertRaisesRegex(H.Error,'actual_credential_binding_changed'):self.mapped()

    def test_unknown_registration_time_stays_unknown(self):
        self.f[0]['document']['users'][0]['created_at']=None
        self.mapped()
        self.assertIsNone(self.f[0]['document']['users'][0]['created_at'])


class SourceTests(unittest.TestCase):
    def source(self):
        f=fixture();d=f[0]['document']
        out={'contract':'legacy.connection-source.v1','source_schema':'synthetic','coverage':H.COVERAGE,'observed_at':100,
             'users':[{k:r[k] for k in H.SOURCE_FIELDS['users']} for r in d['users']],
             'tokens':[{k:r[k] for k in H.SOURCE_FIELDS['tokens']} for r in d['tokens']],
             'oauth_server_grants':[{'id':'grant','user_id':7,'created_at_ms':31000,'revoked_at_ms':50000}],
             'developer_access_requests':[{'id':1,'user_id':7,'created_at':21,'reviewed_at':None,'status':None}]}
        out['counts']={t:len(out[t]) for t in H.SOURCE_TABLES}
        return out

    def test_all_retained_including_deleted_and_revoked_and_unknown_is_preserved(self):
        raw=self.source();before=deepcopy(raw);H.validate_retained_history(raw)
        self.assertEqual(raw,before)

    def test_missing_retained_row_is_rejected(self):
        raw=self.source();raw['tokens'].pop()
        with self.assertRaisesRegex(H.Error,'source_count_mismatch'):H.validate_retained_history(raw)

    def test_public_history_cannot_contain_raw_keys(self):
        raw=self.source();raw['tokens'][0]['key']='synthetic secret should not be here'
        with self.assertRaisesRegex(H.Error,'source_columns_changed'):H.validate_retained_history(raw)

    def test_boolean_is_not_an_integer_row_count(self):
        raw=self.source();raw['counts']['oauth_server_grants']=True
        with self.assertRaisesRegex(H.Error,'source_count_mismatch'):H.validate_retained_history(raw)

    def test_unknown_key_classification_remains_null(self):
        raw=self.source();raw['tokens'][0]['oauth_managed']=None
        H.validate_retained_history(raw)
        self.assertIsNone(raw['tokens'][0]['oauth_managed'])

    def current(self):
        raw=self.source();current=deepcopy(raw);current['contract']='current.connection-source.v1'
        for row in current['users']:row['source_canonical_row_sha256']=sha('user'+str(row['id']))
        for row in current['tokens']:
            row['creation_source']='manual';row['source_row_sha256']=sha('token'+str(row['id']))
        return raw,current

    def test_rich_current_history_binds_all_four_original_arrays(self):
        raw,current=self.current();H.bind_original_history(current,raw)

    def test_oauth_history_timestamp_tampering_is_rejected(self):
        raw,current=self.current();current['oauth_server_grants'][0]['created_at_ms']+=1000
        with self.assertRaisesRegex(H.Error,'not_original_capture'):H.bind_original_history(current,raw)

    def test_revoked_oauth_history_cannot_be_dropped(self):
        raw,current=self.current();current['oauth_server_grants']=[];current['counts']['oauth_server_grants']=0
        with self.assertRaisesRegex(H.Error,'not_original_capture'):H.bind_original_history(current,raw)

    def test_developer_history_cannot_be_invented_as_approved(self):
        raw,current=self.current();current['developer_access_requests'][0].update(status='approved',reviewed_at=22)
        with self.assertRaisesRegex(H.Error,'not_original_capture'):H.bind_original_history(current,raw)

    def test_application_history_cannot_be_dropped_behind_matching_canonical_hash(self):
        raw,current=self.current();current['developer_access_requests']=[];current['counts']['developer_access_requests']=0
        with self.assertRaisesRegex(H.Error,'not_original_capture'):H.bind_original_history(current,raw)


if __name__=='__main__':
    unittest.main()
