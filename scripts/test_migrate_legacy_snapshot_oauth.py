"""Strict CLI/control-flow checks with synthetic rows and mocked psql; no PG proof."""
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import test_migrate_legacy_snapshot as T
from test_legacy_google_identity_import import source as google_source
import legacy_snapshot_oauth as O

I = T.MODULE


class SnapshotOAuthTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'run'
        self.root.mkdir(mode=0o700)
        self.archive = self.root / 'snapshot.dump'
        self.archive.write_bytes(b'synthetic archive, not a production dump')
        self.archive.chmod(0o600)
        self.sha = I.sha256_file(self.archive)
        self.data = google_source()
        self.users = [T.user(71, username='old.email@example.invalid', github_id='70001', telegram_id='80001'),
                      T.user(72, username='second@example.invalid', linux_do_id='90001', discord_id='100001')]
        self.exported = {'providers': self.data['oauth_providers'], 'bindings': self.data['oauth_bindings']}
        records = [dict(user_id=u['id'], account_id=i, username=u['username']) for i,u in enumerate(self.users, 1)]
        self.identity = {'records': records,
                         'mapping_sha256': hashlib.sha256(I.canonical({'source_sha256':self.sha,'records':records}).encode()).hexdigest()}
        self.original = dict(format_version=2, scope='identity-only', source_archive_sha256=self.sha,
                             mapping_sha256=self.identity['mapping_sha256'],
                             mapping_file_sha256=hashlib.sha256((I.canonical(self.identity)+'\n').encode()).hexdigest(),
                             source_target_id_map_sha256=hashlib.sha256(I.id_map_bytes(records).encode()).hexdigest())
        self.contract = dict(component_fingerprints={'identity':'1'*64}, catalog_query_sha256='2'*64,
                             source_file_hashes={}, import_profile='synthetic-reviewed59',
                             identity_version=I.IDENTITY_VERSION,identity_fingerprint_sha256=I.IDENTITY_FINGERPRINT,
                             catalog_query="SELECT 'synthetic'::text AS definition")
        self.source_instance = dict(database_oid=1,database_owner='snapshot_owner',postmaster_started_at='2026-11-01 00:00:00+00',
                                    server_version_num='180000', server_instance_nonce='source-nonce', schema_owner='snapshot_owner')
        self.target = dict(status='initialized-empty-identity-target', migration_id='identity-original',
                           registry_provisioned=True, application_or_worker_started=False,
                           source_archive_sha256=self.sha,
                           connection=dict(role='owner',login_role='installer',set_role='owner',database='target',port=5432),
                           instance_identity={**self.source_instance,'database_oid':2,'database_owner':'owner','server_instance_nonce':'target-nonce'},
                           contract={**self.contract,'catalog_fingerprint_sha256':'3'*64})

    def plan(self):
        return O.mapping(I,self.users,self.exported,self.identity,self.original,self.sha)

    def manifest(self, plan):
        return dict(scope='identity-oauth-only',operation_id='oauth-original',mapping_sha256=hashlib.sha256(I.canonical(plan).encode()).hexdigest(),
                    identity_migration_id='identity-original',
                    identity_manifest_sha256=hashlib.sha256(I.canonical(self.original).encode()).hexdigest())

    def sql(self, plan=None):
        plan = plan or self.plan()
        return O.build_sql(I,plan,self.manifest(plan),self.original,operation='oauth-original',
                           identity_operation='identity-original',target=self.target,contract=self.contract)

    def endpoint(self):
        target_path = self.write('original-target.json', self.target)
        endpoint = copy.deepcopy(self.target)
        endpoint['migration_id'] = 'financial-opening-operation'
        endpoint['instance_identity']['postmaster_started_at'] = '2026-11-02 00:00:00+00'
        endpoint['instance_identity']['system_identifier'] = '123456789'
        return target_path, endpoint

    def resolve(self, target_path, endpoint, *, system='123456789', origin=None):
        self.endpoint_attempt = getattr(self, 'endpoint_attempt', 0) + 1
        endpoint_path = self.write(f'current-endpoint-{self.endpoint_attempt}.json', endpoint)
        encoded = int(system).to_bytes(8, 'big').hex() + '00' * 24
        probe = {'system_identifier': system, 'origin_system_ids': [encoded] if origin is None else origin}
        with patch.object(I, 'psql', return_value=I.canonical(probe)) as call:
            result = O.resolve_target_endpoint(I, self.target, target_path, endpoint_path, output=self.root)
        self.assertTrue(call.call_args.kwargs['readonly'])
        return result, endpoint_path

    def test_endpoint_restart_keeps_historical_identity_and_binds_both_hashes(self):
        path, endpoint = self.endpoint()
        original_bytes = path.read_bytes()
        (resolved, binding), endpoint_path = self.resolve(path, endpoint)
        self.assertEqual(path.read_bytes(), original_bytes)
        self.assertEqual(resolved['migration_id'], 'identity-original')
        self.assertEqual(resolved['status'], 'initialized-empty-identity-target')
        self.assertEqual(resolved['instance_identity'], endpoint['instance_identity'])
        self.assertEqual(binding['original_identity_target_receipt_sha256'], I.sha256_file(path))
        self.assertEqual(binding['current_endpoint_receipt_sha256'], I.sha256_file(endpoint_path))

    def test_endpoint_rejects_other_identity_connection_locale_role_and_version_changes(self):
        path, original = self.endpoint()
        for field in ('database_oid', 'database_owner', 'role', 'session_user', 'server_instance_nonce',
                      'server_version_num', 'datcollate', 'datctype', 'datcollversion', 'port'):
            with self.subTest(field=field):
                endpoint = copy.deepcopy(original)
                endpoint['instance_identity'][field] = 'changed'
                with self.assertRaisesRegex(I.MappingError, 'stable_identity_mismatch'):
                    self.resolve(path, endpoint)
        for field in ('role', 'login_role', 'set_role', 'database', 'port'):
            with self.subTest(connection=field):
                endpoint = copy.deepcopy(original)
                endpoint['connection'][field] = 'changed'
                with self.assertRaisesRegex(I.MappingError, 'connection_mismatch'):
                    self.resolve(path, endpoint)

    def test_endpoint_rejects_unproven_origin_and_wrong_live_system(self):
        path, endpoint = self.endpoint()
        with self.assertRaisesRegex(I.MappingError, 'origin_system_unproven'):
            self.resolve(path, endpoint, origin=[])
        with self.assertRaisesRegex(I.MappingError, 'system_identifier_mismatch'):
            self.resolve(path, endpoint, system='987654321')
        self.target['instance_identity']['system_identifier'] = '111111111'
        path = self.write('different-system-original-target.json', self.target)
        with self.assertRaisesRegex(I.MappingError, 'origin_system_mismatch'):
            self.resolve(path, endpoint)

    def test_normal_cli_endpoint_branch_writes_complete_manifest_once_and_applies(self):
        code,output,calls,stdout,stderr=self.cli(endpoint_restart=True)
        self.assertEqual(code,0,stderr)
        manifest=json.loads((output/'oauth-manifest.private.json').read_text())
        binding=manifest['target_endpoint_binding']
        self.assertEqual(binding['original_identity_target_receipt_sha256'],I.sha256_file(self.root/'target.json'))
        self.assertEqual(binding['current_endpoint_receipt_sha256'],I.sha256_file(self.root/'endpoint.json'))
        self.assertEqual(sum(label=='oauth-target-import' for label,_,_ in calls),1)
        self.assertTrue(json.loads(stdout)['apply'])

    def test_full_provider_cohort_exact_subjects_and_old_usernames(self):
        plan = self.plan()
        self.assertEqual(plan['counts'], {'oauth':4,'telegram':1})
        self.assertEqual(plan['google_count'],1)
        self.assertEqual(plan['users'],self.identity['records'])
        self.assertIn(dict(family='oauth',issuer='https://accounts.google.com',subject='100000000000000000001',user_id=71),plan['records'])
        self.assertEqual({r['issuer'] for r in plan['records'] if r['family']=='oauth'},
                         {'https://accounts.google.com','https://github.com','https://connect.linux.do','https://discord.com'})
        self.assertIn(dict(family='telegram',issuer='',subject='80001',user_id=71),plan['records'])

    def test_strict_phase_246_google_subjects_keep_exact_existing_owners(self):
        self.users = [T.user(1000+i, username=f'existing-{i}@example.invalid') for i in range(1,247)]
        records = [dict(user_id=u['id'],account_id=i,username=u['username']) for i,u in enumerate(self.users,1)]
        self.identity = {'records':records,'mapping_sha256':hashlib.sha256(I.canonical(
            {'source_sha256':self.sha,'records':records}).encode()).hexdigest()}
        self.original.update(mapping_sha256=self.identity['mapping_sha256'],
            mapping_file_sha256=hashlib.sha256((I.canonical(self.identity)+'\n').encode()).hexdigest(),
            source_target_id_map_sha256=hashlib.sha256(I.id_map_bytes(records).encode()).hexdigest())
        self.exported['bindings'] = [dict(id=i,provider_id=1,user_id=1000+i,
            provider_user_id=str(100000000000000000000+i)) for i in range(1,247)]
        code,output,calls,stdout,stderr=self.cli()
        self.assertEqual(code,0,stderr)
        actual=json.loads((output/'oauth-mapping.private.json').read_text())
        self.assertEqual(actual['google_count'],246)
        self.assertEqual({r['subject']:r['user_id'] for r in actual['records']},
                         {r['provider_user_id']:r['user_id'] for r in self.exported['bindings']})
        self.assertEqual(actual['users'],records)
        self.assertIn('core_identity.oauth_identities',calls[-1][1])

    def test_same_cli_operation_recreates_identical_replay_materials(self):
        code,first,_,_,stderr=self.cli()
        self.assertEqual(code,0,stderr)
        self.root=self.root/'replay'
        self.root.mkdir(mode=0o700)
        code,second,_,_,stderr=self.cli()
        self.assertEqual(code,0,stderr)
        for name in ('oauth-mapping.private.json','oauth-manifest.private.json','oauth-import.private.sql'):
            self.assertEqual((first/name).read_bytes(),(second/name).read_bytes())

    def test_sql_dollar_markers_in_existing_username_are_preserved(self):
        self.users[0]['username']='old END $oauth_stage$; name'
        records=copy.deepcopy(self.identity['records'])
        records[0]['username']=self.users[0]['username']
        self.identity={'records':records,'mapping_sha256':hashlib.sha256(I.canonical(
            {'source_sha256':self.sha,'records':records}).encode()).hexdigest()}
        self.original.update(mapping_sha256=self.identity['mapping_sha256'],
            mapping_file_sha256=hashlib.sha256((I.canonical(self.identity)+'\n').encode()).hexdigest(),
            source_target_id_map_sha256=hashlib.sha256(I.id_map_bytes(records).encode()).hexdigest())
        self.assertIn('old END $oauth_stage$; name',self.sql())

    def test_google_origin_drift_and_orphan_binding_refuse(self):
        for field, value in [('authorization_endpoint','https://evil.invalid/auth'),('user_id_field','email'),
                             ('token_endpoint','https://oauth2.googleapis.com/token?alternate=1')]:
            with self.subTest(field=field):
                exported = copy.deepcopy(self.exported)
                exported['providers'][0][field]=value
                with self.assertRaises(I.MappingError):
                    O.mapping(I,self.users,exported,self.identity,self.original,self.sha)
        self.exported['bindings'][0]['user_id']=999
        with self.assertRaises(I.MappingError): self.plan()

    def test_telegram_invalid_or_duplicate_never_silently_skipped(self):
        for subject in (' 80001','80001 ',0,True,'0','01',str(1<<63),'x'*256):
            with self.subTest(subject=subject):
                self.users[0]['telegram_id']=subject
                with self.assertRaisesRegex(I.MappingError,'telegram_source_subject_invalid'): self.plan()
        self.users[0]['telegram_id']=self.users[1]['telegram_id']='80001'
        with self.assertRaisesRegex(I.MappingError,'telegram_source_owner_collision'): self.plan()

    def test_identity_original_bytes_mapping_and_username_are_bound(self):
        for mutate in (lambda: self.original.update(mapping_sha256='0'*64),
                       lambda: self.original.update(source_archive_sha256='0'*64),
                       lambda: self.original.update(source_target_id_map_sha256='0'*64)):
            old=copy.deepcopy(self.original)
            mutate()
            with self.assertRaisesRegex(I.MappingError,'oauth_identity_proof_mismatch'): self.plan()
            self.original=old
        self.users[0]['username']='replacement-name'
        with self.assertRaisesRegex(I.MappingError,'oauth_source_username_mismatch'): self.plan()

    def test_sql_plain_inserts_exact_readback_collision_and_receipt_guards(self):
        sql=self.sql()
        self.assertNotIn('ON CONFLICT',sql)
        self.assertIn("RAISE EXCEPTION 'oauth target collision'",sql)
        self.assertIn('previous.manifest<>',sql)
        self.assertIn('actual IS DISTINCT FROM',sql)
        self.assertIn("revoked_at IS NOT NULL",sql)
        self.assertIn('core_migration.import_runs',sql)
        self.assertIn('SERIALIZABLE',sql)
        self.assertIn('synchronous_commit=on',sql)
        self.assertIn('pg_postmaster_start_time()',sql)
        self.assertIn('original identity receipt mismatch',sql)
        self.assertIn("u.personal_account_id IS DISTINCT FROM",sql)
        self.assertNotIn('core_billing.',sql)
        self.assertNotIn('UPDATE ',sql)
        self.assertNotIn('DELETE ',sql)
        self.assertNotIn('CREATE TABLE',sql)
        self.assertTrue(sql.endswith('COMMIT;\n'))

    def test_operation_cannot_reuse_identity_id_and_instance_requires_nonce(self):
        plan=self.plan()
        with self.assertRaisesRegex(I.MappingError,'oauth_operation_invalid'):
            O.build_sql(I,plan,self.manifest(plan),self.original,operation='identity-original',
                        identity_operation='identity-original',target=self.target,contract=self.contract)
        self.target['instance_identity']['server_instance_nonce']=''
        with self.assertRaisesRegex(I.MappingError,'oauth_instance_receipt_invalid'): self.sql()

    def test_existing_receipt_canonical_oid_string_and_integer_are_equivalent(self):
        integer_sql=self.sql()
        self.target['instance_identity']['database_oid']='2'
        self.assertEqual(integer_sql,self.sql())
        for bad in ('02','2;SELECT 1',True,0,-1,str(1<<32)):
            with self.subTest(value=bad):
                self.target['instance_identity']['database_oid']=bad
                with self.assertRaisesRegex(I.MappingError,'oauth_instance_receipt_invalid'):self.sql()

    def test_offline_column_grants_and_export_never_select_secret(self):
        grants=O.reader_grants('public','profile_reader')
        self.assertNotIn('ALL TABLES',grants)
        self.assertNotIn('client_secret',grants)
        self.assertIn('GRANT SELECT (id,name,slug,enabled,client_id,',grants)
        queries=O.export_queries('public')
        self.assertIn("NOT has_column_privilege",queries)
        self.assertIn("'client_secret','SELECT'",queries)
        self.assertNotIn('row_to_json',queries)
        self.assertNotIn("'client_secret',client_secret",queries)
        with self.assertRaises(ValueError): O.reader_grants('public;COMMIT','reader')

    def write(self,name,data):
        return I.write_private(self.root,name,I.canonical(data)+'\n')

    def cli(self, *, reader_safe=True, failure=None, bad_ack=False, endpoint_restart=False):
        baseline=self.write('native-table-row-count-baseline.jsonl',
                            {'table':'users','row_count':len(self.users)})
        with baseline.open('a') as f:
            for name,rows in zip(O.SOURCE_TABLES,(self.exported['providers'],self.exported['bindings'])):
                f.write(I.canonical({'table':name,'row_count':len(rows)})+'\n')
        source=dict(status='snapshot-restored-baseline-verified-frozen-and-classified',
                    archive=dict(path=str(self.archive),sha256=self.sha),source=dict(schema='public'),
                    private_metadata_files=[str(baseline)],source_instance_identity=self.source_instance,
                    readonly_reader=dict(role='profile_reader',verified_no_application_write_privileges=True,
                                         verified_no_security_definer_application_function_execute=True,row_security_off_guard_verified=True))
        source_path=self.write('source.json',source)
        target_path=self.write('target.json',self.target)
        identity_path=self.write('identity.json',self.identity)
        manifest_path=self.write('manifest.json',self.original)
        output=self.root/'oauth-output'
        endpoint=None
        if endpoint_restart:
            endpoint=copy.deepcopy(self.target)
            endpoint['instance_identity']['postmaster_started_at']='2026-11-02 00:00:00+00'
            endpoint['instance_identity']['system_identifier']='123456789'
            endpoint_path=self.write('endpoint.json',endpoint)
        calls=[]
        def psql(connection,sql,*,readonly,private_dir,label):
            calls.append((label,sql,readonly))
            if label=='source-identity': return I.canonical(self.source_instance)
            if label=='source-export':
                tagged=[('identity',self.source_instance),('schema_identity',{'schema':'public','schema_owner':'snapshot_owner'}),
                        ('user_count',len(self.users)),('oauth_reader',reader_safe),
                        ('custom_oauth_providers_count',len(self.exported['providers'])),
                        ('user_oauth_bindings_count',len(self.exported['bindings']))]
                tagged += [('user',u) for u in self.users]
                tagged += [('oauth_provider',p) for p in self.exported['providers']]
                tagged += [('oauth_binding',b) for b in self.exported['bindings']]
                return '\n'.join(I.canonical({'kind':k,'row':r}) for k,r in tagged)+'\n'
            if label=='oauth-target-system':
                return I.canonical({'system_identifier':'123456789','origin_system_ids':[int(123456789).to_bytes(8,'big').hex()+'00'*24]})
            if label=='oauth-target-identity':
                return I.canonical((endpoint or self.target)['instance_identity'])
            if label=='oauth-target-import':
                if failure: raise I.MappingError(failure)
                plan=self.plan()
                return I.canonical(dict(status='oauth-import-verified',operation_id='oauth-original',
                                        mapping_sha256='0'*64 if bad_ack else self.manifest(plan)['mapping_sha256'],
                                        oauth_count=plan['counts']['oauth'],telegram_count=plan['counts']['telegram'],google_count=plan['google_count']))+'\n'
            raise AssertionError(label)
        argv=['snapshot','--phase','oauth','--source-receipt',str(source_path),'--target-receipt',str(target_path),
              '--identity-mapping',str(identity_path),'--identity-manifest',str(manifest_path),
              '--identity-migration-id','identity-original','--migration-id','oauth-original','--output',str(output),
              '--apply','--confirm-source-sha256',self.sha]
        if endpoint_restart:
            argv += ['--target-endpoint-receipt',str(endpoint_path)]
        stdout,stderr=io.StringIO(),io.StringIO()
        with patch.object(I,'BACKUP_ROOT',self.root.parent),patch.object(I,'current_contract',return_value=self.contract),\
             patch.object(I,'identity_contract',return_value={}),patch.object(I,'verify_instance'),\
             patch.object(I,'psql',side_effect=psql),patch('sys.argv',argv),\
             contextlib.redirect_stdout(stdout),contextlib.redirect_stderr(stderr):
            code=I.main()
        return code,output,calls,stdout.getvalue(),stderr.getvalue()

    def test_real_cli_phase_exports_provenance_and_dispatches_owner_transaction(self):
        code,output,calls,stdout,stderr=self.cli()
        self.assertEqual(code,0,stderr)
        self.assertEqual(json.loads(stdout)['status'],'oauth-import-verified')
        self.assertEqual([c[0] for c in calls],['source-identity','source-export','oauth-target-identity','oauth-target-import'])
        self.assertIn("'provider_user_id',provider_user_id",calls[1][1])
        self.assertNotIn('client_secret,',calls[1][1])
        self.assertEqual(calls[1][1].count('COMMIT;'),1)
        self.assertLess(calls[1][1].index("'oauth_provider'"),calls[1][1].index('COMMIT;'))
        self.assertNotIn('identity-import.private.sql',[p.name for p in output.iterdir()])
        self.assertEqual(json.loads((output/'oauth-mapping.private.json').read_text())['users'],self.identity['records'])
        for p in output.iterdir(): self.assertEqual(p.stat().st_mode&0o777,0o600)
        self.assertNotIn('old.email',stdout+stderr)
        self.assertNotIn('100000000000000000001',stdout+stderr)

    def test_real_cli_reader_secret_privilege_refuses_before_any_target_access(self):
        code,output,calls,stdout,stderr=self.cli(reader_safe=False)
        self.assertEqual(code,1)
        self.assertIn('oauth_reader_client_secret_readable',stderr)
        self.assertEqual([c[0] for c in calls],['source-identity','source-export'])
        self.assertFalse((output/'oauth-import.private.sql').exists())

    def test_real_cli_failed_commit_does_not_report_success_or_retry(self):
        code,output,calls,stdout,stderr=self.cli(failure='oauth-target-import_database_command_failed')
        self.assertEqual(code,1)
        self.assertEqual(json.loads(stderr)['status'],'apply-outcome-unconfirmed')
        self.assertEqual(stdout,'')
        self.assertTrue((output/'oauth-manifest.private.json').exists())
        self.assertEqual(sum(c[0]=='oauth-target-import' for c in calls),1)

    def test_real_cli_lost_or_wrong_ack_keeps_original_replay_materials(self):
        code,output,calls,stdout,stderr=self.cli(bad_ack=True)
        self.assertEqual(code,1)
        self.assertEqual(json.loads(stderr)['status'],'apply-outcome-unconfirmed')
        self.assertIn('oauth_commit_ack_invalid',stderr)
        self.assertTrue((output/'oauth-mapping.private.json').exists())
        self.assertTrue((output/'oauth-import.private.sql').exists())


if __name__=='__main__': unittest.main()
