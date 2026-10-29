#!/usr/bin/env python3
"""Freeze captured Native + committed Web, then run only the independent contract."""
import argparse,fcntl,hashlib,json,os
from pathlib import Path
import secrets,shutil,subprocess,sys

REPO=Path(__file__).resolve().parents[2]
COMMIT='24066ba4c2204a9bf6173c97f4ff2d8196512401'
NATIVE=Path('/home/lightjunction/Documents/Codex/pi1030a2')
BUN=Path('/home/lightjunction/.local/share/mise/installs/bun/1.3.14/bin/bun')
NODE=Path('/home/lightjunction/.local/share/mise/installs/node/26.10.0/bin/node')
PAYLOADS={'summary-empty':'logs/013-performance.body','summary-missing':'logs/081-performance.body','detail-hour':'logs/028-performance.body','detail-minute':'logs/048-performance.body','detail-five-minute':'logs/054-performance.body','detail-empty':'logs/087-performance.body','rows-final':'rows-final.json'}
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def dump(p,v):p.write_text(json.dumps(v,ensure_ascii=False,indent=2)+'\n')
def require(ok,msg):
 if not ok:raise AssertionError(msg)
def environment(root):
 env={k:v for k,v in os.environ.items() if not k.startswith(('LMM_','PG','SQLX_')) and k not in ('DATABASE_URL','SQL_DSN','HTTP_PROXY','HTTPS_PROXY','ALL_PROXY')}
 env.update(PATH=f'{BUN.parent}:{NODE.parent}:/usr/bin:/bin',TZ='Asia/Taipei',INDEPENDENT_WEB_EVIDENCE=str(root),TMPDIR=str(root/'tmp'),XDG_CACHE_HOME=str(root/'cache'),BUN_RUNTIME_TRANSPILER_CACHE_PATH=str(root/'cache/bun'),NODE_OPTIONS='--max-old-space-size=768',XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}',DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus')
 return env

def freeze(root):
 require(root.parent==Path('/home/lightjunction/Documents/Codex') and not root.exists(),'fresh direct evidence root required')
 require(shutil.disk_usage(root.parent).free>1024**3,'space admission')
 require(subprocess.check_output([BUN,'--version'],text=True).strip()=='1.3.14','wrong Bun')
 require(subprocess.check_output([NODE,'--version'],text=True).strip()=='v26.10.0','wrong Node')
 require((REPO/'node_modules/react').is_dir() and (REPO/'node_modules/happy-dom').is_dir(),'existing dependencies missing; do not install')
 full_commit=subprocess.check_output(['git','rev-parse',COMMIT],cwd=REPO,text=True).strip()
 entries=subprocess.check_output(['git','ls-tree','-rz',full_commit,'--','apps/web','bun.lock','package.json'],cwd=REPO).split(b'\0')
 source={}
 for entry in entries:
  if not entry:continue
  meta,name=entry.split(b'\t',1);mode,kind,oid=meta.split();relative=name.decode()
  require(kind==b'blob' and mode in (b'100644',b'100755'),'unsupported frozen file '+relative)
  data=(REPO/relative).read_bytes();require(hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()==oid.decode(),'current Web differs from committed blob: '+relative)
  source[relative]={'sha256':hashlib.sha256(data).hexdigest(),'git_blob':oid.decode()}
 root.mkdir(mode=0o700)
 for name in ['runner','payloads','source','tmp','cache']:(root/name).mkdir()
 for relative,proof in source.items():
  dest=root/'source'/relative;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(REPO/relative,dest);require(sha(dest)==proof['sha256'],'pre-lock source drift '+relative)
 for name in ['independent_web_run.py','independent_web_native.test.tsx','independent_web_README.md']:shutil.copy2(Path(__file__).parent/name,root/'runner'/name)
 test=root/'source/apps/web/src/features/performance-metrics/independent-native.test.tsx';shutil.copy2(root/'runner/independent_web_native.test.tsx',test)
 original=json.loads((NATIVE/'hash-manifest-all.json').read_text());payloads={}
 for label,relative in PAYLOADS.items():
  require(sha(NATIVE/relative)==original[relative],'archived Native evidence mismatch '+relative)
  shutil.copyfile(NATIVE/relative,root/'payloads'/(label+'.json'))
  payloads[label]={'source':str(NATIVE/relative),'sha256':sha(NATIVE/relative)}
 shutil.copy2(NATIVE/'binding.json',root/'native-binding.json')
 # Existing workspace dependencies only. Never depend on the disposable wp1030a1 copy.
 (root/'source/node_modules').symlink_to(REPO/'node_modules',target_is_directory=True)
 dependencies={}
 for name in ['react','react-dom','happy-dom','i18next','react-i18next','@tanstack/react-query','@tanstack/query-core','@base-ui/react','lucide-react','class-variance-authority','clsx','tailwind-merge']:
  p=REPO/'node_modules'/name/'package.json';dependencies[name]={'package_json_sha256':sha(p),'version':json.loads(p.read_text())['version']}
 dump(root/'source-pins.json',source)
 dump(root/'binding.json',{'commit':full_commit,'web_input_count':len(source),'source_pins_sha256':sha(root/'source-pins.json'),'native_original_manifest_sha256':sha(NATIVE/'hash-manifest-all.json'),'payloads':payloads,'native_binding_sha256':sha(root/'native-binding.json'),'bun':{'path':str(BUN),'version':'1.3.14','sha256':sha(BUN)},'node':{'path':str(NODE),'version':'26.10.0','sha256':sha(NODE)},'dependency_root':str(REPO/'node_modules'),'bun_lock_sha256':sha(REPO/'bun.lock'),'dependencies':dependencies,'runner':{p.name:sha(p) for p in (root/'runner').iterdir()},'test_frozen_sha256':sha(test),'scope':'captured Native -> API adapter -> real React SSR/happy-dom data contract, no browser/live HTTP/login'})
 return full_commit

