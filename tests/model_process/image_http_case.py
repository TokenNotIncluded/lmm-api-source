#!/usr/bin/env python3
"""Narrow real Core image HTTP case, adapted from a cached independent PASS.

scripts/verify_image_process.py --binding /private/binding.json  # prepare only
Add --execute --pg-window --evidence-dir /private/new-attempt only after the
serial PG window is released. No Cargo invocation, Docker or paid provider.
Binding JSON supplies core {path,sha256,kind=OS_MAIN,source_root,receipt,
receipt_sha256,source_manifest,source_manifest_sha256}, source_revision and
reviewed_fresh_target. Use the frozen candidate manifest; never live source.
Apply the reviewed1GiB/Swap0/Nice19/180s outer process budget.

Configuration and real Host/loopback supplier conventions follow model_process.rs.
Cached tested bytes and this repo adapter's unrun PG scope are recorded separately
in image_http_evidence.json. No complete Core/Go/Web/vendor acceptance is claimed.
"""
from pathlib import Path
import argparse,base64,hashlib,http.client,http.server,json,os,re,secrets,shutil,signal,socket,threading,time,tomllib,traceback
from email.parser import BytesParser
from email.policy import default as email_policy
import subprocess
import sys
sys.dont_write_bytecode = True
HERE=Path(__file__).resolve().parent
REPO=HERE.parents[1]
FIXTURES=HERE/'fixtures'
DB='image_http65_test'
def require(value, label):
    if not value: raise RuntimeError(label)
def sha(path):
    with Path(path).open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()
def stat_binding(path):
    s=Path(path).stat();return {'device':s.st_dev,'inode':s.st_ino,'bytes':s.st_size,'mtime_ns':s.st_mtime_ns,'ctime_ns':s.st_ctime_ns}
def literal(value):
    if value is None:return 'NULL'
    if type(value) is bool:return 'TRUE' if value else 'FALSE'
    if type(value) is int:return str(value)
    return "'"+str(value).replace("'","''")+"'"
def free_port():
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));return sock.getsockname()[1]

