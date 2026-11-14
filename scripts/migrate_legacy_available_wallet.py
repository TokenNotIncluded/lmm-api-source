#!/usr/bin/env python3
"""Explicit partial migration: original signed users.quota only; other rights deferred.

Uses existing Core ledger functions, no DDL. Prepare is read-only and can run
live; apply uses either an actual short maintenance window or explicit online
ledger-table locking and empty-account CAS, never a never-started claim. Unconfirmed transport is never retried. All inputs/outputs are private.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import uuid

SCOPE = 'financial-available-wallet-debt-v1'
POLICY = 'original-signed-users-quota-only-no-rescale-all-other-rights-explicitly-deferred'
SAFE = 9007199254740991
IDENT = re.compile(r'[a-z_][a-z0-9_]*\Z')
HEX = re.compile(r'[0-9a-f]{64}\Z')


def canonical(v):
    return json.dumps(v,sort_keys=True,separators=(',',':'),ensure_ascii=False)


def digest(v):
    return hashlib.sha256(canonical(v).encode()).hexdigest()


def literal(v):
    return "'" + (v if isinstance(v,str) else canonical(v)).replace("'","''") + "'"


def require(v,message):
    if not v:
        raise ValueError(message)


def load(p):
    return json.loads(Path(p).read_text())


def save(directory,name,value):
    p=directory/name
    with p.open('x') as f:
        f.write(canonical(value)+'\n' if not isinstance(value,str) else value)
        f.flush();os.fsync(f.fileno())
    return p


def psql(connection,sql,output,label,*,readonly=False,appname=None):
    for key in ['database','role']:
        require(IDENT.fullmatch(connection[key]),'invalid_connection_identifier')
    login=connection.get('login_role',connection['role'])
    require(IDENT.fullmatch(login),'invalid_login')
    env={k:v for k,v in os.environ.items() if not k.startswith('PG')}
    env.update(PGDATABASE=connection['database'],PGUSER=login,PGPORT=str(connection['port']),PGCONNECT_TIMEOUT='10')
    if connection.get('transport')=='loopback-tcp':
        require(connection['host'] in ['127.0.0.1','::1'],'nonloopback_target')
        env.update(PGHOST=connection['host'],PGHOSTADDR=connection['host'],PGREQUIREAUTH='scram-sha-256')
    else:
        socket=Path(connection['socket_directory'])
        require(socket.is_absolute() and socket.stat().st_uid==os.getuid() and socket.stat().st_mode&0o077==0,'private_socket_required')
        env['PGHOST']=str(socket)
    if connection.get('pgpass_file'):
        secret=Path(connection['pgpass_file'])
        require(secret.stat().st_uid==os.getuid() and secret.stat().st_mode&0o077==0,'private_passfile_required')
        env['PGPASSFILE']=str(secret)
    options='-c timezone=UTC'
    if connection.get('set_role'):
        require(connection['set_role']==connection['role'] and login!=connection['role'],'independent_installer_required')
        options+=' -c role='+connection['role']+' -c log_parameter_max_length=0 -c log_parameter_max_length_on_error=0 -c log_error_verbosity=terse'
    if readonly:options+=' -c default_transaction_read_only=on'
    env['PGOPTIONS']=options
    if appname:env['PGAPPNAME']=appname
    save(output,label+'.sql.private',sql)
    r=subprocess.run(['psql','-XqAt','--no-password','-v','ON_ERROR_STOP=1'],input=sql,text=True,capture_output=True,env=env)
    save(output,label+'.stdout.private',r.stdout);save(output,label+'.stderr.private',r.stderr)
    save(output,label+'.exit.json',{'actual_exit':r.returncode})
    return r


def observation_sql():
    return """BEGIN READ ONLY; SELECT jsonb_build_object(
 'database',current_database(),'database_oid',(SELECT oid FROM pg_database WHERE datname=current_database()),
 'system_identifier',(pg_control_system()).system_identifier::text,'nonce',current_setting('lmm.migration_instance_nonce'),
 'role',current_user,'login',session_user,'postmaster_started_at',pg_postmaster_start_time()::text,
 'identity_import',(SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE manifest->>'scope'='identity-only'),
 'users',(SELECT jsonb_agg(jsonb_build_object('id',id,'account',personal_account_id) ORDER BY id) FROM core_identity.users)); COMMIT;"""


def prepare(plan,proof,source,identity_map,target,unit,observation,source_quotas,migration_id):
    require({x['key']:x['value'] for x in unit['observed_options']}=={'CreditsPerUSD':'500000','QuotaPerUnit':'500000'},'unit_not_exact_500000')
    require(plan['source_archive_sha256']==source['archive']['sha256']==proof['source_archive_sha256']==target['source_archive_sha256'],'source_archive_mismatch')
    require(plan['unit']=='credit_500k_usd' and plan['credits_per_usd']==500000,'source_unit_mismatch')
    native={x['id']:x['account'] for x in observation['users']}
    mapping={x['source_user_id']:(x['target_user_id'],x['target_personal_account_id']) for x in identity_map}
    require(len(mapping)==len(identity_map),'duplicate_identity_mapping')
    rows=[]
    for x in plan['wallet_positions']:
        uid=x['source_user_id'];quota=x['signed_available_units']
        require(type(quota) is int and abs(quota)<=SAFE,'invalid_raw_quota')
        require(source_quotas.get(uid)==quota,'actual_source_quota_mismatch')
        require(uid in mapping and mapping[uid][1]==x['account_id'] and native.get(mapping[uid][0])==x['account_id'],'actual_native_identity_mismatch')
        require(HEX.fullmatch(x['source_row_sha256']),'invalid_source_row_digest')
        rows.append({'source_user_id':uid,'target_user_id':mapping[uid][0],'account_id':x['account_id'],'available_units':quota,'source_row_sha256':x['source_row_sha256']})
    require(len(rows)==len(mapping)==len(source_quotas) and len({x['account_id'] for x in rows})==len(rows),'source_coverage_mismatch')
    identity=observation['identity_import']
    require(identity and identity['source_archive_sha256']==plan['source_archive_sha256'],'committed_identity_required')
    expected=target['instance_identity']
    require(str(observation['database_oid'])==str(expected['database_oid']) and observation['database']==target['connection']['database'] and observation['nonce']==expected['server_instance_nonce'],'target_instance_mismatch')
    core={'scope':SCOPE,'policy':POLICY,'source_archive_sha256':plan['source_archive_sha256'],'source_plan_mapping_sha256':plan['mapping_sha256'],'positions':rows}
    evidence=digest(core)
    review={'format_version':1,'scope':SCOPE,'decision':'approved-explicit-partial-original-quota-migration','policy':POLICY,'credits_per_usd':500000,'evidence_sha256':evidence,'reserved_other_rights_proven_none':False,'all_financial_rights_complete':False}
    return {**core,'format_version':1,'migration_id':migration_id,'mapping_sha256':evidence,'observed_at':int(__import__('datetime').datetime.fromisoformat(source['started_utc']).timestamp()),'identity_import':identity,'source_review':review,'source_files':{'source_export':{'sha256':proof['source_export_sha256']}},'connection':target['connection'],'target':{k:observation[k] for k in ['database','database_oid','system_identifier','nonce','role','login','postmaster_started_at']},'deferred':{'policy':'retain-source-and-private-snapshot-no-waiver-no-credit-created','families':{k:v for k,v in plan['families'].items() if k!='wallet_positions'},'affiliate_positions':[{'source_user_id':x['source_user_id'],'signed_affiliate_units':x['signed_affiliate_units']} for x in plan['wallet_positions'] if x['signed_affiliate_units']!=0],'source_blockers':plan['blocker_counts']},'target_has_previously_started':True,'apply_requires_actual_current_maintenance':True}


def build_sql(material,*,reconcile=False,online=False):
    require(material['scope']==SCOPE and material['policy']==POLICY,'partial_profile_mismatch')
    bound={k:material[k] for k in ['scope','policy','source_archive_sha256','source_plan_mapping_sha256','positions']}
    require(digest(bound)==material['mapping_sha256']==material['source_review']['evidence_sha256'],'partial_material_binding_changed')
    require(all(type(x['available_units']) is int and abs(x['available_units'])<=SAFE for x in material['positions']),'partial_amount_invalid')
    m=material;t=m['target'];payload=[]
    for x in m['positions']:
        payload.append({'account_id':x['account_id'],'available_units':x['available_units'],'affiliate_units':0,'reserved_units':0,'mapping_sha256':m['mapping_sha256'],'migration_id':m['migration_id'],'observed_at':m['observed_at'],'source_archive_sha256':m['source_archive_sha256'],'source_entity_key':'legacy.users/'+str(x['source_user_id']),'source_row_sha256':x['source_row_sha256']})
    manifest={'format_version':1,'scope':SCOPE,'partial':True,'full_financial_import_complete':False,'material':m}
    body=f"""DO $wallet_partial$ DECLARE p jsonb; prior jsonb; payload jsonb := {literal(payload)}::jsonb; BEGIN
 IF current_database()<>{literal(t['database'])} OR current_user<>{literal(t['role'])} OR session_user<>{literal(t['login'])}
 OR (SELECT oid FROM pg_database WHERE datname=current_database())<>{int(t['database_oid'])}
 OR (pg_control_system()).system_identifier::text<>{literal(t['system_identifier'])}
 OR current_setting('lmm.migration_instance_nonce')<>{literal(t['nonce'])}
 THEN RAISE EXCEPTION 'wallet target physical identity mismatch'; END IF;
 SELECT manifest INTO prior FROM core_migration.import_runs WHERE migration_id={literal(m['migration_id'])};
 IF FOUND AND prior IS DISTINCT FROM {literal(manifest)}::jsonb THEN RAISE EXCEPTION 'partial profile replay mismatch'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core_migration.import_runs r WHERE to_jsonb(r)={literal(m['identity_import'])}::jsonb)
 THEN RAISE EXCEPTION 'committed identity binding changed'; END IF;
