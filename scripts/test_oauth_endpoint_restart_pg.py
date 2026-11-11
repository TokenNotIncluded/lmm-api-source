"""Opt-in tiny PG18 restart test for the OAuth endpoint receipt boundary.

Synthetic owner facts test the physical-instance proof, not historical identity
import or actual provider login. A full normal CLI rehearsal is separate.
"""
import copy
import importlib.util
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import unittest

import legacy_snapshot_oauth as O

SPEC = importlib.util.spec_from_file_location('endpoint_identity', Path(__file__).with_name('migrate-legacy-snapshot.py'))
I = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(I)


@unittest.skipUnless(os.environ.get('LMM_LOCAL_OAUTH_ENDPOINT_PG') == '1', 'explicit local PG fixture opt-in required')
class OAuthEndpointRestartPostgres(unittest.TestCase):
    def test_same_cluster_restart_and_physical_identity_negatives(self):
        os.umask(0o077)
        parent = Path.home() / '.local/state/lmm-api'
        parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        root = Path(tempfile.mkdtemp(prefix='oauth-endpoint-pg-', dir=parent))
        data, socket = root/'pgdata', root/'sock'
        socket.mkdir(mode=0o700)
        nonce = secrets.token_hex(32)
        port = 65462
        def run(label, argv, sql=None):
            env = {k:v for k,v in os.environ.items() if not k.startswith('PG')}
            p = subprocess.run(argv, input=sql, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               env=env, timeout=60)
            (root/(label+'.stdout.private')).write_text(p.stdout)
            (root/(label+'.stderr.private')).write_text(p.stderr)
            self.assertEqual(p.returncode, 0, label)
            return p.stdout
        def admin(label, sql):
            return run(label, ['psql','-X','-qAt','--no-password','-v','ON_ERROR_STOP=1',
                              '-h',str(socket),'-p',str(port),'-U','fixture_admin','-d','postgres'], sql)
        run('initdb', ['initdb','-D',str(data),'--username=fixture_admin','--auth=trust','--no-locale','--encoding=UTF8'])
        with (data/'postgresql.conf').open('a') as f:
            f.write(f"\nlisten_addresses=''\nunix_socket_directories='{socket}'\nport={port}\nshared_buffers='16MB'\nmax_connections=10\nlmm.migration_instance_nonce='{nonce}'\n")
        started = False
        try:
            run('start', ['pg_ctl','-D',str(data),'-l',str(root/'postgres.private.log'),'-w','start'])
            started = True
            admin('roles', "CREATE ROLE endpoint_owner NOLOGIN; CREATE ROLE endpoint_installer LOGIN NOINHERIT;"
                  "GRANT endpoint_owner TO endpoint_installer; CREATE DATABASE endpoint_target OWNER endpoint_owner TEMPLATE template0;"
                  "REVOKE CONNECT ON DATABASE endpoint_target FROM PUBLIC; GRANT CONNECT ON DATABASE endpoint_target TO endpoint_installer;"
                  "GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO endpoint_owner;")
            passfile=root/'pgpass.private';passfile.write_text('');passfile.chmod(0o600)
            connection={'socket_directory':str(socket),'port':port,'database':'endpoint_target','role':'endpoint_owner',
                        'login_role':'endpoint_installer','set_role':'endpoint_owner','pgpass_file':str(passfile)}
            def query(label, sql, readonly=True):
                return I.psql(connection, sql, readonly=readonly, private_dir=root, label=label)
            query('synthetic-origin-schema', "CREATE SCHEMA core_identity; CREATE TABLE core_identity.privileged_import_facts("
                  "migration_id text,archive_sha256 bytea,system_id bytea);", False)
            old=json.loads(query('original-instance', I.IDENTITY_SQL))
            identifier=json.loads(query('original-system', "SELECT to_json(system_identifier::text) FROM pg_catalog.pg_control_system();"))
            encoded=int(identifier).to_bytes(8,'big').hex()+'00'*24
            target={'status':'initialized-empty-identity-target','migration_id':'synthetic-identity','source_archive_sha256':'a'*64,
                    'connection':connection,'instance_identity':old,'application_or_worker_started':False,'registry_provisioned':True}
            original=I.write_private(root,'original-target.private.json',I.canonical(target)+'\n')
            original_bytes=original.read_bytes()
            query('synthetic-origin-record', "INSERT INTO core_identity.privileged_import_facts VALUES('synthetic-identity',"
                  "decode('"+'a'*64+"','hex'),decode('"+encoded+"','hex'));",False)
            run('restart', ['pg_ctl','-D',str(data),'-l',str(root/'postgres.private.log'),'-m','fast','-w','restart'])
            current=json.loads(query('current-instance', I.IDENTITY_SQL))
            self.assertNotEqual(old['postmaster_started_at'],current['postmaster_started_at'])
            current.pop('read_only')
            current['system_identifier']=identifier
            endpoint=dict(target,migration_id='synthetic-financial-opening',instance_identity=current)
            endpoint_path=I.write_private(root,'current-endpoint.private.json',I.canonical(endpoint)+'\n')
            check_count = 0
            def check(candidate_path):
                nonlocal check_count
                check_count += 1
                evidence = root / f'check-{check_count:02d}'
                evidence.mkdir(mode=0o700)
                return O.resolve_target_endpoint(I,target,original,candidate_path,output=evidence)
            resolved,binding=check(endpoint_path)
            self.assertEqual(resolved['migration_id'],'synthetic-identity')
            self.assertEqual(original.read_bytes(),original_bytes)
            self.assertEqual(binding['original_identity_target_receipt_sha256'],I.sha256_file(original))
            self.assertEqual(binding['current_endpoint_receipt_sha256'],I.sha256_file(endpoint_path))
            negative_count=0
            for field in ['database_oid','database_owner','role','server_instance_nonce','server_version_num','datcollate','datctype','port']:
                bad=copy.deepcopy(endpoint);bad['instance_identity'][field]='changed'
                path=I.write_private(root,'bad-'+field+'.private.json',I.canonical(bad)+'\n')
                with self.assertRaisesRegex(I.MappingError,'stable_identity_mismatch'):
                    check(path)
                negative_count+=1
            bad=copy.deepcopy(endpoint);bad['instance_identity']['system_identifier']=str(int(identifier)+1)
            path=I.write_private(root,'wrong-system.private.json',I.canonical(bad)+'\n')
            with self.assertRaisesRegex(I.MappingError,'system_identifier_mismatch'):
                check(path)
            negative_count+=1
            # A current frame alone cannot re-label a restored database as the original.
            query('synthetic-bad-origin', "DELETE FROM core_identity.privileged_import_facts;",False)
            with self.assertRaisesRegex(I.MappingError,'origin_system_unproven'):
                check(endpoint_path)
            negative_count+=1
            result={'status':'LOCAL_PG_PERSISTENT_RESTART_ENDPOINT_PASS','negative_cases':negative_count,
                    'original_receipt_unchanged':True,'original_receipt_sha256':I.sha256_file(original),
                    'endpoint_receipt_sha256':I.sha256_file(endpoint_path),'synthetic_origin_facts':True,
                    'normal_historical_identity_apply_tested':False,'production_actions':False}
            I.write_private(root,'result.private.json',I.canonical(result)+'\n')
            print(json.dumps({'status':result['status'],'negative_cases':negative_count,'root':str(root)}),flush=True)
        finally:
            if started:
                run('stop', ['pg_ctl','-D',str(data),'-m','fast','-w','stop'])


if __name__=='__main__':
    unittest.main()
