#!/usr/bin/env python3
"""Run the original bounded browser suite in a private network/mount namespace.

No builds, public hosts, production DBs, token-lifetime changes or business mocks.
The cf Extore helper supplies its unchanged resolver lifecycle and TLS forwarder.
The launcher/PG/Chromium run under the caller's original non-root UID/GID.
"""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import time

os.umask(0o077)
sys.dont_write_bytecode=True
BUNDLE=Path(__file__).resolve().parent
ENV_KEYS=('LMM_ACCEPTANCE_ROOT','LMM_ACCEPTANCE_NODE','LMM_ACCEPTANCE_PLAYWRIGHT',
          'LMM_ACCEPTANCE_CHROMIUM','LMM_ACCEPTANCE_PG_BIN')

def sha(path):
 return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def record(path,value,uid=None,gid=None):
 path.write_text(json.dumps(value,sort_keys=True,indent=2)+'\n')
 path.chmod(0o600)
 if uid is not None:os.chown(path,uid,gid)

def private_directory(path,uid):
 assert path.is_absolute() and str(path)==str(path.resolve(strict=True)), 'canonical directory required'
 mode=path.stat()
 assert stat.S_ISDIR(mode.st_mode) and mode.st_uid==uid and stat.S_IMODE(mode.st_mode)==0o700, str(path)
 return path

def check_bundle():
 freeze=json.loads((BUNDLE/'preparation.json').read_text())
 assert freeze['status']=='FROZEN_NOT_EXECUTED'
 assert freeze['source_commit']=='fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5'
 assert freeze['extore_source_commit']=='cf3469dbe827afcfb2e3f054a63f7b9b7c40bc2b'
 for name,value in freeze['files'].items():
  assert '/' not in name and (BUNDLE/name).is_file() and sha(BUNDLE/name)==value['sha256'],name
 return freeze

def namespace_processes(namespace):
 found=[]
 for p in Path('/proc').iterdir():
  if not p.name.isdecimal() or int(p.name)==os.getpid():continue
  try:
   if os.readlink(p/'ns/net')==namespace:
    ticks=(p/'stat').read_text().rsplit(') ',1)[1].split()[19]
    found.append({'pid':int(p.name),'start_ticks':ticks})
  except (OSError,ProcessLookupError,IndexError):pass
 return found

def signal_exact(item,sig,namespace):
 p=Path('/proc')/str(item['pid'])
 try:
  if os.readlink(p/'ns/net')==namespace and (p/'stat').read_text().rsplit(') ',1)[1].split()[19]==item['start_ticks']:
   os.kill(item['pid'],sig)
 except (OSError,ProcessLookupError,IndexError):pass

def inside(plan_path):
 assert os.geteuid()==0, 'namespace setup requires sudo, while fixture must remain non-root'
 plan=json.loads(plan_path.read_text())
 uid,gid=plan['uid'],plan['gid']
 assert uid>0 and gid>=0
 evidence=private_directory(Path(plan['evidence']),uid)
 own_net=os.readlink('/proc/self/ns/net')
 own_mount=os.readlink('/proc/self/ns/mnt')
 assert own_net!=plan['parent_netns'] and own_mount!=plan['parent_mountns']
 check_bundle()
 probe=BUNDLE/'native_plugin_probe_cf.py'
 spec=importlib.util.spec_from_file_location('isolated_cf_resolver',probe)
 module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
 subprocess.run(['/usr/bin/env','ip','link','set','lo','up'],check=True,capture_output=True,timeout=10)
 # This sysctl belongs to the new network namespace; it lets the non-root TLS
 # forwarder bind 443 while retaining PostgreSQL's non-root execution contract.
 subprocess.run(['/usr/bin/env','sysctl','-q','-w','net.ipv4.ip_unprivileged_port_start=0'],check=True,capture_output=True,timeout=10)
 assert Path('/proc/sys/net/ipv4/ip_unprivileged_port_start').read_text().strip()=='0'
 namespace_report={'status':'NOT_RUN','parent_netns':plan['parent_netns'],'parent_mountns':plan['parent_mountns'],
  'network_namespace':own_net,'mount_namespace':own_mount,'runtime_uid':uid,'runtime_gid':gid,
  'source_commit':'fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5',
  'extore_source_commit':'cf3469dbe827afcfb2e3f054a63f7b9b7c40bc2b',
  'helper_sha256':sha(probe),'maximum_inner_seconds':1800,'maximum_outer_child_seconds':1805,
  'production_data_used':False,'network_scope':'new loopback-only network namespace',
  'initial_namespace_processes':namespace_processes(own_net)}
 assert namespace_report['initial_namespace_processes']==[], 'unexpected process in newly-created network namespace'
 child=None;failure=None;exit_code=None;forced=[]
 started=time.monotonic()
 try:
  args=argparse.Namespace(private_hosts=True,parent_mountns=plan['parent_mountns'])
  with module.private_host_mapping(args,evidence,namespace_report):
   env=plan['environment']
   env.update(LMM_ACCEPTANCE_ISOLATED='1',LMM_ACCEPTANCE_NETNS=own_net,LMM_ACCEPTANCE_MOUNTNS=own_mount)
   assert all(name not in env for name in ['GH_TOKEN','GITHUB_TOKEN','DATABASE_URL','SQL_DSN'])
   argv=[plan['python'],'-B',str(BUNDLE/'run-round.py'),'--round',plan['round'],'--candidate',plan['candidate']]
   with (evidence/'launcher.private.log').open('xb') as output:
    os.chown(evidence/'launcher.private.log',uid,gid)
    child=subprocess.Popen(argv,cwd=BUNDLE,env=env,user=uid,group=gid,extra_groups=[],
      stdout=output,stderr=subprocess.STDOUT,start_new_session=True)
    try:exit_code=child.wait(timeout=1805)
    except subprocess.TimeoutExpired:failure='outer_child_deadline'
    finally:
     # The original launcher's normal cleanup is authoritative. This only
     # rescues a crashed launcher and makes forced cleanup a failed attempt.
     grace=time.monotonic()+5
     while namespace_processes(own_net) and time.monotonic()<grace:time.sleep(.2)
     leftover=namespace_processes(own_net)
     if leftover:
      forced=leftover;failure=failure or 'owned_processes_survived_original_cleanup'
      for item in leftover:signal_exact(item,signal.SIGTERM,own_net)
      end=time.monotonic()+15
      while namespace_processes(own_net) and time.monotonic()<end:time.sleep(.2)
      for item in namespace_processes(own_net):signal_exact(item,signal.SIGKILL,own_net)
      if child.poll() is None:
       try:child.wait(timeout=10)
       except subprocess.TimeoutExpired:pass
  namespace_report['surviving_namespace_processes']=namespace_processes(own_net)
  namespace_report['inner_exit_code']=exit_code
  namespace_report['forced_cleanup_processes']=forced
  namespace_report['status']='PASS' if exit_code==0 and not failure and not namespace_report['surviving_namespace_processes'] else 'FAIL'
 except BaseException as error:
  failure=failure or type(error).__name__+': '+str(error)
  namespace_report['status']='FAIL'
 finally:
  remaining=namespace_processes(own_net)
  if remaining:
   failure=failure or 'namespace_process_cleanup_required'
   namespace_report['status']='FAIL'
   for item in remaining:signal_exact(item,signal.SIGKILL,own_net)
   if child and child.poll() is None:
    try:child.wait(timeout=10)
    except subprocess.TimeoutExpired:pass
  namespace_report.update(failure=failure,elapsed_seconds=time.monotonic()-started,
   surviving_namespace_processes=namespace_processes(own_net))
  if namespace_report['surviving_namespace_processes']:namespace_report['status']='FAIL'
  for name in ('private-hosts','private-nsswitch'):
   if (evidence/name).exists():os.chown(evidence/name,uid,gid)
  record(evidence/'namespace-result.json',namespace_report,uid,gid)
 return 0 if namespace_report['status']=='PASS' else 1

