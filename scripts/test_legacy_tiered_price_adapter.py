#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Synthetic contracts only; production options stay in private evidence."""
import hashlib
import json
import unittest

import legacy_tiered_price_adapter as A


def options(expression, mode="tiered_expr", unit="500000"):
    return [{"key": "LegacyPricingQuotaPerUnit", "value": unit},
            {"key": "billing_setting.billing_mode", "value": json.dumps({"synthetic": mode})},
            {"key": "billing_setting.billing_expr", "value": json.dumps({"synthetic": expression})}]


def row(values, provider=None, channel_type=1):
    return A.provider_price_row(values, "synthetic", "reviewed-group", 1_250_000,
        provider=provider or {}, source_channel_type=channel_type,
        contract_revision="synthetic-reviewed-native-decimal")


class AdapterTests(unittest.TestCase):
    def test_original_identity_and_zero_variable_semantics(self):
        expression = 'tier("synthetic", p * .125 + cr * .025 + cc * 0 + c * 1.75)'
        result = A.compile_model(options(expression), "synthetic")
        self.assertEqual(result["source_expression"], expression)
        self.assertEqual(result["source_expression_sha256"], hashlib.sha256(expression.encode()).hexdigest())
        tariff = result["tiered"]
        self.assertEqual(tariff["tiers"][0]["rates"]["input"], {"numerator": 1, "denominator": 8})
        self.assertTrue(tariff["uses_cache_write"])
        self.assertFalse(result["legacy_float_bitwise_equivalent"])

    def test_existing_provider_wire_with_explicit_group(self):
        values = options('tier("synthetic", p * 7 + cr * 0.2 + c * 13)')
        price, origin = row(values)
        self.assertEqual(price["kind"], "tiered_decimal")
        self.assertEqual(price["source_options_sha256"], A.R.F.digest(values))
        self.assertEqual(origin["compiled_price"]["group_multiplier_ppm"], 1_250_000)
        self.assertNotIn("rounding", price)

    def test_length_boundary_and_distinct_creation_ttl(self):
        expression = '(len <= 100000 ? tier("short", p*.2+cc*.25+cc1h*.4+c*1) : tier("long", p*.4+cc*.5+cc1h*.8+c*2)) * (1)'
        result = A.compile_model(options(expression), "synthetic")["tiered"]
        self.assertEqual(result["revision"], 2)
        self.assertEqual(result["tiers"][0]["up_to_input_tokens"], 100000)
        self.assertNotIn("up_to_input_tokens", result["tiers"][1])
        self.assertEqual(result["tiers"][0]["rates"]["cache_write_1h"], {"numerator": 2, "denominator": 5})

    def test_non_audio_requires_adapter_and_operation_contract(self):
        values = options('tier("synthetic", p*.7+ai*2+cr*.07+c*4)')
        with self.assertRaises(A.R.MappingError): row(values)
        provider = {"protocol": "gemini", "non_audio_contract": "gemini-non-audio-v1",
                    "audio_rules": [], "models": [{"source_model": "synthetic",
                    "source_operations": ["generateContent"]}]}
        price, origin = row(values, provider, 24)
        self.assertIn("ai*2", price["source_expression"])
        self.assertEqual(origin["compiled_price"]["tiered"]["tiers"][0]["rates"]["input"],
                         {"numerator": 7, "denominator": 10})
        provider["models"][0]["source_operations"] = ["audio.transcriptions"]
        with self.assertRaisesRegex(A.R.MappingError, "non_audio_contract"):
            row(values, provider, 24)

    def test_inactive_expression_currency_and_duplicate_options_reject(self):
        e = 'tier("synthetic", p*1)'
        for bad in [options(e, "ratio"), options(e, unit="1000000"), options(e)+options(e)]:
            with self.assertRaises(A.R.MappingError): A.compile_model(bad, "synthetic")

    def test_calendar_is_inventory_candidate_not_silent_time_compatibility(self):
        predicate = ('weekday("UTC")>=1&&weekday("UTC")<=5&&'
                     '((hour("UTC")>=1&&hour("UTC")<4)||(hour("UTC")>=6&&hour("UTC")<10))&&!('
                     '(date("Asia/Shanghai")>=20260101&&date("Asia/Shanghai")<=20260103))')
        expression = predicate+'?tier("peak",p*2+cr*.2+c*6):tier("off_peak",p*1+cr*.1+c*3)'
        values = options(expression)
        compiled = A.compile_model(values, "synthetic")
        self.assertFalse(compiled["calendar_selection_equivalent"])
        self.assertEqual(compiled["tiered"]["calendar"]["source_expression"], expression)
        with self.assertRaisesRegex(A.R.MappingError, "settlement_time_not_preserved"):
            row(values)

    def test_image_grammar_does_not_flatten_sixth_dimension(self):
        e = '(tier("standard", p * 7 + cr_text * .2 + img * 9 + cr_img * .3 + img_o * 11)) * (1)'
        # The actual Rust image parser requires an integer part in a decimal.
        self.assertFalse(A.image_shape(e))
        e = e.replace("*.2", "*0.2").replace("*.3", "*0.3").replace("* .2", "* 0.2").replace("* .3", "* 0.3")
        self.assertTrue(A.image_shape(e))
        for bad in [e.replace('p * 7', 'p * 7 + c * 0'),
                    e.replace('cr_text', 'cr_ text'), e.replace('standard', 'stan dard'),
                    e.replace('p * 7', 'p * 7 + p * 0')]:
            self.assertFalse(A.image_shape(bad))

    def test_scope_expression_hash_is_checked_before_conversion(self):
        expression = 'tier("synthetic", p*2)'; values = options(expression)
        scope = {"source_options_semantic_sha256": A.R.F.digest(values), "source_archive_sha256": "a"*64,
                 "models": [{"model": "synthetic", "price_kind": "tiered_expression",
                 "source_billing_mode": "tiered_expr", "expression_sha256": "0"*64,
                 "source_groups": ["reviewed-group"], "source_channel_types": [1], "consumption_rows_30d": 2}]}
        with self.assertRaisesRegex(A.R.MappingError, "expression_changed"):
            A.inventory(values, scope)

    def test_no_generic_expression_eval_or_image_as_ratio(self):
        for e in ['tier("synthetic", __import__("os").system("false"))',
                  'tier("synthetic", p * 1 + img_o * 2)', 'tier("synthetic", p * c)']:
            with self.assertRaises(A.R.MappingError): A.compile_model(options(e), "synthetic")


if __name__ == "__main__":
    unittest.main()
