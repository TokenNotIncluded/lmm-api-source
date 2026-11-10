"""Exact equivalence with the original financial bearer substring guard."""
import importlib.util
import itertools
from pathlib import Path
import random
import unittest

spec = importlib.util.spec_from_file_location(
    'financial_secret_guard_subject', Path(__file__).with_name('migrate-legacy-financial.py'))
F = importlib.util.module_from_spec(spec)
spec.loader.exec_module(F)


def original_rejects(value, secrets):
    # Original algorithm: each string, including dict keys, checks every bearer.
    if type(value) is str:
        return any(secret in value for secret in secrets)
    if type(value) is dict:
        return any(original_rejects(key, secrets) or original_rejects(child, secrets)
                   for key, child in value.items())
    if type(value) is list:
        return any(original_rejects(child, secrets) for child in value)
    return False


class FinancialSecretGuardTests(unittest.TestCase):
    def equivalent(self, value, secrets):
        expected = original_rejects(value, secrets)
        try:
            F.reject_known_key_secrets(value, [{'key': s} for s in secrets])
        except F.MappingError as exc:
            self.assertEqual(str(exc), 'reconciliation_key_secret_in_output')
            actual = True
        else:
            actual = False
        self.assertEqual(actual, expected)

    def test_exhaustive_small_unicode_equivalence(self):
        alphabet = ('a', '界', '😀')
        patterns = [''.join(x) for n in range(3)
                    for x in itertools.product(alphabet, repeat=n)]
        values = [''.join(x) for n in range(4)
                  for x in itertools.product(alphabet, repeat=n)]
        for size in range(4):
            for secrets in itertools.combinations(patterns, size):
                for value in values:
                    self.equivalent(value, secrets)

    def test_seeded_recursive_random_equivalence(self):
        rng = random.Random(20261111)
        alphabet = 'ab界😀\u0301\0'

        def text():
            return ''.join(rng.choice(alphabet) for _ in range(rng.randrange(81)))

        for _ in range(1000):
            secrets = [''.join(rng.choice(alphabet) for _ in range(rng.randrange(1, 15)))
                       for _ in range(rng.randrange(33))]
            value = {'metadata': [{text(): [text(), None, False, 7]}, text()]}
            if secrets and rng.randrange(2):
                secret = rng.choice(secrets)
                if rng.randrange(2):
                    value['metadata'].append({'nested': 'before'+secret+'after'})
                else:
                    value['metadata'].append({'before'+secret+'after': True})
            self.equivalent(value, secrets)

    def test_empty_duplicate_overlap_and_original_recursion(self):
        for secrets in ([], [''], ['', ''], ['aba', 'bab', 'aba'], ['界😀', '😀'], ['ab']):
            for value in ('', 'ababa', 'a界😀b', 'e\u0301', '\0', {}, [], None, 0,
                          {'': ['a', 'b']}, {'nested': [{'界😀': True}]}, ('ab',)):
                self.equivalent(value, secrets)

    def test_metadata_key_and_value_rejected(self):
        secret = 'synthetic-bearer-never-from-production'
        for value in ({'metadata': {'before'+secret+'after': True}},
                      {'metadata': [{'nested': 'before'+secret+'after'}]}):
            with self.assertRaisesRegex(F.MappingError, '^reconciliation_key_secret_in_output$'):
                F.reject_known_key_secrets(value, [{'key': secret}])

    def test_many_patterns_short_hash_and_long_text_branches(self):
        secrets = ['synthetic-bearer-'+str(n).zfill(32) for n in range(878)]
        self.equivalent({'fields': [{'row_hash': 'a'*64} for _ in range(20)]}, secrets)
        self.equivalent({'metadata': 'x'*2000}, secrets)
        self.equivalent({'metadata': 'x'*2000+secrets[-1]}, secrets)
        self.equivalent({'metadata': 'x'+secrets[-1]+'y'}, secrets)


if __name__ == '__main__':
    unittest.main()
