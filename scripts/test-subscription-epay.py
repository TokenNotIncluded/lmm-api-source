#!/usr/bin/env python3
"""Real isolated PG18 SQL evidence; Rust auth/signature/RPC are tested separately.

Requires an empty *_test database in DATABASE_URL. --capture-target writes pins
only after proving the exact Core58 predecessor and fresh/upgrade equivalence.
"""
from concurrent.futures import ThreadPoolExecutor
import argparse
import importlib.util
import json
import os
from pathlib import Path
import secrets
import subprocess

ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / 'services/core'
spec = importlib.util.spec_from_file_location('schema_fixture', ROOT/'scripts/test-commerce-self-purchase.py')
f = importlib.util.module_from_spec(spec)
spec.loader.exec_module(f)
need, sql = f.need, f.sql


def literal(value):
    if isinstance(value, int): return str(value)
    if not isinstance(value, str): value=json.dumps(value,separators=(',',':'))
    return "'"+value.replace("'","''")+"'"


def failure(statement, role):
    p=subprocess.run(['psql','-X','-q','-At','-v','ON_ERROR_STOP=1',os.environ['DATABASE_URL']],input=f'SET ROLE "{role}";\n'+statement,text=True,capture_output=True,timeout=30)
    need(p.returncode!=0,'expected rejection: '+statement[:100])
    return p.stderr


def snapshot(tables,sequences,owner):
    queries=[]
    for table in tables:
        if table in ('core_meta.component_contract','core_meta.runtime_contract'):continue
        row="(to_jsonb(t)-'subscription_order_id')" if table=='core_payments.provider_identifiers' else 'to_jsonb(t)'
        queries.append(f"SELECT jsonb_build_array('{table}',coalesce(jsonb_agg({row} ORDER BY {row}::text),'[]'::jsonb))::text FROM {table} t")
    queries += [f"SELECT jsonb_build_array('{seq}',last_value::text,log_cnt::text,is_called)::text FROM {seq}" for seq in sequences]
    return f.framed(sorted(sql(';\n'.join(queries),owner).splitlines()))


def acl_snapshot(rels,types,funcs,owner):
    source=(CORE/'src/core_schema/upgrade/finance.rs').read_text()
    query=source.split('pub(super) async fn permissions(',1)[1].split('r#"',1)[1].split('"#',1)[0]
    query=query.replace('$1','ARRAY['+','.join(rels)+']::bigint[]').replace('$2','ARRAY['+','.join(types)+']::bigint[]')
    query=query.replace('FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace','FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace WHERE p.oid IN ('+','.join(funcs)+')')
    added=sql("SELECT attnum FROM pg_attribute WHERE attrelid='core_payments.provider_identifiers'::regclass AND attname='subscription_order_id' AND NOT attisdropped AND attacl IS NULL",owner)
    rows=[]
    for fact in sql(query,owner).splitlines():
        v=json.loads(fact)
        if added and v==['column','core_payments','provider_identifiers',int(added),None]:continue
        rows.append(fact)
    return f.framed(rows)


def apply(order,proof,key,role):
    # Production SQL entry, including real COMMIT/deferred constraints. Only
    # SERIALIZABLE aborts retry the complete original operation.
    statement=(CORE/'src/subscriptions/external_grant.sql').read_text().replace('$1',literal(order)).replace('$2',literal(proof)).replace('$3',literal(key))
    for _ in range(8):
        p=subprocess.run(['psql','-X','-q','-At','-v','ON_ERROR_STOP=1','-v','VERBOSITY=verbose',os.environ['DATABASE_URL']],input=f'SET ROLE "{role}"; BEGIN ISOLATION LEVEL SERIALIZABLE;\n'+statement+';COMMIT;',text=True,capture_output=True,timeout=30)
        if not p.returncode:return json.loads(p.stdout.strip())
        if '40001' not in p.stderr:raise AssertionError(p.stderr)
    raise AssertionError('serialization retry budget exhausted')


