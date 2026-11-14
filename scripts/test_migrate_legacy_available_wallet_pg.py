#!/usr/bin/env python3
"""One real PG/Core regression: signed quota conservation and replay no-op."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from urllib.parse import urlencode
import migrate_legacy_available_wallet as M


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--core',type=Path,required=True);p.add_argument('--socket',type=Path,required=True)
    p.add_argument('--port',type=int,required=True);p.add_argument('--admin',required=True)
    p.add_argument('--output',type=Path,required=True);a=p.parse_args();os.umask(0o077);a.output.mkdir(mode=0o700)
    nonce=str(os.getpid())+'_'+str(int(time.time()));db='qa_wallet_'+nonce;owner='qa_wo_'+nonce;login='qa_wi_'+nonce
    admin={'socket_directory':str(a.socket),'port':a.port,'database':'postgres','role':a.admin}
    def query(conn,label,sql,readonly=False):
        r=M.psql(conn,sql,a.output,label,readonly=readonly)
        if r.returncode:raise RuntimeError(label+': '+r.stderr[-1000:])
        return r.stdout
    query(admin,'create-roles',f'CREATE ROLE {owner} NOLOGIN; CREATE ROLE {login} LOGIN NOINHERIT; GRANT {owner} TO {login} WITH INHERIT FALSE, SET TRUE; GRANT SET ON PARAMETER log_parameter_max_length,log_parameter_max_length_on_error,log_error_verbosity TO {owner};')
    query(admin,'create-db',f'CREATE DATABASE {db} OWNER {owner};')
    admin_db={**admin,'database':db};query(admin_db,'setup-public',f'GRANT USAGE,CREATE ON SCHEMA public TO {owner}; ALTER DATABASE {db} SET lmm.migration_instance_nonce=\'{"d"*64}\';')
    conn={'socket_directory':str(a.socket),'port':a.port,'database':db,'role':owner,'login_role':login,'set_role':owner}
    url='postgresql://'+login+'@localhost/'+db+'?'+urlencode({'host':str(a.socket),'port':a.port,'options':'-c role='+owner})
    (a.output/'owner.url.private').write_text(url);(a.output/'owner.toml').write_text('version=1\n[database]\nurl_file='+json.dumps(str(a.output/'owner.url.private'))+'\nmax_connections=2\n')
    env={k:v for k,v in os.environ.items() if not k.startswith(('PG','LMM_CORE'))}
    q=subprocess.run([str(a.core),'--config',str(a.output/'owner.toml'),'init-db'],capture_output=True,text=True,env=env)
    (a.output/'init.stdout.private').write_text(q.stdout);(a.output/'init.stderr.private').write_text(q.stderr)
    if q.returncode:raise RuntimeError('actual Core init exit '+str(q.returncode))
    query(conn,'identities',"INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES(1,'personal'),(2,'personal'),(3,'personal'); INSERT INTO core_identity.users(id,personal_account_id) VALUES(1,1),(2,2),(3,3); INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('identity-test','"+'a'*64+"','"+'b'*64+"','{\"scope\":\"identity-only\"}');")
    obs=json.loads(query(conn,'observation',M.observation_sql(),True));positions=[{'source_user_id':i,'account_id':i,'signed_available_units':v,'source_row_sha256':str(i)*64,'signed_affiliate_units':99 if i==1 else 0} for i,v in [(1,1000),(2,-125),(3,0)]]
    plan={'source_archive_sha256':'a'*64,'unit':'credit_500k_usd','credits_per_usd':500000,'wallet_positions':positions,'mapping_sha256':'c'*64,'families':{'escrow':{'source_row_count':5}},'blocker_counts':{'deferred_escrow':5}}
    target={'connection':conn,'source_archive_sha256':'a'*64,'instance_identity':{'database_oid':obs['database_oid'],'server_instance_nonce':obs['nonce']}}
    material=M.prepare(plan,{'source_archive_sha256':'a'*64,'source_export_sha256':'e'*64},{'archive':{'sha256':'a'*64},'started_utc':'2026-11-14T00:00:00+00:00'},[{'source_user_id':i,'target_user_id':i,'target_personal_account_id':i} for i in [1,2,3]],target,{'observed_options':[{'key':'CreditsPerUSD','value':'500000'},{'key':'QuotaPerUnit','value':'500000'}]},obs,{1:1000,2:-125,3:0},'wallet-test')
    with ThreadPoolExecutor(max_workers=1) as pool:
        reader=pool.submit(M.psql,conn,'BEGIN READ ONLY; SELECT pg_sleep(3); COMMIT;',a.output,'concurrent-live-reader',readonly=True,appname='qa-wallet-live-reader')
        for n in range(40):
            if int(query(conn,'reader-ready-'+str(n),"SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND application_name='qa-wallet-live-reader';",True))==1:break
            time.sleep(.05)
        else:raise AssertionError('live reader did not connect')
        query(conn,'first-apply',M.build_sql(material,online=True))
        assert reader.result().returncode==0
    counts="SELECT jsonb_build_object('journals',(SELECT count(*) FROM core_billing.ledger_journals),'entries',(SELECT count(*) FROM core_billing.ledger_entries),'delta',(SELECT sum(delta_units) FROM core_billing.ledger_entries),'opening',(SELECT count(*) FROM core_billing.opening_imports),'wallet',(SELECT sum(b.balance_units) FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='wallet'),'debt',(SELECT sum(b.balance_units) FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='debt'),'affiliate',(SELECT sum(b.balance_units) FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='affiliate'));"
    before=json.loads(query(conn,'before-replay',counts,True));assert before['delta']==0 and before['wallet']==1000 and before['debt']==-125 and before['affiliate']==0 and before['opening']==3,before
    query(conn,'replay',M.build_sql(material));after=json.loads(query(conn,'after-replay',counts,True));assert before==after,(before,after)
    query(conn,'readonly-reconcile',M.build_sql(material,reconcile=True),True)
    result={'status':'PASS_REAL_PG_CONSERVATION_AND_IDEMPOTENCY','actual_core_init_exit':0,'before_replay':before,'after_replay':after,'deferred_escrow_not_credited':True,'deferred_affiliate_not_credited':True,'database_retained':db,'owner':owner,'installer':login,'new_pg_instance_started':False}
    M.save(a.output,'result.private.json',result);print(M.canonical(result));return 0

if __name__=='__main__':sys.exit(main())
