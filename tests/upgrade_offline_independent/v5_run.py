#!/usr/bin/env python3
"""Real offline CLI acceptance over two synthetic, pinned v5/26 predecessors."""
import argparse, copy, hashlib, http.client, importlib.util, json, os, re, secrets, shutil, socket, subprocess, sys, time
from pathlib import Path
from argon2 import PasswordHasher

TEST=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('v5_fixture',TEST/'fixture.py');fixture=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixture)
spec=importlib.util.spec_from_file_location('v5_independent',TEST/'run.py');independent=importlib.util.module_from_spec(spec);spec.loader.exec_module(independent)
OLD={'fresh_v5_core26':('0037ded40a24e094abdfa035624e5612ad530a135962982308c3582399ec0412','3e15f119c022c1166504a099c34eb01f363a9c3a6a02c51b67f62fe6d226fad0',160,6301),'console_v1_v5_core26':('d7055821f0fd07b4afa610aa365ec4845e69986186c91673e4bfcd86b2c26c3a','581ac405489e4f4454a46eaf134a3bd84632c550c352486a67673cbd48b382ca',161,6355)}
TARGET='557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d'
class Failure(Exception):pass
class ProductFailure(Failure):pass
def require(ok,why):
 if not ok:raise Failure(why)
def sha(b):return hashlib.sha256(b).hexdigest()
def quote(s):return "'"+s.replace("'","''")+"'"
def framed(rows):
 h=hashlib.sha256()
 for row in rows:
  data=row.encode();h.update(len(data).to_bytes(8,'big'));h.update(data)
 return h.hexdigest()

def old_input(args,kind):
 base=args.source/'services/core';upgrade=base/'src/upgrade_offline';old=json.loads((upgrade/'console_predecessor.json').read_text());manifest=old['manifest'] if kind.startswith('fresh_') else json.loads((upgrade/'console_bridge_v1_successor.json').read_text())
 pin,catalog,tables,definitions=OLD[kind]
 raw=(json.dumps(manifest,separators=(',',':'),ensure_ascii=False)+'\n').encode() if kind.startswith('fresh_') else (upgrade/'console_bridge_v1_successor.json').read_bytes()
 require(sha(raw)==pin,'fixture manifest bytes mismatch'); require(len(manifest['components'])==26 and manifest['identity']['version']==5,'fixture old manifest shape')
 parts=['BEGIN;','SET LOCAL ROLE lmm_core_owner;'];pinned={}
 for component in manifest['components']:
  name=component['name'];file=name.replace('-','_')+'.sql'
  path=base/'schema'/file if name=='console' and not kind.startswith('fresh_') else args.historical/file
  data=path.read_bytes()
  if name=='trust-pricing':data+=b'\n'+(args.historical/'image_trust_extension-bridge_old.sql').read_bytes()
  require(sha(data)==component['raw_schema_sha256'],'fixture component mismatch: '+name);pinned[name]={'sha256':sha(data),'path':str(path)};parts.append(data.decode())
 text=(base/'src/core_schema.rs').read_text();match=re.search(r'const CONTRACT: &str = "(.*?)";',text);require(match is not None,'fixture missing installer contract');contract=match.group(1).encode();metadata=[(args.historical/n).read_bytes() for n in ('core_upgrade.sql','component_upgrade.sql','ledger_revision.sql')]
 if not kind.startswith('fresh_'):metadata.append((base/'src/core_schema/upgrade/console_receipt.sql').read_bytes())
 require(sha(contract+b''.join(metadata))==manifest['installer_contract_sql_sha256'],'fixture installer contract mismatch')
 parts.append(contract.decode());parts.extend(x.decode() for x in metadata)
 ident=manifest['identity'];parts.append(f"INSERT INTO core_meta.schema_contract(version,fingerprint) VALUES(5,decode('{ident['fingerprint_sha256']}','hex'));")
 for item in manifest['components']:parts.append(f"INSERT INTO core_meta.component_contract VALUES('{item['name']}',decode('{item['raw_schema_sha256']}','hex'));")
 parts.append('COMMIT;')
 return manifest,'\n'.join(parts),pinned

