"""Fresh compiled Core49 identity import, default guards and exact replay.

Uses the isolated template and transport of the financial PostgreSQL suite.
Every case runs on its own owned clone; no legacy or application DB is read.
"""
import os
import unittest
import test_financial_import_pg as pg
import test_migrate_legacy_snapshot as identity_cases
import legacy_cutover_import as cutover


@unittest.skipUnless(os.environ.get('LMM_FINANCIAL_PG_FIXTURE_RECEIPT'), 'compiled isolated fixture required')
class Fresh49IdentityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.transport = pg.UnixPostgreSQLTests
        cls.transport.setUpClass()

    def setUp(self):
        self.client = self.transport(methodName='runTest')
        self.addCleanup(self.client.doCleanups)
        self.client.create_test_database()
        self.client.install_fresh_core()
        self.client.inventory = self.client.query(pg.F.inventory_sql())
        plan = identity_cases.mapped([
            identity_cases.user(71, created_at=1700000001), identity_cases.user(72),
        ])
        contract = pg.F.IDENTITY.current_contract()
        target = {**contract,
            'catalog_fingerprint_sha256': self.client.query(pg.F.IDENTITY.catalog_hash_sql(contract)),
            'allow_initialized_core_seed': True,
            'initialized_core_seed_contract': pg.F.IDENTITY.CORE_SEED_CONTRACT,
            'initialized_core_seed_sha256': self.client.query(pg.F.IDENTITY.fresh_seed_hash_query()),
            'trust_policy_seed': self.client.query(pg.F.IDENTITY.trust_policy_seed_query()),
        }
        self.statement = pg.F.IDENTITY.build_import_sql(plan,
            migration_id='synthetic-core49-identity', source_sha256=identity_cases.SOURCE_SHA256,
            target_contract=target, manifest=identity_cases.manifest_for(plan),
            target_identity=self.client.query(pg.F.IDENTITY.IDENTITY_SQL))

    def rejected(self, mutation, message):
        self.client.sql(mutation)
        self.client.rejected_unchanged(self.statement, message)
        self.assertEqual(self.client.query('SELECT count(*) FROM core_identity.users'), 0)
        self.assertEqual(self.client.query('SELECT count(*) FROM core_migration.import_runs'), 0)

    def test_known_and_unknown_registration_time_and_exact_replay(self):
        self.client.sql(self.statement)
        self.assertEqual(self.client.query("SELECT jsonb_agg(jsonb_build_array(u.id,extract(epoch FROM a.created_at)::bigint) ORDER BY u.id) FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id"), [[71,1700000001],[72,0]])
        before = self.client.full_state()
        self.client.sql(self.statement)
        self.assertEqual(self.client.full_state(), before)

    def test_cutover_runner_preserves_source_time_and_unknown_epoch(self):
        users = [identity_cases.user(71, created_at=1700000001), identity_cases.user(72)]
        statements, stats = cutover.plan_identity({'users':users, 'two_fas':[], 'options':{}, 'oauth_bindings':[]}, identity_cases.SOURCE_SHA256)
        self.client.sql('BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL timezone=\'UTC\';' + '\n'.join(statements) + '\nCOMMIT;')
        self.assertEqual(self.client.query("SELECT jsonb_agg(jsonb_build_array(id,extract(epoch FROM created_at)::bigint) ORDER BY id) FROM core_identity.accounts"), [[71,1700000001],[72,0]])
        self.assertEqual((stats['registration_time_known_count'], stats['registration_time_unknown_count']), (1,1))

    def test_request_policy_must_keep_disabled_defaults(self):
        self.rejected('UPDATE core_request_limits.policy SET enabled=true', 'non-canonical initialized completion seed')

    def test_auto_policy_must_keep_empty_groups(self):
        self.rejected("UPDATE core_request_limits.auto_policy SET groups=ARRAY['changed']", 'non-canonical initialized completion seed')

    def test_promotion_secret_is_bound_even_with_valid_length(self):
        self.rejected('UPDATE core_promotions.secrets SET code_key=sha256(code_key)', 'initialized seed receipt mismatch')

    def test_channel_projection_must_start_ready(self):
        self.rejected('UPDATE core_channels.projection_state SET ready=false', 'non-canonical initialized completion seed')

    def test_appeals_sequence_keeps_its_exact_js_safe_maximum(self):
        self.rejected('ALTER SEQUENCE core_account_appeals.requests_id_seq MAXVALUE 9223372036854775807', 'non-canonical sequence definition')

    def test_current_offline_upgrade_receipt_rejects_fresh_import(self):
        value = "'" + '0' * 64 + "'"
        columns = 'operation_id,procedure,binding,predecessor_catalog_sha256,successor_catalog_sha256,preserved_data_sha256,preserved_permissions_sha256,runtime_permissions_sha256'
        self.rejected('INSERT INTO core_meta.offline_upgrades(' + columns + ") VALUES('synthetic-forbidden','core-offline-upgrade-v3','{}'," + ','.join([value] * 5) + ')', 'fresh target has upgrade receipts')


class CutoverTimeValidationTests(unittest.TestCase):
    def test_invalid_registration_times_fail_before_generating_an_import(self):
        for value in (False, True, '1700000001', -1, 253402300800):
            with self.subTest(value=value), self.assertRaisesRegex(SystemExit, 'registration_time_invalid'):
                cutover.plan_identity({'users':[identity_cases.user(71, created_at=value)], 'two_fas':[], 'options':{}, 'oauth_bindings':[]}, identity_cases.SOURCE_SHA256)


if __name__ == '__main__':
    unittest.main()
