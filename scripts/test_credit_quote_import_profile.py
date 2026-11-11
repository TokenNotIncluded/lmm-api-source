"""Fresh66 source/guard regressions. Mock receipt hashes grant no admission."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import financial_import as F
import test_financial_import as historical_helpers

I = F.IDENTITY


class CreditQuoteImportProfileTests(unittest.TestCase):
    def admitted_fixture(self):
        review = copy.deepcopy(I.REVIEWED_CREDIT_QUOTES)
        review.update(actual_init_receipt_sha256='ab' * 32, acl_receipt_sha256='cd' * 32)
        return patch.object(I, 'REVIEWED_CREDIT_QUOTES', review)

    def test_missing_init_receipt_remains_pending(self):
        review = copy.deepcopy(I.REVIEWED_CREDIT_QUOTES)
        review.update(actual_init_receipt_sha256=None, acl_receipt_sha256=None)
        with patch.object(I, 'REVIEWED_CREDIT_QUOTES', review):
            with self.assertRaisesRegex(I.MappingError, 'credit_quotes_actual_init_receipt_sha256_pending'):
                I.current_contract()

    def test_init_alone_does_not_admit_without_actual_acl(self):
        review = copy.deepcopy(I.REVIEWED_CREDIT_QUOTES)
        review['actual_init_receipt_sha256'] = 'ab' * 32
        review['acl_receipt_sha256'] = None
        with patch.object(I, 'REVIEWED_CREDIT_QUOTES', review):
            with self.assertRaisesRegex(I.MappingError, 'credit_quotes_acl_receipt_sha256_pending'):
                I.current_contract()

    def test_synthetic_receipts_select_exact_core184_manifest66(self):
        with self.admitted_fixture():
            contract = I.current_contract()
        self.assertEqual(contract['import_profile'], I.CREDIT_QUOTES_SEED_CONTRACT)
        self.assertEqual(contract['source_schema_revision'], '184569d74ba5481dd23a560087e817f7943783de')
        self.assertEqual(contract['compiled_manifest_sha256'], I.CREDIT_QUOTES_MANIFEST_SHA256)
        self.assertEqual(contract['fresh_catalog_sha256'], I.CREDIT_QUOTES_CATALOG_SHA256)
        self.assertEqual(len(contract['component_fingerprints']), 66)
        for name in ('payment_credit_quotes.sql', 'payment_credit_quote_referrals.sql'):
            self.assertIn('services/core/schema/' + name, contract['source_file_hashes'])

    def test_old_target_pin_cannot_mix_with_registry66(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registry = root / 'services/core/src/core_schema.rs'
            registry.parent.mkdir(parents=True)
            registry.write_bytes((I.ROOT / registry.relative_to(root)).read_bytes())
            target = root / 'services/core/src/core_schema/upgrade/fresh_target.json'
            target.parent.mkdir(parents=True)
            target.write_text(json.dumps({'component_count': 65,
                'manifest_sha256': I.CURRENT_LEDGER_MANIFEST_SHA256,
                'catalog_sha256': I.CURRENT_LEDGER_CATALOG_SHA256}))
            with self.admitted_fixture():
                with self.assertRaisesRegex(I.MappingError, 'credit_quotes_assembly_not_reviewed'):
                    I.reviewed_profile(root)

    def test_changed_component_prefix_alias_or_source_is_rejected(self):
        for field in ('prefix', 'alias', 'source'):
            review = copy.deepcopy(I.REVIEWED_CREDIT_QUOTES)
            if field == 'prefix':
                review['components']['ledger'][1] = 'ef' * 32
            elif field == 'alias':
                review['components']['payment-credit-quotes'][0] = 'include_str!("../schema/payment_credit_quotes.sql")'
            else:
                review['files']['services/core/src/payments.rs'] = 'ef' * 32
            with patch.object(I, 'REVIEWED_CREDIT_QUOTES', review):
                with self.assertRaisesRegex(I.MappingError, 'credit_quotes_(component_not_reviewed|source_review_incomplete)'):
                    I.reviewed_credit_quotes(require_receipts=False)

    def test_initial8c_cannot_be_relabelled_final_pin_source(self):
        review = copy.deepcopy(I.REVIEWED_CREDIT_QUOTES)
        review['source_schema_revision'] = '8c521781b95079e82cbf6250df3473abdeec9512'
        with patch.object(I, 'REVIEWED_CREDIT_QUOTES', review):
            with self.assertRaisesRegex(I.MappingError, 'credit_quotes_source_revision_not_reviewed'):
                I.reviewed_credit_quotes(require_receipts=False)

    def test_profile_json_digest_is_sealed(self):
        with patch.object(I, 'CREDIT_QUOTES_REVIEW_SHA256', '00' * 32):
            with self.assertRaisesRegex(I.MappingError, 'credit_quotes_review_file_changed'):
                I.reviewed_credit_quotes(require_receipts=False)

    def test_financial_source_contract_binds_all_successor_pins(self):
        with self.admitted_fixture():
            contract = I.current_contract()
        source = F.reviewed_source_contract(contract)
        for key in ('import_profile', 'source_schema_revision', 'compiled_manifest_sha256',
                    'fresh_catalog_sha256', 'seed_query_sha256'):
            self.assertEqual(source[key], contract[key])

    def test_financial_prepare_refuses_historical_catalog_for_profile66(self):
        with self.admitted_fixture():
            source = historical_helpers.source_fixture()
            snapshot = historical_helpers.snapshot_fixture(source)
            snapshot['catalog_fingerprint_sha256'] = I.CURRENT_LEDGER_CATALOG_SHA256
            with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
                historical_helpers.material_fixture(source, snapshot)

    def test_financial_validate_refuses_historical_catalog_for_profile66(self):
        with self.admitted_fixture():
            source = historical_helpers.source_fixture()
            snapshot = historical_helpers.snapshot_fixture(source)
            snapshot['catalog_fingerprint_sha256'] = I.CREDIT_QUOTES_CATALOG_SHA256
            material = historical_helpers.material_fixture(source, snapshot)
            F.validate_material(material)
            material['target']['catalog_fingerprint_sha256'] = I.CURRENT_LEDGER_CATALOG_SHA256
            with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
                F.validate_material(material)

    def test_full_seed_query_and_existing_guards_are_preserved(self):
        self.assertEqual(I.fresh_seed_hash_query(I.CREDIT_QUOTES_SEED_CONTRACT),
                         I.fresh_seed_hash_query(I.CURRENT_LEDGER_SEED_CONTRACT))
        with self.admitted_fixture():
            contract = I.current_contract()
        historical = copy.deepcopy(contract)
        historical['component_fingerprints'].pop('payment-credit-quotes')
        for guard in (I.upgrade_receipt_tables, I.fresh_business_tables, I.profile_acl_guard_sql):
            self.assertEqual(guard(contract), guard(historical))

    def test_historical_review_objects_remain_unchanged(self):
        before = copy.deepcopy((I.REVIEWED_CORE65, I.REVIEWED_CURRENT_LEDGER))
        I.reviewed_credit_quotes(require_receipts=False)
        self.assertEqual(before, (I.REVIEWED_CORE65, I.REVIEWED_CURRENT_LEDGER))


if __name__ == '__main__':
    unittest.main()
