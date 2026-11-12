"""Current fresh66 module-split source and migration profile regressions."""
import copy
import unittest
from unittest.mock import patch

import financial_import as F

I = F.IDENTITY


class ModuleSplitImportProfileTests(unittest.TestCase):
    def test_current_contract_has_exact_66_components_and_no_removed_schema(self):
        contract = I.current_contract()
        self.assertEqual(contract['import_profile'], I.MODULE_SPLIT_SEED_CONTRACT)
        self.assertEqual(contract['compiled_manifest_sha256'], I.MODULE_SPLIT_MANIFEST_SHA256)
        self.assertEqual(contract['fresh_catalog_sha256'], I.MODULE_SPLIT_CATALOG_SHA256)
        self.assertEqual(len(contract['component_fingerprints']), 66)
        self.assertNotIn('checkins', contract['component_fingerprints'])
        self.assertNotIn('navigation-options', contract['component_fingerprints'])
        self.assertNotIn('core_checkins.', I.fresh_seed_hash_query(I.MODULE_SPLIT_SEED_CONTRACT))
        self.assertIn('core_meta.pancake_language_upgrades',
                      I.fresh_seed_hash_query(I.MODULE_SPLIT_SEED_CONTRACT))
        self.assertNotIn('services/core/schema/checkins.sql', contract['source_file_hashes'])
        self.assertNotIn('services/core/schema/navigation_options.sql', contract['source_file_hashes'])
        self.assertNotIn('core_meta.navigation_options_upgrades', I.upgrade_receipt_tables(contract))

    def test_each_actual_receipt_is_independently_required(self):
        for init, acl, missing in ((None, None, 'actual_init'), ('ab' * 32, None, 'acl')):
            review = copy.deepcopy(I.REVIEWED_MODULE_SPLIT)
            review.update(actual_init_receipt_sha256=init, acl_receipt_sha256=acl)
            with patch.object(I, 'REVIEWED_MODULE_SPLIT', review):
                with self.assertRaisesRegex(I.MappingError, 'module_split_' + missing + '_receipt_sha256_pending'):
                    I.reviewed_module_split()

    def test_review_seal_and_historical_profiles_remain_strict(self):
        with patch.object(I, 'MODULE_SPLIT_REVIEW_SHA256', '00' * 32):
            with self.assertRaisesRegex(I.MappingError, 'module_split_review_file_changed'):
                I.reviewed_module_split(require_receipts=False)
        before = copy.deepcopy((I.REVIEWED_CHECKINS, I.REVIEWED_PANCAKE_LANGUAGE,
                                I.REVIEWED_CREDIT_QUOTES))
        I.reviewed_module_split(require_receipts=False)
        self.assertEqual(before, (I.REVIEWED_CHECKINS, I.REVIEWED_PANCAKE_LANGUAGE,
                                  I.REVIEWED_CREDIT_QUOTES))


if __name__ == '__main__':
    unittest.main()
