"""Actual owner-provisioned Unix PG18 coverage for the fresh67 importer.

Requires genuine final Core67 init/seed and ACL receipts. No synthetic receipt
patching is used here. The inherited financial regressions retain their scope.
"""
from copy import deepcopy
import json
import unittest
import uuid

import test_financial_import as helpers
import test_financial_import_pg as historical_pg

F = historical_pg.F
I = F.IDENTITY


class PancakeLanguageUnixPostgreSQLTests(historical_pg.UnixPostgreSQLTests):
    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        if cls.contract['import_profile'] != I.PANCAKE_LANGUAGE_SEED_CONTRACT:
            raise AssertionError('fresh67 pancake language fixture required')
        original = helpers.snapshot_fixture

        def current_snapshot(source):
            snapshot = original(source)
            snapshot['catalog_fingerprint_sha256'] = cls.contract['fresh_catalog_sha256']
            return snapshot

        helpers.snapshot_fixture = current_snapshot
        cls.addClassCleanup(setattr, helpers, 'snapshot_fixture', original)

    def test_profile67_rejects_mixed66_material_without_writes(self):
        self.assertEqual(self.material['target']['source_contract']['import_profile'], I.PANCAKE_LANGUAGE_SEED_CONTRACT)
        historical = I.reviewed_credit_quotes()
        for key, value in (('import_profile', I.CREDIT_QUOTES_SEED_CONTRACT),
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
        material['target']['component_fingerprints'].pop('subscription-pancake-language-optional')
        with self.assertRaisesRegex(F.Error, 'component_source_changed'):
            F.build_apply_sql(material)
        material = deepcopy(self.material)
        material['target']['catalog_fingerprint_sha256'] = historical['fresh_catalog_sha256']
        with self.assertRaisesRegex(F.Error, 'target_fresh_catalog_mismatch'):
            F.build_apply_sql(material)
        self.assertEqual(self.full_state(), self.before)

    def test_actual67_seed_catalog_opening_resolution_and_replay(self):
        self.assertEqual(self.receipt['fixtures']['financial']['seed_query_sha256'], self.contract['seed_query_sha256'])
        self.assertIn('core_meta.pancake_language_upgrades', self.inventory['tables'])
        app = 'lmm-financial-' + uuid.uuid4().hex
        committed = json.loads(self.sql(self.apply_sql, application_name=app).stdout)
        self.assertEqual(committed['status'], 'COMMITTED')
        after = self.full_state()
        self.assertEqual(self.query(I.catalog_hash_sql(self.contract)), I.PANCAKE_LANGUAGE_CATALOG_SHA256)
        self.assertEqual(json.loads(self.sql(F.build_resolve_sql(self.material, app)).stdout)['status'], 'COMMITTED')
        self.assertEqual(json.loads(self.sql(self.apply_sql).stdout), committed)
        self.assertEqual(self.full_state(), after)

    def test_new_receipt_acl_and_nonempty_upgrade_are_rejected(self):
        table = 'core_meta.pancake_language_upgrades'
        self.assertFalse(self.query('SELECT ' + I.profile_acl_guard_sql(self.contract)))
        self.sql('GRANT SELECT(upgrade_id) ON ' + table + ' TO ' + self.runtime_role)
        self.assertTrue(self.query('SELECT ' + I.profile_acl_guard_sql(self.contract)))
        self.sql('REVOKE SELECT(upgrade_id) ON ' + table + ' FROM ' + self.runtime_role)
        self.assertFalse(self.query('SELECT ' + I.profile_acl_guard_sql(self.contract)))
        self.sql('INSERT INTO ' + table + '(upgrade_id,request_sha256,request,preserved_data_sha256,'
                 'preserved_permissions_sha256,successor_catalog_sha256) VALUES('
                 "'synthetic-forbidden',decode(repeat('0',64),'hex'),'{}'::jsonb,"
                 "decode(repeat('0',64),'hex'),decode(repeat('0',64),'hex'),decode(repeat('0',64),'hex'))")
        self.assertTrue(self.query('SELECT ' + I.upgrade_receipts_guard_sql(self.contract)))
        self.rejected_unchanged(self.apply_sql, 'financial_upgrade_receipts_not_empty')

    def test_originals_decimal_sql_preserves_unknown_source_and_strict_replay(self):
        from decimal import Decimal
        import legacy_pancake_subscriptions as W
        from test_legacy_pancake_subscriptions import OriginalTests

        source, review, _ = OriginalTests().fixture()
        archive = self.material['source_archive_sha256']
        source['source_archive_sha256'] = review['source_archive_sha256'] = archive
        user = self.material['native_users'][0]
        row, binding = source['orders'][0], review['orders'][0]
        row['user_id'] = user['id']
        row['plan_snapshot'].update(allow_wallet_overflow=True,
            precise_price=Decimal('1234567890.123456789'), integral_decimal=Decimal('1'))
        binding.update(account_id=user['personal_account_id'], source_row_sha256=F.digest(row),
                       disposition='retain-unknown')
        material = W.prepare_material(self.material, source, review, migration_id='synthetic-originals-decimal')
        self.assertEqual(material['records'][0]['disposition'], 'UNKNOWN')
        sql = W.build_apply_sql(material)
        self.sql(sql)
        stored = "SELECT manifest->'material' FROM core_migration.import_runs WHERE migration_id='synthetic-originals-decimal'"
        for key, expected in (('precise_price', '1234567890.123456789'), ('integral_decimal', '1')):
            self.assertEqual(self.query("SELECT m #>> '{source,orders,0,plan_snapshot," + key + "}' FROM (" + stored + ") q(m)"), expected)
            self.assertEqual(self.query("SELECT result #>> '{source_snapshot," + key + "}' FROM core_subscriptions.commands"
                                        " WHERE operation_key=" + F.literal(material['records'][0]['operation_key'])), expected)
        self.assertEqual(self.query('SELECT count(*) FROM core_subscriptions.pancake_orders'), 0)
        after = self.full_state()
        self.sql(sql)
        self.assertEqual(self.full_state(), after)
        source['orders'][0]['plan_snapshot']['precise_price'] = Decimal('1234567890.123456788')
        review['orders'][0]['source_row_sha256'] = F.digest(source['orders'][0])
        changed = W.prepare_material(self.material, source, review, migration_id=material['migration_id'])
        self.rejected_unchanged(W.build_apply_sql(changed), 'original_replay_conflict')


if __name__ == '__main__':
    unittest.main()
