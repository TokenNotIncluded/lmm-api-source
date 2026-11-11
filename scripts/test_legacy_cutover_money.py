"""Money cutover must preserve paid contract terms before emitting SQL."""
import json
import unittest

from legacy_cutover_money import plan_terms, subscription_order_links, plan_promotions, discount_available, discount_capacity_holds, topup_can_settle, TOPUP_PAYABLE_SQL, plan_wallet


class SubscriptionTermsTest(unittest.TestCase):
    def test_calendar_contracts_preserve_calendar_months(self):
        for unit, months in (("month", 1), ("year", 12)):
            terms, _ = plan_terms({
                "id": 1, "duration_unit": unit, "duration_value": 1,
                "quota_reset_period": "never", "title": "Calendar", "price_amount": "2",
                "currency": "USD", "total_amount": 1000000,
                "max_purchase_per_user": 1, "enabled": True, "archived_at": 0, "allow_balance_pay": True,
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
            "max_purchase_per_user": 1, "enabled": True, "archived_at": 0, "allow_balance_pay": True,
        }, {})
        self.assertEqual(terms["duration_seconds"], 604800)
        self.assertEqual(terms["price_units"], "1000000")
        self.assertFalse(archived)

    def test_balance_payment_policy_preserves_source_bool_and_rejects_unknown(self):
        source = {"id": 1, "duration_unit": "day", "duration_value": 7,
            "quota_reset_period": "never", "title": "External-only", "price_amount": "2",
            "currency": "USD", "total_amount": 1000000, "max_purchase_per_user": 1,
            "enabled": True, "archived_at": 0}
        for allowed in (False, True):
            with self.subTest(allowed=allowed):
                terms, _ = plan_terms({**source, "allow_balance_pay": allowed}, {})
                self.assertIs(terms["allow_balance_pay"], allowed)
                self.assertTrue(terms["enabled"])
        for value in (None, 0, 1, "false", "true"):
            with self.subTest(value=value), self.assertRaisesRegex(SystemExit, "plan_balance_payment_policy_unproven"):
                plan_terms({**source, "allow_balance_pay": value}, {})
        with self.assertRaisesRegex(SystemExit, "plan_balance_payment_policy_unproven"):
            plan_terms(source, {})




class DiscountPaymentLifecycleTest(unittest.TestCase):
    def test_released_timeout_and_unreserved_payable_orders_still_hold_capacity(self):
        def order(trade, status="failed", provider="waffo_pancake", reason="checkout_timeout"):
            return {"trade_no":trade,"discount_code_id":1,"status":status,"payment_provider":provider,"failure_reason_code":reason}
        rows=[order("released-timeout"),order("unreserved-timeout"),order("unreserved-pending","pending"),order("closed-failure",reason="declined"),order("closed-expired","expired"),order("unknown-reason",reason=None),order("consumed","success")]
        reservations=[{"top_up_trade_no":trade,"discount_code_id":1,"status":status,"expires_time":1} for trade,status in [("released-timeout","released"),("closed-failure","reserved"),("closed-expired","reserved"),("unknown-reason","reserved"),("missing-order","reserved"),("consumed","consumed")]]
        data={"discount_codes":[{"id":1}],"discount_code_reservations":reservations,"discount_top_ups":rows}
        self.assertEqual(discount_capacity_holds(data),{1:5})
        self.assertTrue(topup_can_settle(rows[0]))
        self.assertFalse(topup_can_settle(rows[3]))
        self.assertFalse(topup_can_settle(rows[5]))
        self.assertIn("checkout_timeout", TOPUP_PAYABLE_SQL)
        with self.assertRaisesRegex(SystemExit,"discount_payment_lifecycle_missing"):
            discount_capacity_holds({k:v for k,v in data.items() if k!="discount_top_ups"})
        with self.assertRaisesRegex(SystemExit,"duplicate"):
            discount_capacity_holds({**data,"discount_top_ups":rows+[rows[0]]})

    def test_wallet_keeps_timeout_status_and_quarantines_missing_owners(self):
        timeout={"id":11,"user_id":1,"status":"failed","payment_provider":"waffo_pancake","failure_reason_code":"checkout_timeout","credited_quota":123}
        data={"users":[{"id":1}],"wallet_transfers":[],"top_ups_payable":[timeout,{**timeout,"id":12,"user_id":9}],
              "identity_binding":{"accounts":{1:7}},
              "opening_binding":{"run":{"migration_id":"formal-opening","mapping_sha256":"b"*64}}}
        sql,stats=plan_wallet(data,"a"*64,"fixture",100)
        self.assertEqual(stats["pending_topup_liabilities"],1)
        self.assertEqual(stats["late_payable_timeout_topup_liabilities"],1)
        self.assertEqual(stats["unrepresented_payable_topup_orphan_rows"],1)
        request=json.loads(sql[0].split("(",1)[1].removesuffix("::jsonb);")[1:-1].replace("''","'"))
        self.assertEqual(request["request"]["source_status"],"failed")
        self.assertEqual(request["status"],"failed")
        self.assertEqual(request["payload_json"],timeout)
        self.assertEqual(request["settlement_state"],"unknown")
        self.assertEqual(request["owner_user_id"],1)
        self.assertEqual(request["owner_account_id"],7)
        self.assertEqual(request["migration_id"],"formal-opening")
        self.assertEqual(request["mapping_sha256"],"b"*64)
        self.assertIs(request["spendable"],False)
        self.assertEqual(request["spendable_units"],0)
        self.assertNotIn('post_ledger',"".join(sql))
        with self.assertRaisesRegex(SystemExit,"topup_not_payable"):
            plan_wallet({**data,"top_ups_payable":[{**timeout,"status":"success"}]},"a"*64,"fixture",100)
        with self.assertRaisesRegex(SystemExit,"topup_payable_lifecycle_missing"):
            plan_wallet({k:v for k,v in data.items() if k!="top_ups_payable"},"a"*64,"fixture",100)

if __name__ == "__main__":
    unittest.main()