class Case(fixture.Fixture):
 def setup_v5(self,kind):
  self.target=self.cluster('target');self.sql(self.target,'CREATE DATABASE coretarget')
  self.run('roles',['psql','-X','-q','-v','ON_ERROR_STOP=1','-h',self.target['socket'],'-p',self.target['port'],'-U','postgres','-d','coretarget','-f',self.args.source/'deployment/postgres/create-core-roles.sql'])
  self.sql(self.target,'ALTER DATABASE coretarget OWNER TO lmm_core_owner')
  self.installer_password=secrets.token_hex(24)
  self.sql(self.target,"SET password_encryption='scram-sha-256'; CREATE ROLE lmm_core_installer LOGIN NOINHERIT PASSWORD "+quote(self.installer_password)+"; GRANT lmm_core_owner TO lmm_core_installer WITH INHERIT FALSE, SET TRUE; GRANT CONNECT ON DATABASE coretarget TO lmm_core_installer; CREATE ROLE lmm_extensions LOGIN; REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_control_system() TO lmm_core_owner",'coretarget')
  manifest,ddl,pins=old_input(self.args,kind);self.save('historical-inputs.json',pins);self.sql(self.target,ddl,'coretarget')
  rows=self.sql(self.target,manifest['catalog']['query_utf8'],'coretarget').splitlines();actual=framed(rows)
  counts=json.loads(self.sql(self.target,"SELECT json_build_object('tables',count(*) FILTER (WHERE c.relkind='r'),'sequences',count(*) FILTER (WHERE c.relkind='S')) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'",'coretarget'))
  expected=OLD[kind];self.save('predecessor-catalog.json',{'kind':kind,'manifest_sha256':expected[0],'actual_catalog':actual,'expected_catalog':expected[1],'definitions':len(rows),'counts':counts})
  require((actual,counts['tables'],counts['sequences'],len(rows))==(expected[1],expected[2],21,expected[3]),'fixture actual predecessor catalog/count mismatch')
  grants=(self.args.source/'services/core/src/identity/grants.rs').read_text();oauth=re.search(r'const CATALOG: &str = r#"(.*?)"#;',grants,re.S);require(oauth is not None,'fixture oauth query missing');oauthhash=framed(self.sql(self.target,oauth.group(1),'coretarget').splitlines());oauthddl=next(x['raw_schema_sha256'] for x in manifest['components'] if x['name']=='oauth-grants')
  self.sql(self.target,f"SET ROLE lmm_core_owner; INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES(decode('{actual}','hex')); INSERT INTO core_oauth.contract(version,ddl_fingerprint,catalog_fingerprint) VALUES(1,decode('{oauthddl}','hex'),decode('{oauthhash}','hex')); GRANT USAGE ON SCHEMA core_console TO lmm_core_runtime; GRANT SELECT ON core_console.options TO lmm_core_runtime;",'coretarget')
  if kind=='console_v1_v5_core26':
   self.sql(self.target,"""INSERT INTO core_meta.console_revisions(upgrade_id,procedure,request_sha256,predecessor_manifest_sha256,predecessor_catalog_sha256,successor_manifest_sha256,successor_catalog_sha256,added_catalog_entries,preserved_data_sha256,preserved_permissions_sha256,database_system_identifier,database_oid,database_name,installer_role,owner_role,runtime_role,extensions_role) SELECT 'synthetic-v1-receipt','core-26-console-public-options-v1',decode(repeat('11',32),'hex'),decode('0037ded40a24e094abdfa035624e5612ad530a135962982308c3582399ec0412','hex'),decode('3e15f119c022c1166504a099c34eb01f363a9c3a6a02c51b67f62fe6d226fad0','hex'),decode('d7055821f0fd07b4afa610aa365ec4845e69986186c91673e4bfcd86b2c26c3a','hex'),decode('581ac405489e4f4454a46eaf134a3bd84632c550c352486a67673cbd48b382ca','hex'),'[]'::jsonb,decode(repeat('22',32),'hex'),decode(repeat('33',32),'hex'),system_identifier::text,(SELECT oid FROM pg_database WHERE datname='coretarget'),'coretarget','lmm_core_installer','lmm_core_owner','lmm_core_runtime','lmm_extensions' FROM pg_control_system()""",'coretarget')
  self.password='synthetic bridge password';password_hash=PasswordHasher(time_cost=2,memory_cost=19456,parallelism=1,hash_len=32).hash(self.password)
  for i,(name,email,allowed) in enumerate([('legacy_allowed','Legacy.Alias@Example.test',True),('legacy_blocked','blocked@example.test',False),('legacy_dup_a','duplicate@example.test',True),('legacy_dup_b','duplicate@example.test',True),('verified_control','verified-control@example.test',False)],start=1):
   aid=int(self.sql(self.target,"INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id",'coretarget'))
   self.sql(self.target,f"INSERT INTO core_identity.users(id,personal_account_id) VALUES({9000+i},{aid}); INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES({quote(name)},{9000+i},{quote(password_hash)}); INSERT INTO core_identity.user_profiles(user_id,username,email,email_login_allowed) VALUES({9000+i},{quote(name)},{quote(email)},{str(allowed).lower()});",'coretarget')
  self.sql(self.target,"SELECT setval('core_identity.users_id_seq',9005,true); INSERT INTO core_console.options(key,value,revision) VALUES('Notice','preserve fixture',9); INSERT INTO core_console.initialization(version) VALUES('synthetic-v5'); INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT decode(repeat('ac',32),'hex'),'api_key',id,auth_version,personal_account_id,clock_timestamp()+interval '365 days' FROM core_identity.users WHERE id=9001",'coretarget')
  self.authority=self.cluster('authority');self.sql(self.authority,'CREATE ROLE history_owner NOLOGIN; CREATE ROLE history_writer LOGIN; GRANT CREATE ON DATABASE postgres TO history_owner')
  authority_id=sha(secrets.token_bytes(32));config=self.config('history-install',self.url(self.authority,'postgres','postgres'))
  self.run('history-install',[self.args.core,'--config',config,'upgrade-history'],json.dumps({'action':'install','authority_id':authority_id,'owner':'history_owner','writer':'history_writer'}).encode())
  self.config_path=self.config('operator',self.url(self.target,'lmm_core_installer','coretarget',self.installer_password));management=self.file('management.url',self.url(self.target,'postgres','postgres'));history=self.file('history.url',self.url(self.authority,'history_writer','postgres'));probe=self.file('probe.url',self.url(self.target,'lmm_core_runtime','coretarget'));self.secrets += [management,history,probe]
  identity=json.loads(self.sql(self.target,"SELECT json_build_object('system',system_identifier::text,'oid',(SELECT oid::bigint FROM pg_database WHERE datname='coretarget')) FROM pg_control_system()"))
  self.request={'version':3,'action':'apply','operation_id':'independent-v5-'+secrets.token_hex(16),'candidate_sha256':self.args.core_sha256,'authority_id':authority_id,'history_url_file':str(history),'management_url_file':str(management),'runtime_probe_url_file':str(probe),'hba_file':str(self.target['hba']),'runtime':{'console':True,'assistant_agent':False},'services':[],'bridge':{'version':3,'procedure':'core-offline-upgrade-v3','predecessor':kind,'predecessor_manifest_sha256':expected[0],'predecessor_catalog_sha256':expected[1],'successor_manifest_sha256':fixture.sha(self.args.source/'services/core/src/upgrade_offline/target.json'),'database_name':'coretarget','database_oid':identity['oid'],'system_identifier':identity['system'],'installer':'lmm_core_installer','owner':'lmm_core_owner','runtime':'lmm_core_runtime','extensions':'lmm_extensions'}}
  return manifest

