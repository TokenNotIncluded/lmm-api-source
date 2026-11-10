"""Streaming bytes equal the pre-stream digest from commit 61176475188e28e8f87946c94a033afd018eae2d."""
from decimal import Decimal
import hashlib
import importlib.util
import itertools
from pathlib import Path
import sys
import unittest

SCRIPTS = Path(__file__).parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location('financial_digest_subject', SCRIPTS/'migrate-legacy-financial.py')
F = importlib.util.module_from_spec(spec)
spec.loader.exec_module(F)
canonical, require, MappingError = F.canonical, F.require, F.MappingError


def original_bytes(value):
    # Tag every type, not just Decimal: a literal JSONB {"$decimal":"1.00"}
    # must not hash as the numeric 1.00. The private raw export is unmodified.
    def typed(item):
        if item is None:
            return ["null"]
        if type(item) is bool:
            return ["boolean", item]
        if type(item) is int:
            return ["integer", str(item)]
        if isinstance(item, Decimal):
            require(item.is_finite(), "non_finite_decimal")
            return ["decimal", str(item)]
        if type(item) is str:
            return ["string", item]
        if isinstance(item, list):
            return ["array", [typed(child) for child in item]]
        if isinstance(item, dict):
            require(all(type(key) in (str, int) for key in item), "hash_object_key_invalid")
            return ["object", sorted([[typed(key), typed(child)] for key, child in item.items()], key=lambda pair: canonical(pair[0]))]
        raise MappingError("binary_float_or_unsupported_hash_type_rejected")
    return canonical(typed(value)).encode()


class StreamingDigestProof(unittest.TestCase):
    def bytes_and_hash_equal(self, value):
        expected = original_bytes(value)
        offset = 0
        largest = 0
        for chunk in F._typed_canonical_chunks(value):
            self.assertEqual(chunk, expected[offset:offset+len(chunk)])
            offset += len(chunk)
            largest = max(largest, len(chunk))
        self.assertEqual(offset, len(expected))
        self.assertEqual(F.digest(value), hashlib.sha256(original_bytes(value)).hexdigest())
        return len(expected), largest

    def test_scalar_nested_utf8_bytes_equal(self):
        values = [None, True, False, 0, -1, 2**100, '', '界😀e\u0301', '\0\n\t"\\',
                  Decimal('1.00'), Decimal('-0.00'), Decimal('1E+100'), [], {},
                  {'source': [{3: 'three', '3': [True, Decimal('0.0001'), None]}]}]
        for value in values:
            self.bytes_and_hash_equal(value)

    def test_mixed_key_order_and_nesting_equal(self):
        pairs = [(2, Decimal('2.00')), ('2', 'two'), (-10, ['界', False]), ('\n', {})]
        digests = set()
        for order in itertools.permutations(pairs):
            value = {'nested': [dict(order)]}
            self.bytes_and_hash_equal(value)
            digests.add(F.digest(value))
        self.assertEqual(len(digests), 1)
        value = Decimal('-1.2500')
        for n in range(24):
            value = {str(n): [value]}
        self.bytes_and_hash_equal(value)

    def test_original_type_tags_and_decimal_scale_distinct(self):
        values = [True, 1, '1', Decimal('1'), Decimal('1.0'), Decimal('1.00'),
                  {'$decimal': '1.00'}, None, 'null', Decimal('-0'), Decimal('0')]
        for value in values:
            self.bytes_and_hash_equal(value)
        self.assertEqual(len({F.digest(v) for v in values}), len(values))

    def test_dict_subclass_items_values_not_getitem_override(self):
        class DifferentLookup(dict):
            def __getitem__(self, key):
                return 'lookup-override-is-not-the-stored-value'

        value = DifferentLookup({'z': Decimal('2.00'), 'a': [True, {'界': None}]})
        self.assertNotEqual(value['z'], dict(value.items())['z'])
        self.bytes_and_hash_equal(value)
        self.assertEqual(F.digest(value), F.digest(dict(value.items())))

    def test_rejections_and_original_first_error_order_equal(self):
        values = [1.0, float('nan'), float('inf'), Decimal('NaN'), Decimal('sNaN'),
                  Decimal('Infinity'), {True: 'invalid key'}, {1.5: Decimal('NaN')},
                  {'z': Decimal('NaN'), 'a': 1.0}, {'z': 1.0, 'a': Decimal('NaN')},
                  {'z': '\ud800', 'a': 1.0}, '\ud800', '\udfff', (1, 2), object()]
        for value in values:
            try:
                original_bytes(value)
            except Exception as old:
                with self.assertRaises(type(old)) as new:
                    F.digest(value)
                if isinstance(old, F.MappingError):
                    self.assertEqual(str(new.exception), str(old))
            else:
                self.fail('expected original rejection')

    def test_large_table_bytes_and_digest_equal(self):
        rows = [{'id': n, 'state': 'synthetic', 'amount': Decimal(f'{n}.2500'),
                 'metadata': {'text': '界😀'*64, 'optional': None, 'flag': n%2 == 0},
                 'units': [n, -n, 2**63-1]} for n in range(8192)]
        total, largest = self.bytes_and_hash_equal(rows)
        self.assertGreater(total, 4*1024*1024)
        self.assertLess(largest, 1024)


if __name__ == '__main__':
    unittest.main()
