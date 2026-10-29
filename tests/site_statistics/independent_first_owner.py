#!/usr/bin/env python3
"""Independent first-owner regression, generalized from the unchanged R6 p2 failure.

Only launch/binding/cleanup differ; CLI, login, identity and zero-value oracle stay exact.
Run after space-first with an explicitly frozen PASS candidate. No Core compilation.
"""
import hashlib,http.client,json,os,secrets,socket,subprocess,time,traceback
from pathlib import Path
import argparse,shutil,sys
parser=argparse.ArgumentParser(description="Real production first-owner CLI/password/empty-finance regression; no synthetic audit")
parser.add_argument('--root',type=Path,required=True)
parser.add_argument('--candidate',type=Path,required=True)
parser.add_argument('--core-sha',required=True)
parser.add_argument('--source-pins-sha',required=True)
parser.add_argument('--worker',action='store_true')
args=parser.parse_args();os.umask(0o077)
r=args.root.absolute();candidate=args.candidate.resolve()
assert r.parent==Path('/home/lightjunction/Documents/Codex'),'private short evidence root required'
assert len(str(r/'s/.s.PGSQL.55617').encode())<104,'short socket root required'
assert json.loads((candidate/'result.json').read_text())['status']=='passed','candidate is not PASS'
assert json.loads((candidate/'binaries.json').read_text())['lmm-core']['sha256']==args.core_sha
core=candidate/'lmm-core';expected_core=args.core_sha
if not args.worker:
 assert not r.exists(),'fresh evidence root required'
 assert shutil.disk_usage(r.parent).free>=2*1024**3,'space-first admission'
 r.mkdir(mode=0o700);shutil.copy2(Path(__file__),r/'run.py')
 command=['systemd-run','--user','--wait','--pipe','--collect','--unit=lmm-finance-first-owner-'+r.name,
  '-p','MemoryMax=2G','-p','MemoryHigh=1536M','-p','MemorySwapMax=4G','-p','CPUQuota=100%','-p','TasksMax=96',
  '/usr/bin/flock','/tmp/lmm-delegate/heavy-build.lock',sys.executable,'-B',str(r/'run.py'),
  '--root',str(r),'--candidate',str(candidate),'--core-sha',args.core_sha,'--source-pins-sha',args.source_pins_sha,'--worker']
 env={k:v for k,v in os.environ.items() if not k.startswith(('LMM_','PG','SQLX_')) and k not in ('DATABASE_URL','SQL_DSN','HTTP_PROXY','HTTPS_PROXY','ALL_PROXY')}
 env.update(XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}',DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus',PYTHONDONTWRITEBYTECODE='1')
 (r/'launch.json').write_text(json.dumps(command,indent=2)+'\n')
 with (r/'unit.stdout').open('w') as output,(r/'unit.stderr').open('w') as error:
  code=subprocess.run(command,stdout=output,stderr=error,env=env).returncode
 (r/'launch-result.json').write_text(json.dumps({'exit_code':code})+'\n')
 manifest={}
 for path in r.rglob('*'):
  if path.is_file() and path.name!='hash-manifest.json':
   with path.open('rb') as stream:manifest[str(path.relative_to(r))]=hashlib.file_digest(stream,'sha256').hexdigest()
 (r/'hash-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 raise SystemExit(code)

seq=0;pg=False;process=None
def save(name,v): (r/name).write_text(json.dumps(v,indent=2)+'\n')
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def run(label,argv,body=None):
 global seq
 seq+=1;p=r/f'{seq:02}-{label}'
 save(p.name+'.argv.json',[str(x) for x in argv])
 if body is not None:p.with_suffix('.stdin.private').write_text(body)
 c=subprocess.run([str(x) for x in argv],input=body,text=True,capture_output=True,timeout=90)
 p.with_suffix('.stdout').write_text(c.stdout);p.with_suffix('.stderr').write_text(c.stderr)
 assert c.returncode==0,(label,c.returncode)
 return c.stdout.strip()
sock=r/'s';sock.mkdir()
base=['psql','-X','-qAt','-v','ON_ERROR_STOP=1','-h',str(sock),'-p','55617','-U','postgres','-d','finance_owner']
def sql(text):return run('sql',base,text)
try:
 memory=Path('/proc/meminfo').read_text();(r/'memory-admission.txt').write_text(memory)
 assert int(next(x.split()[1] for x in memory.splitlines() if x.startswith('MemAvailable:')))>=2097152,'2GiB admission'
 assert sha(core)==expected_core
 assert sha(candidate/'source-pins.json')==args.source_pins_sha,'source pins differ from frozen candidate'
 pins=json.loads((candidate/'source-pins.json').read_text());assert all(sha(candidate/'source'/p)==h for p,h in pins.items())
 save('binding.json',{'core':str(core),'core_sha256':expected_core,'pins_sha256':sha(candidate/'source-pins.json'),'verified_pins':len(pins),'runner_sha256':sha(Path(__file__))})
 run('initdb',['initdb','-D',r/'pg','-U','postgres','--no-locale','--encoding=UTF8','--auth=trust','--wal-segsize=1'])
 run('pg-start',['pg_ctl','-D',r/'pg','-l',r/'postgres.log','-o',f'-c listen_addresses= -p 55617 -c unix_socket_directories={sock} -c unix_socket_permissions=0700 -c shared_buffers=16MB -c max_connections=15 -c min_wal_size=2MB -c max_wal_size=32MB','-w','start']);pg=True
 setup=base.copy();setup[-1]='postgres'
 run('roles',setup+['-f',candidate/'source/deployment/postgres/create-core-roles.sql'])
 run('database',setup,'CREATE ROLE independent_installer LOGIN NOINHERIT; GRANT lmm_core_owner TO independent_installer WITH INHERIT FALSE,SET TRUE; CREATE DATABASE finance_owner OWNER lmm_core_owner;')
 sql('GRANT CONNECT ON DATABASE finance_owner TO independent_installer; REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_control_system() TO lmm_core_owner;')
 from urllib.parse import urlencode
 query={'host':str(sock),'port':55617,'sslmode':'disable'}
 (r/'owner.url').write_text('postgresql://independent_installer@localhost/finance_owner?'+urlencode({**query,'options':'-c role=lmm_core_owner'}))
 (r/'owner.toml').write_text('version=1\n[database]\nurl_file='+json.dumps(str(r/'owner.url'))+'\n')
 run('native-init',[core,'--config',r/'owner.toml','init-db'])
 password='Synthetic!'+secrets.token_hex(16)
 credentials={'username':'financeowner','display_name':'Independent Finance Owner','password':password}
 save('browser-credentials.private.json',credentials)
 receipt=json.loads(run('real-first-owner',[core,'--config',r/'owner.toml','create-first-owner'],json.dumps(credentials)))
 assert receipt['setup_completed'] is True
 run('canonical-runtime-grants',base+['-v','enable_console=true','-v','enable_web_identity=true','-f',candidate/'source/deployment/postgres/grant-core-runtime.sql'])
 (r/'runtime.url').write_text('postgresql://lmm_core_runtime@localhost/finance_owner?'+urlencode(query))
 (r/'refresh.key').write_bytes(secrets.token_bytes(32))
 with socket.socket() as listener:listener.bind(('127.0.0.1',0));port=listener.getsockname()[1]
 origin=f'http://127.0.0.1:{port}'
 (r/'core.toml').write_text(f'''version=1
[server]
listen="127.0.0.1:{port}"
worker_threads=1
max_inflight=8
drain_timeout_seconds=3
[database]
url_file="{r/'runtime.url'}"
max_connections=3
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
origin="{origin}"
refresh_key_file="{r/'refresh.key'}"
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
''')
 argv=[str(core),'--config',str(r/'core.toml'),'serve'];save('serve-command.json',argv)
 with (r/'core.log').open('w') as output:process=subprocess.Popen(argv,stdout=output,stderr=subprocess.STDOUT)
 deadline=time.monotonic()+15
 while True:
  assert process.poll() is None,'Core startup failed'
  try:
   with socket.create_connection(('127.0.0.1',port),.1):break
  except OSError:assert time.monotonic()<deadline;time.sleep(.05)
 assert sha(Path(f'/proc/{process.pid}/exe'))==expected_core
 save('actual-native.json',{'MainPID':process.pid,'sha256':expected_core,'origin':origin,'runtime_roles':sql("SELECT json_agg(json_build_object('role',rolname,'super',rolsuper,'bypassrls',rolbypassrls)) FROM pg_roles WHERE rolname IN ('lmm_core_runtime','lmm_core_owner','independent_installer')")})
 def request_http(label,method,path,body=None,token=None):
  headers={'Origin':origin,'Content-Type':'application/json'}
  if token:headers['Authorization']='Bearer '+token
  c=http.client.HTTPConnection('127.0.0.1',port,timeout=20)
  c.request(method,path,json.dumps(body) if body is not None else None,headers)
  response=c.getresponse();wire=response.read();status=response.status;c.close()
  (r/(label+'.body.private')).write_bytes(wire)
  save(label+'.request.json',{'method':method,'path':path,'authenticated':bool(token)})
  save(label+'.response.json',{'status':status})
  return status,json.loads(wire)
 status,login=request_http('password-login','POST','/api/user/login',{'username':credentials['username'],'password':password})
 assert status==200 and login['success'] is True,'real password login failed'
 token=login['data']['access_token']
 status,identity=request_http('identity','GET','/core/v1/identity',token=token)
 assert status==200 and identity['platform_level']==6 and identity['user_id']==receipt['user_id']
 audits=json.loads(sql("SELECT jsonb_agg(jsonb_build_object('actor',actor_user_id,'action',action,'target',target) ORDER BY id) FROM core_identity.audit"));save('identity-audits.json',audits)
 save('independent-empty-facts.json',{'users':json.loads(sql('SELECT jsonb_agg(to_jsonb(u)) FROM core_identity.users u')),'request_count':int(sql('SELECT count(*) FROM core_charging.requests')),'opening_count':int(sql('SELECT count(*) FROM core_billing.account_opening_counters')),'payment_count':int(sql('SELECT count(*) FROM core_payments.intents'))})
 status,finance=request_http('finance','GET','/api/finance/site-statistics',token=token)
 expected={'credits_per_usd':500000,'total_used_credits':'0','total_balance_credits':'0','recharge':{'currencies':[],'virtual_units':[],'confirmed_orders':0,'unconfirmed_orders':0,'invalid_orders':0}}
 save('finance-oracle.json',{'expected_status':200,'expected_data_without_as_of':expected,'basis':'fresh production first-owner; no opening/import/request/payment/balance history'})
 assert status==200,'formal first-owner finance read must be a known empty site'
 assert {k:v for k,v in finance['data'].items() if k!='as_of'}==expected
 save('result.json',{'status':'PASS','scope':'formal first-owner CLI, real password login and known empty financial DTO; no browser'})
except Exception:
 (r/'failure.txt').write_text(traceback.format_exc());save('result.json',{'status':'FAIL'});raise
finally:
 if process and process.poll() is None:process.terminate();process.wait(timeout=10)
 if pg:run('pg-stop',['pg_ctl','-D',r/'pg','-m','fast','-w','stop'])
 save('stopped.json',{'core_stopped':process is None or process.poll() is not None,'pg_stopped':not(r/'pg/postmaster.pid').exists()})
 if (r/'result.json').exists() and json.loads((r/'result.json').read_text())['status']=='PASS':
  removed={}
  names=['pg','s','owner.url','runtime.url','refresh.key','browser-credentials.private.json','password-login.body.private']
  names.extend(p.name for p in r.glob('*-real-first-owner.stdin.private'))
  for name in names:
   path=r/name
   if path.exists():
    removed[name]=sum(p.stat().st_size for p in path.rglob('*') if p.is_file()) if path.is_dir() else path.stat().st_size
    shutil.rmtree(path) if path.is_dir() else path.unlink()
  save('cleanup.json',{'logical_bytes_removed':sum(removed.values()),'paths':removed})
 cg=Path('/sys/fs/cgroup')/Path('/proc/self/cgroup').read_text().strip().split('::')[1].lstrip('/')
 save('resources.json',{n:(cg/n).read_text().strip() for n in ['memory.peak','memory.max','memory.high','memory.swap.max','cpu.max']})
 save('hash-manifest.json',{str(p.relative_to(r)):sha(p) for p in r.rglob('*') if p.is_file() and p.name!='hash-manifest.json'})
