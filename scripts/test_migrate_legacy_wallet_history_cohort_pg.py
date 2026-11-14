#!/usr/bin/env python3
"""Real PG: mapped top-up history and explicit orphan retention, no money changes."""
import argparse
import json
import os
from pathlib import Path
import time
import migrate_legacy_available_wallet as W
import migrate_legacy_wallet_history_cohort as M
import prepare_wallet_history_snapshot as H


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--wallet-test-result',type=Path,required=True)
    p.add_argument('--socket',type=Path,required=True);p.add_argument('--port',type=int,required=True)
    p.add_argument('--admin',required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();os.umask(0o077);a.output.mkdir(mode=0o700)
    base=W.load(a.wallet_test_result);template=base['database_retained']
    W.require(template.startswith('qa_wallet_') and W.IDENT.fullmatch(template),'owned_wallet_qa_fixture_required')
    db='qa_cohort_'+str(os.getpid())+'_'+str(int(time.time()))
    admin={'socket_directory':str(a.socket),'port':a.port,'database':'postgres','role':a.admin}
    def query(c,n,s,readonly=False):
        r=W.psql(c,s,a.output,n,readonly=readonly)
        if r.returncode:raise RuntimeError(n+': '+r.stderr[-800:])
        return r.stdout
    query(admin,'clone',f'CREATE DATABASE {db} TEMPLATE {template} OWNER {base["owner"]};')
    query(admin,'nonce',f"ALTER DATABASE {db} SET lmm.migration_instance_nonce='{'9'*64}';")
    conn={**admin,'database':db,'role':base['owner'],'login_role':base['installer'],'set_role':base['owner']}
    obs=json.loads(query(conn,'actual-target',W.observation_sql(),True))
    opening=json.loads(query(conn,'opening',"SELECT to_jsonb(r) FROM core_billing.opening_import_runs r WHERE migration_id='wallet-test';",True))
    wallet={'scope':W.SCOPE,'source_archive_sha256':'a'*64,'migration_id':'wallet-test','mapping_sha256':opening['mapping_sha256'],'target':obs,'connection':conn,'positions':[{'source_user_id':i,'target_user_id':i,'account_id':i} for i in [1,2,3]]}
    row={'id':1,'user_id':1,'amount':10,'create_time':100,'complete_time':0,'credited_quota':0,'platform_amount_micros':0,'money_text':'10.125','trade_no':'fixture-1','payment_method':'epay','status':'pending','settlement_currency':None}
    export={'source_namespace':H.NAMESPACE,'source_row_count':2,'observed_at':200,'rows':[row,{**row,'id':2,'user_id':99,'trade_no':'orphan-2'}]}
    m=M.prepare(export,wallet);assert m['manifest']['source_row_count']==1 and m['manifest']['source_cohort']['full_namespace_restore_complete'] is False
    money="SELECT jsonb_build_object('money',(SELECT jsonb_agg(to_jsonb(x) ORDER BY ledger_account_id) FROM core_billing.balance_state x),'journals',(SELECT count(*) FROM core_billing.ledger_journals));"
    before=query(conn,'before',money,True);query(conn,'apply',M.build_sql(m));first=json.loads(query(conn,'verify',H.verification_sql(m['manifest']),True))
    assert first['imported_rows']==1 and first['exact_manifest_match'] is True
    query(conn,'replay',M.build_sql(m));after=query(conn,'after',money,True);assert before==after
    result={'status':'PASS_REAL_PG_COHORT_OWNER_GUARD_ORPHANS_NO_MONEY_IDEMPOTENCY','mapped_rows':1,'unmapped_rows':1,'money_unchanged':True,'database_retained':db,'new_pg_instance_started':False,'original_fixture_unchanged':template}
    W.save(a.output,'result.private.json',result);print(W.canonical(result));return 0


if __name__=='__main__':raise SystemExit(main())
