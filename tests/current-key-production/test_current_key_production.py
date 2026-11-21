"""Synthetic policy/receipt regressions. These are not financial approvals."""
from copy import deepcopy
import hashlib
import unittest

import current_key_production as M
import current_key_lineage as L
import test_current_identity as IT
import test_migrate_current_runtime_keys as KT

ID = L.ID


def attach_canonical_test_rows(bundle):
    """Invented full rows for parser tests, explicitly synthetic source kind."""
    routing = {r['id']: r for r in bundle['routing']['facts']['users']}
    rows = [{**r, 'quota': 123, 'aff_quota': 0, 'group': routing[r['id']]['group']}
            for r in bundle['source']['data']['users']]
    bundle['canonical_source'] = {
        'source_export_sha256': 'c' * 64, 'source_export_proof_sha256': 'd' * 64,
        'source_receipt_sha256': 'e' * 64, 'full_source_row_digest_format': M.F.PLANNER.DIGEST_FORMAT,
        'source_user_row_digests': [{'source_user_id': r['id'], 'source_row_sha256': M.F.PLANNER.digest(r)} for r in rows],
    }
    return rows


def key_source(bundle, rows, tokens):
    source, _ = KT.inputs(tokens)
    source['source_archive_sha256'] = bundle['source']['source_archive_sha256']
    source['users'] = [{k: r[k] for k in ('id', 'status', 'deleted_at', 'auth_version', 'group')}
                       | {'billing_preference': None} for r in rows]
    source['policy'] = ID.routing_policy(bundle['routing']['facts'])
    source['snapshot_at'] = 1700000100
    return source


def target_from_authority(source, identity, authority, *, dormant=False, extra=True):
    mapping = [{k: r[k] for k in L.MAP_FIELDS} for r in identity['plan']['mapping']]
    users = [{'user_id': r['target_user_id'], 'personal_account_id': r['target_personal_account_id'],
        'auth_version': r['auth_version'], 'active': r['active'] and r['account_active'],
        'developer_access_granted': r['developer_granted'] is True,
        'routing_groups': [] if dormant else ['default']} for r in authority]
    if extra:
        users.append({'user_id': 20, 'personal_account_id': 21, 'auth_version': 7, 'active': True,
                      'developer_access_granted': True, 'routing_groups': ['paid']})
    return {'format_version': 1, 'source_archive_sha256': source['source_archive_sha256'],
        'source_target_id_map': mapping,
        'source_target_id_map_sha256': hashlib.sha256(M.P.id_map_bytes(mapping).encode()).hexdigest(),
        'users': sorted(users, key=lambda r: r['user_id'])}


def receipt(identity):
    return {'migration_id': identity['migration_id'], 'source_archive_sha256': identity['source_archive_sha256'],
        'mapping_sha256': identity['mapping_sha256'], 'manifest': {'scope': ID.SCOPE,
        'source_namespace_sha256': identity['source_namespace_sha256'], 'mapping': identity['plan']['mapping'],
        'material': identity, 'auth_version_lineage': identity['plan']['auth_version_lineage'],
        'actual_identity_authority': identity['plan']['expected_identity_authority'], 'post_state': {}}}


def identity_fixture(*, dormant=False, privileged=False, deleted=False):
    bundle, projection, sequences = IT.inputs(privileged=privileged)
    # This owner is the genuine history-completion path: source v1, native
    # before v1 inactive, native after active/password policy v3.
    bundle['source']['data']['users'][-1]['trust_level_override'] = 1
    bundle['source']['source_identity_facts_sha256'] = ID.digest(ID.identity_facts(bundle['source']['data']))
    bundle['source']['current_policy_records'] = ID.mapped_current(bundle['source']['data'], bundle['source']['source_archive_sha256'])
    bundle['current'] = bundle['source']['current_policy_records']
    if dormant:
        projection['active_routing_groups'] = projection['defined_routing_groups'] = ['paid']
        for o in bundle['routing']['facts']['options']:
            if o['key'] == 'GroupRatio':
                o['value'] = '{"paid":1}'
            elif o['key'] == 'UserUsableGroups':
                o['value'] = '{"paid":"Paid"}'
        ID.attach_routing(bundle, bundle['routing']['facts'])
    capabilities = {'abilities': [], 'channels': []}
    ID.attach_capabilities(bundle, capabilities)
    rows = attach_canonical_test_rows(bundle)
    contract = KT.synthetic_contract()
    snap = KT.snapshot({'users': []}, contract)
    snap['identity_projection'] = projection
    for name, state in sequences.items():
        snap['state']['sequences'][name] = {**deepcopy(next(iter(snap['state']['sequences'].values()))), **state}
    snap['state']['inventory']['sequences'] = sorted(snap['state']['sequences'])
    identity = ID.prepare(bundle, snap, contract, 'synthetic-current-identity', ['synthetic_runtime'])
    ID.validate_material(identity)
    tokens = [KT.token(user_id=8, group='', deleted_at='2026-01-01T00:00:00Z' if deleted else None)]
    source = key_source(bundle, rows, tokens)
    target = target_from_authority(source, identity, identity['plan']['expected_identity_authority'], dormant=dormant)
    run = receipt(identity)
    native = {'public.routing_groups': [] if dormant else [{'name': 'default'}],
              'core_identity.user_routing_groups': [r['after'] for r in identity['plan']['operations']
                                                   if r['table'] == 'core_identity.user_routing_groups']}
    tables = {'users': rows, **capabilities}
    bindings = L.validate(rows, source, target, bundle['namespace'], run, identity['plan']['expected_identity_authority'])
    dormant_bindings = L.dormant_bindings(tables, source, target, run, native)
    return bundle, rows, source, target, run, native, bindings, dormant_bindings


