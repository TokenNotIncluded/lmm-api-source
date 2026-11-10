"""One opt-in serial actual Core59/PG18 strict OAuth CLI development test.

Synthetic 246 Google owners; no production login, credentials or funds are used.
Run this exact method, not inherited full-chain tests or discovery.
"""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import test_legacy_schema_profile_pg as P
from test_legacy_google_identity_import import source as google_source

I,F=P.I,P.F


def oauth_source():
    source=P.continuity_source_fixture()
    prototype=copy.deepcopy(source['users'][0])
    source['users'] += [{**prototype,'id':uid,'username':f'old-google-{uid}@example.invalid',
                         'display_name':f'Original {uid}','email':'','role':1}
                        for uid in range(74,317)]
    for u in source['users']:
        u.update(github_id='',linux_do_id='',discord_id='',telegram_id='')
    source['users'][0]['github_id']='710001'
    source['users'][1]['linux_do_id']='720001'
    source['users'][2]['discord_id']='730001'
    source['users'][3]['telegram_id']='740001'
    provider=google_source()['oauth_providers'][0]
    provider['client_secret']='SYNTHETIC-NOT-EXPORTED'
    source['custom_oauth_providers']=[provider]
    source['user_oauth_bindings']=[dict(id=i,provider_id=1,user_id=u['id'],
        provider_user_id=str(100000000000000000000+i)) for i,u in enumerate(source['users'],1)]
    return source


