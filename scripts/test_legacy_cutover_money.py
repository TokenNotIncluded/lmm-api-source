"""Money cutover must preserve paid contract terms before emitting SQL."""
import unittest

from legacy_cutover_money import plan_terms, subscription_order_links, plan_promotions, discount_available


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

    def test_timestamp_candidate_does_not_repair_missing_source_link(self):
        result = subscription_order_links({
            "user_subscriptions": [{"id": 9, "user_id": 2, "plan_id": 3, "start_time": 100}],
            "subscription_orders": [{"id": 1, "user_id": 2, "plan_id": 3,
                "complete_time": 100, "user_subscription_id": 0, "status": "success"}],
        })
        self.assertEqual(result[0]["state"], "missing_source_link")
        self.assertEqual(result[0]["source_subscription_id"], 0)
        self.assertEqual(result[0]["timestamp_candidate_ids"], [9])

    def test_checkin_history_does_not_authorize_a_new_reward_budget(self):
        data = {key: [] for key in ("redemptions", "discount_codes", "discount_code_reservations",
            "red_packets", "red_packet_items", "referral_rewards", "other_counts")}
        data.update(users=[{"id": 1, "role": 100, "deleted_at": None}],
            options={"checkin_setting.enabled": "true"}, checkin_recent=[{"n": 10, "total": 100}],
            checkin_total=[{"n": 10}], red_packet_claims=[{"n": 0}])
        sql, stats = plan_promotions(data, "a" * 64, "test", 1000)
        self.assertEqual(sql, [])
        self.assertTrue(stats["unrepresented_checkin_enabled"])
        self.assertEqual(stats["legacy_checkins_history_not_imported"], 10)

    def test_discount_reservations_and_start_time_preserve_capacity(self):
        code = {"status": 1, "deleted_at": None, "starts_time": 0,
                "expired_time": 0, "max_uses": 1, "used_count": 0}
        self.assertTrue(discount_available(code, 0, 100))
        self.assertFalse(discount_available(code, 1, 100))
        self.assertFalse(discount_available({**code, "starts_time": 101}, 0, 100))
        self.assertTrue(discount_available({**code, "expired_time": 100}, 0, 100))

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