class ProcessFixture:
    """Only the process/SQL/evidence helpers used by this case; no Go fixture."""
    def __init__(self, core, out):
        self.pair, self.out = {'core':core}, out
        self.source, self.core = Path(core['source_root']), Path(core['path'])
        self.commands, self.checks, self.trace = [], [], []
        self.children, self.active_commands, self.secrets = [], {}, []
        self.lock = threading.RLock()
        self.work, self.pg_started, self.provider = None, False, None
        self.core_proc = None
        self.phase, self.initial_stat = 'prepare', {}
        self.tools = {name:shutil.which(name) for name in ('initdb','pg_ctl','psql','ionice','df','findmnt')}
        require(all(self.tools.values()), 'local PG18/tools missing')
        self.env = {k:v for k,v in os.environ.items() if k in ('PATH','HOME','USER','LOGNAME','LANG','LC_ALL','TZ','XDG_CACHE_HOME')}
        self.env.update(MISE_DISABLE_TOOLS='rust', MISE_AUTO_INSTALL='0')
    def clean(self, value):
        if isinstance(value, dict):
            return {k: ('[private]' if k.lower() in ('secret', 'key', 'password', 'refresh_secret', 'access_token')
                        else self.clean(v)) for k, v in value.items()}
        if isinstance(value, list): return [self.clean(v) for v in value]
        if isinstance(value, str):
            for token in self.secrets:
                value = value.replace(token, '[private]')
        return value

    def save(self, name, value):
        with self.lock:
            target = self.out / name
            raw = json.dumps(self.clean(value), ensure_ascii=False, indent=2) + '\n'
            temp = target.with_suffix(target.suffix + '.tmp')
            temp.write_text(raw); temp.chmod(0o600); temp.replace(target)

    def progress(self, stage):
        self.stage = stage
        self.save('live-handles.json', {'stage': stage, 'runner_pid': os.getpid(), 'phase': self.phase,
                  'pg_pid': self.pg_pid if self.pg_started else None,
                  'children': [{'name': name, 'pid': proc.pid, 'returncode': proc.poll()}
                               for name, proc, log in self.children], 'commands': self.active_commands})
        print(json.dumps({'stage': stage, 'pid': os.getpid()}, ensure_ascii=False), flush=True)

    def record(self, name, detail=None):
        self.checks.append({'name': name, 'phase': self.phase, 'result': 'PASS', 'detail': detail})
        self.save('checks.json', self.checks)

    def file(self, name, data):
        path = self.work / name
        path.write_bytes(data if isinstance(data, bytes) else data.encode())
        path.chmod(0o600)
        return path

    def cmd(self, name, argv, data=None, private=False, env=None, check=True, timeout=120, cwd=None):
        started = time.monotonic()
        argv = list(map(str, argv))
        p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             cwd=cwd or self.source, env=env or self.env, start_new_session=True)
        self.active_commands[name] = p.pid
        self.progress(name)
        try:
            stdout, stderr = p.communicate(data, timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGTERM)
            try: stdout, stderr = p.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL); stdout, stderr = p.communicate()
            (self.out / (name + '.timeout.log')).write_text(self.clean(('[private stdout omitted]\n' if private else stdout.decode(errors='replace')) + stderr.decode(errors='replace')))
            self.save(name + '-timeout.json', {'pid': p.pid, 'argv': argv, 'timeout_seconds': timeout, 'stopped': p.poll() is not None})
            raise RuntimeError(name + ' timed out; command stopped')
        except BaseException:
            if p.poll() is None:
                os.killpg(p.pid, signal.SIGTERM)
                try: stdout, stderr = p.communicate(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(p.pid, signal.SIGKILL); stdout, stderr = p.communicate()
                text = ('[private stdout omitted]\n' if private else stdout.decode(errors='replace')) + stderr.decode(errors='replace')
                (self.out / (name + '.interrupted.log')).write_text(self.clean(text))
            raise
        finally:
            self.active_commands.pop(name, None)
        log = '[private stdout omitted]\n' if private else stdout.decode(errors='replace')
        log += stderr.decode(errors='replace')
        (self.out / (name + '.log')).write_text(self.clean(log))
        self.commands.append({'name': name, 'argv': argv, 'pid': p.pid, 'returncode': p.returncode,
                              'seconds': time.monotonic() - started, 'private_stdout': private})
        with (self.out / 'commands.jsonl').open('a') as log:
            log.write(json.dumps(self.clean(self.commands[-1])) + '\n')
        if check: require(p.returncode == 0, name + ' failed; see preserved log')
        return p.returncode, stdout.decode(), stderr.decode()

    def sql(self, name, query, db=DB, user='fixture_admin', check=True, private=False, app=None):
        env = dict(self.env)
        if app: env['PGAPPNAME'] = app
        args = [self.tools['psql'], '-X', '-A', '-t', '-q', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=verbose',
                '-h', self.sock, '-p', self.pgport, '-U', user, '-d', db]
        self.save(name + '-sql.json', {'database': db, 'user': user, 'sql': query})
        return self.cmd(name, args, query.encode(), env=env, check=check, private=private)

    def value(self, name, query, **kw):
        return self.sql(name, query, **kw)[1].strip()

    def jsonsql(self, name, query, **kw):
        return json.loads(self.value(name, query, **kw))

    def startproc(self, name, binary, config, cwd):
        require(stat_binding(binary) == self.initial_stat[str(binary)], 'binary stat drift')
        log = (self.out / (name + '.log')).open('wb')
        proc = subprocess.Popen([str(binary), '--config', str(config)], stdout=log, stderr=subprocess.STDOUT,
                                env=self.env, cwd=cwd, start_new_session=True)
        self.children.append((name, proc, log))
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            require(proc.poll() is None, name + ' exited during exec')
            try:
                if Path('/proc/' + str(proc.pid) + '/exe').resolve() == binary: break
            except FileNotFoundError: pass
            time.sleep(.02)
        else: raise RuntimeError(name + ' executable binding failed')
        self.save(name + '-process.json', {'pid': proc.pid, 'argv': [str(binary), '--config', str(config)],
                  'cwd': str(cwd), 'binary_sha256': self.pair['core' if binary == self.core else 'go']['sha256'],
                  'config_sha256': sha(config), 'nice': os.getpriority(os.PRIO_PROCESS, proc.pid)})
        self.progress(name + '-started')
        return proc

    def stopproc(self, proc):
        if proc is None or proc.poll() is not None: return
        os.killpg(proc.pid, signal.SIGTERM)
        try: proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL); proc.wait(timeout=3)

    def cleanup(self):
        errors = []
        if self.work and (self.work / 'data/postmaster.pid').exists() and not self.pg_started:
            try:
                pid = int((self.work / 'data/postmaster.pid').read_text().splitlines()[0])
                argv = Path('/proc/' + str(pid) + '/cmdline').read_bytes().split(b'\0')
                if os.fsencode(self.work / 'data') in argv and Path('/proc/' + str(pid) + '/exe').resolve().name == 'postgres':
                    self.pg_pid, self.pg_started = pid, True
            except (FileNotFoundError, ProcessLookupError, ValueError): pass
        for name, proc, log in reversed(self.children):
            try: self.stopproc(proc)
            except Exception as error: errors.append(name + ': ' + str(error))
            try: log.close()
            except Exception: pass
        if self.provider:
            self.provider.shutdown(); self.provider.server_close()
        if self.pg_started:
            try:
                self.cmd('pg-stop', [self.tools['pg_ctl'], '-D', self.work / 'data', '-m', 'fast', '-w', 'stop'])
                self.pg_started = False
            except Exception as error: errors.append('PG cleanup: ' + str(error))
        if self.work and not self.pg_started:
            shutil.rmtree(self.work)
        for log in self.out.glob('*.log'):
            log.write_text(self.clean(log.read_text(errors='replace')))
        self.save('cleanup.json', {'owned_fixture_removed': bool(self.work and not self.work.exists()),
                  'pg_stopped': not self.pg_started, 'errors': errors,
                  'children': [{'name': n, 'pid': p.pid, 'returncode': p.poll()} for n, p, _ in self.children]})
        return errors


