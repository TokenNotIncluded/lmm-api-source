#!/usr/bin/env python3
"""Offline store/support ownership regression tests; no paid or remote calls."""
import copy
import hashlib
import json
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import legacy_cutover_import as CUTOVER
import legacy_extensions_import as IMPORT


class EndpointTransportTests(unittest.TestCase):
    def endpoint(self, connection, *, readonly, database=None, user=None, role=None):
        import financial_import as F
        from pathlib import Path
        from unittest.mock import Mock
        endpoint = {"source_archive_sha256": "a" * 64,
                    "connection": connection, "instance_identity": {"marker": "bound"}}
        private = Mock()
        private.read_text.return_value = json.dumps(endpoint)
        args = SimpleNamespace(archive_sha256="a" * 64, socket="/unused", port=1234)
        with patch.object(F.IDENTITY, "private_backup_path", return_value=private), patch.object(IMPORT, "Db") as db:
            value = IMPORT.endpoint_db(args, Path("/unused/receipt"), readonly=readonly,
                                       database=database, user=user, role=role)
            return value, db.call_args.kwargs

    def connection(self, transport="unix"):
        value = {"transport": transport, "port": 1234, "database": "identity_target",
                 "role": "target_owner", "login_role": "target_installer", "set_role": "target_owner"}
        if transport == "unix":
            value["socket_directory"] = "/private/socket"
        else:
            value["host"] = "127.0.0.1"
        return value

    def test_readonly_unix_retains_bound_endpoint_and_role_pair(self):
        for explicit_transport in (True, False):
            connection = self.connection()
            if not explicit_transport:
                del connection["transport"]
            _, arguments = self.endpoint(connection, readonly=True)
            self.assertTrue(arguments["readonly"])
            self.assertEqual(arguments["role"], "target_owner")
            self.assertEqual(arguments["connection"], connection)
            self.assertEqual(arguments["endpoint"]["instance_identity"], {"marker": "bound"})

    def test_unix_module_writes_and_unknown_transports_rejected(self):
        for transport, readonly in (("unix", False), ("remote-tcp", True), ("remote-tcp", False)):
            with self.assertRaisesRegex(SystemExit, "target_connection_binding_mismatch"):
                self.endpoint(self.connection(transport), readonly=readonly)

    def test_loopback_writes_and_identity_binding_checks_preserved(self):
        _, arguments = self.endpoint(self.connection("loopback-tcp"), readonly=False)
        self.assertFalse(arguments["readonly"])
        for field in ("database", "user", "role"):
            with self.assertRaisesRegex(SystemExit, "target_connection_binding_mismatch"):
                self.endpoint(self.connection(), readonly=True, **{field: "different"})
        connection = self.connection()
        del connection["login_role"]
        del connection["set_role"]
        with self.assertRaisesRegex(SystemExit, "target_connection_binding_mismatch"):
            self.endpoint(connection, readonly=True)


class Context:
    def __init__(self):
        self.args = SimpleNamespace(identity_binding={"accounts": {11: 902, 12: 901, 13: 903}})
        self.tables = {
            "merchant_store_orders": [{"id": "order-source-id", "buyer_id": 11, "seller_id": 12,
                                       "product_id": "product-source-id", "product_title": "Original title"}],
            "merchant_store_support_conversations": [{"id": "source-conversation", "order_id": "order-source-id",
                "buyer_id": 11, "seller_id": 12, "product_id": "product-source-id", "created_at": 1,
                "updated_at": 3, "status": "open", "buyer_read_id": 1, "seller_read_id": 2}],
            "merchant_store_support_messages": [
                {"id": 1, "conversation_id": "source-conversation", "sender_id": 11,
                 "request_key": "buyer-msg", "ciphertext": "buyer", "created_at": 2},
                {"id": 2, "conversation_id": "source-conversation", "sender_id": 12,
                 "request_key": "seller-msg", "ciphertext": "seller", "created_at": 3}],
        }

    def rows(self, table, order="1"):
        return self.tables.get(table, [])

    def count(self, table):
        return len(self.rows(table))


def support_plan(ctx):
    with patch.object(IMPORT, "store_secret", return_value="test-only"), \
         patch.object(IMPORT, "store_decrypt", side_effect=lambda secret, kind, ident, ciphertext: ciphertext):
        return IMPORT.phase_support(ctx)


