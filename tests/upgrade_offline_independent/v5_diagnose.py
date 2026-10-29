#!/usr/bin/env python3
"""Diagnose saved synthetic runtime authorization on an isolated PGDATA copy."""
import argparse,json,os,shutil,subprocess,tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--source-data',type=Path,required=True);p.add_argument('--source-run',type=Path,required=True);p.add_argument('--core',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();os.umask(0o077);a.output.mkdir(mode=0o700);data=a.output/'data';shutil.copytree(a.source_data,data);sock=Path(tempfile.mkdtemp(prefix='lmm-v5-diag-',dir='/tmp'));hba=a.output/'hba.conf';hba.write_text('local all all trust\n');started=False
try:
 options=f'-c listen_addresses= -c unix_socket_directories={sock} -p 55604 -c hba_file={hba} -c shared_buffers=16MB -c max_connections=20'
 subprocess.run(['pg_ctl','-D',str(data),'-l',str(a.output/'postgres.log'),'-o',options,'-w','start'],check=True,capture_output=True,timeout=30);started=True
 sql="SELECT jsonb_agg(jsonb_build_object('table',c.relname,'select',has_table_privilege('lmm_core_runtime',c.oid,'SELECT')) ORDER BY c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_meta' AND c.relkind IN ('r','p');"
 result=subprocess.run(['psql','-X','-qAt','-v','ON_ERROR_STOP=1','-h',str(sock),'-p','55604','-U','postgres','-d','coretarget'],input=sql.encode(),capture_output=True,check=True)
 facts=json.loads(result.stdout);(a.output/'runtime-meta-select.json').write_text(json.dumps(facts,indent=2)+'\n')
 url=a.output/'runtime.url';url.write_text(f'postgresql://lmm_core_runtime@localhost/coretarget?host={sock}&port=55604&sslmode=disable');config=(a.source_run/'fixture-runtime.toml').read_text().replace(str(a.source_run/'fixture-runtime.url'),str(url));runtime=a.output/'runtime.toml';runtime.write_text(config)
 result=subprocess.run([str(a.core),'--config',str(runtime),'serve'],capture_output=True,timeout=30);(a.output/'core-serve.log').write_bytes(result.stdout+result.stderr)
 verdict={'runtime_start_exit':result.returncode,'runtime_start_error':result.stderr.decode(errors='replace'),'meta_select_denied':[x['table'] for x in facts if not x['select']],'scope':'isolated copy of synthetic failure; exact canonical grants; no changed privileges'}
 (a.output/'diagnosis.json').write_text(json.dumps(verdict,indent=2)+'\n');print(json.dumps(verdict),flush=True)
finally:
 if started:subprocess.run(['pg_ctl','-D',str(data),'-m','fast','-w','stop'],check=True,capture_output=True,timeout=30)
 if sock.exists():shutil.rmtree(sock)