def main():
    args=argparse.ArgumentParser();args.add_argument('--capture-target',action='store_true');args=args.parse_args()
    components,metadata,manifest=f.source_contract()
    need(len(components)==59 and components[58][0]=='subscription-epay','exact Core59 assembly required')
    pre=json.loads((CORE/'src/core_schema/upgrade/commerce_display_rate_target.json').read_text())
    need(f.manifest_hash(dict(manifest,components=manifest['components'][:58]))==pre['manifest_sha256'],'Core58 manifest changed')
    facts=json.loads(sql("SELECT jsonb_build_array(current_database(),current_setting('server_version_num')::int,EXISTS(SELECT FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema','public') AND left(nspname,3)<>'pg_'))"))
    need(facts[0].endswith('_test') and 180000<=facts[1]<190000 and not facts[2],'fresh private PG18 *_test required')
    suffix=secrets.token_hex(4);owner='sub_owner_'+suffix;runtime='sub_runtime_'+suffix
    sql(f'CREATE ROLE "{owner}" NOLOGIN;CREATE ROLE "{runtime}" NOLOGIN;ALTER DATABASE "{facts[0]}" OWNER TO "{owner}";REVOKE ALL ON SCHEMA public FROM PUBLIC;GRANT USAGE,CREATE ON SCHEMA public TO "{owner}";')
    sql('BEGIN;\n'+'\n'.join(ddl for _,ddl in components[:58])+'\n'+'\n'.join(metadata)+'\nCOMMIT;',owner)
    need(f.framed(sql(manifest['catalog']['query_utf8'],owner).splitlines())==pre['catalog_sha256'],'Core58 catalog changed')
    print('PASS exact Core58 manifest/catalog',flush=True)
    sql("""INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES(8301,'personal'),(8302,'personal');
INSERT INTO core_identity.users(id,personal_account_id,platform_role) VALUES(8301,8301,'superadmin'),(8302,8302,'user');
SELECT core_billing.open_ledger_wallet(8302);
SELECT core_billing.post_ledger('{"scope":"epay-upgrade-fixture","operation_key":"original-wallet-credit","reason":"Synthetic unchanged original balance","action":{"kind":"credit","account_id":8302,"amount_units":10000,"payment_reference":"synthetic-original"}}');
INSERT INTO core_payments.intents(account_id,created_by_user_id,channel_id,provider,merchant,environment,currency,amount_minor,credit_units,credits_per_minor,prepare_key,prepare_hash)
VALUES(8302,8302,7,'epay','1001','test','CNY',125,125,1,'native-original-key',decode(repeat('00',32),'hex'));
INSERT INTO core_payments.provider_identifiers(provider,merchant,environment,kind,identifier,native_intent_id)
VALUES('epay','1001','test','payment','native-transaction',1);
""",owner)
    for name,ddl in components[:58]:sql(f"INSERT INTO core_meta.component_contract VALUES({literal(name)},decode('{f.sha(ddl.encode())}','hex'))",owner)
    sql(f"INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES(decode('{pre['catalog_sha256']}','hex'))",owner)
    sql(f'GRANT USAGE ON SCHEMA core_identity,core_billing,core_subscriptions,core_charging,core_payments TO "{runtime}";GRANT SELECT ON core_identity.users,core_subscriptions.plan_revisions,core_subscriptions.plans TO "{runtime}";GRANT SELECT,INSERT ON core_payments.provider_identifiers TO "{runtime}";',owner)
    baseline=(ROOT/'deployment/postgres/grant-core-runtime.sql').read_text().split('-- BEGIN subscriptions runtime grants\n',1)[1].split('-- BEGIN subscription-epay runtime grants',1)[0]
    sql(baseline.replace(':"runtime_role"',f'"{runtime}"'),owner)
    app="n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'"
    tables=sql(f"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {app} AND c.relkind IN ('r','p') ORDER BY 1",owner).splitlines()
    seq=sql(f"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {app} AND c.relkind='S' ORDER BY 1",owner).splitlines()
    rels=sql(f"SELECT c.oid::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {app} ORDER BY 1",owner).splitlines()
    types=sql(f"SELECT t.oid::text FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE {app} ORDER BY 1",owner).splitlines()
    funcs=sql(f"SELECT p.oid::text FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE {app} ORDER BY 1",owner).splitlines()
    old_data=snapshot(tables,seq,owner);old_acl=acl_snapshot(rels,types,funcs,owner)
    failure('BEGIN;\n'+components[58][1]+"\nSELECT 1/0;COMMIT;",owner)
    need(f.framed(sql(manifest['catalog']['query_utf8'],owner).splitlines())==pre['catalog_sha256'],'failed migration did not roll back')
    sql('BEGIN;\n'+components[58][1]+'\n'+(CORE/'schema/subscription_epay_grants.sql').read_text().replace(':"runtime_role"',f'"{runtime}"')+'\nCOMMIT;',owner)
    need(snapshot(tables,seq,owner)==old_data,'predecessor data changed')
    need(acl_snapshot(rels,types,funcs,owner)==old_acl,'predecessor ACL changed')
    target={'catalog_sha256':f.framed(sql(manifest['catalog']['query_utf8'],owner).splitlines()),'manifest_sha256':f.manifest_hash(manifest)}
    # A second truly fresh DB must deparse to the same catalog as the upgrade.
    original=os.environ['DATABASE_URL'];fresh_name=facts[0]+'_fresh_test'
    sql(f'CREATE DATABASE "{fresh_name}"')
    os.environ['DATABASE_URL']=original.replace('/'+facts[0]+'?', '/'+fresh_name+'?')
    need(os.environ['DATABASE_URL']!=original,'Unix-socket URL with explicit query required')
    try:
        sql(f'ALTER DATABASE "{fresh_name}" OWNER TO "{owner}";REVOKE ALL ON SCHEMA public FROM PUBLIC;GRANT USAGE,CREATE ON SCHEMA public TO "{owner}";')
        sql('BEGIN;\n'+'\n'.join(ddl for _,ddl in components)+'\n'+'\n'.join(metadata)+'\nCOMMIT;',owner)
        need(f.framed(sql(manifest['catalog']['query_utf8'],owner).splitlines())==target['catalog_sha256'],'fresh and upgraded catalogs differ')
    finally:os.environ['DATABASE_URL']=original
    if args.capture_target:
        for path in ('subscription_epay_target.json','fresh_target.json'):
            (CORE/'src/core_schema/upgrade'/path).write_text(json.dumps(target,indent=2)+'\n')
    else:
        for path in ('subscription_epay_target.json','fresh_target.json'):
            need(json.loads((CORE/'src/core_schema/upgrade'/path).read_text())==target,'target pin mismatch '+path)
    print('PASS failed-upgrade rollback; predecessor data/ACL preservation; fresh/upgrade pin equivalence',flush=True)
    terms={'title':'Synthetic EPay plan','price_units':'100000','duration_seconds':3600,'duration_months':0,'reset_period':'grant','reset_seconds':0,'limit_units':'1000','models':['*'],'groups':['*'],'max_purchases_per_user':4,'enabled':True}
    sql(f"BEGIN;INSERT INTO core_subscriptions.plans(revision) VALUES(1);INSERT INTO core_subscriptions.plan_revisions VALUES(1,1,{literal(terms)}::jsonb,8301,1);COMMIT;INSERT INTO core_subscriptions.external_prices VALUES(1,1,7,'CNY',125,8301);",owner)
    def new_order(char,confirmed=True,canceled=False):
        ident='sub_'+char*64
        sql(f"INSERT INTO core_subscriptions.external_orders(id,actor_user_id,account_id,operation_key,plan_id,plan_revision,channel_id,provider,merchant,environment,currency,amount_minor,channel_binding,quoted_at,expires_at,confirmed_at,canceled_at) VALUES('{ident}',8302,8302,'operation_{char*16}',1,1,7,'epay','1001','test','CNY',125,'{{}}',1,601,{'10' if confirmed else 'NULL'},{'20' if canceled else 'NULL'})",runtime)
        return ident
    a=new_order('a',True,True);b=new_order('b',False);c=new_order('c')
    p='sproof_'+'d'*64
    sql(f"INSERT INTO core_payments.provider_identifiers(provider,merchant,environment,kind,identifier,subscription_order_id) VALUES('epay','1001','test','payment','subscription-transaction','{a}');INSERT INTO core_subscriptions.external_proofs VALUES('{p}','{a}','event-sub','subscription-transaction',decode(repeat('11',32),'hex'),'synthetic-signature-input','synthetic-signature',20);",runtime)
    wallet=sql('SELECT jsonb_agg(to_jsonb(j)) FROM core_billing.ledger_journals j',owner)
    with ThreadPoolExecutor(max_workers=8) as pool:receipts=list(pool.map(lambda _:apply(a,p,'same-original-operation',runtime),range(8)))
    need(all(r==receipts[0] for r in receipts),'concurrent replays changed receipt')
    need(sql('SELECT count(*) FROM core_subscriptions.external_grants',owner)=='1','multiple grants')
    need(sql("SELECT count(*) FROM core_charging.subscriptions WHERE grant_reference LIKE 'external-subscription:%'",owner)=='1','multiple subscription grants')
    need(sql('SELECT jsonb_agg(to_jsonb(j)) FROM core_billing.ledger_journals j',owner)==wallet,'wallet journal changed')
    failure(f"BEGIN ISOLATION LEVEL SERIALIZABLE;SELECT core_subscriptions.apply_epay_grant('{a}','{p}','different-original-key');COMMIT;",runtime)
    failure(f"BEGIN ISOLATION LEVEL SERIALIZABLE;SELECT core_subscriptions.apply_epay_grant('{b}','{p}','same-original-operation');COMMIT;",runtime)
    failure(f"UPDATE core_subscriptions.external_orders SET amount_minor=126 WHERE id='{a}'",owner)
    failure(f"INSERT INTO core_payments.provider_identifiers(provider,merchant,environment,kind,identifier,subscription_order_id) VALUES('epay','1001','test','payment','native-transaction','{c}')",runtime)
    failure(f"INSERT INTO core_payments.provider_identifiers(provider,merchant,environment,kind,identifier,native_intent_id) VALUES('epay','1001','test','payment','subscription-transaction',1)",runtime)
    failure(f"DELETE FROM core_subscriptions.external_proofs WHERE id='{p}'",owner)
    failure('TRUNCATE core_subscriptions.external_grants',owner)
    failure('SELECT * FROM core_meta.subscription_epay_upgrades',runtime)
    failure(f"UPDATE core_subscriptions.external_orders SET account_id=8301 WHERE id='{a}'",runtime)
    # A grant without the namespace claim fails at COMMIT, rolling back its insert.
    p2='sproof_'+'e'*64
    sql(f"INSERT INTO core_subscriptions.external_proofs VALUES('{p2}','{c}','event-unclaimed','unclaimed',decode(repeat('22',32),'hex'),'synthetic-input','synthetic-signature',20)",runtime)
    failure(f"BEGIN ISOLATION LEVEL SERIALIZABLE;SELECT core_subscriptions.apply_epay_grant('{c}','{p2}','same-original-operation');COMMIT;",runtime)
    need(sql('SELECT count(*) FROM core_subscriptions.external_grants',owner)=='1','failed transaction leaked receipt')
    need(sql("SELECT count(*) FROM core_charging.subscriptions WHERE grant_reference LIKE 'external-subscription:%'",owner)=='1','failed transaction leaked grant')
    need(sql("SELECT core_subscriptions.external_ends_at('{\"duration_months\":1,\"duration_seconds\":0}',extract(epoch FROM '2026-01-31 12:34:56+00'::timestamptz)::bigint)=extract(epoch FROM '2026-03-03 12:34:56+00'::timestamptz)::bigint",runtime)=='t','calendar month overflow differs from Go')
    print('PASS eight concurrent same-operation grants; canceled-issued late payment; immutable replay; wallet untouched; namespace collision both directions; failure rollback; least privileges',flush=True)
    report={'evidence':'real PostgreSQL SQL only; synthetic accepted proof fixtures; Rust signature/session/RPC separate','target':target,'predecessor':pre,'preserved_data_sha256':old_data,'preserved_permissions_sha256':old_acl,'grant':receipts[0]}
    path=os.environ.get('SUB01_SQL_REPORT')
    if path:Path(path).write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report),flush=True)

if __name__=='__main__':main()
