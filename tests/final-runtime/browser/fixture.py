#!/usr/bin/env python3
"""Independent real Core/Go/PostgreSQL browser fixture. No business response mocks."""
from pathlib import Path
import argparse, hashlib, http.client, http.server, importlib.util, json, os, secrets, shutil, signal, subprocess, sys, tempfile, threading, time, traceback
from unittest.mock import patch
from urllib.parse import urlparse
os.umask(0o077)
sys.dont_write_bytecode=True
ROOT=Path(os.environ['LMM_ACCEPTANCE_ROOT'])
SOURCE=ROOT.parent/'lmm-api-source-main'
LMM_ORIGIN='https://native.api.lmm.best'
EXTORE_ORIGIN='https://extore.lmm.best'
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
def save(path,value):
 path.write_text(json.dumps(value,ensure_ascii=False,sort_keys=True,indent=2)+'\n');path.chmod(0o600)
parser=argparse.ArgumentParser()
parser.add_argument('--round',required=True)
parser.add_argument('--candidate',type=Path,required=True)
parser.add_argument('--max-seconds',type=int,default=1800)
args=parser.parse_args()
assert args.round.startswith('browser-') and '/' not in args.round and '..' not in args.round
assert 120 <= args.max_seconds <= 1800
started_at=time.time();hard_deadline=started_at+args.max_seconds
roundroot=ROOT/args.round;roundroot.mkdir(mode=0o700)
try:
 candidate_path=args.candidate.resolve(strict=True);candidate=json.loads(candidate_path.read_text())
 SOURCE=Path(candidate['source']).resolve(strict=True)
 SOURCE_REPOSITORY=Path(candidate['source_repository']).resolve(strict=True)
 SOURCE_COMMIT=candidate['source_commit']
 assert len(SOURCE_COMMIT)==40 and all(c in '0123456789abcdef' for c in SOURCE_COMMIT)
 CORE_COMMIT=candidate['core']['source_commit'];WEB_COMMIT=candidate['web']['source_commit']
 assert CORE_COMMIT==SOURCE_COMMIT,'new Core and Go must share the published session-binding candidate'
 assert CORE_COMMIT==WEB_COMMIT==SOURCE_COMMIT=='293e53f799b09f187893e9cdf0c3503973916676'
 assert candidate['extensions']['source_commit']==SOURCE_COMMIT
 EXTORE_COMMIT=candidate['extore']['source_commit']
 assert EXTORE_COMMIT=='8e3063690cf5595884b1e639535869fd593e0296'
 assert len(EXTORE_COMMIT)==40 and candidate['extore_host']['extore_source_commit']==EXTORE_COMMIT
 EXT=Path(candidate['extore']['binary']['path']).resolve(strict=True);EXT_SHA=candidate['extore']['binary']['sha256']
 PLUGIN=Path(candidate['extore']['library']['path']).resolve(strict=True);PLUGIN_SHA=candidate['extore']['library']['sha256']
 HOST=Path(candidate['extore_host']['path']).resolve(strict=True);HOST_SHA=candidate['extore_host']['sha256']
 helper=Path(candidate['extore']['helper']['path']).resolve(strict=True);HELPER_SHA=candidate['extore']['helper']['sha256']
 STATIC=Path(candidate['extore']['static']['root']).resolve(strict=True)
 static_manifest=Path(candidate['extore']['static']['binding']).resolve(strict=True);STATIC_MANIFEST_SHA=candidate['extore']['static']['binding_sha256']
 assert sha(static_manifest)==STATIC_MANIFEST_SHA
 static_binding=json.loads(static_manifest.read_text());assert static_binding['source_commit']==EXTORE_COMMIT
 assert set(static_binding['files'])=={str(p.relative_to(STATIC)) for p in STATIC.rglob('*') if p.is_file()}
 for name,want in static_binding['files'].items():
  target=(STATIC/name).resolve(strict=True);assert target.is_relative_to(STATIC) and sha(target)==want,name
 for p,digest in [(EXT,EXT_SHA),(PLUGIN,PLUGIN_SHA),(HOST,HOST_SHA),(helper,HELPER_SHA)]:assert sha(p)==digest,'feature artifact input changed: '+str(p)
 expected_helper=subprocess.run(['git','-C',candidate['extore']['source_repository'],'show',EXTORE_COMMIT+':scripts/native_plugin_fixture.py'],capture_output=True,check=True).stdout
 assert helper.read_bytes()==expected_helper,'Extore helper differs from frozen source'
 extore_spec=importlib.util.spec_from_file_location('real_extore_fixture',helper)
 extore_module=importlib.util.module_from_spec(extore_spec);extore_spec.loader.exec_module(extore_module)
 from https_fixture_transport import LocalHTTPS, certificates
 transport_source=Path(__file__).with_name('https_fixture_transport.py');TRANSPORT_SHA=sha(transport_source)
 host_manifest=subprocess.run(['git','-C',str(SOURCE_REPOSITORY),'show',SOURCE_COMMIT+':services/extensions/extore-host/Cargo.toml'],capture_output=True,check=True).stdout
 import tomllib
 host_api=tomllib.loads(host_manifest.decode())['dependencies']['extore-plugin-api']
 assert host_api['path']=='../../../third_party/extore/crates/extore-plugin-api'
 gitlink=subprocess.run(['git','-C',str(SOURCE_REPOSITORY),'ls-tree',SOURCE_COMMIT,'third_party/extore'],capture_output=True,check=True,text=True).stdout.strip().split()
 assert gitlink==['160000','commit',EXTORE_COMMIT,'third_party/extore'],'host API gitlink differs from plugin'
 CORE=Path(candidate['core']['path']).resolve(strict=True);CORE_SHA=candidate['core']['sha256']
 GO=Path(candidate['extensions']['path']).resolve(strict=True);GO_SHA=candidate['extensions']['sha256']
 DIST=Path(candidate['web']['dist']).resolve(strict=True)
 binding_path=Path(candidate['web']['binding']).resolve(strict=True)
 assert sha(CORE)==CORE_SHA and sha(GO)==GO_SHA
 assert sha(binding_path)==candidate['web']['binding_sha256']
 binding=json.loads(binding_path.read_text());assert binding['source_commit']==WEB_COMMIT
 assert set(binding['files'])=={str(p.relative_to(DIST)) for p in DIST.rglob('*') if p.is_file()}
 for name,want in binding['files'].items():
  p=(DIST/name).resolve(strict=True);assert p.is_relative_to(DIST) and sha(p)==want,name
 required=['deployment/postgres/grant-core-runtime.sql', 'deployment/postgres/verify-channel-observations-owner.sql', 'services/extensions/internal/modules/store/schema.sql', 'tests/console/independent_process/run.py', 'tests/console/independent_process/pg_fault.py']
 source_hashes={}
 for rel in required:
  p=(SOURCE/rel).resolve(strict=True);assert p.is_relative_to(SOURCE)
  expected=subprocess.run(['git','-C',str(SOURCE_REPOSITORY),'show',f'{SOURCE_COMMIT}:{rel}'],check=True,capture_output=True).stdout
  if rel in candidate['fixture_overlays']:
   overlay=candidate['fixture_overlays'][rel]
   assert overlay['test_harness_commit']=='efd2248548b6744785bf85f5b571b8b19c00de4b'
   assert hashlib.sha256(expected).hexdigest()==overlay['before_sha256']
   actual=p.read_bytes();assert hashlib.sha256(actual).hexdigest()==overlay['after_sha256']
   assert hashlib.sha1(b'blob '+str(len(actual)).encode()+b'\0'+actual).hexdigest()==overlay['git_blob']
  else:assert p.read_bytes()==expected,'frozen source does not match commit: '+rel
  if rel.startswith(('deployment/postgres/','tests/console/')):
   core_expected=subprocess.run(['git','-C',str(SOURCE_REPOSITORY),'show',f'{CORE_COMMIT}:{rel}'],check=True,capture_output=True).stdout
   assert expected==core_expected,'Core fixture input differs from the matched candidate: '+rel
  source_hashes[rel]=sha(p)
 save(roundroot/'inputs.json',{'core_sha256':CORE_SHA,'go_sha256':GO_SHA,'web_dist_binding_sha256':sha(binding_path),'web_dist_index_sha256':binding['files']['index.html'],'web_source_commit':WEB_COMMIT,'core_source_commit':CORE_COMMIT,'feature_source_commit':SOURCE_COMMIT,'extore_source_commit':EXTORE_COMMIT,'extore_sha256':EXT_SHA,'extore_host_sha256':HOST_SHA,'extore_plugin_sha256':PLUGIN_SHA,'extore_helper_sha256':HELPER_SHA,'extore_static_manifest_sha256':STATIC_MANIFEST_SHA,'extore_static_file_count':len(static_binding['files']),'transport_source_sha256':TRANSPORT_SHA,'web_embedded_revision':binding['embedded_build_revision'],'fixture_runner_sha256':sha(__file__),'candidate_manifest_sha256':sha(candidate_path),'frozen_source_hashes':source_hashes,'artifact_lineage':{'validation_class':candidate['validation_class'],'core_profile':candidate['core']['profile'],'extore_compiled_source_commit':candidate['extore']['binary']['compiled_source_commit'],'extore_host_compiled_source_commit':candidate['extore_host']['source_commit'],'extore_host_compiled_extore_source_commit':candidate['extore_host']['compiled_extore_source_commit']},'community_module':'enabled with official install-db, isolated DB/roles, and real /api/site/ Go routing','business_response_mocks':False})
 assert candidate['fixture_overlays']=={},'293 includes the original approved TCP_NODELAY fixture fix'
 recorded=json.loads((roundroot/'inputs.json').read_text());recorded['fixture_overlays']=candidate['fixture_overlays'];recorded['test_harness_commit']=candidate['test_harness_commit'];save(roundroot/'inputs.json',recorded)
 fixture_source=SOURCE/'tests/console/independent_process';sys.dont_write_bytecode=True
 sys.path.insert(0,str(fixture_source))
 spec=importlib.util.spec_from_file_location('independent_console_fixture',fixture_source/'run.py')
 base=importlib.util.module_from_spec(spec);spec.loader.exec_module(base)
