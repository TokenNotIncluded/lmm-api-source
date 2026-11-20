#!/usr/bin/env python3
"""Bounded launcher for one real official-candidate browser fixture; no builds."""
from pathlib import Path
import argparse, hashlib, http.client, json, os, signal, subprocess, sys, time
from urllib.parse import urlparse
os.umask(0o077)
ROOT=Path(os.environ['LMM_ACCEPTANCE_ROOT'])
TESTS=Path(__file__).resolve().parent
NODE=os.environ['LMM_ACCEPTANCE_NODE']
def save(path,value):
 path.write_text(json.dumps(value,sort_keys=True,indent=2)+'\n');path.chmod(0o600)
def control(cfg,action,label=None):
 u=urlparse(cfg['controlOrigin'])
 assert u.scheme=='http' and u.hostname=='127.0.0.1' and u.port
 body={'action':action}
 if label is not None:body['label']=label
 c=http.client.HTTPConnection('127.0.0.1',u.port,timeout=15)
 try:
  c.request('POST','/__acceptance/control',json.dumps(body),{'Content-Type':'application/json','X-Acceptance-Key':cfg['controlKey']})
  r=c.getresponse();raw=r.read()
  value=json.loads(raw)
  assert r.status==200 and value.get('completed') is True
  return {'action':action,'status':r.status,'completed':True}
 finally:c.close()
parser=argparse.ArgumentParser()
parser.add_argument('--round',required=True)
parser.add_argument('--candidate',type=Path,required=True)
args=parser.parse_args()
assert args.round.startswith('browser-') and '/' not in args.round and '..' not in args.round
freeze_path=TESTS/'preparation.json'
freeze=json.loads(freeze_path.read_text())
assert freeze['status']=='FROZEN_NOT_EXECUTED'
for name,want in freeze['files'].items():
 p=TESTS/name
 assert hashlib.sha256(p.read_bytes()).hexdigest()==want['sha256'],name+' changed after independent review'
candidate=args.candidate.resolve(strict=True)
assert json.loads(candidate.read_text())['source_commit']=='293e53f799b09f187893e9cdf0c3503973916676'
roundroot=ROOT/args.round
assert not roundroot.exists(),'never overwrite prior round evidence'
launcher=ROOT/(args.round+'-launcher');launcher.mkdir(mode=0o700)
started=time.time();deadline=started+1800
fixture=None;browser=None;cfg=None;ready=None;failure=None;browser_exit=None;cleanup_requests=[]
save(launcher/'inputs.json',{'started_at':started,'maximum_total_seconds':1800,'minimum_cleanup_reserve_seconds':150,'candidate':str(candidate),'candidate_sha256':hashlib.sha256(candidate.read_bytes()).hexdigest(),'tests_freeze_sha256':hashlib.sha256(freeze_path.read_bytes()).hexdigest(),'tests':freeze['files']})
fixture_log=(launcher/'fixture-driver.log').open('wb')
browser_log=(launcher/'browser-driver.log').open('wb')
try:
 fixture=subprocess.Popen([sys.executable,str(TESTS/'fixture.py'),'--round',args.round,'--candidate',str(candidate),'--max-seconds','1795'],cwd=TESTS,stdout=fixture_log,stderr=subprocess.STDOUT,start_new_session=True)
 ready_path=roundroot/'fixture-ready.json'
 while not ready_path.exists():
  assert fixture.poll() is None,'fixture failed before readiness'
  assert time.time()<deadline-1200,'startup used the real expiry coverage budget'
  time.sleep(.5)
 ready=json.loads(ready_path.read_text())
 cfg=json.loads((roundroot/'browser-private.json').read_text())
 assert cfg['evidence']==str(roundroot)
 cleanup_deadline=min(deadline,float(ready['deadline_epoch']))
 browser_deadline=cleanup_deadline-150
 save(launcher/'running.json',{'fixture_driver_pid':fixture.pid,'fixture_ready':True,'browser_deadline_epoch':browser_deadline,'fixture_deadline_epoch':cleanup_deadline,'runtime_pids':{k:v for k,v in ready.items() if k.endswith('_pid')}})
 browser=subprocess.Popen([NODE,str(TESTS/'browser.mjs')],cwd=TESTS,env={**os.environ,'LMM_ACCEPTANCE_ROUND':args.round},stdout=browser_log,stderr=subprocess.STDOUT,start_new_session=True)
 print(json.dumps({'phase':'browser-started','round':args.round,'fixture_driver_pid':fixture.pid,'browser_driver_pid':browser.pid,'browser_deadline_epoch':browser_deadline}),flush=True)
 while browser.poll() is None:
  assert fixture.poll() is None,'fixture exited during browser acceptance'
  assert time.time()<browser_deadline,'browser exhausted bounded budget; cleanup reserve retained'
  time.sleep(.5)
 browser_exit=browser.returncode
 assert browser_exit==0,'browser failed; immutable observations are in its result.json'
