"""Fresh68 profile regressions. Mock receipts are unit fixtures only."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import financial_import as F
import test_financial_import as helpers

I = F.IDENTITY


class CheckinsImportProfileTests(unittest.TestCase):
    def admitted_fixture(self):
        review = copy.deepcopy(I.REVIEWED_CHECKINS)
        review.update(actual_init_receipt_sha256='ab' * 32, acl_receipt_sha256='cd' * 32)
        return patch.object(I, 'REVIEWED_CHECKINS', review)

    def test_each_actual_receipt_is_independently_required(self):
        for init, acl, missing in ((None, None, 'actual_init'), ('ab' * 32, None, 'acl')):
            review = copy.deepcopy(I.REVIEWED_CHECKINS)
            review.update(actual_init_receipt_sha256=init, acl_receipt_sha256=acl)
            with patch.object(I, 'REVIEWED_CHECKINS', review):
                with self.assertRaisesRegex(I.MappingError, 'checkins_' + missing + '_receipt_sha256_pending'):
                    I.current_contract()

    def test_unit_fixture_selects_exact_final_source_and_manifest68(self):
        with self.admitted_fixture():
            contract = I.current_contract()
        self.assertEqual(contract['import_profile'], I.CHECKINS_SEED_CONTRACT)
        self.assertEqual(contract['source_schema_revision'], 'b62eeef8c628fea984e1939651754af09fef0f0f')
        self.assertEqual(contract['compiled_manifest_sha256'], I.CHECKINS_MANIFEST_SHA256)
        self.assertEqual(contract['fresh_catalog_sha256'], I.CHECKINS_CATALOG_SHA256)
        self.assertEqual(len(contract['component_fingerprints']), 68)
        self.assertIn('services/core/schema/checkins.sql', contract['source_file_hashes'])
        self.assertEqual(F.reviewed_source_contract(contract)['import_profile'], I.CHECKINS_SEED_CONTRACT)

    def test_old_target_cannot_mix_with_registry68(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registry = root / 'services/core/src/core_schema.rs'
            registry.parent.mkdir(parents=True)
            registry.write_bytes((I.ROOT / registry.relative_to(root)).read_bytes())
            target = root / 'services/core/src/core_schema/upgrade/fresh_target.json'
            target.parent.mkdir(parents=True)
            target.write_text(json.dumps({'component_count': 67,
                'manifest_sha256': I.PANCAKE_LANGUAGE_MANIFEST_SHA256,
                'catalog_sha256': I.PANCAKE_LANGUAGE_CATALOG_SHA256}))
            with self.assertRaisesRegex(I.MappingError, 'checkins_assembly_not_reviewed'):
                I.reviewed_profile(root)

    def test_review_seal_source_revision_prefix_expression_and_files_are_strict(self):
        with patch.object(I, 'CHECKINS_REVIEW_SHA256', '00' * 32):
            with self.assertRaisesRegex(I.MappingError, 'checkins_review_file_changed'):
                I.reviewed_checkins(require_receipts=False)
        for field, expected in (('revision', 'source_revision_not_reviewed'),
                                ('prefix', 'component_not_reviewed'),
                                ('expression', 'component_not_reviewed'),
                                ('source', 'source_review_incomplete')):
            review = copy.deepcopy(I.REVIEWED_CHECKINS)
            if field == 'revision':
                review['source_schema_revision'] = I.REVIEWED_PANCAKE_LANGUAGE['source_schema_revision']
            elif field == 'prefix':
                review['components']['ledger'][1] = 'ef' * 32
            elif field == 'expression':
                review['components']['checkins'][0] = 'include_str!("../schema/ledger.sql")'
            else:
                review['files']['services/core/src/runtime_checkins.rs'] = 'ef' * 32
            with patch.object(I, 'REVIEWED_CHECKINS', review):
                with self.assertRaisesRegex(I.MappingError, 'checkins_' + expected):
                    I.reviewed_checkins(require_receipts=False)

    def test_all_six_checkin_tables_enter_full_seed_and_empty_target_guards(self):
        with self.admitted_fixture():
            contract = I.current_contract()
        historical = copy.deepcopy(contract)
        historical['component_fingerprints'].pop('checkins')
        tables = tuple('core_checkins.' + name for name in I.CHECKINS_TABLE_ORDERS)
        self.assertEqual(len(tables), 6)
        self.assertEqual(I.fresh_business_tables(contract), I.fresh_business_tables(historical) + tables)
        self.assertEqual(I.upgrade_receipt_tables(contract), I.upgrade_receipt_tables(historical))
        seed = I.fresh_seed_hash_query(I.CHECKINS_SEED_CONTRACT)
        for table in tables:
            self.assertIn(table, seed)
            with self.admitted_fixture():
                self.assertIn(table, F.fresh_empty_tables())
        self.assertNotIn('core_checkins.', I.fresh_seed_hash_query(I.PANCAKE_LANGUAGE_SEED_CONTRACT))
        guard = I.profile_acl_guard_sql(contract)
        self.assertIn("n.nspname='core_checkins'", guard)
        self.assertIn("('policy_revisions','active_policy')", guard)
        self.assertIn("('days','claim_commands')", guard)
        self.assertIn('a.is_grantable', guard)

    def test_financial_prepare_and_validate_reject_catalog67(self):
        with self.admitted_fixture():
            source = helpers.source_fixture()
            snapshot = helpers.snapshot_fixture(source)
            snapshot['catalog_fingerprint_sha256'] = I.PANCAKE_LANGUAGE_CATALOG_SHA256
            with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
                helpers.material_fixture(source, snapshot)
            snapshot['catalog_fingerprint_sha256'] = I.CHECKINS_CATALOG_SHA256
            material = helpers.material_fixture(source, snapshot)
            F.validate_material(material)
            material['target']['catalog_fingerprint_sha256'] = I.PANCAKE_LANGUAGE_CATALOG_SHA256
            with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
                F.validate_material(material)

    def test_historical67_and66_reviews_keep_their_exact_seals(self):
        before = copy.deepcopy((I.REVIEWED_PANCAKE_LANGUAGE, I.REVIEWED_CREDIT_QUOTES))
        I.reviewed_checkins(require_receipts=False)
        self.assertEqual(before, (I.REVIEWED_PANCAKE_LANGUAGE, I.REVIEWED_CREDIT_QUOTES))
        self.assertEqual(I.sha256_file(I.PANCAKE_LANGUAGE_REVIEW_FILE), I.PANCAKE_LANGUAGE_REVIEW_SHA256)
        self.assertEqual(I.sha256_file(I.CREDIT_QUOTES_REVIEW_FILE), I.CREDIT_QUOTES_REVIEW_SHA256)


if __name__ == '__main__':
    unittest.main()