except Exception as e:
 save(roundroot/'fixture-preflight-failure.json',{'failure':type(e).__name__+': '+str(e),'traceback':traceback.format_exc()});raise
frozen_source=SOURCE
fargs=argparse.Namespace(source=frozen_source,core=CORE,evidence=roundroot/'fixture-evidence',pins=roundroot/'pins.json',freeze=roundroot/'freeze.json',core_sha256=CORE_SHA)
requests=[];request_lock=threading.Lock();stop=threading.Event()
signal.signal(signal.SIGTERM,lambda *_:stop.set())
signal.signal(signal.SIGINT,lambda *_:stop.set())
servers=[]
class RetainedExtore(extore_module.ExtoreFixture):
 def __init__(self,*args,**kwargs):
  super().__init__(*args,**kwargs)
  self.shop_a='8fced514-e61d-4de3-b043-ae84bdd87831'
  self.shop_b='55b24835-af0c-47eb-a949-58dac1007e9a'
  self.products={self.shop_a:'plugin-product-a',self.shop_b:'plugin-product-b'}
 def _seed(self):
  super()._seed()
  import uuid
  assert str(uuid.UUID(self.shop_a))==self.shop_a and str(uuid.UUID(self.shop_b))==self.shop_b
  with self.connection() as db:
   row=db.execute('SELECT config FROM products WHERE id=?',(self.products[self.shop_a],)).fetchone()
   value=json.loads(row[0])
   value['variants'].append({'id':'professional','name':'Professional','description':'Second real fixture variant','price':'27.5000','currency':'CNY','enabled':True,'attributes':{'color':'green','api_key':'PRIVATE_SECOND_VARIANT_SENTINEL'}})
   db.execute('UPDATE products SET config=? WHERE id=?',(json.dumps(value),self.products[self.shop_a]))
 def __exit__(self,*args):
  temporary=self._temporary
  self._temporary=None
  try:super().__exit__(*args)
  finally:
   if temporary is not None:temporary._finalizer.detach();self._retained_temporary=temporary