def toml_inline(value):
    if isinstance(value,str):return json.dumps(value,ensure_ascii=False)
    if type(value) is bool:return str(value).lower()
    if type(value) is int:return str(value)
    if isinstance(value,list):return '['+','.join(map(toml_inline,value))+']'
    if isinstance(value,dict):return '{'+','.join(json.dumps(k)+'='+toml_inline(v) for k,v in value.items())+'}'
    raise ValueError('unsupported TOML fixture value')
def model_toml(model):
    text='\n'.join(json.dumps(k)+'='+toml_inline(v) for k,v in model.items())+'\n'
    require(tomllib.loads(text)==model,'fixture TOML roundtrip')
    return text
EXPR='(tier("standard", p * 5 + cr_text * 1.25 + img * 8 + cr_img * 2 + img_o * 30)) * (1)'
PNG='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lZkAAAAASUVORK5CYII='
def closure(source):
    found={}
    def visit(p):
        p=p.resolve();require(p.is_relative_to(source.resolve()),'SQL include escapes snapshot');require(p.is_file(),'missing SQL include '+str(p))
        if str(p) in found:return
        found[str(p)]=sha(p)
        for line in p.read_text().splitlines():
            m=re.match(r'\s*\\ir\s+(.+?)\s*$',line)
            if m:visit(p.parent/m[1].strip("'\""))
    visit(source/'deployment/postgres/grant-core-runtime.sql');return found

def source_hashes(path):
    value=json.loads(Path(path).read_text());files=value.get('files',value)
    if isinstance(files,list):files={entry['path']:entry['sha256'] for entry in files}
    require(isinstance(files,dict) and all(isinstance(k,str) and isinstance(v,str) and len(v)==64 for k,v in files.items()),'unsupported source manifest shape')
    return files

