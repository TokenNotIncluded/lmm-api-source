#!/usr/bin/env python3
"""Historical full refund only: owner-reviewed synthetic source grants -> actual Core ledger.
No compilation, production URL, external database, native-pay simulation, partial or external refund.
Requires an explicitly allocated PG/process window. It never builds or pulls images.

Usage (all paths in inputs are absolute, operator-provided frozen artifacts):
  python3 tests/commerce/historical_refund_process.py --inputs INPUTS.json --output ~/.cache/NEW-ATTEMPT

INPUTS has core/go objects with binary, sha256, source_manifest, source_root,
and optional builder_receipt. schema_contract is {"path": FILE, "sha256": HASH}
for the builder's complete native schema-manifest JSON. No personal cache input
or recorded credentials are required by this runner. The complete recursive
runtime-grant SQL closure is read from core.source_root and saved privately.
--preflight runs native schema-manifest only; it does not start PG or HTTP hosts.
All outputs (including synthetic tokens, URLs and failure dumps) stay private.
A failed run is retained, never retried automatically. No production migration
or full financial_import admission is accepted by these synthetic owner grants.
"""
from __future__ import annotations
import argparse, http.client, json, os, re, secrets
import signal, socket, subprocess, sys, time, traceback
from pathlib import Path
from urllib.parse import quote
sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[2]
CACHE = Path.home() / '.cache'
sys.path.insert(0, str(REPO / 'tests/architecture_completion'))
from pg_fixture import Fixture
from verify_process import digest as sha, stop_group
from historical_refund_cases import LegacyRefundCases

class Blocked(RuntimeError): pass
class PreflightReady(Exception): pass

def need(value, message):
    if not value: raise AssertionError(message)

def write(path, value):
    raw = value if isinstance(value, str) else json.dumps(value, indent=2, sort_keys=True) + '\n'
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as f: f.write(raw)

def run(command, root, label, data=None, timeout=60):
    env = {'PATH':'/usr/bin:/bin', 'LANG':'C.UTF-8'}
    p = subprocess.run([str(v) for v in command], input=data, capture_output=True, env=env, timeout=timeout)
    write(root / (label+'.stdout'), p.stdout.decode(errors='replace'))
    write(root / (label+'.stderr'), p.stderr.decode(errors='replace'))
    write(root / (label+'.result.json'), {'command':[str(v) for v in command], 'exit_code':p.returncode})
    need(p.returncode == 0, f'{label}: exit {p.returncode}; retained private logs')
    return p.stdout.decode()

def checked_artifact(spec, label):
    if not re.fullmatch('[0-9a-f]{64}',spec.get('sha256','')): raise Blocked(f'{label}: frozen SHA256 not delivered')
    path = Path(spec['binary']); manifest = Path(spec['source_manifest'])
    if not path.is_absolute() or not path.is_file(): raise Blocked(f'{label}: frozen actual binary missing')
    if not manifest.is_file(): raise Blocked(f'{label}: builder source manifest missing')
    need(sha(path) == spec['sha256'], f'{label}: binary digest mismatch')
    source = json.loads(manifest.read_text()); files = source.get('files') or source.get('source_sha256')
    if files is None and source and all(isinstance(v,str) and re.fullmatch('[0-9a-f]{64}',v) for v in source.values()): files=source
    if not isinstance(files, dict): raise Blocked(f'{label}: source manifest requires files/path-SHA256 map')
    base = Path(spec.get('source_root', REPO)); drift=[]
    for name, expected in files.items():
        p = base / name
        if not p.is_file() or sha(p) != expected: drift.append(name)
    # A supplied builder receipt can bind an immutable earlier component binary.
    # Any live-source drift must be enumerated by root, never silently accepted.
    receipt=None
    if spec.get('builder_receipt'):
        receipt_path=Path(spec['builder_receipt']);receipt=json.loads(receipt_path.read_text())
        need(receipt.get('binary_sha256',receipt.get('artifact_sha256')) == spec['sha256'],label+': builder artifact digest mismatch')
        if 'source_manifest_sha256' in receipt:
            need(receipt['source_manifest_sha256']==sha(manifest),label+': builder manifest digest mismatch')
        if 'source_inputs_unchanged' in receipt:
            need(receipt['source_inputs_unchanged'] is True,label+': build source changed during compilation')
    allowed=set(spec.get('permitted_live_source_drift',[]))
    need(not allowed or receipt is not None,label+': source drift requires explicit frozen builder receipt')
    need(set(drift)<=allowed, f'{label}: fixed source drift: '+','.join(drift))
    return path, {'binary':str(path), 'sha256':spec['sha256'], 'source_manifest':str(manifest),
                  'source_manifest_sha256':sha(manifest), 'source_root':str(base), 'source_count':len(files),
                  'source_commit':source.get('source_commit',receipt.get('git_head') if receipt else None),
                  'builder_receipt_sha256':sha(Path(spec['builder_receipt'])) if receipt else None,
                  'live_source_drift':drift,'live_source_fully_matches':not drift}

