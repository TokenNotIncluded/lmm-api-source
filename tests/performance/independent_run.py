#!/usr/bin/env python3
"""Independent local PG + actual Native + timed TCP supplier acceptance."""
import argparse, contextlib, fcntl, hashlib, http.client, http.server, json, math, os
from pathlib import Path
import secrets, shutil, socket, subprocess, sys, threading, time, traceback
from urllib.parse import urlencode

FREEZE=None
CORE_SHA=None
CASES=['actual-json-and-aggregate','sse-content-ttft','groups-and-session-permissions','query-boundaries','terminal-finalization-failure','append-once-and-restart','process-death-missing-sample','hidden-model-filter']

def sha(p):
    with Path(p).open('rb') as f: return hashlib.file_digest(f,'sha256').hexdigest()
def require(ok,msg):
    if not ok: raise AssertionError(msg)
def dump(p,v): p.write_text(json.dumps(v,ensure_ascii=False,indent=2)+'\n')
def freeport():
    with socket.socket() as s: s.bind(('127.0.0.1',0)); return s.getsockname()[1]

class Run:
    def __init__(self,root):
        self.root=root; self.log=root/'logs'; self.log.mkdir(); self.seq=0
        self.results={k:'NOT_RUN' for k in CASES}; self.core=None; self.pg=False
        self.provider=None; self.supplier=[]; self.supplier_lock=threading.Lock()
        self.mode='json'; self.delay=.22; self.tokens=5; self.checkpoint=threading.Event(); self.release=threading.Event()
        self.port=freeport(); self.socket=root/'s'; self.socket.mkdir(); self.pgenv=dict(os.environ)
        self.base=['psql','-X','-q','-A','-t','-v','ON_ERROR_STOP=1','-h',str(self.socket),'-p','55603','-U','postgres','-d','perf']
        self.core_port=freeport(); self.generation=0
    def run(self,label,cmd,input=None,timeout=90,check=True):
        self.seq+=1; p=self.log/f'{self.seq:03}-{label}'
        dump(p.with_suffix('.command.json'),[str(x) for x in cmd])
        if input is not None:p.with_suffix('.stdin').write_bytes(input)
        c=subprocess.run([str(x) for x in cmd],input=input,capture_output=True,timeout=timeout,env=self.pgenv)
        p.with_suffix('.stdout').write_bytes(c.stdout);p.with_suffix('.stderr').write_bytes(c.stderr)
        require(not check or c.returncode==0,f'{label} exit {c.returncode}; see {p}')
        return c
    def sql(self,query,check=True):
        return self.run('sql',self.base,input=query.encode(),check=check).stdout.decode().strip()
    def rows(self):
        return json.loads(self.sql("SELECT coalesce(jsonb_agg(jsonb_build_object('id',r.id,'state',r.state,'model',r.price->>'model','group',r.price->>'group','authorized_at',r.authorized_at,'detail',e.detail)),'[]') FROM core_charging.requests r LEFT JOIN core_charging.usage_events e ON e.request_id=r.id AND e.event_id='performance-v1'"))
    def wait_samples(self,n):
        end=time.monotonic()+12
        while time.monotonic()<end:
            rows=self.rows()
            if sum(r['detail'] is not None for r in rows)>=n: return rows
            time.sleep(.1)
        raise AssertionError(f'performance sample not persisted: expected {n}, got {rows}')
    def http(self,path,token=None,body=None,label='http',expect=None):
        self.seq+=1;p=self.log/f'{self.seq:03}-{label}'
        headers={}
        if token: headers['Authorization']='Bearer '+token
        payload=None if body is None else json.dumps(body).encode()
        if payload: headers['Content-Type']='application/json'
        dump(p.with_suffix('.request.json'),{'path':path,'method':'GET' if body is None else 'POST','authenticated':bool(token),'body':body})
        t=time.monotonic();conn=http.client.HTTPConnection('127.0.0.1',self.core_port,timeout=20)
        try:
            conn.request('GET' if body is None else 'POST',path,payload,headers)
            res=conn.getresponse();data=res.read();status=res.status
            p.with_suffix('.body').write_bytes(data)
            dump(p.with_suffix('.response.json'),{'status':status,'headers':res.getheaders(),'elapsed':time.monotonic()-t})
        except Exception as e:
            dump(p.with_suffix('.error.json'),{'error':repr(e),'elapsed':time.monotonic()-t});raise
        finally: conn.close()
        if expect is not None: require(status==expect,f'{label} HTTP {status}, expected {expect}; {p}')
        try: parsed=json.loads(data)
        except ValueError: parsed=data.decode(errors='replace')
        return status,parsed,time.monotonic()-t
    def perf(self,query='',session=None,detail=False,expect=200):
        path='/api/perf-metrics'+('' if detail else '/summary')+query
        return self.http(path,session or self.creds['session'],label='performance',expect=expect)[1]
    def completion(self,stream=False,key=0,model='local-model',expect=200):
        return self.http('/v1/chat/completions',self.creds['keys'][key]['secret'],{'model':model,'messages':[{'role':'user','content':'Synthetic local independent probe'}],'stream':stream,'max_tokens':20},label='completion',expect=expect)
    def start_provider(self):
        owner=self
        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version='HTTP/1.1'
            def log_message(self,*args): pass
            def do_POST(self):
                body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                entry={'ordinal':len(owner.supplier)+1,'received_monotonic':time.monotonic(),'path':self.path,'body':body,'mode':owner.mode}
                with owner.supplier_lock: owner.supplier.append(entry);dump(owner.root/'supplier.json',owner.supplier)
                mode=owner.mode; tokens=owner.tokens
                try:
                    if mode=='fail-finalize':
                        owner.sql("CREATE FUNCTION public.independent_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.state='settled' THEN RAISE EXCEPTION 'independent finalization fault' USING ERRCODE='23514'; END IF; RETURN NEW; END $$; CREATE TRIGGER independent_fail BEFORE UPDATE ON core_charging.requests FOR EACH ROW EXECUTE FUNCTION public.independent_fail()")
                    if not body.get('stream'):
                        time.sleep(owner.delay)
                        response={'id':'synthetic-local','object':'chat.completion','model':'provider-model','choices':[{'index':0,'message':{'role':'assistant','content':'hello'},'finish_reason':'stop'}],'usage':{'prompt_tokens':10,'completion_tokens':tokens,'total_tokens':10+tokens}}
                        raw=json.dumps(response).encode();entry['first_meaningful_monotonic']=time.monotonic()
                        self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw);self.wfile.flush()
                    else:
                        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Connection','close');self.end_headers();self.close_connection=True
                        def emit(delta,finish=None,usage=None):
                            msg={'id':'synthetic-stream','object':'chat.completion.chunk','model':'provider-model','choices':[{'index':0,'delta':delta,'finish_reason':finish}]}
                            if usage: msg['usage']=usage
                            self.wfile.write(('data: '+json.dumps(msg)+'\n\n').encode());self.wfile.flush()
                        self.wfile.write(b': heartbeat\n\n');self.wfile.flush();emit({'role':'assistant'});time.sleep(.14)
                        emit({'content':''});time.sleep(.18)
                        self.wfile.write(b': second-heartbeat\n\n');self.wfile.flush();time.sleep(.16)
                        if mode=='crash': owner.checkpoint.set();owner.release.wait(15);return
                        entry['first_meaningful_monotonic']=time.monotonic();emit({'content':'hello'});time.sleep(.16)
                        emit({},'stop',{'prompt_tokens':10,'completion_tokens':tokens,'total_tokens':10+tokens})
                        self.wfile.write(b'data: [DONE]\n\n');self.wfile.flush()
                    entry['terminal_monotonic']=time.monotonic()
                except Exception as e: entry['error']=repr(e)
                finally:
                    with owner.supplier_lock: dump(owner.root/'supplier.json',owner.supplier)
        self.provider=http.server.ThreadingHTTPServer(('127.0.0.1',self.port),Handler)
        self.provider.daemon_threads=True
        threading.Thread(target=self.provider.serve_forever,daemon=True).start()
    def prepare(self):
        cfg=json.loads((FREEZE/'compile-config.json').read_text())
        self.run('copy-native',['cp','--reflink=auto',FREEZE/'lmm-core',self.root/'lmm-core'])
        require(sha(self.root/'lmm-core')==CORE_SHA,'copied native hash')
        self.run('link-fixture',[cfg['compiler'],'--edition=2024',self.root/'runner/independent_fixture.rs','--extern','lmm_core='+str(FREEZE/'liblmm_core.rlib'),*cfg['externs'],*cfg['search'],*cfg['link'],'-o',self.root/'fixture'],timeout=180)
        self.run('pg-version',['postgres','--version'])
        self.run('initdb',['initdb','-D',self.root/'pg','-U','postgres','--no-locale','--encoding=UTF8','--auth=trust','--wal-segsize=1'])
        self.run('pg-start',['pg_ctl','-D',self.root/'pg','-l',self.root/'postgres.log','-o',f'-c listen_addresses= -p 55603 -c unix_socket_directories={self.socket} -c unix_socket_permissions=0700 -c shared_buffers=16MB -c max_connections=20 -c min_wal_size=2MB -c max_wal_size=128MB','-w','start'])
        self.pg=True
        self.run('create-db',self.base[:-1]+['postgres','-c','CREATE DATABASE perf'])
        self.run('roles',self.base+['-f',FREEZE/'source/deployment/postgres/create-core-roles.sql'])
        self.sql("ALTER DATABASE perf OWNER TO lmm_core_owner; CREATE ROLE independent_installer LOGIN NOINHERIT; GRANT lmm_core_owner TO independent_installer WITH INHERIT FALSE,SET TRUE; GRANT CONNECT ON DATABASE perf TO independent_installer; REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_control_system() TO lmm_core_owner")
        def url(role,owner=False):
            q={'host':str(self.socket),'port':55603,'sslmode':'disable'}
            if owner:q['options']='-c role=lmm_core_owner'
            return 'postgresql://'+role+'@localhost/perf?'+urlencode(q)
        (self.root/'owner.url').write_text(url('independent_installer',True))
        (self.root/'owner.toml').write_text('version=1\n[database]\nurl_file='+json.dumps(str(self.root/'owner.url'))+'\n')
        self.run('native-init',[self.root/'lmm-core','--config',self.root/'owner.toml','init-db'])
        self.run('seed',[self.root/'fixture',self.root/'owner.url'])
        self.creds=json.loads(next(self.log.glob('*-seed.stdout')).read_text())
        dump(self.root/'credentials.json',self.creds)
        self.run('canonical-grants',self.base+['-v','enable_models=true','-f',FREEZE/'source/deployment/postgres/grant-core-runtime.sql'])
        (self.root/'runtime.url').write_text(url('lmm_core_runtime'))
        (self.root/'supplier.key').write_text('synthetic-independent-only')
        self.start_provider()
        routes=[]; rules=[]
        for i,(g,m) in enumerate([('allowed','local-model'),('秘密组','local-model'),('allowed','other-model')],1):
            routes.append("{id=%d,model='%s',group='%s',enabled=true,speeds=['standard'],targets=[{id=%d,upstream_id=1,upstream_model='provider-model',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}],failover={max_attempts=1,retry_on=[],allow_replay_after_send=false}}"%(i,m,g,i))
            rules.append("{id=%d,model='%s',group='%s',speed='standard',rates={input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0},group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000}"%(i,m,g))
        model="version=1\ninput_token_ceiling=100\nlease_seconds=20\ntimeout_seconds=3\nmax_output_tokens=20\nlocal_streaming=true\ncredentials=[{reference={id=7,revision=1},secret_file='%s'}]\nkey_groups=[%s]\n[routing]\nversion=1\ngroups=[{name='allowed',enabled=true},{name='秘密组',enabled=true}]\nupstreams=[{id=1,endpoint='http://127.0.0.1:%d/v1',protocol='openai_chat',credential={id=7,revision=1},enabled=true}]\nroutes=[%s]\naliases=[]\n[prices]\nversion=1\nrules=[%s]\n"%(self.root/'supplier.key',','.join("{key_id=%d,group='%s'}"%(k['id'],k['group']) for k in self.creds['keys']),self.port,','.join(routes),','.join(rules))
        (self.root/'model.toml').write_text(model)
        (self.root/'core.toml').write_text("version=1\n[server]\nlisten='127.0.0.1:%d'\nworker_threads=1\nmax_inflight=16\ndrain_timeout_seconds=3\n[database]\nurl_file='%s'\nmax_connections=3\n[model]\nconfig_file='%s'\nlocal_http=true\n"%(self.core_port,self.root/'runtime.url',self.root/'model.toml'))
        self.start_core()
    def start_core(self):
        self.generation+=1
        argv=[str(self.root/'lmm-core'),'--config',str(self.root/'core.toml'),'serve'];dump(self.root/f'core-{self.generation}-command.json',argv)
        with (self.root/f'core-{self.generation}.log').open('wb') as out:self.core=subprocess.Popen(argv,stdout=out,stderr=subprocess.STDOUT,env=self.pgenv)
        deadline=time.monotonic()+15
        while time.monotonic()<deadline:
            require(self.core.poll() is None,'actual Native exited at startup; inspect core log')
            try:
                with socket.create_connection(('127.0.0.1',self.core_port),.1):break
            except OSError:time.sleep(.1)
        else:raise AssertionError('actual Native did not listen')
        exe=Path(f'/proc/{self.core.pid}/exe');require(sha(exe)==CORE_SHA,'actual MainPID executable mismatch')
        dump(self.root/f'core-{self.generation}-identity.json',{'MainPID':self.core.pid,'sha256':sha(exe),'exe':os.readlink(exe),'cgroup':Path(f'/proc/{self.core.pid}/cgroup').read_text(),'pg_roles':self.sql("SELECT rolname,rolsuper,rolcreatedb,rolcreaterole,rolbypassrls FROM pg_roles WHERE rolname IN ('lmm_core_owner','lmm_core_runtime','independent_installer')")})
    def stop_core(self,kill=False):
        if self.core and self.core.poll() is None:
            self.core.kill() if kill else self.core.terminate()
            try:self.core.wait(timeout=8)
            except subprocess.TimeoutExpired:self.core.kill();self.core.wait(timeout=3)
    def oracle(self,result,groups=('allowed','秘密组')):
        cov=result['coverage'];a=result['data']['aggregate'];rows=self.rows()
        samples=[r['detail'] for r in rows if r['group'] in groups and (result['data'].get('model_name') is None or r['model']==result['data']['model_name']) and r['detail'] and cov['window_start']<=r['detail']['finished_at']<cov['window_end_exclusive']]
        require(a['request_count']==len(samples),f'count oracle mismatch: {a}, {samples}')
        require(a['success_count']==sum(s['success'] for s in samples),'success count oracle')
        elapsed=sum(s['elapsed_us'] for s in samples);first=[s['first_output_us'] for s in samples if s['first_output_us'] is not None]
        through=[s for s in samples if s['success'] and s['output_tokens'] is not None and s['elapsed_us']>0]
        outputs=sum(s['output_tokens'] for s in through);duration=sum(s['elapsed_us'] for s in through)
        expected={'latency_sum_us':str(elapsed),'ttft_sum_us':str(sum(first)),'ttft_count':len(first),'output_tokens':str(outputs),'throughput_time_us':str(duration),'throughput_count':len(through)}
        for k,v in expected.items():require(a[k]==v,f'aggregate {k}: {a[k]} != {v}')
        for k,v in {'avg_latency_ms':elapsed/len(samples)/1000 if samples else None,'avg_ttft_ms':sum(first)/len(first)/1000 if first else None,'avg_tps':outputs/duration*1e6 if duration else None,'success_rate':sum(s['success'] for s in samples)/len(samples)*100 if samples else None}.items():
            require(a[k] is None if v is None else math.isclose(a[k],v,rel_tol=1e-9),f'weighted {k}: {a[k]} != {v}')
        require(cov['sampled_requests']==len(samples),'coverage count')
        require(result['availability']=='partial' and cov['historical_imports'] is False and cov['unreserved_http_requests'] is False,'coverage must remain explicit')
        require(cov['window_end_exclusive']-cov['window_start'] in (3600,86400,2592000),'window width')
        return samples
    def case_json(self):
        empty=self.perf();require(empty['data']['aggregate']['request_count']==0,'fresh fixture not empty');self.oracle(empty)
        for delay,tokens in [(0.22,5),(.55,17)]:
            self.delay=delay;self.tokens=tokens;status,body,elapsed=self.completion();require(body['choices'][0]['message']['content']=='hello','actual JSON content')
        rows=self.wait_samples(2);require(len(self.supplier)==2,'JSON dispatch count')
        for r in rows:require(r['detail']['success'] and r['detail']['first_output_us'] is None,'nonstream success/TTFT null')
        require(min(r['detail']['elapsed_us'] for r in rows)>=200000,'JSON delay missing from timing')
        self.oracle(self.perf())
    def case_sse(self):
        self.mode='sse';self.tokens=11
        _,body,_=self.completion(True);require('hello' in body and '[DONE]' in body,'actual SSE content/terminal')
        rows=self.wait_samples(3);s=next(r['detail'] for r in rows if r['detail']['first_output_us'] is not None)
        require(s['success'] and 450000<=s['first_output_us']<=s['elapsed_us'],'TTFT counted pre-content or invalid elapsed')
        require(s['elapsed_us']>=620000,'SSE terminal delay missing')
        self.oracle(self.perf());self.mode='json'
    def case_groups(self):
        self.delay=.25;self.tokens=9;self.completion(key=1);self.wait_samples(4)
        self.completion(model='other-model');self.wait_samples(5)
        allresult=self.perf('?model=local-model',detail=True);self.oracle(allresult)
        require({x['group'] for x in allresult['data']['groups']}=={'allowed','秘密组'},'group fields lost')
        reader=self.perf('?model=local-model',session=self.creds['reader_session'],detail=True);self.oracle(reader,('allowed',))
        require([x['group'] for x in reader['data']['groups']]==['allowed'],'ungranted group disclosed')
        for token in [None,self.creds['keys'][0]['secret']]:
            status,body,_=self.http('/api/perf-metrics/summary',token,label='auth-negative')
            require(status in (401,403),'anonymous/key permitted personal performance')
        require(len(self.supplier)==5,'metrics queries triggered provider')
        self.oracle(self.perf())
    def case_queries(self):
        for q in ['?hours=0','?hours=721','?hours=-1','?hours=x','?bucket_seconds=1','?bucket_seconds=0','?group=allowed','?model=','?model='+('x'*257),'?model=%00','?hours=24&hours=1']:
            self.perf(q,expect=400)
        self.perf(detail=True,expect=400)
        for bucket in (60,300,3600):
            for hours in (1,24,720):
                before=time.time();r=self.perf(f'?bucket_seconds={bucket}&hours={hours}&model=local-model',detail=True);after=time.time()
                require(int(before)+1<=r['coverage']['window_end_exclusive']<=int(after)+1,'UTC next-second end')
                require(r['coverage']['window_end_exclusive']-r['coverage']['window_start']==hours*3600,'hours width')
                self.oracle(r)
                for g in r['data']['groups']:
                    for b in g['series']:require(b['ts']%bucket==0,'UTC bucket alignment')
        r=self.perf('?model=nonexistent',detail=True);require(r['data']['aggregate']['request_count']==0,'unknown model fabricated')
    def case_failure(self):
        self.mode='fail-finalize';before=len(self.supplier)
        _,body,_=self.completion(expect=503)
        require(body['error']['code']=='settlement_pending','failure must retain explicit pending settlement')
        rows=self.wait_samples(6)
        failed=[r for r in rows if not r['detail']['success']]
        require(len(failed)==1 and failed[0]['state']!='settled','terminal finalization failure misclassified successful')
        require(len(self.supplier)==before+1,'failure redispatched provider')
        self.oracle(self.perf());self.sql('DROP TRIGGER independent_fail ON core_charging.requests; DROP FUNCTION public.independent_fail()');self.mode='json'
    def case_restart(self):
        before=self.rows();count=len(self.supplier);self.stop_core();self.start_core()
        after=self.rows();require({r['id']:r['detail'] for r in before}=={r['id']:r['detail'] for r in after},'restart altered samples')
        require(len(self.supplier)==count,'restart redispatched supplier');self.oracle(self.perf())
        require(self.sql("SELECT count(*)=count(DISTINCT request_id) FROM core_charging.usage_events WHERE event_id='performance-v1'")=='t','duplicate samples')
    def case_missing(self):
        self.mode='crash'; self.checkpoint.clear();self.release.clear();errors=[]
        def client():
            try:self.completion(True)
            except Exception as e:errors.append(repr(e))
        th=threading.Thread(target=client);th.start();require(self.checkpoint.wait(8),'crash checkpoint not reached')
        self.stop_core(kill=True);self.release.set();th.join(10);require(not th.is_alive(),'crash client failed to terminate')
        dump(self.root/'expected-crash-client.json',errors);self.mode='json';self.start_core()
        result=self.perf();self.oracle(result)
        missing=[r for r in self.rows() if r['detail'] is None]
        require(len(missing)==1 and result['coverage']['missing_samples']==1,'process-death sample fabricated or omitted from missing count')
    def case_hidden(self):
        cfg=json.loads(self.sql('SELECT document FROM core_abuse.settings WHERE singleton'))
        policy={'id':'independent-hidden','name':'Independent synthetic hidden catalog','mode':'fixed_text','fixed_text':'synthetic','public_models':['other-model'],'clients':{},'limits':{'requests_per_minute_per_user':10,'requests_per_minute_global':10,'max_concurrent_per_user':1,'max_concurrent_global':1,'max_input_bytes':1024,'max_output_bytes':1024,'max_output_tokens':20,'output_bytes_per_second':1024,'total_timeout_seconds':3}}
        new=json.dumps({'version':1,'policies':[policy]}).replace("'","''")
        self.sql("UPDATE core_abuse.settings SET revision=revision+1,document='"+new+"'::jsonb WHERE singleton; INSERT INTO core_abuse.user_marks(user_id,marked,policy_id,revision) VALUES(8,true,'independent-hidden',1) ON CONFLICT(user_id) DO UPDATE SET marked=true,policy_id='independent-hidden',revision=core_abuse.user_marks.revision+1")
        r=self.perf(session=self.creds['reader_session']);require([m['model_name'] for m in r['data']['models']]==['other-model'] and r['data']['aggregate']['request_count']==1,'hidden model disclosed or allowed model lost')
        d=self.perf('?model=local-model',session=self.creds['reader_session'],detail=True);require(d['data']['groups']==[],'detail disclosed hidden model')
    def execute(self):
        success=False
        try:
            self.prepare()
            for name,fn in zip(CASES,[self.case_json,self.case_sse,self.case_groups,self.case_queries,self.case_failure,self.case_restart,self.case_missing,self.case_hidden]):
                self.results[name]='RUNNING';dump(self.root/'results.json',self.results)
                fn();self.results[name]='PASS';dump(self.root/'results.json',self.results)
            dump(self.root/'rows-final.json',self.rows())
            success=True
        except Exception:
            (self.root/'failure.txt').write_text(traceback.format_exc())
            for k in self.results:
                if self.results[k]=='RUNNING':self.results[k]='FAIL'
            dump(self.root/'results.json',self.results)
        finally:
            self.stop_core();self.release.set()
            if self.provider:self.provider.shutdown();self.provider.server_close()
            if self.pg:
                self.run('pg-stop',['pg_ctl','-D',self.root/'pg','-m','fast','-w','stop'],check=False)
                require(not (self.root/'pg/postmaster.pid').exists(),'PG still active')
            dump(self.root/'stopped.json',{'core_exited':self.core is None or self.core.poll() is not None,'pg_stopped':not (self.root/'pg/postmaster.pid').exists()})
        if success:
            removed={}
            for name in ['pg','s','lmm-core','fixture','owner.url','credentials.json','supplier.key','runtime.url']:
                path=self.root/name
                if not path.exists():continue
                removed[name]=sum(p.stat().st_size for p in path.rglob('*') if p.is_file()) if path.is_dir() else path.stat().st_size
                if path.is_dir():shutil.rmtree(path)
                else:path.unlink()
            for path in self.log.glob('*-seed.stdout'):
                digest=sha(path);path.write_text(json.dumps({'synthetic_credentials_removed':True,'original_sha256':digest})+'\n')
            dump(self.root/'cleanup.json',{'logical_bytes_removed':sum(removed.values()),'paths':removed})
        return success