def verify(binding,execute=False):
    evidence=json.loads((HERE/'image_http_evidence.json').read_text())
    for name,expected in evidence['fixture_hashes'].items():require(sha(FIXTURES/name)==expected,'fixture drift '+name)
    core=binding['core'];require(core['kind']=='OS_MAIN','not OS main')
    source=Path(core['source_root']);require(source.resolve()==source and source!=REPO,'frozen source required')
    for field in ['receipt','source_manifest']:require(sha(core[field])==core[field+'_sha256'],'builder '+field+' changed')
    receipt=json.loads(Path(core['receipt']).read_text());require(receipt['binary_sha256']==core['sha256'],'receipt binary mismatch')
    require(Path(core['path']).stat().st_size==receipt['binary_size'],'binary size mismatch')
    require(receipt['production_rust_revision']==binding['source_revision'],'receipt revision mismatch')
    require(receipt['fresh_target']==binding['reviewed_fresh_target'],'target receipt mismatch')
    manifest=source_hashes(core['source_manifest'])
    for path,expected in evidence['source_scope'].items():require(manifest.get(path)==expected and sha(source/path)==expected,'reviewed source mismatch '+path)
    target_path='services/core/src/core_schema/upgrade/fresh_target.json'
    require(json.loads((source/target_path).read_text())==binding['reviewed_fresh_target'],'target content mismatch')
    includes=closure(source)
    require(all(manifest[str(Path(p).relative_to(source))]==h for p,h in includes.items()),'SQL closure mismatch')
    if execute:
        require(sha(core['path'])==core['sha256'],'binary hash mismatch')
        require(binding.get('window_authorized') is True,'no serial PG window supplied')
        require(not binding.get('product_blockers'),'product blocker')
    return {'status':'REPOSITORY_ADAPTER_PREPARED_NOT_PG_RUN','source_inputs':len(evidence['source_scope']),'sql_closure':includes,'executed':False}

class Provider(http.server.ThreadingHTTPServer):
    daemon_threads=True
    def __init__(self):super().__init__(('127.0.0.1',0),Handler);self.calls=[];self.errors=[]
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_POST(self):
        try:
            require(self.path in ['/v1/images/generations','/v1/images/edits'],'provider path')
            raw=self.rfile.read(int(self.headers['Content-Length']))
            if self.path.endswith('edits'):
                msg=BytesParser(policy=email_policy).parsebytes(('Content-Type: '+self.headers['Content-Type']+'\r\nMIME-Version: 1.0\r\n\r\n').encode()+raw)
                body={part.get_param('name',header='content-disposition'):part.get_payload(decode=True).decode() for part in msg.iter_parts() if part.get_param('name',header='content-disposition') not in ['image','image[]','mask']}
            else:body=json.loads(raw)
            require(self.headers['Authorization']=='Bearer synthetic-image-independent-key','provider credential')
            require(body['model']=='supplier-image','provider model rewrite');self.server.calls.append({'path':self.path,'body':body})
            usage={'input_tokens':100,'output_tokens':10,'total_tokens':110,'input_tokens_details':{'text_tokens':40,'image_tokens':60,'cached_tokens':40,'cached_tokens_details':{'text_tokens':10,'image_tokens':30,'audio_tokens':0}}}
            if 'unknown-cache' in body['prompt']:usage['input_tokens_details'].pop('cached_tokens_details')
            response={'created':1,'data':[{'b64_json':PNG}]*(2 if 'flare-fixed' in body['prompt'] else 1),'usage':usage,'size':'1024x1024','quality':'standard'}
            payload=json.dumps(response).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
        except Exception as e:self.server.errors.append(str(e));self.send_error(500)