def runtime_grants(source_root, out):
    """Freeze the installer's existing static relative SQL includes, unchanged."""
    source_root = source_root.resolve()
    files = {}

    def expand(path, ancestors=()):
        path = path.resolve()
        need(path.is_relative_to(source_root), 'runtime SQL include escaped frozen source')
        need(path not in ancestors, 'runtime SQL include cycle')
        files[str(path.relative_to(source_root))] = sha(path)
        rows = []
        for line in path.read_text().splitlines(keepends=True):
            include = re.fullmatch(r"\s*\\ir\s+([A-Za-z0-9_./-]+)\s*", line)
            if include:
                rows.append(expand(path.parent / include.group(1), (*ancestors, path)))
                rows.append('\n')
            else:
                need(not re.match(r'\s*\\(?:i|ir|include|include_relative)\b', line),
                     'unsupported runtime SQL include: review before execution')
                rows.append(line)
        return ''.join(rows)

    sql = expand(source_root / 'deployment/postgres/grant-core-runtime.sql')
    target = out / 'runtime-grants.expanded.sql'
    write(target, sql)
    write(out / 'sql-closure.json', {'scope': 'unchanged recursive installer includes',
                                   'files': files, 'expanded_sha256': sha(target)})
    return target

def identity(process, expected):
    path = Path(f'/proc/{process.pid}/exe'); st = path.stat()
    need(sha(path) == expected, 'actual /proc executable differs from frozen artifact')
    ticks = Path(f'/proc/{process.pid}/stat').read_text().rsplit(') ',1)[1].split()[19]
    return {'pid':process.pid,'start_ticks':ticks, 'exe_sha256':expected, 'device':st.st_dev,
            'inode':st.st_ino,'boot_id':Path('/proc/sys/kernel/random/boot_id').read_text().strip()}

def address():
    with socket.socket() as s:
        s.bind(('127.0.0.1',0)); return s.getsockname()[1]

def request(port, path, body=None, host=None, user=None, sent=None):
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=10)
    started = time.monotonic_ns()
    try:
        data = None if body is None else json.dumps(body, separators=(',',':')).encode()
        headers = {'Content-Type':'application/json'}
        if host: headers['Authorization'] = 'Bearer '+host
        if user: headers['X-LMM-User-Credential'] = user
        connection.request('GET' if body is None else 'POST', path, body=data, headers=headers)
        sent_ns = time.monotonic_ns()
        if sent: sent.set()
        response = connection.getresponse(); raw = response.read()
        return {'status':response.status,'body':json.loads(raw), 'started_ns':started,
                'sent_ns':sent_ns,'finished_ns':time.monotonic_ns()}
    finally: connection.close()

def spawn(binary, digest, config, root, label):
    log = (root/(label+'.log')).open('xb'); os.chmod(root/(label+'.log'),0o600)
    proc = subprocess.Popen([str(binary),'--config',str(config)], env={'PATH':'/usr/bin:/bin','GOMAXPROCS':'1','GOMEMLIMIT':'512MiB'},
                            stdin=subprocess.DEVNULL,stdout=log,stderr=log,start_new_session=True)
    log.close()
    try:
        write(root/(label+'.identity.spawn.json'),identity(proc,digest))
    except BaseException:
        os.killpg(proc.pid,signal.SIGKILL);proc.wait(timeout=3);raise
    return proc

