#!/usr/bin/env python3
"""Independent v3 offline recovery and death-window acceptance, synthetic PG only."""
import argparse,copy,hashlib,importlib.util,json,os,secrets,shutil,signal,socket,struct,subprocess,sys,tempfile,time
from pathlib import Path

TEST=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('r4_fixture',TEST/'fixture.py');fixture=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixture)
spec2=importlib.util.spec_from_file_location('management_pause',TEST/'pause_proxy.py');pause=importlib.util.module_from_spec(spec2);spec2.loader.exec_module(pause)

class ProductFailure(AssertionError):pass
class EvidenceBlocked(RuntimeError):pass

def require(condition,message):
 if not condition:raise ProductFailure(message)

def digest(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def event(f,name,data):
 f.save('independent-'+name+'.json',data)
 print(name,json.dumps(data,ensure_ascii=False),flush=True)

class Fixture(fixture.Fixture):
 def run(self,name,argv,body=None,expected=0,env=None):
  if name=='daemon-reload' and self.unit_file and self.unit_file.exists():
   with self.unit_file.open('a') as fp:fp.write('Slice='+os.environ['INDEPENDENT_SLICE']+'\n')
  return super().run(name,argv,body,expected,env)


def acl(f,query):
 sql=r'''WITH ns AS (SELECT oid,nspname,nspowner,nspacl FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\'),
 facts AS (
 SELECT jsonb_build_array('schema-owner',n.nspname,pg_get_userbyid(n.nspowner))::text x FROM ns n
 UNION ALL SELECT jsonb_build_array('relation-owner',n.nspname,c.relname,pg_get_userbyid(c.relowner))::text FROM pg_class c JOIN ns n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p','v','m','S','f')
 UNION ALL SELECT jsonb_build_array('schema-grant',n.nspname,CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END,a.privilege_type,a.is_grantable)::text FROM ns n CROSS JOIN LATERAL aclexplode(coalesce(n.nspacl,acldefault('n',n.nspowner))) a
 UNION ALL SELECT jsonb_build_array('relation-grant',n.nspname,c.relname,CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END,a.privilege_type,a.is_grantable)::text FROM pg_class c JOIN ns n ON n.oid=c.relnamespace CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault(CASE WHEN c.relkind='S' THEN 'S'::"char" ELSE 'r'::"char" END,c.relowner))) a WHERE c.relkind IN ('r','p','v','m','S','f')
 UNION ALL SELECT jsonb_build_array('column-grant',n.nspname,c.relname,col.attname,CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END,a.privilege_type,a.is_grantable)::text FROM pg_class c JOIN ns n ON n.oid=c.relnamespace JOIN pg_attribute col ON col.attrelid=c.oid AND col.attnum>0 AND NOT col.attisdropped CROSS JOIN LATERAL aclexplode(col.attacl) a
 UNION ALL SELECT jsonb_build_array('routine-owner',n.nspname,p.oid::regprocedure::text,pg_get_userbyid(p.proowner))::text FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace
 UNION ALL SELECT jsonb_build_array('routine-grant',n.nspname,p.oid::regprocedure::text,CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END,a.privilege_type,a.is_grantable)::text FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace CROSS JOIN LATERAL aclexplode(coalesce(p.proacl,acldefault('f',p.proowner))) a
 ) SELECT x FROM facts ORDER BY x COLLATE "C"'''
 return set(query(sql).splitlines())


def snapshots(f,query):
 tables=['core_identity.accounts','core_identity.users','core_identity.credentials','core_identity.password_logins','core_identity.user_profiles','core_console.options']
 rows={}
 for name in tables:
  content=query("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)::text FROM "+name+' t')
  parsed=json.loads(content);rows[name]={'count':len(parsed),'sha256':hashlib.sha256(content.encode()).hexdigest()}
 require(rows['core_identity.users']['count']>=1,'fixture_missing_user')
 require(rows['core_identity.password_logins']['count']>=1,'fixture_missing_password_credential')
 require(rows['core_identity.credentials']['count']>=1,'fixture_missing_api_credential')
 old_tables=query("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core\\_%' ESCAPE '\\' AND c.relkind IN ('r','p') ORDER BY n.nspname,c.relname").splitlines()
 if not old_tables:raise EvidenceBlocked('no original core tables discovered')
 all_rows_sql=[]
 for name in old_tables:
  literal="'"+name.replace("'","''")+"'"
  all_rows_sql.append("SELECT "+literal+" AS name,count(*) AS n,coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) AS payload FROM "+name+" t")
 all_rows=json.loads(query("SELECT jsonb_object_agg(name,jsonb_build_object('count',n,'sha256',encode(sha256(convert_to(payload::text,'UTF8')),'hex')) ORDER BY name)::text FROM ("+' UNION ALL '.join(all_rows_sql)+") rows"))
 runtime_contract=json.loads(query("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)::text FROM core_meta.runtime_contract t"))
 names=query("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core\\_%' ESCAPE '\\' AND c.relkind='S' ORDER BY n.nspname,c.relname").splitlines()
 require(len(names)==22,'predecessor_or_successor_sequence_count_not_22')
 clauses=["SELECT "+"'"+name.replace("'","''")+"'::text AS name,last_value::text AS value,log_cnt::text AS log,is_called FROM "+name for name in names]
 sequences=json.loads(query("SELECT jsonb_object_agg(name,jsonb_build_array(value,log,is_called) ORDER BY name)::text FROM ("+' UNION ALL '.join(clauses)+") s"))
 privileges=acl(f,query)
 role_sql="SELECT jsonb_object_agg(rolname,jsonb_build_object('password_sha256',encode(sha256(convert_to(coalesce(rolpassword,''),'UTF8')),'hex'),'super',rolsuper,'create_db',rolcreatedb,'create_role',rolcreaterole,'replication',rolreplication,'bypass_rls',rolbypassrls,'inherit',rolinherit,'login',rolcanlogin))::text FROM pg_authid WHERE rolname IN ('lmm_core_installer','lmm_core_owner','lmm_core_runtime','lmm_extensions')"
 roles=json.loads(f.sql(f.target,role_sql))
 memberships=f.sql(f.target,"SELECT coalesce(jsonb_agg(jsonb_build_array(p.rolname,m.rolname,a.admin_option,a.inherit_option,a.set_option) ORDER BY p.rolname,m.rolname),'[]')::text FROM pg_auth_members a JOIN pg_roles p ON p.oid=a.roleid JOIN pg_roles m ON m.oid=a.member WHERE p.rolname LIKE 'lmm_%' OR m.rolname LIKE 'lmm_%'")
 return {'rows':rows,'all_table_rows':all_rows,'runtime_contract':runtime_contract,'sequences':sequences,'acl':sorted(privileges),'roles':roles,'memberships':json.loads(memberships)}


def compare_preserved(before,after):
 require(before['rows']==after['rows'],'PRODUCT_BUSINESS_ROWS_OR_CREDENTIALS_CHANGED')
 changed=[name for name,value in before['all_table_rows'].items() if after['all_table_rows'].get(name)!=value]
 require(changed==['core_meta.runtime_contract'],'PRODUCT_EXISTING_TABLE_ROWS_CHANGED:'+json.dumps(changed))
 contract=copy.deepcopy(before['runtime_contract'])
 require(len(contract)==1 and contract[0].get('catalog_fingerprint')=='\\xacbee20015ea1397adfda06232f25c811956823e00b15e574d74794702f418e5','FIXTURE_PREDECESSOR_RUNTIME_CONTRACT_WRONG')
 contract[0]['catalog_fingerprint']='\\x557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d'
 require(after['runtime_contract']==contract,'PRODUCT_RUNTIME_CONTRACT_OTHER_FIELDS_OR_TARGET_FINGERPRINT_WRONG')
 require(before['sequences']==after['sequences'],'PRODUCT_EXISTING_SEQUENCE_STATE_CHANGED')
 missing=sorted(set(before['acl'])-set(after['acl']))
 require(not missing,'PRODUCT_EXISTING_OWNER_OR_ACL_REMOVED:'+json.dumps(missing[:5]))
 require(before['memberships']==after['memberships'],'PRODUCT_ROLE_MEMBERSHIPS_CHANGED')
 for name in before['roles']:
  a=dict(before['roles'][name]);b=dict(after['roles'][name])
  if name=='lmm_core_runtime':
   require(b['login'] is False,'PRODUCT_RUNTIME_ROLE_STILL_LOGIN');a.pop('login');b.pop('login')
  require(a==b,'PRODUCT_ROLE_OR_PASSWORD_CHANGED:'+name)
 return {'business_tables':len(before['rows']),'all_existing_tables':len(before['all_table_rows']),'populated_existing_tables':sum(value['count']>0 for value in before['all_table_rows'].values()),'credential_rows':before['rows']['core_identity.credentials']['count'],'password_rows':before['rows']['core_identity.password_logins']['count'],'old_sequences':len(before['sequences']),'old_acl_facts':len(before['acl']),'old_acl_removed':len(missing),'runtime_nologin':True}


def request_call(f,name,request):
 p=f.run(name,[f.args.core,'--config',f.config_path,'upgrade-offline'],json.dumps(request).encode(),expected=None)
 try:parsed=json.loads(p.stdout)
 except ValueError:parsed=None
 return p,parsed


def expected_reject(f,name,request,code=None):
 p,out=request_call(f,name,request)
 require(p.returncode!=0,'PRODUCT_ACCEPTED_'+name)
 if code:require(out is not None and out.get('code')==code,'PRODUCT_WRONG_REJECTION_'+name+':'+str(out))
 event(f,name,{'exit':p.returncode,'report':out})


def state(f):
 history=f.sql(f.authority,"SELECT coalesce(jsonb_agg(jsonb_build_object('phase',phase,'sequence',sequence,'receipt_sha256',receipt_sha256) ORDER BY operation_id),'[]')::text FROM lmm_upgrade_history.operations")
 db=f.sql(f.target,"SELECT jsonb_build_object('allow_connections',datallowconn,'oid',oid)::text FROM pg_database WHERE datname='coretarget'")
 return {'external_history':json.loads(history),'target_database':json.loads(db)}


def runtime_rejected(f,name):
 env=dict(f.env,PGOPTIONS='-c client_min_messages=error')
 p=f.run(name,['psql','-X','-q','-v','VERBOSITY=verbose','-h',f.target['socket'],'-p',f.target['port'],'-U','lmm_core_runtime','-d','coretarget','-c','SELECT 1'],expected=None,env=env)
 err=p.stderr.decode(errors='replace')
 require(p.returncode!=0 and 'pg_hba.conf' in err,'PRODUCT_RUNTIME_HBA_NOT_PROVEN:'+err[:250])
 # psql does not expose connection SQLSTATE in its stderr even with VERBOSITY.
 # Read the PostgreSQL ErrorResponse fields from a second real startup instead.
 client=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);client.settimeout(5)
 try:
  client.connect(str(f.target['socket']/f".s.PGSQL.{f.target['port']}"))
  body=struct.pack('!I',196608)+b'user\0lmm_core_runtime\0database\0coretarget\0\0'
  client.sendall(struct.pack('!I',len(body)+4)+body)
  kind,data,raw=pause.frame(client)
  if kind!=b'E':raise EvidenceBlocked('runtime startup did not return a PostgreSQL ErrorResponse')
  fields={x[:1].decode():x[1:].decode(errors='replace') for x in data.split(b'\0') if x}
 finally:client.close()
 require(fields.get('C')=='28000' and 'pg_hba.conf' in fields.get('M',''),'PRODUCT_RUNTIME_PROTOCOL_HBA_REJECTION_WRONG:'+str(fields))
 return {'psql_exit':p.returncode,'sqlstate':fields['C'],'message':fields['M'],'error_response_hex':raw.hex(),'error_response_fields':fields,'hba_rejection':True}