class Fixture(base.Runner):
 def command(self,label,argv,**kw):
  if label=='formal_first_owner':
   body=json.loads(kw['data'])
   self.password=self.secret('Pa!'+secrets.token_hex(8))
   body['password']=self.password;kw['data']=json.dumps(body).encode()
  return super().command(label,argv,**kw)
 def grants(self):
  self.command('official_grants',['psql','-X','-q','-v','ON_ERROR_STOP=1','-v','runtime_role=console_runtime','-v','enable_console=true','-v','enable_web_identity=true','-v','enable_commerce=true','-v','enable_models=true','-f',self.source/'deployment/postgres/grant-core-runtime.sql'],env={**self.env,'PGHOST':str(self.sockdir),'PGPORT':'5432','PGUSER':'console_installer','PGDATABASE':'console_acceptance','PGOPTIONS':'-c role=lmm_core_owner','PGSSLMODE':'disable'})
 def start_core(self,label):
  assert sha(CORE)==CORE_SHA,'core artifact drift'
  text=self.config.read_text().replace(f'origin="http://127.0.0.1:{self.port}"',f'origin="{LMM_ORIGIN}"').replace('local_http=true','local_http=false')
  if '[rpc]' not in text:
   self.control=self.file('identity.token',secrets.token_hex(24))
   self.money=self.file('commerce.token',secrets.token_hex(24))
   policy=self.file('commerce.toml','version=1\npay=true\nsettle=true\nrefund=true\nread=true\ncomplete_delivery=true\nreconcile=true\n')
   apikey=self.file('api-key.key',secrets.token_bytes(32))
   text=text.replace('[identity]\n',f'[identity]\napi_key_encryption_key_file={json.dumps(str(apikey))}\n')
   self.socket=self.sockdir/'core.sock'
   upstream_key=self.file('upstream.token','synthetic-browser-upstream-key')
   channel_secret=self.file('channel-secret.key',secrets.token_bytes(32))
   model=self.file('model.toml',f"""version=1
channel_secret_key_file={json.dumps(str(channel_secret))}
input_token_ceiling=100
lease_seconds=20
timeout_seconds=3
max_output_tokens=20
local_streaming=false
credentials=[{{reference={{id=7,revision=1}},secret_file={json.dumps(str(upstream_key))}}}]
key_groups=[]
external_models=[{{upstream_id=1,upstream_model="provider-model",protocol="openai_chat",endpoint="https://baseline-channel-fixture.invalid/v1/chat/completions",max_input_tokens=100,max_output_tokens=20,contract_revision="independent-browser-baseline-v1"}}]
[routing]
version=1
groups=[{{name='allowed',enabled=true}}]
upstreams=[{{id=1,endpoint='https://baseline-channel-fixture.invalid/v1/chat/completions',protocol='openai_chat',credential={{id=7,revision=1}},enabled=true}}]
routes=[{{id=1,model='local-model',group='allowed',enabled=true,speeds=['standard'],targets=[{{id=1,upstream_id=1,upstream_model='provider-model',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}}],failover={{max_attempts=1,retry_on=[],allow_replay_after_send=false}}}}]
aliases=[]
[prices]
version=1
rules=[{{id=1,model='local-model',group='allowed',speed='standard',rates={{input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0}},group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000}}]
""")
   text+=f'\n[rpc]\nsocket={json.dumps(str(self.socket))}\ntoken_file={json.dumps(str(self.control))}\n[commerce]\nconfig_file={json.dumps(str(policy))}\ntoken_file={json.dumps(str(self.money))}\n[model]\nconfig_file={json.dumps(str(model))}\nlocal_http=false\n'
  self.config.write_text(text)
  super().start_core(label)
  ready_path=roundroot/'fixture-ready.json'
  if ready_path.exists():
   ready=json.loads(ready_path.read_text())
   ready['core_pid']=self.main.proc.pid
   ready['core_restarts']=ready.get('core_restarts',0)+1
   save(ready_path,ready)

 def start_go(self,label):
  assert sha(GO)==GO_SHA,'extensions artifact drift'
  self.go=base.Process(self,label,[GO,'--config',self.go_config])
  deadline=time.monotonic()+30
  while time.monotonic()<deadline:
   assert self.go.proc.poll() is None,'go exited'
   try:
    c=http.client.HTTPConnection('127.0.0.1',self.goport,timeout=2);c.request('GET','/health/ready');r=c.getresponse();body=json.loads(r.read());c.close()
    if r.status==200:
     assert sha(f'/proc/{self.go.proc.pid}/exe')==GO_SHA
     self.events.append({'go_pid':self.go.proc.pid,'exe_sha256':GO_SHA,'ready':body});return
   except (OSError,http.client.HTTPException,ValueError):pass
   time.sleep(.1)
  raise RuntimeError('go_readiness_timeout')
