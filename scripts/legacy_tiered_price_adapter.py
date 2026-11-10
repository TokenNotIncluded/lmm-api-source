#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Read-only, source-bound tiered-price candidates; no publication or SQL.

Reuse the routing compiler and its existing wire, not a second evaluator.
Candidate arithmetic is NativeTariff exact decimal, NOT bitwise legacy float.
Calendar selection time and classified image dimensions remain explicit gaps.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

import legacy_routing_migration as R

SOURCE_REVISION = "cc52aa195c6f2a01d703a1a407be73c31b89867d"
SOURCE_PATHS = (
    "apps/api-go/service/tiered_settle.go",
    "apps/api-go/pkg/billingexpr/settle.go",
    "apps/api-go/pkg/billingexpr/run.go",
    "apps/api-go/pkg/billingexpr/types.go",
    "apps/api-go/common/quota_math.go",
    "apps/api-go/setting/billing_setting/tiered_billing.go",
    "apps/api-go/relay/channel/openai/relay_image.go",
)


def options_map(options):
    R.require(type(options) is list and all(type(x) is dict and set(x) == {"key", "value"}
              and type(x["key"]) is str for x in options), "price_options_invalid")
    result = {x["key"]: x["value"] for x in options}
    R.require(len(result) == len(options), "price_options_duplicate_key")
    return result


def expression_source(options, model):
    values = options_map(options)
    modes = R.F.parse_json(values.get("billing_setting.billing_mode", "{}"))
    expressions = R.F.parse_json(values.get("billing_setting.billing_expr", "{}"))
    R.require(type(modes) is dict and type(expressions) is dict, "price_options_invalid")
    R.require(modes.get(model, "ratio") == "tiered_expr", "price_source_billing_mode_mismatch")
    expression = expressions.get(model)
    R.require(type(expression) is str and bool(expression.strip()), "price_source_expression_missing")
    R.require(str(values.get("LegacyPricingQuotaPerUnit")) == "500000",
              "price_source_currency_unreviewed")
    return {"model": model, "billing_price_key": model, "source_mode": "tiered_expr",
            "source_expression": expression,
            "source_expression_sha256": hashlib.sha256(expression.encode()).hexdigest(),
            "source_options_sha256": R.F.digest(options), "source_revision": SOURCE_REVISION,
            "arithmetic_contract": "native_decimal_half_up_positive_min_one",
            "legacy_float_bitwise_equivalent": False}


def compile_model(options, model, *, non_audio=False):
    """Return a candidate with original expression identity, not an approval.

    non_audio is a caller-supplied reviewed fact. Use provider_price_row when
    constructing provider contracts: it checks the existing adapter/operations
    constraints instead of inferring audio absence from a model's name.
    """
    result = expression_source(options, model)
    result["tiered"] = R.compile_token_expression(result["source_expression"], non_audio=non_audio)
    result["calendar_selection_equivalent"] = "calendar" not in result["tiered"]
    return result


def provider_price_row(options, model, group, group_multiplier_ppm, *, provider,
                       source_channel_type, contract_revision):
    """Build an existing provider.prices row using the existing admission gate.

    Group/routing/usage facts belong to the caller; this adapter invents none.
    Calendar candidates are available in the inventory but cannot be admitted
    as settlement-time-equivalent prices through this function.
    """
    origin = expression_source(options, model)
    R.require(R.text_reference(contract_revision) and R.text_reference(group),
              "price_contract_revision_missing")
    row = {"group": group, "model": model, "kind": "tiered_decimal",
           "source_options_sha256": origin["source_options_sha256"],
           "source_expression": origin["source_expression"],
           "source_rate_unit": "USD_per_million_tokens", "contract_revision": contract_revision,
           "group_multiplier_ppm": group_multiplier_ppm, "speed_multiplier_ppm": 1_000_000}
    values = options_map(options)
    compiled, reason = R.price_for({**provider, "prices": [row]}, group, model,
        origin["source_options_sha256"],
        R.F.parse_json(values["billing_setting.billing_expr"]),
        R.F.parse_json(values["billing_setting.billing_mode"]),
        source_channel_type=source_channel_type, source_options=options,
        source_revision=SOURCE_REVISION)
    R.require(reason is None and compiled is not None, "price_candidate_not_compiled")
    R.require("calendar" not in compiled["tiered"],
              "legacy_calendar_settlement_time_not_preserved")
    return row, {**origin, "compiled_price": compiled}


