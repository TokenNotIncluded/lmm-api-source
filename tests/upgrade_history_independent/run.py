#!/usr/bin/env python3
"""Independent frozen History API/CLI acceptance; private local PG18 only."""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

SOURCE = Path(__file__).resolve().parent
REPO = SOURCE.parents[1]
LIB_SHA = 'ea4a0d31f0d1568ce2f05ff35428a45ab69573f64fdf0ef2f1c8ac8214beccc9'
CORE_SHA = '6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31'


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--freeze-root', required=True, type=Path)
    parser.add_argument('--evidence-root', required=True, type=Path)
    parser.add_argument('--attempt', required=True)
    args = parser.parse_args()
    freeze, root = args.freeze_root.resolve(), args.evidence_root.resolve()
    if root.is_relative_to(REPO.parents[1]) or root.is_relative_to(freeze):
        parser.error('evidence root must be outside both repositories and the upstream freeze')
    if not re.fullmatch(r'attempt-[0-9]{2}', args.attempt):
        parser.error('attempt must be attempt-NN')
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    e = root / args.attempt
    e.mkdir(mode=0o700, exist_ok=False)
    os.chdir(e)
    records, started = [], set()
    success = False
    proxy = None
    binary = e / 'independent-history'
    env = {'PATH': '/usr/bin:/bin', 'HOME': str(Path.home()), 'LANG': 'C.UTF-8',
           'LC_ALL': 'C.UTF-8', 'TZ': 'UTC', 'TMPDIR': str(e / 'tmp'), 'PYTHONDONTWRITEBYTECODE': '1'}

    def save(name, value):
        (e / name).write_text(json.dumps(value, indent=2) + '\n')

    def run(name, argv, data=None, ok=True, timeout=600):
        record = {'name': name, 'argv': list(map(str, argv)), 'cwd': str(e), 'started': time.time()}
        records.append(record)
        save('commands.json', records)
        result = subprocess.run(record['argv'], cwd=e, env=env, input=data,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
        (e / (name + '.stdout.log')).write_bytes(result.stdout)
        (e / (name + '.stderr.log')).write_bytes(result.stderr)
        record.update(exit_code=result.returncode, finished=time.time(), expected_success=ok)
        save('commands.json', records)
        assert (result.returncode == 0) == ok, name + ' unexpected exit; preserved logs'
        return result.stdout.decode().strip()

    def socket(which):
        return '/proc/self/cwd/sock-' + which

    def port(which):
        return '55941' if which == 'authority' else '55942'

    def url(which, role, database):
        return f'postgresql://{role}@localhost/{database}?host={socket(which)}&port={port(which)}'

    sql_count = 0

    def sql(which, statement, database='postgres', role='ind_admin', ok=True):
        nonlocal sql_count
        sql_count += 1
        name = f'sql-{sql_count:03}'
        (e / (name + '.sql')).write_text(statement)
        output = run(name, ['psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=verbose',
                           '-h', socket(which), '-p', port(which), '-U', role, '-d', database],
                     statement.encode(), ok=ok)
        if not ok:
            assert '42501' in (e / (name + '.stderr.log')).read_text(), 'expected privilege denial: ' + name
        return output

    def pg_start(which, label):
        run(label, ['pg_ctl', '-D', e / ('pg-' + which), '-l', e / (which + '.postgres.log'), '-w', 'start'])
        started.add(which)

    def pg_stop(which, label):
        run(label, ['pg_ctl', '-D', e / ('pg-' + which), '-m', 'fast', '-w', 'stop'])
        started.remove(which)

    def cli(name, request, role='ind_writer', ok=True):
        save(name + '.request.json', request)
        output = run(name, [freeze / 'lmm-core', '--config', e / (role + '.toml'), 'upgrade-history'],
                     json.dumps(request).encode(), ok=ok)
        if ok:
            report = json.loads(output)
            assert report['protocol'] == 'core.upgrade-history.v1'
            assert report['retry_allowed'] is False and report['site_open_allowed'] is False
            return report['result']
        return None

    def expected(binding, sequence):
        return {'site_id': binding['site_id'], 'operation_id': binding['operation_id'], 'binding': binding,
                'sequence': sequence, 'phase': {1: 'closing', 2: 'held', 3: 'started', 4: 'committed'}[sequence],
                'fence_sha256': fixture['fence'] if sequence >= 2 else None,
                'receipt_sha256': fixture['receipt'] if sequence == 4 else None}

    def authority_rows():
        return json.loads(sql('authority', "SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY site_id),'[]') FROM lmm_upgrade_history.operations o", 'history_authority'))

    def verify_rows(label, normal, lost):
        want = sorted([expected(fixture['normal'], normal), expected(fixture['lost'], lost)], key=lambda x: x['site_id'])
        got = authority_rows()
        assert got == want, label + ' independent durable SQL oracle differs from original intent'
        save(label + '.rows.json', got)

    def inspect(label, binding, sequence):
        before = authority_rows()
        result = cli(label, {'action': 'inspect', 'authority_id': fixture['authority_id'], 'binding': binding})
        assert result == {'status': 'observed_history_only', 'operation': None if sequence is None else expected(binding, sequence)}
        assert authority_rows() == before, label + ' CLI inspect mutated history'

    def api(stage, label=None):
        save('fixture.json', fixture)
        output = json.loads(run(label or ('api-' + stage), [binary, stage, e / 'fixture.json']))
        assert output['status'] == 'PASS' and output['stage'] == stage
        return output

    save('result.json', {'status': 'RUNNING', 'owner': 'upgrade_history_independent_side_agent'})
    lock = open('/tmp/lmm-delegate/heavy-build.lock', 'a')
    fcntl.flock(lock, fcntl.LOCK_EX)
    try:
        available = int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))) * 1024
        cg = next(x.split(':', 2)[2].strip() for x in Path('/proc/self/cgroup').read_text().splitlines() if x.startswith('0::'))
        control = Path('/sys/fs/cgroup') / cg.lstrip('/')
        limits = {n: (control / n).read_text().strip() for n in ['memory.max', 'memory.high', 'memory.swap.max', 'cpu.max']}
        save('resources.json', {'memory_available': available, 'limits': limits, 'shared_lock': str(lock.name), 'prior_3GiB_failure': 'not modified or superseded'})
        assert available >= 2 * 1024**3, '2GiB admission unavailable'
        assert limits == {'memory.max': '2147483648', 'memory.high': '1610612736', 'memory.swap.max': '4294967296', 'cpu.max': '100000 100000'}
        assert sha(freeze / 'liblmm_core.rlib') == LIB_SHA
        assert sha(freeze / 'lmm-core') == CORE_SHA == json.loads((freeze / 'binaries.json').read_text())['lmm-core']['sha256']
        source_pins = json.loads((freeze / 'source-pins.json').read_text())
        cache_pins = json.loads((freeze / 'cache-pins.json').read_text())
        pins = {str(freeze / 'source' / path): value for path, value in source_pins.items()}
        pins.update(cache_pins)
        for path, value in pins.items():
            assert sha(path) == value, 'frozen input drift: ' + path
        for name in ['compile-config.json', 'binaries.json', 'freeze.json', 'source-pins.json', 'cache-pins.json']:
            shutil.copyfile(freeze / name, e / ('upstream-' + name))
        shutil.copytree(freeze / 'source', e / 'frozen-product')
        for name in ['run.py', 'independent.rs']:
            shutil.copyfile(SOURCE / name, e / name)
        helper_rel = 'tests/console/independent_process/pg_fault.py'
        helper = freeze / 'r2/source' / helper_rel
        helper_pin = json.loads((freeze / 'r2/source-pins.json').read_text())[helper_rel]
        assert sha(helper) == helper_pin
        shutil.copyfile(helper, e / 'frozen_pg_fault.py')
        save('binding.json', {'freeze_root': str(freeze), 'library_sha256': LIB_SHA, 'core_sha256': CORE_SHA,
                              'product_module_sha256': source_pins['services/core/src/upgrade_history.rs'],
                              'product_schema_sha256': source_pins['services/core/schema/upgrade_history.sql'],
                              'fault_helper_sha256': helper_pin, 'independent_rs_sha256': sha(e / 'independent.rs'),
                              'cache_pins': len(cache_pins), 'source_pins': len(source_pins), 'author_test_executed': False})
        (e / 'tmp').mkdir(mode=0o700)
        cfg = json.loads((freeze / 'compile-config.json').read_text())
        run('compile', [cfg['compiler'], '--edition=2024', e / 'independent.rs', *cfg['search'], *cfg['externs'],
                        '--extern', 'lmm_core=' + str(freeze / 'liblmm_core.rlib'), *cfg['link'], '-D', 'warnings', '-o', binary])
        save('executable.json', {'sha256': sha(binary), 'bytes': binary.stat().st_size})
        run('pg-version', ['postgres', '--version'])
        for which in ['authority', 'target']:
            data = e / ('pg-' + which)
            (e / ('sock-' + which)).mkdir(mode=0o700)
            run(which + '-init', ['initdb', '-D', data, '-U', 'ind_admin', '--auth-local=trust', '--auth-host=reject', '--encoding=UTF8', '--no-locale'])
            with (data / 'postgresql.conf').open('a') as f:
                f.write(f"\nlisten_addresses=''\nunix_socket_directories='../sock-{which}'\nport={port(which)}\nmax_connections=16\nshared_buffers='16MB'\nfsync=on\nfull_page_writes=on\nsynchronous_commit=on\n")
            pg_start(which, which + '-start')
        authority_id = sql('authority', 'SELECT system_identifier::text FROM pg_control_system()')
        target_id = sql('target', 'SELECT system_identifier::text FROM pg_control_system()')
        assert authority_id != target_id
        sql('authority', 'CREATE ROLE ind_owner NOLOGIN; CREATE ROLE ind_installer LOGIN NOINHERIT; CREATE ROLE ind_writer LOGIN NOINHERIT; GRANT ind_owner TO ind_installer WITH INHERIT FALSE, SET TRUE;')
        sql('authority', 'CREATE DATABASE history_authority OWNER ind_owner')
        sql('authority', 'REVOKE CONNECT ON DATABASE history_authority FROM PUBLIC; GRANT CONNECT ON DATABASE history_authority TO ind_installer,ind_writer; REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_control_system() TO ind_owner;', 'history_authority')
        sql('target', 'CREATE DATABASE business_target')
        sql('target', "CREATE TABLE account(id integer PRIMARY KEY,balance integer NOT NULL); INSERT INTO account VALUES(1,1000); CREATE TABLE applied_intent(operation_id text PRIMARY KEY,amount integer NOT NULL,request_hash text NOT NULL);", 'business_target')
        target_oid = int(sql('target', "SELECT oid FROM pg_database WHERE datname='business_target'"))
        normal = {'site_id': 'independent-site-normal-0001', 'operation_id': 'independent-operation-normal-0001',
                  'request_sha256': digest({'debit': 125, 'account': 1}), 'candidate_sha256': digest({'candidate': 'history-business-oracle-v1'}),
                  'target_system_identifier': target_id, 'target_database': 'business_target', 'target_database_oid': target_oid}
        lost = {**normal, 'site_id': 'independent-site-lostack-0002', 'operation_id': 'independent-operation-lostack-0002'}
        fixture = {'writer_url': url('authority', 'ind_writer', 'history_authority'), 'authority_id': digest({'fixture_authority': 'independent-history'}),
                   'authority_system_id': authority_id, 'normal': normal, 'lost': lost, 'fence': digest({'synthetic_closed_fence': True}), 'receipt': 'a' * 64, 'proxy_url': None}
        save('fixture.json', fixture)
        for role in ['ind_installer', 'ind_writer']:
            secret = e / (role + '.url')
            secret.write_text(url('authority', role, 'history_authority'))
            secret.chmod(0o600)
            (e / (role + '.toml')).write_text('version=1\n[database]\nurl_file=' + json.dumps(str(secret)) + '\n')
        install = {'action': 'install', 'authority_id': fixture['authority_id'], 'owner': 'ind_owner', 'writer': 'ind_writer'}
        assert cli('cli-install', install, 'ind_installer') == {'status': 'installed', 'authority_id': fixture['authority_id']}
        inspect('cli-absent', normal, None)
        assert authority_rows() == []
        pg_stop('target', 'target-checkpoint-stop')
        shutil.copytree(e / 'pg-target', e / 'target-before')
        pg_start('target', 'target-checkpoint-restart')

        api('reserve'); verify_rows('closing', 1, 1); inspect('cli-closing', normal, 1)
        api('hold'); verify_rows('held', 2, 2); inspect('cli-held', normal, 2)
        api('race'); verify_rows('consumed', 3, 2); inspect('cli-started', normal, 3)
        # Synthetic business effect follows the sole successful API consumer.
        # This is a test-owned SQL transaction, not product upgrade/apply.
        sql('target', f"BEGIN; INSERT INTO applied_intent VALUES('{normal['operation_id']}',125,'{normal['request_sha256']}'); UPDATE account SET balance=balance-125 WHERE id=1; COMMIT;", 'business_target')
        target_query = "SELECT jsonb_build_object('balance',(SELECT balance FROM account WHERE id=1),'marks',(SELECT coalesce(jsonb_agg(to_jsonb(a)),'[]') FROM applied_intent a))"
        effect = json.loads(sql('target', target_query, 'business_target'))
        assert effect == {'balance': 875, 'marks': [{'operation_id': normal['operation_id'], 'amount': 125, 'request_hash': normal['request_sha256']}]}
        save('actual-target-receipt.json', effect)
        fixture['receipt'] = digest(effect)
        api('commit'); verify_rows('committed', 4, 2); inspect('cli-committed', normal, 4)

        spec = importlib.util.spec_from_file_location('independent_frozen_fault', e / 'frozen_pg_fault.py')
        fault = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fault)
        proxy = fault.Proxy(socket('authority') + '/.s.PGSQL.55941', 'ind_writer', 'history_authority',
                            writer_sql=b'lmm_upgrade_history.advance', label='independent_history')
        proxy.arm()
        fixture['proxy_url'] = f'postgresql://ind_writer@127.0.0.1:{proxy.server_address[1]}/history_authority?sslmode=disable'
        api('fault')
        assert proxy.dropped.is_set() and not proxy.errors
        assert 'upstream_command_complete_commit_suppressed' in proxy.events
        assert 'upstream_ready_idle_suppressed_connection_closed' in proxy.events
        save('commit-ack-loss.json', {'events': proxy.events, 'errors': proxy.errors, 'actual_commit_ack_suppressed': True})
        proxy.close(); proxy = None
        verify_rows('ack-lost-but-consumed', 4, 3)
        inspect('cli-after-ack-loss', lost, 3)

        before_restore = authority_rows()
        pg_stop('target', 'target-restore-stop')
        (e / 'pg-target').rename(e / 'target-after-business')
        shutil.copytree(e / 'target-before', e / 'pg-target')
        pg_start('target', 'target-restored-start')
        assert sql('target', 'SELECT system_identifier::text FROM pg_control_system()') == target_id
        assert int(sql('target', "SELECT oid FROM pg_database WHERE datname='business_target'")) == target_oid
        restored = json.loads(sql('target', target_query, 'business_target'))
        assert restored == {'balance': 1000, 'marks': []}, 'physical restore must erase the actual business effect'
        assert authority_rows() == before_restore
        api('no_replay', 'api-after-target-restore')
        assert json.loads(sql('target', target_query, 'business_target')) == restored
        save('physical-restore-oracle.json', {'before_restore': effect, 'after_restore': restored,
                                           'same_target_system_and_oid': True, 'authority_unchanged': True, 'second_consumer_denied': True})
        inspect('cli-restored-committed', normal, 4); inspect('cli-restored-started', lost, 3)

        # Actual authority process stop/restart, with fresh API and CLI clients.
        pg_stop('authority', 'authority-os-stop')
        pg_start('authority', 'authority-os-start')
        assert authority_rows() == before_restore
        api('no_replay', 'api-after-authority-restart')
        inspect('cli-authority-restarted-committed', normal, 4)
        inspect('cli-authority-restarted-started', lost, 3)

        permission_before = authority_rows()
        for table in ['operations', 'installation']:
            for statement in [f'SELECT * FROM lmm_upgrade_history.{table}', f'INSERT INTO lmm_upgrade_history.{table} DEFAULT VALUES',
                              f'UPDATE lmm_upgrade_history.{table} SET {"sequence=sequence" if table == "operations" else "authority_id=authority_id"}',
                              f'DELETE FROM lmm_upgrade_history.{table}', f'TRUNCATE lmm_upgrade_history.{table}', f'DROP TABLE lmm_upgrade_history.{table}']:
                sql('authority', statement, 'history_authority', 'ind_writer', ok=False)
        for statement in ['SET ROLE ind_owner', 'CREATE TABLE lmm_upgrade_history.writer_escape(x int)', 'ALTER ROLE ind_writer SUPERUSER']:
            sql('authority', statement, 'history_authority', 'ind_writer', ok=False)
        safe = sql('authority', "SELECT NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls) AND NOT pg_has_role(current_user,'ind_owner','MEMBER') FROM pg_roles WHERE rolname=current_user", 'history_authority', 'ind_writer')
        assert safe == 't' and authority_rows() == permission_before
        save('writer-privileges.json', {'status': 'PASS', 'real_denials_sqlstate': '42501', 'denied_statements': 15, 'owner_membership': False})
        for action in ['open', 'retry']:
            cli('cli-reject-' + action, {'action': action, 'authority_id': fixture['authority_id'], 'binding': normal}, ok=False)
        cli('cli-reject-authority', {'action': 'inspect', 'authority_id': '0' * 64, 'binding': normal}, ok=False)
        cli('cli-reject-binding', {'action': 'inspect', 'authority_id': fixture['authority_id'], 'binding': {**normal, 'candidate_sha256': 'f' * 64}}, ok=False)
        cli('cli-reject-reinstall', install, 'ind_installer', ok=False)
        assert authority_rows() == permission_before
        inspect('cli-unrelated-absent', {**normal, 'site_id': 'independent-absent-site-0099'}, None)
        verify_rows('final', 4, 3)
        assert json.loads(sql('target', target_query, 'business_target')) == restored
        for path, value in pins.items():
            assert sha(path) == value, 'input changed during independent run: ' + path
        assert sha(freeze / 'lmm-core') == CORE_SHA and sha(freeze / 'liblmm_core.rlib') == LIB_SHA
        save('result.json', {'status': 'PASS', 'owner': 'upgrade_history_independent_side_agent',
                             'scope': 'frozen native History API and actual Core CLI only',
                             'original_binding': 'PASS', 'authority_and_physical_cluster_rejection': 'PASS',
                             'one_of_four_consumers': 'PASS', 'real_commit_ack_loss_reconnect': 'PASS',
                             'target_physical_restore_no_replay': 'PASS', 'authority_os_restart': 'PASS',
                             'writer_privileges': 'PASS', 'cli_observation_no_open_retry': 'PASS',
                             'author_test_executed': False, 'inputs_unchanged': True,
                             'complete_upgrade_fence_browser_S11_production': 'NOT_TESTED'})
        success = True
    except BaseException as error:
        save('result.json', {'status': 'FAIL', 'error': str(error), 'owner': 'upgrade_history_independent_side_agent',
                             'product_not_modified': True, 'failed_fixtures_retained': True})
        raise
    finally:
        if proxy is not None:
            save('commit-ack-loss-failed.json', {'events': proxy.events, 'errors': proxy.errors})
            proxy.close()
        for which in sorted(started.copy()):
            pg_stop(which, which + '-final-stop')
        if 'control' in locals():
            save('resource-final.json', {n: (control / n).read_text().strip() for n in ['memory.peak', 'memory.swap.peak', 'memory.events', 'cpu.stat']})
        removed = []
        paths = [binary, e / 'tmp']
        if success:
            paths += [e / n for n in ['pg-authority', 'pg-target', 'target-before', 'target-after-business', 'sock-authority', 'sock-target']]
            paths += [e / (role + suffix) for role in ['ind_installer', 'ind_writer'] for suffix in ['.url', '.toml']]
        for path in paths:
            if path.exists():
                size = sum(p.stat().st_size for p in path.rglob('*') if p.is_file()) if path.is_dir() else path.stat().st_size
                shutil.rmtree(path) if path.is_dir() else path.unlink()
                removed.append({'path': str(path), 'bytes': size})
        save('cleanup.json', {'removed': removed, 'failed_database_retained': not success,
                              'upstream_library_cache_source': 'untouched'})
        save('evidence-sha256.json', {str(p.relative_to(e)): sha(p) for p in e.rglob('*') if p.is_file()
                                     and not any(part in ['pg-authority', 'pg-target', 'target-before', 'target-after-business'] for part in p.parts)
                                     and p.name != 'evidence-sha256.json'})


if __name__ == '__main__':
    main()
