#!/usr/bin/env python3
"""Real Core78 current-identity -> current-Key seam on an isolated fixture.

Reuse the exact c343 identity writer and frozen used-Key fixture/Unix COMMIT
proxy. A new fixture must expose a real active group named paid and have no
defined default group. It must live below the existing BACKUP_ROOT. This
driver never supplies fake production financial/fence evidence or enables
the production CLI; production closure validation remains separate.
"""
from copy import deepcopy
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import uuid

import current_key_production as M
import current_key_lineage as L
import test_current_identity_pg as IP
import test_migrate_current_runtime_keys_pg as K
import test_migrate_current_runtime_keys as KT
import test_current_key_production as T

ID = L.ID
check = K.check


class Regression(IP.Regression):
    def seed_current_source(self):
        self.seed_sessions_and_budget()
        self.seed_identity()
        check(self.group == 'paid', 'wrapper fixture requires active paid, not default')
        check(self.query("SELECT count(*) FROM public.routing_groups WHERE name='default'") == 0,
              'dormant fixture must not define default in any release')
        # Existing ordinary owner's source-supported paid membership is part
        # of this synthetic used target, not an authority added by the key
        # writer. Both test owners have real canonical opening funds so a
        # dormant rejection cannot be mistaken for an empty-wallet rejection.
        seed_facts = {'scope': 'synthetic-current-key-wrapper-funding',
                      'accounts': [103, 110], 'wallet_units_each': 1000000,
                      'reserved_units_each': 0, 'affiliate_units_each': 0}
        seed_hash = M.digest(seed_facts)
        seeds = ['BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;',
                 "INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) "
                 "VALUES(103,'paid',true,'synthetic-original-paid-membership');"]
        for account in seed_facts['accounts']:
            request = {'migration_id': 'synthetic-wrapper-funding', 'source_entity_key': 'synthetic-account:' + str(account),
                'source_archive_sha256': seed_hash, 'mapping_sha256': seed_hash, 'source_row_sha256': seed_hash,
                'account_id': account, 'available_units': 1000000, 'reserved_units': 0, 'affiliate_units': 0, 'observed_at': 1000}
            seeds.append('SELECT core_billing.import_opening_balance(' + M.literal(request) + '::jsonb);')
        seeds.append('COMMIT;')
        self.run_sql('\n'.join(seeds))
        # Source history owner108 keeps source auth_version1. Its genuine
        # identity completion reaches native3 through the two Core triggers.
        for row in self.bundle['source']['data']['users']:
            if row['id'] == 108:
                row['trust_level_override'] = 1
        source = self.bundle['source']
        source['source_identity_facts_sha256'] = ID.digest(ID.identity_facts(source['data']))
        source['current_policy_records'] = ID.mapped_current(source['data'], source['source_archive_sha256'])
        self.bundle['current'] = source['current_policy_records']
        for row in self.bundle['routing']['facts']['users']:
            if row['id'] == 103:
                row['group'] = 'paid'
        for row in self.bundle['routing']['facts']['options']:
            if row['key'] == 'GroupRatio':
                row['value'] = '{"paid":1}'
            elif row['key'] == 'UserUsableGroups':
                row['value'] = '{"paid":"Paid"}'
        ID.attach_routing(self.bundle, self.bundle['routing']['facts'])
        self.capabilities = {'channels': [{'id': 1, 'type': 1, 'status': 1, 'group': 'paid',
            'models': 'synthetic-model', 'priority': 0, 'weight': 1, 'model_mapping': None}],
            'abilities': [{'group': 'paid', 'model': 'synthetic-model', 'channel_id': 1,
                'enabled': True, 'priority': 0, 'weight': 1, 'tag': None}]}
        ID.attach_capabilities(self.bundle, self.capabilities)
        self.full_users = T.attach_canonical_test_rows(self.bundle)
        self.identity_id = 'synthetic-wrapper-identity-' + uuid.uuid4().hex[:12]
        identity = self.prepare_identity(self.identity_id)
        result = self.query(ID.build_sql(identity, fixture_nonce=self.nonce))
        check(result['status'] == 'COMMITTED', 'real current identity transaction did not commit')
        self.identity = identity
        self.completed.append('real_current_identity_committed_before_key_projection')
        self.tokens = [
            KT.token(id=self.session_ids[0], user_id=103, group='paid', one_time_reveal=False,
                     key='SyntheticPaidWrapperKey' + uuid.uuid4().hex),
            KT.token(id=self.session_ids[1], user_id=108, group='',
                     key='SyntheticDormantWrapperKey' + uuid.uuid4().hex),
            KT.token(id=self.session_ids[2], user_id=108, group='', deleted_at='2026-01-01T00:00:00Z',
                     key='SyntheticDeletedWrapperKey' + uuid.uuid4().hex),
            KT.token(id=self.session_ids[3], user_id=103, group='paid', unlimited_quota=True, remain_quota=-15,
                     key='SyntheticUnlimitedWrapperKey' + uuid.uuid4().hex),
        ]
        self.source = T.key_source(self.bundle, self.full_users, self.tokens)

    def prepare_current_keys(self, migration, *, hide_receipts=False):
        run = self.query("SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE migration_id=" + M.literal(self.identity_id))
        mapping = [{k: r[k] for k in L.MAP_FIELDS} for r in run['manifest']['mapping']]
        target = {'format_version': 1, 'source_archive_sha256': self.source['source_archive_sha256'],
            'source_target_id_map': mapping,
            'source_target_id_map_sha256': hashlib.sha256(M.P.id_map_bytes(mapping).encode()).hexdigest(),
            'users': self.query(M.C.native_users_sql())}
        actual = self.query(L.identity_authority_sql(mapping))
        bindings = L.validate(self.full_users, self.source, target, self.bundle['namespace'], run, actual)
        native = {name: self.query("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM " + name + " t")
                  for name in ('public.routing_groups', 'core_identity.user_routing_groups')}
        dormant = L.dormant_bindings({**self.capabilities, 'users': self.full_users}, self.source, target, run, native)
        snap = self.query(M.U.snapshot_sql(self.contract, self.inventory, migration))
        if hide_receipts:
            # Deliberately incomplete Python projection: the independent
            # database source-identity guard must still reject before nextval.
            snap['key_imports'] = []
        material = M.prepare_mechanics(self.source, target, T.credential_review(self.source, target, bindings, dormant),
            snapshot=snap, contract=self.contract, migration_id=migration, source_namespace=self.bundle['namespace'],
            runtime_roles=self.fixture['runtime_roles'], current_bindings=bindings, dormant_bindings=dormant,
            encryption_key=bytes(range(32)))
        M.F.write_private(self.output, migration + '-preview-' + str(self.counter) + '.private.json', M.canonical(material) + '\n')
        return material

    def current_proxy(self, material, before_commit):
        conn = deepcopy(self.connection)
        original = Path(conn['socket_directory']) / ('.s.PGSQL.' + str(conn['port']))
        directory = self.output / ('wrapper-proxy-' + uuid.uuid4().hex[:8])
        app = 'lmm-current-keys-' + uuid.uuid4().hex
        with K.CommitDropProxy(original, directory, conn['port'], before_commit=before_commit):
            conn['socket_directory'] = str(directory)
            code, _, _ = self.run_sql(M.build_synthetic_transaction_sql(material, self.nonce),
                                     connection=conn, app=app, expect_ok=False)
            check(code != 0, 'lost acknowledgment was reported as a successful command')
        for _ in range(50):
            active = self.query('SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=' + M.literal(app) + ')')
            if not active:
                break
            time.sleep(0.1)
        check(not active, 'original write still active: do not retry')
        return self.query(M.build_resolve_sql(material, app))

    def current_concurrent_fence(self, material):
        env = {k: v for k, v in os.environ.items() if not k.startswith('PG')}
        env.update(M.I.socket_connection(self.connection))
        env.update(PGAPPNAME='wrapper-fence-' + uuid.uuid4().hex,
                   PGOPTIONS=M.I.psql_options(self.connection, readonly=False))
        proc = subprocess.Popen(['psql', '-X', '--no-password', '-qAt', '-v', 'ON_ERROR_STOP=1'],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True, env=env)
        try:
            proc.stdin.write('SELECT pg_advisory_lock(' + str(M.FENCE) + '); SELECT pg_sleep(20);\n')
            proc.stdin.close()
            for _ in range(50):
                if self.query('SELECT NOT pg_try_advisory_xact_lock(' + str(M.FENCE) + ')'):
                    break
                time.sleep(0.1)
            else:
                raise AssertionError('competing fixture writer did not acquire lock')
            self.reject(M.build_synthetic_transaction_sql(material, self.nonce), 'current_keys_import_busy')
        finally:
            if proc.poll() is None:
                proc.send_signal(signal.SIGINT)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)

    def run_current(self):
        self.seed_current_source()
        migration = 'synthetic-current-wrapper-' + uuid.uuid4().hex[:12]
        material = self.prepare_current_keys(migration)
        check(material['records'][1]['source_auth_version'] == 1
              and material['records'][1]['expected_target_auth_version'] == 3,
              'ordinary committed generation bridge did not bind source1 to actual3')
        self.current_concurrent_fence(material)
        self.run_sql('BEGIN ISOLATION LEVEL SERIALIZABLE; UPDATE core_charging.budgets SET limit_credits=10000,revision=revision+1 WHERE id='
                     + str(self.native_budget_id) + '; COMMIT;')
        self.reject(M.build_synthetic_transaction_sql(material, self.nonce), 'current_keys_target_state_changed')
        self.completed += ['actual_competing_writer_rejected_without_sequence_allocation',
                           'native_budget_CAS_drift_rejected_without_sequence_allocation']
        material = self.prepare_current_keys(migration)
        before = self.state(migration)
        absent = self.current_proxy(material, True)
        check(absent['status'] == 'UNKNOWN' and not absent['automatic_retry_allowed'], 'receipt absence was called rollback')
        after = self.state(migration)
        check(before['tables'] == after['tables'], 'precommit interruption retained key mutations')
        check(all(after['sequences'][s]['last_value'] > before['sequences'][s]['last_value'] for s in M.SEQUENCES),
              'ordinary nonrollbackable sequence gaps were lost')
        self.reject(M.build_synthetic_transaction_sql(material, self.nonce), 'current_keys_target_state_changed')
        fresh = self.prepare_current_keys(migration)
        committed = self.current_proxy(fresh, False)
        check(committed['status'] == 'COMMITTED' and not committed['automatic_retry_allowed'], 'real lost COMMIT did not resolve')
        state = self.state(migration)
        check(self.query(M.build_synthetic_transaction_sql(fresh, self.nonce))['status'] == 'COMMITTED'
              and self.state(migration) == state, 'exact replay changed rows or sequences')
        duplicate = self.prepare_current_keys(migration + '-duplicate', hide_receipts=True)
        self.reject(M.build_synthetic_transaction_sql(duplicate, self.nonce), 'current_keys_source_identity_already_imported')
        self.completed += ['real_precommit_disconnect_rolls_back_rows_with_monotone_ID_gaps',
            'unknown_commit_resolved_readonly_without_automatic_retry', 'exact_replay_does_not_allocate_or_change_state',
            'actual_source_identity_duplicate_rejected_even_when_python_receipts_are_hidden']
        for i, row in enumerate(fresh['records']):
            actual = self.query('SELECT jsonb_build_object(\'revoked\',revoked_at IS NOT NULL,\'version\',user_version,\'kind\',kind) '
                'FROM core_identity.credentials WHERE id=' + str(row['target_credential_id']))
            check(actual['kind'] == 'api_key' and actual['version'] == row['expected_target_auth_version']
                  and actual['revoked'] == (i == 2), 'actual appended credential did not preserve key authority')
        check(self.query("SELECT count(*) FROM public.routing_groups WHERE name='default'") == 0,
              'key import created a native default route')
        self.completed += ['source_auth_version_not_rewritten_actual_identity_generation_used',
            'deleted_dormant_key_remains_revoked_no_route_was_created',
            'all_preexisting_sessions_budgets_wallet_and_identity_history_preserved']
        probes = {'format_version': 1, 'scope': 'current-key-production-seam-synthetic-runtime-probes',
            'database': self.connection['database'], 'fixture_nonce': self.nonce,
            'session': {'bearer': self.session_bearers[0], 'credential_id': self.session_ids[0]},
            'encryption_key_hex': bytes(range(32)).hex(),
            'keys': [{'bearer': raw['key'], 'credential_id': row['target_credential_id'],
                'user_id': row['target_user_id'], 'account_id': row['target_personal_account_id'],
                'expected_identity_auth': 'revoked' if row['revoked'] else 'allowed',
                'group': row['native_effective_group'], 'expected_model_admission': 'denied' if i in (1, 2) else 'allowed',
                'used_units': raw['used_quota'], 'remaining_units': raw['remain_quota'],
                'unlimited': raw['unlimited_quota'], 'recoverable': not row['one_time_reveal']}
                for i, (raw, row) in enumerate(zip(self.tokens, fresh['records']))],
            'required_runtime_assertions': ['existing Session still authenticates',
                'source1 native3 current-history key authenticates using committed generation',
                'active dormant key has no callable model; deleted dormant key authentication fails',
                'paid key retains actual supported model admission, budget used70 remain30',
                'unlimited key keeps negative displayed remainder without a finite cap',
                'recoverable secret decrypts with allocated native ID and one-time secret is unrecoverable']}
        M.F.write_private(self.output, 'runtime-probes.private.json', M.canonical(probes) + '\n')
        result = {'status': 'MECHANICS_VERIFIED_RUNTIME_PENDING', 'checks': self.completed,
            'production_closure_tested': False, 'production_authorized': False, 'production_access': False,
            'runtime_authentication_and_quota': 'NOT_RUN', 'product_rebuilt': False}
        M.F.write_private(self.output, 'result.public.json', M.canonical(result) + '\n')
        print(json.dumps(result, sort_keys=True))


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--fixture', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args(argv)
    os.umask(0o077)
    M.frozen_dependencies()
    regression = Regression(M.C.load_json(args.fixture), args.output)
    regression.run_current()


if __name__ == '__main__':
    main()