def unit_fenced(f):
 s=f.run('independent-unit-show',['systemctl','--user','show',f.unit,'--property=MainPID,UnitFileState,ActiveState,FragmentPath,ControlGroup']).stdout.decode()
 require('MainPID=0' in s and 'UnitFileState=masked' in s and ('ActiveState=inactive' in s or 'ActiveState=failed' in s),'PRODUCT_OLD_SERVICE_NOT_MASKED_STOPPED')
 attempt=f.run('independent-unit-restart-denied',['systemctl','--user','start',f.unit],expected=None)
 require(attempt.returncode!=0,'PRODUCT_MASKED_SERVICE_RESTARTED')
 return {'show':s,'restart_exit':attempt.returncode}


def current_limits():
 rel=Path('/proc/self/cgroup').read_text().strip().split(':')[-1].lstrip('/')
 path=Path('/sys/fs/cgroup')/rel;slice_name=os.environ['INDEPENDENT_SLICE'];ancestors=[x for x in [path,*path.parents] if x.name==slice_name]
 if not ancestors:raise EvidenceBlocked('test scope not inside intended slice')
 parent=ancestors[0]
 return {'scope':str(path),'slice':str(parent),'aggregate':{n:(parent/n).read_text().strip() for n in ['memory.max','memory.high','memory.swap.max','cpu.max']}}


