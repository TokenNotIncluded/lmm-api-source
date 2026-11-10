#!/usr/bin/env python3
"""Prepared actual-main Core/Go fixture. Execute only in assigned PG window.
Synthetic provider HTTPS terminates only inside an empty network namespace.
No production connection, compiler, importer or permission bypass exists here.
"""
import argparse, base64, hashlib, http.client, http.server, ipaddress, json, os
import secrets, signal, socket, ssl, subprocess, sys, tempfile, threading, time, traceback
from datetime import datetime, timedelta, timezone
from pathlib import Path
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa, padding
from cryptography.x509.oid import NameOID

BASE=Path(__file__).resolve().parent
FROZEN=json.loads((BASE/'freeze.json').read_text())
CORE=Path(FROZEN['artifacts']['core']['path'])
GO=Path(FROZEN['artifacts']['go']['path'])
EXPECTED={str(Path(v['path'])):v['sha256'] for v in FROZEN['artifacts'].values()}
MER='MER_0123456789abcdefghijkl';STORE='STO_0123456789abcdefghijkl';PRODUCT='PROD_0123456789abcdefghijkl'
def sha(p):
 h=hashlib.sha256()
 with open(p,'rb') as f:
  for block in iter(lambda:f.read(1048576),b''):h.update(block)
 return h.hexdigest()
def freeport():
 with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
def network_guard(parent):
 current=os.readlink('/proc/self/ns/net')
 assert parent and parent.startswith('net:[') and parent.endswith(']'),'parent network namespace reference required'
 assert current!=parent,'network namespace must differ from launch parent'
 routes=Path('/proc/net/route').read_text().splitlines()[1:]
 assert not routes,'no external IPv4 routes allowed'
 links=json.loads(subprocess.check_output(['ip','-j','link','show'],text=True))
 assert [link['ifname'] for link in links]==['lo'],'loopback must be the only network interface'
 ipv6_defaults=json.loads(subprocess.check_output(['ip','-j','-6','route','show','default'],text=True))
 assert not ipv6_defaults,'no external IPv6 default routes allowed'
 return {'parent_namespace':parent,'current_namespace':current,'different':True,'interfaces':['lo'],'ipv4_routes':routes,'ipv6_default_routes':ipv6_defaults,'external_network':'forbidden'}
