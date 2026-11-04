"""Console cutover retains payment consent facts without creating consent."""
import unittest

from legacy_cutover_keys import plan_console


class PaymentConsentImportTest(unittest.TestCase):
    def plan(self, options):
        return plan_console({"options": options, "users": [],
                             "announcement_reads": [], "setups": []}, None)

    def test_payment_policy_values_are_copied_without_enabling_or_upgrading(self):
        for confirmed, version in (("true", "v1"), ("false", "v1"),
                                   ("true", "v0"), ("", "unknown")):
            with self.subTest(confirmed=confirmed, version=version):
                sql, stats = self.plan({
                    "payment_setting.compliance_confirmed": confirmed,
                    "payment_setting.compliance_terms_version": version,
                })
                self.assertEqual(sql, [
                    "INSERT INTO core_console.options(key,value) VALUES"
                    f"('payment_setting.compliance_confirmed','{confirmed}');",
                    "INSERT INTO core_console.options(key,value) VALUES"
                    f"('payment_setting.compliance_terms_version','{version}');",
                ])
                self.assertEqual(stats["options_copied"], 2)

    def test_missing_policy_is_not_fabricated(self):
        sql, stats = self.plan({})
        self.assertEqual(sql, [])
        self.assertEqual(stats.get("options_copied", 0), 0)
        sql, stats = self.plan({"payment_setting.compliance_confirmed": "true"})
        self.assertEqual(len(sql), 1)
        self.assertNotIn("compliance_terms_version", sql[0])

    def test_other_payment_options_remain_outside_the_whitelist(self):
        sql, stats = self.plan({"payment_setting.private_key": "synthetic-only"})
        self.assertEqual(sql, [])
        self.assertEqual(stats["legacy_options_not_consumed_by_core"], 1)


if __name__ == "__main__":
    unittest.main()