"""
    if not reconcile and not online:
        body+=""" IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid() AND backend_type='client backend') THEN RAISE EXCEPTION 'maintenance target has another client'; END IF;
"""
    for x in m['positions']:
        body+=f" IF NOT EXISTS(SELECT 1 FROM core_identity.users WHERE id={x['target_user_id']} AND personal_account_id={x['account_id']}) THEN RAISE EXCEPTION 'native identity changed'; END IF;\n"
    if not reconcile:
        body+=" FOR p IN SELECT value FROM jsonb_array_elements(payload) LOOP PERFORM core_billing.import_opening_balance(p); END LOOP;\n"
        body+=f" INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES({literal(m['migration_id'])},{literal(m['source_archive_sha256'])},{literal(m['mapping_sha256'])},{literal(manifest)}::jsonb) ON CONFLICT DO NOTHING;\n"
    body+=f""" FOR p IN SELECT value FROM jsonb_array_elements(payload) LOOP
 IF NOT EXISTS(SELECT 1 FROM core_billing.opening_imports WHERE migration_id={literal(m['migration_id'])} AND request=p)
 THEN RAISE EXCEPTION 'partial opening receipt mismatch'; END IF;
 IF (SELECT count(*) FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.owner_account_id=(p->>'account_id')::bigint AND ((a.bucket='wallet' AND b.balance_units=greatest((p->>'available_units')::bigint,0)) OR (a.bucket='debt' AND b.balance_units=least((p->>'available_units')::bigint,0))))<>2
 THEN RAISE EXCEPTION 'source quota wallet debt exact readback mismatch'; END IF;
 END LOOP;
 IF (SELECT count(*) FROM core_billing.opening_imports WHERE migration_id={literal(m['migration_id'])})<>jsonb_array_length(payload)
 OR EXISTS(SELECT 1 FROM core_billing.ledger_journals j JOIN core_billing.ledger_entries e ON e.journal_id=j.id JOIN core_billing.opening_imports o ON (o.result->>'journal_id')::bigint=j.id WHERE o.migration_id={literal(m['migration_id'])} GROUP BY j.id HAVING sum(e.delta_units::numeric)<>0)
 THEN RAISE EXCEPTION 'partial opening count or ledger conservation mismatch'; END IF;
 END $wallet_partial$;
 SELECT jsonb_build_object('status','COMMITTED_AND_EXACT_READBACK','scope',{literal(SCOPE)},'migration_id',{literal(m['migration_id'])},'accounts',{len(payload)},'full_financial_import_complete',false);
 COMMIT;