def snapshot(f,query,columns=None,old_sequences=None):
 if columns is None:
  names=query("SELECT format('%I.%I',n.nspname,c.relname)||'|'||string_agg(format('%I',a.attname),',' ORDER BY a.attnum) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='r' GROUP BY n.nspname,c.relname ORDER BY n.nspname,c.relname").splitlines();columns=dict(x.split('|',1) for x in names)
 parts=[]
 for name,cols in columns.items():parts.append('SELECT '+quote(name)+" name,count(*) n,coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) payload FROM (SELECT "+cols+' FROM '+name+') t')
 rows=json.loads(query("SELECT jsonb_object_agg(name,jsonb_build_object('count',n,'sha256',encode(sha256(convert_to(payload::text,'UTF8')),'hex'))) FROM ("+' UNION ALL '.join(parts)+') x'))
 seqnames=query("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname,c.relname").splitlines()
 if old_sequences is not None:
  require(set(old_sequences).issubset(seqnames),'old sequence missing after upgrade')
  seqnames=sorted(old_sequences)
 require(len(seqnames)==21,'21 old sequences required')
 seq=json.loads(query("SELECT jsonb_object_agg(name,jsonb_build_array(value,log,is_called,definition)) FROM ("+' UNION ALL '.join("SELECT "+quote(n)+" name,last_value::text value,log_cnt::text log,is_called,(SELECT to_jsonb(s) - 'seqrelid' FROM pg_sequence s WHERE seqrelid="+quote(n)+"::regclass) definition FROM "+n for n in seqnames)+') s'))
 acl=sorted(independent.acl(f,query));contracts={n:json.loads(query('SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM '+n+' t')) for n in ('core_meta.component_contract','core_meta.schema_contract','core_meta.runtime_contract')}
 return {'columns':columns,'rows':rows,'sequences':seq,'acl':acl,'contracts':contracts}

