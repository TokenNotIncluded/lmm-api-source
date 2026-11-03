# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Cash field evidence is distinct from a quote, internal credit, or acceptance."""
import json
import os
import subprocess
import unittest
from legacy_acquisition_cash_audit import audit_sql


@unittest.skipUnless(os.environ.get('LMM_ACQUISITION_CASH_TEST_DSN'), 'isolated database required')
class CashEvidenceTests(unittest.TestCase):
    def sql(self, sql):
        return subprocess.run(['psql', os.environ['LMM_ACQUISITION_CASH_TEST_DSN'], '-X', '-qAt', '-v', 'ON_ERROR_STOP=1'], input=sql, text=True, capture_output=True, check=True).stdout

    def test_deduplication_quotes_internal_credit_and_missing_cash_fields(self):
        self.sql('''CREATE SCHEMA cash_fixture;
CREATE TABLE cash_fixture.top_ups(user_id bigint,trade_no text,status text,complete_time bigint,settled_amount_micros bigint,settlement_currency text,payment_method text,payment_provider text);
CREATE TABLE cash_fixture.subscription_orders(id bigint,user_id bigint,trade_no text,status text,complete_time bigint,expected_amount_micros bigint,settlement_currency text,payment_method text,payment_provider text);
CREATE TABLE cash_fixture.subscription_payment_events(subscription_order_id bigint,created_time bigint,settlement_amount_micros bigint,settlement_currency text,payment_provider text);
CREATE TABLE cash_fixture.finance_ledger_entries(user_id bigint,occurred_at bigint,amount_micros bigint,currency text,payment_method text,payment_provider text,direction int,entry_type text,source_type text);
INSERT INTO cash_fixture.subscription_orders VALUES
(1,1,'mirror','success',100,100,'USD','stripe','stripe'),
(2,1,'quote','success',100,100,'USD','stripe','stripe'),
(3,1,'internal','success',100,100,'USD','balance','stripe');
INSERT INTO cash_fixture.subscription_payment_events VALUES(1,100,100,'USD','stripe');
INSERT INTO cash_fixture.top_ups VALUES
(1,'mirror','success',100,100,'USD','stripe','stripe'),
(1,'cash','success',100,100,' usd ','stripe','stripe'),
(1,'amount','success',100,0,'USD','stripe','stripe'),
(1,'time','success',0,100,'USD','stripe','stripe'),
(1,'currency','success',100,100,'US','stripe','stripe'),
(1,'pending','pending',100,100,'USD','stripe','stripe'),
(1,'source','success',100,100,'USD','',''),
(1,'ldc','success',100,100,'USD','ldc','epay');
INSERT INTO cash_fixture.finance_ledger_entries VALUES
(1,100,100,'USD','stripe','stripe',-1,'revenue','refund'),
(1,100,100,'USD','stripe','stripe',1,'revenue','refund'),
(1,100,100,'USD','stripe','stripe',0,'revenue','refund');''')
        try:
            result = json.loads(self.sql(audit_sql('cash_fixture')))
            self.assertEqual((result['event_rows'], result['cash_evidence_rows'], result['internal_credit_rows'], result['unproven_rows']), (12, 4, 2, 6))
            counts = {(r['kind'],r['evidence']):r['rows'] for r in result['groups']}
            self.assertEqual(counts[('subscription_order','subscription_cash_unproven')], 1)
            self.assertEqual(counts[('refund','cash_evidence_present')], 2)
            self.assertEqual(counts[('refund','invalid_refund_direction')], 1)
            self.assertEqual(self.sql('SELECT count(*) FROM cash_fixture.top_ups').strip(), '8')
            self.assertNotIn('user_id', json.dumps(result))
        finally:
            self.sql('DROP SCHEMA cash_fixture CASCADE')


if __name__ == '__main__':
    unittest.main()
