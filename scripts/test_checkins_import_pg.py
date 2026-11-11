"""Actual compiled-Core68 Unix PG18 fixture regressions.

Requires genuine release init/ACL admission and an owner-provisioned fixture.
Historical profiles and their test modules retain their own immutable bindings.
"""
from copy import deepcopy
import json
import unittest
import uuid

import test_financial_import as helpers
import test_financial_import_pg as historical_pg

F = historical_pg.F
I = F.IDENTITY


class CheckinsUnixPostgreSQLTests(historical_pg.UnixPostgreSQLTests):
    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        if cls.contract['import_profile'] != I.CHECKINS_SEED_CONTRACT:
            raise AssertionError('fresh68 checkins fixture required')
        original = helpers.snapshot_fixture

        def current_snapshot(source):
            snapshot = original(source)
            snapshot['catalog_fingerprint_sha256'] = cls.contract['fresh_catalog_sha256']
            return snapshot

        helpers.snapshot_fixture = current_snapshot
        cls.addClassCleanup(setattr, helpers, 'snapshot_fixture', original)

    def test_profile68_refuses_mixed67_material_without_writes(self):
        historical = I.reviewed_pancake_language()
        for key, value in (('import_profile', I.PANCAKE_LANGUAGE_SEED_CONTRACT),
                           ('source_schema_revision', historical['source_schema_revision']),
                           ('compiled_manifest_sha256', historical['compiled_manifest_sha256']),
                           ('fresh_catalog_sha256', historical['fresh_catalog_sha256'])):
            material = deepcopy(self.material)
            material['target']['source_contract'][key] = value
            with self.subTest(binding=key):
                with self.assertRaisesRegex(F.Error, 'source_contract_changed'):
                    F.build_apply_sql(material)
                self.assertEqual(self.full_state(), self.before)
        material = deepcopy(self.material)
        material['target']['component_fingerprints'].pop('checkins')
        with self.assertRaisesRegex(F.Error, 'component_source_changed'):
            F.build_apply_sql(material)
        material = deepcopy(self.material)
        material['target']['catalog_fingerprint_sha256'] = historical['fresh_catalog_sha256']
        with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
            F.build_apply_sql(material)
        self.assertEqual(self.full_state(), self.before)

    def test_full68_seed_catalog_opening_resolution_and_replay(self):
        self.assertEqual(self.receipt['fixtures']['financial']['seed_query_sha256'], self.contract['seed_query_sha256'])
        for name in I.CHECKINS_TABLE_ORDERS:
            table = 'core_checkins.' + name
            self.assertIn(table, self.inventory['tables'])
            self.assertEqual(self.query('SELECT count(*) FROM ' + table), 0)
        app = 'lmm-financial-' + uuid.uuid4().hex
        committed = json.loads(self.sql(self.apply_sql, application_name=app).stdout)
        self.assertEqual(committed['status'], 'COMMITTED')
        after = self.full_state()
        self.assertEqual(self.query(I.catalog_hash_sql(self.contract)), I.CHECKINS_CATALOG_SHA256)
        self.assertEqual(json.loads(self.sql(F.build_resolve_sql(self.material, app)).stdout)['status'], 'COMMITTED')
        self.assertEqual(json.loads(self.sql(self.apply_sql).stdout), committed)
        self.assertEqual(self.full_state(), after)

    def test_owner_only_policy_seed_and_runtime_append_only_acl_guards(self):
        guard = 'SELECT ' + I.profile_acl_guard_sql(self.contract)
        self.assertFalse(self.query(guard))
        for table, privilege in (('policy_revisions', 'UPDATE'), ('active_policy', 'INSERT'),
                                  ('seed_receipts', 'SELECT'), ('owner_commands', 'INSERT'),
                                  ('days', 'DELETE'), ('claim_commands', 'UPDATE')):
            relation = 'core_checkins.' + table
            with self.subTest(table=table, privilege=privilege):
                self.sql('GRANT ' + privilege + ' ON ' + relation + ' TO ' + self.runtime_role)
                self.assertTrue(self.query(guard))
                self.sql('REVOKE ' + privilege + ' ON ' + relation + ' FROM ' + self.runtime_role)
                self.assertFalse(self.query(guard))
        self.sql('GRANT SELECT ON core_checkins.policy_revisions,core_checkins.active_policy TO ' + self.runtime_role)
        self.sql('GRANT SELECT,INSERT ON core_checkins.days,core_checkins.claim_commands TO ' + self.runtime_role)
        self.assertFalse(self.query(guard))
        self.assertEqual(self.full_state(), self.before)
        archive = self.material['source_archive_sha256']
        self.sql('INSERT INTO core_checkins.seed_receipts(seed_id,migration_id,identity_migration_id,'
                 'source_archive_sha256,mapping_sha256,source_cutoff,source_options_sha256,settings,result) VALUES('
                 "'synthetic-seed','synthetic-identity','synthetic-identity'," + F.literal(archive) +
                 ",repeat('a',64),1,repeat('b',64),'{}'::jsonb,'{}'::jsonb)")
        with self.assertRaisesRegex(F.Error, 'fresh49_business_or_upgrade_records_not_empty'):
            self.capture_material()


if __name__ == '__main__':
    unittest.main()