class OAuthSnapshotPostgresTests(P.Schema58PostgresTests):
    @unittest.skipUnless(os.environ.get('LMM_IMPORT_CORE59_BINARY'),'explicit frozen Core59 binary required')
    def test_strict_oauth_cli_246_exact_replay_and_collision(self):
        contract=I.current_contract()
        self.assertEqual(contract['import_profile'],I.CORE59_SEED_CONTRACT)
        root=I.BACKUP_ROOT/'auth01-strict-oauth'
        root.mkdir(mode=0o700,exist_ok=True)
        self.private=Path(tempfile.mkdtemp(prefix='run-',dir=root))
        print('Strict OAuth private evidence: '+str(self.private),flush=True)
        source=oauth_source()
        self.assertEqual(len(source['users']),246)
        expected=[dict(family='oauth',issuer='https://accounts.google.com',subject=r['provider_user_id'],user_id=r['user_id'])
                  for r in source['user_oauth_bindings']]
        expected += [dict(family='oauth',issuer=issuer,subject=subject,user_id=uid) for issuer,subject,uid in (
            ('https://github.com','710001',71),('https://connect.linux.do','720001',72),('https://discord.com','730001',73))]
        expected += [dict(family='telegram',issuer='',subject='740001',user_id=74)]
        expected.sort(key=lambda r:(r['family'],r['issuer'],r['subject']))
        with self.private_cluster():
            (self.output/'private-evidence-path.txt').write_text(str(self.private)+'\n')
            with patch.object(P,'continuity_source_fixture',return_value=source):
                source_path,receipt,original=self.restored_physical_source()
            self.run_command(['psql','-X','-w','-qAt','-v','ON_ERROR_STOP=1','-U','profile_reader'],
                sql='SELECT client_secret FROM public.custom_oauth_providers;',database='profile_restored',
                error='permission denied for table custom_oauth_providers')
            actual_init=self.init_actual_core59()
            self.sql('GRANT EXECUTE ON FUNCTION pg_control_system() TO profile_owner;',owner=False)
            self.inventory=self.query(F.inventory_sql())
            target_contract={**contract,'catalog_fingerprint_sha256':self.sql(I.catalog_hash_sql(contract)),
                'allow_initialized_core_seed':True,'initialized_core_seed_contract':I.CORE59_SEED_CONTRACT,
                'initialized_core_seed_sha256':self.sql(I.fresh_seed_hash_query(I.CORE59_SEED_CONTRACT)),
                'trust_policy_seed':self.query(I.trust_policy_seed_query())}
            target=self.private_json(self.private,'identity-target.private.json',dict(
                status='initialized-empty-identity-target',migration_id='auth01-identity',
                source_archive_sha256=receipt['archive']['sha256'],registry_provisioned=True,
                application_or_worker_started=False,connection=dict(socket_directory=self.env['PGHOST'],
                port=int(self.env['PGPORT']),database='profile_target',role='profile_owner',
                login_role='profile_installer',set_role='profile_owner',pgpass_file=receipt['readonly_reader']['pgpass_file']),
                instance_identity=self.query(F.instance_sql()),contract=target_contract))
            identity=self.private/'identity'
            identity_result=self.import_cli('migrate-legacy-snapshot.py',identity,source_receipt=source_path,
                target_receipt=target,migration_id='auth01-identity',apply=True,
                confirm_source_sha256=receipt['archive']['sha256'])
            self.assertEqual(identity_result['status'],'identity-import-verified')
            args=dict(phase='oauth',source_receipt=source_path,target_receipt=target,
                      identity_mapping=identity/'identity-mapping.private.json',identity_manifest=identity/'manifest.private.json',
                      identity_migration_id='auth01-identity',migration_id='auth01-oauth')
            before=self.query(F.state_sql(self.inventory,'no-exclusion'))
            dry=self.private/'oauth-plan'
            self.assertEqual(self.import_cli('migrate-legacy-snapshot.py',dry,**args)['status'],'oauth-dry-run-sql-plan-ready')
            sql=(dry/'oauth-import.private.sql').read_text()
            collision="INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) VALUES('https://accounts.google.com','100000000000000000001',72);\n"
            self.sql('BEGIN ISOLATION LEVEL SERIALIZABLE;\n'+collision+
                sql.removeprefix('BEGIN ISOLATION LEVEL SERIALIZABLE;\n'),error='oauth target collision')
            self.assertEqual(before,self.query(F.state_sql(self.inventory,'no-exclusion')))
            applied=self.private/'oauth-apply'
            result=self.import_cli('migrate-legacy-snapshot.py',applied,**args,apply=True,
                confirm_source_sha256=receipt['archive']['sha256'])
            self.assertEqual(result['status'],'oauth-import-verified')
            self.assertEqual((result['google_count'],result['oauth'],result['telegram']),(246,249,1))
            after=self.record_phase('oauth',before,['core_identity.oauth_identities','core_telegram_auth.bindings'],[])
            rows=self.query("SELECT jsonb_agg(jsonb_build_object('family',family,'issuer',issuer,'subject',subject,'user_id',user_id)"
                " ORDER BY family COLLATE \"C\",issuer COLLATE \"C\",subject COLLATE \"C\") FROM ("
                "SELECT 'oauth'::text family,issuer,subject,user_id FROM core_identity.oauth_identities UNION ALL "
                "SELECT 'telegram','',subject,user_id FROM core_telegram_auth.bindings) links")
            self.assertEqual(rows,expected)
            self.private_json(self.private,'exact-bindings-readback.private.json',rows)
            profiles=self.query('SELECT jsonb_object_agg(user_id::text,username) FROM core_identity.user_profiles')
            self.assertEqual(profiles,{str(u['id']):u['username'] for u in source['users']})
            self.assertEqual(self.query(sql)['status'],'oauth-import-verified')
            self.assertEqual(after,self.query(F.state_sql(self.inventory,'no-exclusion')))
            replay=self.private/'oauth-replay'
            self.assertEqual(self.import_cli('migrate-legacy-snapshot.py',replay,**args,apply=True,
                confirm_source_sha256=receipt['archive']['sha256'])['status'],'oauth-import-verified')
            self.assertEqual(after,self.query(F.state_sql(self.inventory,'no-exclusion')))
            for name in ('oauth-mapping.private.json','oauth-manifest.private.json','oauth-import.private.sql'):
                self.assertEqual((applied/name).read_bytes(),(replay/name).read_bytes())
            self.import_cli('migrate-legacy-snapshot.py',self.private/'oauth-other-operation',
                **{**args,'migration_id':'auth01-wrong-new-operation'},apply=True,
                confirm_source_sha256=receipt['archive']['sha256'],expected_error='oauth-target-import_database_command_failed')
            self.assertEqual(after,self.query(F.state_sql(self.inventory,'no-exclusion')))
            report=dict(status='STRICT_OAUTH_PG_SELF_TEST_PASS',synthetic_only=True,google_count=246,
                oauth_count=249,telegram_count=1,exact_original_owners=True,existing_usernames_preserved=True,
                collision_transaction_rolled_back=True,same_operation_cli_and_sql_replay_unchanged=True,
                funds_and_all_unowned_state_unchanged=True,reader_client_secret_denied=True,
                actual_core59_init=actual_init,archive_sha256=receipt['archive']['sha256'],
                production_migration=False,provider_network_login=False,independent_acceptance=False,
                private_evidence=str(self.private))
            self.private_json(self.private,'result.private.json',report)
            (self.output/'result.json').write_text(json.dumps(report,indent=2)+'\n')
