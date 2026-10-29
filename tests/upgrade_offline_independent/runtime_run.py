#!/usr/bin/env python3
"""Independent oracle; PG/systemd mechanics adapted from test-upgrade-offline.py.

Does not import or call the author's setup/exercise/runtime_checkpoint oracle.
Only synthetic v6 databases, actual old/new CLI, private sockets and temporary units.
"""
import argparse
import fcntl
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import time
from urllib.parse import urlencode

CORE_SHA = 'dea60909b00bed72db09772200db99233b9eb68bdb9a62197aa38288786d71aa'
MANIFEST_SHA = '47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078'
CATALOG_SHA = '557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d'
PUBLIC = ['schema_contract', 'component_contract', 'runtime_contract', 'schema_upgrades', 'component_upgrades', 'ledger_revisions']
PRIVATE = ['offline_upgrades', 'console_revisions']


def require(ok, message):
    if not ok:
        raise AssertionError(message)


def sha(p):
    with Path(p).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


class Acceptance:
    def __init__(self, args):
        self.args, self.root = args, args.root
        self.env = {'PATH': '/usr/bin:/bin', 'HOME': str(Path.home()), 'LANG': 'C', 'TZ': 'UTC',
                    'XDG_RUNTIME_DIR': f'/run/user/{os.getuid()}', 'DBUS_SESSION_BUS_ADDRESS': f'unix:path=/run/user/{os.getuid()}/bus'}
        self.commands, self.http_events, self.clusters = [], [], []
        self.process = self.unit = self.unitfile = None
        self.secret_files = []
        self.status = {name: 'NOT_RUN' for name in ['v6_offline_upgrade', 'manual_fixture_reopen', 'owner_only_sql',
                       'restricted_native_http', 'missing_public_contract', 'missing_billing_read',
                       'private_select_misgrants', 'excess_database_create']}
        self.current = 'v6_offline_upgrade'

    def save(self, name, value):
        (self.root/name).write_text(json.dumps(value, indent=2)+'\n')

    def run(self, label, argv, body=None, env=None, expected=0, timeout=90):
        result = subprocess.run(list(map(str, argv)), input=body, capture_output=True, env=env or self.env, timeout=timeout)
        name = f'{len(self.commands):03d}-{label}.log'
        (self.root/name).write_bytes(result.stdout+result.stderr)
        self.commands.append({'label': label, 'argv': list(map(str, argv)), 'exit': result.returncode,
                              'input_sha256': hashlib.sha256(body).hexdigest() if body else None, 'log': name})
        self.save('commands.json', self.commands)
        if expected is not None:
            require(result.returncode == expected, f'{label}: exit {result.returncode}; see {name}')
        return result

    def sql(self, query, cluster=None, db='coretarget', user='postgres', expect=0, label='sql', owner=False):
        c = cluster or self.target
        env = dict(self.env)
        if user == 'lmm_core_runtime':
            env['PGPASSWORD'] = self.runtime_password
        if user == 'lmm_core_installer':
            env['PGPASSWORD'] = self.installer_password
        if owner:
            env['PGOPTIONS'] = '-c role=lmm_core_owner'
        return self.run(label, ['psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=verbose',
                        '-h', c['socket'], '-p', '55601', '-U', user, '-d', db], query.encode(), env, expect)

    def scalar(self, query, **kw):
        return self.sql(query, **kw).stdout.decode().strip()

    def cluster(self, name):
        d = self.root/name
        d.mkdir(); (d/'socket').mkdir()
        c = {'root': d, 'data': d/'data', 'socket': d/'socket', 'hba': d/'external-hba.conf', 'started': False}
        c['hba'].write_text('local all all trust\n')
        self.clusters.append(c)
        self.run('initdb-'+name, ['initdb', '-D', c['data'], '-U', 'postgres', '--no-locale', '--encoding=UTF8', '--auth=trust', '--wal-segsize=1'])
        options = f"-c listen_addresses= -p 55601 -c unix_socket_directories={c['socket']} -c unix_socket_permissions=0700 -c hba_file={c['hba']} -c shared_buffers=16MB -c max_connections=20 -c min_wal_size=2MB -c max_wal_size=128MB"
        self.run('pg-start-'+name, ['pg_ctl', '-D', c['data'], '-l', d/'postgres.log', '-o', options, '-w', 'start'])
        c['started'] = True
        return c

    def url(self, c, role, db, password='', owner=False):
        query = {'host': str(c['socket']), 'port': 55601, 'sslmode': 'disable'}
        if owner:
            query['options'] = '-c role=lmm_core_owner'
        return 'postgresql://'+role+(':'+password if password else '')+'@localhost/'+db+'?'+urlencode(query)

    def secret_file(self, name, text):
        p = self.root/name
        p.write_text(text)
        self.secret_files.append(p)
        return p

    def config(self, name, url):
        file = self.secret_file(name+'.url', url)
        return self.secret_file(name+'.toml', 'version=1\n[database]\nurl_file='+json.dumps(str(file))+'\n')

    def prepare(self):
        self.source = self.root/'source'
        pins = json.loads((self.args.freeze/'source-pins.json').read_text())
        for name in pins:
            target = self.source/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(self.args.freeze/'source'/name, target)
            require(sha(target) == pins[name], 'source_copy_drift:'+name)
        self.save('source-pins.json', pins)
        self.core = self.root/'actual-core'
        self.run('freeze-native', ['cp', '--reflink=auto', self.args.freeze/'lmm-core', self.core])
        require(sha(self.core) == CORE_SHA, 'candidate_SHA_mismatch')
        manifest = self.run('native-manifest', [self.core, 'schema-manifest']).stdout
        require(hashlib.sha256(manifest).hexdigest() == MANIFEST_SHA, 'native_manifest_changed')
        self.manifest = json.loads(manifest)
        self.target = self.cluster('target')
        self.sql('CREATE DATABASE coretarget', db='postgres')
        self.run('canonical-roles', ['psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-h', self.target['socket'], '-p', '55601', '-U', 'postgres', '-d', 'coretarget', '-f', self.source/'deployment/postgres/create-core-roles.sql'])
        self.installer_password = secrets.token_hex(24)
        self.owner_password = 'Fixture!'+secrets.token_hex(12)
        self.sql("ALTER DATABASE coretarget OWNER TO lmm_core_owner; SET password_encryption='scram-sha-256'; CREATE ROLE lmm_core_installer LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '"+self.installer_password+"'; GRANT lmm_core_owner TO lmm_core_installer WITH INHERIT FALSE, SET TRUE; GRANT CONNECT ON DATABASE coretarget TO lmm_core_installer; CREATE ROLE lmm_extensions LOGIN; REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_control_system() TO lmm_core_owner")
        owner = self.config('owner', self.url(self.target, 'lmm_core_installer', 'coretarget', self.installer_password, True))
        self.run('old-v6-init-db', [self.args.previous_core, '--config', owner, 'init-db'])
        receipt = json.loads(self.run('old-first-owner', [self.args.previous_core, '--config', owner, 'create-first-owner'],
                            json.dumps({'username': 'runtimeowner', 'display_name': 'Independent fixture', 'password': self.owner_password}).encode()).stdout)
        require(receipt['setup_completed'] is True, 'old_owner_creation_failed')
        require(self.scalar('SELECT version FROM core_meta.schema_contract WHERE singleton') == '6', 'not_actual_v6')
        require(self.scalar('SELECT count(*) FROM core_meta.component_contract') == '27', 'not_actual_27_components')
        self.run('official-grants', ['psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-h', self.target['socket'], '-p', '55601', '-U', 'postgres', '-d', 'coretarget', '-v', 'enable_console=true', '-v', 'enable_web_identity=true', '-f', self.source/'deployment/postgres/grant-core-runtime.sql'])
        self.sql("INSERT INTO core_console.options(key,value) VALUES('Notice','Independent v6 predecessor Notice') ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        self.password_digest_sql = "SELECT encode(sha256(convert_to(coalesce(jsonb_agg(to_jsonb(p) ORDER BY to_jsonb(p)::text),'[]')::text,'UTF8')),'hex') FROM core_identity.password_logins p"
        self.password_before = self.scalar(self.password_digest_sql)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0)); self.port = sock.getsockname()[1]
        runtime_url = self.secret_file('runtime.url', self.url(self.target, 'lmm_core_runtime', 'coretarget'))
        key = self.root/'refresh.key'; key.write_bytes(secrets.token_bytes(32)); self.secret_files.append(key)
        self.runtime_config = self.secret_file('runtime.toml', f'''version=1
[server]
listen="127.0.0.1:{self.port}"
worker_threads=1
drain_timeout_seconds=3
[database]
url_file={json.dumps(str(runtime_url))}
max_connections=2
[console]
enabled=true
[identity]
registration_enabled=false
password_register_enabled=false
oauth_register_enabled=false
registration_disabled_methods=[]
email_verification_enabled=false
email_domain_restriction_enabled=false
email_domain_whitelist=[]
[identity.web]
origin="http://127.0.0.1:{self.port}"
refresh_key_file={json.dumps(str(key))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
''')
        self.unit = 'lmm-runtime-old-'+secrets.token_hex(6)+'.service'
        self.unitfile = Path(self.env['XDG_RUNTIME_DIR'])/'systemd/user'/self.unit
        self.unitfile.write_text(f'[Unit]\nDescription=Independent synthetic v6 predecessor\n[Service]\nSlice={os.environ["RUNTIME_ACCEPTANCE_SLICE"]}\nExecStart={self.args.previous_core} --config {self.runtime_config} serve\nKillMode=control-group\nTimeoutStopSec=15\nLimitCORE=0\n')
        self.run('daemon-reload', ['systemctl', '--user', 'daemon-reload'])
        self.run('old-runtime-start', ['systemctl', '--user', 'start', self.unit])
        self.wait_ready(None)
        self.authority = self.cluster('authority')
        self.sql('CREATE ROLE history_owner NOLOGIN; CREATE ROLE history_writer LOGIN; GRANT CREATE ON DATABASE postgres TO history_owner', cluster=self.authority, db='postgres')
        authority_id = secrets.token_hex(32)
        install = self.config('history-install', self.url(self.authority, 'postgres', 'postgres'))
        self.run('history-install', [self.core, '--config', install, 'upgrade-history'], json.dumps({'action':'install','authority_id':authority_id,'owner':'history_owner','writer':'history_writer'}).encode())
        self.operator = self.config('operator', self.url(self.target, 'lmm_core_installer', 'coretarget', self.installer_password))
        history = self.secret_file('history.url', self.url(self.authority, 'history_writer', 'postgres'))
        management = self.secret_file('management.url', self.url(self.target, 'postgres', 'postgres'))
        probe = self.secret_file('probe.url', self.url(self.target, 'lmm_core_runtime', 'coretarget'))
        identity = json.loads(self.scalar("SELECT json_build_object('system',system_identifier::text,'oid',(SELECT oid::bigint FROM pg_database WHERE datname='coretarget')) FROM pg_control_system()"))
        self.request = {'version':3,'action':'apply','operation_id':'independent-runtime-'+secrets.token_hex(16), 'candidate_sha256':CORE_SHA,
            'authority_id':authority_id,'history_url_file':str(history),'management_url_file':str(management),'runtime_probe_url_file':str(probe),
            'hba_file':str(self.target['hba']),'runtime':{'console':True,'assistant_agent':False},
            'services':[{'name':self.unit,'scope':'user','executable_sha256':sha(self.args.previous_core)}],
            'bridge':{'version':3,'procedure':'core-offline-upgrade-v3','predecessor':'fresh_v6_core27',
              'predecessor_manifest_sha256':'59bc164463df00df11a253feecbf9e08dd914e6e3cc7fcb3af68d193287eb005',
              'predecessor_catalog_sha256':'acbee20015ea1397adfda06232f25c811956823e00b15e574d74794702f418e5',
              'successor_manifest_sha256':sha(self.source/'services/core/src/upgrade_offline/target.json'),
              'database_name':'coretarget','database_oid':identity['oid'],'system_identifier':identity['system'],
              'installer':'lmm_core_installer','owner':'lmm_core_owner','runtime':'lmm_core_runtime','extensions':'lmm_extensions'}}
        self.save('upgrade-request.json', self.request)
        self.secret_file('credentials.json', json.dumps({'owner_password':self.owner_password,'installer_password':self.installer_password}))

    def upgrade(self):
        self.prepare()
        result = json.loads(self.run('actual-upgrade-apply', [self.core,'--config',self.operator,'upgrade-offline'], json.dumps(self.request).encode(), timeout=180).stdout)
        require(result.get('status') == 'committed', 'upgrade_not_committed')
        require(self.scalar("SELECT NOT datallowconn FROM pg_database WHERE datname='coretarget'", db='postgres') == 't', 'upgrade_did_not_keep_database_closed')
        state = self.run('old-unit-fenced', ['systemctl','--user','show',self.unit,'--property=MainPID,ActiveState,UnitFileState']).stdout.decode()
        require('MainPID=0' in state and 'UnitFileState=masked' in state, 'old_unit_not_fenced')
        require(self.target['hba'].read_text().startswith('# lmm offline v3 '), 'HBA_fence_missing')
        require(self.scalar('SELECT phase FROM lmm_upgrade_history.operations', cluster=self.authority, db='postgres') == 'committed', 'external_history_not_committed')
        self.save('upgrade-independent-result.json', {'status':result['status'],'old_unit_state':state,'database_closed':True,'history_committed':True})

    def reopen(self):
        self.gate = self.target['hba'].read_bytes()
        self.runtime_password = secrets.token_hex(24)
        self.secret_file('runtime-password.txt', self.runtime_password)
        self.sql("SET password_encryption='scram-sha-256'; ALTER ROLE lmm_core_runtime LOGIN PASSWORD '"+self.runtime_password+"'; GRANT CONNECT ON DATABASE coretarget TO lmm_core_runtime; ALTER DATABASE coretarget ALLOW_CONNECTIONS true", db='postgres')
        self.target['hba'].write_bytes(b'local "coretarget" "postgres" trust\nlocal "coretarget" "lmm_core_runtime" scram-sha-256\n'+self.gate)
        self.sql('SELECT pg_reload_conf(); SELECT pg_sleep(0.15)', db='postgres')
        self.secret_file('runtime.url', self.url(self.target,'lmm_core_runtime','coretarget',self.runtime_password))
        require(self.scalar(self.password_digest_sql) == self.password_before, 'original_password_rows_changed')
        require(self.scalar("SELECT value FROM core_console.options WHERE key='Notice'") == 'Independent v6 predecessor Notice', 'original_Notice_lost')
        require(self.scalar('SELECT count(*) FROM core_meta.offline_upgrades') == '1', 'upgrade_receipt_count')
        rows = self.scalar(self.manifest['catalog']['query_utf8']).splitlines()
        h = hashlib.sha256()
        for row in rows:
            data = row.encode(); h.update(len(data).to_bytes(8,'big')); h.update(data)
        require(h.hexdigest() == CATALOG_SHA, 'upgraded_catalog_not_target')
        self.save('manual-fixture-opening.json', {'product_finalization':False,'actions':['runtime LOGIN + synthetic password','runtime CONNECT','private HBA runtime scram rule and local postgres oracle rule','ALLOW_CONNECTIONS true','pg_reload_conf'], 'metadata_grants_added':False,'password_rows_unchanged':True,'catalog_sha256':h.hexdigest()})

    def role_oracle(self):
        role = json.loads(self.scalar("SELECT json_build_object('session_user',session_user,'current_user',current_user,'super',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'replication',rolreplication,'bypassrls',rolbypassrls,'owner_member',pg_has_role(current_user,'lmm_core_owner','MEMBER'),'db_create',has_database_privilege(current_database(),'CREATE'),'schema_create',has_schema_privilege('core_meta','CREATE')) FROM pg_roles WHERE rolname=current_user", user='lmm_core_runtime'))
        require(role['session_user'] == role['current_user'] == 'lmm_core_runtime', 'unexpected_runtime_identity')
        require(not any(v for k,v in role.items() if k not in ('session_user','current_user')), 'runtime_has_excess_authority')
        self.save('actual-runtime-role.json', role)
        rights = []
        for table in PRIVATE:
            row = json.loads(self.scalar("SELECT json_build_object('table_any',has_table_privilege('core_meta."+table+"','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN'),'column_any',has_any_column_privilege('core_meta."+table+"','SELECT,INSERT,UPDATE,REFERENCES'))", user='lmm_core_runtime'))
            require(not any(row.values()), 'private_metadata_privilege_leak:'+table)
            rights.append({'table':table, **row})
        for table in PUBLIC:
            require(self.scalar("SELECT has_table_privilege('core_meta."+table+"','SELECT')", user='lmm_core_runtime') == 't', 'public_contract_unreadable:'+table)
        billing = json.loads(self.scalar("SELECT json_build_object('count',count(*),'all_select',bool_and(has_table_privilege(c.oid,'SELECT'))) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_billing' AND c.relkind IN ('r','p')", user='lmm_core_runtime'))
        require(billing['count'] > 2 and billing['all_select'], 'billing_read_requirement_not_satisfied')
        self.save('effective-ACL.json', {'private':rights,'public_contracts':PUBLIC,'billing':billing})

    def private_probes(self):
        self.role_oracle()
        evidence = []
        for table in PRIVATE:
            column = self.scalar("SELECT quote_ident(attname) FROM pg_attribute WHERE attrelid='core_meta."+table+"'::regclass AND attnum>0 AND NOT attisdropped ORDER BY attnum LIMIT 1")
            statements = [f'SELECT * FROM core_meta.{table} LIMIT 0', f'INSERT INTO core_meta.{table} DEFAULT VALUES',
                          f'UPDATE core_meta.{table} SET {column}={column} WHERE false', f'DELETE FROM core_meta.{table} WHERE false',
                          f'TRUNCATE core_meta.{table}', f'ALTER TABLE core_meta.{table} ADD COLUMN runtime_independent_probe integer']
            for statement in statements:
                p = self.sql('BEGIN; '+statement+'; ROLLBACK;', user='lmm_core_runtime', expect=None, label='private-access-denied')
                require(p.returncode != 0 and '42501' in p.stderr.decode(), 'private_access_not_permission_denied:'+statement)
                evidence.append({'statement':statement,'exit':p.returncode,'sqlstate':'42501'})
        p = self.sql('BEGIN; CREATE TABLE core_meta.runtime_independent_probe(x integer); ROLLBACK;', user='lmm_core_runtime', expect=None, label='schema-DDL-denied')
        require(p.returncode != 0 and b'42501' in p.stderr, 'runtime_schema_DDL_not_denied')
        evidence.append({'statement':'CREATE TABLE core_meta.runtime_independent_probe','exit':p.returncode,'sqlstate':'42501'})
        self.save('private-SQL-probes.json', evidence)

    def http(self, method, path, body=None, token=None):
        headers = {'Origin':f'http://127.0.0.1:{self.port}', 'Content-Type':'application/json'}
        if token:
            headers['Authorization'] = 'Bearer '+token
        conn = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
        try:
            conn.request(method,path,json.dumps(body) if body is not None else None,headers)
            res = conn.getresponse(); value = json.loads(res.read())
            safe = value if path.startswith(('/api/option','/health/','/core/v1/identity')) else {'success':value.get('success'),'data_keys':sorted(value.get('data',{}))}
            self.http_events.append({'method':method,'path':path,'status':res.status,'request':body if path.startswith('/api/option') else None,'response':safe})
            self.save('http.json',self.http_events)
            return res.status,value
        finally:
            conn.close()

    def wait_ready(self, process):
        deadline = time.monotonic()+25
        while time.monotonic()<deadline:
            require(process is None or process.poll() is None, 'actual_native_exited_before_ready')
            try:
                status, body = self.http('GET','/health/ready')
                if status == 200 and body.get('service') == 'lmm-core' and body.get('status') == 'ready':
                    return
            except OSError:
                pass
            time.sleep(.1)
        raise AssertionError('actual_native_readiness_timeout')

    def stop_process(self):
        if self.process is not None:
            if self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(12)
                except subprocess.TimeoutExpired:
                    self.process.kill(); self.process.wait(5)
                    raise AssertionError('native_forced_kill')
            self.process = None

    def native_http(self):
        with (self.root/'actual-runtime.log').open('wb') as log:
            self.process = subprocess.Popen([str(self.core),'--config',str(self.runtime_config),'serve'],stdout=log,stderr=subprocess.STDOUT,env=self.env)
            self.wait_ready(self.process)
            pid = self.process.pid
            require(sha(Path(f'/proc/{pid}/exe')) == CORE_SHA, 'running_native_hash_mismatch')
            self.save('actual-process.json', {'MainPID':pid,'exe_sha256':sha(Path(f'/proc/{pid}/exe')),'argv':[str(self.core),'--config',str(self.runtime_config),'serve'],'cgroup':Path(f'/proc/{pid}/cgroup').read_text()})
            status,auth = self.http('POST','/api/user/login',{'username':'runtimeowner','password':self.owner_password})
            require(status==200 and auth.get('success') is True, 'old_password_login_failed')
            token = auth['data']['access_token']
            status,who = self.http('GET','/core/v1/identity',token=token)
            require(status==200 and who.get('platform_level')==6 and who.get('credential_kind')=='session', 'old_owner_not_L6_session')
            value = 'Independent actual upgraded runtime Notice'
            status,result = self.http('PUT','/api/option/',{'key':'Notice','value':value},token)
            require(status==200 and result.get('success') is True and result.get('saved') is True,'Notice_not_saved')
            status,result = self.http('GET','/api/option/',token=token)
            require(status==200 and result.get('success') is True and {x['key']:x['value'] for x in result['data']}.get('Notice')==value,'Notice_readback_mismatch')
            require(self.scalar("SELECT value FROM core_console.options WHERE key='Notice'")==value,'Notice_SQL_mismatch')
            self.stop_process()

    def rejected_start(self, name, marker):
        p = self.run(name,[self.core,'--config',self.runtime_config,'serve'],expected=None,timeout=25)
        require(p.returncode==1 and marker.encode() in p.stderr+p.stdout,'native_did_not_reject_'+name)
        self.save(name+'.json', {'exit':p.returncode,'required_marker':marker,'core_sha256':sha(self.core)})

    def missing_public(self):
        self.sql('REVOKE SELECT ON core_meta.component_upgrades FROM lmm_core_runtime')
        try:
            require(self.scalar("SELECT has_table_privilege('core_meta.component_upgrades','SELECT')",user='lmm_core_runtime')=='f','public_revoke_ineffective')
            self.rejected_start('missing-public-contract','MissingPrivileges')
        finally:
            self.sql('GRANT SELECT ON core_meta.component_upgrades TO lmm_core_runtime')

    def missing_billing(self):
        table = self.scalar("SELECT quote_ident(c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_billing' AND c.relkind IN ('r','p') AND c.relname NOT IN ('balance_state','account_policies') ORDER BY c.relname LIMIT 1")
        require(bool(table),'billing_negative_fixture_empty')
        self.sql('REVOKE SELECT ON core_billing.'+table+' FROM lmm_core_runtime')
        try:
            require(self.scalar("SELECT has_table_privilege('core_billing."+table+"','SELECT')",user='lmm_core_runtime')=='f','billing_revoke_ineffective')
            self.rejected_start('missing-billing-read','MissingPrivileges')
        finally:
            self.sql('GRANT SELECT ON core_billing.'+table+' TO lmm_core_runtime')

    def misgrants(self):
        for table in PRIVATE:
            column = self.scalar("SELECT quote_ident(attname) FROM pg_attribute WHERE attrelid='core_meta."+table+"'::regclass AND attnum>0 AND NOT attisdropped ORDER BY attnum LIMIT 1")
            for mode,privilege in [('table','SELECT'),('column','SELECT ('+column+')')]:
                self.sql('GRANT '+privilege+' ON core_meta.'+table+' TO lmm_core_runtime')
                try:
                    self.rejected_start('private-'+table+'-'+mode,'UnsafePrivileges')
                finally:
                    self.sql('REVOKE '+privilege+' ON core_meta.'+table+' FROM lmm_core_runtime')
                self.role_oracle()

    def excess(self):
        self.sql('GRANT CREATE ON DATABASE coretarget TO lmm_core_runtime')
        try:
            self.rejected_start('excess-database-create','UnsafePrivileges')
        finally:
            self.sql('REVOKE CREATE ON DATABASE coretarget FROM lmm_core_runtime')
        self.role_oracle()

    def execute(self):
        for name,fn in [('v6_offline_upgrade',self.upgrade),('manual_fixture_reopen',self.reopen),('owner_only_sql',self.private_probes),
                        ('restricted_native_http',self.native_http),('missing_public_contract',self.missing_public),('missing_billing_read',self.missing_billing),
                        ('private_select_misgrants',self.misgrants),('excess_database_create',self.excess)]:
            self.current = name; print('START',name,flush=True)
            fn(); self.status[name]='PASS'; self.save('progress.json',self.status); print('PASS',name,flush=True)

    def close(self, passed):
        self.stop_process()
        if self.unit:
            self.run('old-unit-stop',['systemctl','--user','stop',self.unit],expected=None)
            self.run('old-unit-unmask',['systemctl','--user','unmask',self.unit],expected=None)
        if self.unitfile:
            self.unitfile.unlink(missing_ok=True)
            self.run('cleanup-daemon-reload',['systemctl','--user','daemon-reload'])
        if hasattr(self,'gate'):
            self.target['hba'].write_bytes(self.gate)
            self.sql('SELECT pg_reload_conf(); ALTER ROLE lmm_core_runtime NOLOGIN; REVOKE CONNECT ON DATABASE coretarget FROM lmm_core_runtime; ALTER DATABASE coretarget ALLOW_CONNECTIONS false',db='postgres')
            self.save('fixture-reclosed.json',{'closed':self.scalar("SELECT NOT datallowconn FROM pg_database WHERE datname='coretarget'",db='postgres')=='t','product_finalization':False})
        for c in reversed(self.clusters):
            if c['started']:
                self.run('pg-stop',['pg_ctl','-D',c['data'],'-m','fast','-w','stop']);c['started']=False
        if passed:
            for c in self.clusters:
                shutil.copy2(c['root']/'postgres.log',self.root/(c['root'].name+'-postgres.log'))
                shutil.rmtree(c['root'])
            for p in self.secret_files:
                p.unlink(missing_ok=True)
            shutil.rmtree(self.source)
            self.core.unlink()