def credential_review(source, target, bindings, dormant):
    result = KT.review(source, target)
    result['proposal_sha256'] = L.plan(source, target, bindings, dormant)['proposal_sha256']
    return result


def prepare(source, target, bindings, dormant, namespace=KT.NAMESPACE):
    contract = KT.synthetic_contract()
    return M.prepare_mechanics(source, target, credential_review(source, target, bindings, dormant),
        snapshot=KT.snapshot(target, contract), contract=contract, migration_id='synthetic-current-wrapper',
        source_namespace=namespace, runtime_roles=['synthetic_runtime'], current_bindings=bindings,
        dormant_bindings=dormant, encryption_key=bytes(range(32)))


class CurrentIdentityTests(unittest.TestCase):
    def test_real_identity_planner_history_generation_is_bound_not_rewritten(self):
        bundle, rows, source, target, run, native, bindings, dormant = identity_fixture()
        self.assertEqual(next(r for r in source['users'] if r['id'] == 8)['auth_version'], 1)
        self.assertEqual(bindings['9']['lineage']['planned_final_auth_version'], 3)
        frozen = deepcopy((source, target))
        m = prepare(source, target, bindings, dormant, bundle['namespace'])
        self.assertEqual(m['records'][0]['expected_target_auth_version'], 3)
        self.assertEqual(m['records'][0]['source_auth_version'], 1)
        self.assertEqual((source, target), frozen)
        self.assertTrue(any(r['user_id'] == 20 for r in target['users']))

    def test_nongeneration_operations_do_not_require_invented_version_increment(self):
        _, _, _, _, _, _, bindings, _ = identity_fixture()
        lineage = bindings['3']['lineage']
        self.assertEqual(lineage['binding_kind'], 'reviewed_ordinary_source_delta')
        self.assertEqual(lineage['planned_authority_changes'], [])
        self.assertTrue(lineage['planned_authority_operations'])
        self.assertEqual(lineage['planned_final_auth_version'], lineage['source_auth_version'])

    def test_actual_authority_drift_rejected(self):
        b, rows, s, t, run, _, _, _ = identity_fixture()
        observed = deepcopy(run['manifest']['actual_identity_authority'])
        observed[-1]['auth_version'] += 1
        with self.assertRaises(ValueError):
            L.validate(rows, s, t, b['namespace'], run, observed)

    def test_full_canonical_row_changed_outside_identity_columns_rejected(self):
        b, rows, s, t, run, _, _, _ = identity_fixture()
        rows[0]['quota'] += 1
        with self.assertRaises(ValueError):
            L.validate(rows, s, t, b['namespace'], run, run['manifest']['actual_identity_authority'])

    def test_proposed_identity_material_without_committed_receipt_rejected(self):
        b, rows, s, t, run, _, _, _ = identity_fixture()
        with self.assertRaises(ValueError):
            L.validate(rows, s, t, b['namespace'], run['manifest']['material'], run['manifest']['actual_identity_authority'])

    def test_foreign_source_namespace_rejected(self):
        b, rows, s, t, run, _, _, _ = identity_fixture()
        namespace = {**b['namespace'], 'system_identifier': '5678'}
        with self.assertRaises(ValueError):
            L.validate(rows, s, t, namespace, run, run['manifest']['actual_identity_authority'])

    def test_receipt_plan_hash_and_generation_not_accepted_as_arbitrary_hash(self):
        b, rows, s, t, run, _, _, _ = identity_fixture()
        run = deepcopy(run)
        run['manifest']['material']['plan']['operations'][0]['reason'] += '-tampered'
        with self.assertRaises(ValueError):
            L.validate(rows, s, t, b['namespace'], run, run['manifest']['actual_identity_authority'])

    def test_privileged_owner_never_receives_ordinary_lineage_bridge(self):
        _, _, _, _, _, _, bindings, _ = identity_fixture(privileged=True)
        self.assertNotIn('1', bindings)