f=Fixture(fargs)
f.env['PATH']=os.environ['LMM_ACCEPTANCE_PG_BIN']+':/usr/bin:/bin'
f.webport=base.free_port();f.goport=base.free_port()
control=secrets.token_hex(24)
temp_root=Path(tempfile.mkdtemp(prefix='lmm-pab-'));temp_root.chmod(0o700)
(temp_root/'chromium').mkdir(mode=0o700)
f.env['TMPDIR']=str(temp_root)
def capture_resources(label):
 assert isinstance(label,str) and label.replace('-','').isalnum() and len(label)<60
 mem={}
 for line in Path('/proc/meminfo').read_text().splitlines():
  k,v=line.split(':',1)
  if k in ('MemTotal','MemAvailable','SwapTotal','SwapFree'):mem[k]=v.strip()
 processes=[]
 for label_name,process_pid in [('core',f.main.proc.pid),('go',f.go.proc.pid),('postgres',f.pg.proc.pid),('extore',extore_fixture._server.pid),('host',host_pid)]:
  p=Path('/proc')/str(process_pid)
  row={'label':label_name,'pid':process_pid,'alive':p.exists()}
  if p.exists():
   try:
    fields=dict(line.split(':',1) for line in (p/'status').read_text().splitlines() if ':' in line)
    row.update({k:fields.get(k,'').strip() for k in ['State','VmRSS','VmSwap','Threads']})
    row['fd_count']=len(list((p/'fd').iterdir()))
    row['start_ticks']=(p/'stat').read_text().rsplit(') ',1)[1].split()[19]
   except FileNotFoundError:row['alive']=False
  processes.append(row)
 pressures={}
 for name in ['cpu','memory','io']:
  p=Path('/proc/pressure')/name
  if p.exists():pressures[name]=p.read_text().strip()
 value={'label':label,'observed_at_epoch':time.time(),'memory':mem,'loadavg':Path('/proc/loadavg').read_text().strip(),'pressure':pressures,'disk_free_bytes':shutil.disk_usage(ROOT).free,'processes':processes}
 save(roundroot/('resources-'+label+'.json'),value)
 return value