def compare(before,after,target_manifest,pre_catalog,kind):
 exceptions={'core_meta.component_contract','core_meta.schema_contract','core_meta.runtime_contract'}
 changed=[name for name,entry in before['rows'].items() if name not in exceptions and after['rows'][name]!=entry];require(not changed,'old business rows changed '+str(changed))
 require(before['sequences']==after['sequences'],'old sequence state/definition changed')
 lost=sorted(set(before['acl'])-set(after['acl']));require(not lost,'old ACL removed '+str(lost[:3]))
 oldfacts=[json.loads(x) for x in before['acl']];newfacts=[json.loads(x) for x in after['acl']]
 schemas={x[1] for x in oldfacts if x[0]=='schema-owner'};relations={(x[1],x[2]) for x in oldfacts if x[0]=='relation-owner'};routines={(x[1],x[2]) for x in oldfacts if x[0]=='routine-owner'}
 def existing(x):
  return (x[0].startswith('schema-') and x[1] in schemas) or (x[0].startswith(('relation-','column-')) and (x[1],x[2]) in relations) or (x[0].startswith('routine-') and (x[1],x[2]) in routines)
 old_set={json.dumps(x,sort_keys=True) for x in oldfacts};added=sorted({json.dumps(x,sort_keys=True) for x in newfacts if existing(x)}-old_set)
 expected=[['column-grant','core_identity','session_authentication','user_id','lmm_core_runtime','UPDATE',False]]
 if kind=='console_v1_v5_core26':expected.extend([['routine-grant','core_console','core_console.lock_public_options()','lmm_core_runtime','EXECUTE',False],['routine-grant','core_console','core_console.write_public_option(text,text)','lmm_core_runtime','EXECUTE',False]])
 require(added==sorted(json.dumps(x,sort_keys=True) for x in expected),'unexpected added privileges on existing objects '+str(added))
 target={x['name']:'\\x'+x['raw_schema_sha256'] for x in target_manifest['components']};actual={x['name']:x['fingerprint'] for x in after['contracts']['core_meta.component_contract']};require(actual==target,'component receipts mismatch')
 oldschema=before['contracts']['core_meta.schema_contract'][0];schema=copy.deepcopy(oldschema);schema.update(version=6,fingerprint='\\x'+target_manifest['identity']['fingerprint_sha256']);require(after['contracts']['core_meta.schema_contract']==[schema],'identity contract transition wrong')
 oldruntime=before['contracts']['core_meta.runtime_contract'][0];require(oldruntime['catalog_fingerprint']=='\\x'+pre_catalog,'old runtime fingerprint wrong');runtime=copy.deepcopy(oldruntime);runtime['catalog_fingerprint']='\\x'+TARGET;require(after['contracts']['core_meta.runtime_contract']==[runtime],'runtime contract transition wrong')
 return {'old_tables':len(before['rows']),'populated':sum(x['count']>0 for x in before['rows'].values()),'old_sequences':len(before['sequences']),'old_acl_facts':len(before['acl']),'old_acl_removed':len(lost),'expected_grants_on_old_objects':[json.loads(x) for x in added],'old_business_rows_unchanged':True}

