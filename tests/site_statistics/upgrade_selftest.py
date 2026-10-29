#!/usr/bin/env python3
"""Actual Core 27 -> 28 upgrade-db selftest on disposable local PostgreSQL 18."""

import argparse
import copy
import fcntl
import hashlib
import http.client
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
OLD_FREEZE = Path('/home/lightjunction/Documents/Codex/2026-10-30-native-performance-ytm_vzye/compile-r2')
OLD_CORE_SHA = 'd6d356311de8cf6589c2ec4429603a0d9308ca634b1aed20b78a6c42b7c76c10'
PRE_MANIFEST = '47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078'
PRE_CATALOG = '557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d'


def require(value, message):
    if not value:
        raise AssertionError(message)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def framed(lines):
    digest = hashlib.sha256()
    for line in lines:
        value = line.encode()
        digest.update(len(value).to_bytes(8, 'big'))
        digest.update(value)
    return digest.hexdigest()


def load_fixture():
    path = ROOT / 'tests/upgrade_offline_independent/fixture.py'
    spec = importlib.util.spec_from_file_location('upgrade_fixture', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.Fixture


class Test:
    def __init__(self, args):
        self.args = args
        self.root = args.output.resolve()
        self.fixture = load_fixture()(args)
        self.f = self.fixture
        self.process = None
        self.ack_socket_dir = None
        self.password = 'Fixture!' + secrets.token_hex(16)
        self.f.file('synthetic-owner-password.txt', self.password)
        self.post_manifest = args.target_manifest
        self.post_catalog = args.target_catalog
        self.post_definitions = args.target_definitions
        self.progress = {}

    def mark(self, name, detail=None):
        self.progress[name] = detail if detail is not None else 'PASS'
        self.f.save('progress.json', self.progress)
        print('PASS', name, flush=True)

    def sql(self, query, db='coretarget', user='postgres', owner=False):
        return self.f.sql(self.target, query, db, user, owner)

    def psql_file(self, name, db, path, flags=()):
        return self.f.run(name, ['psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1',
            '-h', self.target['socket'], '-p', self.target['port'], '-U', 'postgres',
            '-d', db, *flags, '-f', path])

    def manifest(self, native, expected):
        data = self.f.run('schema-manifest', [native, 'schema-manifest']).stdout
        require(hashlib.sha256(data).hexdigest() == expected, 'manifest SHA differs')
        return json.loads(data)

    def catalog(self, db, manifest, expected, count=None):
        rows = self.sql(manifest['catalog']['query_utf8'], db).splitlines()
        digest = framed(rows)
        require(digest == expected, f'{db} catalog SHA {digest}, expected {expected}, rows {len(rows)}')
        if count is not None:
            require(len(rows) == count, f'{db} catalog count {len(rows)}, expected {count}')
        return rows

    def bind(self):
        freeze = self.args.freeze.resolve(strict=True)
        require(json.loads((freeze / 'result.json').read_text())['status'] == 'passed', 'candidate build not passed')
        require(sha(self.args.previous_core) == OLD_CORE_SHA, 'old native SHA mismatch')
        binary = freeze / 'lmm-core'
        binaries = json.loads((freeze / 'binaries.json').read_text())
        require(sha(binary) == binaries['lmm-core']['sha256'], 'candidate native SHA mismatch')
        require(self.args.core.resolve() == binary.resolve(), 'candidate not frozen native')
        pins = json.loads((freeze / 'source-pins.json').read_text())
        for name, expected in pins.items():
            path = (freeze / 'source' / name).resolve(strict=True)
            require(path.is_relative_to((freeze / 'source').resolve()), 'source pin escapes freeze')
            require(sha(path) == expected, 'frozen source drift: ' + name)
        old_pins = json.loads((OLD_FREEZE / 'source-pins.json').read_text())
        old_grant = 'deployment/postgres/grant-core-runtime.sql'
        old_roles = 'deployment/postgres/create-core-roles.sql'
        for name in (old_grant, old_roles):
            require(sha(OLD_FREEZE / 'source' / name) == old_pins[name], 'old fixture pin: ' + name)
        target = json.loads((freeze / 'source/services/core/src/core_schema/upgrade/finance_target.json').read_text())
        require(target['manifest_sha256'] == self.post_manifest and target['catalog_sha256'] == self.post_catalog,
                'compiled target mismatch')
        require(len(target['definitions']) == self.post_definitions, 'target definition count')
        self.old_manifest = self.manifest(self.args.previous_core, PRE_MANIFEST)
        self.new_manifest = self.manifest(self.args.core, self.post_manifest)
        self.target_definitions = target['definitions']
        self.f.save('binding.json', {'old_native_sha256': OLD_CORE_SHA,
            'new_native_sha256': sha(binary), 'new_source_pins_sha256': sha(freeze / 'source-pins.json'),
            'new_source_count': len(pins), 'fixture_sha256': sha(ROOT / 'tests/upgrade_offline_independent/fixture.py'),
            'selftest_sha256': sha(Path(__file__)), 'old_roles_sha256': old_pins[old_roles],
            'old_grant_sha256': old_pins[old_grant], 'target_catalog': self.post_catalog,
            'scope': 'synthetic PG18 fixed DDL CLI; no production opening'})
        self.mark('native_and_source_binding')

    def setup(self):
        self.target = self.f.cluster('target')
        self.sql('CREATE DATABASE coretarget', 'postgres')
        self.psql_file('old-canonical-roles', 'coretarget',
                       OLD_FREEZE / 'source/deployment/postgres/create-core-roles.sql')
        self.f.installer_password = secrets.token_hex(24)
        self.sql("ALTER DATABASE coretarget OWNER TO lmm_core_owner; "
                 "CREATE ROLE lmm_core_installer LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; "
                 "GRANT lmm_core_owner TO lmm_core_installer WITH INHERIT FALSE, SET TRUE; "
                 "CREATE ROLE lmm_extensions LOGIN; "
                 "REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; "
                 "GRANT EXECUTE ON FUNCTION pg_control_system() TO lmm_core_owner")
        self.sql('GRANT CONNECT ON DATABASE coretarget TO lmm_core_installer', 'postgres')
        owner = self.f.config('owner-init', self.f.url(self.target, 'lmm_core_installer', 'coretarget',
                              options='&options=-c%20role%3Dlmm_core_owner'))
        self.f.run('old-init', [self.args.previous_core, '--config', owner, 'init-db'])
        created = json.loads(self.f.run('old-first-owner', [self.args.previous_core, '--config', owner,
            'create-first-owner'], json.dumps({'username':'financeowner','display_name':'Synthetic finance owner',
                                               'password':self.password}).encode()).stdout)
        require(created.get('setup_completed') is True, 'old first owner not created')
        self.psql_file('old-canonical-runtime-grants', 'coretarget',
            OLD_FREEZE / 'source/deployment/postgres/grant-core-runtime.sql',
            ['-v', 'enable_console=true', '-v', 'enable_web_identity=true'])
        self.sql("UPDATE core_identity.user_profiles SET email='finance-fixture@example.invalid',email_login_allowed=true; "
                 "INSERT INTO core_console.options(key,value) VALUES('Notice','finance-upgrade-synthetic') "
                 "ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        self.sql('REVOKE CONNECT ON DATABASE coretarget FROM PUBLIC,lmm_core_runtime; '
                 'ALTER ROLE lmm_core_runtime NOLOGIN', 'postgres')
        self.operator = self.f.config('operator', self.f.url(self.target, 'lmm_core_installer', 'coretarget'))
        ident = json.loads(self.sql("SELECT json_build_object('name',current_database(),"
            "'oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),"
            "'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),"
            "'server_version_num',current_setting('server_version_num')::int)"))
        self.request = {'version':1, 'procedure':'core-site-finance-v1',
            'upgrade_id':'finance-selftest-' + secrets.token_hex(12), 'database':ident,
            'roles':{'installer':'lmm_core_installer','owner':'lmm_core_owner',
                     'runtime':'lmm_core_runtime','extensions':'lmm_extensions'},
            'predecessor_manifest_sha256':PRE_MANIFEST, 'predecessor_catalog_sha256':PRE_CATALOG,
            'successor_manifest_sha256':self.post_manifest}
        require(self.sql("SELECT count(*) FROM core_meta.component_contract") == '27', 'old components')
        self.old_catalog = self.catalog('coretarget', self.old_manifest, PRE_CATALOG)
        require(self.counts('coretarget') == (171, 22), 'old table/sequence counts')
        self.before_password = self.password_rows('coretarget')
        predecessor = {'catalog':PRE_CATALOG,'definitions':len(self.old_catalog),
                       'tables':171,'sequences':22,'password_rows_sha256':self.before_password}
        if not self.args.target_only:
            self.before_data = self.data('coretarget')
            self.before_acl = self.acls('coretarget')
            self.before_components = self.sql("SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY name),'[]')::text FROM core_meta.component_contract c")
            self.before_runtime_contract = self.sql("SELECT (to_jsonb(r)-'catalog_fingerprint')::text FROM core_meta.runtime_contract r WHERE singleton")
            predecessor.update(data_sha256=hashlib.sha256(self.before_data.encode()).hexdigest(),
                               acl_fact_count=len(self.before_acl))
        self.f.save('predecessor.json', predecessor)
        self.mark('old_native_27_predecessor')

    def counts(self, db):
        result = self.sql("SELECT count(*) FILTER(WHERE c.relkind IN ('r','p')),"
            "count(*) FILTER(WHERE c.relkind='S') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace "
            "WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'", db)
        return tuple(map(int, result.split('|')))

    def data(self, db):
        tables = self.sql("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n "
            "ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') "
            "AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') ORDER BY format('%I.%I',n.nspname,c.relname) COLLATE \"C\"", db).splitlines()
        require(len(tables) == 171, 'old table set changed')
        sequences = self.sql("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n "
            "ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') "
            "AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY format('%I.%I',n.nspname,c.relname) COLLATE \"C\"", db).splitlines()
        require(len(sequences) == 22, 'old sequence set changed')
        return self.data_by_names(db, tables, sequences)

    def data_by_names(self, db, tables, sequences):
        commands = []
        for name in tables:
            if name in ('core_meta.component_contract', 'core_meta.runtime_contract'):
                continue
            literal = name.replace("'", "''")
            commands.append(f"SELECT json_build_array('table','{literal}',coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb))::text FROM {name} t;")
        for name in sequences:
            literal = name.replace("'", "''")
            commands.append(f"SELECT json_build_array('sequence','{literal}',last_value,log_cnt,is_called)::text FROM {name};")
        return self.sql('\n'.join(commands), db)

    def old_data_after(self, db='coretarget'):
        tables = self.sql("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n "
            "ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') "
            "AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') "
            "AND NOT (n.nspname='core_meta' AND c.relname='site_finance_upgrades') "
            "AND NOT (n.nspname='core_history' AND c.relname IN ('finance_sources','finance_users','finance_topups')) ORDER BY format('%I.%I',n.nspname,c.relname) COLLATE \"C\"", db).splitlines()
        seq = self.sql("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n "
            "ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') "
            "AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY format('%I.%I',n.nspname,c.relname) COLLATE \"C\"", db).splitlines()
        require(len(tables) == 171 and len(seq) == 22, 'preserved object counts')
        return self.data_by_names(db, tables, seq)

    def acls(self, db):
        sql = """WITH ns AS (SELECT * FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema') AND left(nspname,3)<>'pg_'), facts AS (
          SELECT json_build_array('schema',nspname,nspowner,nspacl)::text f FROM ns
          UNION ALL SELECT json_build_array('relation',n.nspname,c.relname,c.relowner,c.relacl)::text FROM pg_class c JOIN ns n ON n.oid=c.relnamespace
          UNION ALL SELECT json_build_array('column',n.nspname,c.relname,a.attnum,a.attacl)::text FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN ns n ON n.oid=c.relnamespace WHERE a.attnum>0 AND NOT a.attisdropped
          UNION ALL SELECT json_build_array('function',n.nspname,p.oid::regprocedure::text,p.proowner,p.proacl)::text FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace
          UNION ALL SELECT json_build_array('type',n.nspname,t.typname,t.typowner,t.typacl)::text FROM pg_type t JOIN ns n ON n.oid=t.typnamespace
          UNION ALL SELECT json_build_array('default',defaclrole,defaclnamespace,defaclobjtype,defaclacl)::text FROM pg_default_acl
          UNION ALL SELECT json_build_array('database',datname,datdba,datacl,datallowconn,datconnlimit)::text FROM pg_database WHERE datname=current_database()
          UNION ALL SELECT json_build_array('role',r.oid,r.rolname,r.rolsuper,r.rolinherit,r.rolcreaterole,r.rolcreatedb,r.rolcanlogin,r.rolreplication,r.rolbypassrls,r.rolconnlimit,r.rolvaliduntil,r.rolconfig,encode(sha256(convert_to(a.rolpassword,'UTF8')),'hex'))::text FROM pg_roles r JOIN pg_authid a ON a.oid=r.oid
          UNION ALL SELECT json_build_array('member',roleid,member,grantor,admin_option,inherit_option,set_option)::text FROM pg_auth_members
        ) SELECT f FROM facts ORDER BY f COLLATE "C"
        """
        return set(self.sql(sql, db).splitlines())

    def password_rows(self, db):
        return self.sql("SELECT encode(sha256(convert_to(coalesce(jsonb_agg(to_jsonb(p) ORDER BY to_jsonb(p)::text),'[]')::text,'UTF8')),'hex') FROM core_identity.password_logins p", db)

    def cli(self, name, request, config=None, success=True, input_error=None):
        self.f.save(name + '.request.json', request)
        result = self.f.run(name, [self.args.core, '--config', config or self.operator, 'upgrade-db'],
                            json.dumps(request).encode(), expected=None)
        if input_error is not None:
            expected = f'Error: Custom {{ kind: InvalidInput, error: "{input_error}" }}'
            require(not success and result.returncode != 0 and not result.stdout
                    and result.stderr.decode().strip() == expected,
                    f'{name}: unexpected pre-dispatch rejection')
            return {'code': input_error}
        try:
            report = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise AssertionError(f'{name}: CLI exit {result.returncode} without JSON; '
                                 f'stderr={result.stderr.decode(errors="replace").strip()}') from error
        require((result.returncode == 0) == success, f'{name}: {report}')
        return report

    def negative_acl(self):
        self.sql('CREATE DATABASE negative TEMPLATE coretarget', 'postgres')
        self.sql('REVOKE CONNECT ON DATABASE negative FROM PUBLIC,lmm_core_runtime; '
                 'GRANT CONNECT ON DATABASE negative TO lmm_core_installer', 'postgres')
        self.sql('ALTER DEFAULT PRIVILEGES FOR ROLE lmm_core_owner IN SCHEMA core_meta GRANT SELECT ON TABLES TO lmm_core_runtime', 'negative')
        before_data = self.data('negative')
        before_acl = self.acls('negative')
        before_components = self.sql("SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY name),'[]')::text FROM core_meta.component_contract c", 'negative')
        before_runtime = self.sql("SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]')::text FROM core_meta.runtime_contract r", 'negative')
        before_catalog = self.catalog('negative', self.old_manifest, PRE_CATALOG)
        ident = json.loads(self.sql("SELECT json_build_object('name',current_database(),"
            "'oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),"
            "'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),"
            "'server_version_num',current_setting('server_version_num')::int)", 'negative'))
        request = copy.deepcopy(self.request)
        request['database'] = ident
        request['upgrade_id'] = 'negative-acl-' + secrets.token_hex(12)
        config = self.f.config('negative-operator', self.f.url(self.target, 'lmm_core_installer', 'negative'))
        report = self.cli('negative-default-acl', request, config, False)
        code = report.get('code','')
        allowed_early = {'UPGRADE_DB_OFFLINE_ROLE_CONTRACT_REQUIRED', 'UPGRADE_DB_OTHER_CLIENT_OR_PREPARED_TRANSACTION',
                         'UPGRADE_DB_FOREIGN_OWNED_OBJECT'}
        require(code == 'SITE_FINANCE_RUNTIME_PRIVILEGES_INVALID' or code in allowed_early,
                'unexpected negative rejection: ' + str(report))
        require(self.counts('negative') == (171,22), 'negative mutated object count')
        require(self.data('negative') == before_data, 'negative changed old data')
        require(self.acls('negative') == before_acl, 'negative changed ACL')
        require(self.sql("SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY name),'[]')::text FROM core_meta.component_contract c", 'negative') == before_components, 'negative changed component rows')
        require(self.sql("SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]')::text FROM core_meta.runtime_contract r", 'negative') == before_runtime, 'negative changed runtime contract')
        require(self.sql("SELECT to_regclass('core_meta.site_finance_upgrades') IS NULL", 'negative') == 't', 'negative receipt exists')
        require(self.catalog('negative', self.old_manifest, PRE_CATALOG) == before_catalog, 'negative catalog changed')
        self.mark('default_acl_rejection', {'code':code,'stage':'finance_privileges' if code.startswith('SITE_FINANCE_') else 'shared_begin'})

    def fresh(self):
        self.sql('CREATE DATABASE fresh28', 'postgres')
        self.sql('ALTER DATABASE fresh28 OWNER TO lmm_core_owner', 'postgres')
        self.sql('GRANT USAGE,CREATE ON SCHEMA public TO lmm_core_owner', 'fresh28')
        conf = self.f.config('fresh-init', self.f.url(self.target,'postgres','fresh28',options='&options=-c%20role%3Dlmm_core_owner'))
        self.f.run('fresh-28-init', [self.args.core,'--config',conf,'init-db'])
        self.fresh_definitions = self.catalog('fresh28', self.new_manifest, self.post_catalog, self.post_definitions)
        require(self.fresh_definitions == self.target_definitions, 'fresh differs from frozen definition list')
        self.mark('fresh_native_28_catalog')

    def prepare_ack(self):
        if self.args.target_only:
            return
        self.sql('CREATE DATABASE ackcase TEMPLATE coretarget', 'postgres')
        self.sql('ALTER DATABASE ackcase OWNER TO lmm_core_owner; '
                 'REVOKE CONNECT ON DATABASE ackcase FROM PUBLIC,lmm_core_runtime; '
                 'GRANT CONNECT ON DATABASE ackcase TO lmm_core_installer', 'postgres')
        self.ack_data = self.data('ackcase')
        self.ack_acl = self.acls('ackcase')
        self.catalog('ackcase', self.old_manifest, PRE_CATALOG)
        identity = json.loads(self.sql("SELECT json_build_object('name',current_database(),"
            "'oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),"
            "'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),"
            "'server_version_num',current_setting('server_version_num')::int)", 'ackcase'))
        self.ack_request = copy.deepcopy(self.request)
        self.ack_request['database'] = identity
        self.ack_request['upgrade_id'] = 'ack-loss-' + secrets.token_hex(12)
        self.mark('ack_predecessor')

    def ack_fenced(self, label):
        require(self.sql("SELECT NOT rolcanlogin AND NOT has_database_privilege('lmm_core_runtime','ackcase','CONNECT') FROM pg_roles WHERE rolname='lmm_core_runtime'", 'postgres') == 't', label + ': runtime fence lost')
        probe = self.f.run(label + '-runtime-connect', ['psql','-X','-q','-h',self.target['socket'],
            '-p',self.target['port'],'-U','lmm_core_runtime','-d','ackcase','-c','SELECT 1'], expected=None)
        require(probe.returncode != 0 and b'not permitted to log in' in probe.stderr,
                label + ': runtime connection was not rejected by role fence')

    def ack_loss(self):
        if self.args.target_only:
            return
        self.ack_fenced('ack-before')
        self.ack_socket_dir = Path(tempfile.mkdtemp(prefix='lmm-fin-ack-', dir='/tmp'))
        proxy = self.f.fault.Proxy(self.target['socket'] / f".s.PGSQL.{self.target['port']}",
            'lmm_core_installer','ackcase',writer_sql=b'INSERT INTO core_meta.site_finance_upgrades',
            label='finance-receipt',listen_socket=self.ack_socket_dir / '.s.PGSQL.55602')
        config = self.f.config('ack-proxy-operator',
            f'postgresql://lmm_core_installer@localhost/ackcase?host={self.ack_socket_dir}&port=55602&sslmode=disable')
        try:
            proxy.arm()
            report = self.cli('ack-loss-upgrade', self.ack_request, config, success=False)
        finally:
            proxy.close()
            self.f.save('ack-proxy-events.json', {'events':proxy.events,'errors':proxy.errors,
                                                   'dropped':proxy.dropped.is_set()})
        require(proxy.dropped.is_set() and not proxy.errors and
                'upstream_command_complete_commit_suppressed' in proxy.events and
                'upstream_ready_idle_suppressed_connection_closed' in proxy.events,
                'COMMIT acknowledgement loss not proven')
        require(report.get('status') == 'unknown' and report.get('code') == 'UPGRADE_DB_COMMIT_UNKNOWN_REUSE_ORIGINAL_ID'
                and report.get('upgrade_id') == self.ack_request['upgrade_id']
                and type(report.get('original_backend_pid')) is int and report['original_backend_pid'] > 0
                and report.get('recovery') == 'reuse_exact_original_request_and_id; observe_original_backend_and_receipt_under_session_fence',
                'unexpected COMMIT unknown report: ' + str(report))
        deadline = time.monotonic() + 15
        while self.sql(f"SELECT NOT EXISTS(SELECT FROM pg_stat_activity WHERE pid={report['original_backend_pid']})", 'postgres') != 't':
            require(time.monotonic() < deadline, 'original backend still active after proxy closed')
            time.sleep(.1)
        owner = lambda sql: self.f.sql(self.target, sql, 'ackcase', 'lmm_core_installer', owner=True)
        require(owner('SELECT current_user') == 'lmm_core_owner', 'independent owner connection not active')
        require(owner('SELECT count(*) FROM core_meta.site_finance_upgrades') == '1', 'committed receipt missing')
        require(json.loads(owner('SELECT request::text FROM core_meta.site_finance_upgrades')) == self.ack_request,
                'committed receipt request differs')
        require(owner("SELECT encode(successor_catalog_sha256,'hex') FROM core_meta.site_finance_upgrades") == self.post_catalog,
                'committed receipt catalog differs')
        rows = owner(self.new_manifest['catalog']['query_utf8']).splitlines()
        require(len(rows) == self.post_definitions and framed(rows) == self.post_catalog and rows == self.target_definitions,
                'committed target catalog differs')
        require(self.counts('ackcase') == (175,22), 'ack target object counts')
        require(self.old_data_after('ackcase') == self.ack_data, 'ack upgrade changed old data or sequences')
        require(self.ack_acl.issubset(self.acls('ackcase')), 'ack upgrade changed old ACL')
        self.ack_fenced('ack-after')
        direct = self.f.config('ack-direct-operator', self.f.url(self.target,'lmm_core_installer','ackcase'))
        replay = self.cli('ack-exact-replay', self.ack_request, direct)
        require(replay.get('status') == 'already_committed', 'ack recovery replay not already_committed')
        self.ack_fenced('ack-replay')
        require(owner('SELECT count(*) FROM core_meta.site_finance_upgrades') == '1', 'ack replay changed receipt count')
        self.mark('actual_commit_ack_loss', {'status':report['status'],'code':report['code'],
            'upstream_commit':'observed and withheld','owner_receipts':1,'catalog':self.post_catalog,
            'definitions':len(rows),'replay':replay['status'],'runtime_fenced':True})

    def apply(self):
        report = self.cli('upgrade-27-to-28', self.request)
        require(report.get('status') == 'committed' and report.get('successor_catalog_sha256') == self.post_catalog, 'upgrade report')
        require(self.counts('coretarget') == (175,22), 'upgraded table/sequence counts')
        require(self.password_rows('coretarget') == self.before_password, 'password rows changed')
        require(self.sql('SELECT count(*) FROM core_meta.component_contract') == '28', 'new component count')
        if not self.args.target_only:
            require(self.old_data_after() == self.before_data, 'old rows or sequence values changed')
            after_acl = self.acls('coretarget')
            require(self.before_acl.issubset(after_acl), 'old ACL facts changed')
            for category in ('role', 'member', 'schema', 'database', 'default'):
                before = {fact for fact in self.before_acl if json.loads(fact)[0] == category}
                after = {fact for fact in after_acl if json.loads(fact)[0] == category}
                require(before == after, f'{category} facts changed')
            require(self.sql("SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY name),'[]')::text FROM core_meta.component_contract c WHERE name<>'site-finance'") == self.before_components, 'old component rows changed')
            require(self.sql("SELECT (to_jsonb(r)-'catalog_fingerprint')::text FROM core_meta.runtime_contract r WHERE singleton") == self.before_runtime_contract, 'old runtime contract fields changed')
        require(self.sql('SELECT count(*) FROM core_meta.site_finance_upgrades') == '1', 'receipt count')
        require(self.sql("SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton") == self.post_catalog, 'stored catalog')
        require(self.sql("SELECT NOT has_table_privilege('lmm_core_runtime','core_meta.site_finance_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN') AND NOT has_any_column_privilege('lmm_core_runtime','core_meta.site_finance_upgrades','SELECT,INSERT,UPDATE,REFERENCES')") == 't', 'private receipt accessible')
        require(self.sql("SELECT bool_and(has_table_privilege('lmm_core_runtime','core_history.'||name,'SELECT') AND NOT has_table_privilege('lmm_core_runtime','core_history.'||name,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN') AND NOT has_any_column_privilege('lmm_core_runtime','core_history.'||name,'INSERT,UPDATE,REFERENCES')) FROM (VALUES('finance_sources'),('finance_users'),('finance_topups'),('finance_native_payments')) x(name)") == 't', 'finance runtime is not SELECT-only')
        actual = self.catalog('coretarget', self.new_manifest, self.post_catalog, self.post_definitions)
        require(actual == self.fresh_definitions == self.target_definitions, 'fresh/upgraded catalog differs')
        if self.args.target_only:
            self.mark('actual_target_upgrade', {'status':report['status'],'catalog':self.post_catalog,
                'definitions':self.post_definitions,'tables':175,'sequences':22,'receipts':1,
                'full_preservation_and_replay':'NOT_REPEATED; R4 evidence is separate'})
            return
        receipt_before = self.sql("SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]')::text FROM core_meta.site_finance_upgrades r")
        replay = self.cli('upgrade-idempotent-replay', self.request)
        require(replay.get('status') == 'already_committed', 'exact replay status')
        changed_id = copy.deepcopy(self.request); changed_id['upgrade_id'] += '-different'
        bad_id = self.cli('upgrade-different-id', changed_id, success=False)
        require(bad_id.get('code') == 'SITE_FINANCE_ORIGINAL_RECEIPT_ABSENT', 'different ID rejection')
        changed_target = copy.deepcopy(self.request); changed_target['successor_manifest_sha256'] = '0' * 64
        bad_target = self.cli('upgrade-different-target', changed_target, success=False,
                              input_error='SITE_FINANCE_UPGRADE_CONTRACT_INVALID')
        require(bad_target.get('code') == 'SITE_FINANCE_UPGRADE_CONTRACT_INVALID', 'wrong target rejection')
        require(self.sql('SELECT count(*) FROM core_meta.site_finance_upgrades') == '1', 'receipt changed on rejects')
        require(self.sql("SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]')::text FROM core_meta.site_finance_upgrades r") == receipt_before, 'receipt contents changed on replay or rejects')
        require(self.old_data_after() == self.before_data, 'replay or rejects changed old rows or sequence values')
        require(self.acls('coretarget') == after_acl, 'replay or rejects changed ACL or roles')
        require(self.catalog('coretarget', self.new_manifest, self.post_catalog, self.post_definitions) == actual, 'replay or rejects changed catalog')
        self.mark('actual_upgrade_preservation_and_replay', {'status':report['status'],
            'replay':replay['status'],'different_id':bad_id['code'],'wrong_target':bad_target['code'],
            'catalog':self.post_catalog,'definitions':self.post_definitions,'tables':175,'sequences':22,'receipts':1})

    def runtime(self):
        self.sql('ALTER ROLE lmm_core_runtime LOGIN; GRANT CONNECT ON DATABASE coretarget TO lmm_core_runtime', 'postgres')
        runtime_url = self.f.file('runtime.url', self.f.url(self.target,'lmm_core_runtime','coretarget'))
        key = self.root / 'refresh.key'
        key.write_bytes(secrets.token_bytes(32))
        key.chmod(0o600)
        with socket.socket() as s:
            s.bind(('127.0.0.1',0)); port = s.getsockname()[1]
        conf = self.f.file('runtime.toml', f'''version=1
[server]
listen="127.0.0.1:{port}"
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
origin="http://127.0.0.1:{port}"
refresh_key_file={json.dumps(str(key))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
''')
        result = json.loads(self.f.run('runtime-check-db',[self.args.core,'--config',conf,'check-db']).stdout)
        require(result.get('status') == 'validated', f'check-db: {result}')
        with (self.root / 'runtime.log').open('wb') as log:
            self.process = subprocess.Popen([str(self.args.core),'--config',str(conf),'serve'], env=self.f.env,
                                            stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 30
                ready = False
                while time.monotonic() < deadline:
                    require(self.process.poll() is None, 'runtime exited before ready')
                    try:
                        conn = http.client.HTTPConnection('127.0.0.1',port,timeout=2)
                        conn.request('GET','/health/ready'); response=conn.getresponse(); response.read(); conn.close()
                        if response.status == 200:
                            ready=True; break
                    except OSError:
                        pass
                    time.sleep(.1)
                require(ready, 'runtime not ready')
                conn = http.client.HTTPConnection('127.0.0.1',port,timeout=5)
                payload=json.dumps({'username':'financeowner','password':self.password})
                conn.request('POST','/api/user/login',payload,
                             {'Content-Type':'application/json','Origin':f'http://127.0.0.1:{port}'})
                response=conn.getresponse(); body=json.loads(response.read()); conn.close()
                self.f.save('runtime-login.json', {'http_status':response.status,
                    'success':body.get('success'), 'message':body.get('message')})
                require(response.status == 200 and body.get('success') is True, 'original owner password login failed')
            finally:
                self.process.terminate(); self.process.wait(timeout=15); self.process=None
        self.sql('ALTER VIEW core_history.finance_native_payments SET (security_barrier=false)')
        tampered = self.f.run('view-barrier-check-db',[self.args.core,'--config',conf,'check-db'],expected=None)
        rejected = json.loads(tampered.stdout)
        require(tampered.returncode != 0 and rejected.get('status') == 'rejected', 'tampered view accepted')
        self.sql('ALTER VIEW core_history.finance_native_payments SET (security_barrier=true)')
        restored = json.loads(self.f.run('view-restored-check-db',[self.args.core,'--config',conf,'check-db']).stdout)
        require(restored.get('status') == 'validated', 'restored view check failed')
        self.catalog('coretarget', self.new_manifest, self.post_catalog, self.post_definitions)
        self.mark('runtime_and_view_tamper', {'check_db':result.get('status'),'runtime_ready':True,
            'old_password_login':True,'view_tamper_rejected':rejected.get('code'),
            'fixture_opening':'runtime LOGIN and CONNECT; not product finalization'})

    def floor_guard(self):
        if not self.args.check_native_intent_floor:
            return
        self.sql('CREATE DATABASE floorcheck TEMPLATE coretarget', 'postgres')
        next_value = int(self.sql("SELECT nextval('core_payments.intents_id_seq')", 'floorcheck'))
        high_water = int(self.sql("SELECT pg_sequence_last_value('core_payments.intents_id_seq'::regclass)", 'floorcheck'))
        inserted = self.sql("""INSERT INTO core_history.finance_sources
          (source_key,contract,source_archive_sha256,mapping_sha256,tables_sha256,normalization_sha256,
           rows_sha256,observed_at,user_count,topup_count,credits_per_usd,complete,native_request_ids)
          VALUES (repeat('a',64),'core.site-finance-source.v1',repeat('b',64),repeat('c',64),repeat('d',64),
                  repeat('e',64),repeat('f',64),1,0,0,500000,true,
                  ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE "C"))
          RETURNING native_intent_floor""", 'floorcheck')
        require(int(inserted) == high_water and high_water >= next_value, 'generated intent floor differs from sequence high-water')
        require(self.sql("SELECT native_request_ids=ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE \"C\") FROM core_history.finance_sources", 'floorcheck') == 't', 'native request seal differs from the existing request set')
        self.sql("SELECT complete FROM core_history.finance_native_payments", 'floorcheck')
        require(self.sql("SELECT bool_and(complete) FROM core_history.finance_native_payments", 'floorcheck') == 't', 'clean sequence should be complete')
        self.sql('ALTER SEQUENCE core_payments.intents_id_seq INCREMENT BY 2', 'floorcheck')
        require(self.sql("SELECT bool_and(complete) FROM core_history.finance_native_payments", 'floorcheck') == 'f', 'sequence guard did not fail closed')
        self.sql('ALTER SEQUENCE core_payments.intents_id_seq INCREMENT BY 1', 'floorcheck')
        require(self.sql("SELECT bool_and(complete) FROM core_history.finance_native_payments", 'floorcheck') == 't', 'restored sequence should be complete')
        self.mark('native_intent_floor_fixture', {'database':'floorcheck synthetic clone',
            'source_material_columns':12,'native_request_ids':'explicit SQL snapshot','next_id':next_value,'floor':int(inserted),
            'sequence_guard':'increment 2 rejected; increment 1 restored'})

    def seal_http(self):
        if not self.args.check_native_seal_http:
            return
        observed = 1790000000
        for before_seal in (True, False):
            db = 'seal_pre' if before_seal else 'seal_post'
            self.sql(f'CREATE DATABASE {db} TEMPLATE coretarget', 'postgres')
            self.sql(f'REVOKE CONNECT ON DATABASE {db} FROM PUBLIC; '
                     f'GRANT CONNECT ON DATABASE {db} TO lmm_core_runtime', 'postgres')
            actor = json.loads(self.sql("SELECT json_build_object('user',u.id,'account',u.personal_account_id) "
                "FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id "
                "WHERE p.username='financeowner'", db))
            account, user = actor['account'], actor['user']
            self.sql(f'SET ROLE lmm_core_owner; SELECT core_billing.open_ledger_wallet({account})', db)

            def insert_intent(created, journal=None):
                return int(self.sql(f"""INSERT INTO core_payments.intents
                    (account_id,created_by_user_id,channel_id,provider,merchant,environment,currency,
                     amount_minor,credit_units,credits_per_minor,prepare_key,prepare_hash,
                     credit_journal_id,provider_transaction,created_at)
                    VALUES ({account},{user},1,'stripe','synthetic','test','USD',123,615000,5000,
                            'synthetic-seal-key-0001',decode(repeat('a',64),'hex'),
                            {journal if journal is not None else 'NULL'},
                            {"'pi_synthetic'" if journal is not None else 'NULL'},to_timestamp({created}))
                    RETURNING id""", db))

            def credit():
                request = {'scope':'synthetic-seal', 'operation_key':'synthetic-seal-credit',
                    'reason':'synthetic independent seal fixture',
                    'action':{'kind':'credit','account_id':account,'amount_units':615000,
                              'payment_reference':'synthetic-seal-payment'}}
                payload = json.dumps(request,separators=(',',':')).replace("'","''")
                result = json.loads(self.sql(f"SET ROLE lmm_core_owner; SELECT core_billing.post_ledger('{payload}'::jsonb)", db))
                require(result.get('status') == 'posted', 'synthetic credit journal rejected')
                return result['journal_id']

            intent = insert_intent(observed+86400) if before_seal else None
            floor = int(self.sql(f"""INSERT INTO core_history.finance_sources
                (source_key,contract,source_archive_sha256,mapping_sha256,tables_sha256,normalization_sha256,
                 rows_sha256,observed_at,user_count,topup_count,credits_per_usd,complete,native_request_ids)
                VALUES (repeat('a',64),'core.site-finance-source.v1',repeat('b',64),repeat('c',64),
                        repeat('d',64),repeat('e',64),repeat('f',64),{observed},0,0,500000,true,
                        ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE "C"))
                RETURNING native_intent_floor""", db))
            require(self.sql("SELECT native_request_ids=ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE \"C\") FROM core_history.finance_sources", db) == 't', f'{db}: request seal differs from existing requests')
            journal = credit()
            if before_seal:
                self.sql(f"UPDATE core_payments.intents SET credit_journal_id={journal},provider_transaction='pi_synthetic' WHERE id={intent}", db)
                require(intent <= floor, 'pre-seal ID exceeds captured floor')
            else:
                intent = insert_intent(observed, journal)
                require(intent > floor, 'post-seal ID not above captured floor')
            proof_hash = hashlib.sha256(f'{db}:{intent}:credit'.encode()).hexdigest()
            proof = {'event_id':'evt_'+proof_hash[:24], 'transaction':'pi_synthetic',
                     'kind':'Credit', 'amount_minor':123, 'refund_id':None, 'hold_receipt_id':None}
            proof_json = json.dumps(proof,separators=(',',':')).replace("'","''")
            self.sql(f"INSERT INTO core_payments.accepted_proofs(id,channel_id,event_id,intent_id,payload_hash,signed_payload,signature,verified_facts,receipt_wire) "
                     f"VALUES ('proof_{proof_hash}',1,'evt_{proof_hash[:24]}',{intent},decode('{proof_hash}','hex'),decode('00','hex'),decode('00','hex'),'{proof_json}'::jsonb,decode('00','hex'))", db)
            facts = json.loads(self.sql(f"SELECT json_build_object('id',id,'created_at',extract(epoch FROM created_at)::bigint,"
                f"'credited',credit_journal_id IS NOT NULL) FROM core_payments.intents WHERE id={intent}", db))
            require(facts == {'id':intent,'created_at':observed+86400 if before_seal else observed,
                              'credited':True}, 'sealed intent identity/time mismatch')
            with socket.socket() as sock:
                sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
            url=self.f.file(db+'.url',self.f.url(self.target,'lmm_core_runtime',db))
            key=self.root/(db+'.key'); key.write_bytes(secrets.token_bytes(32)); key.chmod(0o600)
            conf=self.f.file(db+'.toml',f'''version=1
[server]
listen="127.0.0.1:{port}"
worker_threads=1
drain_timeout_seconds=3
[database]
url_file={json.dumps(str(url))}
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
origin="http://127.0.0.1:{port}"
refresh_key_file={json.dumps(str(key))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
''')
            with (self.root/(db+'.runtime.log')).open('wb') as log:
                self.process=subprocess.Popen([str(self.args.core),'--config',str(conf),'serve'],env=self.f.env,
                                              stdout=log,stderr=subprocess.STDOUT)
                try:
                    deadline=time.monotonic()+30
                    ready=False
                    while time.monotonic()<deadline:
                        require(self.process.poll() is None, f'{db}: runtime exited')
                        try:
                            conn=http.client.HTTPConnection('127.0.0.1',port,timeout=2)
                            conn.request('GET','/health/ready');response=conn.getresponse();response.read();conn.close()
                            if response.status == 200:
                                ready=True;break
                        except OSError:
                            pass
                        time.sleep(.1)
                    require(ready, f'{db}: runtime not ready')
                    headers={'Content-Type':'application/json','Origin':f'http://127.0.0.1:{port}'}
                    conn=http.client.HTTPConnection('127.0.0.1',port,timeout=5)
                    conn.request('POST','/api/user/login',json.dumps({'username':'financeowner','password':self.password}),headers)
                    response=conn.getresponse();auth=json.loads(response.read());conn.close()
                    require(response.status == 200 and auth.get('success') is True, f'{db}: owner login failed')
                    token=auth['data']['access_token']
                    conn=http.client.HTTPConnection('127.0.0.1',port,timeout=5)
                    started=int(time.time())
                    conn.request('GET','/api/finance/site-statistics',headers={'Authorization':'Bearer '+token})
                    response=conn.getresponse();status=response.status;body=json.loads(response.read());conn.close()
                    ended=int(time.time())
                    self.f.save(db+'-http.json',{'status':status,'body':body,'intent':facts,'floor':floor,'observed_at':observed})
                    if before_seal:
                        require(status == 503 and body.get('success') is False
                                and body.get('code') == 'SITE_STATISTICS_SOURCE_UNAVAILABLE'
                                and 'data' not in body, 'pre-seal credited intent did not produce 503')
                    else:
                        require(status == 200 and body.get('success') is True, 'same-second post-seal intent unavailable')
                        value=body['data'];timestamp=value.pop('as_of',None)
                        expected={'credits_per_usd':500000,'total_used_credits':'0','total_balance_credits':'615000',
                            'recharge':{'currencies':[{'currency':'USD','gross_amount_micros':'1230000',
                            'refunded_amount_micros':'0','net_amount_micros':'1230000','orders':1}],
                            'virtual_units':[],'confirmed_orders':1,'unconfirmed_orders':0,'invalid_orders':0}}
                        require(type(timestamp) is int and started <= timestamp <= ended and value == expected,
                                'same-second aggregate differs from fixed oracle')
                finally:
                    self.process.terminate();self.process.wait(timeout=15);self.process=None
            self.mark(db, {'id':intent,'floor':floor,'created_at':facts['created_at'],
                           'observed_at':observed,'http':status})

    def close(self, passed):
        if self.process is not None:
            self.process.terminate(); self.process.wait(timeout=15); self.process=None
        for c in self.f.clusters:
            self.f.stop(c)
        if passed:
            for c in self.f.clusters:
                shutil.rmtree(c['data'])
                shutil.rmtree(c['socket'])
            if self.ack_socket_dir is not None:
                shutil.rmtree(self.ack_socket_dir)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--freeze',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--target-manifest',required=True)
    parser.add_argument('--target-catalog',required=True)
    parser.add_argument('--target-definitions',type=int,required=True)
    parser.add_argument('--check-native-intent-floor',action='store_true')
    parser.add_argument('--check-native-seal-http',action='store_true')
    parser.add_argument('--target-only',action='store_true',
                        help='skip full R4 preservation and replay; retain target ACL rejection')
    args=parser.parse_args()
    require(all(len(value) == 64 and all(c in '0123456789abcdef' for c in value)
                for value in (args.target_manifest,args.target_catalog)), 'invalid target SHA')
    require(args.target_definitions > 0, 'invalid target definition count')
    args.source=ROOT
    args.core=args.freeze/'lmm-core'
    args.previous_core=OLD_FREEZE/'lmm-core'
    lock=open('/tmp/lmm-delegate/heavy-build.lock','a')
    fcntl.flock(lock,fcntl.LOCK_EX)
    available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024
    cg=next(x.split(':',2)[2] for x in Path('/proc/self/cgroup').read_text().splitlines() if x.startswith('0::'))
    cgroup=Path('/sys/fs/cgroup')/cg.lstrip('/')
    limits={k:(cgroup/k).read_text().strip() for k in ('memory.max','memory.high','memory.swap.max','cpu.max')}
    require(available >= 2*1024**3, '2GiB memory admission failed')
    require(limits == {'memory.max':'2147483648','memory.high':'1610612736','memory.swap.max':'4294967296','cpu.max':'100000 100000'}, 'wrong resource limits')
    test=Test(args)
    test.f.save('resources.json',{'memory_available':available,'limits':limits,'lock':lock.name})
    passed=False
    try:
        cases = [test.bind,test.setup,test.negative_acl,test.prepare_ack]
        cases.extend((test.fresh,test.apply,test.ack_loss,test.floor_guard,test.runtime,test.seal_http))
        for fn in cases:
            print('START',fn.__name__,flush=True); fn()
        passed=True
    finally:
        test.close(passed)
        test.f.save('result.json',{'status':'PASS' if passed else 'FAILED','progress':test.progress,
                                  'commit_ack_loss':('PASS' if 'actual_commit_ack_loss' in test.progress else
                                      'INCOMPLETE' if (test.root / 'ack-proxy-events.json').exists() else 'NOT_TESTED'),
                                  'production_opening':'NOT_TESTED'})


if __name__ == '__main__':
    main()
