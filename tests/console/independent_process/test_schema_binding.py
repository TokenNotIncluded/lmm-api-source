"""Verifier tests only: these do not constitute real PostgreSQL acceptance."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

spec = importlib.util.spec_from_file_location('console_runner', Path(__file__).with_name('run.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class SchemaBindingTests(unittest.TestCase):
    def verify(self, mutation=None, wrong_hash=False):
        manifest = {'format_version': 1, 'scope': 'compiled-core-schema-contract',
                    'components': [{'name': 'identity', 'raw_schema_sha256': 'a' * 64},
                                   {'name': 'navigation', 'raw_schema_sha256': 'b' * 64}],
                    'identity': {'version': 6, 'fingerprint_sha256': 'a' * 64}}
        actual = {item['name']: item['raw_schema_sha256'] for item in manifest['components']}
        identity = copy.deepcopy(manifest['identity'])
        if mutation:
            mutation(manifest, actual, identity)
        with tempfile.TemporaryDirectory() as directory:
            core = Path(directory) / 'core'
            core.write_bytes(b'fixed test artifact')
            subject = object.__new__(runner.Runner)
            subject.core = core
            subject.args = SimpleNamespace(core_sha256=hashlib.sha256(core.read_bytes()).hexdigest())
            if wrong_hash:
                subject.args.core_sha256 = "0" * 64
            subject.command = lambda *_: json.dumps(manifest)
            subject.sql = lambda label, _: json.dumps(actual if label == 'installed_components' else identity)
            saved = []
            subject.save = lambda name, value: saved.append((name, value))
            subject.verify_installed_schema()
            self.assertEqual(saved[0][1]['result'], 'PASS')

    def test_rejects_binary_hash_mismatch(self):
        with self.assertRaisesRegex(runner.Failure, "schema_binary_hash_drift"):
            self.verify(wrong_hash=True)

    def test_matching_contract(self):
        self.verify()

    def test_rejects_missing_extra_changed_components_and_identity(self):
        mutations = [
            lambda m, a, i: a.pop('navigation'),
            lambda m, a, i: a.update(extra='c' * 64),
            lambda m, a, i: a.update(navigation='c' * 64),
            lambda m, a, i: i.update(version=7),
            lambda m, a, i: i.update(fingerprint_sha256='c' * 64),
            lambda m, a, i: m['components'].append(m['components'][0]),
            lambda m, a, i: m.update(format_version=2),
        ]
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index), self.assertRaises(runner.Failure):
                self.verify(mutation)


if __name__ == '__main__':
    unittest.main()
