#!/usr/bin/env python3
"""Pure source-boundary cases; real SQL and native paid-turn tests are separate."""
from copy import deepcopy
import unittest
import current_weekly_discounts as W

class WeeklySourceTests(unittest.TestCase):
    def setUp(self):
        self.users=[{"id":1},{"id":2}]
        self.refs={"source_archive_sha256":"a"*64,"source_export_sha256":"b"*64,
                   "source_export_proof_sha256":"c"*64}
        self.row={"id":1,"user_id":1,"week_start":345600,"discount_percent":0,
                  "status":"declined","reason":"Original zero decision",
                  "conversation_id":77,"created_at":345642,"claimed_at":0,"code_id":0}
        self.coupon={"id":7,"owner_user_id":1,"discount_percent":5,"max_uses":1,
                     "expired_time":950400,"min_amount":0}
    def project(self,rows,coupons=()):
        return W.project(self.users,list(rows),list(coupons),self.refs,synthetic=True)
    def claimed(self):
        return {**self.row,"discount_percent":5,"status":"claimed","code_id":7,"claimed_at":345660}
    def test_zero_decision_and_original_integer_conversation_are_preserved(self):
        original=deepcopy(self.row)
        out=self.project([self.row])
        self.assertEqual(self.row,original)
        self.assertEqual(out["decision_count"],1)
        decision=out["rows"][0]
        self.assertEqual(decision["discount_percent"],0)
        self.assertEqual(decision["status"],"declined")
        self.assertEqual(decision["source_conversation_id"],77)
        self.assertIsNone(decision["source_code_id"])
    def test_claimed_history_references_exact_existing_private_coupon(self):
        out=self.project([self.claimed()],[self.coupon])
        decision=out["rows"][0]
        self.assertEqual(decision["source_code_id"],7)
        self.assertEqual(decision["claimed_at"],345660)
        self.assertEqual(len(decision["source_coupon_row_sha256"]),64)
        self.assertEqual(out["rows"],self.project([self.claimed()],[self.coupon])["rows"])
    def test_duplicate_week_and_foreign_owner_cannot_create_second_decision(self):
        cases=[
            [self.row,deepcopy(self.row)],
            [self.row,{**self.row,"id":2,"discount_percent":5,"status":"offered"}],
            [{**self.row,"user_id":3}],
            [{**self.row,"week_start":345601}],
            [{**self.row,"created_at":950400}],
        ]
        for rows in cases:
            with self.subTest(rows=len(rows)):
                with self.assertRaises(W.Error):self.project(rows)
    def test_changed_coupon_owner_terms_and_missing_coupon_are_rejected(self):
        for column,value in [("owner_user_id",2),("discount_percent",10),("max_uses",2),
                             ("expired_time",950401),("min_amount",1)]:
            with self.subTest(column=column):
                with self.assertRaises(W.Error):
                    self.project([self.claimed()],[{**self.coupon,column:value}])
        with self.assertRaises(W.Error):self.project([self.claimed()])
    def test_declined_and_offered_cannot_smuggle_a_code_or_claim(self):
        for row in [
            {**self.row,"code_id":7},{**self.row,"claimed_at":345660},
            {**self.row,"discount_percent":5},
            {**self.row,"status":"offered"},
            {**self.row,"status":"claimed","discount_percent":5,"code_id":7,"claimed_at":0},
        ]:
            with self.assertRaises(W.Error):self.project([row],[self.coupon])
    def test_source_integer_types_are_not_replaced_by_strings_or_booleans(self):
        for column,value in [("id",True),("id",2**63),("week_start","345600"),("conversation_id","77"),
                             ("conversation_id",True),("conversation_id",2**63),("discount_percent",False)]:
            with self.subTest(column=column):
                with self.assertRaises(W.Error):self.project([{**self.row,column:value}])
    def test_history_is_not_rejudged_using_new_ordinary_user_cap(self):
        out=self.project([{**self.row,"status":"offered","discount_percent":99}])
        self.assertEqual(out["rows"][0]["discount_percent"],99)
    def test_production_target_is_closed_until_independent_capture(self):
        if W.TARGET_BINDING_SHA256 is None:
            with self.assertRaisesRegex(W.Error,"weekly_final88_target_unbound"):
                W.contract_from_manifest(b"{}",{},synthetic=False)
        else:
            with self.assertRaises(W.Error):
                W.contract_from_manifest(b"{}",{},synthetic=False)

if __name__=="__main__":unittest.main()
