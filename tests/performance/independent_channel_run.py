#!/usr/bin/env python3
"""Independent actual Native ChannelRead gate; no author harness or decision oracle."""
import argparse, fcntl, hashlib, http.client, json, os
from pathlib import Path
import secrets, shutil, subprocess, sys, traceback
import independent_run as mechanics

FREEZE=Path('/home/lightjunction/Documents/Codex/2026-10-30-native-performance-ytm_vzye/compile-r2')
CORE_SHA='d6d356311de8cf6589c2ec4429603a0d9308ca634b1aed20b78a6c42b7c76c10'
RLIB_SHA='693984ccec2e06da3500dd5e128e19f0874fa3cf838a9d90d40079c1cc416945'
MECHANICS_SHA='252179be7165a5729d6d7101f78af4196f34b616316d58235dc8aa74632d686d'
READS=['/api/channel','/api/channel/','/api/channel/search?keyword=1','/api/channel/ops','/api/channel/1']
ACTIONS=['ChannelRead','ChannelOperate','ChannelWrite','ChannelSensitiveWrite','ChannelSecretView','AcquisitionRead','AcquisitionWrite','AcquisitionDetails','AcquisitionExport']
GROUPS=['fixture-policy-proof','admin-default','channel-deny-directory-allow','ordinary-key-anonymous-revoked','canonical-root','unsupported-writes','no-protected-state-change']
require=mechanics.require;sha=mechanics.sha;dump=mechanics.dump

