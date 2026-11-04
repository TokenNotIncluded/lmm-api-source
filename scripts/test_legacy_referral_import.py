import unittest
import legacy_referral_export as export
import legacy_referral_import as imp


class ReferralImport(unittest.TestCase):
    def fixture(self):
        policy = '{"min_top_up_amounts":{"USD":"10"},"reward_quota":100,"max_reward_quota":100,"min_top_up_quota":100}'
        doc = {'users': [{'id': 1, 'inviter_id': 2, 'referral_first_top_up_id': 0, 'stripe_customer': ''},
                         {'id': 2, 'inviter_id': 0, 'referral_first_top_up_id': 0, 'stripe_customer': ''}],
               'rewards': [{'id': 7, 'invitee_id': 1, 'inviter_id': 2, 'top_up_id': None,
                            'status': 'earned', 'stage': 'awaiting_first_top_up', 'policy_snapshot': policy,
                            'revoked_quota': 0, 'penalty_quota': 0, 'penalty_percent': 20,
                            'max_penalty_quota': 100, 'quota': 100, 'registration_quota': 100}],
               'ledger': [{'reward_id': 7}], 'referral_rebases': [], 'wallet_rebases': []}
        review = {'document_sha256': imp.digest(doc), 'archive_sha256': 'a'*64, 'migration_id': 'synthetic',
                  'unit': 'credit_500k_usd', 'unit_evidence': 'synthetic explicit audited unit',
                  'users': [{'id': uid, 'prior_cash': False, 'evidence': 'synthetic unpaid user',
                             'affiliate_history_credit': 100} for uid in [1, 2]],
                  'pending': [{'id': 7, 'source_policy_sha256': imp.digest(policy), 'evidence': 'synthetic source conversion',
                               **{k: 100 for k in imp.TERMS}}]}
        return doc, review

    def test_complete_raw_source_preserved_without_registration_reissue(self):
        doc, review = self.fixture()
        sql = imp.plan(doc, review, 'a'*64, 'synthetic')
        self.assertIn('core_referrals.source', sql)
        self.assertIn('source_entity_key', sql)
        self.assertNotIn('post_ledger', sql)
        self.assertNotIn('core_referrals.delta', sql)

    def test_unknown_unit_partial_review_or_rebase_reject(self):
        for mutation in ['unit', 'cohort', 'rebase', 'policy']:
            with self.subTest(mutation=mutation):
                doc, review = self.fixture()
                if mutation == 'unit': review['unit_evidence'] = ''
                elif mutation == 'cohort': review['pending'] = []
                elif mutation == 'rebase':
                    doc['referral_rebases'] = [{'reward_id': 7}]
                    review['document_sha256'] = imp.digest(doc)
                else: review['pending'][0]['source_policy_sha256'] = 'x'
                with self.assertRaises(ValueError): imp.plan(doc, review, 'a'*64, 'synthetic')

    def test_export_is_readonly_and_schema_identifier_rejected(self):
        self.assertIn('BEGIN READ ONLY', export.export_sql())
        with self.assertRaises(ValueError): export.export_sql('public; DELETE FROM users')


class FrozenReview(unittest.TestCase):
    def fixture(self):
        from legacy_referral_review import frozen_review
        doc, _ = ReferralImport().fixture()
        doc['format_version'] = 1
        doc['source_archive_sha256'] = 'a'*64
        doc['options'] = {'QuotaPerUnit':'500000','QuotaForInviter':'100','ReferralMinTopUpQuota':'100',
                          'ReferralMinTopUpAmounts':'{"USD":"10"}'}
        doc['ledger_entries'] = doc.pop('ledger')
        doc['referral_credit_rebases'] = doc.pop('referral_rebases')
        doc['wallet_credit_rebases'] = [{'plan': {'user_ids':[2], 'include_affiliate':True,
            'referral_bases':[], 'usd_credit_conversion':500000}}]
        doc.pop('wallet_rebases')
        for user in doc['users']: user['aff_history'] = 100
        return doc, {'pending_prior_cash':[{'invitee_id':1,'qualifying_prior_count':0}]}, frozen_review

    def test_affected_inviter_with_complete_plan_and_no_named_base_keeps_delta(self):
        doc, prior, build = self.fixture()
        review = build(doc, prior, 'synthetic')
        sql = imp.plan(doc, review, 'a'*64, 'synthetic')
        self.assertIn('core_referrals.contracts', sql)
        self.assertEqual(review['pending'][0]['reward_credit'], 100)

    def test_missing_wallet_baselines_or_prior_cohort_reject(self):
        for field in ['baseline','cohort']:
            doc, prior, build = self.fixture()
            if field=='baseline': doc['wallet_credit_rebases'][0]['plan']['referral_bases']=None
            else: prior['pending_prior_cash']=[]
            with self.assertRaises(ValueError): build(doc, prior, 'synthetic')

if __name__ == '__main__':
    unittest.main()
