# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""The reconciliation must reject financial drift, missing rows and ambiguity."""
from fractions import Fraction
import unittest
import verify_legacy_cutover_rehearsal as audit


class ComparisonTests(unittest.TestCase):
    def test_exact_rows_ignore_order_but_detect_missing_extra_and_changed_money(self):
        original=[{'id':1,'balance':-7},{'id':2,'balance':100}]
        self.assertTrue(audit.compare_rows(original,list(reversed(original)))['passed'])
        changed=[{'id':1,'balance':-8},{'id':3,'balance':100}]
        result=audit.compare_rows(original,changed)
        self.assertEqual((result['missing'],result['extra'],result['different'],result['passed']),(1,1,1,False))

    def test_boolean_and_integer_cannot_replace_each_other(self):
        self.assertFalse(audit.compare_rows([{'id':1,'enabled':True}],[{'id':1,'enabled':1}])['passed'])

    def test_duplicate_source_or_target_ids_do_not_disappear_in_a_map(self):
        rows=[{'id':1,'amount':10},{'id':1,'amount':20}]
        for source,target in ((rows,[]),([],rows)):
            with self.assertRaisesRegex(audit.F.Error,'duplicate_comparison_identifiers'):
                audit.compare_rows(source,target)

    def test_exact_fraction_rounding_never_uses_binary_float(self):
        self.assertEqual([audit.normalized_quota(v,Fraction(2)) for v in (0,1,2,3)], [0,1,1,2])
        divisor=Fraction('7.123457')
        self.assertEqual(audit.normalized_quota(7123457000000001,divisor),1000000000000000)
        for raw,divisor in ((-1,Fraction(1)),(True,Fraction(1)),(1,Fraction(0))):
            with self.assertRaisesRegex(audit.F.Error,'invalid_quota_conversion'):
                audit.normalized_quota(raw,divisor)


if __name__=='__main__':
    unittest.main()