def runtime_checks(f,original_hba):
 # Fixture-only reopening: explicit administrative changes are not product finalization.
 f.sql(f.target,'ALTER DATABASE coretarget ALLOW_CONNECTIONS true; ALTER ROLE lmm_core_runtime LOGIN')
 f.target['hba'].write_bytes(original_hba)
 f.run('fixture-hba-reload',['pg_ctl','-D',f.target['data'],'reload'])
 f.run('fixture-runtime-grants',['psql','-X','-q','-v','ON_ERROR_STOP=1','-h',f.target['socket'],'-p',f.target['port'],'-U','postgres','-d','coretarget','-v','enable_console=true','-v','enable_web_identity=true','-f',f.args.source/'deployment/postgres/grant-core-runtime.sql'])
 grants=json.loads(f.sql(f.target,"SELECT json_build_object('offline_receipt_select',has_table_privilege('lmm_core_runtime','core_meta.offline_upgrades','SELECT'),'public_contracts_complete',(SELECT bool_and(has_table_privilege('lmm_core_runtime',format('core_meta.%I',name),'SELECT')) FROM unnest(ARRAY['schema_contract','component_contract','runtime_contract','schema_upgrades','component_upgrades','ledger_revisions']) AS name))",'coretarget'))
 require(grants=={'offline_receipt_select':False,'public_contracts_complete':True},'fixture grants differ from official authority matrix')
 profile_sql="SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY user_id),'[]')::text FROM core_identity.user_profiles p"
 password_sql="SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY user_id),'[]')::text FROM core_identity.password_logins p"
 old_profiles=f.sql(f.target,profile_sql,'coretarget');old_passwords=f.sql(f.target,password_sql,'coretarget')
 def key(name):
  path=f.root/name;path.write_bytes(secrets.token_bytes(32));path.chmod(0o600);return path
 refresh=key('fixture-refresh.key');proof=key('fixture-proof.key');rate=key('fixture-rate.key');mail=key('fixture-mail.key')
 sock=socket.socket();sock.bind(('127.0.0.1',0));port=sock.getsockname()[1];sock.close();origin=f'http://127.0.0.1:{port}'
 mailconfig=f.file('fixture-email.toml',f'challenge_key_id="proof-v1"\nproof_key_file={json.dumps(str(proof))}\nrate_key_file={json.dumps(str(rate))}\nactive_key_id="mail-v1"\nreset_origin={json.dumps(origin)}\nlocal_http=true\n[key_files]\nmail-v1={json.dumps(str(mail))}\n[smtp]\nhost="127.0.0.1"\nport=1\nfrom="synthetic@example.test"\ntls="plaintext_loopback_test"\n')
 url=f.file('fixture-runtime.url',f.url(f.target,'lmm_core_runtime','coretarget'))
 config=f.file('fixture-runtime.toml',f'version=1\n[server]\nlisten="127.0.0.1:{port}"\nworker_threads=1\ndrain_timeout_seconds=3\n[database]\nurl_file={json.dumps(str(url))}\nmax_connections=2\n[console]\nenabled=true\n[identity]\nemail_verification_enabled=false\nemail_domain_restriction_enabled=false\nemail_config_file={json.dumps(str(mailconfig))}\n[identity.web]\norigin={json.dumps(origin)}\nrefresh_key_file={json.dumps(str(refresh))}\nlocal_http=true\nuser_agreement_enabled=false\nprivacy_policy_enabled=false\n')
 log=f.root/'fixture-core-runtime.log';server=None
 try:
  with log.open('wb') as output:
   server=subprocess.Popen([str(f.args.core),'--config',str(config),'serve'],env=f.env,stdout=output,stderr=subprocess.STDOUT)
   deadline=time.monotonic()+30
   while True:
    if server.poll() is not None:raise Failure('fixture new Core failed to start; inspect fixture-core-runtime.log')
    try:
     connection=http.client.HTTPConnection('127.0.0.1',port,timeout=2);connection.request('GET','/health/ready');response=connection.getresponse();response.read();connection.close()
     if response.status==200:break
    except OSError:pass
    if time.monotonic()>deadline:raise Failure('fixture new Core not ready; inspect fixture-core-runtime.log')
    time.sleep(.1)
   def post(path,body):
    connection=http.client.HTTPConnection('127.0.0.1',port,timeout=15);connection.request('POST',path,json.dumps(body),{'Content-Type':'application/json','Origin':origin});response=connection.getresponse();data=response.read();status=response.status;connection.close()
    try:parsed=json.loads(data)
    except ValueError:parsed={}
    return status,parsed
   observed=[]
   for login,password,allowed in [('Legacy.Alias@Example.test',f.password,True),('legacy_blocked',f.password,True),('Legacy.Alias@Example.test','wrong old password',False),('blocked@example.test',f.password,False),('duplicate@example.test',f.password,False),('legacy.alias@example.test',f.password,False)]:
    status,body=post('/api/user/login',{'username':login,'password':password});ok=status==200 and body.get('success') is True
    if ok!=allowed or (not allowed and (status!=401 or body.get('code')!='AUTH_UNAUTHORIZED')):raise ProductFailure('runtime login mismatch '+login+' status='+str(status)+' body='+str({k:v for k,v in body.items() if k!='data'}))
    expected_user={'Legacy.Alias@Example.test':'legacy_allowed','legacy_blocked':'legacy_blocked'}.get(login) if allowed else None
    if allowed and (body.get('data',{}).get('user',{}).get('username')!=expected_user or body.get('data',{}).get('session',{}).get('login_method')!='password'):raise ProductFailure('runtime alias resolved to wrong user or method '+login)
    observed.append({'login':login,'status':status,'allowed':ok,'user':expected_user})
   resets=[]
   for address in ['legacy.alias@example.test','blocked@example.test','duplicate@example.test']:
    status,body=post('/api/identity/email/password-reset/request',{'email':address});
    if status!=202 or body.get('data',{}).get('status')!='accepted_if_eligible':raise ProductFailure('reset request unexpected status '+str(status)+' body='+str(body))
    status2,body2=post('/api/user/reset',{'email':address,'token':'00000000000000000000000000000000','new_password':'synthetic replacement password'})
    if status2!=400 or body2.get('error',{}).get('code')!='EMAIL_PROOF_INVALID':raise ProductFailure('unverified reset accepted or unexpected status '+str(status2)+' body='+str(body2))
    resets.append({'address':address,'request_status':status,'invalid_proof_status':status2})
   facts=json.loads(f.sql(f.target,"SELECT json_build_object('verified',(SELECT count(*) FROM core_identity.email_addresses),'challenges',(SELECT count(*) FROM core_identity.email_challenges),'outbox',(SELECT count(*) FROM core_identity.email_delivery_outbox),'promoted',(SELECT count(*) FROM core_identity.user_profiles WHERE email_version<>0))",'coretarget'))
   if facts!={'verified':0,'challenges':0,'outbox':0,'promoted':0} or f.sql(f.target,profile_sql,'coretarget')!=old_profiles or f.sql(f.target,password_sql,'coretarget')!=old_passwords:raise ProductFailure('unverified email reset changed identity authority/profile/password')
   # Separate fixture-owned verification event is a positive control; it is not migration output.
   f.sql(f.target,"UPDATE core_identity.user_profiles SET email_version=1 WHERE username='verified_control'; INSERT INTO core_identity.email_addresses(email_key,user_id,state,email_version,verified_at) SELECT email,user_id,'verified',1,clock_timestamp() FROM core_identity.user_profiles WHERE username='verified_control'",'coretarget')
   status,body=post('/api/identity/email/password-reset/request',{'email':'verified-control@example.test'})
   positive=json.loads(f.sql(f.target,"SELECT json_build_object('challenges',(SELECT count(*) FROM core_identity.email_challenges WHERE purpose='password_reset' AND user_id=9005),'outbox',(SELECT count(*) FROM core_identity.email_delivery_outbox))",'coretarget'))
   if status!=202 or body.get('data',{}).get('status')!='accepted_if_eligible' or positive!={'challenges':1,'outbox':1}:raise ProductFailure('verified positive control failed '+str(positive))
   result={'fixture_only_reopening':True,'new_core_sha256':f.args.core_sha256,'fixture_runtime_grants':grants,'login':observed,'unverified_reset':resets,'email_facts':facts,'password_and_profiles_unchanged':True,'post_migration_fixture_verified_positive_control':positive}
 finally:
  if server and server.poll() is None:
   server.terminate()
   try:server.wait(timeout=15)
   except subprocess.TimeoutExpired:server.kill();server.wait(timeout=5)
 f.sql(f.target,'REVOKE SELECT ON core_meta.component_contract FROM lmm_core_runtime','coretarget')
 try:denied=subprocess.run([str(f.args.core),'--config',str(config),'serve'],env=f.env,capture_output=True,timeout=15)
 except subprocess.TimeoutExpired:raise ProductFailure('runtime started without required public component_contract SELECT')
 if denied.returncode==0 or b'MissingPrivileges' not in denied.stderr:raise ProductFailure('missing public contract SELECT produced unexpected result: '+denied.stderr.decode(errors='replace')[:250])
 f.file('fixture-missing-contract.stderr',denied.stderr.decode(errors='replace'))
 result['missing_public_contract_select']={'exit':denied.returncode,'error':'MissingPrivileges'}
 # Disposable negative controls: each owner-only receipt's table and individual
 # column SELECT must reject startup. Revoke each grant after its probe.
 f.sql(f.target,'GRANT SELECT ON core_meta.component_contract TO lmm_core_runtime','coretarget')
 forbidden=[]
 for table,column in [('offline_upgrades','operation_id'),('console_revisions','upgrade_id')]:
  for level in ('table','column'):
   privilege='SELECT' if level=='table' else 'SELECT ('+column+')'
   f.sql(f.target,'GRANT '+privilege+' ON core_meta.'+table+' TO lmm_core_runtime','coretarget')
   try:excessive=subprocess.run([str(f.args.core),'--config',str(config),'serve'],env=f.env,capture_output=True,timeout=15)
   except subprocess.TimeoutExpired:raise ProductFailure('runtime started with owner-only '+table+' '+level+' SELECT')
   if excessive.returncode==0 or b'UnsafePrivileges' not in excessive.stderr:raise ProductFailure('owner-only '+table+' '+level+' SELECT produced unexpected result: '+excessive.stderr.decode(errors='replace')[:250])
   f.file('fixture-forbidden-'+table+'-'+level+'.stderr',excessive.stderr.decode(errors='replace'))
   f.sql(f.target,'REVOKE '+privilege+' ON core_meta.'+table+' FROM lmm_core_runtime','coretarget')
   forbidden.append({'table':table,'level':level,'exit':excessive.returncode,'error':'UnsafePrivileges','revoked_after_probe':True})
 result['fixture_temporary_forbidden_select']=forbidden
 return result

