#!/usr/bin/env python3
"""Owner-only confirmed moderation for imported pending contracts.

plan: read-only consistent snapshot + exact economic preview, private output.
apply: same request and approved hash, SERIALIZABLE locked snapshot comparison,
then the Core SQL moderation boundary. No Go, callback, arbitrary ledger payload
or inferred refund is accepted. Ordinary disable/enable is a separate operation.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
from legacy_referral_import import digest, jlit, lit


def validate(r):
    if set(r) != {'request_id','actor','user_id','action','reason','evidence','penalize'}:
        raise ValueError('invalid moderation request fields')
    if type(r['actor']) is not int or r['actor']<=0 or type(r['user_id']) is not int or r['user_id']<=0:
        raise ValueError('invalid actor/target')
    if not isinstance(r['request_id'],str) or not 1<=len(r['request_id'])<=100:
        raise ValueError('invalid request id')
    if not isinstance(r['evidence'],str) or not 1<=len(r['evidence'].strip())<=1000:
        raise ValueError('explicit moderation evidence required')
    if type(r['penalize']) is not bool:
        raise ValueError('invalid penalty decision')
    if not ((r['action']=='ban_abuse' and r['reason'] in ('abuse','bulk_registration')) or
            (r['action']=='restore_referral' and r['reason']=='mistaken_ban' and not r['penalize'])):
        raise ValueError('invalid confirmed moderation action')


def snapshot_sql(r):
    validate(r)
    return f'''SELECT jsonb_build_object(
 'database',current_database(),'database_oid',(SELECT oid::text FROM pg_database WHERE datname=current_database()),
 'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
 'actor',(SELECT jsonb_build_object('id',id,'active',active,'role',platform_role) FROM core_identity.users WHERE id={r['actor']}),
 'target',(SELECT jsonb_build_object('id',id,'active',active,'role',platform_role,'auth_version',auth_version) FROM core_identity.users WHERE id={r['user_id']}),
 'contract',(SELECT to_jsonb(c) FROM core_referrals.contracts c WHERE invitee={r['user_id']}),
 'affiliate',(SELECT jsonb_build_object('account',a.id,'balance',b.balance_units,'revision',b.revision)
 FROM core_referrals.contracts c JOIN core_identity.users u ON u.id=c.inviter
 JOIN core_billing.ledger_accounts a ON a.owner_account_id=u.personal_account_id AND a.bucket='affiliate'
 JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE c.invitee={r['user_id']}))'''


def preview(r, state):
    validate(r)
    c=state['contract']
    if c is None: raise ValueError('no executable migrated pending contract')
    actor,target=state['actor'],state['target']
    if not actor['active'] or actor['role'] not in ('admin','superadmin') or not (
            target['role']=='user' or (actor['role']=='superadmin' and target['role']=='admin')):
        raise ValueError('no permission to moderate target')
    delta=0
    if r['action']=='ban_abuse' and c['status']=='earned':
        penalty=c['quota']//100*c['penalty_percent']+c['quota']%100*c['penalty_percent']//100 if r['penalize'] else 0
        if c['max_penalty_credit']>0: penalty=min(penalty,c['max_penalty_credit'])
        delta=-c['quota']-penalty
    elif r['action']=='restore_referral' and c['status']!='earned':
        delta=c['penalty']+(c['revoked'] if c['reason']!='refund' else 0)
    result={'request':r,'state':state,'effect':{'affiliate_delta_credit':delta,
            'target_active':r['action']=='restore_referral','registration_reissue_credit':0}}
    return {**result,'confirmation_sha256':digest(result)}


def apply_sql(plan, confirmation):
    if confirmation!=plan['confirmation_sha256'] or preview(plan['request'],plan['state'])!=plan:
        raise ValueError('confirmation/preview mismatch')
    r=plan['request']; snapshot=snapshot_sql(r)
    # Exact retries remain no-ops even after an appeal changed current state.
    sql = f'''BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on;
DO $moderation$ DECLARE current_snapshot jsonb; prior core_referrals.moderation%ROWTYPE;
BEGIN
 IF current_user<>pg_get_userbyid((SELECT nspowner FROM pg_namespace WHERE nspname='core_referrals')) THEN
 RAISE EXCEPTION 'offline owner required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('referral-moderate:'||{lit(r['request_id'])},0));
 SELECT * INTO prior FROM core_referrals.moderation WHERE request_id={lit(r['request_id'])};
 IF NOT FOUND THEN
  PERFORM id FROM core_identity.users WHERE id IN ({r['actor']},{r['user_id']}) ORDER BY id FOR UPDATE;
  PERFORM id FROM core_referrals.contracts WHERE invitee={r['user_id']} FOR UPDATE;
  PERFORM b.ledger_account_id FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id
    JOIN core_identity.users u ON u.personal_account_id=a.owner_account_id JOIN core_referrals.contracts c ON c.inviter=u.id
    WHERE c.invitee={r['user_id']} AND a.bucket='affiliate' FOR NO KEY UPDATE OF b;
  {snapshot} INTO current_snapshot;
  IF current_snapshot IS DISTINCT FROM {jlit(plan['state'])} THEN
   RAISE EXCEPTION 'moderation preview stale'; END IF;
 END IF;
 PERFORM core_referrals.moderate({lit(r['request_id'])},{r['actor']},{r['user_id']},{lit(r['action'])},{lit(r['reason'])},{lit(r['evidence'])},{lit(r['penalize'])});
END $moderation$;
COMMIT;'''
    start, tail = sql.split('DO $moderation$ ', 1)
    body, end = tail.split(' $moderation$;', 1)
    return start + 'DO ' + lit(body) + ';' + end


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('mode',choices=('plan','apply'))
    p.add_argument('--socket',type=Path,required=True)
    p.add_argument('--port',type=int,required=True)
    p.add_argument('--database',required=True)
    p.add_argument('--login',required=True)
    p.add_argument('--owner',required=True)
    p.add_argument('--input',type=Path,required=True)
    p.add_argument('--output',type=Path)
    p.add_argument('--confirmation-sha256')
    a=p.parse_args()
    if not a.socket.is_absolute() or not re.fullmatch('[a-zA-Z_][a-zA-Z0-9_]*',a.owner):
        raise ValueError('local Unix socket and owner role required')
    env={k:v for k,v in os.environ.items() if not k.startswith('PG')}
    env.update(PGHOST=str(a.socket),PGPORT=str(a.port),PGDATABASE=a.database,PGUSER=a.login,
        PGAPPNAME='lmm-referral-moderation',PGOPTIONS=f'-c role={a.owner} -c default_transaction_read_only={"on" if a.mode=="plan" else "off"}')
    data=json.loads(a.input.read_text())
    sql=('BEGIN READ ONLY ISOLATION LEVEL REPEATABLE READ;'+snapshot_sql(data)+';COMMIT;') if a.mode=='plan' else apply_sql(data,a.confirmation_sha256)
    result=subprocess.run(['psql','-X','--no-password','-qAt','-v','ON_ERROR_STOP=1'],input=sql,text=True,capture_output=True,env=env)
    if result.returncode: raise RuntimeError('moderation database operation rejected; inspect private PostgreSQL log')
    if a.mode=='plan':
        if a.output is None: raise ValueError('private preview output required')
        planned=preview(data,json.loads(result.stdout))
        with os.fdopen(os.open(a.output,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'w') as f: json.dump(planned,f,sort_keys=True)
        print(json.dumps({'confirmation_sha256':planned['confirmation_sha256'],'applied':False}))
    else: print(json.dumps({'request_id':data['request']['request_id'],'applied':True}))


if __name__=='__main__': main()