"""
    if reconcile:
        current_check=""" IF (SELECT count(*) FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.owner_account_id=(p->>'account_id')::bigint AND ((a.bucket='wallet' AND b.balance_units=greatest((p->>'available_units')::bigint,0)) OR (a.bucket='debt' AND b.balance_units=least((p->>'available_units')::bigint,0))))<>2
 THEN RAISE EXCEPTION 'source quota wallet debt exact readback mismatch'; END IF;"""
        opening_delta_check=""" IF coalesce((SELECT sum(e.delta_units) FROM core_billing.opening_imports o JOIN core_billing.ledger_entries e ON e.journal_id=(o.result->>'journal_id')::bigint JOIN core_billing.ledger_accounts a ON a.id=e.ledger_account_id WHERE o.request=p AND a.bucket='wallet'),0)<>greatest((p->>'available_units')::bigint,0)
 OR coalesce((SELECT sum(e.delta_units) FROM core_billing.opening_imports o JOIN core_billing.ledger_entries e ON e.journal_id=(o.result->>'journal_id')::bigint JOIN core_billing.ledger_accounts a ON a.id=e.ledger_account_id WHERE o.request=p AND a.bucket='debt'),0)<>least((p->>'available_units')::bigint,0)
 THEN RAISE EXCEPTION 'immutable opening wallet debt delta mismatch'; END IF;"""
        require(body.count(current_check)==1,'readback_guard_construction_mismatch')
        body=body.replace(current_check,opening_delta_check,1)
        return "SELECT pg_advisory_lock(741803121);\nBEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY;\nSET LOCAL timezone='UTC';\n"+body
    return "BEGIN ISOLATION LEVEL SERIALIZABLE;\nSET LOCAL synchronous_commit=on;\nSET LOCAL standard_conforming_strings=on;\nSET LOCAL timezone='UTC';\nSET LOCAL lock_timeout='5s';\nSET LOCAL statement_timeout='120s';\nSELECT pg_advisory_xact_lock(741803121);\nLOCK TABLE core_billing.opening_import_runs,core_billing.opening_imports,core_billing.ledger_accounts,core_billing.balance_state,core_billing.ledger_journals,core_billing.ledger_entries,core_billing.ledger_operations IN SHARE ROW EXCLUSIVE MODE;\n"+body


def main():
    os.umask(0o077)
    p=argparse.ArgumentParser(description=__doc__);mode=p.add_mutually_exclusive_group(required=True)
    for name in ['prepare','apply','reconcile']:mode.add_argument('--'+name,action='store_true')
    for name in ['plan','export-proof','source-receipt','identity-map','target-receipt','unit-evidence','material','maintenance-receipt']:p.add_argument('--'+name,type=Path)
    p.add_argument('--online',action='store_true',help='Explicit partial-profile online apply using SERIALIZABLE ledger table locks and empty-account CAS')
    p.add_argument('--migration-id');p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(mode=0o700)
    outcome={'actual_exit':1,'target_write_attempted':False,'automatic_retry':False}
    try:
        require(not a.online or a.apply,'online_only_for_explicit_apply')
        if a.prepare:
            source=load(a.source_receipt);target=load(a.target_receipt)
            obs=psql(target['connection'],observation_sql(),a.output,'target-observation',readonly=True);require(obs.returncode==0,'target_observation_failed')
            schema=source['source']['schema'];require(IDENT.fullmatch(schema),'invalid_source_schema')
            raw=psql(source['readonly_reader'],f'BEGIN READ ONLY; SELECT jsonb_agg(jsonb_build_object(\'id\',id,\'quota\',quota) ORDER BY id) FROM {schema}.users; COMMIT;',a.output,'source-raw-quota',readonly=True);require(raw.returncode==0,'source_quota_read_failed')
            quotas={x['id']:x['quota'] for x in json.loads(raw.stdout)}
            material=prepare(load(a.plan),load(a.export_proof),source,[json.loads(x) for x in a.identity_map.read_text().splitlines() if x],target,load(a.unit_evidence),json.loads(obs.stdout),quotas,a.migration_id)
            save(a.output,'material.private.json',material);save(a.output,'opening.private.sql',build_sql(material));save(a.output,'deferred-rights.private.json',material['deferred']);outcome.update(actual_exit=0,status='PARTIAL_AVAILABLE_WALLET_PREPARED_NOT_APPLIED',accounts=len(material['positions']))
        else:
            material=load(a.material)
            require(material['scope']==SCOPE and material['policy']==POLICY,'profile_mismatch')
            if a.apply and not a.online:
                maintenance=load(a.maintenance_receipt)
                require(maintenance.get('core_service_active_state')=='inactive' and maintenance.get('stop_actual_exit')==0 and maintenance.get('target_database')==material['target']['database'] and time.time()-maintenance['recorded_epoch']<300,'actual_current_short_maintenance_required')
            appname='lmm-available-wallet-'+uuid.uuid4().hex
            save(a.output,'attempt.private.json',{'application_name':appname,'migration_id':material['migration_id'],'scope':SCOPE,'target':material['target'],'status':'READONLY_RECONCILE' if a.reconcile else 'MAY_HAVE_BEEN_SENT','execution_mode':'ONLINE_SERIALIZABLE_LEDGER_LOCKS_EMPTY_ACCOUNT_CAS' if a.online else 'SHORT_MAINTENANCE','target_has_previously_started':True,'started_epoch':time.time()})
            outcome['target_write_attempted']=a.apply
            result=psql(material['connection'],build_sql(material,reconcile=a.reconcile,online=a.online),a.output,'reconcile' if a.reconcile else 'apply',readonly=a.reconcile,appname=appname)
            if result.returncode==0:outcome.update(actual_exit=0,status='COMMITTED_AND_EXACT_READBACK' if a.apply else 'READONLY_RECONCILED')
            else:outcome.update(status='OUTCOME_UNCONFIRMED' if a.apply else 'RECONCILE_FAILED',psql_exit=result.returncode)
    except BaseException as e:
        outcome.update(status='OUTCOME_UNCONFIRMED' if outcome['target_write_attempted'] else 'REFUSED_BEFORE_WRITE',error=str(e))
    finally:save(a.output,'result.private.json',outcome)
    print(canonical(outcome));return outcome['actual_exit']

if __name__=='__main__':sys.exit(main())
