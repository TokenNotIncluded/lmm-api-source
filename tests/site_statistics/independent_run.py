#!/usr/bin/env python3
"""Prepared actual Native finance acceptance. No execution without explicit pins.

Reuses only our previously accepted PG/process mechanics and identity seeder.
The owner importer is the system under test; independent_oracle supplies answers.
"""
import argparse
import copy
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import time
import traceback
from urllib.parse import urlencode

import independent_oracle as oracle

REPO = Path(__file__).resolve().parents[2]
MECHANICS_SHA = '252179be7165a5729d6d7101f78af4196f34b616316d58235dc8aa74632d686d'
SEED_SHA = 'f3f3d49dfe9f9b4fc25ebd8004193cc1487d103d0aadf82fd4d4b1246510a5cc'
OLD_CORE = 'd6d356311de8cf6589c2ec4429603a0d9308ca634b1aed20b78a6c42b7c76c10'
ENDPOINT = '/api/finance/site-statistics'
GROUPS = ['fresh28-native-binding', 'opening-without-finance-is-unknown',
          'owner-import-and-independent-readback', 'dto-permissions-and-read-only',
          'runtime-finance-acl', 'native-usd-and-restart', 'source-count-rejection',
          'unresolved-credited-refund', 'refund-id-amount-mismatch', 'refund-counter-mismatch',
          'precredit-credit-rejection', 'precredit-refund-rejection',
          'epoch-at-plus-one-rejection', 'epoch-plus-two-accepted',
          'native-usage-and-refund', 'second-source-rejected', 'beyond-i64-pg-http']
SEAL_GROUPS = ['preexisting-intent-seal-and-late-credit',
               'post-seal-native-and-noop-floor', 'seal-sequence-premises',
               'historical-request-pending-release-settle', 'native-only-preseal-request']
SOURCE_COLUMNS = ('source_key','contract','source_archive_sha256','mapping_sha256',
                  'tables_sha256','normalization_sha256','rows_sha256','observed_at',
                  'user_count','topup_count','credits_per_usd','complete')
IMPORT_FILES = ['legacy_site_statistics.py','financial_import.py','migrate-legacy-financial.py',
                'migrate-legacy-snapshot.py','legacy_financial_preflight.py',
                'legacy_escrow_mapping.py','legacy_bounty_basis.py']


