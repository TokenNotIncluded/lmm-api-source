#!/usr/bin/env python3
"""Prepare one-field personal USER-ID repairs; never apply to production or post money."""
import argparse
import copy
import os
from pathlib import Path
import migrate_legacy_available_wallet as W


def prepare(source,live,wallet,physical):
    owners={x['source_user_id']:x for x in wallet['positions']}
    rows={r['id']:r for r in live['orders']}
    W.require(set(rows)=={o['id'] for o in source['orders']},'store_order_cohort_changed')
    result=[]
    for order in source['orders']:
        row=rows[order['id']];old=row['body'];buyer=owners[order['buyer_id']];seller=owners[order['seller_id']]
        W.require(old['imported']['source']=='go-v0.2.118' and old['imported']['snapshot']==W.digest(order),
                  'store_original_source_snapshot_changed')
        W.require(old['total']=={'amount_credits':order['price_quota'],'unit':'credits'}
                  and old['unit_price']=={'amount_credits':order['unit_price_quota'],'unit':'credits'}
                  and old['quantity']==order['quantity'],'store_original_principal_changed')
        W.require(old['seller']=={'kind':'personal','id':seller['target_user_id']}
                  and old['created_by']==buyer['target_user_id'],'store_other_personal_refs_need_separate_repair')
        W.require(old['buyer']['kind']=='personal' and old['buyer']['id'] in (buyer['account_id'],buyer['target_user_id']),
                  'store_buyer_not_proven_original_user_or_ledger_account')
        new=copy.deepcopy(old);new['buyer']['id']=buyer['target_user_id']
        result.append({'shop_id':row['shop_id'],'id':row['id'],'before':old,'after':new,
                       'source_buyer_user_id':order['buyer_id'],'native_buyer_user_id':buyer['target_user_id'],
                       'original_ledger_account_id':buyer['account_id']})
    return {'scope':'legacy-store-personal-user-reference-repair-v1','physical':physical,
            'source_archive_sha256':wallet['source_archive_sha256'],'rows':result,
            'changed_rows':sum(r['before']!=r['after'] for r in result),'money_written':False}


def build_sql(m):
    L=W.literal;p=m['physical'];checks='';updates='';readback=''
    for r in m['rows']:
        W.require({**r['before'],'buyer':r['after']['buyer']}==r['after'], 'repair_must_only_change_buyer')
        checks+=f"IF NOT EXISTS(SELECT 1 FROM lmm_store.orders WHERE shop_id={L(r['shop_id'])} AND id={L(r['id'])} AND body IN ({L(r['before'])}::jsonb,{L(r['after'])}::jsonb)) THEN RAISE EXCEPTION 'store expected body changed'; END IF;\n"
        if r['before']!=r['after']:
            updates+=f"UPDATE lmm_store.orders SET body=jsonb_set(body,'{{buyer,id}}',to_jsonb({r['native_buyer_user_id']}::bigint),false) WHERE shop_id={L(r['shop_id'])} AND id={L(r['id'])} AND body={L(r['before'])}::jsonb;\n"
        readback+=f"IF NOT EXISTS(SELECT 1 FROM lmm_store.orders WHERE shop_id={L(r['shop_id'])} AND id={L(r['id'])} AND body={L(r['after'])}::jsonb) THEN RAISE EXCEPTION 'store exact repaired readback mismatch'; END IF;\n"
    sql = f"""\\set ON_ERROR_STOP on
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on; SET LOCAL standard_conforming_strings=on;
SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s';
LOCK TABLE lmm_store.orders IN SHARE ROW EXCLUSIVE MODE;
DO $store_personal_repair$
BEGIN
 IF current_database()<>{L(p['database'])} OR (SELECT oid FROM pg_database WHERE datname=current_database())<>{int(p['database_oid'])} OR (pg_control_system()).system_identifier::text<>{L(p['system_identifier'])} THEN RAISE EXCEPTION 'store physical database mismatch'; END IF;
 IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=current_user AND rolsuper) AND current_user<>(SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname='lmm_store') THEN RAISE EXCEPTION 'store metadata repair owner required'; END IF;
 {checks}
 {updates}
 {readback}
END $store_personal_repair$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED_EXACT_PERSONAL_USER_REFERENCE','checked_orders',{len(m['rows'])},'changed_rows',{m['changed_rows']},'exact_match',true,'money_written',false);
"""
    tag='$store_repair_'+W.digest(m)+'$'
    W.require(tag not in W.canonical(m),'store_repair_delimiter_collision')
    return sql.replace('$store_personal_repair$',tag)


def main():
    os.umask(0o077);p=argparse.ArgumentParser(description=__doc__)
    for key in ['source','live','wallet-material','physical','output']:p.add_argument('--'+key,type=Path,required=True)
    a=p.parse_args();a.output.mkdir(mode=0o700)
    m=prepare(W.load(a.source),W.load(a.live),W.load(a.wallet_material),W.load(a.physical))
    W.save(a.output,'material.private.json',m);W.save(a.output,'repair.private.sql',build_sql(m))
    print(W.canonical({'status':'PREPARED_NOT_APPLIED','checked_orders':len(m['rows']),'changed_rows':m['changed_rows'],'money_written':False}))


if __name__=='__main__':main()
