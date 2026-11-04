import copy
import unittest
from prepare_legacy_settlement_basis import prepare, import_sql, NAMESPACE, LEGACY_COMMIT
from prepare_wallet_history_snapshot import digest

class SettlementFactsTest(unittest.TestCase):
    def setUp(self):
        row={'id':1,'user_id':7,'trade_no':'synthetic-original-ref','status':'failed','payment_provider':'waffo_pancake','failure_reason_code':'checkout_timeout','money':'0.123456789123456789'}
        self.source={'source_namespace':NAMESPACE,'observed_at':100,'source_user_ids':[7],
            'tables':{'top_ups':[row],'wallet_credit_rebases':[],'credit_options':[]}}
        self.oracle={'source_commit':LEGACY_COMMIT,'source_orders_sha256':digest([row]),'network_isolated':True,'transaction_rolled_back':True,
            'binary_sha256':'b'*64,'capture_source_sha256':'c'*64,'original_archive_sha256':'d'*64,
            'source_tables_sha256':digest(self.source['tables']),'source_user_ids_sha256':digest([7]),
            'results':[{'id':1,'user_id':7,'quota':123,'expected_micros':100000,'rejected':False}]}
    def run_plan(self,source=None,oracle=None):
        return prepare(source or self.source,oracle or self.oracle,archive_sha256='a'*64,expected_rows=1)
    def test_preserves_original_reference_quote_and_non_authority(self):
        m=self.run_plan();self.assertEqual(m['tables']['top_ups'],self.source['tables']['top_ups'])
        self.assertFalse(m['runtime_settlement_available']);self.assertEqual(m['balance_effect'],'none')
        self.assertIn('opening_source_mismatch',import_sql(m));self.assertNotIn('post_ledger',import_sql(m))
    def test_rejected_quota_stays_rejected_and_orphan_stays_unmapped(self):
        s=copy.deepcopy(self.source);s['source_user_ids']=[8]
        o=copy.deepcopy(self.oracle);o['source_user_ids_sha256']=digest([8]);o['results'][0].update(quota=0,rejected=True)
        m=self.run_plan(s,o);self.assertEqual(m['original_quota_rejected_count'],1);self.assertEqual(m['unmapped_order_ids'],[1])
    def test_tampered_scope_rows_owners_and_failed_observations_are_rejected(self):
        for mutate in [lambda o:o.update(source_orders_sha256='e'*64),lambda o:o.update(source_tables_sha256='e'*64),lambda o:o.update(network_isolated=False),lambda o:o['results'][0].update(user_id=8),lambda o:o['results'][0].update(quota=0),lambda o:o['results'][0].update(rejected=True)]:
            o=copy.deepcopy(self.oracle);mutate(o)
            with self.assertRaises(ValueError):self.run_plan(oracle=o)
        s=copy.deepcopy(self.source);s['tables']['top_ups'][0]['money']='0.1'
        with self.assertRaisesRegex(ValueError,'source_binding'):self.run_plan(source=s)

if __name__=='__main__':unittest.main()
