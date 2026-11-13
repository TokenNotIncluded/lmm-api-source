import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import legacy_store_refund_basis as B


class LegacyStoreRefundBasisTests(unittest.TestCase):
    def test_cli_accepts_formal_jsonl_and_existing_json_array(self):
        self.identities = [
            {"source_user_id": uid, "target_user_id": uid, "target_personal_account_id": account}
            for uid, account in ((1138, 1246), (1139, 1247))]
        self.material["identity_map_sha256"] = B.F.digest({1138: 1246, 1139: 1247})
        self.material["opening"] = [{"account_id": account, "source_entity_key": f"user:{uid}",
                                     "observed_at": 200}
                                    for uid, account in ((1138, 1246), (1139, 1247))]
        for order in self.source["orders"]:
            order.update(buyer_id=1138, seller_id=1139)
        self.source["transfers"][0].update(from_user_id=1138, to_user_id=1139)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, value in (("source", self.source), ("material", self.material)):
                path = root / f"{name}.json"
                path.write_text(json.dumps(value))
                path.chmod(0o600)
            for suffix, raw in (("jsonl", "".join(json.dumps(row) + "\n" for row in self.identities)),
                                ("json", json.dumps(self.identities, indent=2))):
                for source_format in ("normalized", "source-dto"):
                    with self.subTest(identities=suffix, source=source_format):
                        path = root / f"identities.{suffix}"
                        path.write_text(raw)
                        path.chmod(0o600)
                        source_path = root / f"source-{source_format}.json"
                        source_path.write_text(json.dumps(self.source_dto() if source_format == "source-dto" else self.source))
                        source_path.chmod(0o600)
                        output = root / f"output-{suffix}-{source_format}"
                        args = [sys.executable, B.__file__, "--source-export", str(source_path),
                                "--financial-material", str(root / "material.json"),
                                "--identities", str(path), "--output", str(output)]
                        if source_format == "source-dto":
                            args.extend(["--source-format", source_format])
                        result = subprocess.run(args, capture_output=True, text=True)
                        self.assertEqual(result.returncode, 0, result.stderr)
                        value = json.loads((output / "candidate.json").read_text())
                        self.assertEqual(value["inputs"]["identities_sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
                        self.assertEqual(value["inputs"]["source_export_sha256"], hashlib.sha256(source_path.read_bytes()).hexdigest())
                        self.assertEqual((output / "candidate.json").stat().st_mode & 0o777, 0o600)
                        self.assertEqual(output.stat().st_mode & 0o777, 0o700)
                        normalized = output / "normalized-source.private.json"
                        self.assertEqual(normalized.exists(), source_format == "source-dto")
                        if normalized.exists():
                            self.assertEqual(json.loads(normalized.read_text()), self.source)
                            self.assertEqual(normalized.stat().st_mode & 0o777, 0o600)
                        self.assertNotIn("PRIVATE-SOURCE", result.stdout)
                        self.assertEqual((value["grants"][0]["buyer_user_id"], value["grants"][0]["seller_user_id"]),
                                         (1138, 1139))
                        review = {"scope": "legacy-store-full-refund-review-v1", "decision": "approved-owner-import",
                                  "candidate_sha256": B.F.digest(value), "complete_source_tables": True}
                        review_path = root / f"review-{suffix}.json"
                        review_path.write_text(json.dumps(review))
                        review_path.chmod(0o600)
                        reviewed = root / f"reviewed-{suffix}-{source_format}"
                        args[args.index(str(output))] = str(reviewed)
                        result = subprocess.run([*args, "--review", str(review_path)], capture_output=True, text=True)
                        self.assertEqual(result.returncode, 0, result.stderr)
                        sql = (reviewed / "owner-import.sql").read_text()
                        self.assertEqual(sql.count("SELECT core_commerce.import_legacy_refund_grant("), 1)
                        self.assertNotIn("PRIVATE-SOURCE", sql)
                        for forbidden in ("post_ledger", "import_opening_balance", "import_legacy_escrow_batch", "native_pay"):
                            self.assertNotIn(forbidden, sql)

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

    def source_dto(self):
        return {"scope": "legacy-store-source-dto-v1", "source_archive_sha256": self.source["source_archive_sha256"],
                "mapping_sha256": self.source["mapping_sha256"],
                "tables": {table: copy.deepcopy(self.source[name]) for name, table in B.SOURCE_TABLES.items()},
                "table_counts": {table: len(self.source[name]) for name, table in B.SOURCE_TABLES.items()}}

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
        self.assertEqual(grant["shop_id"], B.STORE.legacy_shop_id(11))
        self.assertNotEqual(grant["shop_id"], B.STORE.legacy_shop_id(101))
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

    def test_formal_uid_1138_uses_account_1246_without_another_money_action(self):
        self.identities = [
            {"source_user_id": uid, "target_user_id": uid, "target_personal_account_id": account}
            for uid, account in ((1138, 1246), (1139, 1247))]
        self.material["identity_map_sha256"] = B.F.digest({1138: 1246, 1139: 1247})
        self.material["opening"] = [{"account_id": account, "source_entity_key": f"user:{uid}",
                                     "observed_at": 200}
                                    for uid, account in ((1138, 1246), (1139, 1247))]
        for order in self.source["orders"]:
            order.update(buyer_id=1138, seller_id=1139)
        self.source["transfers"][0].update(from_user_id=1138, to_user_id=1139)
        original_source = copy.deepcopy(self.source)
        original_material = copy.deepcopy(self.material)
        value = self.build()
        self.assertEqual(len(value["grants"]), 1)
        grant = value["grants"][0]
        self.assertEqual((grant["buyer_user_id"], grant["seller_user_id"]), (1138, 1139))
        self.assertEqual(grant["shop_id"], B.STORE.legacy_shop_id(1139))
        self.assertNotEqual(grant["shop_id"], B.STORE.legacy_shop_id(1247))
        self.assertEqual((grant["source_order"]["buyer_id"], grant["source_order"]["seller_id"]), (1138, 1139))
        self.assertEqual(grant["source_order"], {k: original_source["orders"][0][k] for k in B.ORDER_FIELDS})
        self.assertEqual(grant["source_sale"], {k: original_source["transfers"][0][k] for k in B.SALE_FIELDS})
        self.assertEqual(grant["source_row_sha256"], hashlib.sha256(B.STORE.jdump(original_source["orders"][0]).encode()).hexdigest())
        self.assertEqual(grant["source_sale_sha256"], hashlib.sha256(B.STORE.jdump(original_source["transfers"][0]).encode()).hexdigest())
        self.assertEqual(self.source, original_source)
        self.assertEqual(self.material, original_material)
        review = {"scope": "legacy-store-full-refund-review-v1", "decision": "approved-owner-import",
                  "candidate_sha256": B.F.digest(value), "complete_source_tables": True}
        sql = B.reviewed_sql(value, review)
        self.assertEqual(sql.count("core_commerce.import_legacy_refund_grant("), 1)
        self.assertEqual(sql, B.reviewed_sql(value, review))
        for forbidden in ("post_ledger", "import_opening_balance", "import_legacy_escrow_batch", "native_pay"):
            self.assertNotIn(forbidden, sql)
        self.material["opening"][0]["account_id"] = 1138
        with self.assertRaisesRegex(B.F.Error, "store_original_opening_missing"):
            self.build()

    def test_completed_or_pending_original_refund_cannot_be_minted_as_free_cap(self):
        for status in ("completed", "requested", "pending", "unknown", "approved"):
            with self.subTest(status=status):
                self.source["refunds"] = [{"id": "occupied", "order_id": "c" * 64, "status": status}]
                with self.assertRaises(B.F.Error):
                    self.build()

    def test_noncash_and_zero_basis_preserve_history_without_grants(self):
        paid = self.source["orders"][0]
        paid["payment_method"] = "noncash"
        value = self.build()
        self.assertEqual(value["grants"], [])
        self.assertEqual([r["reason"] for r in value["history_only"]],
                         ["noncash_no_balance_basis", "already_refunded"])
        paid.update(payment_method="balance", price_quota=0, unit_price_quota=0, fee_quota=0)
        value = self.build()
        self.assertEqual(value["grants"], [])
        self.assertEqual(value["history_only"][0]["reason"], "zero_balance_basis")
        paid["fee_quota"] = 1
        with self.assertRaisesRegex(B.F.Error, "store_zero_basis_inconsistent"):
            self.build()

    def test_complete_source_dto_normalizes_rows_without_renaming_financial_export(self):
        raw = self.source_dto()
        self.assertEqual(B.normalize_source_dto(raw), self.source)
        self.assertIn("pickup_token_ciphertext", B.normalize_source_dto(raw)["orders"][0])
        raw["table_counts"]["merchant_store_orders"] += 1
        with self.assertRaisesRegex(B.F.Error, "store_source_dto_count"):
            B.normalize_source_dto(raw)
        with self.assertRaisesRegex(B.F.Error, "store_source_dto_shape"):
            B.normalize_source_dto({"kind": "merchant_store_orders", "row": self.source["orders"][0]})

    def test_source_dto_rejects_bad_shapes_counts_and_missing_fields(self):
        for value in (None, [], {**self.source_dto(), "tables": None},
                      {**self.source_dto(), "table_counts": []}):
            with self.subTest(value=value), self.assertRaises(B.F.Error):
                B.normalize_source_dto(value)
        for table in B.SOURCE_TABLES.values():
            for change in ("missing_table", "bad_count", "bad_rows", "missing_field"):
                raw = self.source_dto()
                if change == "missing_table":
                    del raw["tables"][table]
                elif change == "bad_count":
                    raw["table_counts"][table] = True
                elif change == "bad_rows":
                    raw["tables"][table] = [None]
                    raw["table_counts"][table] = 1
                else:
                    del raw["tables"][table][0]["id"]
                with self.subTest(table=table, change=change), self.assertRaises(B.F.Error):
                    B.normalize_source_dto(raw)

    def test_cross_domain_numeric_collisions_keep_buyer_seller_and_shop_roles(self):
        self.identities = [
            {"source_user_id": 1, "target_user_id": 12, "target_personal_account_id": 2},
            {"source_user_id": 2, "target_user_id": 11, "target_personal_account_id": 1}]
        self.material["identity_map_sha256"] = B.F.digest({1: 2, 2: 1})
        self.material["opening"] = [
            {"account_id": 2, "source_entity_key": "user:1", "observed_at": 200},
            {"account_id": 1, "source_entity_key": "user:2", "observed_at": 200}]
        grant = self.build()["grants"][0]
        self.assertEqual((grant["buyer_user_id"], grant["seller_user_id"]), (11, 12))
        self.assertEqual(grant["shop_id"], B.STORE.legacy_shop_id(12))
        self.assertNotEqual(grant["shop_id"], B.STORE.legacy_shop_id(2))
        self.assertEqual((grant["source_sale"]["from_user_id"], grant["source_sale"]["to_user_id"]), (2, 1))
        self.assertEqual(self.build(), self.build(), "same immutable inputs produce the same candidate")
        self.identities[0]["target_personal_account_id"] = 3
        with self.assertRaisesRegex(B.F.Error, "escrow_identity_plan_mismatch"):
            self.build()
        self.identities.pop()
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