def capture_state(label):
 assert isinstance(label,str) and label.replace('-','').isalnum() and len(label)<60
 query="""SELECT json_build_object(
 'flows',COALESCE((SELECT json_agg(json_build_object('user_id',body->'user_id','consumed',body->'consumed','expires_at',body->'expires_at')) FROM lmm_store.extore_flows),'[]'::json),
 'connections',COALESCE((SELECT json_agg(json_build_object('record_id',id,'local_shop',shop_id,'origin',body->>'origin','upstream_shop',body->>'shop_id','client_id',body->>'client_id','revoked_at',body->'revoked_at','version',body->'version','credential_is_encrypted',(body->>'ciphertext') LIKE 'v1:%')) FROM lmm_store.extore_connections),'[]'::json),
 'sources',COALESCE((SELECT json_agg(json_build_object('id',id,'local_shop',shop_id,'connection_id',body->>'connection_id','source_id',body->>'source_id','product_id',body->>'product_id','revision',body->>'revision','variant_ids',body->'variant_ids','last_synced_at',body->'last_synced_at')) FROM lmm_store.extore_products),'[]'::json),
 'products',COALESCE((SELECT json_agg(json_build_object('id',id,'shop_id',shop_id,'name',body->>'name','enabled',body->'enabled')) FROM lmm_store.products),'[]'::json),
 'variants',COALESCE((SELECT json_agg(json_build_object('id',id,'shop_id',shop_id,'product_id',body->>'product_id','name',body->>'name','price',body->'price','stock',body->'stock')) FROM lmm_store.variants),'[]'::json),
 'orders_count',(SELECT count(*) FROM lmm_store.orders),
 'money_operations_count',(SELECT count(*) FROM lmm_store.money_operations))"""
 raw=f.command('store_snapshot_'+label,['psql','-X','-q','-t','-A','-v','ON_ERROR_STOP=1','-h',f.sockdir,'-U','fixture_admin','-d','lmm_store_browser','-c',query])
 store=json.loads(raw)
 with extore_fixture.connection() as db:
  cursor=db.execute('SELECT id,client_id,shop_id,scopes,product_ids,revoked,last_used,code_used,redirect_uri FROM commerce_grants ORDER BY id')
  names=[x[0] for x in cursor.description];grants=[dict(zip(names,row)) for row in cursor]
  cursor=db.execute('SELECT grant_id,kind,count(*) AS count,count(used) AS used FROM commerce_tokens GROUP BY grant_id,kind ORDER BY grant_id,kind')
  names=[x[0] for x in cursor.description];tokens=[dict(zip(names,row)) for row in cursor]
  cursor=db.execute('SELECT grant_id,kind,created,expires,used IS NOT NULL AS used FROM commerce_tokens ORDER BY created,kind')
  names=[x[0] for x in cursor.description];expiry_metadata=[dict(zip(names,row)) for row in cursor]
  issuances=db.execute('SELECT count(*) FROM commerce_issuances').fetchone()[0]
  products=[]
  for pid,shop,raw_config in db.execute('SELECT id,shop_id,config FROM products ORDER BY id'):
   value=json.loads(raw_config)
   products.append({'id':pid,'shop_id':shop,'name':value['name'],'variants':[{k:v[k] for k in ['id','name','price','currency','enabled']} for v in value['variants']]})
 gateway=[e for e in transport.events if e.get('layer')=='gateway' and e.get('host')=='extore.lmm.best']
 native_maps=[line for line in Path(f'/proc/{host_pid}/maps').read_text().splitlines() if str(plugin_copy) in line]
 save(roundroot/('state-'+label+'.json'),{'label':label,'time':time.time(),'store':store,'extore':{'grants':grants,'token_counts':tokens,'token_expiry_metadata':expiry_metadata,'issuances_count':issuances,'products':products},'native_host':{'pid':host_pid,'exe_sha256':sha(f'/proc/{host_pid}/exe'),'plugin_sha256':sha(plugin_copy),'library_maps':native_maps},'upstream_http':gateway,'catalog_http_requests':sum(e['path']=='/api/integrations/commerce/products' for e in gateway)})

class Web(http.server.SimpleHTTPRequestHandler):
 def __init__(self,*a,**kw):super().__init__(*a,directory=str(DIST),**kw)
 def log_message(self,*_):pass
 def do_GET(self):
  if self.path.startswith(('/api/','/core/','/health/','/v1/')):self.proxy()
  else:
   if not Path(self.translate_path(self.path)).is_file():self.path='/index.html'
   super().do_GET()
 def do_POST(self):
  if self.path=='/__acceptance/control':
   if self.headers.get('X-Acceptance-Key')!=control:self.send_error(403);return
   payload=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))))
   action=payload['action']
   if action=='stop-core':f.main.stop()
   elif action=='start-core':f.start_core('browser_restart_core')
   elif action=='stop':stop.set()
   elif action=='snapshot':capture_state(payload['label'])
   elif action=='resources':capture_resources(payload['label'])
   else:self.send_error(400);return
   raw=json.dumps({'action':action,'completed':True}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw);return
  self.proxy()
 def do_PUT(self):self.proxy()
 def do_DELETE(self):self.proxy()
 def do_PATCH(self):self.proxy()
 def proxy(self):
  request_started=time.time()
  body=self.rfile.read(int(self.headers.get('Content-Length','0')))
  headers={k:v for k,v in self.headers.items() if k.lower() not in ('host','connection','accept-encoding')}
  port=f.goport if self.path.startswith(('/api/store','/api/site/','/extensions/')) else f.port
  conn=http.client.HTTPConnection('127.0.0.1',port,timeout=20)
  try:
   conn.request(self.command,self.path,body,headers);reply=conn.getresponse();raw=reply.read()
   status=reply.status;out_headers=reply.getheaders()
  except (OSError,http.client.HTTPException):
   status=502;raw=b'Upstream unavailable';out_headers=[('Content-Type','text/plain')]
  finally:conn.close()
  with request_lock:
   requests.append({'method':self.command,'path':self.path.split('?')[0],'status':status,'request_started_at':request_started,'response_received_at':time.time(),'retry_after':next((value for key,value in out_headers if key.lower()=='retry-after'),None),'upstream':'extensions' if port==f.goport else 'core'})
   save(roundroot/'requests.json',requests)
  self.send_response(status)
  for k,v in out_headers:
   if k.lower() not in ('transfer-encoding','connection','content-length'):self.send_header(k,v)
  self.send_header('Content-Length',str(len(raw)));self.end_headers()
  try:self.wfile.write(raw)
  except (BrokenPipeError,ConnectionResetError):pass
