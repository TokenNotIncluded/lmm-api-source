"""Pure contract projection tests; no Core65 admission or database connection."""
import unittest

import financial_import as F


class FinancialProfileTests(unittest.TestCase):
    def base(self):
        return {name: "synthetic:" + name for name in F.CONTRACT_FIELDS}

    def test_historical_projection_preserves_original_material_shape(self):
        value = self.base()
        value["import_profile"] = "historical-profile"
        self.assertEqual(F.reviewed_source_contract(value), self.base())

    def test_explicit_profiles_seal_all_reviewed_pins(self):
        pins = ("source_schema_revision", "compiled_manifest_sha256",
                "fresh_catalog_sha256", "seed_query_sha256")
        for profile in (F.IDENTITY.CORE59_SEED_CONTRACT, F.IDENTITY.CORE65_SEED_CONTRACT, F.IDENTITY.CURRENT_LEDGER_SEED_CONTRACT):
            with self.subTest(profile=profile):
                value = {**self.base(), "import_profile": profile,
                         **{name: "synthetic:" + name for name in pins}}
                self.assertEqual(F.reviewed_source_contract(value), value)
                for name in pins:
                    incomplete = {key: item for key, item in value.items() if key != name}
                    with self.assertRaises(KeyError):
                        F.reviewed_source_contract(incomplete)


if __name__ == "__main__":
    unittest.main()