def main():
 p=argparse.ArgumentParser();p.add_argument('--core',type=Path,required=True);p.add_argument('--core-sha256',required=True);p.add_argument('--source',type=Path,required=True);p.add_argument('--historical',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--database-only',action='store_true');args=p.parse_args();args.mode='none';args.output.mkdir(mode=0o700)
 limits=independent.current_limits();require(limits['aggregate']=={'memory.max':'2147483648','memory.high':'1610612736','memory.swap.max':'4294967296','cpu.max':'100000 100000'},'resource limits wrong');require(int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>=2*1024**3,'insufficient available memory');require(shutil.disk_usage(args.output).free>=4*1024**3,'insufficient storage')
 (args.output/'resource-limits.json').write_text(json.dumps(limits,indent=2)+'\n');target_manifest=json.loads((args.source/'services/core/src/upgrade_offline/target.json').read_text())
 (args.output/'test-source-pins.json').write_text(json.dumps({p.name:fixture.sha(p) for p in [TEST/'v5_run.py',TEST/'v5_launch.py',TEST/'fixture.py',TEST/'run.py']},indent=2)+'\n')
 for kind in OLD:
  args.output_original=args.output;args.output=args.output_original/kind;f=Case(args);success=False
  try:
   print('setup',kind,flush=True);manifest=f.setup_v5(kind);original_hba=f.target['hba'].read_bytes();before=snapshot(f,lambda q:f.sql(f.target,q,'coretarget'));f.save('before.json',before)
   require(len(before['columns'])==OLD[kind][2], 'old table count wrong')
   p,out=independent.request_call(f,'actual-apply',f.request)
   if p.returncode!=0 or not out or out.get('status')!='committed':raise ProductFailure('CLI apply rejected: '+str(out)+' exit='+str(p.returncode))
   history=independent.state(f);require(history['external_history'][0]['phase']=='committed' and not history['target_database']['allow_connections'],'history or fence wrong after commit')
   after=snapshot(f,lambda q:f.target_sql(q),before['columns'],before['sequences']);f.save('after.json',after);preserved=compare(before,after,target_manifest,OLD[kind][1],kind)
   target_rows=f.target_sql(target_manifest['catalog']['query_utf8']).splitlines();actual_target=framed(target_rows)
   f.save('successor-catalog.json',{'actual':actual_target,'expected':TARGET,'definitions':len(target_rows)})
   require(actual_target==TARGET,'independent target catalog mismatch')
   receipt=json.loads(f.target_sql("SELECT json_build_object('count',count(*),'catalog',max(successor_catalog_sha256),'operation',max(operation_id)) FROM core_meta.offline_upgrades"));require(receipt=={'count':1,'catalog':TARGET,'operation':f.request['operation_id']},'target receipt wrong')
   v1_count=int(f.target_sql('SELECT count(*) FROM core_meta.console_revisions'));require(v1_count==(1 if kind=='console_v1_v5_core26' else 0),'historical console receipt changed or invented')
   email=json.loads(f.target_sql("SELECT json_build_object('verified', (SELECT count(*) FROM core_identity.email_addresses), 'challenges',(SELECT count(*) FROM core_identity.email_challenges), 'outbox',(SELECT count(*) FROM core_identity.email_delivery_outbox), 'promoted',(SELECT count(*) FROM core_identity.user_profiles WHERE email_version<>0))"));require(email=={'verified':0,'challenges':0,'outbox':0,'promoted':0},'legacy email promoted')
   recover=copy.deepcopy(f.request);recover['action']='recover';_,recovery=f.cli('actual-recover',f.config_path,recover);require(recovery['status']=='committed_history_observed' and recovery['site_open_allowed'] is False and recovery['retry_allowed'] is False,'committed recovery wrong')
   independent.expected_reject(f,'v5-committed-replay-refused',f.request,'OFFLINE_STARTED_RECOVER_ONLY')
   f.save('acceptance.json',{'status':'DATABASE_PASS_RUNTIME_PENDING','scope':'services=[]; no old v5 Core binary; product finalization not run','apply':out,'history':history,'recovery':recovery,'receipt':receipt,'preservation':preserved,'email':email,'manifest_sha256':OLD[kind][0],'catalog':OLD[kind][1]});print('database-pass',kind,json.dumps(preserved),flush=True)
   if args.database_only:
    f.save('result.json',{'status':'DATABASE_PASS_RUNTIME_PENDING','scope':'services=[]; runtime and product finalization not tested','kind':kind,'old_catalog':OLD[kind][1],'old_tables':OLD[kind][2],'old_sequences':21,'core_sha256':args.core_sha256})
   else:
    runtime=runtime_checks(f,original_hba);f.save('runtime-check.json',runtime)
    f.save('result.json',{'status':'PASS','scope':'database and explicitly fixture-opened new Core; services=[]; no product finalization','kind':kind,'old_catalog':OLD[kind][1],'old_tables':OLD[kind][2],'old_sequences':21,'core_sha256':args.core_sha256});print('runtime-pass',kind,json.dumps(runtime),flush=True)
   success=True
  except BaseException as e:
   f.save('result.json',{'status':'FAIL' if isinstance(e,ProductFailure) else 'BLOCKED','failure':str(e),'kind':kind,'old_catalog':OLD[kind][1],'old_tables':OLD[kind][2],'old_sequences':21,'core_sha256':args.core_sha256,'stop_dependent_cases':True});print('STOP',kind,type(e).__name__,str(e),flush=True);raise
  finally:
   f.close()
   for c in f.clusters:
    if c['socket'].exists():shutil.rmtree(c['socket'])
    if success and not c['started'] and c['data'].exists():shutil.rmtree(c['data'])
   args.output=args.output_original
if __name__=='__main__':main()