def image_shape(expression):
    """Inventory only: mirror the current five-dimensional grammar boundary.

    This does not emit an image tariff or replace Core's validation. Existing
    image sidecars still require actual endpoint, usage and reservation facts.
    """
    wrapper = re.fullmatch(r'[ \t\r\n\f]*\([ \t\r\n\f]*tier[ \t\r\n\f]*\([ \t\r\n\f]*"standard"[ \t\r\n\f]*,(.*?)\)[ \t\r\n\f]*\)[ \t\r\n\f]*\*[ \t\r\n\f]*\([ \t\r\n\f]*1[ \t\r\n\f]*\)[ \t\r\n\f]*', expression, re.S)
    if not wrapper:
        return False
    seen = set()
    for term in wrapper[1].split('+'):
        match = re.fullmatch(r'[ \t\r\n\f]*(p|cr_text|img|cr_img|img_o)[ \t\r\n\f]*\*[ \t\r\n\f]*([0-9]+(?:\.[0-9]{0,9})?)[ \t\r\n\f]*', term)
        if not match or match[1] in seen:
            return False
        seen.add(match[1])
    return seen == {"p", "cr_text", "img", "cr_img", "img_o"}


def inventory(options, scope):
    R.require(scope.get("source_options_semantic_sha256") == R.F.digest(options),
              "price_scope_options_changed")
    output, seen = [], set()
    for selected in scope["models"]:
        if selected["price_kind"] != "tiered_expression":
            continue
        model = selected["model"]
        R.require(model not in seen, "price_scope_model_duplicate")
        seen.add(model)
        source = expression_source(options, model)
        R.require(selected["source_billing_mode"] == source["source_mode"]
                  and selected["expression_sha256"] == source["source_expression_sha256"],
                  "price_scope_expression_changed")
        record = {**source, "source_groups": selected["source_groups"],
                  "source_channel_types": selected["source_channel_types"],
                  "consumption_rows_30d": selected["consumption_rows_30d"]}
        try:
            compiled = compile_model(options, model)
            tariff = compiled["tiered"]
            record.update(tiered=tariff, status="calendar_selection_time_gap" if "calendar" in tariff
                          else "native_decimal_candidate", family="calendar" if "calendar" in tariff
                          else "input_length_tiers" if len(tariff["tiers"]) > 1 else "linear_tokens")
        except R.MappingError as error:
            # Conditional candidates are analysis, never permission to remove ai.
            try:
                conditional = compile_model(options, model, non_audio=True)
                record.update(status="requires_non_audio_provider_contract", family="audio_input_term",
                              conditional_tiered=conditional["tiered"])
            except R.MappingError:
                record.update(status="existing_native_image_sidecar_candidate" if image_shape(source["source_expression"])
                              else "classified_image_dimensions_not_representable",
                              family="classified_image", compiler_error=str(error))
        output.append(record)
    return {"format_version": 1, "scope": "legacy-used-tiered-price-candidates",
            "source_revision": SOURCE_REVISION, "source_options_sha256": R.F.digest(options),
            "source_archive_sha256": scope["source_archive_sha256"],
            "status_counts": dict(Counter(r["status"] for r in output)), "models": output,
            "publication_executed": False, "exact_legacy_float_equivalence_claimed": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--options", type=Path, required=True)
    parser.add_argument("--scope", type=Path, required=True)
    parser.add_argument("--source-repository", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    # Read small manifests only; never open the full source archive or database.
    R.require(args.options.stat().st_size <= 2_000_000 and args.scope.stat().st_size <= 2_000_000,
              "price_input_too_large")
    raw = args.options.read_bytes(); scope_raw = args.scope.read_bytes()
    scope = R.F.parse_json(scope_raw.decode()); options = R.F.parse_json(raw.decode())
    R.require(hashlib.sha256(raw).hexdigest() == scope["source_options_file_sha256"],
              "price_scope_options_file_changed")
    report = inventory(options, scope)
    report["source_code_sha256"] = {name: hashlib.sha256(subprocess.check_output(
        ["git", "-c", "core.packedGitWindowSize=1m", "-c", "core.packedGitLimit=8m",
         "show", SOURCE_REVISION + ":" + name], cwd=args.source_repository)).hexdigest()
        for name in SOURCE_PATHS}
    report["compiler_sha256"] = hashlib.sha256(Path(R.__file__).read_bytes()).hexdigest()
    report["input_files"] = {str(args.options): hashlib.sha256(raw).hexdigest(),
                             str(args.scope): hashlib.sha256(scope_raw).hexdigest()}
    with open(args.output, "x", encoding="utf-8", opener=lambda p,f: os.open(p,f,0o600)) as f:
        json.dump(report, f, ensure_ascii=False, indent=2); f.write("\n")
    print(json.dumps({"models": len(report["models"]), "status_counts": report["status_counts"],
                      "report": str(args.output)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