def ready(proc, digest, port, root, label):
    deadline = time.monotonic()+20
    while time.monotonic()<deadline:
        need(proc.poll() is None, label+' exited before ready')
        try:
            r=request(port,'/health/ready')
            if r['status']==200:
                write(root/(label+'.ready.json'),r)
                write(root/(label+'.identity.ready.json'),identity(proc,digest)); return r['body']
        except (OSError,http.client.HTTPException): pass
        time.sleep(.05)
    raise AssertionError(label+' readiness timeout')

def main():
    os.setpriority(os.PRIO_PROCESS,0,19)
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs',type=Path,required=True,help='frozen core/go artifact+source manifest JSON')
    parser.add_argument('--output',type=Path,required=True,help='new private cache attempt directory')
    parser.add_argument('--preflight',action='store_true',help='validate artifacts/schema only; no database/host')
    args=parser.parse_args(); out=args.output
    need(out.is_absolute() and out.resolve().is_relative_to(CACHE),'output must be an absolute cache directory')
    out.mkdir(mode=0o700,exist_ok=False); write(out/'harness.source.py',Path(__file__).read_text()); result={'result':'INCOMPLETE','scope':'two historical balance paid grants; real SQL conservation, ACK and UNKNOWN recovery; synthetic source only',
       'full_project_complete':False,'debt_closed':False,'argv':sys.argv,'inputs':str(args.inputs)}
    processes=[]; fixture=None; locker=None; sockets=None
    try:
        inputs=json.loads(args.inputs.read_text()); write(out/'inputs.json',inputs)
        write(out/'historical_refund_cases.source.py',(Path(__file__).parent/'historical_refund_cases.py').read_text())
        core,core_binding=checked_artifact(inputs['core'],'Core'); go,go_binding=checked_artifact(inputs['go'],'Go')
        write(out/'artifacts.json',{'core':core_binding,'go':go_binding})
        # A libtestbin cannot satisfy this native executable subcommand.
        manifest=json.loads(run([core,'schema-manifest'],out,'core-schema-manifest'))
        need(len(manifest['components'])==65,'actual native Core must expose all 65 compiled components')
        frozen_core_root=Path(inputs['core'].get('source_root',REPO))
        schema = inputs['schema_contract']
        schema_path = Path(schema['path'])
        need(schema_path.is_absolute() and re.fullmatch('[0-9a-f]{64}', schema['sha256']),
             'frozen schema contract requires an absolute path and SHA256')
        need(sha(schema_path) == schema['sha256'], 'frozen schema contract digest mismatch')
        need(manifest == json.loads(schema_path.read_text()),
             'native compiled manifest differs from frozen full65 contract')
        frozen_go_root=Path(inputs['go'].get('source_root',REPO))
        grants=runtime_grants(frozen_core_root,out)
        store_schema=frozen_go_root/'services/extensions/internal/modules/store/schema.sql'
        need(grants.is_file() and store_schema.is_file(),'frozen native runtime grants and store schema are required')
        write(out/'support-inputs.json',{str(p):sha(p) for p in [
            grants, store_schema, Path(__file__), Path(__file__).parent/'historical_refund_cases.py',
            REPO/'tests/architecture_completion/pg_fixture.py',
            REPO/'tests/architecture_completion/build_lease.py',
            REPO/'tests/commerce/verify_process.py', REPO/'tests/commerce/resource_group.py',
            schema_path, out/'sql-closure.json']})
        if args.preflight:
            result.update(result='READY',schema_components=65,resources_started=False); raise PreflightReady()
        # Existing reviewed fixture: new owned local cluster, no image pull, no external DSN.
        fixture=Fixture(out);fixture.start();ca=fixture.allocate('A02');ga=fixture.allocate('A05')
        for allocation,prefix in [(ca,'lmm_core'),(ga,'lmm_store')]:
            old=allocation['database'];new=prefix+'_'+fixture.nonce
            fixture.sql(f'ALTER DATABASE {old} RENAME TO {new};')
            fixture.databases.remove(old);fixture.databases.add(new)
            allocation['database']=new;fixture.allocations[allocation['task']]['database']=new
            fixture.record('qd-renamed-'+allocation['task'],{'old_database':old,'database':new,'owner':allocation['owner'],'runtime':allocation['runtime']})
        cp=secrets.token_hex(32)
        fixture.sql(f"CREATE ROLE lmm_core_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{cp}';")
        fixture.sql(f'ALTER DATABASE {ca["database"]} OWNER TO lmm_core_owner;')
        fixture.sql('ALTER SCHEMA public OWNER TO lmm_core_owner;',ca['database'])
        def url(allocation,role,password):
            return f'postgresql://{role}:{quote(password,safe="")}@127.0.0.1:{allocation["port"]}/{allocation["database"]}?sslmode=disable'
        def pw(allocation):
            task=allocation['task'];return (fixture.root/task/'runtime.pgpass').read_text().strip().split(':')[-1]
        owner_url=out/'core-owner.url';write(owner_url,url(ca,'lmm_core_owner',cp))
        core_url=out/'core-runtime.url';write(core_url,url(ca,ca['runtime'],pw(ca)))
        store_url=out/'store-runtime.url';write(store_url,url(ga,ga['runtime'],pw(ga)))
        owner_config=out/'owner.toml';write(owner_config,f'version=1\n[database]\nurl_file="{owner_url}"\n')
        run([core,'--config',owner_config,'init-db'],out,'native-init')
        users={}
        for uid in [7,9,11,13]:
            issued=json.loads(run([core,'--config',owner_config,'bootstrap-user'],out,'bootstrap-'+str(uid),
                                 json.dumps({'user_id':uid,'platform_level':1}).encode()))
            users[uid]=issued['secret']
        accounts=json.loads(fixture.sql("SELECT jsonb_object_agg(id::text,personal_account_id)::text FROM core_identity.users WHERE id IN (7,9,11,13)",ca['database']))
        buyer,seller=int(accounts['7']),int(accounts['9'])
        fixture.sql('ALTER ROLE lmm_core_owner NOLOGIN;')
        fixture.sql('SET ROLE lmm_core_owner;\n\\set runtime_role '+ca['runtime']+'\n\\set enable_commerce true\n\\set enable_events true\n'+grants.read_text(),ca['database'])
        fixture.sql('SET ROLE '+ga['owner']+';\n'+store_schema.read_text()+f'\nRESET ROLE; GRANT USAGE ON SCHEMA lmm_store TO {ga["runtime"]}; GRANT SELECT,INSERT,UPDATE ON ALL TABLES IN SCHEMA lmm_store TO {ga["runtime"]}; GRANT DELETE ON lmm_store.traffic_days,lmm_store.traffic_receipts,lmm_store.cart_items,lmm_store.favorites TO {ga["runtime"]};',ga['database'])
        # Go receives no Core DSN or Core CONNECT grant.
        fixture.expect_connect_denied(ca['database'],ga['runtime'],pw(ga))
        sockets=CACHE/('qds-'+secrets.token_hex(4));sockets.mkdir(mode=0o700)
        host='synthetic-host-'+secrets.token_hex(24); hostfile=out/'host.token';write(hostfile,host)
        identityfile=out/'identity.token';write(identityfile,'synthetic-identity-'+secrets.token_hex(24))
        commercefile=out/'commerce.token';write(commercefile,'synthetic-commerce-'+secrets.token_hex(24))
        policy=out/'commerce.toml';write(policy,'version=1\npay=true\nsettle=true\nrefund=true\nread=true\ncomplete_delivery=true\nreconcile=true\n')
        core_port,go_port=address(),address(); sock=sockets/'core.sock'
        config=out/'core.toml';write(config,f'version=1\n[server]\nlisten="127.0.0.1:{core_port}"\nworker_threads=1\n[database]\nurl_file="{core_url}"\nmax_connections=4\n[rpc]\nsocket="{sock}"\ntoken_file="{identityfile}"\n[commerce]\nconfig_file="{policy}"\ntoken_file="{commercefile}"\n')
        gconfig=out/'go.toml';write(gconfig,f'version=1\n[host]\nlisten="127.0.0.1:{go_port}"\ntoken_file="{hostfile}"\nmodules=["identity","store"]\nweb_compat=true\n[core]\nsocket="{sock}"\nidentity_token_file="{identityfile}"\ncommerce_token_file="{commercefile}"\n[store]\ndatabase_name="{ga["database"]}"\ndatabase_url_file="{store_url}"\n')
        authority=json.loads(run([core,'--config',config,'check-db'],out,'native-check-db'))
        need(authority['status']=='validated' and authority['runtime_authority']['status']=='validated','configured native runtime authority invalid')
        capabilities=authority['runtime_authority']['required_capabilities']
        need(capabilities['commerce'] and capabilities['events'],'commerce/RPC events authority missing')
        need(authority['execution']['executable_sha256']==core_binding['sha256'],'native check-db binary identity mismatch')
        pc=spawn(core,core_binding['sha256'],config,out,'core');processes.append(pc)
        health=ready(pc,core_binding['sha256'],core_port,out,'core');need(health['capabilities']['commerce'] is True,'actual Core commerce disabled')
        pg=spawn(go,go_binding['sha256'],gconfig,out,'go');processes.append(pg);ready(pg,go_binding['sha256'],go_port,out,'go')
        cases=LegacyRefundCases(fixture,ca['database'],ga['database'],out,go_port,users,accounts,request,write,need,ca['runtime'])
        cases.seed();cases.run()
        result.update(result='PASS',schema_components=65,scope='two synthetic source-paid + two historical-refunded reads; actual Go/Core/PG money, ACK and UNKNOWN',production_migration_accepted=False,financial_import_admission_accepted=False)
    except PreflightReady:
        pass
    except Blocked as e:
        result.update(result='WAITING_INPUT',error=str(e))
    except BaseException as e:
        result.update(result='FAIL',error_type=type(e).__name__,error=str(e));write(out/'failure.txt',traceback.format_exc())
    finally:
        if locker and locker.poll() is None:
            locker.kill();locker.wait(timeout=3)
        for proc in reversed(processes):
            try:stop_group(proc)
            except BaseException as e:
                result.update(result='FAIL',process_cleanup_error=str(e))
        if fixture and fixture.container and result['result']=='PASS':
            try:fixture.cleanup();result['cluster_removed']=True
            except BaseException as e:result.update(result='FAIL',cleanup_error=str(e),cluster_removed=False)
        elif fixture and fixture.container:
            result['cluster_retained_stopped']=True
            for database in sorted(fixture.databases):
                try:
                    dump=fixture.docker('exec',fixture.container,'pg_dump','-U','a09_admin','-d',database,'-Fc',timeout=30)
                    fd=os.open(out/(database+'.failure.dump'),os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
                    with os.fdopen(fd,'wb') as f:f.write(dump.stdout)
                except BaseException as e:result.setdefault('dump_errors',[]).append(type(e).__name__)
            try:fixture.docker('stop','--time','5',fixture.container,timeout=15);result['owned_pg_stopped']=True
            except BaseException as e:result['owned_pg_stop_error']=type(e).__name__

        if sockets and sockets.exists():
            socket_path=sockets/'core.sock'
            if socket_path.exists():socket_path.unlink()
            lock_path=sockets/'core.lock'
            if lock_path.is_file() and not lock_path.is_symlink() and lock_path.stat().st_uid==os.getuid() and lock_path.stat().st_size==0:lock_path.unlink()
            try:sockets.rmdir()
            except OSError as e:result.update(result='FAIL',socket_cleanup_error=str(e))
        write(out/'result.json',result)
        print(json.dumps({'result':result['result'],'report':str(out/'result.json')},sort_keys=True))
    return 0 if result['result'] in ('PASS','READY') else 2 if result['result']=='WAITING_INPUT' else 1

if __name__=='__main__':sys.exit(main())