class ChannelRun(mechanics.Run):
    def __init__(self,root):
        super().__init__(root);self.results={k:'NOT_RUN' for k in GROUPS};self.requests=[]
    def run(self,label,cmd,input=None,timeout=90,check=True):
        if label=='link-fixture':cmd=[self.root/'runner/independent_channel_fixture.rs' if str(x)==str(self.root/'runner/independent_fixture.rs') else x for x in cmd]
        result=super().run(label,cmd,input,timeout,check)
        if label=='link-fixture':dump(self.root/'helper-artifact.json',{'sha256':sha(self.root/'fixture')})
        return result
    def request(self,path,token,status,method='GET',label='channel'):
        self.seq+=1;p=self.log/f'{self.seq:03}-{label}'
        headers={'Authorization':'Bearer '+token} if token else {}
        conn=http.client.HTTPConnection('127.0.0.1',self.core_port,timeout=8)
        try:
            conn.request(method,path,headers=headers);response=conn.getresponse();body=response.read();actual=response.status;rh=dict(response.getheaders())
        finally:conn.close()
        entry={'method':method,'path':path,'expected':status,'actual':actual,'authenticated':bool(token),'headers':rh}
        self.requests.append(entry);dump(p.with_suffix('.json'),entry);p.with_suffix('.body').write_bytes(body)
        require(actual==status,f'{label}: {method} {path} expected {status}, got {actual}: {body[:500]!r}')
        value=json.loads(body)
        if path.startswith('/api/channel'):
            require(rh.get('cache-control')=='no-store','channel response must not be cached')
            require(value.get('success') is (status==200),'channel success flag contradicts HTTP')
            for forbidden in ['provider-model','synthetic-independent-only','credential','endpoint','base_url','127.0.0.1']:
                require(forbidden not in body.decode(),f'channel leaks protected routing data: {forbidden}')
            if status==403:require(value['code']=='CHANNEL_FORBIDDEN','wrong channel deny code')
            if status==401:require(value['code']=='AUTH_UNAUTHORIZED','wrong channel unauth code')
            if status==501:require(value['code']=='CHANNEL_OPERATION_UNAVAILABLE','write invented success')
        return value
    def session(self,name):return self.creds['sessions'][name]['secret']
    def read_matrix(self,name,expected):
        token=None if name=='anonymous' else self.creds['keys'][0]['secret'] if name=='key' else self.session(name)
        for path in READS:
            value=self.request(path,token,expected,label=name)
            if expected==200:
                d=value['data']
                require(d.get('read_only') is True,'channel must remain read-only')
                if path=='/api/channel/ops':
                    require(d['source']=='active_routing_snapshot','ops snapshot source')
                    require(d['capabilities']=={'list':True,'search':True,'read':True,'create':False,'update':False,'delete':False,'test':False,'balance':False,'fetch_models':False,'copy':False,'key':False},'ops capabilities broadened')
                else:
                    channels=[d] if path=='/api/channel/1' else d['items']
                    require(len(channels)==1,'expected exactly one configured channel')
                    channel=channels[0]
                    require(set(channel)=={'id','name','name_source','type','type_source','protocol','status','models','group','priority','read_only','metadata_source'},'unexpected channel fields')
                    require(channel['id']==1 and channel['models']=='local-model,other-model' and channel['group']=='allowed,秘密组','snapshot model/group projection')
                    require(channel['metadata_source']=='active_routing_snapshot' and channel['name_source']=='generated_from_upstream_id','fabricated legacy metadata')
    def policy_proof(self):
        payload=json.loads(self.sql("SELECT jsonb_agg(jsonb_build_object('user',p.user_id,'revision',p.revision,'contract',p.contract,'sha',encode(p.sha256,'hex'),'payload',encode(p.canonical_payload,'hex'),'verified',p.verified_complete,'rows',(SELECT jsonb_object_agg(action,effect) FROM core_identity.permission_policy_rows r WHERE r.user_id=p.user_id))) FROM core_identity.permission_policies p"))
        require({x['user'] for x in payload}=={9,11},'unexpected fixture policies')
        for doc in payload:
            rows={a:'deny' if a=='ChannelRead' else 'allow' for a in ACTIONS}
            raw=b'core.permissions.v1\0'+(1).to_bytes(8,'big')+b'\0'+doc['user'].to_bytes(8,'big')+b'\0'+b''.join(a.encode()+b'\0'+rows[a].encode()+b'\0' for a in ACTIONS)
            require(doc['rows']==rows and doc['verified'] and doc['revision']==1 and doc['contract']=='core.permissions.v1','incomplete policy fixture')
            require(doc['payload']==raw.hex() and doc['sha']==hashlib.sha256(raw).hexdigest(),'independent policy canonical/hash proof')
        dump(self.root/'policy-proof.json',payload)
    def snapshot(self,name):
        catalog=json.loads(self.sql("SELECT jsonb_agg(jsonb_build_array(n.nspname,c.relname,c.relkind) ORDER BY n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE (n.nspname LIKE 'core\\_%' ESCAPE '\\' OR n.nspname='public') AND c.relkind IN ('r','p','S')"))
        statements=['BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;']
        for schema,table,kind in catalog:
            require(all(ch.isalnum() or ch=='_' for ch in schema+table),'unsafe identifier')
            relation=f'"{schema}"."{table}"';label=schema+'.'+table
            if kind=='S':
                statements.append("SELECT jsonb_build_object('relation','"+label+"','kind','S','rows',jsonb_build_array(jsonb_build_object('last_value',last_value,'log_cnt',log_cnt,'is_called',is_called))) FROM "+relation+';')
            else:
                statements.append("SELECT jsonb_build_object('relation','"+label+"','kind','"+kind+"','rows',coalesce(jsonb_agg(row_to_json(t) ORDER BY row_to_json(t)::text),'[]'::jsonb)) FROM "+relation+' t;')
        statements.append('COMMIT;')
        output=self.sql('\n'.join(statements));rows=[json.loads(line) for line in output.splitlines()]
        state={r['relation']:{'kind':r['kind'],'row_count':len(r['rows']),'sha256':hashlib.sha256(json.dumps(r['rows'],sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()} for r in rows}
        require(len(state)==len(catalog) and len(state)>100,'incomplete protected relation inventory')
        dump(self.root/(name+'.json'),state);return state
    def execute(self):
        success=False
        try:
            self.prepare()
            self.results['fixture-policy-proof']='RUNNING';self.policy_proof();self.results['fixture-policy-proof']='PASS'
            before=self.snapshot('protected-before')
            self.results['admin-default']='RUNNING';self.read_matrix('admin',200);self.request('/core/v1/admin/users',self.session('admin'),200,label='directory-admin');self.results['admin-default']='PASS'
            self.results['channel-deny-directory-allow']='RUNNING';self.read_matrix('admin_channel_deny',403)
            directory=self.request('/core/v1/admin/users',self.session('admin_channel_deny'),200,label='directory-denied-channel-admin')
            dump(self.root/'directory-counterexample.json',{'channel_status':403,'directory_status':200,'directory_body_keys':list(directory)})
            self.results['channel-deny-directory-allow']='PASS'
            self.results['ordinary-key-anonymous-revoked']='RUNNING'
            for name,expected in [('ordinary',403),('key',403),('anonymous',401),('revoked_admin',401)]:self.read_matrix(name,expected)
            self.results['ordinary-key-anonymous-revoked']='PASS'
            self.results['canonical-root']='RUNNING'
            for name in ['root','root_channel_deny']:self.read_matrix(name,200)
            self.results['canonical-root']='PASS'
            self.results['unsupported-writes']='RUNNING'
            for method,path in [('POST','/api/channel'),('PUT','/api/channel/'),('DELETE','/api/channel/1')]:self.request(path,self.session('admin'),501,method,label='unsupported-write')
            self.results['unsupported-writes']='PASS'
            self.results['no-protected-state-change']='RUNNING';after=self.snapshot('protected-after')
            diff={name:{'before':before.get(name),'after':after.get(name)} for name in before.keys()|after.keys() if before.get(name)!=after.get(name)}
            dump(self.root/'protected-diff.json',diff);require(not diff,'protected tables/sequences changed during HTTP')
            require(len(self.supplier)==0,'channel reads or unsupported writes contacted supplier')
            self.results['no-protected-state-change']='PASS';success=True
        except Exception:
            (self.root/'failure.txt').write_text(traceback.format_exc())
            for k in self.results:
                if self.results[k]=='RUNNING':self.results[k]='FAIL'
        finally:
            dump(self.root/'results.json',self.results);dump(self.root/'http-matrix.json',self.requests)
            self.stop_core();self.release.set()
            if self.provider:self.provider.shutdown();self.provider.server_close()
            if self.pg:self.run('pg-stop',['pg_ctl','-D',self.root/'pg','-m','fast','-w','stop'],check=False)
            stopped={'core_exited':self.core is None or self.core.poll() is not None,'pg_stopped':not (self.root/'pg/postmaster.pid').exists(),'supplier_calls':len(self.supplier)}
            dump(self.root/'stopped.json',stopped);require(stopped['core_exited'] and stopped['pg_stopped'],'process cleanup failed')
        if success:
            removed={}
            for name in ['pg','s','lmm-core','fixture','owner.url','credentials.json','supplier.key','runtime.url']:
                path=self.root/name
                if not path.exists():continue
                removed[name]=sum(p.stat().st_size for p in path.rglob('*') if p.is_file()) if path.is_dir() else path.stat().st_size
                if path.is_dir():shutil.rmtree(path)
                else:path.unlink()
            for path in self.log.glob('*-seed.stdout'):
                digest=sha(path);dump(path,{'synthetic_credentials_removed':True,'original_sha256':digest})
            dump(self.root/'cleanup.json',{'logical_bytes_removed':sum(removed.values()),'paths':removed})
        return success

def launch(root):
    require(root.parent==Path('/home/lightjunction/Documents/Codex') and not root.exists(),'fresh short private root required')
    require(len(str(root/'s/.s.PGSQL.55603').encode())<104,'socket path too long')
    require(shutil.disk_usage(root.parent).free>2*1024**3,'disk admission')
    require(sha(FREEZE/'lmm-core')==CORE_SHA and sha(FREEZE/'liblmm_core.rlib')==RLIB_SHA,'R2 artifact mismatch')
    pins=json.loads((FREEZE/'source-pins.json').read_text());require(all(sha(FREEZE/'source'/n)==h for n,h in pins.items()),'frozen source mismatch')
    source=Path(__file__).parent;require(sha(source/'independent_run.py')==MECHANICS_SHA,'review mechanics change before reuse')
    root.mkdir(mode=0o700);(root/'runner').mkdir()
    for name in ['independent_channel_run.py','independent_channel_fixture.rs','independent_channel_README.md','independent_run.py']:shutil.copy2(source/name,root/'runner'/name)
    for name in ['source-pins.json','compile-config.json']:shutil.copy2(FREEZE/name,root/name)
    dump(root/'binding.json',{'freeze':str(FREEZE),'core_sha256':CORE_SHA,'rlib_sha256':RLIB_SHA,'source_count':len(pins),'source_pins_sha256':sha(FREEZE/'source-pins.json'),'compile_config_sha256':sha(FREEZE/'compile-config.json'),'runner':{p.name:sha(p) for p in (root/'runner').iterdir()},'author_harness_executed':False,'scope':'actual Native HTTP ChannelRead; synthetic native policies; not real historical import'})
    env={k:v for k,v in os.environ.items() if not k.startswith(('LMM_','PG','SQLX_')) and k not in ('DATABASE_URL','SQL_DSN','HTTP_PROXY','HTTPS_PROXY','ALL_PROXY')}
    env.update(XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}',DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus')
    suffix=secrets.token_hex(4);slice_name='lmmchannel_'+suffix+'.slice';scope='lmm-channel-independent-'+suffix
    unit=Path(env['XDG_RUNTIME_DIR'])/'systemd/user'/slice_name
    unit.write_text('[Unit]\nDescription=Independent actual Native channel acceptance\n[Slice]\nMemoryAccounting=yes\nMemoryMax=2G\nMemoryHigh=1536M\nMemorySwapMax=4G\nCPUQuota=100%\nTasksMax=256\n')
    cmd=['systemd-run','--user','--scope','--quiet','--unit='+scope,'--slice='+slice_name,'python3','-B',str(root/'runner/independent_channel_run.py'),'--root',str(root),'--worker']
    dump(root/'launch.json',{'argv':cmd,'slice':slice_name})
    try:
        subprocess.run(['systemctl','--user','daemon-reload'],env=env,check=True)
        code=subprocess.run(cmd,env=env).returncode
        with (root/'resources.txt').open('w') as out:subprocess.run(['systemctl','--user','show',slice_name,'-p','MemoryPeak','-p','MemoryMax','-p','MemoryHigh','-p','MemorySwapMax','-p','CPUQuotaPerSecUSec','-p','TasksMax'],env=env,stdout=out)
        subprocess.run(['systemctl','--user','stop',slice_name],env=env,check=True)
        with (root/'shutdown-units.txt').open('w') as out:subprocess.run(['systemctl','--user','show',slice_name,scope+'.scope','-p','Id','-p','ActiveState','-p','SubState'],env=env,stdout=out)
    finally:
        unit.unlink(missing_ok=True);subprocess.run(['systemctl','--user','daemon-reload'],env=env)
    dump(root/'launch-result.json',{'exit_code':code,'slice_removed':not unit.exists()})
    dump(root/'hash-manifest.json',{str(p.relative_to(root)):sha(p) for p in root.rglob('*') if p.is_file() and p.name!='hash-manifest.json'})
    print(json.dumps({'root':str(root),'exit':code,'results':json.loads((root/'results.json').read_text())}));return code

def main():
    os.umask(0o077);parser=argparse.ArgumentParser();parser.add_argument('--root',required=True,type=Path);parser.add_argument('--worker',action='store_true');args=parser.parse_args()
    mechanics.FREEZE=FREEZE;mechanics.CORE_SHA=CORE_SHA
    if not args.worker:return launch(args.root.absolute())
    with open('/tmp/lmm-delegate/heavy-build.lock','a') as lock:
        (args.root/'waiting-for-lock').write_text(str(os.getpid()));fcntl.flock(lock,fcntl.LOCK_EX);(args.root/'waiting-for-lock').unlink()
        memory=Path('/proc/meminfo').read_text();(args.root/'memory-admission.txt').write_text(memory)
        require(int(next(line.split()[1] for line in memory.splitlines() if line.startswith('MemAvailable:')))>=2097152,'2GiB admission')
        return 0 if ChannelRun(args.root).execute() else 1
if __name__=='__main__':sys.exit(main())