failed=None;store_proxy=None;extore_fixture=None;transport=None;host_pid=None;host_copy=None;plugin_copy=None;community_owner_proxy=None;community_proxy=None
try:
 original=tempfile.mkdtemp
 def private_temp(*a,**kw):kw['dir']=str(temp_root);return original(*a,**kw)
 with patch.object(base.tempfile,'mkdtemp',private_temp):f.fresh_cli_and_runtime()
 f.sql('isolate_core_connect','REVOKE CONNECT ON DATABASE console_acceptance FROM PUBLIC; GRANT CONNECT ON DATABASE console_acceptance TO console_runtime,console_installer;')
 f.command('store_roles',['psql','-X','-q','-v','ON_ERROR_STOP=1','-h',f.sockdir,'-U','fixture_admin','-d','postgres'],data=b'CREATE ROLE lmm_store_browser_owner NOLOGIN; CREATE ROLE lmm_store_browser_runtime LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; CREATE DATABASE lmm_store_browser OWNER lmm_store_browser_owner; REVOKE ALL ON DATABASE lmm_store_browser FROM PUBLIC; GRANT CONNECT ON DATABASE lmm_store_browser TO lmm_store_browser_runtime;')
 store_sql="ALTER SCHEMA public OWNER TO lmm_store_browser_owner; REVOKE ALL ON SCHEMA public FROM PUBLIC; SET ROLE lmm_store_browser_owner;\n"+(frozen_source/'services/extensions/internal/modules/store/schema.sql').read_text()+"\nRESET ROLE; GRANT USAGE ON SCHEMA lmm_store TO lmm_store_browser_runtime; GRANT SELECT,INSERT,UPDATE ON ALL TABLES IN SCHEMA lmm_store TO lmm_store_browser_runtime; GRANT DELETE ON lmm_store.traffic_days,lmm_store.traffic_receipts,lmm_store.cart_items,lmm_store.favorites TO lmm_store_browser_runtime;"
 f.command('store_schema',['psql','-X','-q','-v','ON_ERROR_STOP=1','-h',f.sockdir,'-U','fixture_admin','-d','lmm_store_browser'],data=store_sql.encode())
 store_proxy=base.Proxy(f.sockdir/'.s.PGSQL.5432','lmm_store_browser_runtime','lmm_store_browser')
 dburl=f.file('store.url',f'postgresql://lmm_store_browser_runtime:{f.secret(secrets.token_hex(16))}@127.0.0.1:{store_proxy.server_address[1]}/lmm_store_browser?sslmode=disable')
 f.command('community_roles',['psql','-X','-q','-v','ON_ERROR_STOP=1','-h',f.sockdir,'-U','fixture_admin','-d','postgres'],data=b'CREATE ROLE lmm_community_browser_owner NOLOGIN; CREATE ROLE lmm_community_browser_installer LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; CREATE ROLE lmm_community_browser_runtime LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; GRANT lmm_community_browser_owner TO lmm_community_browser_installer; CREATE DATABASE lmm_community_browser OWNER lmm_community_browser_owner; REVOKE ALL ON DATABASE lmm_community_browser FROM PUBLIC; GRANT CONNECT ON DATABASE lmm_community_browser TO lmm_community_browser_installer,lmm_community_browser_runtime;')
 community_owner_proxy=base.Proxy(f.sockdir/'.s.PGSQL.5432','lmm_community_browser_installer','lmm_community_browser')
 community_owner_url=f.file('community-owner.url',f'postgresql://lmm_community_browser_installer:{f.secret(secrets.token_hex(16))}@127.0.0.1:{community_owner_proxy.server_address[1]}/lmm_community_browser?sslmode=disable')
 installed=f.command('community_install',['%s' % GO,'install-db','--component','community','--owner-url-file',community_owner_url,'--database','lmm_community_browser','--owner-role','lmm_community_browser_owner','--runtime-role','lmm_community_browser_runtime'])
 save(roundroot/'community-install-receipt.json',{'official_go_sha256':GO_SHA,'installer_role':'lmm_community_browser_installer','owner_role':'lmm_community_browser_owner','runtime_role':'lmm_community_browser_runtime','receipt':json.loads(installed)})
 community_owner_proxy.close();community_owner_proxy=None
 community_proxy=base.Proxy(f.sockdir/'.s.PGSQL.5432','lmm_community_browser_runtime','lmm_community_browser')
 community_url=f.file('community.url',f'postgresql://lmm_community_browser_runtime:{f.secret(secrets.token_hex(16))}@127.0.0.1:{community_proxy.server_address[1]}/lmm_community_browser?sslmode=disable')
 host=f.file('host.token',secrets.token_hex(24))
 extore_fixture=RetainedExtore(EXT,origin=EXTORE_ORIGIN)
 with patch.dict(os.environ,{'EXTORE_STATIC':str(STATIC)}):extore_fixture.__enter__()
 f.secret(extore_fixture._session)
 assert sha(f'/proc/{extore_fixture._server.pid}/exe')==EXT_SHA
 runtime_bin=temp_root/'plugin-bin';runtime_bin.mkdir(mode=0o700)
 runtime_socket=temp_root/'plugin-socket';runtime_socket.mkdir(mode=0o700)
 host_copy=runtime_bin/'lmm-extore-host';plugin_copy=runtime_bin/'libextore_zplugin.so'
 shutil.copyfile(HOST,host_copy);host_copy.chmod(0o500)
 shutil.copyfile(PLUGIN,plugin_copy);plugin_copy.chmod(0o500)
 assert sha(host_copy)==HOST_SHA and sha(plugin_copy)==PLUGIN_SHA
 plugin_socket=runtime_socket/'extore.sock'
 cert=certificates(temp_root/'tls')
 transport=LocalHTTPS({'extore.lmm.best':('127.0.0.1',extore_fixture.port),'native.api.lmm.best':('127.0.0.1',f.webport)},cert,roundroot/'transport.jsonl')
 for key in ['HTTPS_PROXY','HTTP_PROXY','https_proxy','http_proxy']:f.env[key]=transport.proxy_url
 for key in ['NO_PROXY','no_proxy']:f.env[key]='';os.environ[key]=''
 f.env['SSL_CERT_FILE']=cert['ca']
 extore_key=f.file('extore.key',secrets.token_bytes(32))
 f.go_config=f.file('extensions.toml',f"""version=1
[host]
listen='127.0.0.1:{f.goport}'
token_file={json.dumps(str(host))}
modules=['identity','store','community']
web_compat=true
local_http=false
public_origin='{LMM_ORIGIN}'
[core]
socket={json.dumps(str(f.socket))}
identity_token_file={json.dumps(str(f.control))}
commerce_token_file={json.dumps(str(f.money))}
[community]
database_name='lmm_community_browser'
database_url_file={json.dumps(str(community_url))}
[store]
database_name='lmm_store_browser'
database_url_file={json.dumps(str(dburl))}
extore_key_file={json.dumps(str(extore_key))}
extore_redirect_uri='{LMM_ORIGIN}/store/manage'
[store.extore_plugin]
origin='{EXTORE_ORIGIN}'
host={json.dumps(str(host_copy))}
library={json.dumps(str(plugin_copy))}
sha256='{PLUGIN_SHA}'
socket={json.dumps(str(plugin_socket))}
data_dir={json.dumps(str(extore_fixture.data_dir))}
""")
 f.start_go('browser_go')
 web=http.server.ThreadingHTTPServer(('127.0.0.1',f.webport),Web);servers.append(web);threading.Thread(target=web.serve_forever,daemon=True).start()
 import ssl,urllib.request
 opener=urllib.request.build_opener(urllib.request.ProxyHandler({'http':transport.proxy_url,'https':transport.proxy_url}),urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=cert['ca'])))
 checks=[]
 for origin,path in [(EXTORE_ORIGIN,'/health'),(LMM_ORIGIN,'/health/ready'),(LMM_ORIGIN,'/')]:
  with opener.open(origin+path,timeout=20) as response:
   raw=response.read();assert response.status==200
   if origin==EXTORE_ORIGIN:assert json.loads(raw)['runtime']=='rust'
   if path=='/':assert hashlib.sha256(raw).hexdigest()==binding['files']['index.html']
   checks.append({'origin':origin,'path':path,'status':response.status,'sha256':hashlib.sha256(raw).hexdigest()})
 routed={e['host']:e['target'] for e in transport.events if e.get('layer')=='gateway'}
 assert routed['extore.lmm.best']==['127.0.0.1',extore_fixture.port]
 assert routed['native.api.lmm.best']==['127.0.0.1',f.webport]
 children=set()
 for task in Path(f'/proc/{f.go.proc.pid}/task').iterdir():
  try:children.update(int(x) for x in (task/'children').read_text().split())
  except FileNotFoundError:pass
 host_pids=[pid for pid in children if Path(f'/proc/{pid}/exe').exists() and sha(f'/proc/{pid}/exe')==HOST_SHA]
 assert len(host_pids)==1,'actual Go-managed native host missing'
 host_pid=host_pids[0]
 maps=[line for line in Path(f'/proc/{host_pid}/maps').read_text().splitlines() if str(plugin_copy) in line]
 assert maps,'actual native Extore library not mapped'
 save(roundroot/'transport-preflight.json',{'checks':checks,'routes':routed,'tls_version_minimum':'TLSv1.2','go_ca_file':cert['ca'],'ca_sha256':cert['ca_sha256'],'certificate_sha256':cert['certificate_sha256'],'chromium_test_trust':'exact leaf certificate SPKI, isolated browser process','hosts_file_changed':False,'public_fallback':False,'business_response_mocks':False})
 save(roundroot/'native-host-start.json',{'go_pid':f.go.proc.pid,'host_pid':host_pid,'host_sha256':HOST_SHA,'library_sha256':PLUGIN_SHA,'library_maps':maps,'socket':str(plugin_socket),'data_dir':str(extore_fixture.data_dir)})
 private={'origin':LMM_ORIGIN,'controlOrigin':f'http://127.0.0.1:{f.webport}','extoreOrigin':EXTORE_ORIGIN,'proxy':transport.proxy_url,'spki':cert['spki'],'ca_file':cert['ca'],'adminPassword':f.password,'controlKey':control,'owner_id':f.owner_id,'extoreSession':extore_fixture._session,'extoreShop':extore_fixture.shop_a,'extoreProduct':extore_fixture.products[extore_fixture.shop_a],'playwright':os.environ['LMM_ACCEPTANCE_PLAYWRIGHT'],'chromium':os.environ['LMM_ACCEPTANCE_CHROMIUM'],'tmpdir':str(temp_root/'chromium'),'evidence':str(roundroot)}
 save(roundroot/'browser-private.json',private)
 save(roundroot/'fixture-ready.json',{'ready':True,'started_at_epoch':started_at,'deadline_epoch':hard_deadline,'origin':LMM_ORIGIN,'core_pid':f.main.proc.pid,'go_pid':f.go.proc.pid,'postgres_pid':f.pg.proc.pid,'extore_pid':extore_fixture._server.pid,'host_pid':host_pid,'work':str(f.work),'sockdir':str(f.sockdir),'core_sha256':CORE_SHA,'go_sha256':GO_SHA,'host_sha256':HOST_SHA,'extore_sha256':EXT_SHA,'plugin_sha256':PLUGIN_SHA,'source_hashes':source_hashes,'extore_data_dir':str(extore_fixture.data_dir),'plugin_socket':str(plugin_socket),'extore_sqlite':str(extore_fixture.data_dir/'extore.sqlite3'),'plugin_library':str(plugin_copy),'private_runtime':str(temp_root)})
 capture_state('fixture-preflight')
 print(json.dumps({'ready':True,'evidence':str(roundroot),'origin':LMM_ORIGIN,'feature_source_commit':SOURCE_COMMIT,'extore_source_commit':EXTORE_COMMIT}),flush=True)
 deadline=time.monotonic()+max(0,hard_deadline-time.time())
 while not stop.wait(.5):
  assert time.monotonic()<deadline,'fixture review deadline'
  assert f.pg.proc.poll() is None and f.go.proc.poll() is None,'fixture product exited'