except Exception as error:
 failure=type(error).__name__+': '+str(error)
 save(launcher/'failure.json',{'failure':failure,'observed_at':time.time()})
finally:
 if browser is not None and browser.poll() is None:
  # Signal only this exact launcher-owned process group, including its Chromium.
  os.killpg(browser.pid,signal.SIGTERM)
  try:browser.wait(timeout=10)
  except subprocess.TimeoutExpired:
   os.killpg(browser.pid,signal.SIGKILL);browser.wait(timeout=10)
 if cfg is not None and fixture is not None and fixture.poll() is None:
  for action,label in [('snapshot','before-cleanup'),('resources','before-cleanup'),('stop',None)]:
   try:cleanup_requests.append(control(cfg,action,label))
   except Exception as error:cleanup_requests.append({'action':action,'completed':False,'error':type(error).__name__})
 if fixture is not None and fixture.poll() is None:
  try:fixture.wait(timeout=25)
  except subprocess.TimeoutExpired:
   fixture.terminate()
   try:fixture.wait(timeout=25)
   except subprocess.TimeoutExpired:failure=failure or 'fixture driver did not finish its normal cleanup'
 fixture_log.close();browser_log.close()
 cleanup=None
 if (roundroot/'fixture-cleanup.json').exists():cleanup=json.loads((roundroot/'fixture-cleanup.json').read_text())
 surviving=[]
 if ready:
  for key,pid in ready.items():
   if key.endswith('_pid') and Path('/proc',str(pid)).exists():surviving.append({'role':key,'pid':pid})
 hashes={}
 for name,want in freeze['files'].items():
  actual=hashlib.sha256((TESTS/name).read_bytes()).hexdigest()
  hashes[name]={'sha256':actual,'unchanged':actual==want['sha256']}
 result={'overall':'PASS' if failure is None and browser_exit==0 and cleanup and cleanup['all_stopped'] and cleanup.get('failure') is None and fixture.poll()==0 and time.time()<=deadline and not surviving and all(v['unchanged'] for v in hashes.values()) else 'FAIL','started_at':started,'finished_at':time.time(),'elapsed_seconds':time.time()-started,'failure':failure,'browser_exit_code':browser_exit if browser_exit is not None else (browser.poll() if browser else None),'fixture_exit_code':fixture.poll() if fixture else None,'cleanup_requests':cleanup_requests,'fixture_cleanup':cleanup,'surviving_registered_pids':surviving,'test_inputs_recheck':hashes,'evidence':str(roundroot)}
 save(launcher/'result.json',result)
 print(json.dumps({'overall':result['overall'],'elapsed_seconds':result['elapsed_seconds'],'browser_exit_code':result['browser_exit_code'],'cleanup_all_stopped':bool(cleanup and cleanup.get('all_stopped')),'surviving_registered_pids':surviving,'evidence':str(roundroot),'launcher':str(launcher)}),flush=True)
sys.exit(0 if result['overall']=='PASS' else 1)
