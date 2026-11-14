#!/usr/bin/env python3
"""Real Native69 PG: unknown payment retention never credits money; replay is a no-op."""
import argparse
import json
import os
from pathlib import Path
import time
import migrate_legacy_available_wallet as W
import migrate_legacy_pending_payment_rights as M


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--wallet-test-result',type=Path,required=True)
    p.add_argument('--socket',type=Path,required=True);p.add_argument('--port',type=int,required=True)
    p.add_argument('--admin',required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();os.umask(0o077);a.output.mkdir(mode=0o700)
    base=W.load(a.wallet_test_result);template=base['database_retained']
    W.require(template.startswith('qa_wallet_') and W.IDENT.fullmatch(template),'owned_wallet_qa_fixture_required')
    db='qa_pending_'+str(os.getpid())+'_'+str(int(time.time()))
    admin={'socket_directory':str(a.socket),'port':a.port,'database':'postgres','role':a.admin}
    def query(c,n,s,readonly=False):
        r=W.psql(c,s,a.output,n,readonly=readonly)
        if r.returncode:raise RuntimeError(n+': '+r.stderr[-800:])
        return r.stdout
    query(admin,'clone',f'CREATE DATABASE {db} TEMPLATE {template} OWNER {base["owner"]};')
    query(admin,'nonce',f"ALTER DATABASE {db} SET lmm.migration_instance_nonce='{'f'*64}';")
    conn={**admin,'database':db,'role':base['owner'],'login_role':base['installer'],'set_role':base['owner']}
    obs=json.loads(query(conn,'actual-target',W.observation_sql(),True))
    opening=json.loads(query(conn,'opening',"SELECT to_jsonb(r) FROM core_billing.opening_import_runs r WHERE migration_id='wallet-test';",True))
    wallet={'scope':W.SCOPE,'source_archive_sha256':'a'*64,'source_plan_mapping_sha256':'d'*64,'migration_id':'wallet-test','mapping_sha256':opening['mapping_sha256'],'observed_at':opening['observed_at'],'target':obs,'connection':conn,'positions':[{'source_user_id':i,'target_user_id':i,'account_id':i} for i in [1,2,3]]}
    row={'terminal':False,'status':'pending','source_topup_id':1,'source_user_id':1,'source_row_sha256':'b'*64,'source_namespace':'legacy.top_ups','pending_credit_rebase_key':'test-rebase','legacy_money_numeric':None,'legacy_deprecated_amount':20,'credit_facts':{'pending_credit_rebase_effective_quota':17},'fiat_facts':{},'settlement_currency':None}
    risk={'source_archive_sha256':'a'*64,'mapping_sha256':'d'*64,'orders':[row,{**row,'source_topup_id':2,'source_user_id':99}]}
    m=M.prepare(risk,wallet,{'parent_binding':{'source_migration_id':'test-rebase','plan_sha256':'c'*64}},'qa-pending-rights')
    assert len(m['records'])==len(m['deferred'])==1 and m['records'][0]['amount_raw_json']['credit_facts']['pending_credit_rebase_effective_quota']==17
    raw='{"id":42,"user_id":1,"status":"pending","money":1.2500000000001}'
    subscriptions=M.prepare_subscription_orders({'source_archive_sha256':'a'*64,'observed_at':wallet['observed_at'],
        'subscription_orders':{'count':1,'raw_rows':[raw]},'subscription_order_credit_rebases':{'raw_rows':[]}},wallet,'qa-pending-rights')
    assert subscriptions['records'][0]['payload_json']['source_json_text']==raw
    m['records']+=subscriptions['records']
    counts="SELECT jsonb_build_object('money',(SELECT jsonb_agg(to_jsonb(x) ORDER BY ledger_account_id) FROM core_billing.balance_state x),'journals',(SELECT count(*) FROM core_billing.ledger_journals),'rights',(SELECT count(*) FROM core_billing.legacy_pending_liabilities));"
    before=json.loads(query(conn,'before',counts,True));query(conn,'apply',M.build_sql(m));first=json.loads(query(conn,'after',counts,True))
    assert first['money']==before['money'] and first['journals']==before['journals'] and first['rights']==2
    query(conn,'replay',M.build_sql(m));after=json.loads(query(conn,'after-replay',counts,True));assert first==after
    result={'status':'PASS_REAL_PG_UNKNOWN_RIGHTS_NO_CREDIT_IDEMPOTENCY','rights':2,'subscription_decimal_text_exact':True,'unmapped_owner_deferred':1,'money_unchanged':True,'journals_unchanged':True,'database_retained':db,'new_pg_instance_started':False,'original_fixture_unchanged':template}
    W.save(a.output,'result.private.json',result);print(W.canonical(result));return 0


if __name__=='__main__':raise SystemExit(main())