def execute(args):
 f=Fixture(args);management_proxy=None;proxy_dir=None;child=None;runtime_dir=None;status='BLOCKED'
 try:
  limits=current_limits();expected={'memory.max':'2147483648','memory.high':'1610612736','memory.swap.max':'4294967296','cpu.max':'100000 100000'}
  if limits['aggregate']!=expected:raise EvidenceBlocked('resource slice limits do not match request')
  event(f,'resource-limits',limits)
  available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024
  gate={'memory_available':available,'evidence_filesystem_free':shutil.disk_usage(f.root).free,'tmp_filesystem_free':shutil.disk_usage('/tmp').free,'memory_pressure':Path('/proc/pressure/memory').read_text(),'io_pressure':Path('/proc/pressure/io').read_text()}
  event(f,'resource-gate',gate)
  if available<2*1024**3 or gate['evidence_filesystem_free']<4*1024**3 or gate['tmp_filesystem_free']<512*1024**2:raise EvidenceBlocked('resource gate failed after shared lock acquired')
  print('fixture setup: target and independent history PostgreSQL 18',flush=True);f.setup()
  unit_state=f.run('independent-live-unit',['systemctl','--user','show',f.unit,'--property=MainPID,ControlGroup,ActiveState']).stdout.decode()
  if not ('ActiveState=active' in unit_state and os.environ['INDEPENDENT_SLICE'] in unit_state):raise EvidenceBlocked('fixture service not active inside bounded slice')
  metadata=json.loads(f.sql(f.target,"SELECT jsonb_build_object('owner',pg_get_userbyid(datdba),'installer_connect',has_database_privilege('lmm_core_installer','coretarget','CONNECT')) FROM pg_database WHERE datname='coretarget'"))
  control=f.sql(f.target,"SELECT NOT has_function_privilege('lmm_core_runtime','pg_control_system()','EXECUTE')",'coretarget')
  if metadata!={'owner':'lmm_core_owner','installer_connect':True} or control!='t':raise EvidenceBlocked('fixture ownership/CONNECT/target pg_control_system preconditions failed')
  data_dir=Path(f.sql(f.target,"SHOW data_directory")).resolve();hba=Path(f.sql(f.target,"SHOW hba_file")).resolve()
  if hba!=f.target['hba'].resolve() or hba.is_relative_to(data_dir):raise EvidenceBlocked('fixture HBA is not outside physical data directory')
  event(f,'fixture-contract',{'pg_version':f.sql(f.target,'SHOW server_version'),'database':metadata,'target_runtime_pg_control_system_execute':False,'unit':f.unit,'initial_state':unit_state,'hba_external_to_pgdata':True,'hba_path':str(hba),'pgdata':str(data_dir)})
  f.sql(f.target,"INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT decode(repeat('ac',32),'hex'),'api_key',id,auth_version,personal_account_id,clock_timestamp()+interval '365 days' FROM core_identity.users ORDER BY id LIMIT 1",'coretarget')
  before=snapshots(f,lambda q:f.sql(f.target,q,'coretarget'))
  f.save('independent-before.json',before)
  event(f,'preconditions',{'users':before['rows']['core_identity.users']['count'],'password_credentials':before['rows']['core_identity.password_logins']['count'],'api_credentials':before['rows']['core_identity.credentials']['count'],'sequences':len(before['sequences']),'acl_facts':len(before['acl'])})
  missing=copy.deepcopy(f.request);del missing['authority_id'];expected_reject(f,'missing-authority-binding',missing)
  wrong=copy.deepcopy(f.request);wrong['candidate_sha256']='0'*64;expected_reject(f,'wrong-binary-binding',wrong,'OFFLINE_EXECUTABLE_MISMATCH')
  wrong=copy.deepcopy(f.request);wrong['authority_id']='1'*64;expected_reject(f,'wrong-authority-binding',wrong,'OFFLINE_HISTORY_AUTHORITY_MISMATCH')
  wrong=copy.deepcopy(f.request);wrong['bridge']['database_oid']+=1;expected_reject(f,'wrong-database-binding',wrong,'OFFLINE_MANAGEMENT_IDENTITY_MISMATCH')
  s=state(f);require(s['external_history']==[] and s['target_database']['allow_connections'] is True,'PRODUCT_BAD_BINDING_MUTATED_FENCE_OR_HISTORY')
  f.save('independent-before-apply-state.json',s)
  print('actual apply: suppress only target COMMIT acknowledgement',flush=True)
  p,out=request_call(f,'independent-target-ack-apply',f.request)
  require(p.returncode!=0 and out and out.get('code')=='OFFLINE_COMMIT_UNKNOWN_USE_RECOVER','PRODUCT_UNEXPECTED_TARGET_COMMIT_RESULT:'+str(out))
  target_proxy=f.proxies[0]
  if not target_proxy.dropped.is_set() or target_proxy.errors:raise EvidenceBlocked('target acknowledgement fault not proven by proxy')
  events=list(target_proxy.events)
  if not ('upstream_command_complete_commit_suppressed' in events and 'upstream_ready_idle_suppressed_connection_closed' in events):raise EvidenceBlocked('upstream committed target transaction not proven')
  f.save('independent-target-ack.json',{'cli_exit':p.returncode,'report':out,'proxy_events':events,'proxy_errors':target_proxy.errors})
  f.assert_closed();fenced=unit_fenced(f);started=state(f)
  require(len(started['external_history'])==1 and started['external_history'][0]['phase']=='started','PRODUCT_EXTERNAL_STARTED_MISSING')
  after=snapshots(f,lambda q:f.target_sql(q));f.save('independent-after-commit.json',after);comparison=compare_preserved(before,after)
  receipt=json.loads(f.target_sql("SELECT jsonb_build_object('count',count(*),'operation',max(operation_id),'successor',max(successor_catalog_sha256)) FROM core_meta.offline_upgrades"))
  require(receipt['count']==1 and receipt['operation']==f.request['operation_id'] and receipt['successor']=='557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d','PRODUCT_TARGET_RECEIPT_OR_CATALOG_WRONG')
  event(f,'started-plus-actual-receipt',{'state':started,'receipt':receipt,'preservation':comparison,'unit':fenced})
  before_hba=digest(hba)
  proxy_dir=Path(tempfile.mkdtemp(prefix='lmm-v3-mgmt-',dir='/tmp'));proxy_sock=proxy_dir/'.s.PGSQL.55603'
  management_proxy=pause.ManagementPause(proxy_sock,f.target['socket']/f".s.PGSQL.{f.target['port']}")
  management_file=Path(f.request['management_url_file']);original_management=management_file.read_text();management_file.write_text(f'postgresql://postgres@localhost/postgres?host={proxy_dir}&port=55603&sslmode=disable');management_file.chmod(0o600)
  recover=copy.deepcopy(f.request);recover['action']='recover'
  outpath=f.root/'independent-killed-recover.stdout';errpath=f.root/'independent-killed-recover.stderr'
  with outpath.open('wb') as stdout,errpath.open('wb') as stderr:
   child=subprocess.Popen([str(f.args.core),'--config',str(f.config_path),'upgrade-offline'],stdin=subprocess.PIPE,stdout=stdout,stderr=stderr,env=f.env,start_new_session=True)
   child.stdin.write(json.dumps(recover).encode());child.stdin.close()
   deadline=time.monotonic()+60
   while not management_proxy.hit.wait(.05):
    if child.poll() is not None:raise EvidenceBlocked('recover exited before target ALLOW window; inspect stdout/stderr')
    if time.monotonic()>deadline:raise EvidenceBlocked('management pause did not observe completed ALTER DATABASE')
   open_state=state(f)
   if not (open_state['target_database']['allow_connections'] is True and open_state['external_history'][0]['phase']=='started'):raise EvidenceBlocked('did not hit Started and ALLOW true together')
   runtime_probe=runtime_rejected(f,'independent-runtime-denied-during-allow')
   os.kill(child.pid,signal.SIGKILL);rc=child.wait(timeout=10)
  management_proxy.close();proxy_events=list(management_proxy.events);proxy_errors=list(management_proxy.errors);management_proxy=None;management_file.write_text(original_management);management_file.chmod(0o600)
  if rc!=-signal.SIGKILL or proxy_errors or proxy_events!=['alter_database_command_complete_observed','alter_database_ready_idle_observed_ack_held']:raise EvidenceBlocked('management pause or SIGKILL instrumentation was not proven')
  require(digest(hba)==before_hba,'PRODUCT_EXTERNAL_HBA_CHANGED_IN_WINDOW')
  post_kill=state(f)
  if not (post_kill['target_database']['allow_connections'] is True and post_kill['external_history'][0]['phase']=='started'):raise EvidenceBlocked('post-kill state changed before observation')
  event(f,'sigkill-allow-window',{'controller_wait_exit':rc,'management_proxy_events':proxy_events,'management_proxy_errors':proxy_errors,'state_at_kill':open_state,'state_after_kill':post_kill,'runtime_probe':runtime_probe,'external_hba_unchanged':True})
  expected_reject(f,'started-replay-refused',f.request,'OFFLINE_STARTED_RECOVER_ONLY')
  require(state(f)['target_database']['allow_connections'] is True,'FIXTURE_REPLAY_UNEXPECTEDLY_CHANGED_ALLOW_WINDOW')
  _,observed=f.cli('independent-recover-started-receipt',f.config_path,recover)
  require(observed.get('status')=='committed_receipt_recovered' and observed.get('retry_allowed') is False and observed.get('site_open_allowed') is False and observed.get('target_contract_matches') is True,'PRODUCT_STARTED_RECEIPT_RECOVERY_INVALID:'+str(observed))
  f.assert_closed();closed=state(f);require(closed['external_history'][0]['phase']=='committed' and closed['target_database']['allow_connections'] is False,'PRODUCT_RECOVERY_DID_NOT_RESTORE_FENCE')
  event(f,'recovered',{'report':observed,'state':closed,'unit':unit_fenced(f)})
  f.stop(f.target);f.target['data'].rename(f.target['directory']/'committed-data');shutil.copytree(f.backup,f.target['data']);f.start(f.target)
  require(digest(hba)==before_hba,'PRODUCT_PHYSICAL_RESTORE_REPLACED_EXTERNAL_HBA')
  restored_before=state(f);require(restored_before['external_history'][0]['phase']=='committed','PRODUCT_EXTERNAL_HISTORY_LOST_ON_TARGET_RESTORE')
  _,restored=f.cli('independent-recover-after-physical-restore',f.config_path,recover)
  require(restored.get('status')=='database_diverged_from_committed_history' and restored.get('observed_receipt') is False and restored.get('retry_allowed') is False and restored.get('site_open_allowed') is False,'PRODUCT_RESTORED_TARGET_MISCLASSIFIED:'+str(restored))
  expected_reject(f,'restored-replay-refused',f.request,'OFFLINE_STARTED_RECOVER_ONLY');f.assert_closed()
  require(f.target_sql("SELECT to_regclass('core_meta.offline_upgrades') IS NULL")=='t','PRODUCT_RESTORE_REPLAYED_RECEIPT_DDL')
  final=state(f);require(final['external_history'][0]['phase']=='committed' and not final['target_database']['allow_connections'],'PRODUCT_RESTORE_DID_NOT_REMAIN_FENCED')
  event(f,'physical-restore',{'before_recover':restored_before,'report':restored,'final':final,'unit':unit_fenced(f),'external_hba_unchanged':True})
  status='PASS';f.save('independent-result.json',{'status':status,'binary_sha256':digest(args.core),'previous_binary_sha256':digest(args.previous_core),'scope':'real CLI, PostgreSQL 18, isolated synthetic target and external history, explicit user service; no site finalization','events':'independent-*.json','limits':limits})
 except ProductFailure as e:
  status='FAIL';f.save('independent-result.json',{'status':status,'failure':str(e),'binary_sha256':digest(args.core),'stop_dependent_cases':True});raise
 except BaseException as e:
  f.save('independent-result.json',{'status':status,'reason':repr(e),'binary_sha256':digest(args.core),'stop_dependent_cases':True});raise
 finally:
  if child and child.poll() is None:child.kill();child.wait(timeout=5)
  if management_proxy:management_proxy.close()
  if proxy_dir and proxy_dir.exists():shutil.rmtree(proxy_dir)
  try:f.close()
  finally:
   for c in f.clusters:
    if c['socket'].exists():shutil.rmtree(c['socket'])
   proxy_dir2=getattr(f,'proxy_directory',None)
   if proxy_dir2 and proxy_dir2.exists():shutil.rmtree(proxy_dir2)
   stopped=all(not c['started'] for c in f.clusters)
   if status=='PASS' and stopped:
    for c in f.clusters:
     for d in [c['data'],c['directory']/'pre-upgrade',c['directory']/'committed-data']:
      if d.exists():shutil.rmtree(d)
   f.save('independent-cleanup.json',{'status':status,'fixture_clusters_stopped':stopped,'synthetic_pgdata_removed':status=='PASS' and stopped,'slice_cleanup_by_launcher':True})


def main():
 os.umask(0o077);p=argparse.ArgumentParser();p.add_argument('--core',type=Path,required=True);p.add_argument('--core-sha256',required=True);p.add_argument('--previous-core',type=Path,required=True);p.add_argument('--source',type=Path,required=True);p.add_argument('--reference',type=Path,required=True);p.add_argument('--output',type=Path,required=True);args=p.parse_args();args.mode='target-ack'
 if args.output.resolve().is_relative_to(TEST.parents[1]):raise SystemExit('evidence output must be outside the source repository')
 if digest(args.core)!=args.core_sha256:raise SystemExit('Core binary hash does not match frozen candidate')
 if digest(args.previous_core)!='6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31':raise SystemExit('predecessor binary mismatch')
 execute(args)
if __name__=='__main__':main()