def outside(args):
 assert os.geteuid()!=0, 'invoke as ordinary owner; only namespace setup uses sudo'
 assert re.fullmatch(r'browser-[A-Za-z0-9._-]+',args.round) and '..' not in args.round
 check_bundle()
 for name in ENV_KEYS:assert os.environ.get(name),name+' is required'
 root=private_directory(Path(os.environ['LMM_ACCEPTANCE_ROOT']),os.geteuid())
 candidate=args.candidate.resolve(strict=True)
 data=json.loads(candidate.read_text())
 assert data['source_commit']=='fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5'
 assert data['extore']['source_commit']=='cf3469dbe827afcfb2e3f054a63f7b9b7c40bc2b'
 assert data['official_distribution']['run_id']==44936380440
 for name in ENV_KEYS[1:]:
  p=Path(os.environ[name]);assert p.is_absolute() and p.exists(),name
 for suffix in ('','-launcher','-namespace'):
  assert not (root/(args.round+suffix)).exists(), 'never overwrite a prior attempt'
 evidence=root/(args.round+'-namespace');evidence.mkdir(mode=0o700)
 environment={key:os.environ[key] for key in ('PATH','HOME','LANG','TZ','GITHUB_SHA','GITHUB_RUN_ID','GITHUB_RUN_ATTEMPT') if key in os.environ}
 environment.update({key:os.environ[key] for key in ENV_KEYS})
 plan={'uid':os.geteuid(),'gid':os.getegid(),'round':args.round,'candidate':str(candidate),
  'candidate_sha256':sha(candidate),'python':sys.executable,'environment':environment,'evidence':str(evidence),
  'parent_netns':os.readlink('/proc/self/ns/net'),'parent_mountns':os.readlink('/proc/self/ns/mnt'),
  'parent_resolver':{name:sha(name) for name in ('/etc/hosts','/etc/nsswitch.conf')},
  'preparation_sha256':sha(BUNDLE/'preparation.json')}
 plan_path=evidence/'plan.json';record(plan_path,plan)
 result=subprocess.run(['sudo','-n','unshare','--net','--mount','--propagation','private','--',
   sys.executable,'-B',str(Path(__file__).resolve()),'--inside-plan',str(plan_path)])
 unchanged={name:sha(name)==digest for name,digest in plan['parent_resolver'].items()}
 summary={'exit_code':result.returncode,'candidate_unchanged':sha(candidate)==plan['candidate_sha256'],
  'parent_resolver_unchanged':unchanged,'namespace_receipt':str(evidence/'namespace-result.json')}
 summary['status']='PASS' if result.returncode==0 and summary['candidate_unchanged'] and all(unchanged.values()) else 'FAIL'
 record(evidence/'outer-result.json',summary)
 print(json.dumps(summary),flush=True)
 return 0 if summary['status']=='PASS' else 1

def main():
 parser=argparse.ArgumentParser()
 parser.add_argument('--round')
 parser.add_argument('--candidate',type=Path)
 parser.add_argument('--inside-plan',type=Path,help=argparse.SUPPRESS)
 args=parser.parse_args()
 if args.inside_plan:return inside(args.inside_plan.resolve(strict=True))
 assert args.round and args.candidate, '--round and --candidate are required'
 return outside(args)

if __name__=='__main__':raise SystemExit(main())
