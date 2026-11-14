#!/usr/bin/env python3
"""Preserve nonterminal legacy payment rights; never settle, credit or dispatch callbacks."""
import argparse
import os
from pathlib import Path
import sys
import time
import uuid
import migrate_legacy_available_wallet as W

SCOPE='financial-nonspendable-pending-payment-rights-v1'


def prepare(risk,wallet,policy,migration_id):
    W.require(wallet['scope']==W.SCOPE and risk['source_archive_sha256']==wallet['source_archive_sha256']
              and risk['mapping_sha256']==wallet['source_plan_mapping_sha256'],'pending_source_binding_mismatch')
    owners={x['source_user_id']:x for x in wallet['positions']}
    records=[];deferred=[];seen=set()
    for row in risk['orders']:
        W.require(row['terminal'] is False and row['status'] in ('pending','failed'),'not_nonterminal_payment')
        key=str(row['source_topup_id']);W.require(key not in seen,'duplicate_source_payment');seen.add(key)
        owner=owners.get(row['source_user_id'])
        if owner is None:
            deferred.append({'reason':'unmapped_original_owner_no_reassignment','source':row});continue
        parent=policy['parent_binding']
        parent_sha=parent['plan_sha256'] if row['pending_credit_rebase_key']==parent['source_migration_id'] else None
        records.append({'migration_id':migration_id,'source_archive_sha256':risk['source_archive_sha256'],
            'mapping_sha256':risk['mapping_sha256'],'source_table':row['source_namespace'],
            'source_row_key':key,'source_row_sha256':row['source_row_sha256'],'source_parent_plan_sha256':parent_sha,
            'owner_user_id':owner['target_user_id'],'owner_account_id':owner['account_id'],
            'status':row['status'],'settlement_state':'unknown','payload_json':row,
            'amount_raw_json':{'legacy_money_numeric':row['legacy_money_numeric'],
                               'legacy_deprecated_amount':row['legacy_deprecated_amount'],
                               'credit_facts':row['credit_facts'],'fiat_facts':row['fiat_facts']},
            'currency_raw_json':row['settlement_currency'],'spendable':False,'spendable_units':0,
            'observed_at':wallet['observed_at'],
            'request':{'policy':'preserve-original-provider-state-and-rebase-no-settlement-no-credit'},
            'result':{'settlement':'unknown','payment_proven_absent':False}})
    return {'scope':SCOPE,'migration_id':migration_id,'source_archive_sha256':risk['source_archive_sha256'],
            'target':wallet['target'],'connection':wallet['connection'],
            'wallet_migration_id':wallet['migration_id'],'wallet_mapping_sha256':wallet['mapping_sha256'],
            'identities':[owners[i] for i in sorted({r['source_user_id'] for r in risk['orders'] if r['source_user_id'] in owners})],
            'records':records,'deferred':deferred,'all_financial_rights_complete':False}


def build_sql(m):
    W.require(m['scope']==SCOPE and all(r['spendable'] is False and r['spendable_units']==0
              and r['settlement_state']=='unknown' for r in m['records']),'pending_rights_cannot_credit_or_settle')
    L=W.literal;t=m['target']
    manifest={'scope':SCOPE,'partial':True,'full_financial_import_complete':False,'material':m}
    checks=''
    for x in m['identities']:
        checks+=f"IF NOT EXISTS(SELECT 1 FROM core_identity.users u JOIN core_billing.opening_imports o ON o.account_id=u.personal_account_id WHERE u.id={x['target_user_id']} AND u.personal_account_id={x['account_id']} AND o.migration_id={L(m['wallet_migration_id'])} AND o.source_entity_key={L('legacy.users/'+str(x['source_user_id']))}) THEN RAISE EXCEPTION 'pending original owner binding changed'; END IF;\n"
    money="SELECT coalesce(jsonb_agg(jsonb_build_array(ledger_account_id,balance_units) ORDER BY ledger_account_id),'[]'::jsonb) FROM core_billing.balance_state"
    return f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on; SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='60s';
