# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import copy
import unittest
import prepare_hero_sms_history_snapshot as h


class SmsHistoryTests(unittest.TestCase):
    def setUp(self):
        self.data = {'source_namespace':h.NAMESPACE, 'observed_at':20, 'source_user_ids':[7],
            'tables':{h.ORDERS:[{'id':'original-order','user_id':7,'status':'completed','created_at':10,
                'reserved_quota':100,'refunded_quota':10,'charge_quota':90,
                'provider_price_cny':'1.000000000000000001','phone_ciphertext':'original-encrypted-phone'}],
                h.LEDGERS:[{'id':1,'order_id':'original-order','user_id':7,'created_at':10,'entry_type':'reserve','amount_quota':-100},
                           {'id':2,'order_id':'original-order','user_id':7,'created_at':11,'entry_type':'refund','amount_quota':10}]}}

    def build(self, data=None):
        return h.prepare(data or self.data,expected_orders=1,expected_ledgers=2,archive_sha256='a'*64)

    def test_preserves_every_original_field_and_encrypted_payload_without_enabling_calls(self):
        m=self.build()
        self.assertEqual(m['tables'],self.data['tables'])
        self.assertEqual(m['balance_effect'],'none')
        self.assertFalse(m['native_supplier_operations_enabled'])
        self.assertIn('hero_sms_history_import_conflict',h.import_sql(m))

    def test_unmapped_history_is_preserved_not_assigned_to_another_user(self):
        d=copy.deepcopy(self.data);d['source_user_ids']=[]
        self.assertEqual(self.build(d)['unmapped_order_ids'],['original-order'])

    def test_pending_holds_and_individual_money_or_ownership_errors_are_rejected(self):
        for mutate in [lambda d:d['tables'][h.ORDERS][0].update(status='pending'),
                       lambda d:d['tables'][h.ORDERS][0].update(charge_quota=89),
                       lambda d:d['tables'][h.LEDGERS][0].update(user_id=8),
                       lambda d:d['tables'][h.LEDGERS][1].update(amount_quota=11),
                       lambda d:d['tables'][h.LEDGERS][1].update(id=1),
                       lambda d:d['tables'][h.LEDGERS][0].update(order_id='absent'),
                       lambda d:d['tables'][h.ORDERS][0].update(reserved_quota=True),
                       lambda d:d['tables'][h.LEDGERS][0].update(created_at=21)]:
            data=copy.deepcopy(self.data);mutate(data)
            with self.assertRaises(ValueError):self.build(data)


if __name__=='__main__':unittest.main()
