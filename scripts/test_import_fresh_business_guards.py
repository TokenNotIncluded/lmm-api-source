"""Fresh-import fences must cover actual tables introduced by each component."""
from pathlib import Path
import re
import unittest

import financial_import as F


class FreshBusinessGuardTests(unittest.TestCase):
    def test_additive_components_fence_every_actual_table(self):
        importer = F.IDENTITY
        baseline = {"component_fingerprints": {}}
        historical_receipts = set(importer.upgrade_receipt_tables(baseline))
        for component, filename in (
            ("payment-discounts", "payment_discounts.sql"),
            ("payment-discount-management", "payment_discount_management.sql"),
            ("responses-continuity", "responses_continuity.sql"),
        ):
            with self.subTest(component=component):
                schema = (Path(__file__).resolve().parents[1] /
                          "services/core/schema" / filename).read_text()
                tables = set(re.findall(r"^CREATE TABLE ([a-z_]+\.[a-z_]+)\s*\(",
                                        schema, re.MULTILINE))
                self.assertTrue(tables, "test must inspect actual component tables")
                receipts = {name for name in tables if name.startswith("core_meta.")}
                business = tables - receipts
                contract = {"component_fingerprints": {component: "reviewed-separately"}}
                self.assertEqual(set(importer.fresh_business_tables(contract)), business)
                self.assertEqual(set(importer.upgrade_receipt_tables(contract)) -
                                 historical_receipts, receipts)

    def test_receipt_acl_guards_only_reference_installed_suffixes(self):
        importer = F.IDENTITY
        base = {"commerce-self-purchase": "fixture"}
        cases = (
            ("payment-discounts", "core_meta.payment_discount_upgrades"),
            ("payment-discount-management", "core_meta.payment_discount_management_upgrades"),
            ("responses-continuity", "core_meta.responses_continuity_upgrades"),
        )
        historical = importer.profile_acl_guard_sql({"component_fingerprints": base})
        for component, table in cases:
            with self.subTest(component=component):
                self.assertNotIn(table, historical)
                guard = importer.profile_acl_guard_sql({
                    "component_fingerprints": {**base, component: "fixture"}})
                # SQL-generation regression only; actual privilege rejection is
                # verified separately against the native fresh database.
                self.assertEqual(guard.count("c.oid='" + table + "'::regclass"), 3)
                self.assertIn("aclexplode(t.attacl)", guard)
                self.assertIn("d.defaclrole=c.relowner", guard)
                self.assertIn("d.defaclnamespace IN (0,c.relnamespace)", guard)
                self.assertIn("d.defaclobjtype='r'", guard)
                for other, absent in cases:
                    if other != component:
                        self.assertNotIn(absent, guard)

    def test_historical_contract_never_queries_uninstalled_components(self):
        historical = {"component_fingerprints": {}}
        self.assertEqual(F.IDENTITY.fresh_business_tables(historical), ())
        receipts = F.IDENTITY.upgrade_receipt_tables(historical)
        self.assertFalse(any("discount" in name or "continuity" in name
                             for name in receipts))


if __name__ == "__main__":
    unittest.main()
