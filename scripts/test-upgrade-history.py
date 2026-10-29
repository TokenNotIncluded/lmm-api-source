#!/usr/bin/env python3
"""Two private PostgreSQL clusters; never resolves production configuration."""
import argparse
import fcntl
import json
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def restore_target(root):
    env = dict(os.environ)
    data, snapshot = root / 'target', root / 'target-before'
    subprocess.run(['pg_ctl', '-D', str(data), '-m', 'fast', '-w', 'stop'], check=True, capture_output=True, env=env)
    shutil.rmtree(data)
    shutil.copytree(snapshot, data)
    subprocess.run(['pg_ctl', '-D', str(data), '-l', str(root / 'target-restored.log'), '-w', 'start'], check=True, capture_output=True, env=env)


if len(sys.argv) == 2 and sys.argv[1] == 'restore-target':
    restore_target(Path(os.environ['HISTORY_PRIVATE']))
    raise SystemExit(0)

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--core', required=True, type=Path)
p.add_argument('--test', required=True, type=Path)
p.add_argument('--evidence', required=True, type=Path)
args = p.parse_args()
repo=Path(__file__).resolve().parents[1]
if args.evidence.resolve().is_relative_to(repo):
    p.error('--evidence must be outside the source checkout')
args.evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
lock = open('/tmp/lmm-delegate/heavy-build.lock', 'a')
fcntl.flock(lock, fcntl.LOCK_EX)
available = int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))) * 1024
if available < 2 * 1024**3:
    (args.evidence / 'result.json').write_text(json.dumps({'status': 'resource_blocked', 'memory_available': available}) + '\n')
    raise SystemExit(75)
root = Path(tempfile.mkdtemp(prefix='fixture-', dir=args.evidence))
sock = Path(tempfile.mkdtemp(prefix='lmm-hist-', dir='/tmp'))
environment = {k: v for k, v in os.environ.items() if not k.startswith(('PG', 'LMM_', 'HISTORY_')) and k not in ['DATABASE_URL','SQL_DSN']}
environment['LC_ALL'] = 'C.UTF-8'
commands = []


