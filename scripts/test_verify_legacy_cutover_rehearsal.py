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


    def test_source_access_and_personal_account_access_have_separate_counts(self):
        source = [{'id': 11, 'status': 1, 'deleted': False},
                  {'id': 15, 'status': 1, 'deleted': False},
                  {'id': 16, 'status': 2, 'deleted': False},
                  {'id': 17, 'status': 1, 'deleted': True}]
        target = [{'user_id': 11, 'active': True, 'account_active': False},
                  {'user_id': 15, 'active': False, 'account_active': False},
                  {'user_id': 16, 'active': False, 'account_active': False},
                  {'user_id': 17, 'active': True, 'account_active': True},
                  {'user_id': 99, 'active': False, 'account_active': False}]
        result = audit.ordinary_account_access(source, target)
        self.assertFalse(result['passed'])
        self.assertEqual(result['different'], 3)
        self.assertEqual(result['source_enabled_target_user_inactive'], 1)
        self.assertEqual(result['source_enabled_target_personal_account_inactive'], 2)
        self.assertEqual(result['source_inactive_target_user_active'], 1)
        self.assertEqual(result['source_inactive_target_personal_account_active'], 1)
        self.assertEqual(audit.ordinary_account_access(source, target[:3])['missing'], 1)

    def test_enabled_disabled_and_deleted_ordinary_users_match_their_source(self):
        source = [{'id': i, 'status': status, 'deleted': deleted}
                  for i, status, deleted in [(1, 1, False), (2, 2, False), (3, 1, True)]]
        target = [{'user_id': r['id'], 'active': r['id'] == 1, 'account_active': r['id'] == 1}
                  for r in source]
        self.assertTrue(audit.ordinary_account_access(source, target)['passed'])


class PersonalAccountReconciliationTests(unittest.TestCase):
    archive = 'a' * 64

    def setUp(self):
        self.rows = [{'user_id': 9, 'account_id': 42}, {'user_id': 15, 'account_id': 2}]
        manifest = {'format_version': 2, 'scope': 'identity-only',
                    'source_archive_sha256': self.archive, 'mapping_sha256': 'b' * 64,
                    'source_target_id_map_sha256': audit.F.sha(
                        audit.F.IDENTITY.id_map_bytes(self.rows).encode())}
        self.receipt = {'manifest': manifest, 'source_archive_sha256': self.archive,
                        'mapping_sha256': manifest['mapping_sha256']}

    def test_committed_nonidentity_mapping_checks_actual_owner(self):
        accounts = audit.verified_personal_accounts([15, 9], self.rows[::-1], [self.receipt], self.archive)
        for field in ('account_id', 'owner_account_id', 'target_account_id',
                      'payer_account_id', 'recipient_account_id'):
            with self.subTest(field=field):
                expected = audit.map_source_accounts(
                    [{'id': 71, 'source_account_user_id': 9}], accounts, field)
                self.assertTrue(audit.compare_rows(expected, [{'id': 71, field: 42}])['passed'])
                self.assertFalse(audit.compare_rows(expected, [{'id': 71, field: 9}])['passed'])

    def test_same_ids_are_accepted_without_the_formal_identity_receipt(self):
        self.assertEqual(audit.verified_personal_accounts([9], [{'user_id': 9, 'account_id': 9}],
                                                          [], self.archive), {9: 9})
        with self.assertRaisesRegex(audit.F.Error, 'identity_import_receipt_required'):
            audit.verified_personal_accounts([9, 15], self.rows, [], self.archive)

    def test_missing_users_duplicate_accounts_and_unmapped_owners_are_rejected(self):
        for users, rows in (([9], self.rows), ([9, 15], [self.rows[0], self.rows[0]]),
                            ([9, 15], [self.rows[0], {'user_id': 15, 'account_id': 42}])):
            with self.assertRaisesRegex(audit.F.Error, 'identity_account_coverage_mismatch'):
                audit.verified_personal_accounts(users, rows, [self.receipt], self.archive)
        with self.assertRaisesRegex(audit.F.Error, 'source_account_user_unmapped'):
            audit.map_source_accounts([{'source_account_user_id': 404}], {9: 42}, 'owner_account_id')

    def test_receipts_must_match_native_accounts_archive_and_import_run(self):
        cases = [
            (self.rows, [self.receipt, self.receipt], 'identity_import_run_ambiguous'),
            ([{'user_id': 9, 'account_id': 2}, {'user_id': 15, 'account_id': 42}],
             [self.receipt], 'identity_map_file_hash_mismatch'),
            (self.rows, [{**self.receipt, 'source_archive_sha256': 'c' * 64}],
             'identity_import_run_binding_mismatch'),
            (self.rows, [{**self.receipt, 'mapping_sha256': 'c' * 64}],
             'identity_import_run_binding_mismatch'),
            (self.rows, [{**self.receipt, 'manifest': {**self.receipt['manifest'],
                                                       'source_archive_sha256': 'c' * 64}}],
             'identity_manifest_source_mismatch'),
        ]
        for rows, receipts, error in cases:
            with self.subTest(error=error), self.assertRaisesRegex(audit.F.Error, error):
                audit.verified_personal_accounts([9, 15], rows, receipts, self.archive)


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
