"""Actual Unix PG importer coverage for the separately reviewed ledger successor.

Run with the owner-provisioned LMM_FINANCIAL_PG_FIXTURE_RECEIPT. Historical
fixtures and profiles keep their own contracts; all inherited PG tests run.
"""
from copy import deepcopy
import json
import uuid
import unittest

import test_financial_import as helpers
import test_financial_import_pg as historical_pg

F = historical_pg.F


class CurrentLedgerUnixPostgreSQLTests(historical_pg.UnixPostgreSQLTests):
    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        if cls.contract['import_profile'] != F.IDENTITY.CURRENT_LEDGER_SEED_CONTRACT:
            raise AssertionError('current-ledger fixture required')
        # The historical file helper's synthetic snapshot enumerates old
        # profiles. Adapt only its catalog fact for this successor test class.
        original = helpers.snapshot_fixture

        def current_snapshot(source):
            snapshot = original(source)
            snapshot['catalog_fingerprint_sha256'] = cls.contract['fresh_catalog_sha256']
            return snapshot

        helpers.snapshot_fixture = current_snapshot
        cls.addClassCleanup(setattr, helpers, 'snapshot_fixture', original)

    def test_current_profile_material_rejects_historical_and_mixed_binding(self):
        self.assertEqual(self.material['target']['source_contract']['import_profile'],
                         F.IDENTITY.CURRENT_LEDGER_SEED_CONTRACT)
        historical = F.IDENTITY.reviewed65()
        variants = []
        for key, value in (
            ('import_profile', F.IDENTITY.CORE65_SEED_CONTRACT),
            ('source_schema_revision', historical['source_schema_revision']),
            ('compiled_manifest_sha256', historical['compiled_manifest_sha256']),
            ('fresh_catalog_sha256', historical['fresh_catalog_sha256']),
            ('seed_query_sha256', '0' * 64),
        ):
            material = deepcopy(self.material)
            material['target']['source_contract'][key] = value
            variants.append((key, material, 'source_contract_changed'))
        for name in F.IDENTITY.CURRENT_LEDGER_COMPONENT_HASHES:
            material = deepcopy(self.material)
            material['target']['component_fingerprints'][name] = historical['components'][name][1]
            variants.append((name, material, 'component_source_changed'))
        material = deepcopy(self.material)
        material['target']['catalog_fingerprint_sha256'] = historical['fresh_catalog_sha256']
        variants.append(('catalog', material, 'target_fresh_catalog_mismatch'))
        for name, material, error in variants:
            with self.subTest(binding=name):
                with self.assertRaisesRegex(F.Error, error):
                    F.build_apply_sql(material)
                self.assertEqual(self.full_state(), self.before)

    def test_current_profile_seed_and_actual_post_import_catalog_check(self):
        # setUp already used the actual seed SQL in the identity importer and
        # captured the resulting target for the financial importer.
        self.assertEqual(self.receipt['fixtures']['financial']['seed_query_sha256'],
                         self.contract['seed_query_sha256'])
        app = 'lmm-financial-' + uuid.uuid4().hex
        committed = json.loads(self.sql(self.apply_sql, application_name=app).stdout)
        self.assertEqual(committed['status'], 'COMMITTED')
        after = self.full_state()
        self.assertEqual(self.query(F.IDENTITY.catalog_hash_sql(self.contract)),
                         self.contract['fresh_catalog_sha256'])
        resolution = json.loads(self.sql(F.build_resolve_sql(self.material, app)).stdout)
        self.assertEqual(resolution['status'], 'COMMITTED')
        self.assertEqual(json.loads(self.sql(self.apply_sql).stdout), committed)
        self.assertEqual(self.full_state(), after)


if __name__ == '__main__':
    unittest.main()
