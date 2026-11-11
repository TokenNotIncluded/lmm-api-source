"""Fresh ledger capture lineage checks, without PostgreSQL or compilation."""
import copy
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CAPTURE = load('fresh_capture', ROOT / 'scripts/capture-core62-target.py')
CONTRACT = load('fresh_contract', ROOT / 'scripts/test-commerce-self-purchase.py')


class CreditQuoteCaptureTests(unittest.TestCase):
    def setUp(self):
        import hashlib
        self.components, _, self.manifest = CONTRACT.source_contract()
        self.quote_sha = hashlib.sha256(b'\n'.join(
            (ROOT / name).read_bytes() for name in CAPTURE.CREDIT_QUOTE_FILES
        )).hexdigest()

    def validate(self, manifest=None, quote_sha=None):
        CAPTURE.validate_credit_quote_lineage(
            self.manifest if manifest is None else manifest, CONTRACT.manifest_hash,
            self.quote_sha if quote_sha is None else quote_sha,
        )

    def test_exact_double_sql_and_unchanged_reviewed_prefix(self):
        import hashlib
        self.validate()
        self.assertEqual(hashlib.sha256(self.components[-1][1].encode()).hexdigest(), self.quote_sha)
        self.assertEqual(CAPTURE.final_component_count(self.components, credit_quote_update=True), 66)
        with self.assertRaisesRegex(ValueError, 'exact final assembly'):
            CAPTURE.final_component_count(self.components)

    def test_prefix_or_identity_change_is_rejected(self):
        for field in ('components', 'identity', 'catalog', 'installer'):
            changed = copy.deepcopy(self.manifest)
            if field == 'components':
                changed[field][0]['raw_schema_sha256'] = 'ab' * 32
            else:
                changed[field] = {}
            with self.assertRaisesRegex(ValueError, '65-component prefix'):
                self.validate(changed)

    def test_missing_extra_or_reordered_component_is_rejected(self):
        for components in (self.manifest['components'][:-1], self.manifest['components'] + [self.manifest['components'][-1]],
                           list(reversed(self.manifest['components']))):
            changed = copy.deepcopy(self.manifest)
            changed['components'] = components
            with self.assertRaisesRegex(ValueError, 'exact 66th component'):
                self.validate(changed)

    def test_base_only_or_swapped_quote_sql_is_rejected(self):
        import hashlib
        files = [ROOT / name for name in CAPTURE.CREDIT_QUOTE_FILES]
        for sql in (files[0].read_bytes(), b'\n'.join(path.read_bytes() for path in reversed(files))):
            with self.assertRaisesRegex(ValueError, 'exact two frozen SQL files'):
                self.validate(quote_sha=hashlib.sha256(sql).hexdigest())

    def test_capture_resolver_rejects_changed_constant_paths_or_separator(self):
        from unittest.mock import patch
        original_read = Path.read_text
        producer = CONTRACT.CORE / 'src/payments.rs'
        for before, after in (('payment_credit_quote_referrals.sql', 'ledger.sql'), ('"\\n"', '""')):
            def changed_read(path, *args, **kwargs):
                raw = original_read(path, *args, **kwargs)
                return raw.replace(before, after) if path == producer else raw
            with patch.object(Path, 'read_text', changed_read):
                with self.assertRaisesRegex(AssertionError, 'credit quote composite constant changed'):
                    CONTRACT.source_contract()


class CurrentLedgerLineageTests(unittest.TestCase):
    def setUp(self):
        _, _, self.manifest = CONTRACT.source_contract()
        # Keep the historical65 ledger fixture when the live registry adds66.
        if self.manifest['components'][-1]['name'] == 'payment-credit-quotes':
            self.manifest['components'].pop()
        import json
        self.predecessor = json.loads((ROOT / 'services/core/src/core_schema/upgrade/subscription_epay_target.json').read_text())
        self.ledger = {item['name']: item['raw_schema_sha256'] for item in self.manifest['components']
                       if item['name'] in CAPTURE.PRIOR_LEDGER_COMPONENTS}

    def validate(self, manifest=None, ledger=None):
        CAPTURE.validate_current_ledger_lineage(
            self.manifest if manifest is None else manifest, self.predecessor,
            CONTRACT.manifest_hash, self.ledger if ledger is None else ledger,
        )

    def test_only_ledger_replacement_matches_both_frozen_manifests(self):
        before = copy.deepcopy(self.manifest)
        self.validate()
        self.assertEqual(self.manifest, before)

    def test_other_prefix_component_change_is_rejected(self):
        changed = copy.deepcopy(self.manifest)
        changed['components'][0]['raw_schema_sha256'] = 'ab' * 32
        with self.assertRaisesRegex(ValueError, '59 prefix changed'):
            self.validate(changed)

    def test_launch_suffix_change_is_rejected(self):
        changed = copy.deepcopy(self.manifest)
        changed['components'][-1]['raw_schema_sha256'] = 'ab' * 32
        with self.assertRaisesRegex(ValueError, 'fresh manifest changed'):
            self.validate(changed)

    def test_identity_and_catalog_contract_changes_are_rejected(self):
        for key in ('identity', 'catalog'):
            changed = copy.deepcopy(self.manifest)
            changed[key] = {}
            with self.assertRaisesRegex(ValueError, '59 prefix changed'):
                self.validate(changed)

    def test_unfrozen_ledger_bytes_are_rejected(self):
        changed = dict(self.ledger, ledger='ab' * 32)
        with self.assertRaisesRegex(ValueError, 'current compiled ledger producer differs'):
            self.validate(ledger=changed)

    def test_unchanged_ledger_cannot_use_update_mode(self):
        changed = copy.deepcopy(self.manifest)
        for item in changed['components']:
            if item['name'] == 'ledger':
                item['raw_schema_sha256'] = CAPTURE.PRIOR_LEDGER_SHA256
        with self.assertRaisesRegex(ValueError, 'requires an actual producer change'):
            self.validate(changed, dict(self.ledger, ledger=CAPTURE.PRIOR_LEDGER_SHA256))

    def test_missing_final_producer_cannot_pass_as_base_ledger_update(self):
        changed = dict(self.ledger)
        changed.pop('commerce-self-purchase')
        with self.assertRaisesRegex(ValueError, 'three reviewed components'):
            self.validate(ledger=changed)

    def test_assembled_final_ledger_definitions_keep_the_new_action(self):
        import re
        components, _, _ = CONTRACT.source_contract()
        assembled = '\n'.join(ddl for _, ddl in components)
        for function in ('post_ledger', 'ledger_apply_entry'):
            definitions = re.findall(r'CREATE (?:OR REPLACE )?FUNCTION core_billing\.' + function
                                     + r'\([\s\S]*?END \$\$;', assembled)
            self.assertTrue(definitions)
            self.assertIn('legacy_wallet_reserve', definitions[-1])
        self.assertIn('affiliate_clawback', definitions[-1])
        constraints = re.findall(r'ADD CONSTRAINT ledger_journals_kind_check CHECK\([\s\S]*?;', assembled)
        self.assertTrue(constraints)
        self.assertIn('legacy_wallet_reserve', constraints[-1])
        self.assertIn('affiliate_award', constraints[-1])


if __name__ == '__main__':
    unittest.main()
