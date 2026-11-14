#!/usr/bin/env python3
"""Owned local clone: exact real-data buyer repair, unchanged money fields, replay no-op."""
import argparse
import json
import os
from pathlib import Path
import time
import migrate_legacy_available_wallet as W
import repair_legacy_store_personal_refs as M


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for key in ['source','live','wallet-material','socket','output']:p.add_argument('--'+key,type=Path,required=True)
    p.add_argument('--port',type=int,required=True);p.add_argument('--admin',required=True)
    a=p.parse_args();os.umask(0o077);a.output.mkdir(mode=0o700)
    db='qa_store_refs_'+str(os.getpid())+'_'+str(int(time.time()))
    c={'socket_directory':str(a.socket),'port':a.port,'database':'postgres','role':a.admin}
    def query(n,s,readonly=False):
        r=W.psql(c,s,a.output,n,readonly=readonly)
        if r.returncode:raise RuntimeError(n+': '+r.stderr[-1000:])
        return r.stdout
    query('clone',f'CREATE DATABASE {db} TEMPLATE rights_store_26752cda OWNER {a.admin};')
    c['database']=db;live=W.load(a.live)
    # The retained older dump is not the current live preimage: seed the QA
    # clone with the actual independently captured live bodies, never production.
    for i,r in enumerate(live['orders']):query('seed-'+str(i),f"UPDATE lmm_store.orders SET body={W.literal(r['body'])}::jsonb WHERE shop_id={W.literal(r['shop_id'])} AND id={W.literal(r['id'])};")
    physical=json.loads(query('physical',"SELECT jsonb_build_object('database',current_database(),'database_oid',(SELECT oid FROM pg_database WHERE datname=current_database()),'system_identifier',(pg_control_system()).system_identifier::text);",True))
    m=M.prepare(W.load(a.source),live,W.load(a.wallet_material),physical)
    before=json.loads(query('before',"SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM lmm_store.orders o;",True))
    query('repair',M.build_sql(m));after=json.loads(query('after',"SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM lmm_store.orders o;",True))
    expected={r['id']:r['after'] for r in m['rows']};assert all(r['body']==expected[r['id']] for r in after)
    assert sum(a['body']!=b['body'] for a,b in zip(before,after))==m['changed_rows']==1
    assert all({**a['body'],'buyer':b['body']['buyer']}==b['body'] for a,b in zip(before,after))
    query('replay',M.build_sql(m));replayed=json.loads(query('after-replay',"SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM lmm_store.orders o;",True));assert after==replayed
    W.save(a.output,'result.private.json',{'status':'PASS_REAL_PG_ONE_BUYER_FIELD_EXACT_CONSERVATION_IDEMPOTENCY','checked_orders':4,'changed_rows':1,'database_retained':db,'production_writes':0})
    print('PASS_REAL_PG_ONE_BUYER_FIELD_EXACT_CONSERVATION_IDEMPOTENCY')


if __name__=='__main__':main()
