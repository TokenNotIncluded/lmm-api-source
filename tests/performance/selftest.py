#!/usr/bin/env python3
"""Developer check: actual Core, owner-created synthetic funds and delayed TCP provider."""
import argparse
import fcntl
import hashlib
import http.client
import importlib.util
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--freeze', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    args.output.mkdir(mode=0o700)
    root = args.output
    lock = open('/tmp/lmm-delegate/heavy-build.lock', 'a')
    fcntl.flock(lock, fcntl.LOCK_EX)
    helper = args.source/'tests/upgrade_offline_independent/runtime_run.py'
    spec = importlib.util.spec_from_file_location('pg_mechanics', helper)
    mechanics = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mechanics)
    fixture = mechanics.Acceptance(argparse.Namespace(root=root))
    fixture.source = args.freeze/'source'
    core = args.freeze/'lmm-core'
    binding = {'core_sha256': sha(core), 'pg_mechanics_sha256': sha(helper),
               'runner_sha256': sha(__file__), 'source_pins_sha256': sha(args.freeze/'source-pins.json')}
    fixture.save('binding.json', binding)
    process = server = None
    hits = []
    result = {'status': 'NOT_RUN', 'scope': 'developer actual Native/PG/TCP performance; synthetic funds only'}
    try:
        fixture.target = fixture.cluster('target')
        fixture.sql('CREATE DATABASE coretarget', db='postgres')
        fixture.run('roles', ['psql', '-Xq', '-v', 'ON_ERROR_STOP=1', '-h', fixture.target['socket'],
                    '-p', '55601', '-U', 'postgres', '-d', 'coretarget', '-f', fixture.source/'deployment/postgres/create-core-roles.sql'])
        fixture.sql('ALTER DATABASE coretarget OWNER TO lmm_core_owner')
        url = fixture.secret_file('owner.url', fixture.url(fixture.target, 'postgres', 'coretarget', owner=True))
        seed = root/'seed.rs'
        seed.write_text('''#[path = "'''+str(fixture.source/'services/core/tests/common/model_fixture.rs')+'''"]
mod fixture;
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pool=sqlx::PgPool::connect(&std::fs::read_to_string(&args[1]).unwrap()).await.unwrap();
    let (identity,_,session,key,key_id,account)=fixture::fixture(&pool,1_000_000).await;
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES (7,'allowed','synthetic-performance-fixture')").execute(&pool).await.unwrap();
    let outsider=identity.bootstrap_user(8,1).await.unwrap();
    std::fs::write(&args[2],serde_json::to_vec(&serde_json::json!({"session":session,"key":key,"key_id":key_id,"account":account,"outsider_session":outsider.secret})).unwrap()).unwrap();
}
''')
        cfg = json.loads((args.freeze/'compile-config.json').read_text())
        fixture.run('compile-seed', [cfg['compiler'], '--edition=2024', '--crate-name', 'performance_seed', seed,
                    *cfg['search'], *cfg['externs'], *cfg['link'], '--extern', 'lmm_core='+str(args.freeze/'liblmm_core.rlib'), '-o', root/'seed'], timeout=300)
        credentials = root/'credentials.json'
        fixture.run('seed-owned-synthetic-data', [root/'seed', url, credentials], timeout=90)
        credentials = json.loads(credentials.read_text())
        fixture.run('grants', ['psql', '-Xq', '-v', 'ON_ERROR_STOP=1', '-h', fixture.target['socket'], '-p', '55601',
                    '-U', 'postgres', '-d', 'coretarget', '-v', 'enable_models=true', '-v', 'enable_console=true',
                    '-f', fixture.source/'deployment/postgres/grant-core-runtime.sql'])

        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                value = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                assert self.headers['Authorization'] == 'Bearer synthetic-upstream-key'
                assert value['model'] == 'provider-model'
                mode = value['messages'][0]['content']
                hits.append({'mode': mode, 'at': time.monotonic()})
                if value.get('stream'):
                    frame = lambda v: ('data: '+json.dumps(v)+'\n\n').encode()
                    start = frame({'id':'local','choices':[{'index':0,'delta':{'role':'assistant','content':''},'finish_reason':None}]})
                    content = frame({'id':'local','choices':[{'index':0,'delta':{'content':'measured output'},'finish_reason':None}]})
                    finish = frame({'id':'local','choices':[{'index':0,'delta':{},'finish_reason':'stop'}]})
                    usage = frame({'id':'local','choices':[],'usage':{'prompt_tokens':10,'completion_tokens':20,'prompt_tokens_details':{'cached_tokens':2}}})
                    chunks = [b': heartbeat\n\n'+start, content, finish+usage+(b'' if mode=='truncate' else b'data: [DONE]\n\n')]
                    self.send_response(200); self.send_header('Content-Type','text/event-stream')
                    self.send_header('Content-Length',str(sum(map(len,chunks)))); self.end_headers()
                    for chunk in chunks:
                        time.sleep(.06); self.wfile.write(chunk); self.wfile.flush()
                else:
                    time.sleep(.08)
                    data = json.dumps({'id':'local','object':'chat.completion','model':'provider-model',
                        'choices':[{'index':0,'message':{'role':'assistant','content':'measured JSON'},'finish_reason':'stop'}],
                        'usage':{'prompt_tokens':10,'completion_tokens':20,'prompt_tokens_details':{'cached_tokens':2}}}).encode()
                    self.send_response(200); self.send_header('Content-Type','application/json')
                    self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)

        server = ThreadingHTTPServer(('127.0.0.1',0),Provider)
        thread = threading.Thread(target=server.serve_forever,daemon=True); thread.start()
        token = fixture.secret_file('upstream.token','synthetic-upstream-key')
        model = fixture.secret_file('model.toml',f'''version=1
input_token_ceiling=100
lease_seconds=20
timeout_seconds=5
max_output_tokens=20
local_streaming=true
credentials=[{{reference={{id=7,revision=1}},secret_file={json.dumps(str(token))}}}]
key_groups=[{{key_id={credentials['key_id']},group='allowed'}}]
[routing]
version=1
groups=[{{name='allowed',enabled=true}}]
upstreams=[{{id=1,endpoint='http://127.0.0.1:{server.server_port}/custom/v1',protocol='openai_chat',credential={{id=7,revision=1}},enabled=true}}]
routes=[{{id=1,model='local-model',group='allowed',enabled=true,speeds=['standard'],targets=[{{id=1,upstream_id=1,upstream_model='provider-model',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}}],failover={{max_attempts=1,retry_on=[],allow_replay_after_send=false}}}}]
aliases=[]
[prices]
version=1
rules=[{{id=1,model='local-model',group='allowed',speed='standard',rates={{input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0}},group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000}}]
''')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1',0)); fixture.port=sock.getsockname()[1]
        runtime = fixture.secret_file('runtime.url',fixture.url(fixture.target,'lmm_core_runtime','coretarget'))
        config = fixture.secret_file('runtime.toml',f'''version=1
[server]
listen='127.0.0.1:{fixture.port}'
worker_threads=2
max_inflight=16
drain_timeout_seconds=3
[database]
url_file={json.dumps(str(runtime))}
max_connections=4
[console]
enabled=true
[model]
config_file={json.dumps(str(model))}
local_http=true
''')
        log = open(root/'actual-core.log','wb')
        def start():
            p = subprocess.Popen([core,'--config',config,'serve'],env=fixture.env,stdout=log,stderr=subprocess.STDOUT)
            fixture.wait_ready(p)
            assert sha(Path('/proc')/str(p.pid)/'exe') == binding['core_sha256']
            return p
        process=start()
        samples=[]
        def model_call(mode,stream):
            conn=http.client.HTTPConnection('127.0.0.1',fixture.port,timeout=15)
            conn.request('POST','/v1/chat/completions',json.dumps({'model':'local-model','messages':[{'role':'user','content':mode}],'max_tokens':20,'stream':stream}),
                         {'Content-Type':'application/json','Authorization':'Bearer '+credentials['key'],'Idempotency-Key':secrets.token_hex(16)})
            response=conn.getresponse();body=response.read();status=response.status;request_id=response.getheader('x-request-id');conn.close()
            assert status==200,(status,body[:300]);return request_id
        for mode,stream in [('json',False),('stream',True),('truncate',True)]:
            request_id=model_call(mode,stream)
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                raw=fixture.scalar("SELECT detail::text FROM core_charging.usage_events WHERE event_id='performance-v1' AND request_id='"+request_id+"'")
                if raw:break
                time.sleep(.05)
            assert raw,'missing measured observation'
            sample=json.loads(raw);assert sample['success']==(mode!='truncate'),sample
            assert sample['elapsed_us']>=60_000,sample
            if stream:assert sample['first_output_us']>=100_000,sample
            else:assert sample['first_output_us'] is None,sample
            samples.append(sample)
        fixture.save('measured-samples.json',samples)
        assert len(hits)==3
        status,summary=fixture.http('GET','/api/perf-metrics/summary',token=credentials['session'])
        assert status==200 and summary['availability']=='partial',summary
        stats=summary['data']['aggregate'];assert stats['request_count']==3 and stats['success_count']==2,summary
        assert abs(stats['success_rate']-200/3)<.00001
        assert abs(stats['avg_latency_ms']-sum(s['elapsed_us'] for s in samples)/3000)<.001
        assert abs(stats['avg_tps']-40_000_000/sum(s['elapsed_us'] for s in samples if s['success']))<.001
        for path in ['/api/perf-metrics?model=local-model&bucket_seconds=60','/api/perf-metrics?model=local-model&bucket_seconds=300']:
            status,value=fixture.http('GET',path,token=credentials['session']);assert status==200 and value['data']['groups'][0]['request_count']==3,value
        for token,expected in [(None,401),(credentials['key'],403)]:
            assert fixture.http('GET','/api/perf-metrics/summary',token=token)[0]==expected
        for query in ['hours=0','hours=721','bucket_seconds=61']:
            assert fixture.http('GET','/api/perf-metrics/summary?'+query,token=credentials['session'])[0]==400
        assert fixture.http('GET','/api/perf-metrics',token=credentials['session'])[0]==400
        outsider=fixture.http('GET','/api/perf-metrics/summary',token=credentials['outsider_session'])
        assert outsider[0]==200 and outsider[1]['data']['models']==[],outsider
        fixture.sql("CREATE FUNCTION public.reject_performance_fixture() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_id='performance-v1' THEN RAISE EXCEPTION 'synthetic telemetry write failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_performance_fixture BEFORE INSERT ON core_charging.usage_events FOR EACH ROW EXECUTE FUNCTION public.reject_performance_fixture()")
        model_call('missing-sample',False);time.sleep(.3)
        value=fixture.http('GET','/api/perf-metrics/summary',token=credentials['session'])[1]
        assert value['coverage']['missing_samples']==1 and value['data']['aggregate']['request_count']==3,value
        fixture.sql('DROP TRIGGER reject_performance_fixture ON core_charging.usage_events; DROP FUNCTION public.reject_performance_fixture()')
        process.terminate();process.wait(timeout=10);process=start()
        value=fixture.http('GET','/api/perf-metrics/summary',token=credentials['session'])[1]
        assert value['coverage']['missing_samples']==1 and value['data']['aggregate']['request_count']==3,value
        assert len(hits)==4,'telemetry failure or restart repeated upstream dispatch'
        fixture.save('final-summary.json',value)
        fixture.sql("UPDATE core_identity.user_routing_groups SET enabled=false WHERE user_id=7")
        assert fixture.http('GET','/api/perf-metrics/summary',token=credentials['session'])[0]==401
        result.update(status='PASS',actual_provider_requests=len(hits),samples=3,missing_samples=1)
    except BaseException as error:
        result.update(status='FAIL',error=type(error).__name__,detail=str(error))
        raise
    finally:
        if process is not None:
            process.terminate()
            try:process.wait(timeout=10)
            except subprocess.TimeoutExpired:process.kill();process.wait(timeout=5)
        if server is not None:server.shutdown();server.server_close()
        for cluster in fixture.clusters:
            if cluster['started']:
                fixture.run('pg-stop',['pg_ctl','-D',cluster['data'],'-m','fast','-w','stop'],expected=None)
        fixture.save('result.json',result)


if __name__=='__main__':
    main()
