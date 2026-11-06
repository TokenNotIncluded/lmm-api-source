"""Offline import values retain their PostgreSQL scalar representation."""
from decimal import Decimal
import unittest

from legacy_sql import literal


class LiteralTest(unittest.TestCase):
    def test_scalars_preserve_type_and_text(self):
        cases = (
            (None, "NULL"),
            (True, "TRUE"),
            (False, "FALSE"),
            (0, "0"),
            (-42, "-42"),
            (9007199254740993, "9007199254740993"),
            ("", "''"),
            ("NULL", "'NULL'"),
            ("O'Reilly", "'O''Reilly'"),
            ("路径\\file\n第二行", "'路径\\file\n第二行'"),
            (Decimal("1.2300"), "'1.2300'"),
        )
        for value, expected in cases:
            with self.subTest(value=value):
                self.assertEqual(literal(value), expected)


if __name__ == "__main__":
    unittest.main()