def run(argv, name, env=None, body=None):
    result = subprocess.run([str(x) for x in argv], env=env or environment, capture_output=True, timeout=180, input=body)
    (args.evidence / (name + '.log')).write_bytes(result.stdout + result.stderr)
    commands.append({'name': name, 'argv': [str(x) for x in argv], 'exit_code': result.returncode})
    (args.evidence / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
    if result.returncode:
        raise RuntimeError(name + ' failed; original evidence retained')
    return result.stdout.decode().strip()


def sql(which, statement, name):
    return run(['psql', '-X', '-q', '-t', '-A', '-v', 'ON_ERROR_STOP=1', '-h', sock / which,
                '-p', '55831' if which == 'authority' else '55832', '-U', 'postgres', '-d', 'postgres', '-c', statement], name)


started = []
proxy = None
try:
    for name, port in [('authority', 55831), ('target', 55832)]:
        data = root / name
        socket = sock / name
        socket.mkdir(mode=0o700)
        run(['initdb', '-D', data, '-U', 'postgres', '--auth=trust', '--encoding=UTF8', '--no-locale'], name + '-init')
        with (data / 'postgresql.conf').open('a') as f:
            f.write(f"\nlisten_addresses=''\nunix_socket_directories='{socket}'\nport={port}\nmax_connections=16\nshared_buffers='16MB'\nfsync=on\nfull_page_writes=on\nsynchronous_commit=on\n")
        run(['pg_ctl', '-D', data, '-l', root / (name + '.log'), '-w', 'start'], name + '-start')
        started.append(data)
    auth_id = sql('authority', 'SELECT system_identifier FROM pg_control_system()', 'authority-identity')
    target_id = sql('target', 'SELECT system_identifier FROM pg_control_system()', 'target-identity')
    assert auth_id != target_id
    sql('authority', 'CREATE ROLE hist_owner NOLOGIN; CREATE ROLE hist_installer LOGIN NOINHERIT; CREATE ROLE hist_writer LOGIN NOINHERIT; CREATE ROLE hist_runtime LOGIN; GRANT hist_owner TO hist_installer WITH INHERIT FALSE, SET TRUE; GRANT EXECUTE ON FUNCTION pg_control_system() TO hist_owner', 'roles')
    sql('authority', 'CREATE DATABASE authority OWNER hist_owner', 'database')
    sql('target', 'CREATE TABLE target_receipt(id integer PRIMARY KEY)', 'target-seed')
    run(['pg_ctl', '-D', root / 'target', '-m', 'fast', '-w', 'stop'], 'target-stop-for-copy')
    shutil.copytree(root / 'target', root / 'target-before')
    run(['pg_ctl', '-D', root / 'target', '-l', root / 'target.log', '-w', 'start'], 'target-restart-for-test')
    module_path=Path(__file__).resolve().parents[1]/'tests/console/independent_process/pg_fault.py'
    spec=importlib.util.spec_from_file_location('upgrade_history_pg_fault',module_path)
    fault=importlib.util.module_from_spec(spec);spec.loader.exec_module(fault)
    proxy=fault.Proxy(sock/'authority'/'.s.PGSQL.55831','hist_writer','authority',writer_sql=b'lmm_upgrade_history.advance',label='history')
    proxy.arm()
    environment['HISTORY_PROXY_PORT']=str(proxy.server_address[1])
    environment.update(HISTORY_PRIVATE=str(root), HISTORY_SOCKET=str(sock / 'authority'), HISTORY_TARGET_SOCKET=str(sock / 'target'), HISTORY_CORE_BIN=str(args.core.resolve()), HISTORY_TARGET_SYSTEM_ID=target_id,HISTORY_AUTHORITY_SYSTEM_ID=auth_id,HISTORY_RESTORE_SCRIPT=str(Path(__file__).resolve()))
    run([args.test.resolve(), '--ignored', '--exact', 'external_history_survives_target_restore_and_consumes_once', '--nocapture'], 'actual-history-test', environment)
    assert proxy.dropped.is_set() and not proxy.errors, 'real history commit acknowledgement fault not observed'
    (args.evidence/'commit-fault.json').write_text(json.dumps({'events':proxy.events,'errors':proxy.errors,'actual_commit_acknowledgement_dropped':True},indent=2)+'\n')
    proxy.close();proxy=None
    run(['pg_ctl','-D',root/'authority','-m','fast','-w','stop'],'authority-restart-stop')
    run(['pg_ctl','-D',root/'authority','-l',root/'authority.log','-w','start'],'authority-restart-start')
    (root/'authority-url').write_text(f'postgresql://hist_writer@localhost/authority?host={sock / "authority"}&port=55831')
    for site,operation,phase in [('isolated-site-0001','original-upgrade-0001','committed'),('lost-ack-site-0001','lost-ack-operation-0001','started')]:
        binding={'site_id':site,'operation_id':operation,'request_sha256':'a'*64,'candidate_sha256':'b'*64,'target_system_identifier':target_id,'target_database':'postgres','target_database_oid':5}
        inspect={'action':'inspect','authority_id':'1'*64,'binding':binding}
        report=json.loads(run([args.core.resolve(),'--config',root/'owner.toml','upgrade-history'],'post-restart-'+phase,body=json.dumps(inspect).encode()))
        assert report['result']['operation']['phase']==phase and report['retry_allowed'] is False and report['site_open_allowed'] is False
    # Both processes stopped cleanly before their successful fixture is removed.
    for data in reversed(started):
        run(['pg_ctl', '-D', data, '-m', 'fast', '-w', 'stop'], data.name + '-final-stop')
    started.clear()
    for path in root.glob('*.log'):
        shutil.copy2(path, args.evidence / ('fixture-' + path.name))
    shutil.rmtree(root)
    shutil.rmtree(sock)
    (args.evidence / 'result.json').write_text(json.dumps({'status':'passed','physical_target_restored':True,'authority_separate_cluster':True,'fixture_removed':True,'production_or_upgrade_apply_acceptance':False},indent=2)+'\n')
except BaseException as error:
    if proxy is not None:
        proxy.close()
    for data in reversed(started):
        subprocess.run(['pg_ctl','-D',str(data),'-m','fast','-w','stop'],env=environment,capture_output=True)
    (args.evidence / 'result.json').write_text(json.dumps({'status':'failed','error':type(error).__name__,'fixture_retained':str(root),'socket_retained':str(sock)},indent=2)+'\n')
    raise