class OwnershipTests(unittest.TestCase):
    def test_personal_reference_uses_user_id_and_shop_matches_native(self):
        ctx = Context()
        self.assertEqual(IMPORT.store_user(ctx, 11), 11)
        self.assertEqual(IMPORT.store_account(ctx, 11), {"kind": "personal", "id": 11})
        self.assertEqual(IMPORT.store_shop_id(ctx, 12), IMPORT.legacy_shop_id(12))
        self.assertNotEqual(IMPORT.store_shop_id(ctx, 12), IMPORT.legacy_shop_id(901))
        # A source user ID equal to somebody else's account cannot select it.
        with self.assertRaisesRegex(SystemExit, "store_identity_user_unmapped"):
            IMPORT.store_account(ctx, 901)

    def test_missing_and_bad_account_fail_closed(self):
        ctx = Context()
        del ctx.args.identity_binding["accounts"][11]
        with self.assertRaisesRegex(SystemExit, "store_identity_user_unmapped"):
            IMPORT.store_account(ctx, 11)
        ctx.args.identity_binding["accounts"][11] = True
        with self.assertRaisesRegex(SystemExit, "store_identity_account_invalid"):
            IMPORT.store_account(ctx, 11)

    def test_formal_loader_rejects_collisions_and_changed_user(self):
        import financial_import as F
        for rows in ([{"source_user_id": 11, "target_user_id": 11, "target_personal_account_id": 902},
                      {"source_user_id": 12, "target_user_id": 12, "target_personal_account_id": 902}],
                     [{"source_user_id": 11, "target_user_id": 12, "target_personal_account_id": 902}]):
            with self.assertRaises(F.IDENTITY.MappingError):
                F.PLANNER.identity_index(rows)

    def test_support_roles_stable_ids_and_replay_without_payment(self):
        ctx = Context()
        captured = []
        original = IMPORT.put
        def record(table, shop, ident, body):
            captured.append((table, shop, ident, copy.deepcopy(body)))
            return original(table, shop, ident, body)
        with patch.object(IMPORT, "put", side_effect=record):
            first = support_plan(ctx)
        self.assertEqual(first.sql, support_plan(ctx).sql)
        messages = [row[3] for row in captured if row[0] == "lmm_support.messages"]
        self.assertEqual([(m["author_user_id"], m["author_account"]["id"]) for m in messages], [(11, 11), (12, 12)])
        conversation = next(row[3] for row in captured if row[0] == "lmm_support.conversations")
        self.assertEqual(conversation["order_id"], "order-source-id")
        self.assertEqual(conversation["product_id"], "product-source-id")
        self.assertEqual(conversation["buyer"]["id"], 11)
        self.assertEqual(conversation["seller"]["id"], 12)
        self.assertEqual(conversation["shop_id"], IMPORT.legacy_shop_id(12))
        self.assertEqual(conversation["id"], IMPORT.go_id("conversation", 11, IMPORT.personal(11),
            IMPORT.go_id("web-order-support", "order-source-id")))
        self.assertEqual(first.core_sql, [])
        self.assertFalse(any("money_operations" in sql or "post_ledger" in sql for sql in first.sql))

    def test_support_foreign_author_and_mismatched_order_rejected(self):
        ctx = Context()
        ctx.tables["merchant_store_support_messages"][0]["sender_id"] = 13
        with self.assertRaisesRegex(SystemExit, "support_message_author_not_participant"):
            support_plan(ctx)
        ctx = Context()
        ctx.tables["merchant_store_orders"][0]["seller_id"] = 13
        with self.assertRaisesRegex(SystemExit, "support_order_parties_mismatch"):
            support_plan(ctx)

    def test_support_missing_key_cannot_silently_archive_messages(self):
        with patch.object(IMPORT, "store_secret", return_value=None):
            with self.assertRaisesRegex(SystemExit, "support_message_key_missing"):
                IMPORT.phase_support(Context())

    def test_paid_store_order_uses_formal_parties_and_replays_without_pay(self):
        ctx = Context()
        product = {
            "id": "product-source-id", "seller_id": 12, "title": "Product", "description": "",
            "image_urls": "[]", "contact": "", "links": "[]", "test_mode": False, "visibility": "public",
            "purchase_login_required": True, "max_quantity_per_order": 10, "max_quantity_per_buyer": 10,
            "template": "fixed-content", "delivery_strategy": "sequential", "payment_methods": "[]",
            "pickup_login_required": True, "pickup_code_required": False, "email_pickup_link": False,
            "status": "published", "review_note": "", "reviewed_by": 13, "reviewed_at": 1,
            "created_at": 1, "updated_at": 1, "sale_limit": None, "category_id": "", "price_quota": 100,
        }
        order = {
            "id": "f" * 64, "buyer_id": 11, "seller_id": 12, "product_id": product["id"],
            "variant_id": "variant-source-id", "variant_name": "Default", "product_title": "Original title",
            "price_quota": 100, "unit_price_quota": 100, "quantity": 1, "payment_method": "balance",
            "amount_minor": 0, "status": "paid", "delivery_template": "fixed-content", "claimed_at": 0,
            "created_at": 1, "paid_at": 1, "pickup_code_required": False, "seller_terms_version": "",
            "promotion_code": "", "input_digest": "source-input",
        }
        ctx.tables.update(merchant_store_products=[product], merchant_store_orders=[order],
            merchant_store_catalogue_metadata=[{"product_id": product["id"], "default_variant_id": "variant-source-id"}],
            merchant_store_fixed_contents=[{"variant_id": "variant-source-id", "product_id": product["id"], "ciphertext": "private"}],
            merchant_store_order_fixed_deliveries=[{"order_id": order["id"], "ciphertext": "original-snapshot"}])
        original_tables = copy.deepcopy(ctx.tables)
        original_row_sha256 = hashlib.sha256(IMPORT.jdump(order).encode()).hexdigest()
        captured = []
        def capture(table, shop, ident, body):
            captured.append((table, shop, ident, copy.deepcopy(body)))
            return "INSERT " + table + " " + IMPORT.jdump(body)
        with patch.object(IMPORT, "store_secret", return_value="test-only"), \
             patch.object(IMPORT, "store_decrypt", return_value="original-private-body"), \
             patch.object(IMPORT, "put", side_effect=capture):
            first = IMPORT.phase_store(ctx)
            second = IMPORT.phase_store(ctx)
        self.assertEqual(first.sql, second.sql)
        shop = next(row[3] for row in captured if row[0] == "lmm_store.shops")
        self.assertEqual(shop["owner"], {"kind": "personal", "id": 12})
        self.assertEqual(shop["id"], IMPORT.legacy_shop_id(12))
        imported = next(row[3] for row in captured if row[0] == "lmm_store.orders")
        self.assertEqual(imported["id"], order["id"])
        self.assertEqual(imported["buyer"], {"kind": "personal", "id": 11})
        self.assertEqual(imported["seller"], {"kind": "personal", "id": 12})
        self.assertEqual(imported["created_by"], 11)
        self.assertEqual(imported["state"], "settled")
        self.assertEqual(imported["total"]["amount_credits"], 100)
        self.assertEqual(imported["unit_price"]["amount_credits"], order["unit_price_quota"])
        self.assertEqual(imported["quantity"], order["quantity"])
        self.assertEqual(imported["variant_id"], order["variant_id"])
        self.assertEqual(imported["legacy"]["product_id"], order["product_id"])
        self.assertEqual(imported["request_hash"], "legacy:" + order["input_digest"])
        self.assertEqual(imported["imported"], {"source": "go-v0.2.118",
            "snapshot": original_row_sha256, "claimed_at": order["claimed_at"]})
        self.assertEqual(ctx.tables, original_tables)
        self.assertEqual(first.core_sql, [])
        self.assertFalse(any("money_operations" in sql or "post_ledger" in sql for sql in first.sql))

    def test_social_history_preserves_native_likes_and_traffic_without_new_activity(self):
        ctx = Context()
        ctx.tables.update(merchant_store_product_likes=[{"user_id": 11, "product_id": "product", "created_at": 9}],
            merchant_store_product_traffic_days=[{"product_id": "product", "day": 86400, "impressions": 7, "clicks": 2}],
            merchant_store_product_traffic_receipts=[{"id": "source-receipt", "product_id": "product", "kind": "click", "page_started_at": 9, "created_at": 10}])
        plan = IMPORT.Plan()
        IMPORT.store_social_history(plan, ctx, {"product": {"seller_id": 12}})
        self.assertEqual(plan.expect, {"lmm_store.product_likes": 1, "lmm_store.traffic_days": 1, "lmm_store.traffic_receipts": 1})
        self.assertIn('"user_id":11', plan.sql[0])
        self.assertIn(IMPORT.store_shop_id(ctx, 12), plan.sql[0])
        self.assertIn("source-receipt", plan.sql[2])
        duplicate = copy.deepcopy(ctx.tables["merchant_store_product_likes"][0])
        ctx.tables["merchant_store_product_likes"].append(duplicate)
        with self.assertRaisesRegex(SystemExit, "store_social_identity_collision"):
            IMPORT.store_social_history(IMPORT.Plan(), ctx, {"product": {"seller_id": 12}})

    def test_refund_requires_full_actual_principal_and_item_ownership(self):
        order = {"id": "source-order", "status": "refunded", "quantity": 2, "price_quota": 100, "buyer_id": 11, "seller_id": 12}
        refund = {"id": "source-refund", "status": "completed", "mode": "full", "quantity": 2,
                  "principal_quota": 100, "amount_minor": 0, "requested_role": "seller", "requested_by": 12}
        IMPORT.validate_store_refund(refund, order)
        for changed in ({"quantity": 1}, {"principal_quota": 50}, {"mode": "items"}, {"requested_by": 11}):
            with self.assertRaisesRegex(SystemExit, "store_refund_full_principal_or_parties_mismatch"):
                IMPORT.validate_store_refund({**refund, **changed}, order)
        ctx = Context()
        item = {"id": "source-item", "refund_id": "source-refund", "order_id": "source-order", "stock_id": "source-stock", "created_at": 3}
        ctx.tables["merchant_store_refund_items"] = [item]
        self.assertEqual(IMPORT.store_refund_items(ctx, refund, order, {"source-stock": {"order_id": "source-order"}}), [item])
        with self.assertRaisesRegex(SystemExit, "store_refund_item_ownership_invalid"):
            IMPORT.store_refund_items(ctx, refund, order, {"source-stock": {"order_id": "another-order"}})

    def test_paid_promotions_only_deferred_after_expiry(self):
        ctx = Context()
        ctx.tables["merchant_store_promotions"] = [{"created_at": 1, "expires_at": 2, "quota": 100}]
        IMPORT.store_promotion_history(IMPORT.Plan(), ctx)
        ctx.tables["merchant_store_promotions"][0]["expires_at"] = 9999999999
        with self.assertRaisesRegex(SystemExit, "store_active_paid_promotion_requires_native_rights"):
            IMPORT.store_promotion_history(IMPORT.Plan(), ctx)

    def test_support_global_read_marker_is_translated_to_conversation_sequence(self):
        ctx = Context()
        ctx.tables["merchant_store_support_messages"][0]["id"] = 100
        ctx.tables["merchant_store_support_messages"][1]["id"] = 200
        ctx.tables["merchant_store_support_conversations"][0].update(buyer_read_id=50, seller_read_id=100)
        captured = []
        with patch.object(IMPORT, "put", side_effect=lambda table, shop, ident, body: captured.append((table, body)) or "sql"):
            support_plan(ctx)
        conversation = next(body for table, body in captured if table == "lmm_support.conversations")
        self.assertEqual(conversation["buyer_read_id"], 0)
        self.assertEqual(conversation["seller_read_id"], 1)

    def test_wrong_actual_target_account_rejected(self):
        proof = {"source_archive_sha256": "a" * 64, "identity_mapping_sha256": "b" * 64,
                 "expected_identity_manifest": {"scope": "identity-only"}}
        binding = {"accounts": {11: 902}, "provenance": proof}
        run = {"source_archive_sha256": "a" * 64, "mapping_sha256": "b" * 64,
               "manifest": proof["expected_identity_manifest"]}
        db = SimpleNamespace(run=lambda sql: json.dumps({"users": [{"id": 11, "personal_account_id": 901,
                                                                   "kind": "personal"}], "runs": [run]}))
        with self.assertRaisesRegex(SystemExit, "target_identity_account_map_mismatch"):
            CUTOVER.verify_identity_binding(db, binding)


if __name__ == "__main__":
    unittest.main()
