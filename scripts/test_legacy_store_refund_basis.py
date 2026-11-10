import copy
import hashlib
import unittest

import legacy_store_refund_basis as B


class LegacyStoreRefundBasisTests(unittest.TestCase):
    def setUp(self):
        self.identities = [{"source_user_id": n, "target_user_id": n + 10,
                            "target_personal_account_id": n + 100} for n in (1, 2)]
        self.material = {"scope": B.F.SCOPE, "migration_id": "synthetic-opening",
                         "source_archive_sha256": "a" * 64, "mapping_sha256": "b" * 64,
                         "identity_map_sha256": B.F.digest({n: n + 100 for n in (1, 2)}),
                         "source_review": {"credits_per_usd": 500000},
                         "opening": [{"account_id": n + 100, "source_entity_key": f"user:{n}",
                                      "observed_at": 200} for n in (1, 2)]}
        paid = {"id": "c" * 64, "buyer_id": 2, "seller_id": 1, "quantity": 2,
                "unit_price_quota": 50, "price_quota": 100, "payment_method": "balance", "status": "paid",
                "paid_at": 100, "amount_minor": 0, "fee_quota": 5, "fee_held": False,
                "pickup_token_ciphertext": "PRIVATE-SOURCE-NOT-FOR-CORE"}
        refunded = {**paid, "id": "d" * 64, "status": "refunded"}
        self.source = {"source_archive_sha256": "a" * 64, "mapping_sha256": "b" * 64,
                       "orders": [paid, refunded], "transfers": [{"id": paid["id"] + ":sale", "order_id": paid["id"],
                       "kind": "sale", "from_user_id": 2, "to_user_id": 1, "quota": 100}],
                       "refunds": [{"id": "original-completed", "order_id": refunded["id"], "status": "completed"}]}

    def build(self):
        return B.candidate(self.source, self.material, self.identities,
                           {"source_export_sha256": "e" * 64, "financial_material_sha256": "f" * 64,
                            "identities_sha256": "1" * 64})

    def test_source_sale_and_opening_mapping_are_bound_without_new_payment(self):
        value = self.build()
        self.assertEqual(len(value["grants"]), 1)
        self.assertEqual(value["history_only_refunded"], 1)
        grant = value["grants"][0]
        self.assertEqual(grant["buyer_user_id"], 12)
        self.assertEqual(grant["seller_user_id"], 11)
        self.assertEqual(grant["source_order"]["price_quota"], 100)
        self.assertEqual(grant["source_order"]["fee_quota"], 5)
        self.assertEqual(grant["source_row_sha256"], hashlib.sha256(B.STORE.jdump(self.source["orders"][0]).encode()).hexdigest())
        self.assertNotIn("PRIVATE-SOURCE", B.STORE.jdump(value))
        review = {"scope": "legacy-store-full-refund-review-v1", "decision": "approved-owner-import",
                  "candidate_sha256": B.F.digest(value), "complete_source_tables": True}
        sql = B.reviewed_sql(value, review)
        self.assertIn("import_legacy_refund_grant", sql)
        for forbidden in ("post_ledger", "import_opening_balance", "import_legacy_escrow_batch", "native_pay"):
            self.assertNotIn(forbidden, sql)

    def test_completed_or_pending_original_refund_cannot_be_minted_as_free_cap(self):
        for status in ("completed", "requested", "pending", "unknown", "approved"):
            with self.subTest(status=status):
                self.source["refunds"] = [{"id": "occupied", "order_id": "c" * 64, "status": status}]
                with self.assertRaises(B.F.Error):
                    self.build()

    def test_refund_transfer_without_refund_row_is_not_ignored(self):
        self.source["transfers"].append({"id": "old-refund", "order_id": "c" * 64,
                                        "kind": "refund", "quota": 100})
        with self.assertRaises(B.F.Error):
            self.build()

    def test_wrong_original_sale_amount_or_party_rejects(self):
        for field, value in (("quota", 99), ("from_user_id", 1), ("to_user_id", 2)):
            source = copy.deepcopy(self.source)
            self.source["transfers"][0][field] = value
            with self.assertRaises(B.F.Error):
                self.build()
            self.source = source

    def test_opening_identity_and_review_changes_reject(self):
        value = self.build()
        review = {"scope": "legacy-store-full-refund-review-v1", "decision": "approved-owner-import",
                  "candidate_sha256": B.F.digest(value), "complete_source_tables": True}
        value["grants"][0]["source_order"]["price_quota"] = 999
        with self.assertRaises(B.F.Error):
            B.reviewed_sql(value, review)
        self.material["opening"][0]["source_entity_key"] = "user:other"
        with self.assertRaises(B.F.Error):
            self.build()


if __name__ == "__main__":
    unittest.main()
