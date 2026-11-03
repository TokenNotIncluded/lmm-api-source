# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""The reconciliation must reject financial drift, missing rows and ambiguity."""
from fractions import Fraction
import unittest
import verify_legacy_cutover_rehearsal as audit


class ComparisonTests(unittest.TestCase):
    def test_exact_rows_ignore_order_but_detect_missing_extra_and_changed_money(self):
        original=[{'id':1,'balance':-7},{'id':2,'balance':100}]
        self.assertTrue(audit.compare_rows(original,list(reversed(original)))['passed'])
        changed=[{'id':1,'balance':-8},{'id':3,'balance':100}]
        result=audit.compare_rows(original,changed)
        self.assertEqual((result['missing'],result['extra'],result['different'],result['passed']),(1,1,1,False))

    def test_boolean_and_integer_cannot_replace_each_other(self):
        self.assertFalse(audit.compare_rows([{'id':1,'enabled':True}],[{'id':1,'enabled':1}])['passed'])

    def test_duplicate_source_or_target_ids_do_not_disappear_in_a_map(self):
        rows=[{'id':1,'amount':10},{'id':1,'amount':20}]
        for source,target in ((rows,[]),([],rows)):
            with self.assertRaisesRegex(audit.F.Error,'duplicate_comparison_identifiers'):
                audit.compare_rows(source,target)

    def test_exact_fraction_rounding_never_uses_binary_float(self):
        self.assertEqual([audit.normalized_quota(v,Fraction(2)) for v in (0,1,2,3)], [0,1,1,2])
        divisor=Fraction('7.123457')
        self.assertEqual(audit.normalized_quota(7123457000000001,divisor),1000000000000000)
        for raw,divisor in ((-1,Fraction(1)),(True,Fraction(1)),(1,Fraction(0))):
            with self.assertRaisesRegex(audit.F.Error,'invalid_quota_conversion'):
                audit.normalized_quota(raw,divisor)


class SmsReconciliationTests(unittest.TestCase):
    def setUp(self):
        self.orders=[{'id':'one','user_id':1,'status':'completed','reserved_quota':100,
                      'refunded_quota':40,'charge_quota':60},
                     {'id':'two','user_id':2,'status':'cancelled','reserved_quota':100,
                      'refunded_quota':100,'charge_quota':100}]
        self.ledgers=[{'order_id':o['id'],'user_id':o['user_id'],'entry_type':kind,
                       'amount_quota':amount} for o in self.orders
                      for kind,amount in [('reserve',-100),('refund',o['refunded_quota'])]]

    def test_partial_completed_refund_and_full_cancellation(self):
        self.assertTrue(audit.hero_sms_consistency(self.orders,self.ledgers)['passed'])

    def test_opposite_order_errors_cannot_cancel_at_aggregate_level(self):
        self.ledgers[1]['amount_quota']+=1
        self.ledgers[3]['amount_quota']-=1
        self.assertEqual(sum(r['amount_quota'] for r in self.ledgers),-60)
        result=audit.hero_sms_consistency(self.orders,self.ledgers)
        self.assertEqual(result['refund_mismatches'],2)
        self.assertEqual(result['net_debit_mismatches'],2)
        self.assertFalse(result['passed'])

    def test_owner_orphan_and_invalid_entry_are_not_ignored(self):
        self.ledgers[0]['user_id']=9
        self.ledgers.append({'order_id':'absent','user_id':1,'entry_type':'refund','amount_quota':10})
        self.ledgers.append({'order_id':'one','user_id':1,'entry_type':'refund','amount_quota':-10})
        result=audit.hero_sms_consistency(self.orders,self.ledgers)
        self.assertEqual([result[k] for k in ('wrong_ledger_owners','orphan_ledger_rows','invalid_ledger_entries')],[1,1,1])
        self.assertFalse(result['passed'])

    def test_charge_and_cancellation_fields_must_match_settlement(self):
        self.orders[0]['charge_quota']=61
        self.orders[1]['refunded_quota']=99
        self.ledgers[3]['amount_quota']=99
        result=audit.hero_sms_consistency(self.orders,self.ledgers)
        self.assertEqual(result['completed_charge_mismatches'],1)
        self.assertEqual(result['cancelled_or_failed_debits'],1)
        self.assertFalse(result['passed'])

    def test_uncovered_financial_features_remain_a_gate(self):
        checks={'source_sms':{'passed':True}}
        self.assertEqual(audit.reconciliation_status(checks,{}),'SCOPED_PASS')
        for key in ('checkins','discount_codes','referrals','hero_sms_orders'):
            self.assertEqual(audit.reconciliation_status(checks,{key:1}),'SCOPED_PASS_WITH_GAPS')
        self.assertEqual(audit.reconciliation_status({'bad':{'passed':False}},{'checkins':1}),'FAILED')


if __name__=='__main__':
    unittest.main()