except Exception as e:
 failed=type(e).__name__+': '+str(e)
 save(roundroot/'fixture-failure.json',{'failure':failed,'traceback':traceback.format_exc()})
 print(json.dumps({'ready':False,'failure_type':type(e).__name__,'evidence':str(roundroot)}),flush=True)
finally:
 if transport:transport.close()
 for server in reversed(servers):server.shutdown();server.server_close()
 cleanup=[]
 for process in reversed(f.processes):
  try:process.stop(signal.SIGINT if process is f.pg else signal.SIGTERM);cleanup.append({'label':process.label,'pid':process.proc.pid,'exit_code':process.proc.returncode,'stopped':process.proc.poll() is not None})
  except Exception as e:cleanup.append({'label':process.label,'error':type(e).__name__})
 if extore_fixture:
  extore_fixture.__exit__(None,None,None)
  cleanup.append({'label':'extore_server','pid':extore_fixture._server.pid if extore_fixture._server else None,'stopped':extore_fixture._server is None or extore_fixture._server.poll() is not None,'exit_code':extore_fixture._server.returncode if extore_fixture._server else None,'retained_root':str(extore_fixture.root)})
 if host_pid:cleanup.append({'label':'native_plugin_host','pid':host_pid,'stopped':not Path(f'/proc/{host_pid}').exists()})
 for proxy in [community_proxy,community_owner_proxy,store_proxy,f.proxy]:
  if proxy:proxy.close()
 for label,raw in f.raw_logs+[(p.label,bytes(p.output)) for p in f.processes]:
  if label=='hidden_option_fixture':continue
  text=raw.decode(errors='replace')
  for secret in sorted(f.sensitive,key=len,reverse=True):text=text.replace(secret,'[PRIVATE]')
  (roundroot/(label+'.log')).write_text(text)
 drift=[]
 for label,target,want in [('core',CORE,CORE_SHA),('extensions',GO,GO_SHA),('extore',EXT,EXT_SHA),('host',HOST,HOST_SHA),('plugin',PLUGIN,PLUGIN_SHA),('extore_fixture_helper',helper,HELPER_SHA),('transport',transport_source,TRANSPORT_SHA),('extore_static_manifest',static_manifest,STATIC_MANIFEST_SHA),*[(('extore-static/'+name),STATIC/name,want) for name,want in static_binding['files'].items()],*[(name,DIST/name,want) for name,want in binding['files'].items()],*[(name,SOURCE/name,want) for name,want in source_hashes.items()]]:
  try:
   if sha(target)!=want:drift.append(label)
  except OSError:drift.append(label)
 if drift:failed=failed or 'artifact_input_drift'
 save(roundroot/'artifact-recheck.json',{'drift':drift,'unchanged':not drift})
 save(roundroot/'fixture-cleanup.json',{'processes':cleanup,'all_stopped':all(x.get('stopped') for x in cleanup),'fixture_retained':True,'failure':failed})
 save(roundroot/'fixture-events.json',f.events)
 save(roundroot/'fixture-commands.json',f.commands)
raise SystemExit(1 if failed else 0)
