"""Money cutover must preserve paid contract terms before emitting SQL."""
import unittest

from legacy_cutover_money import plan_terms


class SubscriptionTermsTest(unittest.TestCase):
    def test_calendar_contracts_preserve_calendar_months(self):
        for unit, months in (("month", 1), ("year", 12)):
            terms, _ = plan_terms({
                "id": 1, "duration_unit": unit, "duration_value": 1,
                "quota_reset_period": "never", "title": "Calendar", "price_amount": "2",
                "currency": "USD", "total_amount": 1000000,
                "max_purchase_per_user": 1, "enabled": True, "archived_at": 0,
            }, {})
            self.assertEqual(terms["duration_months"], months)
            self.assertEqual(terms["duration_seconds"], 0)

    def test_fixed_duration_terms_preserve_exact_units(self):
        terms, archived = plan_terms({
            "id": 1, "duration_unit": "day", "duration_value": 7,
            "quota_reset_period": "never", "title": "Weekly", "price_amount": "2",
            "currency": "USD", "total_amount": 1000000,
            "max_purchase_per_user": 1, "enabled": True, "archived_at": 0,
        }, {})
        self.assertEqual(terms["duration_seconds"], 604800)
        self.assertEqual(terms["price_units"], "1000000")
        self.assertFalse(archived)


if __name__ == "__main__":
    unittest.main()
