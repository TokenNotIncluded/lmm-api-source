#!/usr/bin/env python3
"""Synthetic c343 -> exact87 -> component88 weekly history regression.

Creates a new local socket-only PostgreSQL instance. Never connects to a live
source or target. The frozen c343 identity transaction must actually commit.
Schema81 coupons are explicit synthetic prior-state fixtures; this does not
qualify their separate production importer. Suffix SQL installation is separate
from the compiled 87->88 owner upgrade tests. All application rows are invented.
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
import current_weekly_discounts as W
SHA87='9df112cfc22c9f4493f3903189fbb084896d59e3c36a38981132c4651b6aaca1'
CAT87='f8debeb9f33cd3cca6d303d554bf8e234183e8e1f8faf9d0e0266007a9eb9882'


def sha(raw):return hashlib.sha256(raw).hexdigest()


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--capture87',type=Path,required=True)
    parser.add_argument('--support',type=Path,required=True)
    args=parser.parse_args(argv)
    SOURCE=args.source.resolve();CAPTURE=args.capture87.resolve();SUPPORT=args.support.resolve()
    sys.path.insert(1,str(SUPPORT))
    import check_final81_gift_pg as B
    import current_key_lineage as L
    import final81_target_guard as T
    import test_current_identity_pg as IP
    import test_current_key_production as CT
    import test_migrate_current_runtime_keys_pg as K
    W.frozen_helpers()
    os.umask(0o077)
    nonce=secrets.token_hex(32);short=nonce[:10]
    work=Path.home()/'.local/state/lmm-api'/('w88-'+short);work.mkdir(mode=0o700)
    sock=work/'socket';sock.mkdir(mode=0o700);data=work/'pgdata';port=str(54000+secrets.randbelow(9000))
    db='lmm_runtime_keys_test_weekly_'+short
    owner,installer,runtime=('weekly88_'+n+'_'+short for n in ('owner','install','runtime'))
    env={k:os.environ[k] for k in ('PATH','HOME','LANG') if k in os.environ}
    env.update(PGCONNECT_TIMEOUT='5',PGCLIENTENCODING='UTF8')
    result={'status':'INCOMPLETE','scope':'COMPONENT88_SQL_AND_REAL_C343_WEEKLY_OWNER_MECHANICS',
            'work_directory':str(work),'nonce':nonce,'production_access':False,
            'compiled_binary_observation':False,'final_target_bound':False,
            'predecessor_scope':'FROZEN_EXACT87_SQL_CONTRACT','checks':[]}
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
        raw87=(CAPTURE/'manifest87.json').read_bytes();assert sha(raw87)==SHA87
        m87=json.loads(raw87);assert len(m87['components'])==87
        resolved=B.source_sql_bytes(SOURCE,m87['components'])
        assert len(resolved)==87
        source_bindings=json.dumps([{'name':component['name'],'sha256':sha(raw)}
            for component,_,raw in resolved],sort_keys=True,separators=(',',':')).encode()
        save('source-bindings.json',[{'name':c['name'],'path':str(p),'sha256':sha(raw)} for c,p,raw in resolved])
        m78=deepcopy(m87);m78['components']=m78['components'][:78]
        raw78=(json.dumps(m78,separators=(',',':'))+'\n').encode()
        c78=L.U.core78_contract(raw78);(work/'manifest78.json').write_bytes(raw78)
        schema_source=(SOURCE/'services/core/src/core_schema.rs').read_text()
        metadata=[json.loads(re.search(r'const CONTRACT: &str = (".*?");',schema_source).group(1))]
        for name in ('UPGRADE_METADATA','ADDITIVE_UPGRADE_METADATA','LEDGER_REVISION_METADATA',
                     'CONSOLE_REVISION_METADATA','OFFLINE_REVISION_METADATA'):
            rel=re.search('const '+name+r': &str = include_str!\("([^"]+)"\);',schema_source).group(1)
            metadata.append((SOURCE/'services/core/src'/rel).resolve().read_text())
        assert sha(''.join(metadata).encode())==m87['installer_contract_sql_sha256']
        grant78=run('read-grant78',['git','-C',str(SOURCE),'show',
              'fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5:deployment/postgres/grant-core-runtime.sql'])
        (work/'grant78.sql').write_text(grant78)
        verify78=run('read-grant78-owner-check',['git','-C',str(SOURCE),'show',
              'fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5:deployment/postgres/verify-channel-observations-owner.sql'])
        (work/'verify-channel-observations-owner.sql').write_text(verify78)
        result['inputs']={'driver_sha256':sha(Path(__file__).read_bytes()),
          'module_sha256':sha(Path(W.__file__).read_bytes()),'ddl88_sha256':sha(W.schema_path().read_bytes()),
          'manifest87_sha256':sha(raw87),'grant78_sha256':sha(grant78.encode()),
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
        # Complete the invented canonical users BEFORE the real identity plan/commit.
        # Match the already-validated c343 routing/history fixture semantics.
        for row in reg.bundle['source']['data']['users']:
            if row['id']==108:row['trust_level_override']=1
            if row['id']==102:row['status']=2  # Historical disabled owner must not be filtered.
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
        reg.bundle['canonical_source']['source_user_row_digests']=[
           {'source_user_id':row['id'],'source_row_sha256':L.F.PLANNER.digest(row)} for row in users]
        identity_id='synthetic-weekly-identity-'+short
        im=reg.prepare_identity(identity_id)
        committed=reg.query(L.ID.build_sql(im,fixture_nonce=nonce))
        assert committed['status']=='COMMITTED'
        identity_run=reg.query('SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE migration_id='+T.literal(identity_id))
        mapping=[{k:row[k] for k in L.MAP_FIELDS} for row in identity_run['manifest']['mapping']]
        result['checks'].append('actual_exact78_c343_identity_committed_with_complete_canonical_users')
        # Prefix SQL is exact setup, not a substitute for testing the compiled
        # 87->88 owner upgrade. c343 above is the real frozen owner transaction.
        for component,path,raw in resolved[78:]:
            sql("BEGIN;\n"+raw.decode()+"\nINSERT INTO core_meta.component_contract(name,fingerprint) VALUES("
                +T.literal(component['name'])+",decode('"+component['raw_schema_sha256']+"','hex')); COMMIT;")
        assert catalog(m87['catalog']['query_utf8'])==CAT87
        result['checks'].append('actual_exact87_predecessor_catalog_matches_reviewed_capture')
        ddl=W.schema_path().read_bytes()
        old_inventory=reg.query(T.F.inventory_sql());old_state=reg.query(T.state_sql(old_inventory))
        sql('BEGIN;\n'+ddl.decode()+"\nINSERT INTO core_meta.component_contract(name,fingerprint) VALUES("
            +T.literal(W.COMPONENT_NAME)+",decode('"+sha(ddl)+"','hex')); COMMIT;")
        installed=reg.query(T.state_sql(old_inventory))
        assert old_state['sequences']==installed['sequences']
        assert {k:v for k,v in old_state['tables'].items() if k!='core_meta.component_contract'}=={
            k:v for k,v in installed['tables'].items() if k!='core_meta.component_contract'}
        result['checks'].append('component88_SQL_install_preserves_all_existing_rows_and_sequences')
        cat88=catalog(m87['catalog']['query_utf8'])
        sql("UPDATE core_meta.runtime_contract SET catalog_fingerprint=decode('"+cat88+"','hex') WHERE singleton")
        m88=deepcopy(m87);m88['components'].append({'name':W.COMPONENT_NAME,'raw_schema_sha256':sha(ddl)})
        raw88=(json.dumps(m88,separators=(',',':'))+'\n').encode()
        (work/'manifest88.synthetic.json').write_bytes(raw88)
        binding={'component_count':88,'manifest_sha256':sha(raw88),'catalog_sha256':cat88}
        save('target88.synthetic.json',binding)
        contract=W.contract_from_manifest(raw88,binding,synthetic=True)
        result['final_target_bound']=False
        refs={'source_archive_sha256':im['source_archive_sha256'],
              'source_export_sha256':im['source_canonical_export_sha256'],
              'source_export_proof_sha256':im['source_canonical_export_proof_sha256']}
        # Invented economic rows mirror the 2 declined/1 offered/4 claimed shape.
        # Existing original coupons are explicit schema81 fixture setup. This is
        # not a test or receipt of the separate original coupon owner importer.
        timestamp=reg.query("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        first_week=345600+((timestamp-345600)//604800-8)*604800
        weekly=[];coupons=[]
        for index,(status,percent) in enumerate([('declined',0),('declined',0),('offered',5),
                                               ('claimed',5),('claimed',5),('claimed',8),('claimed',10)]):
            source_user=users[index%len(users)]['id'];week=first_week+index*604800
            code_id=880000+index if status=='claimed' else 0
            row={'id':index+1,'user_id':source_user,'conversation_id':88000+index,
                 'week_start':week,'discount_percent':percent,'status':status,
                 'reason':'Synthetic original weekly economic decision',
                 'created_at':week+100,'claimed_at':week+200 if status=='claimed' else 0,'code_id':code_id}
            weekly.append(row)
            if code_id:
                coupons.append({'id':code_id,'code':f'HIST-WEEKLY-{index:02d}','owner_user_id':source_user,
                    'discount_percent':percent,'max_uses':1,'min_amount':0,'expired_time':week+604800,
                    'enabled':True,'used_count':0,'created_at':week+200})
        projection=W.project(users,weekly,coupons,refs,synthetic=True)
        coupon_fixture='synthetic-prior-weekly-coupons-'+short
        reg.query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES("
            +T.literal(coupon_fixture)+","+T.literal(refs['source_archive_sha256'])+","+T.literal(sha(b'synthetic coupon fixture'))+
            ",'{\"scope\":\"synthetic-prior-coupon-owner-fixture\",\"not_a_coupon_importer_receipt\":true}'); SELECT true")
        mapped={r['source_user_id']:r for r in mapping};source_codes={r['id']:r for r in coupons}
        for row in projection['rows']:
            if row['source_code_id'] is None:continue
            source=source_codes[row['source_code_id']];target=mapped[row['source_user_id']]['target_user_id']
            reg.query("INSERT INTO core_payments.discount_codes(id,code,name,discount_percent,min_credit_units,max_uses,"
                "enabled,starts_at,ends_at,created_by,created_at) OVERRIDING SYSTEM VALUE VALUES("
                +",".join(T.literal(v) for v in [source['id'],source['code'],'Synthetic prior original coupon',
                    source['discount_percent'],0,1,True,row['claimed_at'],source['expired_time'],target])+
                ",to_timestamp("+str(row['claimed_at'])+"));"
                "INSERT INTO core_payments.legacy_discount_bases(code_id,import_migration_id,source_namespace_sha256,"
                "source_archive_sha256,source_discount_id,source_row_sha256,original_owner_user_id,original_used_count,"
                "original_min_amount,min_amount_unit) VALUES("
                +str(source['id'])+","+T.literal(coupon_fixture)+",decode("+T.literal(im['source_namespace_sha256'])+",'hex'),"
                "decode("+T.literal(refs['source_archive_sha256'])+",'hex'),"+str(source['id'])+
                ",decode("+T.literal(row['source_coupon_row_sha256'])+",'hex'),"+str(target)+",0,0,'legacy_batch_500000'); SELECT true")
        result['checks'].append('four_preexisting_original_private_coupons_seeded_as_explicit_schema81_fixture')
        # Only new browser capability is enabled, with the old role remaining offline.
        grant=W.schema_path().parent/'assistant_weekly_discount_browser_grants.sql'
        run('weekly-browser-grants',['psql','-XqAt','-v','ON_ERROR_STOP=1','-h',str(sock),'-p',port,
            '-U',installer,'-d',db,'-v','runtime_role='+runtime,'-f',str(grant)],
            override={**env,'PGOPTIONS':'-c role='+owner})
        inventory=reg.query(T.F.inventory_sql())
        def observe():
            snapshot=reg.query(W.observation_sql(mapping,identity_id,projection,inventory=inventory,
                contract=contract,runtime_roles=[runtime],actor=101))
            snapshot['fixture_nonce']=nonce
            return snapshot
        prepared=W.prepare_owner(projection,users,observe(),contract,
            migration_id='synthetic-weekly-import-'+short,actor=101,synthetic=True,
            synthetic_source={'weekly':weekly,'coupons':coupons})
        review={'scope':'synthetic-weekly-test-only','material_sha256':W.digest(prepared['material']),
                'production_authorization':False}
        body=W.transaction_sql(prepared,review)
        save('prepared.private.json',prepared);save('review.synthetic.json',review)
        save('source.synthetic.json',{'users':users,'weekly':weekly,'coupons':coupons})
        before=reg.query(T.state_sql(inventory))

        # Competing owner holds the same real migration fence before insertion.
        lock_env={**env,**L.U.I.socket_connection(reg.connection)}
        lock_env.update(PGAPPNAME='weekly-fence-'+short,PGOPTIONS=L.U.I.psql_options(reg.connection,readonly=False))
        holder=subprocess.Popen(['psql','-X','--no-password','-qAt','-v','ON_ERROR_STOP=1'],
            stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,text=True,env=lock_env)
        try:
            holder.stdin.write('SELECT pg_advisory_lock('+str(L.U.FENCE)+'); SELECT pg_sleep(20);\n')
            holder.stdin.close();ready=False
            for _ in range(50):
                ready=reg.query('SELECT NOT pg_try_advisory_xact_lock('+str(L.U.FENCE)+')')
                if ready:break
                time.sleep(.1)
            assert ready
            code,_,label=reg.run_sql(body,expect_ok=False)
            assert code!=0 and 'weekly_import_busy' in (reg.output/(label+'.stderr.private.txt')).read_text()
        finally:
            if holder.poll() is None:
                holder.send_signal(signal.SIGINT)
                try:holder.wait(timeout=5)
                except subprocess.TimeoutExpired:holder.kill();holder.wait(timeout=5)
        assert reg.query(T.state_sql(inventory))==before
        result['checks'].append('competing_offline_owner_fence_refuses_without_rows_or_sequence_changes')

        # A genuine missing source basis row is rejected at the deferred batch boundary.
        needle="FROM jsonb_array_elements("+L.U.literal(W.imported_rows(prepared['material']))+"::jsonb) r;"
        pos=body.rfind(needle);assert pos>=0
        omit=needle.replace(" r;"," r WHERE (r->>'source_weekly_id')::bigint<>1;")
        omitted=body[:pos]+omit+body[pos+len(needle):]
        code,_,label=reg.run_sql(omitted,expect_ok=False)
        assert code!=0 and any(message in (reg.output/(label+'.stderr.private.txt')).read_text()
            for message in ('weekly source rows or owner mapping incomplete','legacy weekly source or original coupon binding missing'))
        assert reg.query(T.state_sql(inventory))==before
        result['checks'].append('deferred_missing_source_basis_refuses_complete_batch_with_no_partial_rows')

        reg.query("INSERT INTO core_console.options(key,value) VALUES('weekly.fixture.cas','1'); SELECT true")
        code,_,label=reg.run_sql(body,expect_ok=False)
        assert code!=0 and 'weekly_before_state_changed' in (reg.output/(label+'.stderr.private.txt')).read_text()
        reg.query("DELETE FROM core_console.options WHERE key='weekly.fixture.cas'; SELECT true")
        assert reg.query(T.state_sql(inventory))==before
        result['checks'].append('unrelated_current_state_change_refuses_saved_review_before_history_insert')

        for precommit in (True,False):
            app='lmm-weekly-'+uuid.uuid4().hex
            conn=deepcopy(reg.connection);proxy=work/('p'+uuid.uuid4().hex[:6])
            with K.CommitDropProxy(sock/('.s.PGSQL.'+port),proxy,int(port),before_commit=precommit):
                conn['socket_directory']=str(proxy)
                code,_,_=reg.run_sql(body,connection=conn,app=app,expect_ok=False)
                assert code!=0
            for _ in range(50):
                active=reg.query('SELECT EXISTS(SELECT FROM pg_stat_activity WHERE application_name='+T.literal(app)+')')
                if not active:break
                time.sleep(.1)
            assert not active
            resolved_result=reg.query(W.resolve_sql(prepared,review,app))
            assert resolved_result['instance_matches'] and resolved_result['original_execution_connections']==0
            assert resolved_result['automatic_retry_allowed'] is False
            if precommit:
                assert resolved_result['exact_committed'] is False
                assert reg.query(T.state_sql(inventory))==before
                result['checks'].append('actual_precommit_disconnect_rolls_back_all_decisions_and_source_bases')
            else:
                assert resolved_result['exact_committed'] is True
                result['checks'].append('actual_commit_ack_loss_resolves_full_receipt_and_all_rows_readonly')
        after=reg.query(T.state_sql(inventory))
        assert before['sequences']==after['sequences']
        assert {k:v for k,v in before['tables'].items() if k not in W.TABLES}=={
            k:v for k,v in after['tables'].items() if k not in W.TABLES}
        assert reg.query(body)['status']=='COMMITTED'
        assert reg.query(T.state_sql(inventory))==after
        result['checks'].append('exact_replay_preserves_existing_coupons_wallet_sessions_and_all_sequences')
        rows=reg.query("SELECT jsonb_agg(to_jsonb(w) ORDER BY id) FROM core_weekly.weekly_discounts w")
        assert len(rows)==7
        assert sorted(r['status'] for r in rows)==['claimed']*4+['declined']*2+['offered']
        assert sorted(r['discount_percent'] for r in rows)==[0,0,5,5,5,8,10]
        assert all(r['origin']=='legacy' and r['operation_id'] is None and r['conversation_id'] is None for r in rows)
        assert reg.query("SELECT enabled FROM core_weekly.weekly_discount_policy WHERE singleton") is False
        read_count=json.loads(sql('SET ROLE '+runtime+'; SELECT count(*) FROM core_weekly.weekly_discounts;',admin=True))
        assert read_count==7
        result['checks'].append('all_seven_original_decisions_remain_readable_while_new_policy_is_disabled')
        rights=json.loads(sql("SET ROLE "+runtime+"; SELECT jsonb_build_object("
            "'basis',has_table_privilege((SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace"
            " WHERE n.nspname='core_weekly' AND c.relname='legacy_weekly_discount_bases'),'SELECT'),"
            "'receipt',has_table_privilege((SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace"
            " WHERE n.nspname='core_meta' AND c.relname='legacy_weekly_discount_imports'),'SELECT'),"
            "'write',has_table_privilege('core_weekly.weekly_discounts','INSERT,DELETE,TRUNCATE'),"
            "'claim',has_function_privilege('core_weekly.claim_weekly_discount_code(text,text)','EXECUTE'),"
            "'policy_write',has_table_privilege('core_weekly.weekly_discount_policy','INSERT,UPDATE,DELETE,TRUNCATE'));",admin=True))
        assert rights=={'basis':False,'receipt':False,'write':False,'claim':True,'policy_write':False}
        result['checks'].append('actual_runtime_history_read_preserves_private_source_and_owner_write_boundaries')
        declined=next(r for r in rows if r['status']=='declined')
        for statement in (
            "UPDATE core_weekly.weekly_discounts SET reason='Changed history' WHERE id="+T.literal(declined['id']),
            "DELETE FROM core_weekly.legacy_weekly_discount_bases WHERE decision_id="+T.literal(declined['id']),
            "UPDATE core_meta.legacy_weekly_discount_imports SET review_utf8='{}' WHERE singleton"):
            code,_,_=reg.run_sql(statement,expect_ok=False);assert code!=0
        assert reg.query(T.state_sql(inventory))==after
        result['checks'].append('original_decisions_source_bases_and_review_receipt_are_immutable')
        offered=next(r for r in rows if r['status']=='offered')
        # Even explicit owner policy activation cannot resurrect a past-week offer.
        sql('UPDATE core_weekly.weekly_discount_policy SET enabled=TRUE;')
        code,_,label=reg.run_sql("SELECT * FROM core_weekly.claim_weekly_discount_code("+
            T.literal(offered['id'])+",'AIWEEK-"+'A'*32+"');",expect_ok=False)
        assert code!=0 and 'weekly claim is not currently eligible' in (reg.output/(label+'.stderr.private.txt')).read_text()
        sql('UPDATE core_weekly.weekly_discount_policy SET enabled=FALSE;')
        assert reg.query(T.state_sql(inventory))==after
        result['checks'].append('past_week_offer_never_mints_a_new_coupon_after_activation')
        result.update(status='PASS',component_count=88,synthetic_catalog_sha256=cat88,
            source_decisions=7,original_coupon_references=4,new_coupons_created=0,ledger_mutations=0,
            coupon_setup_scope='EXPLICIT_SCHEMA81_SYNTHETIC_PRIOR_COUPONS_NOT_IMPORTER_RECEIPT')
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