class KeyPolicyTests(unittest.TestCase):
    def test_normal_active_path_has_original_mapper_crypto_and_quota_results(self):
        source, target = KT.inputs([KT.token(one_time_reveal=False)])
        original = KT.material(source=source, target=target, encryption_key=bytes(range(32)))
        current = prepare(source, target, {}, {})
        # AES-GCM uses fresh nonces. Compare decrypted values plus all policy,
        # ID allocation and budget fields, not arbitrary ciphertext bytes.
        for row in (original['records'][0], current['records'][0]):
            M.C.verify_encrypted_secret(bytes(range(32)), row, source['tokens'][0]['key'])
        normalize = lambda row: {k: v for k, v in row.items() if k != 'encrypted_secret_hex'}
        self.assertEqual(normalize(original['records'][0]), normalize(current['records'][0]))
        self.assertEqual(original['quota_plan']['key_records'], current['quota_plan']['key_records'])
        self.assertEqual(current['before_state'], original['before_state'])

    def test_deleted_dormant_key_stays_revoked_with_unchanged_group_and_budget(self):
        b, _, s, t, _, _, bindings, dormant = identity_fixture(dormant=True, deleted=True)
        m = prepare(s, t, bindings, dormant, b['namespace'])
        row = m['records'][0]
        self.assertTrue(row['revoked'])
        self.assertEqual(row['native_effective_group'], 'default')
        self.assertEqual(row['quota']['opening_consumed'], 70)
        self.assertEqual(row['quota']['remaining'], 30)
        self.assertEqual(dormant['17']['native_callable_models'], 0)
        self.assertFalse(dormant['17']['availability']['native_route_created'])

    def test_enabled_dormant_fixture_retains_enabled_but_no_route(self):
        b, _, s, t, _, _, bindings, dormant = identity_fixture(dormant=True)
        m = prepare(s, t, bindings, dormant, b['namespace'])
        self.assertTrue(m['records'][0]['enabled'])
        self.assertFalse(m['records'][0]['revoked'])
        self.assertNotIn('default', next(r for r in t['users'] if r['user_id'] == 9)['routing_groups'])

    def test_dormant_route_rejects_any_defined_native_default(self):
        b, rows, s, t, run, native, _, _ = identity_fixture(dormant=True)
        native['public.routing_groups'].append({'name': 'default', 'enabled': False})
        with self.assertRaises(ValueError):
            L.dormant_bindings({'users': rows, 'abilities': [], 'channels': []}, s, t, run, native)

    def test_disabled_source_default_ability_is_not_zero_capability(self):
        _, rows, s, t, run, native, _, _ = identity_fixture(dormant=True)
        tables = {'users': rows, 'channels': [{'id': 4, 'type': 1, 'status': 2, 'group': 'paid',
                    'models': 'model', 'priority': 0, 'weight': 1, 'model_mapping': None}],
                  'abilities': [{'group': 'default', 'model': 'model', 'channel_id': 4,
                    'enabled': False, 'priority': 0, 'weight': 1, 'tag': None}]}
        run = deepcopy(run)
        run['manifest']['material']['source_routing_capability_facts_sha256'] = ID.digest(ID.capability_facts({k: tables[k] for k in ('abilities', 'channels')}))
        with self.assertRaises(ValueError):
            L.dormant_bindings(tables, s, t, run, native)

    def test_explicit_default_does_not_borrow_empty_group_exception(self):
        _, rows, s, t, run, native, bindings, _ = identity_fixture(dormant=True)
        s['tokens'][0]['group'] = 'default'
        dormant = L.dormant_bindings({'users': rows, 'abilities': [], 'channels': []}, s, t, run, native)
        self.assertEqual(dormant, {})
        self.assertTrue(L.plan(s, t, bindings, dormant)['records'][0]['blocked_reasons'])

    def test_new_binding_cannot_remove_developer_or_source_permission_denial(self):
        b, _, s, t, _, _, bindings, dormant = identity_fixture(dormant=True)
        next(r for r in t['users'] if r['user_id'] == 9)['developer_access_granted'] = False
        with self.assertRaisesRegex(Exception, 'credential_live_source_key_requires_complete_native_authority'):
            prepare(s, t, bindings, dormant, b['namespace'])

    def test_modified_dormant_binding_after_preview_rejected(self):
        b, _, s, t, _, _, bindings, dormant = identity_fixture(dormant=True)
        m = prepare(s, t, bindings, dormant, b['namespace'])
        m['current_dormant_group_bindings']['17']['availability']['native_route_created'] = True
        with self.assertRaises(ValueError):
            M.validate_mechanics(m)


