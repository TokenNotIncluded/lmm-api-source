"""Fresh67 profile regressions. Mock receipts are unit fixtures only."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import financial_import as F
import test_financial_import as helpers

I = F.IDENTITY


class PancakeLanguageImportProfileTests(unittest.TestCase):
    def admitted_fixture(self):
        review = copy.deepcopy(I.REVIEWED_PANCAKE_LANGUAGE)
        review.update(actual_init_receipt_sha256='ab' * 32, acl_receipt_sha256='cd' * 32)
        return patch.object(I, 'REVIEWED_PANCAKE_LANGUAGE', review)

    def test_each_actual_receipt_is_independently_required(self):
        for init, acl, missing in ((None, None, 'actual_init'), ('ab' * 32, None, 'acl')):
            review = copy.deepcopy(I.REVIEWED_PANCAKE_LANGUAGE)
            review.update(actual_init_receipt_sha256=init, acl_receipt_sha256=acl)
            with patch.object(I, 'REVIEWED_PANCAKE_LANGUAGE', review):
                with self.assertRaisesRegex(I.MappingError, 'pancake_language_' + missing + '_receipt_sha256_pending'):
                    I.current_contract()

    def test_unit_fixture_selects_exact_final_source_and_manifest67(self):
        with self.admitted_fixture():
            contract = I.current_contract()
        self.assertEqual(contract['import_profile'], I.PANCAKE_LANGUAGE_SEED_CONTRACT)
        self.assertEqual(contract['source_schema_revision'], '0510c2455dabcb63e33c719ba52ac189d62612bc')
        self.assertEqual(contract['compiled_manifest_sha256'], I.PANCAKE_LANGUAGE_MANIFEST_SHA256)
        self.assertEqual(contract['fresh_catalog_sha256'], I.PANCAKE_LANGUAGE_CATALOG_SHA256)
        self.assertEqual(len(contract['component_fingerprints']), 67)
        self.assertIn('services/core/schema/subscription_pancake_language_optional.sql', contract['source_file_hashes'])
        self.assertEqual(F.reviewed_source_contract(contract)['import_profile'], I.PANCAKE_LANGUAGE_SEED_CONTRACT)

    def test_old_target_cannot_mix_with_registry67(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registry = root / 'services/core/src/core_schema.rs'
            registry.parent.mkdir(parents=True)
            registry.write_bytes((I.ROOT / registry.relative_to(root)).read_bytes())
            target = root / 'services/core/src/core_schema/upgrade/fresh_target.json'
            target.parent.mkdir(parents=True)
            target.write_text(json.dumps({'component_count': 66,
                'manifest_sha256': I.CREDIT_QUOTES_MANIFEST_SHA256,
                'catalog_sha256': I.CREDIT_QUOTES_CATALOG_SHA256}))
            with self.assertRaisesRegex(I.MappingError, 'pancake_language_assembly_not_reviewed'):
                I.reviewed_profile(root)

    def test_review_seal_source_revision_prefix_expression_and_files_are_strict(self):
        with patch.object(I, 'PANCAKE_LANGUAGE_REVIEW_SHA256', '00' * 32):
            with self.assertRaisesRegex(I.MappingError, 'pancake_language_review_file_changed'):
                I.reviewed_pancake_language(require_receipts=False)
        for field, expected in (('revision', 'source_revision_not_reviewed'),
                                ('prefix', 'component_not_reviewed'),
                                ('expression', 'component_not_reviewed'),
                                ('source', 'source_review_incomplete')):
            review = copy.deepcopy(I.REVIEWED_PANCAKE_LANGUAGE)
            if field == 'revision':
                review['source_schema_revision'] = I.REVIEWED_CREDIT_QUOTES['source_schema_revision']
            elif field == 'prefix':
                review['components']['ledger'][1] = 'ef' * 32
            elif field == 'expression':
                review['components']['subscription-pancake-language-optional'][0] = 'include_str!("../schema/ledger.sql")'
            else:
                review['files']['services/core/src/subscriptions.rs'] = 'ef' * 32
            with patch.object(I, 'REVIEWED_PANCAKE_LANGUAGE', review):
                with self.assertRaisesRegex(I.MappingError, 'pancake_language_' + expected):
                    I.reviewed_pancake_language(require_receipts=False)

    def test_new_receipt_enters_seed_empty_target_and_owner_acl_guards(self):
        with self.admitted_fixture():
            contract = I.current_contract()
        historical = copy.deepcopy(contract)
        historical['component_fingerprints'].pop('subscription-pancake-language-optional')
        receipt = 'core_meta.pancake_language_upgrades'
        self.assertEqual(I.upgrade_receipt_tables(contract), I.upgrade_receipt_tables(historical) + (receipt,))
        self.assertIn(receipt, I.upgrade_receipts_guard_sql(contract))
        self.assertIn(receipt, I.profile_acl_guard_sql(contract))
        self.assertIn(receipt, I.fresh_seed_hash_query(I.PANCAKE_LANGUAGE_SEED_CONTRACT))
        self.assertNotIn(receipt, I.fresh_seed_hash_query(I.CREDIT_QUOTES_SEED_CONTRACT))
        self.assertEqual(I.fresh_business_tables(contract), I.fresh_business_tables(historical))

    def test_financial_prepare_and_validate_reject_catalog66(self):
        with self.admitted_fixture():
            source = helpers.source_fixture()
            snapshot = helpers.snapshot_fixture(source)
            snapshot['catalog_fingerprint_sha256'] = I.CREDIT_QUOTES_CATALOG_SHA256
            with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
                helpers.material_fixture(source, snapshot)
            snapshot['catalog_fingerprint_sha256'] = I.PANCAKE_LANGUAGE_CATALOG_SHA256
            material = helpers.material_fixture(source, snapshot)
            F.validate_material(material)
            material['target']['catalog_fingerprint_sha256'] = I.CREDIT_QUOTES_CATALOG_SHA256
            with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
                F.validate_material(material)

    def test_historical66_review_and_seed_domain_remain_unchanged(self):
        historical = copy.deepcopy(I.REVIEWED_CREDIT_QUOTES)
        I.reviewed_pancake_language(require_receipts=False)
        self.assertEqual(historical, I.REVIEWED_CREDIT_QUOTES)
        self.assertEqual(I.fresh_seed_hash_query(I.CREDIT_QUOTES_SEED_CONTRACT),
                         I.fresh_seed_hash_query(I.CURRENT_LEDGER_SEED_CONTRACT))


if __name__ == '__main__':
    unittest.main()
