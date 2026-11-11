"""Successor wiring with synthetic review receipts. No PG or source CLI."""
from copy import deepcopy
import hashlib
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import financial_import as F

I = F.IDENTITY


def synthetic_review():
    review = deepcopy(I.reviewed65())
    review.update(source_schema_revision='a' * 40,
                  compiled_manifest_sha256=I.CURRENT_LEDGER_MANIFEST_SHA256,
                  fresh_catalog_sha256=I.CURRENT_LEDGER_CATALOG_SHA256)
    for key in ('capture_receipt_sha256', 'actual_init_receipt_sha256', 'acl_receipt_sha256'):
        review[key] = hashlib.sha256(('synthetic-successor:' + key).encode()).hexdigest()
    for name, digest in I.CURRENT_LEDGER_COMPONENT_HASHES.items():
        review['components'][name] = (review['components'][name][0], digest)
    review['files'].update(I.CURRENT_LEDGER_FILE_HASHES)
    return review


class CurrentLedgerProfileTests(unittest.TestCase):
    def test_pending_revision_and_receipts_do_not_admit_profile(self):
        with patch.object(I, 'REVIEWED_CURRENT_LEDGER', None):
            with self.assertRaisesRegex(I.MappingError, 'current_ledger_profile_not_admitted'):
                I.reviewed_current_ledger()
            with self.assertRaisesRegex(I.MappingError, 'core65_assembly_not_reviewed'):
                I.current_contract()

    def test_real_source_binding_and_individually_pending_receipts(self):
        review = I.reviewed_current_ledger(require_receipts=False)
        self.assertEqual(review['source_schema_revision'],
                         'abd532d194fed45faca2f0d7ad890b5770f8cd54')
        self.assertEqual(review['capture_receipt_sha256'],
                         '9457f4532bcf0de91168c62bedf692e07689e847146b6bdfcf9a191eed263ba9')
        self.assertEqual(I.current_contract()['source_schema_revision'], review['source_schema_revision'])
        self.assertEqual(review['actual_init_receipt_sha256'],
                         '03f28c6c981d79f31b310c8459f4a49a093d5f5d6979c413eaeb738302ff1f6f')
        self.assertEqual(review['acl_receipt_sha256'],
                         '665a1f759f45ab8a376d65fe05cdc1503d29e3077f25315722a8fde9ec1be72f')
        for key in ('actual_init_receipt_sha256', 'acl_receipt_sha256'):
            pending = synthetic_review()
            pending[key] = None
            with self.subTest(receipt=key), patch.object(I, 'REVIEWED_CURRENT_LEDGER', pending):
                self.assertIsNone(I.reviewed_current_ledger(require_receipts=False)[key])
                with self.assertRaisesRegex(I.MappingError, 'current_ledger_' + key + '_pending'):
                    I.reviewed_profile()

    def test_exact_successor_assembly_and_full_seed_binding(self):
        historical = deepcopy(I.REVIEWED_CORE65)
        with patch.object(I, 'REVIEWED_CURRENT_LEDGER', synthetic_review()):
            contract = I.current_contract()
            self.assertEqual(contract['import_profile'], I.CURRENT_LEDGER_SEED_CONTRACT)
            self.assertEqual(contract['compiled_manifest_sha256'], I.CURRENT_LEDGER_MANIFEST_SHA256)
            self.assertEqual(contract['fresh_catalog_sha256'], I.CURRENT_LEDGER_CATALOG_SHA256)
            self.assertEqual(contract['source_schema_revision'], 'a' * 40)
            self.assertEqual(len(contract['component_fingerprints']), 65)
            changed = {name for name, pair in I.reviewed_current_ledger()['components'].items()
                       if pair != historical['components'][name]}
            self.assertEqual(changed, set(I.CURRENT_LEDGER_COMPONENT_HASHES))
            self.assertEqual(I.fresh_seed_hash_query(I.CURRENT_LEDGER_SEED_CONTRACT),
                             I.fresh_seed_hash_query(I.CORE65_SEED_CONTRACT))
            self.assertEqual(contract['seed_query_sha256'], hashlib.sha256(
                I.fresh_seed_hash_query(I.CORE65_SEED_CONTRACT).encode()).hexdigest())
            for name in I.fresh_business_tables(contract):
                self.assertIn(name, I.fresh_seed_hash_query(I.CURRENT_LEDGER_SEED_CONTRACT))
            for name in I.upgrade_receipt_tables(contract):
                self.assertIn(name, I.upgrade_receipts_guard_sql(contract))
        self.assertEqual(I.REVIEWED_CORE65, historical)

    def test_missing_receipts_wrong_target_and_unrelated_delta_rejected(self):
        original = synthetic_review()
        mutations = []
        for key in original:
            value = deepcopy(original)
            del value[key]
            mutations.append(value)
        for key in ('compiled_manifest_sha256', 'fresh_catalog_sha256', 'capture_receipt_sha256',
                    'actual_init_receipt_sha256', 'acl_receipt_sha256', 'source_schema_revision'):
            value = deepcopy(original)
            value[key] = None
            mutations.append(value)
        for name in original['components']:
            value = deepcopy(original)
            value['components'][name] = (value['components'][name][0], '0' * 64)
            mutations.append(value)
        for path in original['files']:
            value = deepcopy(original)
            value['files'][path] = '0' * 64
            mutations.append(value)
        value = deepcopy(original)
        value['source_schema_revision'] = I.reviewed65()['source_schema_revision']
        mutations.append(value)
        value = deepcopy(original)
        value['components'] = dict(reversed(list(value['components'].items())))
        mutations.append(value)
        for index, value in enumerate(mutations):
            with self.subTest(mutation=index), patch.object(I, 'REVIEWED_CURRENT_LEDGER', value):
                with self.assertRaises(I.MappingError):
                    I.reviewed_current_ledger()

    def test_selection_rejects_mixed_components_and_wrong_fresh_target(self):
        review = synthetic_review()
        root = I.ROOT
        target = 'services/core/src/core_schema/upgrade/fresh_target.json'
        original_bytes = I._component_bytes
        for name in review['components']:
            expression = review['components'][name][0]
            resolved = I.REVIEWED_SCHEMA_ALIASES.get(expression, (None, None, expression))[2]

            def changed_bytes(path, expr, producer):
                data = original_bytes(path, expr, producer)
                return data + b'\n-- one-byte-domain drift' if expr == resolved else data

            with self.subTest(component=name), patch.object(I, 'REVIEWED_CURRENT_LEDGER', review), \
                    patch.object(I, '_component_bytes', side_effect=changed_bytes):
                with self.assertRaisesRegex(I.MappingError, 'core65_assembly_not_reviewed'):
                    I.reviewed_profile(root)
        original_hash = I.sha256_file
        with patch.object(I, 'REVIEWED_CURRENT_LEDGER', review), patch.object(I, 'sha256_file',
                side_effect=lambda path: '0' * 64 if path == root / target else original_hash(path)):
            with self.assertRaisesRegex(I.MappingError, 'core65_assembly_not_reviewed'):
                I.reviewed_profile(root)

    def test_historical_selection_still_requires_historical_component_set(self):
        historical = I.reviewed65()
        with patch.object(I, 'sha256_file', side_effect=lambda path:
                historical['files'][str(path.relative_to(I.ROOT))]), \
                patch.object(I, '_matches_reviewed_components', return_value=True):
            self.assertEqual(I.reviewed_profile(), I.CORE65_SEED_CONTRACT)
        with patch.object(I, 'sha256_file', side_effect=lambda path:
                historical['files'][str(path.relative_to(I.ROOT))]), \
                patch.object(I, '_matches_reviewed_components', return_value=False):
            with self.assertRaisesRegex(I.MappingError, 'core65_assembly_not_reviewed'):
                I.reviewed_profile()

    def test_financial_material_pins_and_catalog_guards(self):
        # Existing fixture follows the normal material and SQL generation entry points.
        from test_financial_import import material_fixture, source_fixture, snapshot_fixture
        with patch.object(I, 'REVIEWED_CURRENT_LEDGER', synthetic_review()):
            contract = I.current_contract()
            source = source_fixture()
            snapshot = snapshot_fixture(source)
            snapshot['catalog_fingerprint_sha256'] = contract['fresh_catalog_sha256']
            material = material_fixture(source=source, snapshot=snapshot)
            self.assertEqual(material['target']['source_contract'], F.reviewed_source_contract(contract))
            sql = F.build_apply_sql(material)
            self.assertIn(I.CURRENT_LEDGER_CATALOG_SHA256, sql)
            for name in I.fresh_business_tables(contract):
                self.assertIn(name, sql)
            for key in ('import_profile', 'source_schema_revision', 'compiled_manifest_sha256',
                        'fresh_catalog_sha256', 'seed_query_sha256'):
                changed = deepcopy(material)
                changed['target']['source_contract'][key] = 'wrong'
                with self.subTest(pin=key), self.assertRaisesRegex(F.Error, 'source_contract_changed'):
                    F.build_apply_sql(changed)
            changed = deepcopy(material)
            changed['target']['catalog_fingerprint_sha256'] = I.reviewed65()['fresh_catalog_sha256']
            with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
                F.build_apply_sql(changed)
            source = source_fixture()
            snapshot = snapshot_fixture(source)
            snapshot['catalog_fingerprint_sha256'] = I.reviewed65()['fresh_catalog_sha256']
            with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
                material_fixture(source=source, snapshot=snapshot)


if __name__ == '__main__':
    unittest.main()