def worker(root):
 with open('/tmp/lmm-delegate/heavy-build.lock','a') as lock:
  (root/'waiting-for-lock').write_text(str(os.getpid()));fcntl.flock(lock,fcntl.LOCK_EX);(root/'waiting-for-lock').unlink()
  memory=Path('/proc/meminfo').read_text();(root/'memory-admission.txt').write_text(memory);require(int(next(l.split()[1] for l in memory.splitlines() if l.startswith('MemAvailable:')))>=2097152,'2GiB admission')
  bind=json.loads((root/'binding.json').read_text());pins=json.loads((root/'source-pins.json').read_text())
  def verify():
   require(all(sha(root/'source'/n)==v['sha256'] for n,v in pins.items()),'frozen Web drift')
   require(sha(root/'source/apps/web/src/features/performance-metrics/independent-native.test.tsx')==bind['test_frozen_sha256'],'test drift')
   require(all(sha(root/'payloads'/(n+'.json'))==v['sha256'] for n,v in bind['payloads'].items()),'payload drift')
   for name,v in bind['dependencies'].items():require(sha(Path(bind['dependency_root'])/name/'package.json')==v['package_json_sha256'],'dependency metadata drift '+name)
   require(sha(BUN)==bind['bun']['sha256'] and sha(NODE)==bind['node']['sha256'],'runtime drift')
  verify();env=environment(root)
  cmd=[str(BUN),'test','--bail','--preload','./scripts/test-preload.mjs','./src/features/performance-metrics/independent-native.test.tsx']
  dump(root/'test-command.json',{'argv':cmd,'cwd':str(root/'source/apps/web'),'env_explicit':{k:env[k] for k in ['PATH','TZ','NODE_OPTIONS','INDEPENDENT_WEB_EVIDENCE','BUN_RUNTIME_TRANSPILER_CACHE_PATH']}})
  result=subprocess.run(cmd,cwd=root/'source/apps/web',env=env,capture_output=True,timeout=90)
  (root/'test.stdout').write_bytes(result.stdout);(root/'test.stderr').write_bytes(result.stderr)
  verify();dump(root/'result.json',{'exit_code':result.returncode,'status':'PASS' if result.returncode==0 else 'FAIL','frozen_inputs_unchanged':True,'groups':json.loads((root/'results.json').read_text()) if (root/'results.json').exists() else {},'bun':'1.3.14','node':'26.10.0','author_tests_run':False})
  return result.returncode

def main():
 os.umask(0o077);p=argparse.ArgumentParser();p.add_argument('--root',type=Path,required=True);p.add_argument('--worker',action='store_true');args=p.parse_args();root=args.root.absolute()
 if args.worker:return worker(root)
 freeze(root);env=environment(root);suffix=secrets.token_hex(4);slice_name='lmmwebcontract_'+suffix+'.slice';scope='lmm-web-contract-'+suffix
 unit=Path(env['XDG_RUNTIME_DIR'])/'systemd/user'/slice_name
 unit.write_text('[Unit]\nDescription=Independent Native Web data contract\n[Slice]\nMemoryAccounting=yes\nMemoryMax=2G\nMemoryHigh=1536M\nMemorySwapMax=4G\nCPUQuota=100%\nTasksMax=256\n')
 cmd=['systemd-run','--user','--scope','--quiet','--unit='+scope,'--slice='+slice_name,str(Path(sys.executable).resolve()),'-B',str(root/'runner/independent_web_run.py'),'--root',str(root),'--worker']
 dump(root/'launch.json',{'argv':cmd,'slice':slice_name})
 try:
  subprocess.run(['systemctl','--user','daemon-reload'],env=env,check=True)
  code=subprocess.run(cmd,env=env).returncode
  with (root/'resources.txt').open('w') as out:subprocess.run(['systemctl','--user','show',slice_name,'-p','MemoryPeak','-p','MemoryMax','-p','MemoryHigh','-p','MemorySwapMax','-p','CPUQuotaPerSecUSec','-p','TasksMax'],env=env,stdout=out)
  subprocess.run(['systemctl','--user','stop',slice_name],env=env,check=True)
  with (root/'shutdown-units.txt').open('w') as out:subprocess.run(['systemctl','--user','show',slice_name,scope+'.scope','-p','Id','-p','ActiveState','-p','SubState'],env=env,stdout=out)
 finally:unit.unlink(missing_ok=True);subprocess.run(['systemctl','--user','daemon-reload'],env=env)
 dump(root/'launch-result.json',{'exit_code':code,'slice_removed':not unit.exists()})
 dump(root/'hash-manifest.json',{str(p.relative_to(root)):sha(p) for p in sorted(root.rglob('*')) if p.is_file() and not p.is_symlink() and p.name!='hash-manifest.json'})
 print(json.dumps({'root':str(root),'exit_code':code}));return code
if __name__=='__main__':sys.exit(main())