DO $pending_rights$
DECLARE p jsonb; prior jsonb; before_money jsonb; after_money jsonb;
BEGIN
 IF current_database()<>{L(t['database'])} OR current_user<>{L(t['role'])} OR session_user<>{L(t['login'])}
 OR (SELECT oid FROM pg_database WHERE datname=current_database())<>{int(t['database_oid'])}
 OR (pg_control_system()).system_identifier::text<>{L(t['system_identifier'])}
 OR current_setting('lmm.migration_instance_nonce')<>{L(t['nonce'])}
 THEN RAISE EXCEPTION 'pending rights physical target mismatch'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id={L(m['wallet_migration_id'])} AND source_archive_sha256={L(m['source_archive_sha256'])} AND mapping_sha256={L(m['wallet_mapping_sha256'])} AND manifest->>'scope'={L(W.SCOPE)}) THEN RAISE EXCEPTION 'pending rights wallet anchor missing'; END IF;
 SELECT manifest INTO prior FROM core_migration.import_runs WHERE migration_id={L(m['migration_id'])};
 IF FOUND AND prior IS DISTINCT FROM {L(manifest)}::jsonb THEN RAISE EXCEPTION 'pending rights replay conflict'; END IF;
 {checks}
 SELECT ({money}) INTO before_money;
 FOR p IN SELECT value FROM jsonb_array_elements({L(m['records'])}::jsonb) LOOP
  PERFORM core_billing.import_legacy_pending_liability(p);
  IF NOT EXISTS(SELECT 1 FROM core_billing.legacy_pending_liabilities WHERE migration_id=p->>'migration_id' AND source_table=p->>'source_table' AND source_row_key=p->>'source_row_key' AND request=p AND spendable=false AND spendable_units=0 AND settlement_state='unknown') THEN RAISE EXCEPTION 'pending rights exact readback failed'; END IF;
 END LOOP;
 SELECT ({money}) INTO after_money;
 IF before_money IS DISTINCT FROM after_money THEN RAISE EXCEPTION 'pending rights changed money'; END IF;
 IF (SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE migration_id={L(m['migration_id'])})<>{len(m['records'])} THEN RAISE EXCEPTION 'pending rights coverage mismatch'; END IF;
 INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES({L(m['migration_id'])},{L(m['source_archive_sha256'])},{L(W.digest(m))},{L(manifest)}::jsonb) ON CONFLICT DO NOTHING;
END $pending_rights$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED_NONSPENDABLE_RIGHTS_MONEY_UNCHANGED','records',{len(m['records'])},'unmapped_owner_deferred',{len(m['deferred'])},'settlement_completed',false);
"""


def main():
    os.umask(0o077);p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--apply',action='store_true')
    for key in ['risk','wallet-material','policy-evidence','material']:p.add_argument('--'+key,type=Path)
    p.add_argument('--migration-id');p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();a.output.mkdir(mode=0o700);out={'target_write_attempted':False,'automatic_retry':False,'actual_exit':1}
    try:
        if not a.apply:
            m=prepare(W.load(a.risk),W.load(a.wallet_material),W.load(a.policy_evidence),a.migration_id)
            W.save(a.output,'material.private.json',m);W.save(a.output,'pending-rights.private.sql',build_sql(m))
            out.update(status='PREPARED_NOT_APPLIED',actual_exit=0,records=len(m['records']),deferred=len(m['deferred']))
        else:
            m=W.load(a.material);app='lmm-pending-rights-'+uuid.uuid4().hex
            W.save(a.output,'attempt.private.json',{'application_name':app,'migration_id':m['migration_id'],'material_sha256':W.digest(m),'started_epoch':time.time()})
            out['target_write_attempted']=True;r=W.psql(m['connection'],build_sql(m),a.output,'apply',appname=app)
            if r.returncode==0:out.update(actual_exit=0,status='COMMITTED_NONSPENDABLE_RIGHTS_MONEY_UNCHANGED')
            else:out.update(status='OUTCOME_UNCONFIRMED',psql_exit=r.returncode)
    except Exception as e:out.update(status='OUTCOME_UNCONFIRMED' if out['target_write_attempted'] else 'REFUSED_BEFORE_WRITE',error=str(e))
    finally:W.save(a.output,'result.private.json',out)
    print(W.canonical(out));return out['actual_exit']


if __name__=='__main__':sys.exit(main())
