import copy
import unittest
import legacy_pancake_subscriptions as W

class OriginalTests(unittest.TestCase):
    def fixture(self, recurring=False, status='pending'):
        snapshot={'title':'original','total_amount':100,'upgrade_group':''}
        terms={'title':'original','price_units':'20','duration_seconds':0 if recurring else 86400,
               'duration_months':1 if recurring else 0,'reset_period':'grant','reset_seconds':0,
               'limit_units':'100','models':['*'],'groups':['*'],'max_purchases_per_user':1,'enabled':True}
        row={'id':1,'user_id':7,'trade_no':'WAFFO_PANCAKE_SUB-original','status':status,
             'payment_provider':'waffo_pancake','expected_amount_micros':1230000,'settlement_currency':'USD',
             'provider_product_id':'product','provider_store_id':'store','provider_subscription_id':'ORD_original',
             'plan_id':5,'created_at':1700000000,'plan_snapshot':snapshot}
        price={'plan_id':'9','plan_revision':'2','channel_id':'10','currency':'USD','amount_minor':'123',
               'product_id':'product','store_id':'store','provider_plan_reference':'5','product_type':'subscription' if recurring else 'one_time',
               'billing_period':'monthly' if recurring else '', 'return_url':'https://example.invalid/return',
               'checkout_region':'global','checkout_language':'en','tax_category':'saas'}
        b={'source_order_id':1,'source_row_sha256':W.F.digest(row),'account_id':70,
           'snapshot_field':'plan_snapshot','plan':{'id':9,'revision':2,'archived':False,'terms':terms},
           'terms_review':{'decision':'equivalent-no-unsupported-policy','source_snapshot_sha256':W.F.digest(snapshot),
                          'core_terms_sha256':W.F.digest(terms)},
           'binding':{'merchant':'merchant','environment':'live','price':price},'provider_order_id':'ORD_original'}
        if status=='success':b.update(opening_grant_reference='legacy-sub:opening:1',last_paid_period_end=1702592000)
        source={'source_archive_sha256':'a'*64,'orders':[row]}
        review={'source_archive_sha256':'a'*64,'decision':'approve-original-subscription-bindings','orders':[b]}
        return source,review,[{'id':7,'personal_account_id':70}]
    def build(self, value):return W.build_records(*value)
    def test_pending_preserves_exact_original(self):
        f=self.fixture();a=self.build(f)[0];b=self.build(copy.deepcopy(f))[0]
        self.assertEqual(a,b);self.assertEqual(a['binding']['price']['amount_minor'],'123')
        self.assertEqual(a['reference'],'WAFFO_PANCAKE_SUB-original')
        self.assertEqual(a['legacy_source']['source_status'],'pending')
        self.assertNotIn('subscription_id',a)
    def test_unknown_cutoff_refuses_success(self):
        f=self.fixture(recurring=True,status='success');del f[1]['orders'][0]['last_paid_period_end']
        with self.assertRaises(W.F.IDENTITY.MappingError):self.build(f)
    def test_source_row_changed_refuses(self):
        f=self.fixture();f[0]['orders'][0]['expected_amount_micros']=9990000
        with self.assertRaises(W.F.IDENTITY.MappingError):self.build(f)
    def test_noncent_original_never_rounds(self):
        f=self.fixture();r=f[0]['orders'][0];r['expected_amount_micros']=1230001
        f[1]['orders'][0]['source_row_sha256']=W.F.digest(r)
        with self.assertRaises(W.F.IDENTITY.MappingError):self.build(f)
    def test_cross_account_review_refuses(self):
        f=self.fixture();f[1]['orders'][0]['account_id']=71
        with self.assertRaises(W.F.IDENTITY.MappingError):self.build(f)
    def test_unsupported_group_transition_refuses(self):
        f=self.fixture();r=f[0]['orders'][0];r['plan_snapshot']['upgrade_group']='vip'
        b=f[1]['orders'][0];b['source_row_sha256']=W.F.digest(r);b['terms_review']['source_snapshot_sha256']=W.F.digest(r['plan_snapshot'])
        with self.assertRaises(W.F.IDENTITY.MappingError):self.build(f)
    def test_recurring_original_requires_reviewed_cutoff(self):
        result=self.build(self.fixture(recurring=True,status='success'))[0]
        self.assertEqual(result['legacy_source']['last_paid_period_end'],1702592000)
        self.assertEqual(result['opening_grant_reference'],'legacy-sub:opening:1')
    def test_provider_currency_cash_mismatch_refuses(self):
        f=self.fixture();f[1]['orders'][0]['binding']['price']['currency']='CNY'
        with self.assertRaises(W.F.IDENTITY.MappingError):self.build(f)

    def test_missing_material_retains_unknown_without_fabricated_quote(self):
        f=self.fixture();row=f[0]['orders'][0]
        for k in ('expected_amount_micros','settlement_currency','provider_product_id','provider_store_id'):row[k]=None
        row['plan_snapshot']='';row['create_time']=row.pop('created_at')
        b=f[1]['orders'][0];b['source_row_sha256']=W.F.digest(row);b['disposition']='retain-unknown';del b['plan'];del b['binding']
        r=self.build(f)[0]
        self.assertEqual(r['disposition'],'UNKNOWN');self.assertNotIn('order_id',r);self.assertNotIn('binding',r)
        self.assertEqual(r['unknown']['reference'],row['trade_no']);self.assertEqual(r['unknown']['source_status'],'pending')
    def test_all_history_37_preserves_18_bound_and_19_unknown(self):
        source,review,users=self.fixture();row=source['orders'][0];b=review['orders'][0]
        source['orders']=[];review['orders']=[]
        for i in range(1,38):
            r=copy.deepcopy(row);r['id']=i;r['trade_no']='WAFFO_PANCAKE_SUB-original-'+str(i)
            q=copy.deepcopy(b);q['source_order_id']=i
            if i>18:
                for k in ('expected_amount_micros','settlement_currency','provider_product_id','provider_store_id'):r[k]=None
                r['plan_snapshot']='';q['disposition']='retain-unknown';del q['plan'];del q['binding']
            q['source_row_sha256']=W.F.digest(r);source['orders'].append(r);review['orders'].append(q)
        result=W.build_records(source,review,users)
        self.assertEqual(len(result),37);self.assertEqual(sum(r.get('disposition')=='UNKNOWN' for r in result),19)
        self.assertEqual(sum('order_id' in r for r in result),18)
    def test_complete_original_cannot_be_hidden_as_unknown(self):
        f=self.fixture();f[1]['orders'][0]['disposition']='retain-unknown'
        with self.assertRaises(W.F.IDENTITY.MappingError):self.build(f)

    def test_true_policy_pending_is_durable_without_a_payable_projection(self):
        source, review, users = self.fixture()
        row = source['orders'][0]
        row['plan_snapshot']['allow_wallet_overflow'] = True
        binding = review['orders'][0]
        binding.update(source_row_sha256=W.F.digest(row), disposition='retain-unknown')
        record = W.build_records(source, review, users)[0]
        self.assertEqual(record['unknown']['source_status'], 'pending')
        self.assertEqual(record['unknown']['missing_material'], ['unsupported_source_overflow_policy'])
        self.assertEqual(record['unknown']['source_snapshot'], row['plan_snapshot'])
        self.assertNotIn('plan', record)
        self.assertNotIn('order_id', record)
        self.assertNotIn('binding', record)
        row['plan_snapshot']['allow_wallet_overflow'] = False
        binding['source_row_sha256'] = W.F.digest(row)
        with self.assertRaisesRegex(W.F.IDENTITY.MappingError, 'original_complete_material_cannot_be_discarded'):
            W.build_records(source, review, users)

    def test_true_policy_paid_history_requires_exact_terminal_entitlement(self):
        source, review, users = self.fixture(status='success')
        row = source['orders'][0]
        row.update(user_subscription_id=1, provider_subscription_id='')
        row['plan_snapshot'].update(allow_wallet_overflow=True, waffo_pancake_product_type='one_time')
        binding = review['orders'][0]
        binding.update(source_row_sha256=W.F.digest(row), disposition='retain-unknown', projection='historical-only')
        end = binding['last_paid_period_end']
        entitlement = {'source_subscription_id': 1, 'source_plan_id': 5,
            'source_user_id': 7, 'account_id': 70, 'status': 'expired', 'end_time': end,
            'source_row_sha256': 'b' * 64, 'plan_row_sha256': 'c' * 64}
        record = W.build_records(source, review, users, observed_at=end, source_entitlements=[entitlement])[0]
        self.assertEqual(record['unknown']['state'], 'UNKNOWN')
        self.assertEqual(record['unknown']['source_status'], 'success')
        self.assertEqual(record['unknown']['projection'], 'historical-only')
        self.assertEqual(record['unknown']['formal_grant']['reference'], 'legacy-sub:opening:1')
        self.assertNotIn('plan', record)
        for changes in ({'status': 'active'}, {'account_id': 71}, {'source_plan_id': 6}, {'end_time': end + 1}):
            with self.subTest(changes=changes), self.assertRaises(W.F.IDENTITY.MappingError):
                W.build_records(source, review, users, observed_at=end,
                    source_entitlements=[{**entitlement, **changes}])
        for cutoff in (None, True, end - 1):
            with self.subTest(cutoff=cutoff), self.assertRaises(W.F.IDENTITY.MappingError):
                W.build_records(source, review, users, observed_at=cutoff, source_entitlements=[entitlement])

if __name__=='__main__':unittest.main()
