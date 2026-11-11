"""Targeted normal Core fixture for cc52 facts and immutable payment bases."""
import copy
import json
import os
from pathlib import Path
import subprocess
import unittest

import prepare_legacy_settlement_basis as B
import prepare_legacy_payment_bindings as P
from prepare_wallet_history_snapshot import digest, literal
from test_prepare_legacy_payment_bindings import synthetic


@unittest.skipUnless(os.environ.get('LMM_CC52_NATIVE_CONNECTION_FILE'),
                     'explicit fresh private normal Core fixture required')
class CC52SettlementPostgresTests(unittest.TestCase):
    def test_nonidentity_accounts_source_collision_and_owner_boundaries(self):
        connection=json.loads(Path(os.environ['LMM_CC52_NATIVE_CONNECTION_FILE']).read_text())
        self.assertEqual(connection['port'],55510)
        self.assertTrue(connection['database'].startswith('lmm_binding_test_cc52_'))
        self.assertTrue(Path(connection['socket_directory']).is_dir())
        env={k:v for k,v in os.environ.items() if not k.startswith('PG')}
        env.update(PGHOST=connection['socket_directory'],PGPORT=str(connection['port']),
                   PGDATABASE=connection['database'],PGUSER=connection['login_role'],
                   PGOPTIONS='-c role='+connection['role'],PGCONNECT_TIMEOUT='60')
        if connection.get('pgpass_file'):
            env['PGPASSFILE']=connection['pgpass_file']

        def sql(text, success=True):
            result=subprocess.run(['psql','-X','--no-password','-qAt','-v','ON_ERROR_STOP=1'],
                                  input=text,text=True,capture_output=True,env=env,timeout=90)
            if success:
                self.assertEqual(result.returncode,0,result.stderr)
            else:
                self.assertNotEqual(result.returncode,0)
            return result

        actual=json.loads(sql("SELECT jsonb_build_object('session',session_user,'owner',current_user,"
            "'super',(SELECT rolsuper FROM pg_roles WHERE rolname=session_user))").stdout)
        self.assertEqual(actual,{'session':connection['login_role'],'owner':connection['role'],'super':False})
        self.assertEqual(sql('SELECT count(*) FROM core_identity.users').stdout.strip(),'0')
        sql("INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES(7,'personal'),(17,'personal'),(27,'personal');"
            "INSERT INTO core_identity.users(id,personal_account_id) VALUES(7,17),(8,7),(9,27);")
        opening=dict(migration_id='synthetic-opening',source_entity_key='user:7',
            source_archive_sha256='a'*64,mapping_sha256='b'*64,source_row_sha256='c'*64,
            observed_at=100,account_id=17,available_units=0,reserved_units=0,affiliate_units=0)
        second=dict(opening,account_id=7,source_entity_key='legacy.users/8')
        other_archive=dict(second,migration_id='other-archive-opening',source_archive_sha256='d'*64,
                           account_id=27,source_entity_key='user:9')
        sql('BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;'+
            ''.join('SELECT core_billing.import_opening_balance('+literal(v)+'::jsonb);'
                    for v in (opening,second,other_archive))+'COMMIT;')
        facts,review=synthetic(mixed_openings=True)
        source=dict(source_namespace=B.CC52_NAMESPACE,observed_at=facts['observed_at'],
                    source_user_ids=facts['source_user_ids'],tables=facts['tables'])
        oracle=copy.deepcopy(facts['oracle'])
        oracle.update(source_commit=B.CC52_COMMIT,source_profile=B.CC52_PROFILE,
                      source_files_sha256=B.CC52_SOURCE_FILES,source_input_sha256='f'*64)
        facts=B.prepare(source,oracle,archive_sha256='a'*64,expected_rows=2,source_input_sha256='f'*64)
        review['orders'][0]['account_id']=17
        review['orders'][1]['account_id']=7
        review['facts_sha256']=digest(facts)

        # A valid opening for the colliding UID/account 8/7 cannot establish 7/17.
        wrong_archive=copy.deepcopy(facts);wrong_archive['source_archive_sha256']='d'*64
        refused=sql(B.import_sql(wrong_archive),success=False)
        self.assertIn('identity_mapping_mismatch',refused.stderr)
        wrong_users=copy.deepcopy(facts);wrong_users['source_user_ids']=[17]
        refused=sql(B.import_sql(wrong_users),success=False)
        self.assertIn('identity_mapping_mismatch',refused.stderr)
        legacy=copy.deepcopy(facts);legacy.pop('source_profile')
        refused=sql(B.import_sql(legacy),success=False)
        self.assertIn('identity_mapping_mismatch',refused.stderr)
        refused=sql('SET ROLE NONE;'+B.import_sql(facts),success=False)
        self.assertTrue('installation_owner_required' in refused.stderr or 'permission denied' in refused.stderr)
        self.assertEqual(sql('SELECT count(*) FROM core_migration.import_runs').stdout.strip(),'0')
        sql(B.import_sql(facts));sql(B.import_sql(facts))
        self.assertEqual(sql('SELECT count(*) FROM core_migration.import_runs').stdout.strip(),'1')

        before=sql('SELECT count(*) FROM core_billing.ledger_journals').stdout
        wrong_owner=copy.deepcopy(review)
        wrong_owner['orders'][1]['account_id']=17
        refused=sql(P.import_sql(P.prepare(facts,wrong_owner)),success=False)
        self.assertIn('original opening and personal owner',refused.stderr)
        self.assertEqual(sql('SELECT count(*) FROM core_payments.legacy_bases').stdout.strip(),'0')
        plan=P.prepare(facts,review)
        sql(P.import_sql(plan));sql(P.import_sql(plan))
        self.assertEqual(sql('SELECT count(*) FROM core_payments.legacy_bases').stdout.strip(),'2')
        self.assertEqual(sql('SELECT count(*) FROM core_billing.ledger_journals').stdout,before)
        self.assertEqual(sql('SELECT bool_and(b.user_id=u.id AND b.account_id=u.personal_account_id) '
            'FROM core_payments.legacy_bases b JOIN core_identity.users u ON u.id=b.user_id').stdout.strip(),'t')
        changed=copy.deepcopy(review);changed['orders'][0]['merchant']='changed-merchant'
        refused=sql(P.import_sql(P.prepare(facts,changed)),success=False)
        self.assertIn('replay_conflict',refused.stderr)


if __name__=='__main__':
    unittest.main()