def main():
 assert '--execute' in sys.argv,'Preparation only; use RUN.sh in the assigned PG window'
 network=network_guard(os.environ.get('LMM_RSA_PARENT_NETNS'))
 assert os.environ.get('LMM_RSA_PARENT_MNTNS') and os.readlink('/proc/self/ns/mnt')!=os.environ['LMM_RSA_PARENT_MNTNS'],'private mount namespace required'
 assert Path('/etc/hosts').read_bytes()==(BASE/'hosts.fixture').read_bytes(),'private hosts mapping required'
 assert Path('/etc/nsswitch.conf').read_bytes()==(BASE/'nsswitch.fixture').read_bytes(),'private NSS files-only mapping required'
 assert {x[4][0] for x in socket.getaddrinfo('api.waffo.ai',443,type=socket.SOCK_STREAM)}=={'203.0.113.7'},'provider must resolve only to local namespace fixture'
 network['mount_namespace']=os.readlink('/proc/self/ns/mnt');network['parent_mount_namespace']=os.environ['LMM_RSA_PARENT_MNTNS'];network['provider_local_address']='203.0.113.7'
 for p,h in EXPECTED.items():assert sha(p)==h,(p,'binary drift')
 frozen=json.loads((BASE/'freeze.json').read_text())
 for name,digest in frozen['owned_files'].items():assert sha(BASE/name)==digest,(name,'frozen input drift')
 out=Path(tempfile.mkdtemp(prefix='attempt-',dir=BASE));out.chmod(0o700)
 env={k:os.environ[k] for k in ('PATH','HOME','LANG') if k in os.environ}
 env.update(GOMAXPROCS='1',GOMEMLIMIT='256MiB')
 commands=[];trace=[];children=[];pg=False;provider=None;report={'status':'RUNNING','binaries':EXPECTED,'cases':[],'limits':frozen['limits'],'cleanup':{}}
 def save(name,value):
  p=out/name;p.write_text(json.dumps(value,indent=2,ensure_ascii=False)+'\n');p.chmod(0o600)
 def private(name,data):
  p=out/name;p.write_bytes(data if isinstance(data,bytes) else data.encode());p.chmod(0o600);return p
 def cmd(name,args,data=None,custom=None):
  report['current_stage']=name;save('report.json',report)
  args=list(map(str,args));r=subprocess.run(args,input=data,capture_output=True,env=custom or env,timeout=90)
  private(name+'.stdout',r.stdout);private(name+'.stderr',r.stderr);commands.append({'stage':name,'command':args,'exit':r.returncode});save('commands.json',commands)
  assert r.returncode==0,(name,r.returncode,r.stderr.decode(errors='replace')[-1200:]);return r.stdout.decode().strip()
 pgport,coreport,goport=freeport(),freeport(),freeport();sock=Path(tempfile.mkdtemp(prefix='lmm-sub-rsa-'));sock.chmod(0o700)
 psql=['psql','-X','-q','-At','-v','ON_ERROR_STOP=1','-h',sock,'-p',pgport,'-U','fixture_admin']
 def sql(name,q,db='sub_core'):return cmd(name,psql+['-d',db],q.encode())
 def api(name,port,path,body=None,token=None,headers=None,expected=(200,),raw=None):
  report['current_stage']=name;save('report.json',report)
  h={'Content-Type':'application/json',**(headers or {})}
  if token:h['Authorization']='Bearer '+token
  payload=raw if raw is not None else json.dumps(body).encode() if body is not None else None
  method='POST' if payload is not None else 'GET';c=http.client.HTTPConnection('127.0.0.1',port,timeout=30)
  c.request(method,path,payload,h);r=c.getresponse();data=r.read();c.close();private(name+'.response',data)
  trace.append({'stage':name,'method':method,'port':port,'path':path,'status':r.status,'response_sha256':hashlib.sha256(data).hexdigest()});save('http-trace.json',trace)
  assert r.status in expected,(name,r.status,data[:1500]);return json.loads(data) if data else None
 def start(name,binary,cfg):
  log=open(out/(name+'.log'),'ab');p=subprocess.Popen([str(binary),'--config',str(cfg)],env=env,stdout=log,stderr=subprocess.STDOUT);children.append((p,log))
  save(name+'-process.json',{'pid':p.pid,'binary':str(binary),'sha256':EXPECTED[str(binary)]});return p
 def ready(p,port):
  for _ in range(100):
   assert p.poll() is None,'actual service terminated; inspect its log'
   try:
    with socket.create_connection(('127.0.0.1',port),.1):return
   except OSError:time.sleep(.1)
  raise AssertionError('actual service TCP readiness timeout')
 def case(name):report['cases'].append(name);save('report.json',report);print(name,flush=True)
 def snapshot(name):
  value={}
  for db,tables in [('sub_core',['core_subscriptions.external_orders','core_subscriptions.pancake_orders','core_subscriptions.pancake_events','core_subscriptions.pancake_cycles','core_charging.subscriptions']),('lmm_payments_test',['payment_pancake_subscription_inbox','payment_pancake_subscription_checkouts'])]:
   for table in tables:value[table]=json.loads(sql(name+'-'+table.replace('.','-'),f"SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM {table} t",db))
  save(name+'.json',value);return value
 try:
  save('network-isolation.json',network)
  save('handles.json',{'runner_pid':os.getpid(),'out':str(out),'pgport':pgport,'coreport':coreport,'goport':goport,'namespace':os.readlink('/proc/self/ns/net')})
  print('RUNNER_PID='+str(os.getpid())+' EVIDENCE='+str(out),flush=True)
  cmd('mount',['findmnt','-T',out]);cmd('space',['df','-B1',out])
  assert os.statvfs(out).f_bavail*os.statvfs(out).f_frsize>=512*1024*1024,'less than 512MiB actual target capacity'
  save('existing-attempts.json',[p.name for p in BASE.glob('attempt-*')])
  cmd('initdb',['initdb','-D',out/'pgdata','-U','fixture_admin','--auth=trust','--encoding=UTF8','--locale=C','--wal-segsize=1'])
  pg=True
  cmd('pg-start',['pg_ctl','-D',out/'pgdata','-l',out/'postgres.log','-o',f'-k {sock} -p {pgport} -c listen_addresses=127.0.0.1 -c shared_buffers=16MB -c max_connections=20 -c work_mem=1MB -c maintenance_work_mem=8MB -c max_worker_processes=0 -c max_parallel_workers=0 -c min_wal_size=2MB -c max_wal_size=32MB','-w','start'])
  save('postgres-process.json',{'pid':int((out/'pgdata/postmaster.pid').read_text().splitlines()[0])})
  sql('roles','CREATE ROLE lmm_core_owner NOLOGIN; CREATE ROLE core_installer LOGIN NOINHERIT; CREATE ROLE core_runtime LOGIN NOINHERIT; GRANT lmm_core_owner TO core_installer WITH INHERIT FALSE,SET TRUE; CREATE ROLE sub_go_owner NOLOGIN; CREATE ROLE go_installer LOGIN NOINHERIT; CREATE ROLE go_runtime LOGIN NOINHERIT; GRANT sub_go_owner TO go_installer WITH INHERIT FALSE,SET TRUE; CREATE DATABASE sub_core OWNER lmm_core_owner; CREATE DATABASE lmm_payments_test OWNER sub_go_owner;','postgres')
  for db,roles in [('sub_core','core_installer,core_runtime'),('lmm_payments_test','go_installer,go_runtime')]:sql('boundary-'+db,f'REVOKE ALL ON DATABASE {db} FROM PUBLIC; GRANT CONNECT ON DATABASE {db} TO {roles}; REVOKE CREATE ON SCHEMA public FROM PUBLIC;',db)
  def url(user,db,owner=None):
   from urllib.parse import quote
   return f'postgresql://{user}:synthetic@127.0.0.1:{pgport}/{db}?sslmode=disable'+('&options='+quote('-c role='+owner) if owner else '')
  own=private('owner.url',url('core_installer','sub_core','lmm_core_owner'));owncfg=private('owner.toml','version=1\n[database]\nurl_file='+json.dumps(str(own))+'\n')
  cmd('actual-core-init',[CORE,'--config',owncfg,'init-db'])
  manifest_raw=cmd('schema-manifest',[CORE,'schema-manifest']);schema=json.loads(manifest_raw);assert len(schema['components'])==65
  observed=json.loads(sql('components',"SELECT json_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract"));assert observed=={c['name']:c['raw_schema_sha256'] for c in schema['components']}
  assert json.loads(sql('identity-contract',"SELECT json_build_object('version',version,'fingerprint_sha256',encode(fingerprint,'hex')) FROM core_meta.schema_contract WHERE singleton"))==schema['identity']
  catalog=sql('actual-catalog',schema['catalog']['query_utf8']).splitlines();catalog_digest=hashlib.sha256()
  for row in catalog:
   encoded=row.encode();catalog_digest.update(len(encoded).to_bytes(8,'big'));catalog_digest.update(encoded)
  save('catalog-comparison.json',{'actual':catalog_digest.hexdigest(),'expected':'f104ea5933f8b6bc28108bd005f1e61785765657d530fdab701bc2a8a41afaa6','rows':len(catalog)})
  assert catalog_digest.hexdigest()=='f104ea5933f8b6bc28108bd005f1e61785765657d530fdab701bc2a8a41afaa6'
  goown=private('go-owner.url',url('go_installer','lmm_payments_test'))
  cmd('go-install',[GO,'install-db','--owner-url-file',goown,'--database','lmm_payments_test','--owner-role','sub_go_owner','--runtime-role','go_runtime','--component','payments'])
  password='P!'+secrets.token_hex(18);cmd('first-owner',[CORE,'--config',owncfg,'create-first-owner'],json.dumps({'username':'subowner','display_name':'Synthetic subscription owner','password':password}).encode())
  user=json.loads(cmd('bootstrap-user',[CORE,'--config',owncfg,'bootstrap-user'],b'{"user_id":2,"platform_level":0}'))['secret']
  other=json.loads(cmd('bootstrap-other',[CORE,'--config',owncfg,'bootstrap-user'],b'{"user_id":3,"platform_level":0}'))['secret']
  key=rsa.generate_private_key(public_exponent=65537,key_size=2048);merchant=rsa.generate_private_key(public_exponent=65537,key_size=2048)
  pem=lambda k:k.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.PKCS8,serialization.NoEncryption())
  pub=private('provider-public.pem',key.public_key().public_bytes(serialization.Encoding.PEM,serialization.PublicFormat.SubjectPublicKeyInfo));merchantfile=private('merchant.pem',pem(merchant))
  # Direct TLS at fixed public origin, bound only inside lo-only private netns.
  # The unchanged production transport deliberately disables proxy handling.
  ca=rsa.generate_private_key(public_exponent=65537,key_size=2048);name=x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'Private subscription fixture')]);now=datetime.now(timezone.utc)
  cert=x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(ca.public_key()).serial_number(x509.random_serial_number()).not_valid_before(now-timedelta(minutes=1)).not_valid_after(now+timedelta(days=1)).add_extension(x509.BasicConstraints(ca=True,path_length=None),True).add_extension(x509.SubjectAlternativeName([x509.DNSName('api.waffo.ai')]),False).sign(ca,hashes.SHA256())
  certfile=private('fixture-ca.pem',cert.public_bytes(serialization.Encoding.PEM));cafile=private('fixture-ca-key.pem',pem(ca));tls=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);tls.load_cert_chain(certfile,cafile)
  provider_calls=[]
  class Proxy(http.server.BaseHTTPRequestHandler):
   protocol_version='HTTP/1.1'
   def log_message(self,*a):pass
   def do_POST(self):
    assert self.headers['Host']=='api.waffo.ai','unexpected provider host'
    raw=self.rfile.read(int(self.headers['Content-Length']));body=json.loads(raw);provider_calls.append({'path':self.path,'body':body});save('synthetic-provider-calls.json',provider_calls)
    if self.path=='/v1/graphql':
     assert body['variables']['storeId']==STORE
     data={'onetimeProducts':[],'subscriptionProducts':[{'id':PRODUCT,'status':'active','billingPeriod':'weekly'}]}
    else:
     assert self.path=='/v1/actions/checkout/create-session',self.path
     assert body['productId']==PRODUCT and body['currency']=='USD' and body['priceSnapshot']['amount']=='1.25',body
     data={'sessionId':'SES_fixture_subscription','checkoutUrl':'https://checkout.example.test/subscription','expiresAt':(datetime.now(timezone.utc)+timedelta(minutes=30)).isoformat()}
    result=json.dumps({'data':data}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(result)));self.send_header('Connection','close');self.end_headers();self.wfile.write(result)
  provider=http.server.ThreadingHTTPServer(('203.0.113.7',443),Proxy);provider.socket=tls.wrap_socket(provider.socket,server_side=True);threading.Thread(target=provider.serve_forever,daemon=True).start()
  certdir=out/'empty-certs';certdir.mkdir(mode=0o700)
  env.update(SSL_CERT_FILE=str(certfile),SSL_CERT_DIR=str(certdir))
  control=private('identity.token',secrets.token_hex(32));payment=private('payment.token',secrets.token_hex(32));host=private('host.token',secrets.token_hex(32));refresh=private('refresh.key',secrets.token_bytes(32))
  pcfg=private('core-payments.toml',f'version=2\n[permissions]\nprepare=true\nread_user=true\nproof_ingress=true\nproof_read=true\nsubscription_epay_user_orders=true\nsubscription_epay_proof_ingress=true\nsubscription_epay_proof_read=true\nsubscription_epay_grant=true\n[[channels]]\nid=7\nprovider="waffo_pancake"\nmerchant="{MER}"\nenvironment="test"\ncurrency="USD"\ncredits_per_minor=1\nmax_amount_minor=10000\nwebhook_secret_file={json.dumps(str(pub))}\nstore_id="{STORE}"\nproduct_id="{PRODUCT}"\n')
  runtime=private('runtime.url',url('core_runtime','sub_core'))
  ccfg=private('core.toml',f'version=1\n[server]\nlisten="127.0.0.1:{coreport}"\nworker_threads=1\n[database]\nurl_file={json.dumps(str(runtime))}\nmax_connections=4\n[console]\nenabled=true\n[identity.web]\norigin="http://127.0.0.1:{coreport}"\nrefresh_key_file={json.dumps(str(refresh))}\nlocal_http=true\nuser_agreement_enabled=false\nprivacy_policy_enabled=false\n[rpc]\nsocket={json.dumps(str(sock/"core.sock"))}\ntoken_file={json.dumps(str(control))}\n[payments]\nconfig_file={json.dumps(str(pcfg))}\ntoken_file={json.dumps(str(payment))}\n')
  grantenv=dict(env,PGHOST=str(sock),PGPORT=str(pgport),PGUSER='core_installer',PGDATABASE='sub_core',PGOPTIONS='-c role=lmm_core_owner')
  cmd('runtime-grants',['psql','-X','-q','-v','ON_ERROR_STOP=1','-v','runtime_role=core_runtime','-v','enable_console=true','-v','enable_web_identity=true','-v','enable_events=true','-v','enable_payments=true','-f',BASE/'frozen/deployment/postgres/grant-core-runtime.sql'],custom=grantenv)
  check=json.loads(cmd('runtime-check',[CORE,'--config',ccfg,'check-db']));assert check['status']=='validated' and check['runtime_authority']['status']=='validated';assert check['execution']['executable_sha256']==EXPECTED[str(CORE)];case('actual65_init_and_unmodified_runtime_check')
  cp=start('core',CORE,ccfg);ready(cp,coreport)
  admin=api('login',coreport,'/core/v1/auth/login',{'login_name':'subowner','password':password})['secret']
  gp=private('go-payments.toml',f'version=2\n[[channels]]\nid=7\nprovider="waffo_pancake"\nmerchant="{MER}"\nenvironment="test"\ncurrency="USD"\nenabled=false\nsubscription_enabled=true\nprivate_key_file={json.dumps(str(merchantfile))}\nstore_id="{STORE}"\nproduct_id="{PRODUCT}"\nreturn_url="https://example.test/subscriptions"\ntax_category="saas"\n')
  gurl=private('go-runtime.url',url('go_runtime','lmm_payments_test'))
  gcfg=private('extensions.toml',f'version=1\n[host]\nlisten="127.0.0.1:{goport}"\ntoken_file={json.dumps(str(host))}\nmodules=["payments"]\nweb_compat=true\nmax_in_flight=4\n[core]\nsocket={json.dumps(str(sock/"core.sock"))}\nidentity_token_file={json.dumps(str(control))}\npayment_token_file={json.dumps(str(payment))}\n[payments]\ndatabase_name="lmm_payments_test"\ndatabase_url_file={json.dumps(str(gurl))}\nconfig_file={json.dumps(str(gp))}\ncallbacks_enabled=true\n')
  go=start('go',GO,gcfg);ready(go,goport)
  terms={'title':'Synthetic RSA weekly','price_units':'100','duration_seconds':604800,'duration_months':0,'reset_period':'grant','reset_seconds':0,'limit_units':'1000','models':['*'],'groups':['*'],'max_purchases_per_user':0,'enabled':True}
  plan=api('save-plan',coreport,'/core/v1/subscriptions/admin/plans',{'operation_key':'save_rsa_fixture_plan_0001','expected_revision':None,'terms':terms},admin)
  price={'plan_id':str(plan['id']),'plan_revision':str(plan['revision']),'channel_id':'7','currency':'USD','amount_minor':'125','store_id':STORE,'product_id':PRODUCT,'product_type':'subscription','billing_period':'weekly','provider_plan_reference':'PLAN_fixture_weekly','return_url':'https://example.test/subscriptions','checkout_region':'global','checkout_language':'en','tax_category':'saas'}
  api('save-price',coreport,f'/core/v1/subscriptions/admin/plans/{plan["id"]}/pancake-prices',{'operation_key':'save_rsa_fixture_price_0001','price':price},admin)
  path='/api/user/payments/subscriptions/pancake/orders';body={'plan_id':str(plan['id']),'channel_id':'7'};headers={'Idempotency-Key':'purchase_rsa_fixture_original_0001'}
  order=api('prepare',goport,path,body,user,headers);assert api('prepare-replay',goport,path,body,user,headers)==order
  oid=order['order']['order_id'];reference=order['provider_reference'];case('actual_go_prepare_core_rpc_and_idempotency')
  api('cross-user-denied',goport,path+'/'+oid,token=other,expected=(403,404));case('actual_authenticated_order_ownership_denial')
  confirmed=api('confirm',goport,path+'/'+oid+'/confirm',{},user);assert confirmed['checkout']['id']=='SES_fixture_subscription';assert len(provider_calls)==2
  api('confirm-replay',goport,path+'/'+oid+'/confirm',{},user);assert len(provider_calls)==2;case('actual_go_confirm_core_rpc_and_isolated_sdk_checkout')
  s=snapshot('confirmed-state');assert len(s['payment_pancake_subscription_checkouts'])==1 and s['payment_pancake_subscription_checkouts'][0]['checkout_id']=='SES_fixture_subscription'
  assert len(s['core_subscriptions.external_orders'])==1 and s['core_subscriptions.external_orders'][0]['confirmed_at'] is not None
  journals=sql('ledger-before',"SELECT count(*) FROM core_billing.ledger_journals")
  def stamp(seconds):return datetime.fromtimestamp(seconds,timezone.utc).isoformat().replace('+00:00','Z')
  period=int(time.time())-1
  def event(eid,paymentid,start):return {'id':eid,'eventType':'subscription.payment_succeeded','mode':'test','storeId':STORE,'timestamp':stamp(int(time.time())),'data':{'orderMerchantExternalId':reference,'orderId':'ORD_fixture_subscription','paymentId':paymentid,'currency':'USD','chargedAmount':'1.25','paymentStatus':'succeeded','currentPeriodStart':stamp(start),'currentPeriodEnd':stamp(start+604800),'billingPeriod':'weekly','orderMetadata':{'lmm_product_id':PRODUCT,'lmm_plan_id':'PLAN_fixture_weekly'}}}
  callback='/callbacks/subscriptions/pancake/7'
  deliveries={}
  def deliver(name,value=None,signer=key,expected=(200,),replay=None):
   if replay:raw,header=deliveries[replay]
   else:
    raw=json.dumps(value,separators=(',',':')).encode();ts=str(int(time.time()*1000));signature=base64.b64encode(signer.sign(ts.encode()+b'.'+raw,padding.PKCS1v15(),hashes.SHA256())).decode();header='t='+ts+',v1='+signature
   deliveries[name]=(raw,header);private(name+'.request',raw);private(name+'.rsa-signature',header)
   return api(name,goport,callback,raw=raw,headers={'X-Waffo-Signature':header},expected=expected)
  first=event('EVT_fixture_initial','PAY_fixture_initial',period)
  deliver('forged-rsa',first,merchant,(403,));s=snapshot('forged-state');assert len(s['core_subscriptions.pancake_cycles'])==0 and len(s['core_charging.subscriptions'])==0
  bad=event('EVT_fixture_wrong_amount','PAY_fixture_wrong_amount',period);bad['data']['chargedAmount']='1.24';denied=deliver('wrong-amount',bad,expected=(503,));assert denied['error']=='independent_provider_evidence_required';s=snapshot('wrong-amount-state');assert len(s['core_subscriptions.pancake_cycles'])==0 and len(s['core_charging.subscriptions'])==0
  rejected=[x for x in s['payment_pancake_subscription_inbox'] if x['event_id']=='EVT_fixture_wrong_amount'];assert len(rejected)==1 and rejected[0]['state']=='rejected' and rejected[0]['result'] is None
  receipt=deliver('paid',first);s=snapshot('first-paid');assert len(s['core_subscriptions.pancake_cycles'])==1 and len(s['core_charging.subscriptions'])==1;case('real_rsa_reject_and_core_subscription_grant_target_readback')
  deliver('paid-replay',replay='paid');s=snapshot('paid-replay-state');assert len(s['core_subscriptions.pancake_cycles'])==1 and len(s['core_charging.subscriptions'])==1
  go.terminate();go.wait(timeout=20);go=start('go-reopened',GO,gcfg);ready(go,goport);deliver('reopen-replay',replay='paid');s=snapshot('reopen-state');assert len(s['core_subscriptions.pancake_cycles'])==1;case('go_restart_and_signed_original_replay_no_duplicate_grant')
  second=event('EVT_fixture_renewal','PAY_fixture_renewal',period+604800);deliver('renewal',second);s=snapshot('renewal-state');assert len(s['core_subscriptions.pancake_cycles'])==2 and len(s['core_charging.subscriptions'])==2
  deliver('renewal-replay',replay='renewal');s=snapshot('final-state');assert len(s['core_subscriptions.pancake_cycles'])==2 and len(s['core_charging.subscriptions'])==2
  for name in ('paid','renewal'):
   raw,header=deliveries[name];rows=[x for x in s['payment_pancake_subscription_inbox'] if x['payload']=='\\x'+raw.hex() and x['signature']=='\\x'+header.encode().hex()];assert len(rows)==1 and rows[0]['state']=='accepted' and rows[0]['result'] is not None,(name,rows)
  for cycle in s['core_subscriptions.pancake_cycles']:
   grants=[x for x in s['core_charging.subscriptions'] if x['id']==cycle['subscription_id']];assert len(grants)==1
   assert grants[0]['anchor']==cycle['period_start'] and grants[0]['ends_at']==cycle['period_end'] and grants[0]['grant_reference']=='pancake-subscription:'+oid+':'+cycle['cycle_key']
  assert sql('ledger-after',"SELECT count(*) FROM core_billing.ledger_journals")==journals,'subscription callback unexpectedly mutated wallet ledger'
  api('original-order-read',goport,path+'/'+oid,token=user);case('signed_renewal_and_replay_target_cycles_entitlements_readback')
  report['status']='PASS'
 except BaseException as exc:
  report.update(status='FAIL',error=repr(exc),traceback=traceback.format_exc());raise
 finally:
  for p,log in reversed(children):
   if p.poll() is None:p.terminate()
   try:p.wait(timeout=20)
   except subprocess.TimeoutExpired:p.kill();p.wait(timeout=10)
   log.close()
  if provider:provider.shutdown()
  if pg:
   try:
    r=subprocess.run(['pg_ctl','-D',str(out/'pgdata'),'-m','fast','-w','stop'],env=env,capture_output=True,timeout=30);private('pg-stop.log',r.stdout+r.stderr);report['cleanup']['pg_stop_exit']=r.returncode
   except Exception as exc:report['cleanup']['pg_stop_error']=repr(exc)
  report['cleanup']['owned_pg_stopped']=not (out/'pgdata/postmaster.pid').exists();report['cleanup']['children_exited']=all(p.poll() is not None for p,_ in children)
  for p,h in EXPECTED.items():assert sha(p)==h,'binary changed during fixture'
  save('report.json',report)
if __name__=='__main__':main()
