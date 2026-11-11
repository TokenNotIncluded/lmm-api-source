import copy
import unittest

import test_prepare_legacy_settlement_basis as fixture
from prepare_legacy_settlement_basis import prepare as facts_prepare
from prepare_legacy_payment_bindings import prepare, draft, SCOPE
from prepare_wallet_history_snapshot import digest


def synthetic(mixed_openings=False):
    f = fixture.SettlementFactsTest(); f.setUp()
    source = f.source
    row = source['tables']['top_ups'][0]
    row.update(provider_store_id='STO_original', provider_product_id='PROD_original', settlement_currency='USD')
    second = dict(row, id=2, trade_no='synthetic-second-ref')
    if mixed_openings:
        second['user_id'] = 8
        source['source_user_ids'] = [7,8]
    source['tables']['top_ups'].append(second)
    oracle = f.oracle
    oracle['results'][0].update(quota=7654321, expected_micros=1234567)
    oracle['results'].append(dict(oracle['results'][0], id=2, user_id=second['user_id']))
    oracle['source_user_ids_sha256'] = digest(source['source_user_ids'])
    oracle.update(source_orders_sha256=digest(source['tables']['top_ups']), source_tables_sha256=digest(source['tables']))
    facts = facts_prepare(source, oracle, archive_sha256='a'*64, expected_rows=2)
    review = dict(scope=SCOPE, facts_sha256=digest(facts), opening_migration_id='synthetic-opening', orders=[])
    for r in source['tables']['top_ups']:
        review['orders'].append(dict(source_order_id=r['id'], source_row_sha256=digest(r), user_id=r['user_id'],
            account_id=r['user_id'], opening_entity_key=f"legacy.users/{r['user_id']}" if mixed_openings and r['user_id']==8 else f"user:{r['user_id']}", channel_id=9, merchant='synthetic-merchant', environment='test', currency='USD',
            store_id='STO_original', source_product_id='PROD_original'))
    return facts, review


class BindingsTest(unittest.TestCase):
    def test_nullable_discount_matches_original_go_integer_without_rewriting_source(self):
        for value in (None,0,19):
            with self.subTest(value=value):
                facts,review=synthetic()
                row=facts['tables']['top_ups'][0]
                row['discount_code_id']=value
                original=copy.deepcopy(row)
                review['facts_sha256']=digest(facts)
                review['orders'][0]['source_row_sha256']=digest(row)
                plan=prepare(facts,review)
                self.assertEqual(plan['bases'][0]['discount_code_id'],0 if value is None else value)
                self.assertEqual(row,original)
                self.assertEqual(plan['bases'][0]['source_row_sha256'],digest(original))
        facts,review=synthetic()
        facts['tables']['top_ups'][0].pop('discount_code_id',None)
        review['facts_sha256']=digest(facts)
        review['orders'][0]['source_row_sha256']=digest(facts['tables']['top_ups'][0])
        self.assertEqual(prepare(facts,review)['bases'][0]['discount_code_id'],0)
        for value in (True,-1,'0',1.5,2**63):
            facts,review=synthetic()
            facts['tables']['top_ups'][0]['discount_code_id']=value
            review['facts_sha256']=digest(facts)
            review['orders'][0]['source_row_sha256']=digest(facts['tables']['top_ups'][0])
            with self.subTest(invalid_type=type(value).__name__),self.assertRaisesRegex(ValueError,'discount_invalid'):
                prepare(facts,review)

    def test_frozen_quote_and_append_review_are_stable(self):
        facts, review = synthetic()
        first = copy.deepcopy(review);first['orders'] = first['orders'][:1]
        a = prepare(facts, first); b = prepare(facts, review)
        self.assertEqual(a['bases'][0], b['bases'][0])
        self.assertEqual(a['unselected_order_ids'], [2])
        self.assertEqual((a['bases'][0]['amount_micros'], a['bases'][0]['credit_units']), (1234567, 7654321))
        self.assertEqual(a['balance_effect'], 'none')

    def test_changed_source_or_original_terms_cannot_be_reviewed_away(self):
        facts, review = synthetic()
        mutations = [lambda r:r.update(facts_sha256='0'*64),
            lambda r:r['orders'][0].update(source_row_sha256='0'*64),
            lambda r:r['orders'][0].update(currency='EUR'),
            lambda r:r['orders'][0].update(store_id='STO_current'),
            lambda r:r['orders'][0].update(source_product_id='PROD_current'),
            lambda r:r['orders'][0].update(account_id=True),
            lambda r:r['orders'][0].update(opening_entity_key='legacy.users/8')]
        for change in mutations:
            r=copy.deepcopy(review);change(r)
            with self.assertRaises(ValueError):prepare(facts,r)

    def test_draft_has_no_install_authority_and_gaps_stay_visible(self):
        facts, review = synthetic()
        facts['source_user_ids'] = []
        report = draft(facts)
        self.assertEqual(report['orders'], [])
        self.assertEqual(len(report['gaps']), 2)
        self.assertFalse(report['installable'])
        with self.assertRaises(ValueError):prepare(facts, review)
        facts, review = synthetic()
        facts['tables']['top_ups'][0]['payment_provider']='epay'
        review['facts_sha256']=digest(facts)
        with self.assertRaisesRegex(ValueError, 'ineligible'):prepare(facts, review)