def require(ok, message):
    if not ok:
        raise AssertionError(message)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def dump(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


def literal(value):
    text = value if isinstance(value, str) else json.dumps(value, separators=(',', ':'))
    return "'" + text.replace("'", "''") + "'"


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    sys.modules[name] = loaded
    spec.loader.exec_module(loaded)
    return loaded


def worker(root):
    binding = json.loads((root/'binding.json').read_text())
    for relative, digest in binding['frozen_files'].items():
        require(sha(root/relative) == digest, 'frozen input changed: '+relative)
    candidate = Path(binding['candidate'])
    require(sha(candidate/'lmm-core') == binding['core_sha256'], 'candidate Core drift')
    require(sha(candidate/'liblmm_core.rlib') == binding['rlib_sha256'], 'candidate library drift')
    # Legacy machinery imports no author finance or performance oracle here.
    mechanics = module('site_independent_mechanics', root/'runner/pg_mechanics.py')
    mechanics.FREEZE, mechanics.CORE_SHA = candidate, binding['core_sha256']
    sys.path.insert(0, str(root/'importer'))
    writer = module('site_owner_import_under_test', root/'importer/legacy_site_statistics.py')

    class Run(mechanics.Run):
        def __init__(self):
            super().__init__(root)
            self.results = {name: 'NOT_RUN' for name in GROUPS}
            if binding.get('seal_regression'):
                self.results.update({name:'NOT_RUN' for name in SEAL_GROUPS})
                for name in ('epoch-at-plus-one-rejection','epoch-plus-two-accepted'):
                    self.results[name]='NOT_RUN_SUPERSEDED_CONTRACT'
                    if binding.get('full_current'):self.results.pop(name)
            if binding.get('reuse_from'):
                for name in GROUPS[:14]:self.results[name]='REUSED_PASS'
            if binding.get('resume_seals'):
                for name in binding['resume_seals']['groups']:
                    self.results[name]='PRIOR_PASS_SAME_CORE_OLD_WRITER'
            self.active_group = None
            self.cache_client = None
            self.request_workers = {}
            self.observed = 1790000000
            self.fixture = oracle.fixture()
            self.baseline = oracle.expected_statistics()

        def group(self, name, action):
            self.active_group = name
            self.results[name] = 'RUNNING'
            dump(root/'results.json', self.results)
            action()
            self.results[name] = 'PASS'
            dump(root/'results.json', self.results)

        def owner(self, query):
            return self.sql('SET ROLE lmm_core_owner;\n'+query)

        def run(self,label,command,input=None,timeout=90,check=True):
            result=super().run(label,command,input,timeout,check)
            if label == 'link-fixture':
                dump(root/'identity-helper.json',{'sha256':sha(root/'fixture'),'linked_core_library_sha256':binding['rlib_sha256']})
            return result

        def prepare(self):
            config=json.loads((candidate/'compile-config.json').read_text())
            self.run('actual-helper-compiler-version',[config['compiler'],'--version'])
            super().prepare()
            if not binding.get('seal_regression') or binding.get('full_current'):
                self.run('link-refund-helper',[config['compiler'],'--edition=2024',
                    root/'runner/independent_refund.rs','--extern','lmm_core='+str(candidate/'liblmm_core.rlib'),
                    *config['externs'],*config['search'],*config['link'],'-o',root/'refund-helper'],timeout=180)
                dump(root/'refund-helper.json',{'sha256':sha(root/'refund-helper'),'core_rlib_sha256':binding['rlib_sha256']})
            if binding.get('seal_regression'):
                self.run('link-request-helper',[config['compiler'],'--edition=2024',
                    root/'runner/independent_requests.rs','--extern','lmm_core='+str(candidate/'liblmm_core.rlib'),
                    *config['externs'],*config['search'],*config['link'],'-o',root/'request-helper'],timeout=180)
                dump(root/'request-helper.json',{'sha256':sha(root/'request-helper'),'core_rlib_sha256':binding['rlib_sha256']})
            self.stop_core()
            command=[*self.base];command[-1]='postgres'
            self.run('save-clean-seed',command,input=b'CREATE DATABASE seed_template WITH TEMPLATE perf;')

        def start_core(self):
            super().start_core()
            dump(root/f'core-{self.generation}-database.json',{
                'database':self.base[-1], 'runtime_url_sha256':sha(root/'runtime.url'),
                'core_config_sha256':sha(root/'core.toml'),
                'current_database':self.sql('SELECT current_database()')})

        def owner_connection(self, query, label):
            cmd = [*self.base]
            cmd[cmd.index('-U')+1] = 'independent_installer'
            return self.run(label, cmd, input=('SET ROLE lmm_core_owner;\n'+query).encode()).stdout.decode().strip()

        def protected(self, label):
            relations = json.loads(self.sql("SELECT jsonb_agg(jsonb_build_array(n.nspname,c.relname,c.relkind) ORDER BY n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE (n.nspname LIKE 'core\\_%' ESCAPE '\\' OR n.nspname='public') AND c.relkind IN ('r','p','S')"))
            sql = ['BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;']
            for schema, table, kind in relations:
                require(re.fullmatch('[a-zA-Z0-9_]+', schema+table), 'unsafe fixture relation')
                name = schema+'.'+table
                relation = '"'+schema+'"."'+table+'"'
                rows = "jsonb_build_array(jsonb_build_object('last_value',last_value,'is_called',is_called,'log_cnt',log_cnt))" if kind == 'S' else "coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)"
                sql.append(f"SELECT jsonb_build_object('relation',{literal(name)},'rows',{rows}) FROM {relation} t;")
            sql.append('COMMIT;')
            records = [json.loads(line) for line in self.sql('\n'.join(sql)).splitlines()]
            result = {r['relation']: hashlib.sha256(json.dumps(r['rows'], sort_keys=True).encode()).hexdigest() for r in records}
            require(len(result) == len(relations), 'incomplete protected inventory')
            dump(root/(self.base[-1]+'-'+label+'.json'), result)
            return result

        def site(self, label, status=200, expected=None, token='admin'):
            before = self.protected(label+'-before')
            started = int(time.time())
            secret = None if token is None else self.creds['keys'][0]['secret'] if token == 'key' else self.creds['sessions'][token]['secret']
            _, body, _ = self.http(ENDPOINT, secret, label=label, expect=status)
            ended = int(time.time())
            if status == 200:
                oracle.assert_response(body, expected or self.baseline, started, ended)
            else:
                require(body.get('success') is False and 'data' not in body, 'failed read exposed a trusted total')
                if status == 503:
                    require(body.get('code') == 'SITE_STATISTICS_SOURCE_UNAVAILABLE', 'wrong UNKNOWN contract')
            after = self.protected(label+'-after')
            require(before == after, 'read mutated protected tables or sequences')

        def prepare_opening(self):
            self.stop_core()
            components = json.loads(self.sql('SELECT jsonb_agg(name ORDER BY name) FROM core_meta.component_contract'))
            require(len(components) == 28 and 'site-finance' in components, 'not the finance 28-component candidate')
            dump(root/'installed-components.json', components)
            self.accounts = {int(u): a for u, a in json.loads(self.sql('SELECT jsonb_agg(jsonb_build_array(id,personal_account_id)) FROM core_identity.users'))}
            archive, mapping = 'a'*64, 'b'*64
            counters, payloads = [], []
            for user in self.fixture['tables']['users']:
                uid = user['id']
                common = dict(account_id=self.accounts[uid], migration_id='independent-finance-opening',
                              source_entity_key=f'legacy.users/{uid}', source_row_sha256=writer.F.digest(user),
                              source_archive_sha256=archive, mapping_sha256=mapping, observed_at=self.observed)
                wallet = self.fixture['opening'][uid]
                payloads.append({**common, 'available_units':wallet['available'], 'reserved_units':wallet['reserved'], 'affiliate_units':wallet['affiliate']})
                counters.append({**common, 'user_id':uid, 'used_quota':user['used_quota'],
                                 'request_count':user['request_count'], 'aff_history':user['aff_history']})
            sql = ['BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;']
            sql += [f'SELECT core_billing.import_opening_balance({literal(p)}::jsonb);' for p in payloads]
            sql += [f'SELECT core_billing.import_account_opening_counters({literal(c)}::jsonb);' for c in counters]
            sql += ["UPDATE core_identity.users SET active=false WHERE id IN (9,12);",
                    "INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('independent-synthetic-identity',repeat('a',64),repeat('b',64),'{\"synthetic_independent\":true}'::jsonb);", 'COMMIT;']
            self.owner('\n'.join(sql))
            identity = json.loads(self.sql("SET TIME ZONE 'UTC'; SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE migration_id='independent-synthetic-identity'"))
            self.opening = dict(source_archive_sha256=archive,mapping_sha256=mapping,
                                observation={'observed_at':self.observed},counters=counters,identity_import=identity)
            self.identities = [dict(source_user_id=u['id'],target_user_id=u['id'],target_personal_account_id=self.accounts[u['id']]) for u in self.fixture['tables']['users']]
            dump(root/(self.base[-1]+'-synthetic-source.json'), self.fixture)
            dump(root/(self.base[-1]+'-synthetic-opening.json'), self.opening)
            self.start_core()

        def import_source(self):
            self.stop_core()
            # Application action under test. Its output is compared to our labels;
            # it is never used to compute the expected HTTP response.
            tables = self.fixture['tables']
            plan = writer.build_plan(tables=tables,identity_records=self.identities,
                                     opening=self.opening,source_table_counts={k:len(v) for k,v in tables.items()})
            require({r['user_id']:r['normalized_used_credits'] for r in plan['users']} == self.fixture['normalized'], 'owner normalization disagrees with independent facts')
            require({r['source_order_id']:r['classification'] for r in plan['topups']} == self.fixture['labels'], 'owner categories disagree with independent old-contract labels')
            dump(root/(self.base[-1]+'-owner-plan-under-test.json'), plan)
            # Explicit artificial offline fence, restored only by this fixture.
            database=self.base[-1]
            self.sql(f'ALTER ROLE lmm_core_runtime NOLOGIN; REVOKE CONNECT ON DATABASE {database} FROM PUBLIC,lmm_core_runtime;')
            inventory = json.loads(self.owner_connection(writer.F.inventory_sql()+';', 'owner-inventory'))
            snapshot = json.loads(self.owner_connection(writer.snapshot_sql(inventory,['lmm_core_runtime']), 'owner-snapshot'))
            self.import_plan,self.initial_snapshot=plan,snapshot
            sql = writer.build_apply_sql(plan,snapshot,['lmm_core_runtime'])
            before = self.protected('owner-import-before')
            receipt = json.loads(self.owner_connection(sql,'owner-apply'))
            require(receipt['status'] == 'COMMITTED', 'owner import did not commit')
            after = self.protected('owner-import-after')
            allowed = {'core_history.finance_sources','core_history.finance_users','core_history.finance_topups'}
            require({k for k in before if before[k] != after[k]} <= allowed, 'owner import changed unrelated state')
            # A confirmed first commit permits an explicit exact replay; UNKNOWN never retries.
            replay = json.loads(self.owner_connection(sql,'owner-exact-replay'))
            require(replay['status'] == 'NOOP' and self.protected('owner-replay-after') == after, 'same-source replay changed state')
            self.source_key = plan['source']['source_key']
            observed_rows = json.loads(self.sql('SELECT jsonb_agg(to_jsonb(t) ORDER BY source_order_id) FROM core_history.finance_topups t'))
            for row in observed_rows:
                raw = next(r for r in tables['top_ups'] if r['id'] == row['source_order_id'])
                cls = self.fixture['labels'][row['source_order_id']]
                require(row['classification'] == cls, 'stored class mismatch')
                if cls in ('cash','virtual'):
                    require((row['currency'],row['gross_amount_micros'],row['refunded_amount_micros']) == (raw['settlement_currency'].strip().upper(),raw['settled_amount_micros'],raw['refunded_amount_micros']), 'stored original amounts mismatch')
                else:
                    require(all(row[k] is None for k in ['currency','gross_amount_micros','refunded_amount_micros']), 'unknown class invented money')
            dump(root/(database+'-independent-import-readback.json'), observed_rows)
            self.sql(f'ALTER ROLE lmm_core_runtime LOGIN; GRANT CONNECT ON DATABASE {database} TO lmm_core_runtime;')
            self.start_core()

        def auth_matrix(self):
            for name, status in [('admin',200),('root',200),('root_channel_deny',200),
                                 ('ordinary',403),('key',403),(None,401),('revoked_admin',401)]:
                self.site('auth-'+str(name), status, token=name)

        def acl(self):
            for table in ['finance_sources','finance_users','finance_topups','finance_native_payments']:
                relation = 'core_history.'+table
                answer = self.sql(f"SELECT has_table_privilege('lmm_core_runtime',{literal(relation)},'SELECT') AND NOT has_table_privilege('lmm_core_runtime',{literal(relation)},'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') AND NOT has_any_column_privilege('lmm_core_runtime',{literal(relation)},'INSERT,UPDATE,REFERENCES')")
                require(answer == 't', 'finance runtime ACL drift: '+relation)
            self.run('actual-runtime-view',self.base,input=b'SET ROLE lmm_core_runtime; SELECT count(*) FROM core_history.finance_native_payments;')
            for query in ['SELECT * FROM core_meta.site_finance_upgrades',
                          'SELECT request_sha256 FROM core_meta.site_finance_upgrades',
                          'DELETE FROM core_history.finance_topups WHERE false',
                          'TRUNCATE core_history.finance_sources']:
                outcome = self.run('actual-runtime-deny',self.base,input=('SET ROLE lmm_core_runtime; '+query+';').encode(),check=False)
                require(outcome.returncode != 0 and b'permission denied' in outcome.stderr, 'runtime denial missing')

        def clone(self, name, parent='perf'):
            self.stop_core()
            require(re.fullmatch('[a-z_]+',name+parent), 'invalid private database name')
            cmd = [*self.base];cmd[-1] = 'postgres'
            self.run('clone-'+name,cmd,input=f'CREATE DATABASE {name} WITH TEMPLATE {parent} OWNER lmm_core_owner;'.encode())
            self.base[-1] = name
            self.sql(f'REVOKE CREATE,TEMPORARY ON DATABASE {name} FROM PUBLIC; GRANT CONNECT ON DATABASE {name} TO independent_installer,lmm_core_runtime;')
            require(self.sql(f"SELECT has_database_privilege('lmm_core_runtime',{literal(name)},'CREATE,TEMP')")=='f','cloned runtime database authority is unsafe')
            query = urlencode({'host':str(self.socket),'port':55603,'sslmode':'disable'})
            (root/'runtime.url').write_text('postgresql://lmm_core_runtime@localhost/'+name+'?'+query)

        def post(self, action, key):
            result = json.loads(self.owner('SELECT core_billing.post_ledger('+literal(dict(scope='independent-finance',operation_key=key,reason='synthetic independent finance fixture',action=action))+'::jsonb);'))
            require(result.get('status') == 'posted', 'synthetic ledger action rejected')
            return result['journal_id']

        def native(self, credited=True, refunded=23, created=None, tag='123'):
            account = self.accounts[7]
            self.owner(f'SELECT core_billing.open_ledger_wallet({account});')
            journal = self.post(dict(kind='credit',account_id=account,amount_units=615000,payment_reference='independent-usd-'+tag), 'credit-'+tag) if credited else None
            if credited and refunded:
                self.post(dict(kind='refund_topup',journal_id=journal,amount_units=refunded*5000,refund_reference='independent-usd-refund'), 'refund-usd')
            timestamp = 'clock_timestamp()' if created is None else f'to_timestamp({int(created)})'
            transaction = "'pi_independent'" if credited else 'NULL'
            result = self.owner(f"INSERT INTO core_payments.intents(account_id,created_by_user_id,channel_id,provider,merchant,environment,currency,amount_minor,credit_units,credits_per_minor,prepare_key,prepare_hash,credit_journal_id,provider_transaction,refunded_minor,created_at) VALUES({account},7,1,'stripe','independent','test','USD',123,615000,5000,{literal('independent-prepare-'+tag)},decode(repeat('a',64),'hex'),{journal if journal is not None else 'NULL'},{transaction},{refunded if credited else 0},{timestamp}) RETURNING id;")
            intent = int(result)
            receipt='pay_'+('a'*64 if tag=='123' else hashlib.sha256(tag.encode()).hexdigest())
            self.owner(f"INSERT INTO core_payments.receipts(id,intent_id,operation_key,method,request_hash,receipt_wire) VALUES({literal(receipt)},{intent},'independent-receipt','credit',decode(repeat('a',64),'hex'),decode('00','hex'));")
            if credited and refunded:
                self.owner(f"INSERT INTO core_payments.provider_refunds(channel_id,refund_id,intent_id,amount_minor,receipt_id) VALUES(1,'re_independent',{intent},{refunded},{literal(receipt)});")
            if credited:
                self.proof(intent,'Credit',123)
                if refunded:self.proof(intent,'RefundSucceeded',refunded,'re_independent')
            return intent

        def proof(self, intent, kind, amount, refund_id=None):
            digest=hashlib.sha256(json.dumps([intent,kind,amount,refund_id]).encode()).hexdigest()
            event='evt_'+digest[:24]
            facts = dict(event_id=event,transaction='pi_independent',kind=kind,amount_minor=amount,refund_id=refund_id,hold_receipt_id=None)
            self.owner(f"INSERT INTO core_payments.accepted_proofs(id,channel_id,event_id,intent_id,payload_hash,signed_payload,signature,verified_facts,receipt_wire) VALUES({literal('proof_'+digest)},1,{literal(event)},{intent},decode({literal(digest)},'hex'),decode('00','hex'),decode('00','hex'),{literal(facts)}::jsonb,decode('00','hex'));")

        def native_happy(self):
            self.clone('native_paid')
            self.native()
            self.start_core()
            expected = oracle.expected_statistics(123,23)
            self.site('native-usd',expected=expected)
            self.stop_core();self.start_core()
            self.site('native-after-restart',expected=expected)

        def source_count(self):
            self.clone('source_count')
            self.owner(f"INSERT INTO core_history.finance_topups(source_key,source_order_id,source_row_sha256,classification) VALUES({literal(self.source_key)},999,repeat('f',64),'excluded');")
            self.start_core();self.site('source-count',503)

        def credited_fault(self, mismatch=False):
            self.clone('counter_fault' if mismatch else 'refund_fault', 'native_paid')
            if mismatch:
                self.owner('UPDATE core_payments.intents SET refunded_minor=24 WHERE amount_minor=123;')
            else:
                intent = int(self.sql('SELECT id FROM core_payments.intents WHERE amount_minor=123'))
                self.proof(intent,'RefundSucceeded',5,'re_unapplied')
            self.start_core();self.site('native-counter' if mismatch else 'unresolved-refund',503)

        def refund_amount_fault(self):
            self.clone('refund_amount_fault','native_paid')
            intent=int(self.sql('SELECT id FROM core_payments.intents WHERE amount_minor=123'))
            self.proof(intent,'RefundSucceeded',24,'re_independent')
            self.start_core();self.site('refund-original-id-wrong-amount',503)

        def precredit(self, refund=False):
            self.clone('pre_refund' if refund else 'pre_credit')
            intent = self.native(credited=False,refunded=0)
            self.proof(intent,'RefundSucceeded' if refund else 'Credit',23 if refund else 123,'re_precredit' if refund else None)
            self.start_core();self.site('precredit-'+str(refund),503)

        def epoch(self, offset):
            self.clone('epoch_inside' if offset == 1 else 'epoch_outside')
            self.native(created=self.observed+offset)
            self.start_core()
            self.site('epoch-'+str(offset),503 if offset == 1 else 200,oracle.expected_statistics(123,23))

        def native_usage(self):
            self.clone('native_usage')
            self.owner(f'SELECT core_billing.open_ledger_wallet({self.accounts[7]});')
            self.post(dict(kind='gift',account_id=self.accounts[7],amount_units=1000),'usage-funding')
            self.start_core()
            calls=len(self.supplier)
            self.completion(expect=200)
            require(len(self.supplier)==calls+1,'logical request supplier count')
            deadline=time.monotonic()+10
            while True:
                rows=json.loads(self.sql("SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'state',state,'settled',settled,'refunded',refunded,'price',price)),'[]'::jsonb) FROM core_charging.requests"))
                if len(rows)==1 and rows[0]['state']=='settled':break
                require(time.monotonic()<deadline,'actual usage did not settle')
                time.sleep(.1)
            # TokenRates are micro-USD per million tokens (routing/pricing.rs:27).
            # Convert at 500000 credits/USD, then apply frozen admin discount9/10.
            policy=json.loads(self.sql('SELECT spec FROM core_trust_pricing.policies'))
            require(policy['admin_discount']=={'numerator':9,'denominator':10},'native tariff authority differs from the bound independent fixture')
            numerator=(10*2_000_000+5*4_000_000)*500_000*9
            denominator=1_000_000*1_000_000*10
            expected_cost=(numerator+denominator-1)//denominator
            require((rows[0]['price']['input_per_million'],rows[0]['price']['output_per_million'])==(1_000_000,2_000_000),'persisted tariff conversion differs from configured units')
            require((rows[0]['settled'],rows[0]['refunded'])==(expected_cost,0),'actual native charge disagrees with independent tariff arithmetic')
            self.wait_samples(1)  # Drain completion's durable observer before no-write snapshots.
            expected=oracle.expected_statistics()
            expected.update(total_used_credits=str(695+expected_cost),total_balance_credits=str(105+1000-expected_cost))
            self.site('native-usage-settled',expected=expected)
            query=urlencode({'host':str(self.socket),'port':55603,'sslmode':'disable','options':'-c role=lmm_core_owner'})
            refund_url=root/'refund-owner.url'
            refund_url.write_text('postgresql://independent_installer@localhost/native_usage?'+query)
            self.run('actual-native-refund',[root/'refund-helper',refund_url,rows[0]['id'],'7'])
            actual=json.loads(self.sql("SELECT jsonb_build_object('settled',settled,'refunded',refunded) FROM core_charging.requests"))
            require(actual=={'settled':expected_cost,'refunded':7},'real refund readback')
            expected.update(total_used_credits=str(695+expected_cost-7),total_balance_credits=str(105+1000-expected_cost+7))
            self.site('native-usage-refunded',expected=expected)
            self.stop_core();self.start_core()
            self.site('native-usage-restart',expected=expected)
            require(len(self.supplier)==calls+1,'restart or finance read redispatched model')
            dump(root/'native-usage-independent-facts.json',{'input_tokens':10,'output_tokens':5,'input_micro_usd_per_million_tokens':2_000_000,'output_micro_usd_per_million_tokens':4_000_000,'credits_per_usd':500000,'admin_discount_numerator':9,'admin_discount_denominator':10,'expected_settled_credits':18,'refund_credits':7,'expected_net_credits':11})

        def second_source(self):
            self.clone('second_source')
            before=self.protected('second-source-before')
            # Independently change only the source identity, retaining the raw
            # monetary vector. Writer summaries never determine this expectation.
            alternate=copy.deepcopy(self.import_plan)
            alternate['source']['tables_sha256']='d'*64
            alternate['source']['source_key']=writer.canonical_sha({k:v for k,v in alternate['source'].items() if k!='source_key'})
            alternate['plan_sha256']=writer.canonical_sha({k:v for k,v in alternate.items() if k!='plan_sha256'})
            self.sql('ALTER ROLE lmm_core_runtime NOLOGIN; REVOKE CONNECT ON DATABASE second_source FROM PUBLIC,lmm_core_runtime;')
            inventory=json.loads(self.owner_connection(writer.F.inventory_sql()+';','second-source-inventory'))
            actual=json.loads(self.owner_connection(writer.snapshot_sql(inventory,['lmm_core_runtime']),'second-source-owner-snapshot'))
            try:
                writer.build_apply_sql(alternate,actual,['lmm_core_runtime'])
            except writer.Error as error:
                require(str(error)=='target_source_key_conflict','writer rejected for a different reason')
            else:
                raise AssertionError('owner prepare accepted a different source key')
            # A previously prepared SQL action must also refuse the installed
            # source before other target differences can hide this rejection.
            foreign_sql=writer.build_apply_sql(alternate,self.initial_snapshot,['lmm_core_runtime'])
            command=[*self.base];command[command.index('-U')+1]='independent_installer'
            apply=self.run('second-source-owner-apply',command,input=('SET ROLE lmm_core_owner;\n'+foreign_sql).encode(),check=False)
            require(apply.returncode!=0 and b'site_finance_source_key_conflict' in apply.stderr,'owner apply did not reject different source key')
            request_column=',native_request_ids' if binding.get('seal_regression') else ''
            statement="\\set VERBOSITY verbose\nSET ROLE lmm_core_owner; INSERT INTO core_history.finance_sources ("+','.join(SOURCE_COLUMNS)+request_column+") SELECT repeat('f',64),contract,source_archive_sha256,mapping_sha256,tables_sha256,normalization_sha256,rows_sha256,observed_at,0,0,credits_per_usd,complete"+request_column+" FROM core_history.finance_sources;"
            result=self.run('second-source-23505',self.base,input=statement.encode(),check=False)
            require(result.returncode!=0 and b'23505' in result.stderr and b'finance_sources_one_archive' in result.stderr,'second source must fail at the product unique index')
            require(before==self.protected('second-source-after'),'rejected source changed state')
            self.sql('ALTER ROLE lmm_core_runtime LOGIN; GRANT CONNECT ON DATABASE second_source TO lmm_core_runtime;')
            self.start_core();self.site('after-second-source-rejected')

        def beyond_i64(self):
            self.clone('large_totals','seed_template')
            self.fixture=oracle.fixture()
            # Negative affiliate position balances each wallet opening. No system
            # bucket overflows and no ledger trigger is bypassed to make the test.
            for uid in range(1000,2031):
                self.fixture['tables']['users'].append(dict(id=uid,quota=oracle.SAFE,used_quota=oracle.SAFE,
                    request_count=0,aff_history=0,aff_quota=-oracle.SAFE,status=1,deleted_at=None))
                self.fixture['opening'][uid]=dict(available=oracle.SAFE,reserved=0,affiliate=-oracle.SAFE)
                self.fixture['normalized'][uid]=oracle.SAFE
                oid=uid+10000
                self.fixture['tables']['top_ups'].append(dict(id=oid,user_id=uid,trade_no=f'large-{oid}',
                    status='success',payment_provider='stripe',payment_method='stripe',settlement_currency='USD',
                    settled_amount_micros=oracle.SAFE,refunded_amount_micros=1,expected_amount_micros=0,credited_quota=0,money='0'))
                self.fixture['labels'][oid]='cash'
            self.owner("DO $$ DECLARE n bigint; a bigint; BEGIN FOR n IN 1000..2030 LOOP INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id INTO a; INSERT INTO core_identity.users(id,personal_account_id) VALUES(n,a); END LOOP; END $$;")
            self.prepare_opening()
            self.import_source()
            total=1031*oracle.SAFE
            expected=oracle.expected_statistics()
            expected.update(total_used_credits=str(695+total),total_balance_credits=str(105+total))
            usd=expected['recharge']['currencies'][1]
            usd.update(gross_amount_micros=str(2000001+total),refunded_amount_micros=str(1000001+1031),
                       net_amount_micros=str(1000000+total-1031),orders=1033)
            expected['recharge']['confirmed_orders']=1035
            independent_sql=json.loads(self.sql("SELECT jsonb_build_object('used',(SELECT sum(normalized_used_credits::numeric)::text FROM core_history.finance_users),'balance',(SELECT sum(b.balance_units::numeric)::text FROM core_billing.balance_state b JOIN core_billing.ledger_accounts l ON l.id=b.ledger_account_id JOIN core_identity.users u ON u.personal_account_id=l.owner_account_id WHERE l.bucket IN ('wallet','debt')),'gross_usd',(SELECT sum(gross_amount_micros::numeric)::text FROM core_history.finance_topups WHERE classification='cash' AND currency='USD'),'conservation',(SELECT sum(balance_units::numeric)::text FROM core_billing.balance_state))"))
            require(independent_sql=={'used':str(695+total),'balance':str(105+total),'gross_usd':str(2000001+total),'conservation':'0'},'large-fact SQL reconciliation failed')
            require(all(int(expected[k])>2**63-1 for k in ('total_used_credits','total_balance_credits')),'fixture did not exceed i64')
            dump(root/'large-independent-values.json',{'expected':expected,'sql':independent_sql})
            self.site('larger-than-i64',expected=expected)

        def source_seals(self):
            return json.loads(self.sql("SELECT coalesce(jsonb_agg(jsonb_build_object('source_key',source_key,'native_intent_floor',native_intent_floor,'native_request_ids',native_request_ids) ORDER BY source_key),'[]') FROM core_history.finance_sources"))

        def fence(self):
            self.stop_core()
            self.sql(f'ALTER ROLE lmm_core_runtime NOLOGIN; REVOKE CONNECT ON DATABASE {self.base[-1]} FROM PUBLIC,lmm_core_runtime;')

        def unfence(self):
            self.sql(f'ALTER ROLE lmm_core_runtime LOGIN; GRANT CONNECT ON DATABASE {self.base[-1]} TO lmm_core_runtime;')

        def close_cached_client(self):
            if self.cache_client and self.cache_client.poll() is None:
                try:
                    self.cache_client.stdin.write(b'ROLLBACK;\n\\q\n');self.cache_client.stdin.flush()
                    self.cache_client.stdin.close()
                except BrokenPipeError:
                    pass
                try:
                    self.cache_client.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.cache_client.kill();self.cache_client.wait(timeout=3)

        def preexisting_seal(self):
            self.clone('seal_pending','seed_template')
            self.prepare_opening();self.stop_core()
            self.owner('ALTER SEQUENCE core_payments.intents_id_seq CACHE 16;')
            old=self.native(credited=False,refunded=0,created=self.observed+86400)
            require(old==1,'clean sequence fixture did not begin at one')
            self.fence()
            inventory=json.loads(self.owner_connection(writer.F.inventory_sql()+';','seal-inventory'))
            script=b"BEGIN; SELECT json_build_object('pid',pg_backend_pid(),'allocated',nextval('core_payments.intents_id_seq'));\n\\echo independent_cache_ready\n"
            (root/'cached-client.stdin').write_bytes(script)
            dump(root/'cached-client.command.json',self.base)
            with (root/'cached-client.stdout').open('wb') as output, (root/'cached-client.stderr').open('wb') as error:
                self.cache_client=subprocess.Popen(self.base,stdin=subprocess.PIPE,stdout=output,stderr=error,env=self.pgenv)
            self.cache_client.stdin.write(script);self.cache_client.stdin.flush()
            deadline=time.monotonic()+10
            while 'independent_cache_ready' not in (root/'cached-client.stdout').read_text():
                require(self.cache_client.poll() is None and time.monotonic()<deadline,'cached client did not reach open transaction')
                time.sleep(.05)
            allocated=json.loads((root/'cached-client.stdout').read_text().splitlines()[0])
            require(allocated['allocated']==17,'CACHE 16 did not preallocate the expected range')
            activity=f"SELECT jsonb_build_object('caller',current_user,'pid',pid,'datid',datid,'user',usename,'backend_type',backend_type,'state',state) FROM pg_stat_activity WHERE pid={allocated['pid']};"
            super_view=json.loads(self.sql(activity))
            owner_view=json.loads(self.owner_connection(activity,'cached-allocator-owner-view'))
            dump(root/'cached-allocator-observations.json',{'superuser':super_view,'owner':owner_view})
            require(super_view['state']=='idle in transaction' and super_view['backend_type']=='client backend','cached allocator is not live in a transaction')
            before=self.protected('cache-client-before-denied-import')
            command=[*self.base];command[command.index('-U')+1]='independent_installer'
            rejected=self.run('seal-live-transaction-denied',command,
                input=('SET ROLE lmm_core_owner;\n'+writer.snapshot_sql(inventory,['lmm_core_runtime'])).encode(),check=False)
            require(rejected.returncode!=0 and b'site_finance_other_clients' in rejected.stderr,'owner accepted a live cached allocator')
            require(before==self.protected('cache-client-after-denied-import'),'guard denial wrote state')
            self.close_cached_client()
            require(self.cache_client.returncode==0,'cached allocator did not close cleanly')
            require(self.sql("SELECT pg_sequence_last_value('core_payments.intents_id_seq')")=='32','ROLLBACK unexpectedly returned reserved IDs')
            self.owner('ALTER SEQUENCE core_payments.intents_id_seq CACHE 1;')
            self.unfence();self.import_source()
            expected_seal=[{'source_key':self.source_key,'native_intent_floor':32,'native_request_ids':[]}]
            require(set(self.import_plan['source'])==set(SOURCE_COLUMNS),'server seal changed the twelve-field source identity')
            require(self.source_seals()==expected_seal,'seal failed to capture the cached upper bound')
            self.site('preexisting-uncredited-still-readable')
            self.clone('seal_late_credit','seal_pending')
            journal=self.post(dict(kind='credit',account_id=self.accounts[7],amount_units=615000,payment_reference='late-source-credit'),'late-source-credit')
            self.owner(f"UPDATE core_payments.intents SET credit_journal_id={journal},provider_transaction='pi_independent' WHERE id={old};")
            self.proof(old,'Credit',123)
            facts=json.loads(self.sql('SELECT jsonb_build_object(\'id\',id,\'created_at\',extract(epoch FROM created_at)::bigint,\'credited\',credit_journal_id IS NOT NULL) FROM core_payments.intents'))
            require(facts=={'id':1,'created_at':self.observed+86400,'credited':True},'late-credit fixture changed identity/time')
            dump(root/'preexisting-seal-independent-facts.json',{'intent':facts,'source_seals':expected_seal,'cached_allocator':allocated,'expected_http':503})
            self.start_core();self.site('dump-start-gap-late-credit',503)

        def post_seal_native(self):
            self.clone('seal_new_paid','seal_pending')
            # A post-seal identity can legitimately carry the dump-start second.
            # Its identity, not the archive's observation timestamp, is the proof.
            new=self.native(created=self.observed,tag='post-seal')
            require(new==33,'post-seal allocation did not pass the drained cached range')
            require(int(self.sql(f'SELECT extract(epoch FROM created_at)::bigint FROM core_payments.intents WHERE id={new}'))==self.observed,'same-second positive fixture drift')
            self.start_core();self.site('post-seal-new-native',expected=oracle.expected_statistics(123,23))
            self.fence()
            inventory=json.loads(self.owner_connection(writer.F.inventory_sql()+';','noop-seal-inventory'))
            snapshot=json.loads(self.owner_connection(writer.snapshot_sql(inventory,['lmm_core_runtime']),'noop-seal-snapshot'))
            seals=self.source_seals()
            require(snapshot['source_seals']==seals==[{'source_key':self.source_key,'native_intent_floor':32,'native_request_ids':[]}],'snapshot lost the original server seal')
            require(snapshot['native_intent_upper']==33,'NOOP fixture did not advance the sequence')
            sql=writer.build_apply_sql(self.import_plan,snapshot,['lmm_core_runtime'])
            before=self.protected('seal-noop-before')
            receipt=json.loads(self.owner_connection(sql,'seal-noop-after-new-intent'))
            require(receipt['status']=='NOOP','same-source fresh preview did not replay')
            require(before==self.protected('seal-noop-after'),'NOOP changed protected state')
            require(self.source_seals()==seals,'NOOP resampled the server floor')
            dump(root/'post-seal-independent-facts.json',{'new_intent':new,'created_at':self.observed,'dump_started_at':self.observed,'original_floor':32,'snapshot_source_seals':snapshot['source_seals'],'expected_native_gross_micros':1230000,'expected_native_refunded_micros':230000,'expected_native_net_micros':1000000})
            self.unfence();self.start_core()
            self.site('post-seal-native-after-noop',expected=oracle.expected_statistics(123,23))

        def sequence_premises(self):
            for name,mutation,read_reject in [
                ('seal_step','ALTER SEQUENCE core_payments.intents_id_seq INCREMENT BY 2;',True),
                ('seal_cycle','ALTER SEQUENCE core_payments.intents_id_seq CYCLE;',True),
                ('seal_regress',"SELECT setval('core_payments.intents_id_seq',32,true);",False)]:
                self.clone(name,'seal_new_paid')
                self.start_core();self.owner(mutation)
                if read_reject:self.site(name+'-unavailable',503)
                self.fence()
                inventory=json.loads(self.owner_connection(writer.F.inventory_sql()+';',name+'-inventory'))
                command=[*self.base];command[command.index('-U')+1]='independent_installer'
                before=self.protected(name+'-before-guard')
                outcome=self.run(name+'-owner-denied',command,
                    input=('SET ROLE lmm_core_owner;\n'+writer.snapshot_sql(inventory,['lmm_core_runtime'])).encode(),check=False)
                require(outcome.returncode!=0 and b'site_finance_native_intent_sequence_invalid' in outcome.stderr,'invalid sequence admitted by owner guard')
                require(before==self.protected(name+'-after-guard'),'sequence guard wrote state')
                self.unfence()

        def request_action(self, operation, user_id, request_id):
            if operation=='reserve':
                self.request_workers[request_id]=list(secrets.token_bytes(32))
                self.owner(f"INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES({user_id},'allowed',true,'independent-request-seal') ON CONFLICT(user_id,group_name) DO NOTHING;")
            payload={'operation':operation,'user_id':user_id,'id':request_id,'worker':self.request_workers[request_id],
                'session':next(v['secret'] for v in self.creds['sessions'].values() if v['user_id']==user_id) if operation=='reserve' else None}
            private=root/f'request-input-{request_id}-{operation}.json';dump(private,payload)
            query=urlencode({'host':str(self.socket),'port':55603,'sslmode':'disable','options':'-c role=lmm_core_owner'})
            url=root/'request-owner.url';url.write_text('postgresql://independent_installer@localhost/'+self.base[-1]+'?'+query)
            self.run('request-'+operation,[root/'request-helper',url,private])
            actual=json.loads(self.sql(f"SELECT jsonb_build_object('id',id,'actor',actor_user_id,'state',state,'settled',settled,'refunded',refunded,'authorized_at',authorized_at,'price',price) FROM core_charging.requests WHERE id={literal(request_id)}"))
            require(actual['actor']==user_id and actual['state']=={'reserve':'reserved','release':'released','settle':'settled'}[operation],'real Charging state differs')
            require(actual['settled']==(20 if operation=='settle' else 0) and actual['refunded']==0,'legacy Price request independent cost differs')
            require(actual['price']['input_per_million']==1_000_000 and actual['price']['output_per_million']==2_000_000 and actual['price'].get('user_trust') is None,'synthetic Price contract drift')
            return actual

        def historical_requests(self):
            self.clone('request_history_pending','seed_template')
            self.observed=int(time.time())
            self.prepare_opening();self.stop_core()
            request='zz-historical-preseal'
            pending=self.request_action('reserve',8,request)
            self.import_source()
            require(self.source_seals()==[{'source_key':self.source_key,'native_intent_floor':0,'native_request_ids':[request]}],'historical request absent from complete seal')
            expected=oracle.expected_statistics();expected['total_balance_credits']=str(105-30)
            self.site('historical-pending-not-consumed',expected=expected)
            self.clone('request_history_released','request_history_pending')
            self.request_action('release',8,request)
            self.start_core();self.site('historical-released-not-consumed')
            self.clone('request_history_settled','request_history_pending')
            settled=self.request_action('settle',8,request)
            require(settled['authorized_at']==pending['authorized_at'],'settlement changed immutable authorization time')
            self.start_core();self.site('historical-preseal-late-settlement',503)
            dump(root/'historical-request-independent-facts.json',{'pending':pending,'settled':settled,'price_unit':'credits per million tokens; legacy Price, no trust entitlement','expected_cost':(10*1_000_000+5*2_000_000)//1_000_000,'expected_pending_used':695,'expected_pending_balance':75,'late_settlement_expected_http':503,'canonical_archive_cli':'NOT_RUN; synthetic complete three-user map and real owner SQL'})

        def native_requests(self):
            self.clone('request_native_old','seed_template')
            self.observed=int(time.time())
            self.prepare_opening();self.stop_core()
            self.owner(f'SELECT core_billing.open_ledger_wallet({self.accounts[7]});')
            self.post(dict(kind='gift',account_id=self.accounts[7],amount_units=1000),'native-request-gift')
            old='00-native-before-seal'
            self.request_action('reserve',7,old);before=self.request_action('settle',7,old)
            self.import_source()
            identity=json.loads(self.sql(f"SELECT jsonb_build_object('historically_mapped',EXISTS(SELECT 1 FROM core_history.finance_users WHERE user_id=7),'native_audit',EXISTS(SELECT 1 FROM core_identity.audit WHERE actor_user_id=7 AND action='user.bootstrap' AND target->>'user_id'='7'),'created_before_dump',a.created_at<to_timestamp({self.observed})) FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=7"))
            require(identity=={'historically_mapped':False,'native_audit':True,'created_before_dump':True},'old Native identity provenance fixture differs')
            seals=[{'source_key':self.source_key,'native_intent_floor':0,'native_request_ids':[old]}]
            require(self.source_seals()==seals,'Native-only request absent from seal')
            expected=oracle.expected_statistics();expected.update(total_used_credits='715',total_balance_credits='1085')
            self.site('native-only-preseal-still-counted',expected=expected)
            self.stop_core()
            new='aa-historical-after-seal'
            self.request_action('reserve',8,new);after=self.request_action('settle',8,new)
            self.start_core()
            expected.update(total_used_credits='735',total_balance_credits='1065')
            self.site('historical-postseal-new-counted',expected=expected)
            self.fence()
            inventory=json.loads(self.owner_connection(writer.F.inventory_sql()+';','request-noop-inventory'))
            snapshot=json.loads(self.owner_connection(writer.snapshot_sql(inventory,['lmm_core_runtime']),'request-noop-snapshot'))
            require(snapshot['source_seals']==seals,'snapshot resampled the sealed request set')
            require(snapshot['native_request_ids']==sorted([old,new]),'snapshot omitted current requests')
            sql=writer.build_apply_sql(self.import_plan,snapshot,['lmm_core_runtime'])
            protected=self.protected('request-noop-before')
            receipt=json.loads(self.owner_connection(sql,'request-seal-noop'))
            require(receipt['status']=='NOOP' and self.source_seals()==seals,'NOOP rewrote request seals')
            require(protected==self.protected('request-noop-after'),'request NOOP changed state')
            self.unfence();self.start_core();self.site('request-noop-original-seal',expected=expected)
            dump(root/'native-request-independent-facts.json',{'identity':identity,'preseal_request':before,'postseal_request':after,'sealed_requests':[old],'expected_preseal_used':715,'expected_final_used':735,'expected_final_balance':1065,'same_authorized_second_observed':before['authorized_at']==after['authorized_at'],'canonical_archive_cli':'NOT_RUN; synthetic opening/source material, real bootstrap/Charging/owner SQL'})

        def execute(self):
            success = False
            try:
                if binding.get('resume_seals'):
                    self.active_group=SEAL_GROUPS[0]
                    self.prepare();self.prepare_opening();self.import_source()
                elif binding.get('reuse_from'):
                    # Build only the clean baseline needed by the three remaining
                    # cases. Do not rerun the fourteen accepted HTTP scenarios.
                    self.active_group='native-usage-and-refund'
                    self.prepare();self.prepare_opening();self.import_source()
                else:
                    self.group('fresh28-native-binding',lambda:(self.prepare(),self.prepare_opening()))
                    self.group('opening-without-finance-is-unknown',lambda:self.site('missing-finance',503))
                    self.group('owner-import-and-independent-readback',self.import_source)
                    if not binding.get('seal_regression') or binding.get('full_current'):
                        self.group('dto-permissions-and-read-only',self.auth_matrix)
                    self.group('runtime-finance-acl',self.acl)
                    self.group('native-usd-and-restart',self.native_happy)
                    self.group('source-count-rejection',self.source_count)
                    self.group('unresolved-credited-refund',self.credited_fault)
                    self.group('refund-id-amount-mismatch',self.refund_amount_fault)
                    self.group('refund-counter-mismatch',lambda:self.credited_fault(True))
                    self.group('precredit-credit-rejection',self.precredit)
                    self.group('precredit-refund-rejection',lambda:self.precredit(True))
                    if not binding.get('seal_regression'):
                        self.group('epoch-at-plus-one-rejection',lambda:self.epoch(1))
                        self.group('epoch-plus-two-accepted',lambda:self.epoch(2))
                if binding.get('seal_regression'):
                    if binding.get('full_current'):
                        self.group('native-usage-and-refund',self.native_usage)
                    self.group('second-source-rejected',self.second_source)
                    self.group(SEAL_GROUPS[0],self.preexisting_seal)
                    self.group(SEAL_GROUPS[1],self.post_seal_native)
                    self.group(SEAL_GROUPS[2],self.sequence_premises)
                    self.group(SEAL_GROUPS[3],self.historical_requests)
                    self.group(SEAL_GROUPS[4],self.native_requests)
                    if binding.get('full_current'):
                        self.group('beyond-i64-pg-http',self.beyond_i64)
                    require(len(self.supplier)==(1 if binding.get('full_current') else 0),'unexpected local supplier traffic')
                    success=True
                else:
                    self.group('native-usage-and-refund',self.native_usage)
                    self.group('second-source-rejected',self.second_source)
                    self.group('beyond-i64-pg-http',self.beyond_i64)
                    require(len(self.supplier)==1,'unexpected supplier traffic')
                    success = True
            except Exception:
                (root/'failure.txt').write_text(traceback.format_exc())
                if self.active_group:self.results[self.active_group] = 'FAIL'
            finally:
                dump(root/'results.json',self.results)
                self.close_cached_client()
                self.stop_core();self.release.set()
                if self.provider:self.provider.shutdown();self.provider.server_close()
                if self.pg:self.run('pg-stop',['pg_ctl','-D',root/'pg','-m','fast','-w','stop'],check=False)
                dump(root/'stopped.json',{'core_stopped':self.core is None or self.core.poll() is not None,'pg_stopped':not(root/'pg/postmaster.pid').exists(),'supplier_calls':len(self.supplier)})
            require(not(root/'pg/postmaster.pid').exists(),'PG cleanup incomplete')
            if success:
                removed={}
                for name in ['pg','s','lmm-core','fixture','refund-helper','request-helper','request-owner.url','refund-owner.url','owner.url','runtime.url','supplier.key','credentials.json',*[p.name for p in root.glob('request-input-*.json')]]:
                    p=root/name
                    if p.exists():
                        removed[name]=sum(x.stat().st_size for x in p.rglob('*') if x.is_file()) if p.is_dir() else p.stat().st_size
                        shutil.rmtree(p) if p.is_dir() else p.unlink()
                dump(root/'cleanup.json',{'logical_bytes_removed':sum(removed.values()),'paths':removed})
                for path in self.log.glob('*-seed.stdout'):
                    original=sha(path);dump(path,{'synthetic_credentials_removed':True,'original_sha256':original})
            return 0 if success else 1

    with open('/tmp/lmm-delegate/heavy-build.lock','a') as lock:
        (root/'waiting-for-lock').write_text(str(os.getpid()))
        fcntl.flock(lock,fcntl.LOCK_EX);(root/'waiting-for-lock').unlink()
        memory=Path('/proc/meminfo').read_text();(root/'memory-admission.txt').write_text(memory)
        require(int(next(line.split()[1] for line in memory.splitlines() if line.startswith('MemAvailable:'))) >= 2097152,'2GiB memory admission')
        return Run().execute()


def main():
    os.umask(0o077)
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root',type=Path,required=True)
    p.add_argument('--candidate',type=Path)
    p.add_argument('--core-sha');p.add_argument('--rlib-sha')
    p.add_argument('--source-pins-sha')
    p.add_argument('--reuse-first-fourteen',type=Path)
    p.add_argument('--seal-regression',action='store_true')
    p.add_argument('--resume-seals',type=Path)
    p.add_argument('--full-current',action='store_true')
    p.add_argument('--import-tree',type=Path);p.add_argument('--import-sha')
    p.add_argument('--execute',action='store_true');p.add_argument('--worker',action='store_true')
    args=p.parse_args();root=args.root.absolute()
    if args.full_current:args.seal_regression=True
    if args.worker:return worker(root)
    require(args.execute,'Preparation only: use independent_oracle.py; actual run requires --execute and final R2 pins')
    require(not(args.seal_regression and args.reuse_first_fourteen),'new DDL requires affected cases, not same-candidate reuse')
    require(not args.resume_seals or args.seal_regression,'resume-seals requires the seal profile')
    require(not args.full_current or not(args.resume_seals or args.reuse_first_fourteen),'full current matrix cannot reuse previous groups')
    require(all([args.candidate,args.core_sha,args.rlib_sha,args.source_pins_sha,args.import_tree,args.import_sha]),'final Native/library/source/importer bindings are required')
    require(args.core_sha != OLD_CORE,'old 27-component performance R2 is not finance evidence')
    require(root.parent == Path('/home/lightjunction/Documents/Codex') and not root.exists(),'fresh private evidence root required')
    require(len(str(root/'s/.s.PGSQL.55603').encode()) < 104,'short socket root required')
    require(shutil.disk_usage(root.parent).free > 2*1024**3,'disk admission; run space-first before launch')
    candidate=args.candidate.resolve();import_tree=args.import_tree.resolve()
    result=json.loads((candidate/'result.json').read_text())
    require(str(result.get('status','')).lower() in ('pass','passed'),'candidate result is not PASS')
    binaries=json.loads((candidate/'binaries.json').read_text())
    require(binaries['lmm-core']['sha256']==args.core_sha,'published Core hash mismatch')
    if 'liblmm_core.rlib' in binaries:
        require(binaries['liblmm_core.rlib']['sha256']==args.rlib_sha,'published library hash mismatch')
    require(sha(candidate/'lmm-core') == args.core_sha and sha(candidate/'liblmm_core.rlib') == args.rlib_sha,'candidate artifact mismatch')
    require(candidate.name not in ('compile-r1','compile-r2'),'finance R1/R2 are intermediate; final acceptance requires R3 or later')
    require(sha(import_tree/'legacy_site_statistics.py') == args.import_sha,'owner importer mismatch')
    require(sha(candidate/'source-pins.json') == args.source_pins_sha,'candidate differs from announced source pins')
    pins=json.loads((candidate/'source-pins.json').read_text())
    require(all(sha(candidate/'source'/n)==h for n,h in pins.items()),'candidate frozen source mismatch')
    require('services/core/schema/site_statistics.sql' in pins,'finance schema absent from frozen source')
    schema=(candidate/'source/services/core/schema/site_statistics.sql').read_text()
    if args.seal_regression:
        require('native_intent_floor' in schema and 'native_request_ids' in schema and 'i.created_at<=to_timestamp(s.observed_at+1)' not in schema,'seal cases require final ID-only contract, not superseded R5')
    else:
        require('native_intent_floor' not in schema,'current seal schema requires --seal-regression; old epoch cases are superseded')
    reuse=None
    resume=None
    if args.resume_seals:
        previous=args.resume_seals.resolve()
        old=json.loads((previous/'binding.json').read_text());results=json.loads((previous/'results.json').read_text())
        accepted=[name for name in GROUPS if results.get(name)=='PASS']
        require(len(accepted)==12 and results.get(SEAL_GROUPS[0])=='FAIL','expected the frozen 12-pass owner-guard failure')
        require((old['core_sha256'],old['rlib_sha256'])==(args.core_sha,args.rlib_sha) and sha(previous/'source-pins.json')==args.source_pins_sha,'R6 Core inputs changed; cannot resume this scope')
        for name in ['independent_oracle.py','independent_refund.rs','independent_requests.rs']:
            require(sha(previous/'runner'/name)==sha(Path(__file__).parent/name),'request/amount fixture drift: '+name)
        for name in IMPORT_FILES:
            before=(previous/'importer'/name).read_bytes();after=(import_tree/name).read_bytes()
            if name=='legacy_site_statistics.py':
                expected=before.replace(b"AND pid<>pg_backend_pid() AND backend_type='client backend'",b'AND pid IS NOT NULL AND pid<>pg_backend_pid()')
                require(before!=after and after==expected,'owner delta exceeds the reviewed visible-PID fix')
            else:require(before==after,'other importer dependency changed: '+name)
        resume={'root':str(previous),'binding_sha256':sha(previous/'binding.json'),'results_sha256':sha(previous/'results.json'),'groups':accepted,'prior_import_sha256':old['import_sha256'],'scope':'prior same-Core business evidence; current writer baseline and five seal/request groups rerun'}
    if args.reuse_first_fourteen:
        previous=args.reuse_first_fourteen.resolve()
        old=json.loads((previous/'binding.json').read_text());results=json.loads((previous/'results.json').read_text())
        require(all(results.get(name)=='PASS' for name in GROUPS[:14]),'previous fourteen groups did not all pass')
        require((old['core_sha256'],old['rlib_sha256'],old['import_sha256'])==(args.core_sha,args.rlib_sha,args.import_sha),'reused evidence has different product inputs')
        require(sha(previous/'source-pins.json')==args.source_pins_sha,'reused evidence source mismatch')
        for name in ['independent_oracle.py','independent_refund.rs']:
            require(sha(previous/'runner'/name)==sha(Path(__file__).parent/name),'reused raw fixture/oracle/helper differs')
        reuse={'root':str(previous),'binding_sha256':sha(previous/'binding.json'),'results_sha256':sha(previous/'results.json'),'groups':GROUPS[:14]}
    mechanics=REPO/'tests/performance/independent_run.py';seed=REPO/'tests/performance/independent_channel_fixture.rs'
    require(sha(mechanics)==MECHANICS_SHA and sha(seed)==SEED_SHA,'review reused mechanics/seed changes first')
    root.mkdir(mode=0o700);(root/'runner').mkdir();(root/'importer').mkdir()
    for path in Path(__file__).parent.glob('independent_*'):
        if path.is_file():shutil.copy2(path,root/'runner'/path.name)
    shutil.copy2(mechanics,root/'runner/pg_mechanics.py');shutil.copy2(seed,root/'runner/independent_fixture.rs')
    for name in IMPORT_FILES:shutil.copy2(import_tree/name,root/'importer'/name)
    for name in ['services/core/src/core_schema.rs','services/core/schema/site_statistics.sql']:
        destination=root/name;destination.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(candidate/'source'/name,destination)
    for name in ['source-pins.json','compile-config.json','result.json','binaries.json']:shutil.copy2(candidate/name,root/('build-'+name if name=='result.json' else name))
    frozen={str(path.relative_to(root)):sha(path) for path in root.rglob('*') if path.is_file()}
    dump(root/'binding.json',{'candidate':str(candidate),'core_sha256':args.core_sha,'rlib_sha256':args.rlib_sha,'rlib_hash_origin':'binaries.json' if 'liblmm_core.rlib' in binaries else 'independently hashed from same completed build directory','import_sha256':args.import_sha,'frozen_files':frozen,'reuse_from':reuse,'resume_seals':resume,'seal_regression':args.seal_regression,'full_current':args.full_current,'legacy_revision':oracle.LEGACY_REVISION,'scope':'fresh28 actual Native HTTP; synthetic owner import; no upgrade/full source-archive CLI/provider signature acceptance'})
    env={k:v for k,v in os.environ.items() if not k.startswith(('LMM_','PG','SQLX_')) and k not in ('DATABASE_URL','SQL_DSN','HTTP_PROXY','HTTPS_PROXY','ALL_PROXY')}
    env.update(XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}',DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus',PYTHONDONTWRITEBYTECODE='1')
    suffix=secrets.token_hex(4);slice_name='lmmfinance_'+suffix+'.slice';scope='lmm-finance-independent-'+suffix
    unit=Path(env['XDG_RUNTIME_DIR'])/'systemd/user'/slice_name
    unit.write_text('[Slice]\nMemoryAccounting=yes\nMemoryMax=2G\nMemoryHigh=1536M\nMemorySwapMax=4G\nCPUQuota=100%\nTasksMax=256\n')
    command=['systemd-run','--user','--scope','--quiet','--unit='+scope,'--slice='+slice_name,sys.executable,'-B',str(root/'runner/independent_run.py'),'--root',str(root),'--worker']
    dump(root/'launch.json',{'argv':command,'slice':slice_name})
    try:
        subprocess.run(['systemctl','--user','daemon-reload'],env=env,check=True)
        code=subprocess.run(command,env=env).returncode
        with (root/'resources.txt').open('w') as out:subprocess.run(['systemctl','--user','show',slice_name,'-p','MemoryPeak','-p','MemoryMax','-p','MemoryHigh','-p','MemorySwapMax','-p','CPUQuotaPerSecUSec'],env=env,stdout=out)
    finally:
        subprocess.run(['systemctl','--user','stop',slice_name],env=env,check=True)
        unit.unlink(missing_ok=True);subprocess.run(['systemctl','--user','daemon-reload'],env=env,check=True)
    dump(root/'launch-result.json',{'exit_code':code,'unit_removed':not unit.exists()})
    dump(root/'hash-manifest.json',{str(path.relative_to(root)):sha(path) for path in root.rglob('*') if path.is_file() and path.name!='hash-manifest.json'})
    return code


if __name__ == '__main__':
    sys.exit(main())
