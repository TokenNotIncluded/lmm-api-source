"""Synthetic offline records only: no PG, production data, network or Go build."""
import argparse
import base64
import copy
import hashlib
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
import legacy_extensions_import as m

SECRET = "isolated-migration-fixture-never-production-20261110"
TARGET = bytes(range(32))
BCRYPT = "$2a$10$" + "A" * 53  # preservation fixture, not a valid-password claim


def encrypted(kind, ident, value):
    key = hashlib.sha256(f"merchant-store:{kind}:{ident}:{SECRET}".encode()).digest()
    nonce = os.urandom(12)
    return "v1:" + base64.urlsafe_b64encode(nonce + AESGCM(key).encrypt(nonce, value.encode(), None)).decode().rstrip("=")


def product(ident, status="published", template="card-key"):
    return dict(id=ident, seller_id=1, title=ident, description="", image_urls="[]", contact="", links="[]",
                price_quota=1000000, test_mode=False, visibility="public", purchase_login_required=True,
                sale_limit=None, max_quantity_per_order=None, max_quantity_per_buyer=None, template=template,
                delivery_strategy="sequential", payment_methods='["balance"]', pickup_login_required=True,
                pickup_code_required=True, email_pickup_link=False, status=status, review_note="", reviewed_by=0,
                reviewed_at=0, created_at=1, updated_at=2, category_id="")


class Context:
    def __init__(self, args, tables): self.args, self.tables = args, tables
    def rows(self, table, order="1"): return copy.deepcopy(self.tables.get(table, []))
    def count(self, table): return len(self.tables.get(table, []))


class StoreImportTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        source, target = Path(self.tmp.name) / "source", Path(self.tmp.name) / "target"
        source.write_text(SECRET); target.write_bytes(TARGET)
        source.chmod(0o600); target.chmod(0o600)
        self.args = argparse.Namespace(store_key_file=str(source), store_stock_key_file=str(target), legacy_secrets_env=None)
        self.tables = {"merchant_store_products": [product("deleted", "deleted")],
                       "merchant_store_catalogue_metadata": [{"product_id": "deleted", "default_variant_id": "variant"}],
                       "merchant_store_variants": [], "merchant_store_orders": [], "merchant_store_refunds": [],
                       "merchant_store_stocks": []}
        self.captured = []

    def run_plan(self):
        def capture(table, shop, ident, body):
            self.captured.append((table, shop, ident, copy.deepcopy(body)))
            return f"INSERT INTO {table} /* synthetic */;"
        with patch.object(m, "put", capture):
            return m.phase_store(Context(self.args, self.tables))

    def bodies(self, table):
        return {ident: body for t, _, ident, body in self.captured if t == table}

    def add_order(self, n, status="paid", claimed=True):
        ident = f"{n:064x}"
        token = base64.urlsafe_b64encode(bytes([n]) * 32).decode().rstrip("=")
        row = dict(id=ident, product_id="deleted", variant_id="variant", quantity=1, price_quota=1000000,
                   unit_price_quota=1000000, payment_method="balance", amount_minor=0, claimed_at=3 if claimed else 0,
                   status=status, created_at=1, paid_at=2, delivery_template="" if status == "paid" else "card-key",
                   pickup_code_required=True, pickup_code_hash=BCRYPT, pickup_login_required=True,
                   pickup_token_hash=hashlib.sha256(token.encode()).hexdigest(),
                   pickup_token_ciphertext=encrypted("pickup", ident, token), seller_terms_version="",
                   promotion_code="", buyer_id=2, seller_id=1, variant_name="Item", product_title="Old item", input_digest="original")
        self.tables["merchant_store_orders"].append(row)
        if status == "refunded":
            self.tables["merchant_store_refunds"].append(dict(id="refund-" + str(n), order_id=ident, status="completed", mode="full",
                reason="Old full refund", request_key="refund", input_digest="digest", requested_by=2, created_at=2,
                decision_by=1, decision_reason="", decided_at=2, completed_at=3))
        return ident, token

    def add_stock(self, position, state="available", order=""):
        ident = f"stock-{position}"
        self.tables["merchant_store_stocks"].append(dict(id=ident, product_id="deleted", variant_id="variant",
            state=state, position=position, order_id=order, ciphertext=encrypted("stock", "deleted:" + ident, "PRIVATE-" + ident), created_at=1))

    def test_four_orders_and_fifty_real_stock_rows_preserve_protection_and_state(self):
        ids = [self.add_order(1)[0], self.add_order(2)[0], self.add_order(3, "refunded")[0], self.add_order(4, "refunded", False)[0]]
        for n, ident in enumerate(ids, 1): self.add_stock(n, "delivered" if n < 3 else "refunded", ident)
        for n in range(5, 51): self.add_stock(n)
        # Null-state LEFT JOIN rows are products, never fabricated inventory.
        for n in range(4):
            ident = "empty-" + str(n)
            self.tables["merchant_store_products"].append(product(ident, template="redemption-code" if n == 3 else "card-key"))
            self.tables["merchant_store_catalogue_metadata"].append(dict(product_id=ident, default_variant_id=ident + "-variant"))
        plan = self.run_plan()
        self.assertEqual(plan.expect["lmm_store.orders"], 4)
        self.assertEqual(plan.expect["lmm_store.stock_items"], 50)
        self.assertEqual(plan.expect["lmm_store.commands"], 12)
        variants = self.bodies("lmm_store.variants")
        self.assertEqual((variants["variant"]["stock"], variants["variant"]["claimed"], variants["variant"]["stock_sequence"]), (46, 4, 50))
        self.assertTrue(all(v["stock"] == 0 for k, v in variants.items() if k != "variant"))
        self.assertFalse(self.bodies("lmm_store.products")["deleted"]["published"])
        orders = self.bodies("lmm_store.orders")
        self.assertEqual([orders[i]["state"] for i in ids], ["settled", "settled", "refunded", "refunded"])
        for ident in ids:
            self.assertTrue(orders[ident]["legacy"]["pickup_code_required"])
            self.assertIn("imported", orders[ident])
            self.assertNotIn("payment_ref", orders[ident])
        protection = self.bodies("lmm_store.commands")[m.go_id("legacy-order-pickup-v1", ids[0])]
        self.assertEqual(protection, {"salt": "", "hash": "", "legacy_bcrypt": BCRYPT})
        self.assertFalse(any("money_operations" in sql or "core_" in sql for sql in plan.sql))

    def test_target_cipher_is_bound_to_original_shop_and_item(self):
        ident, _ = self.add_order(1); self.add_stock(1, "delivered", ident)
        self.run_plan()
        body = self.bodies("lmm_store.stock_items")["stock-1"]
        raw = base64.urlsafe_b64decode(body["ciphertext"][3:] + "=" * (-len(body["ciphertext"][3:]) % 4))
        aad = ("lmm-store-stock-v1\0" + m.legacy_shop_id(1) + "\0stock-1").encode()
        self.assertEqual(AESGCM(TARGET).decrypt(raw[:12], raw[12:], aad), b"PRIVATE-stock-1")
        with self.assertRaises(InvalidTag): AESGCM(TARGET).decrypt(raw[:12], raw[12:], b"another-owner")
        self.assertNotIn("PRIVATE-", body["ciphertext"])

    def test_paid_unviewed_allocated_stock_stays_claimable_without_new_pay(self):
        ident, _ = self.add_order(1, claimed=False); self.add_stock(1, "delivered", ident)
        self.run_plan(); order = self.bodies("lmm_store.orders")[ident]
        self.assertTrue(order["delivered"])
        self.assertEqual(order["imported"]["claimed_at"], 0)
        self.assertEqual(order["state"], "settled")

    def test_absent_target_key_fails_instead_of_archiving_live_stock(self):
        self.add_stock(1); self.args.store_stock_key_file = None
        with self.assertRaisesRegex(SystemExit, "store_target_stock_key_required"): self.run_plan()

    def test_bad_token_hash_and_missing_code_do_not_drop_protection(self):
        ident, _ = self.add_order(1); self.add_stock(1, "delivered", ident)
        self.tables["merchant_store_orders"][0]["pickup_code_hash"] = ""
        with self.assertRaisesRegex(SystemExit, "store_pickup_bcrypt_not_supported"): self.run_plan()
        self.tables["merchant_store_orders"][0]["pickup_code_hash"] = BCRYPT
        self.tables["merchant_store_orders"][0]["pickup_token_hash"] = "0" * 64
        with self.assertRaisesRegex(SystemExit, "store_pickup_token_hash_mismatch"): self.run_plan()

    def test_stock_cannot_move_to_another_orders_product(self):
        ident, _ = self.add_order(1); self.add_stock(1, "delivered", "missing-order")
        with self.assertRaisesRegex(SystemExit, "store_stock_order_ownership_mismatch"): self.run_plan()

    def test_fixed_order_reads_original_snapshot_not_current_seller_content(self):
        ident, _ = self.add_order(1, claimed=False)
        self.tables["merchant_store_products"][0]["template"] = "fixed-content"
        self.tables["merchant_store_fixed_contents"] = [dict(variant_id="variant", product_id="deleted", ciphertext=encrypted("fixed-content", "deleted:variant", "new-content"))]
        self.tables["merchant_store_order_fixed_deliveries"] = [dict(order_id=ident, ciphertext=encrypted("order-fixed-delivery", ident, "original-content"))]
        self.run_plan()
        self.assertEqual(self.bodies("lmm_store.orders")[ident]["delivery"], "original-content")
        self.assertEqual(self.bodies("lmm_store.variants")["variant"]["legacy"]["fixed_content"], "new-content")


if __name__ == "__main__": unittest.main()