def launch(root):
    require(root.parent==Path('/home/lightjunction/Documents/Codex') and not root.exists(),'fresh direct private root required')
    require(len(str(root/'s/.s.PGSQL.55603').encode())<104,'PG socket path too long')
    require(shutil.disk_usage(root.parent).free>2*1024**3,'insufficient disk')
    require(sha(FREEZE/'lmm-core')==CORE_SHA,'candidate mismatch')
    pins=json.loads((FREEZE/'source-pins.json').read_text());require(all(sha(FREEZE/'source'/n)==h for n,h in pins.items()),'source pin drift')
    root.mkdir(mode=0o700);(root/'runner').mkdir()
    for p in Path(__file__).parent.glob('independent_*'):
        if p.is_file():shutil.copy2(p,root/'runner'/p.name)
    shutil.copy2(FREEZE/'source-pins.json',root/'source-pins.json');shutil.copy2(FREEZE/'compile-config.json',root/'compile-config.json')
    dump(root/'binding.json',{'freeze':str(FREEZE),'core_sha256':CORE_SHA,'rlib_sha256':sha(FREEZE/'liblmm_core.rlib'),'source_pins_sha256':sha(FREEZE/'source-pins.json'),'source_count':len(pins),'source_claim':'All frozen manifest files checked against frozen directory only; no claim of whole current worktree equality. cfg(test) differences must be reported separately.','compile_config_sha256':sha(FREEZE/'compile-config.json'),'runner':{p.name:sha(p) for p in (root/'runner').iterdir()}})
    suffix=secrets.token_hex(4);slice_name='lmmperf_'+suffix+'.slice';scope='lmm-performance-independent-'+suffix
    env={k:v for k,v in os.environ.items() if not k.startswith(('LMM_','PG','SQLX_')) and k not in ('DATABASE_URL','SQL_DSN','HTTP_PROXY','HTTPS_PROXY','ALL_PROXY')}
    env.update(XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}',DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus')
    unit=Path(env['XDG_RUNTIME_DIR'])/'systemd/user'/slice_name
    unit.write_text('[Unit]\nDescription=Independent native performance fixture\n[Slice]\nMemoryAccounting=yes\nMemoryMax=2G\nMemoryHigh=1536M\nMemorySwapMax=4G\nCPUQuota=100%\nTasksMax=256\n')
    cmd=['systemd-run','--user','--scope','--quiet','--unit='+scope,'--slice='+slice_name,'python3','-B',str(root/'runner/independent_run.py'),'--root',str(root),'--freeze',str(FREEZE),'--core-sha',CORE_SHA,'--worker']
    dump(root/'launch.json',{'argv':cmd,'slice':slice_name})
    try:
        subprocess.run(['systemctl','--user','daemon-reload'],env=env,check=True)
        code=subprocess.run(cmd,env=env).returncode
        subprocess.run(['systemctl','--user','show',slice_name,'-p','MemoryPeak','-p','MemoryMax','-p','MemoryHigh','-p','MemorySwapMax','-p','CPUQuotaPerSecUSec','-p','TasksMax'],env=env,stdout=(root/'resources.txt').open('w'))
        subprocess.run(['systemctl','--user','stop',slice_name],env=env,check=True)
    finally:
        unit.unlink(missing_ok=True);subprocess.run(['systemctl','--user','daemon-reload'],env=env)
    dump(root/'launch-result.json',{'exit_code':code,'slice_removed':not unit.exists()})
    dump(root/'hash-manifest.json',{str(p.relative_to(root)):sha(p) for p in root.rglob('*') if p.is_file() and 'pg' not in p.relative_to(root).parts and p.name!='hash-manifest.json'})
    print(json.dumps({'root':str(root),'exit':code,'results':json.loads((root/'results.json').read_text())}));return code

def main():
    global FREEZE,CORE_SHA
    os.umask(0o077);p=argparse.ArgumentParser();p.add_argument('--root',required=True,type=Path);p.add_argument('--freeze',required=True,type=Path);p.add_argument('--core-sha',required=True);p.add_argument('--worker',action='store_true');a=p.parse_args()
    FREEZE=a.freeze.absolute();CORE_SHA=a.core_sha
    require(CORE_SHA!='dfc6034d51ac414030d98c6d2fa0e81dfe19b2b227d43020d31e0837c78f3143','R1 withdrawn: known serializable observation write defect')
    if not a.worker:return launch(a.root.absolute())
    with open('/tmp/lmm-delegate/heavy-build.lock','a') as lock:
        (a.root/'waiting-for-lock').write_text(str(os.getpid()));fcntl.flock(lock,fcntl.LOCK_EX)
        (a.root/'waiting-for-lock').unlink()
        info=Path('/proc/meminfo').read_text();(a.root/'memory-admission.txt').write_text(info)
        available=int(next(l.split()[1] for l in info.splitlines() if l.startswith('MemAvailable:')))
        require(available>=2097152,'2GiB memory admission failed')
        return 0 if Run(a.root).execute() else 1
if __name__=='__main__':sys.exit(main())