class Runner(ProcessFixture):
    def __init__(self,binding,out):
        # Same CLI/config/loopback-provider pattern as model_process.rs.
        super().__init__(binding['core'],out)
        self.binding=binding;self.phase='image-osmain';self.model=None
    def sql(self,name,query,db=DB,**kw):return super().sql(name,query,db=db,**kw)
    def request(self,label,path,body,token,key=None,multipart=False):
        headers={'Content-Type':'application/json','Origin':'http://127.0.0.1:'+str(self.coreport)}
        if token:headers['Authorization']='Bearer '+token
        if key:headers['Idempotency-Key']=key
        payload=json.dumps(body).encode() if body is not None else None
        if multipart:
            boundary='ImageIndependentBoundary'
            payload=b''
            for name,value in body.items():payload+=f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode()
            payload+=f'--{boundary}\r\nContent-Disposition: form-data; name="image"; filename="fixture.png"\r\nContent-Type: image/png\r\n\r\n'.encode()+base64.b64decode(PNG)+f'\r\n--{boundary}--\r\n'.encode()
            headers['Content-Type']='multipart/form-data; boundary='+boundary
        self.save(label+'-request.json',{'method':'GET' if body is None else 'POST','path':path,'body':body,'multipart':multipart,'idempotency_key':key})
        conn=http.client.HTTPConnection('127.0.0.1',self.coreport,timeout=20)
        try:
            conn.request('GET' if body is None else 'POST',path,payload,headers);response=conn.getresponse();data=response.read(1<<20);result={'status':response.status,'headers':dict(response.getheaders()),'json':json.loads(data)}
        finally:conn.close()
        self.save(label+'-http.json',result);return result
    def model_document(self):
        ratio=lambda a,d=1:{'numerator':a,'denominator':d}
        token={'billing_price_key':'gpt-image-2','source_expression':EXPR,'source_sha256':hashlib.sha256(EXPR.encode()).hexdigest(),'currency_basis':'legacy_pricing_unit','credits_per_usd':500000,'usd_multiplier':ratio(1),'max_input_tokens':100,'max_output_tokens':100}
        legacy={'billing_price_key':'google/gemini-3.1-flash-image','model_ratio':ratio(1,4),'completion_ratio':ratio(6),'cache_ratio':ratio(1,10),'image_ratio':ratio(1),'legacy_quota_per_unit':500000,'credits_per_usd':500000,'usd_multiplier':ratio(1),'max_input_tokens':100,'max_output_tokens':100}
        model=tomllib.loads((FIXTURES/'image-model.toml').read_text().format(self.file('provider.key','synthetic-image-independent-key'),'http://127.0.0.1:'+str(self.provider.server_port)+'/v1'))
        rules=[]
        for semantic,basis in [('gpt-image-2',{'native_tokens':token}),('google/gemini-3.1-flash-image',{'legacy_token_ratio':legacy}),('gpt-image-2.5-flare','returned_payloads')]:
            for endpoint in ['generations','edits']:
                rules.append({'upstream_id':1,'vendor':'openai','billing_group':'allowed','upstream_model':'supplier-image','semantic_model':semantic,'endpoint':endpoint,'size':'1024x1024','quality':'standard','usd_micros_per_image':ratio(1000000 if isinstance(basis,str) else 0),'group_multiplier':ratio(1),'max_n':3,'count_basis':basis,'max_budget':2000000,'source_revision':'independent-synthetic-not-production'})
        model['image_prices']={'revision':1,'rules':rules};model['image_providers']=[{'id':1,'vendor':'openai','upstream_model':'supplier-image','endpoint':'http://127.0.0.1:'+str(self.provider.server_port)+'/v1','secret_file':str(self.work/'provider.key')}]
        return model
    def prepare_execution(self):
        self.initial_stat[str(self.core)]=stat_binding(self.core)
        require(os.geteuid()!=0,'unprivileged PG required');os.setpriority(os.PRIO_PROCESS,0,max(19,os.getpriority(os.PRIO_PROCESS,0)))
        self.cmd('ionice-self',['ionice','-c2','-n7','-p',os.getpid()]);self.cmd('capacity',['df','-hT',HERE]);self.cmd('mount',['findmnt','-T',HERE]);require(shutil.disk_usage(HERE).free>1024**3,'insufficient capacity')
        import tempfile
        self.work=Path(tempfile.mkdtemp(prefix='im9-',dir=Path.home()/'.cache'));self.sock=self.work/'s';self.sock.mkdir(mode=0o700);require(len(os.fsencode(self.sock))<100,'socket path');self.pgport,self.coreport,self.goport=free_port(),free_port(),0
        require(self.pgport!=self.coreport,'port collision')
        for tool in ['initdb','pg_ctl','psql']:require(re.search(r'\b18\.',self.cmd(tool+'-version',[self.tools[tool],'--version'])[1]),'PG18 required')
        self.save('fixture.json',{'database':DB,'root':str(self.work),'socket':str(self.sock),'coreport':self.coreport,'PGport':self.pgport,'core':self.binding['core'],'no_go':True,'no_mock_core':True})
    def bootstrap(self):
        self.cmd('pg-init',[self.tools['initdb'],'-D',self.work/'data','-U','fixture_admin','--auth=trust','--encoding=UTF8','--locale=C','--wal-segsize=1'])
        self.cmd('pg-start',[self.tools['pg_ctl'],'-D',self.work/'data','-l',self.out/'postgres.log','-o',f'-k {self.sock} -p {self.pgport} -c listen_addresses=127.0.0.1 -c shared_buffers=16MB -c max_connections=32 -c min_wal_size=2MB -c max_wal_size=32MB','-w','start']);self.pg_started=True;self.pg_pid=int((self.work/'data/postmaster.pid').read_text().splitlines()[0])
        self.sql('roles',f'CREATE ROLE lmm_core_owner NOLOGIN;CREATE ROLE core_installer LOGIN NOINHERIT;CREATE ROLE core_runtime LOGIN NOINHERIT;GRANT lmm_core_owner TO core_installer WITH INHERIT FALSE,SET TRUE;CREATE DATABASE {DB} OWNER lmm_core_owner;',db='postgres')
        self.sql('boundary',f'REVOKE ALL ON DATABASE {DB} FROM PUBLIC;GRANT CONNECT ON DATABASE {DB} TO core_installer,core_runtime;REVOKE CREATE ON SCHEMA public FROM PUBLIC;')
        url=lambda user,owner=False:f'postgresql://{user}@127.0.0.1:{self.pgport}/{DB}?sslmode=disable'+('&options=-c%20role%3Dlmm_core_owner' if owner else '')
        self.ownercfg=self.file('owner.toml','version=1\n[database]\nurl_file='+json.dumps(str(self.file('owner.url',url('core_installer',True))))+'\n')
        self.cmd('core-init',[self.core,'--config',self.ownercfg,'init-db'])
        self.password='P!'+secrets.token_hex(16);self.secrets.append(self.password)
        first=json.loads(self.cmd('first-owner',[self.core,'--config',self.ownercfg,'create-first-owner'],json.dumps({'username':'imageowner','display_name':'Image independent','password':self.password}).encode(),private=True)[1]);require(first['setup_completed'] is True,'owner setup')
        env=dict(self.env,PGHOST=str(self.sock),PGPORT=str(self.pgport),PGUSER='core_installer',PGDATABASE=DB,PGOPTIONS='-c role=lmm_core_owner')
        args=[self.tools['psql'],'-X','-q','-v','ON_ERROR_STOP=1']
        for flag in ['runtime_role=core_runtime','enable_console=true','enable_web_identity=true','enable_events=true','enable_models=true','enable_images=true']:args+=['-v',flag]
        self.cmd('runtime-grants',args+['-f',self.source/'deployment/postgres/grant-core-runtime.sql'],env=env)
        self.sql('routing-group',"INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(1,'allowed',true,'independent-image');")
        account=int(self.value('account','SELECT personal_account_id FROM core_identity.users WHERE id=1'))
        self.value('open-wallet',f'SELECT core_billing.open_ledger_wallet({account})')
        funding={'scope':'independent-image','operation_key':'local-fixture','actor_user_id':1,'reason':'Disposable synthetic credits','action':{'kind':'credit','account_id':account,'amount_units':5000000,'payment_reference':'synthetic:image'}}
        require(self.jsonsql('fund','SELECT core_billing.post_ledger('+literal(json.dumps(funding))+'::jsonb)')['status']=='posted','fund failed')
        self.provider=Provider();threading.Thread(target=self.provider.serve_forever,daemon=True).start();self.model=self.model_document();self.modelcfg=self.file('model.toml',model_toml(self.model))
        identity=self.file('identity.token',secrets.token_hex(32));refresh=self.file('refresh.key',secrets.token_bytes(32))
        self.corecfg=self.file('core.toml',(FIXTURES/'image-core.toml').read_text().format(coreport=self.coreport,runtime=json.dumps(str(self.file('runtime.url',url('core_runtime')))),refresh=json.dumps(str(refresh)),socket=json.dumps(str(self.sock/'core.sock')),identity=json.dumps(str(identity)),model=json.dumps(str(self.modelcfg))))
        self.cmd('check-db',[self.core,'--config',self.corecfg,'check-db']);self.restart()
        login=self.request('login','/core/v1/auth/login',{'login_name':'imageowner','password':self.password},'');require(login['status']==200,'login failed');self.session=login['json']['secret'];self.secrets.append(self.session)
        key=self.request('key','/api/token',{'name':'independent-image','remain_quota':5000000,'unlimited_quota':False,'expired_time':-1,'group':'allowed'},self.session);require(key['status']==200,'key failed');self.key=key['json']['data']['key'];self.secrets.append(self.key)
    def restart(self):
        self.stopproc(self.core_proc);self.core_proc=self.startproc('core-main',self.core,self.corecfg,self.source)
        deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            require(self.core_proc.poll() is None,'Core exited during startup')
            try:
                conn=http.client.HTTPConnection('127.0.0.1',self.coreport,timeout=.3)
                try:
                    conn.request('GET','/health/ready');response=conn.getresponse();ready=json.loads(response.read(65536))
                    if response.status==200 and ready.get('capabilities',{}).get('models') is True:
                        self.save('core-ready.json',ready);return
                finally:conn.close()
            except (OSError,ValueError,http.client.HTTPException):pass
            time.sleep(.05)
        raise RuntimeError('Core listener unavailable')
    def row(self,name,operation):
        return self.jsonsql(name,"SELECT jsonb_build_object('operation_state',o.state,'image_usage',o.image_usage,'output',o.output,'price',o.price,'charge_price',c.price,'charge_state',c.state,'reserved',c.reserved,'settled',c.settled,'refunded',c.refunded) FROM core_image.operations o JOIN core_charging.requests c ON c.id=o.id WHERE o.id="+literal(operation))
    def exercise(self):
        for name,model,prompt,expected,edit in [('native','gpt-image-2','native',343,False),('edit','gpt-image-2','native-edit',343,True),('ratio','google/gemini-3.1-flash-image','unknown-cache ratio',28,False),('fixed','gpt-image-2.5-flare','flare-fixed',900000,False)]:
            count=len(self.provider.calls);key=secrets.token_hex(32);body={'model':model,'prompt':prompt,'n':1,'size':'1024x1024','quality':'standard'}
            if name=='fixed':body['n']=3
            path='/v1/images/edits' if edit else '/v1/images/generations';response=self.request(name,path,body,self.key,key,edit);require(response['status']==200,name+' HTTP failed')
            op=next(v for k,v in response['headers'].items() if k.lower()=='x-lmm-image-operation-id');row=self.row(name+'-receipt',op);require(row['charge_state']=='settled' and row['settled']==expected and row['refunded']==0,name+' price mismatch');require(row['reserved']=={'native':1710,'edit':1710,'ratio':160,'fixed':1350000}[name],name+' reservation mismatch');require(row['output']['usage']==response['json']['usage'],'raw usage changed')
            if name=='fixed':require(row['reserved']==1350000,'fixed n=3 reservation wrong')
            again=self.request(name+'-replay',path,body,self.key,key,edit);require(again['status']==200 and len(self.provider.calls)==count+1,'replay redispatch');self.record(name+' measured settlement + no retry',row)
            if name=='native':self.original=(op,key,body)
        count=len(self.provider.calls);body={'model':'gpt-image-2','prompt':'unknown-cache','n':1,'size':'1024x1024','quality':'standard'};key=secrets.token_hex(32);response=self.request('unknown','/v1/images/generations',body,self.key,key);require(response['status']!=200,'unknown classified as success');op=response['json']['operation_id'];row=self.row('unknown-receipt',op);require(row['charge_state']=='reconcile' and row['settled']==0 and row['reserved']==1710,'unknown hold lost');require(row['output']['usage']['input_tokens_details']['cached_tokens']==40,'unknown raw receipt missing');self.request('unknown-replay','/v1/images/generations',body,self.key,key);require(len(self.provider.calls)==count+1,'unknown redispatched')
        require(not self.provider.errors,'supplier error');self.save('provider-calls.json',self.provider.calls)
        require(len(self.provider.calls)==5,'unexpected supplier POST count')
        wallet=self.jsonsql('wallet-after',"SELECT jsonb_object_agg(a.bucket,b.balance_units) FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.owner_account_id=(SELECT personal_account_id FROM core_identity.users WHERE id=1)")
        require(wallet['wallet']==5000000-343-343-28-900000-1710,'wallet debit mismatch')
        require(wallet['reserved']==1710,'UNKNOWN reserve mismatch')
        self.record('single payer funds match measured settles and UNKNOWN hold',wallet)
        for name,q in [('no-DDL','CREATE TABLE core_image.forbidden_fixture(x int)'),('no-DELETE','DELETE FROM core_image.operations WHERE false')]:require(self.sql(name,q,user='core_runtime',check=False)[0]!=0,'runtime privilege violation')
    def failure_snapshot(self):
        if self.provider:self.save('provider-before-cleanup.json',{'calls':self.provider.calls,'errors':self.provider.errors})
        if self.pg_started:
            try:
                rows=self.jsonsql('failure-financial-rows',"SELECT coalesce(jsonb_agg(jsonb_build_object('id',o.id,'operation_state',o.state,'price',o.price,'image_usage',o.image_usage,'output',o.output,'charge_state',c.state,'reserved',c.reserved,'settled',c.settled,'refunded',c.refunded)),'[]'::jsonb) FROM core_image.operations o LEFT JOIN core_charging.requests c ON c.id=o.id")
                self.save('failure-financial-snapshot.json',rows)
            except Exception as error:self.save('failure-snapshot-error.json',{'error':str(error)})
    def run(self):self.prepare_execution();self.bootstrap();self.exercise()

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binding',type=Path,required=True);parser.add_argument('--execute',action='store_true');parser.add_argument('--pg-window',action='store_true',help='supply only after root explicitly releases the serial PG window');parser.add_argument('--evidence-dir',type=Path);args=parser.parse_args();binding=json.loads(args.binding.read_text());binding['window_authorized']=args.pg_window;prepared=verify(binding,args.execute)
    if not args.execute:print(json.dumps(prepared,indent=2));return
    require(args.evidence_dir is not None,'new evidence dir required');out=args.evidence_dir.resolve();require(not out.is_relative_to(REPO) and out.parent.resolve()==out.parent,'new absolute private output outside checkout required');out.mkdir(mode=0o700,exist_ok=False);runner=Runner(binding,out);status='FAIL';error=None
    def terminate(signum,frame):raise RuntimeError('runner terminated by signal '+str(signum))
    signal.signal(signal.SIGTERM,terminate)
    try:runner.run();status='PASS_LIMITED_IMAGE_OS_MAIN'
    except Exception as e:error=str(e);runner.save('diagnosis.json',{'error':error,'traceback':traceback.format_exc()});runner.failure_snapshot()
    finally:
        cleanup=runner.cleanup()
        if cleanup:status='FAIL'
        runner.save('result.json',{'status':status,'error':error,'cleanup_errors':cleanup,'checks':runner.checks,'independent_run_of_fixed_developer_main_not_independent_build':True,'limits':['Synthetic loopback provider, not a paid vendor capture','Gemini is legacy ratio through images API, not OpenRouter Chat or native Gemini wire','No crash/restart fairness64 or stress test in this narrow phase']})
    print(json.dumps({'status':status,'evidence':str(out)}));raise SystemExit(0 if status.startswith('PASS') else 1)
if __name__=='__main__':main()