# Explicit owner-only integration; the caller creates a fresh private database
# in a network namespace with loopback only, never the restored business DB.
import os
from pathlib import Path
import subprocess
from urllib.parse import urlparse
from prepare_legacy_payment_bindings import import_sql
from prepare_legacy_settlement_basis import import_sql as facts_sql
from prepare_wallet_history_snapshot import literal


@unittest.skipUnless(os.environ.get('LMM_BINDING_TEST_DATABASE_URL'), 'explicit fresh isolated PostgreSQL required')
class BindingsPostgresTest(unittest.TestCase):
    def test_owner_install_replay_conflict_and_no_money(self):
        url = os.environ['LMM_BINDING_TEST_DATABASE_URL']
        parsed = urlparse(url)
        self.assertEqual(parsed.hostname, '127.0.0.1')
        self.assertTrue(parsed.path.startswith('/lmm_binding_test_'))
        self.assertFalse(parsed.query)
        def sql(text, success=True):
            result = subprocess.run(['psql', '-X', '-At', '-v', 'ON_ERROR_STOP=1', url], input=text,
                                    text=True, capture_output=True)
            if success:
                self.assertEqual(result.returncode, 0, result.stderr)
            else:
                self.assertNotEqual(result.returncode, 0)
            return result
        self.assertEqual(sql("SELECT count(*) FROM pg_namespace WHERE nspname LIKE 'core_%'").stdout.strip(), '0')
        root = Path(__file__).resolve().parents[1]
        binary = root/'services/core/target/debug/lmm-core'
        init = subprocess.run([str(binary), '--legacy-env', 'init-db'], capture_output=True, text=True,
                              env={'PATH':os.environ['PATH'], 'HOME':os.environ['HOME'], 'LMM_CORE_DATABASE_URL':url})
        self.assertEqual(init.returncode, 0, init.stderr)
        facts, review = synthetic(mixed_openings=True)
        opening = dict(migration_id='synthetic-opening', source_entity_key='user:7',
                       source_archive_sha256='a'*64, mapping_sha256='b'*64, source_row_sha256='c'*64,
                       observed_at=100, account_id=7, available_units=0, reserved_units=0, affiliate_units=0)
        sql("INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES(7,'personal'),(8,'personal');"
            "INSERT INTO core_identity.users(id,personal_account_id) VALUES(7,7),(8,8);"
            "BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;"
            "SELECT core_billing.import_opening_balance("+literal(opening)+"::jsonb);"
            "SELECT core_billing.import_opening_balance("+literal(dict(opening, account_id=8, source_entity_key='legacy.users/8'))+"::jsonb);COMMIT;")
        first = copy.deepcopy(review);first['orders'] = first['orders'][:1]
        single = prepare(facts, first)
        missing = sql(import_sql(single), success=False)
        self.assertIn('installed_facts_mismatch', missing.stderr)
        sql(facts_sql(facts))
        before = sql("SELECT count(*) FROM core_billing.ledger_journals").stdout
        sql(import_sql(single));sql(import_sql(single))
        wrong_owner = copy.deepcopy(review)
        wrong_owner['orders'][1]['account_id'] = 999
        result = sql(import_sql(prepare(facts, wrong_owner)), success=False)
        self.assertIn('original opening and personal owner', result.stderr)
        self.assertEqual(sql("SELECT count(*) FROM core_payments.legacy_bases").stdout.strip(), '1')
        sql(import_sql(prepare(facts, review)))
        self.assertEqual(sql("SELECT count(*) FROM core_payments.legacy_bases").stdout.strip(), '2')
        self.assertEqual(sql("SELECT count(*) FROM core_billing.ledger_journals").stdout, before)
        self.assertEqual(sql("SELECT b.balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.owner_account_id=7 AND a.bucket='wallet'").stdout.strip(), '0')
        changed = copy.deepcopy(first);changed['orders'][0]['merchant']='another-merchant'
        result = sql(import_sql(prepare(facts, changed)), success=False)
        self.assertIn('replay_conflict', result.stderr)
        runtime_role = 'binding_runtime_' + digest(parsed.path)[:12]
        sql(f'CREATE ROLE {runtime_role} NOLOGIN; GRANT USAGE ON SCHEMA core_payments TO {runtime_role}; GRANT SELECT ON core_payments.legacy_bases TO {runtime_role};')
        result = sql(f'SET ROLE {runtime_role};'+import_sql(single), success=False)
        self.assertIn('installation_owner_required', result.stderr)
        self.assertEqual(sql("SELECT count(*) FROM core_payments.legacy_bases").stdout.strip(), '2')
        sql(f'DROP OWNED BY {runtime_role}; DROP ROLE {runtime_role};')

if __name__ == '__main__':unittest.main()