def rights_fixture():
    tables = {t: [] for t in ('wallet_transfers', 'subscription_pre_consume_records', 'merchant_store_refunds',
        'tool_market_calls', 'discount_code_reservations', 'subscription_reset_vouchers', 'merchant_store_orders',
        'user_subscriptions', 'top_ups', 'subscription_orders', 'open_source_bounty_projects', 'redemptions',
        'subscription_plans', 'assistant_new_user_gifts', 'checkins')}
    native = {t: [] for t in M.RIGHTS_TABLES}
    return tables, native


class ActualRightsTests(unittest.TestCase):
    def test_pending_original_unknown_is_not_current_unsettled_claim(self):
        tables, native = rights_fixture()
        row = {'id': 1, 'user_id': 2, 'status': 'pending'}
        tables['top_ups'] = [row]
        base = {'migration_id': 'actual-old-pending', 'source_table': 'legacy.top_ups', 'source_row_key': '1',
            'source_row_sha256': M.R.F.digest(row), 'owner_user_id': 3, 'owner_account_id': 4,
            'spendable': False, 'spendable_units': 0, 'settlement_state': 'unknown'}
        native['core_billing.legacy_pending_liabilities'] = [base]
        native['core_billing.legacy_pending_liability_receipts'] = [{k: base[k] for k in ('migration_id', 'source_table', 'source_row_key')}
            | {'settlement_state': 'terminal_paid'}]
        basis = 'lbasis_' + 'a' * 64
        native['core_payments.legacy_bases'] = [{'id': basis, 'source_order_id': 1, 'source_user_id': 2}]
        native['core_payments.legacy_applications'] = [{'basis_id': basis, 'journal_id': 9, 'outcome': {'status': 'posted'}}]
        facts = M.rights_facts(tables, native, [{'source_user_id': 2, 'target_user_id': 3, 'target_personal_account_id': 4}], 100)
        self.assertEqual(facts['blockers'], [])
        current = facts['pending']['covered'][0]
        self.assertFalse(current['current_state_inferred_from_original_base'])
        self.assertEqual(current['actual_receipts'][0]['settlement_state'], 'terminal_paid')
        self.assertEqual(len(current['actual_payment_applications']), 1)

    def test_quota_code_exact_inclusive_expiry_and_aggregate_reserved_capacity(self):
        tables, native = rights_fixture()
        tables['redemptions'] = [{'id': n, 'key': str(n) * 32, 'quota': 30, 'reward_type': 'quota',
            'status': 1, 'deleted_at': None, 'expired_time': 100} for n in (1, 2)]
        native['core_promotions.campaigns'] = [{'id': 'a', 'kind': 'redemption', 'award_units': 30,
            'enabled': True, 'starts_at': 0, 'ends_at': 101, 'reserved_units': 30, 'reserved_claims': 1}]
        native['core_promotions.codes'] = [{'digest': '\\x' + hashlib.sha256(r['key'].encode()).hexdigest(),
            'campaign_id': 'a', 'revoked': False} for r in tables['redemptions']]
        facts = M.rights_facts(tables, native, [], 100)
        self.assertEqual(len(facts['redeemable_codes']), 2)
        self.assertIn('current_rights_redemption_reservations_insufficient', facts['blockers'])
        native['core_promotions.campaigns'][0].update(reserved_units=60, reserved_claims=2)
        self.assertEqual(M.rights_facts(tables, native, [], 100)['blockers'], [])
        self.assertEqual(M.rights_facts(tables, native, [], 101)['redeemable_codes'], [])

    def test_reset_voucher_uses_independent_second_expiry_without_converting_to_cash(self):
        tables, native = rights_fixture()
        tables['subscription_plans'] = [{'id': 7}]
        tables['redemptions'] = [{'id': 1, 'status': 1, 'deleted_at': None, 'expired_time': 0,
            'reward_type': 'reset_voucher', 'reset_plan_id': 7, 'reset_voucher_expires_at': 101}]
        self.assertIn('current_rights_reset_voucher_claim_contract_missing', M.rights_facts(tables, native, [], 100)['blockers'])
        self.assertEqual(M.rights_facts(tables, native, [], 101)['blockers'], [])

    def test_unimplemented_real_offered_and_checkin_rights_cannot_be_approved_by_bool(self):
        tables, native = rights_fixture()
        tables['assistant_new_user_gifts'] = [{'id': 1, 'status': 'offered'}]
        tables['checkins'] = [{'id': 1}]
        native['approved'] = True
        blockers = M.rights_facts(tables, native, [], 100)['blockers']
        self.assertIn('current_rights_offered_gift_claim_contract_missing', blockers)
        self.assertIn('checkin_runtime_history_deduplication_unproven', blockers)


if __name__ == '__main__':
    unittest.main()
