"""Exact source resolution for the pending fresh66 payment quote component."""
import unittest

import financial_import as F

I = F.IDENTITY


class CreditQuoteSchemaMappingTests(unittest.TestCase):
    def test_credit_quote_alias_resolves_exactly_two_files_in_compiled_order(self):
        source, constant, expression = I.REVIEWED_SCHEMA_ALIASES['crate::payments::CREDIT_QUOTE_SCHEMA']
        self.assertEqual((source, constant), ('services/core/src/payments.rs', 'CREDIT_QUOTE_SCHEMA'))
        self.assertEqual(expression, 'concat!(include_str!("../schema/payment_credit_quotes.sql"),"\\n",'
                                    'include_str!("../schema/payment_credit_quote_referrals.sql"))')
        I._reviewed_constant((I.ROOT / source).read_text(), constant, expression)
        actual = I._component_bytes(I.ROOT, expression, I.ROOT / source)
        expected = (I.ROOT / 'services/core/schema/payment_credit_quotes.sql').read_bytes() + b'\n' + \
                   (I.ROOT / 'services/core/schema/payment_credit_quote_referrals.sql').read_bytes()
        self.assertEqual(actual, expected)

    def test_unlisted_paths_order_or_separators_are_rejected(self):
        _, constant, expression = I.REVIEWED_SCHEMA_ALIASES['crate::payments::CREDIT_QUOTE_SCHEMA']
        variants = [
            expression.replace('payment_credit_quote_referrals.sql', 'ledger.sql'),
            expression.replace('"\\n"', '""'),
            'concat!(include_str!("../schema/payment_credit_quote_referrals.sql"),"\\n",'
            'include_str!("../schema/payment_credit_quotes.sql"))',
            expression[:-1] + ',include_str!("../schema/identity.sql"))',
        ]
        for changed in variants:
            with self.subTest(expression=changed), self.assertRaisesRegex(I.MappingError, 'component_constant_not_reviewed'):
                I._reviewed_constant('pub const CREDIT_QUOTE_SCHEMA: &str = ' + changed + ';', constant, expression)

    def test_standalone_payments_schema_does_not_rewrite_the_historical_65_contract(self):
        # COMPONENTS uses the original individual SQL producers, not the
        # standalone fixture's aggregate payments::SCHEMA constant.
        self.assertTrue(I._matches_reviewed_components(I.ROOT, I.reviewed_current_ledger()))
        self.assertEqual(I.CURRENT_LEDGER_MANIFEST_SHA256,
                         '67e2cdf264bcfecbb0ca882fc1f4799a9bfa9349366f87d1d7523516e0bc8a06')
        self.assertEqual(I.REVIEWED_CURRENT_LEDGER['compiled_manifest_sha256'], I.CURRENT_LEDGER_MANIFEST_SHA256)


if __name__ == '__main__':
    unittest.main()
