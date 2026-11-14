#!/usr/bin/env python3
"""Real Native69 fixture clone: escrow conservation, no payout and replay no-op."""
import argparse
import json
import os
from pathlib import Path
import time
import migrate_legacy_available_wallet as W
import migrate_legacy_escrow_supplement as M


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--wallet-test-result',type=Path,required=True)
    p.add_argument('--socket',type=Path,required=True);p.add_argument('--port',type=int,required=True)
    p.add_argument('--admin',required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();os.umask(0o077);a.output.mkdir(mode=0o700)
    base=W.load(a.wallet_test_result);template=base['database_retained']
    W.require(template.startswith('qa_wallet_') and W.IDENT.fullmatch(template),'owned_wallet_qa_fixture_required')
    db='qa_escrow_'+str(os.getpid())+'_'+str(int(time.time()))
    admin={'socket_directory':str(a.socket),'port':a.port,'database':'postgres','role':a.admin}
    def query(c,n,s,readonly=False):
        r=W.psql(c,s,a.output,n,readonly=readonly)
        if r.returncode:raise RuntimeError(n+': '+r.stderr[-800:])
        return r.stdout
    query(admin,'clone',f'CREATE DATABASE {db} TEMPLATE {template} OWNER {base["owner"]};')
    query(admin,'nonce',f"ALTER DATABASE {db} SET lmm.migration_instance_nonce='{'e'*64}';")
    conn={**admin,'database':db,'role':base['owner'],'login_role':base['installer'],'set_role':base['owner']}
    obs=json.loads(query(conn,'actual-target',W.observation_sql(),True))
    opening=json.loads(query(conn,'opening',"SELECT to_jsonb(r) FROM core_billing.opening_import_runs r WHERE migration_id='wallet-test';",True))
    review={'scope':M.SCOPE,'fixture_only':True}
    claim={'source_challenge_id':1,'participant_user_id':2,'recipient_account_id':2,'source_row_sha256':'b'*64,'rights_status':'active_future_reward','source_reward_quota':50,'payout_units':50,'status':'accepted','rejected_at':0,'paid_at':0}
    project={'source_project_id':1,'owner_user_id':1,'payer_account_id':1,'source_row_sha256':'f'*64,'source_parent_plan_sha256':'d'*64,'escrow_units':80,'status':'paused','claims':[claim],'disputes':[]}
    request={**opening,'unit':'credit_500k_usd','projects':[project],'review_proof_sha256':W.digest(review)}
    m={'scope':M.SCOPE,'migration_id':'qa-escrow-supplement','source_archive_sha256':opening['source_archive_sha256'],'target':obs,'connection':conn,'review':review,'request':request,'identities':[{'source_user_id':i,'target_user_id':i,'account_id':i} for i in [1,2]]}
    counts="SELECT jsonb_build_object('wallet',(SELECT sum(b.balance_units) FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='wallet'),'debt',(SELECT sum(b.balance_units) FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='debt'),'reserved',(SELECT sum(b.balance_units) FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='reserved'),'journals',(SELECT count(*) FROM core_billing.ledger_journals),'delta',(SELECT sum(delta_units) FROM core_billing.ledger_entries),'claims',(SELECT count(*) FROM core_billing.legacy_escrow_claims),'openings',(SELECT count(*) FROM core_billing.opening_imports));"
    before=json.loads(query(conn,'before',counts,True))
    query(conn,'apply',M.build_sql(m));first=json.loads(query(conn,'after',counts,True))
    assert first['wallet']==before['wallet']==1000 and first['debt']==before['debt']==-125
    assert first['reserved']==before['reserved']+80 and first['delta']==0 and first['claims']==1 and first['openings']==3
    query(conn,'replay',M.build_sql(m));second=json.loads(query(conn,'after-replay',counts,True));assert first==second
    result={'status':'PASS_REAL_PG_ESCROW_CONSERVATION_NO_PAYOUT_IDEMPOTENCY','before':before,'after':first,'after_replay':second,'database_retained':db,'new_pg_instance_started':False,'original_fixture_unchanged':template}
    W.save(a.output,'result.private.json',result);print(W.canonical(result));return 0


if __name__=='__main__':raise SystemExit(main())