def main():
    os.umask(0o077)
    p=argparse.ArgumentParser()
    for name in ['root','freeze','previous-core']:
        p.add_argument('--'+name,type=Path,required=True)
    args=p.parse_args()
    with open('/tmp/lmm-delegate/heavy-build.lock','a') as lock:
        deadline=time.monotonic()+1200
        while True:
            try:
                fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB);break
            except BlockingIOError:
                require(time.monotonic()<deadline,'shared_lock_timeout')
                print('WAIT shared heavy-build lock',flush=True);time.sleep(10)
        group=Path('/sys/fs/cgroup')/Path('/proc/self/cgroup').read_text().strip().split('::')[1].lstrip('/')
        aggregate=next(x for x in [group,*group.parents] if x.name==os.environ['RUNTIME_ACCEPTANCE_SLICE'])
        limits={n:(aggregate/n).read_text().strip() for n in ['memory.max','memory.high','memory.swap.max','cpu.max','pids.max']}
        available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))
        (args.root/'admission.json').write_text(json.dumps({'available_kib':available,'aggregate':str(aggregate),'limits':limits},indent=2)+'\n')
        require(available>=2097152,'2GiB_admission_closed')
        for key,value in [('memory.max',2**31),('memory.high',1536*1024**2),('memory.swap.max',4*1024**3),('pids.max',256)]:
            require(limits[key]!='max' and int(limits[key])<=value,'resource_limit_'+key)
        quota,period=limits['cpu.max'].split();require(quota!='max' and int(quota)<=int(period),'CPU1_limit_missing')
        f=Acceptance(args);passed=False
        try:
            f.execute();passed=True
        except Exception as error:
            f.status[f.current]='FAIL'
            f.save('failure.json',{'case':f.current,'type':type(error).__name__,'message':str(error)})
            print('FAIL',f.current,str(error),flush=True)
        finally:
            try:
                f.close(passed)
            except Exception as error:
                passed=False;f.save('cleanup-failure.json',{'type':type(error).__name__,'message':str(error)})
            f.save('results.json',{'overall':'PASS' if passed else 'INCOMPLETE','cases':f.status,'temporary_inputs_removed':passed,'scope':'independent v6 upgraded actual Native runtime only; fixture manual opening is not product finalization'})
            f.save('resource-final.json',{n:(aggregate/n).read_text().strip() for n in ['memory.peak','memory.events','memory.swap.peak','cpu.stat','pids.peak','pids.events'] if (aggregate/n).exists()})
        return 0 if passed else 1


if __name__=='__main__':
    raise SystemExit(main())
