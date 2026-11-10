"""Fresh65 reviewed-source and synthetic refusal tests; no PG or migration."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


I = load('core65_importer', 'migrate-legacy-snapshot.py')
H = load('core65_contract_helper', 'test-commerce-self-purchase.py')
C = load('final_fresh_capture', 'capture-core62-target.py')


class Core65AdmissionTests(unittest.TestCase):
    def test_missing_or_incomplete_review_cannot_admit(self):
        with patch.object(I, 'REVIEWED_CORE65', None):
            with self.assertRaisesRegex(I.MappingError, 'core65_profile_not_admitted'):
                I.reviewed65()
        for partial in ({}, {'source_schema_revision': None}):
            with patch.object(I, 'REVIEWED_CORE65', partial):
                with self.assertRaisesRegex(I.MappingError, 'core65_review_incomplete'):
                    I.reviewed65()

    def test_reviewed_fresh65_source_matches_native_evidence(self):
        review = I.reviewed65()
        contract = I.current_contract()
        self.assertEqual(contract['import_profile'], I.CORE65_SEED_CONTRACT)
        self.assertEqual(len(contract['component_fingerprints']), 65)
        self.assertEqual(contract['component_fingerprints'],
                         {name: pair[1] for name, pair in review['components'].items()})
        for key in ('source_schema_revision', 'compiled_manifest_sha256', 'fresh_catalog_sha256'):
            self.assertEqual(contract[key], review[key])
        self.assertEqual(contract['profile_review_evidence'], {key: review[key] for key in
            ('capture_receipt_sha256', 'actual_init_receipt_sha256', 'acl_receipt_sha256')})

    def test_review_cannot_omit_evidence_or_accept_changed_history(self):
        review = I.reviewed65()
        for key in review:
            with self.subTest(missing=key), patch.object(I, 'REVIEWED_CORE65',
                    {name: value for name, value in review.items() if name != key}):
                with self.assertRaisesRegex(I.MappingError, 'core65_review_incomplete'):
                    I.reviewed65()
        components = {**review['components'], 'identity': (review['components']['identity'][0], '0' * 64)}
        with patch.object(I, 'REVIEWED_CORE65', {**review, 'components': components}):
            with self.assertRaisesRegex(I.MappingError, 'core65_historical_component_changed'):
                I.reviewed65()

    def test_capture_measures_registry_and_requires_exact_final_image_suffix(self):
        names=[*I.REVIEWED_COMPONENTS_59,*I.CORE65_COMPONENT_EXPRESSIONS]
        self.assertEqual(C.final_component_count([(n,'') for n in names]),65)
        for wrong in (names[:-1],names+['unknown-extra'],names[:-2]+[names[-1],names[-2]]):
            with self.subTest(names=wrong[-3:]),self.assertRaisesRegex(ValueError,r'exact final 59\+6'):
                C.final_component_count([(n,'') for n in wrong])

    def test_unreviewed_source_cannot_reuse_historical_profile(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root/'services/core/src/core_schema.rs'
            path.parent.mkdir(parents=True)
            path.write_text('// synthetic unreviewed65 assembly; not a capture\n')
            with self.assertRaisesRegex(I.MappingError, 'component_rust_source_fingerprint_mismatch'):
                I.reviewed_profile(root)

    def test_all_new_business_and_receipt_rows_have_fresh_fences(self):
        old = {'component_fingerprints': I.REVIEWED_COMPONENTS_59}
        new = {'component_fingerprints': {**I.REVIEWED_COMPONENTS_59, **I.CORE65_COMPONENT_EXPRESSIONS}}
        self.assertEqual(len(I.fresh_business_tables(old)), 4)
        self.assertEqual(len(I.fresh_business_tables(new)), 21)
        self.assertNotIn('core_meta.core59_to62_upgrades', I.upgrade_receipt_tables(old))
        self.assertIn('core_meta.core59_to62_upgrades', I.upgrade_receipt_tables(new))
        query = I.fresh_seed_hash_query(I.CORE65_SEED_CONTRACT)
        for table in (*I.fresh_business_tables(new), 'core_meta.core59_to62_upgrades',
                      'core_federated_auth.providers_id_seq'):
            self.assertIn('FROM ' + table + ' t', query)
        self.assertIn("'log_cnt',t.log_cnt", query)
        self.assertNotIn('core_federated_auth', I.fresh_seed_hash_query(I.CORE59_SEED_CONTRACT))
        guard = I.profile_acl_guard_sql(new)
        self.assertIn("a.privilege_type<>'SELECT' OR a.is_grantable", guard)
        self.assertIn('a.grantee=0', guard)
        self.assertNotIn('core59_to62_upgrades', I.profile_acl_guard_sql(old))
        self.assertIn('guard_pancake_order()',guard)
        self.assertIn('import_legacy_refund_grant(jsonb)',guard)
        self.assertIn('image-native-usage',I.CORE65_COMPONENT_EXPRESSIONS)
        self.assertEqual(len(new['component_fingerprints']),65)

    def test_real_concat_shape_resolves_full_bytes_without_double_receipt(self):
        alias = 'navigation_options::SUBSCRIPTION_STRIPE_WITH_UPGRADE_RECEIPT'
        relative, constant, expression = I.REVIEWED_SCHEMA_ALIASES[alias]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            core = root/'services/core'
            for sub in ('src/core_schema/upgrade', 'schema'):
                (core/sub).mkdir(parents=True)
            producer = core/'src/core_schema/navigation_options.rs'
            producer.write_text('pub(super) const ' + constant + ': &str = ' + expression + ';\n')
            base, receipt = b'-- synthetic Stripe producer\n', b'-- synthetic new upgrade receipt\n'
            (core/'schema/subscription_stripe.sql').write_bytes(base)
            (core/'src/core_schema/upgrade/core59_to62_receipt.sql').write_bytes(receipt)
            (core/'schema/identity.sql').write_text('-- synthetic identity fixture\n')
            for name in ('core_upgrade', 'component_upgrade', 'ledger_revision', 'offline_upgrade'):
                (core/f'schema/{name}.sql').write_text('')
            (core/'src/core_schema/upgrade/console_receipt.sql').write_text('')
            assembly = ('const COMPONENTS: &[(&str, &str)] = &[\n("subscription-stripe",' + alias + '),\n];\n'
                        'const CONTRACT: &str = "";\nconst CATALOG: &str = r#"SELECT fixture"#;\n')
            (core/'src/core_schema.rs').write_text(assembly)
            expected = base + b'\n' + receipt
            pins = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in (producer, core/'src/core_schema.rs')}
            components = {'subscription-stripe': (alias, hashlib.sha256(expected).hexdigest())}
            # These patches pin only a tiny resolver fixture, NOT admission.
            with patch.object(I, 'reviewed_files', return_value=pins), \
                    patch.object(I, 'reviewed_components', return_value=components):
                self.assertEqual(I.component_schemas(root)['subscription-stripe'], expected)
            with patch.object(H, 'CORE', core):
                parts, _, manifest = H.source_contract()
                self.assertEqual(parts, [('subscription-stripe', expected.decode())])
                self.assertEqual(manifest['components'][0]['raw_schema_sha256'], hashlib.sha256(expected).hexdigest())
                producer.write_text(producer.read_text().replace('core59_to62_receipt.sql', 'other.sql'))
                with self.assertRaisesRegex(AssertionError, 'composite constant changed'):
                    H.source_contract()

    def test_pancake_reexport_resolves_actual_submodule_include_path(self):
        alias='crate::subscriptions::PANCAKE_SCHEMA'
        self.assertEqual(I.REVIEWED_SCHEMA_ALIASES[alias],(
            'services/core/src/subscriptions/external_pancake.rs','PANCAKE_SCHEMA',
            'include_str!("../../schema/subscription_pancake.sql")'))
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            core=root/'services/core'
            (core/'src/subscriptions').mkdir(parents=True)
            (core/'schema').mkdir()
            producer=core/'src/subscriptions/external_pancake.rs'
            producer.write_text('pub const PANCAKE_SCHEMA: &str = include_str!("../../schema/subscription_pancake.sql");\n')
            expected=b'-- synthetic Pancake resolver bytes\n'
            (core/'schema/subscription_pancake.sql').write_bytes(expected)
            assembly=core/'src/core_schema.rs'
            assembly.write_text('const COMPONENTS: &[(&str, &str)] = &[("subscription-pancake",'+alias+'),];\n')
            publisher=core/'src/subscriptions.rs'
            for export in ('*','{PANCAKE_SCHEMA, PancakeEventReceipt, PancakeOrder}','{PancakeOrder}'):
                publisher.write_text('mod external_pancake;\npub use external_pancake::'+export+';\n')
                pins={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in (producer,assembly,publisher)}
                with patch.object(I,'reviewed_files',return_value=pins),patch.object(I,'reviewed_components',
                    return_value={'subscription-pancake':(alias,hashlib.sha256(expected).hexdigest())}):
                    if export=='{PancakeOrder}':
                        with self.assertRaisesRegex(I.MappingError,'component_alias_not_reviewed'):
                            I.component_schemas(root)
                    else:
                        self.assertEqual(I.component_schemas(root)['subscription-pancake'],expected)

    def test_receipt_path_exception_cannot_read_other_source_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            producer = root/'services/core/src/core_schema/navigation_options.rs'
            with self.assertRaisesRegex(I.MappingError, 'component_contract_invalid'):
                I._component_schema_paths(root, 'include_str!("upgrade/other.sql")', producer)
            with self.assertRaisesRegex(I.MappingError, 'component_contract_invalid'):
                I._component_schema_paths(root, 'include_str!("upgrade/core59_to62_receipt.sql")',
                                          root/'services/core/src/core_schema/wrong.rs')


if __name__ == '__main__':
    unittest.main()
