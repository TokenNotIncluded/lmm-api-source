#!/usr/bin/env python3
"""Synthetic Core78 c343 -> final85 -> final86 SQL/owner regression.

No Cargo or live source/target connection. Frozen schema installation is
SQL qualification, separate from the compiled owner upgrade entry point. A genuine c343 transaction must
commit before any alias material is prepared; no current identity receipt is
fabricated. All application records are invented.
"""
from copy import deepcopy
import argparse
import sys
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import signal
import subprocess
import time
import traceback
import uuid

import current_affiliate_aliases as A
SHA85='2c4563c35a29fe5c0d579c5d0ea41e9300a3027343fbae641ad098d5524e8d80'
CAT85='1890e9f888a13b535dd653bffe5771745c6e4e44421211ae6723fcffedd64462'


def sha(raw):return hashlib.sha256(raw).hexdigest()


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--capture85',type=Path,required=True)
    parser.add_argument('--support',type=Path,required=True)
    args=parser.parse_args(argv)
    SOURCE=args.source.resolve();CAPTURE=args.capture85.resolve();SUPPORT=args.support.resolve()
    sys.path.insert(1,str(SUPPORT))
    import check_final81_gift_pg as B
    import current_key_lineage as L
    import final81_target_guard as T
    import test_current_identity_pg as IP
    import test_current_key_production as CT
    import test_migrate_current_runtime_keys_pg as K
    A.frozen_helpers()
    os.umask(0o077)
    nonce=secrets.token_hex(32);short=nonce[:12]
    work=Path.home()/'.local/state/lmm-api'/('alias86-test-'+short);work.mkdir(mode=0o700)
    sock=work/'socket';sock.mkdir(mode=0o700);data=work/'pgdata';port=str(54000+secrets.randbelow(9000))
    db='lmm_runtime_keys_test_alias_'+short
    owner,installer,runtime=('alias86_'+n+'_'+short for n in ('owner','install','runtime'))
    env={k:os.environ[k] for k in ('PATH','HOME','LANG') if k in os.environ}
    env.update(PGCONNECT_TIMEOUT='5',PGCLIENTENCODING='UTF8')
    result={'status':'INCOMPLETE','scope':'FINAL_86_SQL_AND_REAL_C343_ALIAS_MECHANICS',
            'work_directory':str(work),'nonce':nonce,'production_access':False,
            'compiled_binary_observation':False,'final_target_bound':False,
            'predecessor_scope':'FROZEN_FINAL85_SQL_CONTRACT','checks':[]}
    count=0;started=False

    def save(name,value):
        (work/name).write_text(json.dumps(value,ensure_ascii=False,sort_keys=True,indent=2)+'\n')

    def run(label,argv,body=None,*,allowed=(0,),override=None,timeout=180):
        cp=subprocess.run(argv,input=body,text=True,capture_output=True,env=override or env,timeout=timeout)
        (work/(label+'.stdout')).write_text(cp.stdout);(work/(label+'.stderr')).write_text(cp.stderr)
        save(label+'.result.json',{'argv':argv,'exit_code':cp.returncode,
              'stdout_sha256':sha(cp.stdout.encode()),'stderr_sha256':sha(cp.stderr.encode())})
        if cp.returncode not in allowed:raise RuntimeError(label+' failed; retained private output')
        return cp.stdout

    def sql(body,*,admin=False,database=db):
        nonlocal count
        count+=1
        e=env if admin else {**env,'PGOPTIONS':'-c role='+owner}
        return run('setup-'+str(count),['psql','-XqAt','-v','ON_ERROR_STOP=1','-h',str(sock),'-p',port,
                   '-U','fixture_admin' if admin else installer,'-d',database],body,override=e)

    def catalog(query,column='definition'):
        rows=json.loads(sql('SELECT json_agg('+column+' ORDER BY '+column+' COLLATE "C") FROM ('+query+') q;'))
        h=hashlib.sha256()
        for row in rows:
            raw=row.encode();h.update(len(raw).to_bytes(8,'big'));h.update(raw)
        return h.hexdigest()

    try:
        raw85=(CAPTURE/'manifest85.json').read_bytes();assert sha(raw85)==SHA85
        m85=json.loads(raw85);assert len(m85['components'])==85
        source_bindings=(CAPTURE/'source-bindings.json').read_bytes()
        assert sha(source_bindings)=='7f012aa45d68432560d0defad1580f5f56934c8f6d75432e08e328eb36b0bf51'
        resolved=B.source_sql_bytes(SOURCE,m85['components'])
        assert [r[0]['name'] for r in resolved]==[r['name'] for r in json.loads(source_bindings)]
        save('source-bindings.json',[{'name':c['name'],'path':str(p),'sha256':sha(raw)} for c,p,raw in resolved])
        m78=deepcopy(m85);m78['components']=m78['components'][:78]
        raw78=(json.dumps(m78,separators=(',',':'))+'\n').encode()
        c78=L.U.core78_contract(raw78);(work/'manifest78.json').write_bytes(raw78)
        schema_source=(SOURCE/'services/core/src/core_schema.rs').read_text()
        metadata=[json.loads(re.search(r'const CONTRACT: &str = (".*?");',schema_source).group(1))]
        for name in ('UPGRADE_METADATA','ADDITIVE_UPGRADE_METADATA','LEDGER_REVISION_METADATA',
                     'CONSOLE_REVISION_METADATA','OFFLINE_REVISION_METADATA'):
            rel=re.search('const '+name+r': &str = include_str!\("([^"]+)"\);',schema_source).group(1)
            metadata.append((SOURCE/'services/core/src'/rel).resolve().read_text())
        assert sha(''.join(metadata).encode())==m85['installer_contract_sql_sha256']
        grant78=run('read-grant78',['git','-C',str(SOURCE),'show',
              'fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5:deployment/postgres/grant-core-runtime.sql'])
        (work/'grant78.sql').write_text(grant78)
        verify78=run('read-grant78-owner-check',['git','-C',str(SOURCE),'show',
              'fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5:deployment/postgres/verify-channel-observations-owner.sql'])
        (work/'verify-channel-observations-owner.sql').write_text(verify78)
        result['inputs']={'driver_sha256':sha(Path(__file__).read_bytes()),
          'module_sha256':sha(Path(A.__file__).read_bytes()),'ddl86_sha256':sha(A.schema_path().read_bytes()),
          'manifest85_sha256':sha(raw85),'grant78_sha256':sha(grant78.encode()),
          'grant78_owner_check_sha256':sha(verify78.encode()),
          'source_bindings_sha256':sha(source_bindings),'installer_metadata_sha256':sha(''.join(metadata).encode())}
        run('initdb',['initdb','-D',str(data),'--username=fixture_admin','--auth-local=trust',
                     '--auth-host=reject','--encoding=UTF8','--no-locale'])
        started=True
        run('start',['pg_ctl','-D',str(data),'-l',str(work/'postgres.log'),'-o',
              f"-k {sock} -p {port} -c listen_addresses='' -c timezone=UTC -c log_timezone=UTC -c autovacuum=off -c unix_socket_permissions=0700 -c max_connections=30 -c lmm.migration_instance_nonce={nonce}",
              '-w','start'])
        sql(f'CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT; CREATE ROLE {runtime} LOGIN;'
            f' GRANT {owner} TO {installer} WITH INHERIT FALSE,SET TRUE; CREATE DATABASE {db} OWNER {owner};'
            f' REVOKE ALL ON DATABASE {db} FROM PUBLIC; GRANT CONNECT ON DATABASE {db} TO {installer};'
            f" COMMENT ON DATABASE {db} IS 'lmm-runtime-keys-synthetic:{nonce}';"
            f" ALTER DATABASE {db} SET lmm.migration_instance_nonce='{nonce}';",admin=True,database='postgres')
        sql(f'GRANT EXECUTE ON FUNCTION pg_control_system() TO {owner}; GRANT pg_read_all_settings TO {owner};'
            ' REVOKE CREATE ON SCHEMA public FROM PUBLIC;',admin=True)
        statements=['BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;']
        statements.extend(raw.decode() for _,_,raw in resolved[:78]);statements.extend(metadata)
        statements.append("INSERT INTO core_meta.schema_contract(version,fingerprint) VALUES("
           +str(m78['identity']['version'])+",decode('"+m78['identity']['fingerprint_sha256']+"','hex'));")
        for c in m78['components']:
            statements.append("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES("
                +T.literal(c['name'])+",decode('"+c['raw_schema_sha256']+"','hex'));")
        statements.append('COMMIT;');sql('\n'.join(statements))
        grants_source=(SOURCE/'services/core/src/identity/grants.rs').read_text()
        oauth_query=re.search(r'const CATALOG: &str = r#"(.*?)"#;',grants_source,re.S).group(1)
        oc=catalog(oauth_query,'d')
        sql("INSERT INTO core_oauth.contract(version,ddl_fingerprint,catalog_fingerprint) VALUES(1,decode('"
            +c78['component_fingerprints']['oauth-grants']+"','hex'),decode('"+oc+"','hex'));")
        actual78=catalog(c78['catalog_query']);assert actual78==L.U.CORE78_CATALOG_SHA256
        sql("INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES(decode('"+actual78+"','hex'));")
        sql((SUPPORT/'seed.sql').read_text().replace("'default'","'paid'"))
        run('grants78',['psql','-XqAt','-v','ON_ERROR_STOP=1','-h',str(sock),'-p',port,'-U',installer,'-d',db,
            '-v','runtime_role='+runtime,'-v','owner_role='+owner,'-v','enable_console=true',
            '-v','enable_web_identity=true','-v','enable_models=true','-f',str(work/'grant78.sql')],
            override={**env,'PGOPTIONS':'-c role='+owner})
        sql('ALTER ROLE '+runtime+' NOLOGIN;',admin=True)
        pgpass=work/'pgpass.private';pgpass.write_text('# isolated trust socket\n');pgpass.chmod(0o600)
        fixture={'format_version':1,'scope':'current-runtime-keys-synthetic-fixture',
          'connection':{'transport':'unix','socket_directory':str(sock),'port':int(port),'database':db,
            'role':owner,'login_role':installer,'set_role':owner,'pgpass_file':str(pgpass)},
          'fixture_nonce':nonce,'core78_manifest':str(work/'manifest78.json'),'runtime_roles':[runtime],
          'fixture_user_id':1,'fixture_group':'paid'}
        reg=IP.Regression(fixture,work/'regression')
        reg.seed_sessions_and_budget();reg.seed_identity()
        # Add invented full source columns BEFORE the actual identity plan/commit.
        # Match the already-validated c343 routing/history fixture semantics.
        for row in reg.bundle['source']['data']['users']:
            if row['id']==108:row['trust_level_override']=1
            if row['id']==102:row['status']=2  # Explicitly disabled source owner retains its alias.
        src=reg.bundle['source']
        src['source_identity_facts_sha256']=L.ID.digest(L.ID.identity_facts(src['data']))
        src['current_policy_records']=L.ID.mapped_current(src['data'],src['source_archive_sha256'])
        reg.bundle['current']=src['current_policy_records']
        for row in reg.bundle['routing']['facts']['users']:
            if row['id']==103:row['group']='paid'
            if row['id']==102:row['status']=2  # Same source identity, not a routing-policy override.
        for row in reg.bundle['routing']['facts']['options']:
            if row['key']=='GroupRatio':row['value']='{"paid":1}'
            elif row['key']=='UserUsableGroups':row['value']='{"paid":"Paid"}'
        L.ID.attach_routing(reg.bundle,reg.bundle['routing']['facts'])
        L.ID.attach_capabilities(reg.bundle,{'channels':[{'id':1,'type':1,'status':1,'group':'paid',
            'models':'synthetic-model','priority':0,'weight':1,'model_mapping':None}],
            'abilities':[{'group':'paid','model':'synthetic-model','channel_id':1,
                          'enabled':True,'priority':0,'weight':1,'tag':None}]})
        users=CT.attach_canonical_test_rows(reg.bundle)
        for index,row in enumerate(users):
            row['aff_code']=None if index>=len(users)-2 else f'A{index:03d}'
        users[1]['aff_code']='Ab12';users[2]['aff_code']='ab12'
        reg.bundle['canonical_source']['source_user_row_digests']=[
           {'source_user_id':row['id'],'source_row_sha256':L.F.PLANNER.digest(row)} for row in users]
        identity_id='synthetic-alias-identity-'+short
        im=reg.prepare_identity(identity_id)
        committed=reg.query(L.ID.build_sql(im,fixture_nonce=nonce))
        assert committed['status']=='COMMITTED'
        identity_run=reg.query('SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE migration_id='+T.literal(identity_id))
        mapping=[{k:row[k] for k in L.MAP_FIELDS} for row in identity_run['manifest']['mapping']]
        result['checks'].append('actual_exact78_c343_identity_committed_with_full_alias_source_rows')
        # Prefix contracts remain exact. Suffix SQL installation is test setup,
        # explicitly not a qualification of the public 85/86 upgrade procedures.
        for c,p,raw in resolved[78:]:
            sql("BEGIN;\n"+raw.decode()+"\nINSERT INTO core_meta.component_contract(name,fingerprint) VALUES("
                +T.literal(c['name'])+",decode('"+c['raw_schema_sha256']+"','hex')); COMMIT;")
        assert catalog(m85['catalog']['query_utf8'])==CAT85
        result['checks'].append('actual_final85_catalog_matches_capture')
        ddl=A.schema_path().read_bytes()
        old_inv=reg.query(T.F.inventory_sql());old_state=reg.query(T.state_sql(old_inv))
        sql('BEGIN;\n'+ddl.decode()+"\nINSERT INTO core_meta.component_contract(name,fingerprint) VALUES("
            +T.literal(A.COMPONENT_NAME)+",decode('"+sha(ddl)+"','hex')); COMMIT;")
        after_install=reg.query(T.state_sql(old_inv))
        # Only the contract registry row differs; existing businesses/sequences do not.
        assert old_state['sequences']==after_install['sequences']
        assert {k:v for k,v in old_state['tables'].items() if k!='core_meta.component_contract'}=={
            k:v for k,v in after_install['tables'].items() if k!='core_meta.component_contract'}
        result['checks'].append('final86_install_preserves_all_preexisting_rows_and_sequences')
        cat86=catalog(m85['catalog']['query_utf8'])
        sql("UPDATE core_meta.runtime_contract SET catalog_fingerprint=decode('"+cat86+"','hex') WHERE singleton;")
        m86=deepcopy(m85);m86['components'].append({'name':A.COMPONENT_NAME,'raw_schema_sha256':sha(ddl)})
        raw86=(json.dumps(m86,separators=(',',':'))+'\n').encode();(work/'manifest86.synthetic.json').write_bytes(raw86)
        binding={'component_count':86,'manifest_sha256':sha(raw86),'catalog_sha256':cat86}
        bound_contract=A.contract_from_manifest(raw86,binding)
        assert bound_contract['synthetic_only'] is False
        contract=A.contract_from_manifest(raw86,binding,synthetic=True)
        assert {k:v for k,v in contract.items() if k!='synthetic_only'}=={
            k:v for k,v in bound_contract.items() if k!='synthetic_only'}
        result['final_target_bound']=True
        sql('GRANT SELECT(code,user_id) ON core_identity.legacy_affiliate_aliases TO '+runtime+';')
        inventory=reg.query(T.F.inventory_sql())
        def observe():
            s=reg.query(A.observation_sql(mapping,identity_id,inventory=inventory,contract=contract,runtime_roles=[runtime],actor=101))
            s['fixture_nonce']=nonce
            return s
        refs={'source_archive_sha256':im['source_archive_sha256'],
              'source_export_sha256':im['source_canonical_export_sha256'],
              'source_export_proof_sha256':im['source_canonical_export_proof_sha256']}
        projection=A.project_users(users,refs,synthetic=True)
        prepared=A.prepare_owner(projection,users,observe(),contract,migration_id='synthetic-alias-import-'+short,actor=101,synthetic=True)
        review={'scope':'synthetic-alias-test-only','material_sha256':A.digest(prepared['material']),
                'production_authorization':False}
        body=A.transaction_sql(prepared,review);save('prepared.private.json',prepared);save('review.synthetic.json',review)
        before=reg.query(T.state_sql(inventory))
        # Competing owner holds the actual shared fence, proven by a lock probe.
        lock_env={**env,**L.U.I.socket_connection(reg.connection)}
        lock_env.update(PGAPPNAME='alias-fence-'+short,
                        PGOPTIONS=L.U.I.psql_options(reg.connection,readonly=False))
        holder=subprocess.Popen(['psql','-X','--no-password','-qAt','-v','ON_ERROR_STOP=1'],
            stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,text=True,env=lock_env)
        try:
            holder.stdin.write('SELECT pg_advisory_lock('+str(L.U.FENCE)+'); SELECT pg_sleep(20);\n')
            holder.stdin.close()
            ready=False
            for _ in range(50):
                ready=reg.query('SELECT NOT pg_try_advisory_xact_lock('+str(L.U.FENCE)+')')
                if ready:break
                time.sleep(.1)
            assert ready
            code,_,label=reg.run_sql(body,expect_ok=False)
            assert code!=0 and 'affiliate_alias_import_busy' in (reg.output/(label+'.stderr.private.txt')).read_text()
        finally:
            if holder.poll() is None:
                holder.send_signal(signal.SIGINT)
                try:holder.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    holder.kill();holder.wait(timeout=5)
        assert reg.query(T.state_sql(inventory))==before
        result['checks'].append('actual_competing_owner_fence_refuses_before_any_insert_or_sequence_use')
        # Missing one real row fails the deferred full-batch check; no ID use.
        excluded=next(row['source_user_id'] for row in projection['rows'] if row['code'] is not None)
        omitted=body.replace("WHERE s->>'code' IS NOT NULL;", "WHERE s->>'code' IS NOT NULL AND (s->>'source_user_id')::bigint<>"+str(excluded)+";")
        assert omitted!=body
        code,_,label=reg.run_sql(omitted,expect_ok=False)
        assert code!=0 and 'affiliate_alias_import_not_complete' in (reg.output/(label+'.stderr.private.txt')).read_text()
        assert reg.query(T.state_sql(inventory))==before
        result['checks'].append('deferred_incomplete_batch_rejected_with_zero_rows_and_zero_sequence_changes')
        # A real unrelated mutation makes the reviewed state CAS fail before inserts.
        reg.query("INSERT INTO core_console.options(key,value) VALUES('alias.fixture.cas','1'); SELECT true")
        code,_,label=reg.run_sql(body,expect_ok=False)
        assert code!=0 and 'affiliate_alias_before_state_changed' in (reg.output/(label+'.stderr.private.txt')).read_text()
        reg.query("DELETE FROM core_console.options WHERE key='alias.fixture.cas'; SELECT true")
        assert reg.query(T.state_sql(inventory))==before
        result['checks'].append('actual_existing_state_CAS_refusal_before_alias_insert')
        for precommit in (True,False):
            app='lmm-affiliate-alias-'+uuid.uuid4().hex
            conn=deepcopy(reg.connection);proxy=work/('proxy-'+uuid.uuid4().hex[:6])
            with K.CommitDropProxy(sock/('.s.PGSQL.'+port),proxy,int(port),before_commit=precommit):
                conn['socket_directory']=str(proxy)
                code,_,_=reg.run_sql(body,connection=conn,app=app,expect_ok=False)
                assert code!=0
            for _ in range(50):
                active=reg.query('SELECT EXISTS(SELECT FROM pg_stat_activity WHERE application_name='+T.literal(app)+')')
                if not active:break
                time.sleep(.1)
            assert not active
            resolved_result=reg.query(A.resolve_sql(prepared,review,app))
            if precommit:
                assert resolved_result['status']=='UNKNOWN'
                assert reg.query(T.state_sql(inventory))==before
                result['checks'].append('real_precommit_disconnect_rolls_back_all_aliases_without_sequence_use')
            else:
                assert resolved_result['status']=='COMMITTED' and resolved_result['automatic_retry_allowed'] is False
                result['checks'].append('real_commit_ack_loss_resolves_receipt_and_all_alias_rows_readonly')
        after=reg.query(T.state_sql(inventory))
        assert after['sequences']==before['sequences']
        assert {k:v for k,v in after['tables'].items() if k not in A.TABLES}=={
            k:v for k,v in before['tables'].items() if k not in A.TABLES}
        assert reg.query(body)['status']=='COMMITTED' and reg.query(T.state_sql(inventory))==after
        result['checks'].append('exact_replay_and_all_unrelated_rows_sessions_wallet_sequences_preserved')
        # Case-sensitive SQL matches the actual runtime query and aliases keep disabled owners.
        aliases=reg.query("SELECT jsonb_agg(to_jsonb(a) ORDER BY source_user_id) FROM core_identity.legacy_affiliate_aliases a")
        assert len(aliases)==projection['alias_count']
        inactive_ids={r['target_user_id'] for r in prepared['snapshot']['identity_authority'] if not r['active']}
        assert inactive_ids and any(row['user_id'] in inactive_ids for row in aliases)
        assert {row['code'] for row in aliases}>={'Ab12','ab12'}
        for code in ('Ab12','ab12'):
            found=reg.query('SELECT user_id FROM core_identity.legacy_affiliate_aliases WHERE code='+T.literal(code)+' COLLATE "C"')
            expected=next(row['user_id'] for row in aliases if row['code']==code);assert found==expected
        result['checks'].append('case_distinct_codes_preserved_and_disabled_source_owners_not_filtered')
        # Actual role can read exactly two columns. No source/receipt/write rights.
        rights=json.loads(sql("SET ROLE "+runtime+"; SELECT jsonb_build_object('code',has_column_privilege('core_identity.legacy_affiliate_aliases','code','SELECT'),"
          "'user',has_column_privilege('core_identity.legacy_affiliate_aliases','user_id','SELECT'),"
          "'source',has_column_privilege('core_identity.legacy_affiliate_aliases','source_user_id','SELECT'),"
          "'receipt',has_table_privilege((SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace"
          " WHERE n.nspname='core_meta' AND c.relname='legacy_affiliate_alias_imports'),'SELECT'),"
          "'write',has_table_privilege('core_identity.legacy_affiliate_aliases','INSERT,UPDATE,DELETE,TRUNCATE'),"
          "'function',has_function_privilege('core_identity.guard_affiliate_registration_inviter()','EXECUTE'));",admin=True))
        assert rights=={'code':True,'user':True,'source':False,'receipt':False,'write':False,'function':False}
        result['checks'].append('runtime_two_column_read_only_and_private_owner_receipt_denied')
        # Registration changes only attribution and checks the current inviter.
        live=next(row for row in aliases if reg.query("SELECT active AND platform_role='user' FROM core_identity.users WHERE id="+str(row['user_id'])))
        disabled=next(row for row in aliases if not reg.query('SELECT active FROM core_identity.users WHERE id='+str(row['user_id'])))
        invitees=[]
        for _ in range(5):
            invitees.append(reg.query("WITH a AS (INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id)"
              " INSERT INTO core_identity.users(personal_account_id) SELECT id FROM a RETURNING id"))
        money_before=reg.query("SELECT jsonb_build_object('journals',(SELECT count(*) FROM core_billing.ledger_journals),"
            "'entries',(SELECT count(*) FROM core_billing.ledger_entries))")
        reg.query('INSERT INTO core_identity.affiliate_registrations(user_id,inviter_id) VALUES('+
                  str(invitees[0])+','+str(live['user_id'])+'); SELECT true')
        code,_,label=reg.run_sql('INSERT INTO core_identity.affiliate_registrations(user_id,inviter_id) VALUES('+
                               str(invitees[1])+','+str(disabled['user_id'])+');',expect_ok=False)
        assert code!=0 and 'affiliate_registration_inviter_unavailable' in (reg.output/(label+'.stderr.private.txt')).read_text()
        code,_,label=reg.run_sql('INSERT INTO core_identity.affiliate_registrations(user_id,inviter_id) VALUES('+
                               str(invitees[1])+','+str(invitees[2])+');',expect_ok=False)
        assert code!=0 and 'affiliate_registration_inviter_unavailable' in (reg.output/(label+'.stderr.private.txt')).read_text()
        reg.query("INSERT INTO core_identity.affiliate_codes(user_id,code) VALUES("+str(invitees[2])+",repeat('f',32));"
              ' INSERT INTO core_identity.affiliate_registrations(user_id,inviter_id) VALUES('+
              str(invitees[1])+','+str(invitees[2])+'); SELECT true')
        alias_rows_before=reg.query("SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY source_user_id),'[]'::jsonb) FROM core_identity.legacy_affiliate_aliases a")
        native_codes_before=reg.query("SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY user_id),'[]'::jsonb) FROM core_identity.affiliate_codes c")
        for user,invitee in [(live['user_id'],invitees[3]),(invitees[2],invitees[4])]:
            account=reg.query('SELECT personal_account_id FROM core_identity.users WHERE id='+str(user))
            reg.query('UPDATE core_identity.accounts SET active=false WHERE id='+str(account)+'; SELECT true')
            assert reg.query('SELECT active FROM core_identity.users WHERE id='+str(user)) is True
            registrations=reg.query('SELECT count(*) FROM core_identity.affiliate_registrations')
            code,_,label=reg.run_sql('INSERT INTO core_identity.affiliate_registrations(user_id,inviter_id) VALUES('+
                                     str(invitee)+','+str(user)+');',expect_ok=False)
            assert code!=0 and 'affiliate_registration_inviter_unavailable' in (reg.output/(label+'.stderr.private.txt')).read_text()
            assert reg.query('SELECT count(*) FROM core_identity.affiliate_registrations')==registrations
            reg.query('UPDATE core_identity.accounts SET active=true WHERE id='+str(account)+'; SELECT true')
        assert reg.query("SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY source_user_id),'[]'::jsonb) FROM core_identity.legacy_affiliate_aliases a")==alias_rows_before
        assert reg.query("SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY user_id),'[]'::jsonb) FROM core_identity.affiliate_codes c")==native_codes_before
        result['checks'].append('active_users_with_disabled_personal_accounts_reject_alias_and_native32_without_removing_codes')
        money_after=reg.query("SELECT jsonb_build_object('journals',(SELECT count(*) FROM core_billing.ledger_journals),"
            "'entries',(SELECT count(*) FROM core_billing.ledger_entries))")
        assert money_before==money_after
        result['checks'].append('current_active_alias_and_native32_registration_work_disabled_and_codeless_reject_no_rewards')
        result.update(status='PASS',component_count=86,synthetic_catalog_sha256=cat86,
                      alias_count=len(aliases),source_users=len(users),public_alias_codes_in_output=False)
    except Exception as error:
        (work/'failure.traceback.private.txt').write_text(traceback.format_exc())
        result.update(status='FAIL',error_type=type(error).__name__,reason=str(error))
    finally:
        if started:
            try:
                cp=subprocess.run(['pg_ctl','-D',str(data),'-m','fast','-t','180','-w','stop'],env=env,text=True,capture_output=True,timeout=190)
                (work/'stop.stdout').write_text(cp.stdout);(work/'stop.stderr').write_text(cp.stderr)
                result['cleanup']={'pg_stop_exit':cp.returncode,'pid_absent':not(data/'postmaster.pid').exists(),
                    'socket_absent':not(sock/('.s.PGSQL.'+port)).exists()}
            except Exception as error:
                result['cleanup']={'pg_stop_exit':None,'error_type':type(error).__name__}
            if result['cleanup'].get('pg_stop_exit')!=0 or not result['cleanup'].get('pid_absent') or not result['cleanup'].get('socket_absent'):
                result['status']='FAIL'
        raw=(json.dumps(result,sort_keys=True,indent=2)+'\n').encode();(work/'result.public.json').write_bytes(raw)
        print(json.dumps({**result,'result_sha256':sha(raw)},sort_keys=True))
    return 0 if result['status']=='PASS' else 1


if __name__=='__main__':
    raise SystemExit(main())
