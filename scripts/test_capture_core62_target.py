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


class CurrentLedgerLineageTests(unittest.TestCase):
    def setUp(self):
        _, _, self.manifest = CONTRACT.source_contract()
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
