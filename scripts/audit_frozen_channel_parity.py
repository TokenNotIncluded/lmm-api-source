#!/usr/bin/env python3
"""Compare frozen legacy and Core channel catalogs without opening a database."""

import argparse
import hashlib
import json
import os
import re
import subprocess
from collections import Counter, defaultdict
from decimal import Decimal
from pathlib import Path

PRICE_OPTIONS = {
    "ModelRatio",
    "CompletionRatio",
    "CacheRatio",
    "CreateCacheRatio",
    "AudioRatio",
    "AudioCompletionRatio",
    "ImageRatio",
    "ModelPrice",
    "ModelPriceLock",
    "GroupRatio",
    "GroupGroupRatio",
    "group_ratio_setting.group_ratio",
    "billing_setting.billing_mode",
    "billing_setting.billing_expr",
    "ServiceTierPricingCatalog",
    "ServiceTierPricingPolicy",
    "dynamic_pricing_setting.base_price_usd_per_million",
    "USDExchangeRate",
    "PublicCreditsPerUSD",
    "QuotaPerUnit",
    "LegacyPricingQuotaPerUnit",
}


def read_json(value):
    return json.loads(value, parse_float=Decimal)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def unescape(value):
    if value == r"\N":
        return None
    controls = {"b": "\b", "f": "\f", "n": "\n", "r": "\r", "t": "\t", "v": "\v"}

    def decode(match):
        token = match[1]
        if token[0] in "01234567":
            return chr(int(token, 8))
        if token.startswith("x") and len(token) > 1:
            return chr(int(token[1:], 16))
        return controls.get(token, token)

    return re.sub(r"\\([0-7]{1,3}|x[0-9A-Fa-f]{1,2}|.)", decode, value)


def read_table(archive, schema, table, fields):
    """Select named COPY columns in memory. No rows of SQL or raw dump data are logged."""
    command = [
        "pg_restore",
        "--data-only",
        "--schema",
        schema,
        "--table",
        table,
        "--file",
        "-",
        str(archive),
    ]
    raw = subprocess.check_output(command, encoding="utf-8")
    header = re.search(r"^COPY [^\n]+? \((.+?)\) FROM stdin;\n", raw, re.MULTILINE)
    if not header:
        raise ValueError(f"missing COPY data for {schema}.{table}")
    columns = [name.strip('"') for name in header[1].split(", ")]
    indexes = [columns.index(name) for name in fields]
    for line in raw[header.end() :].split("\n"):
        if line == r"\.":
            return
        row = line.split("\t")
        yield dict(zip(fields, (unescape(row[i]) for i in indexes)))
    raise ValueError(f"incomplete COPY data for {schema}.{table}")


def csv_set(raw):
    return sorted({field.strip() for field in (raw or "").split(",") if field.strip()})


def unique(rows, key):
    matches = [row for row in rows if key(row)]
    if len(matches) != 1:
        raise ValueError(f"expected one selected row, found {len(matches)}")
    return matches[0]


def audit(source, target, schema, expected=None, historical_resolver=None):
    channels = {}
    declared = defaultdict(set)
    for row in read_table(
        source,
        schema,
        "channels",
        [
            "id",
            "name",
            "status",
            "group",
            "models",
            "model_mapping",
            "priority",
            "weight",
        ],
    ):
        channel = int(row["id"])
        groups, models = csv_set(row["group"]), csv_set(row["models"])
        mapping = read_json(row["model_mapping"] or "{}")
        channels[channel] = {
            "id": channel,
            "name": row["name"],
            "status": int(row["status"]),
            "groups": groups,
            "models": models,
            "model_mapping": mapping,
            "priority": row["priority"],
            "weight": row["weight"],
        }
        if channels[channel]["status"] == 1:
            for group in groups:
                for model in models:
                    declared[group, model].add(channel)
    abilities = defaultdict(set)
    for row in read_table(
        source, schema, "abilities", ["channel_id", "group", "model", "enabled"]
    ):
        if row["enabled"] == "t":
            abilities[row["group"], row["model"]].add(int(row["channel_id"]))
    options = {
        r["key"]: r["value"]
        for r in read_table(source, schema, "options", ["key", "value"])
        if r["key"] in PRICE_OPTIONS
    }

    def table(name, fields):
        return read_table(target, "public", name, fields)

    active = unique(
        table("routing_active_release", ["config_version"]), lambda r: True
    )["config_version"]
    cfg = unique(
        table(
            "routing_config_versions",
            ["version", "price_version", "document", "sealed"],
        ),
        lambda r: r["version"] == active,
    )
    price = unique(
        table("routing_price_versions", ["version", "document", "sealed"]),
        lambda r: r["version"] == cfg["price_version"],
    )
    if cfg["sealed"] != "t" or price["sealed"] != "t":
        raise ValueError("active configuration or price release is unsealed")
    routing, pricing = read_json(cfg["document"]), read_json(price["document"])
    bind = [
        r
        for r in table("channel_release_bindings", ["config_version", "document"])
        if r["config_version"] == active
    ]
    if len(bind) > 1:
        raise ValueError("duplicate active channel bindings")
    names = read_json(bind[0]["document"]).get("names", {}) if bind else {}
    observed = {int(u["id"]): u["enabled"] for u in routing["upstreams"]}
    groups = {g["name"]: g["enabled"] for g in routing["groups"]}
    routes = defaultdict(list)
    for route in routing["routes"]:
        for t in route["targets"]:
            routes[route["group"], route["model"]].append(
                {
                    "channel_id": t["upstream_id"],
                    "model": t["upstream_model"],
                    "priority": t["priority"],
                    "weight": t["weight"],
                    "route_enabled": route["enabled"],
                    "target_enabled": t["enabled"],
                    "speeds": t["speeds"],
                }
            )
    prices = defaultdict(list)
    for rule in pricing["rules"]:
        prices[rule["group"], rule["model"]].append(rule)
    archived = [
        read_json(r["value"])
        for r in read_table(target, "core_console", "options", ["key", "value"])
        if r["key"] == "legacy_pricing_snapshot.v1"
    ]
    if len(archived) > 1:
        raise ValueError("duplicate legacy price snapshots")
    prior_options = (
        {o["key"]: o["value"] for o in archived[0]["options"]} if archived else {}
    )

    def equivalent(a, b):
        if a == b:
            return True
        try:
            return read_json(a) == read_json(b)
        except (TypeError, ValueError):
            return False

    prior_channels = (
        {c["id"]: c for c in archived[0]["enabled_channels"]} if archived else {}
    )
    legacy_gaps = (
        {(r["group"], r["model"]): r["reason"] for r in archived[0]["unsupported"]}
        if archived
        else {}
    )
    resolved = (
        read_json(historical_resolver.read_bytes()) if historical_resolver else {}
    )
    model_options = {
        key: read_json(options[key])
        for key in (
            "ModelPrice",
            "ModelRatio",
            "CompletionRatio",
            "CacheRatio",
            "CreateCacheRatio",
            "ImageRatio",
            "AudioRatio",
            "AudioCompletionRatio",
            "GroupRatio",
            "billing_setting.billing_mode",
            "billing_setting.billing_expr",
        )
        if key in options
    }
    results = []
    for channel in sorted(set(channels) | set(observed)):
        old = channels.get(channel)
        name = names.get(str(channel))
        archived_ch = prior_channels.get(channel)
        results.append(
            {
                "id": channel,
                "legacy": old,
                "native_enabled": observed.get(channel),
                "native_name": name,
                "native_present": channel in observed,
                "archived_name": archived_ch["name"] if archived_ch else None,
                "name_status": (
                    "target_missing"
                    if channel not in observed
                    else "source_missing"
                    if not old
                    else "native_name_unbound"
                    if name is None
                    else "equal"
                    if name == old["name"]
                    else "different"
                ),
            }
        )
    pairs = []
    for group, model in sorted(
        set(declared) | set(abilities) | set(prices) | set(routes)
    ):
        ids = declared[group, model]
        enabled_ids = abilities[group, model]
        targets = routes[group, model]
        rules = prices[group, model]
        historical = resolved.get(model, {})
        normalized = historical.get("normalized")
        pricing_inputs = {}
        for name, values in model_options.items():
            if name == "GroupRatio":
                lookup = group
            elif name.startswith("billing_setting."):
                lookup = model
            else:
                lookup = normalized if normalized is not None else model
            if lookup in values:
                pricing_inputs[name] = values[lookup]
        source_targets = {
            (cid, channels[cid]["model_mapping"].get(model, model)) for cid in ids
        }
        enabled_targets = {
            (t["channel_id"], t["model"])
            for t in targets
            if groups.get(group)
            and observed.get(t["channel_id"])
            and t["route_enabled"]
            and t["target_enabled"]
        }
        pairs.append(
            {
                "group": group,
                "model": model,
                "source_declared_ids": sorted(ids),
                "source_enabled_ability_ids": sorted(enabled_ids),
                "legacy_explicit_price_inputs": pricing_inputs,
                "historical_normalized_model": normalized,
                "historical_completion_resolution": historical or None,
                "missing_native_targets": sorted(source_targets - enabled_targets),
                "extra_native_targets": sorted(enabled_targets - source_targets),
                "native_targets": targets,
                "native_prices": rules,
                "historical_unsupported_reason": legacy_gaps.get((group, model)),
            }
        )
    source_pairs = set(k for k, v in declared.items() if v)
    native_priced = set(k for k, v in prices.items() if v)
    actually_routed = {
        k
        for k, v in routes.items()
        if groups.get(k[0])
        and any(
            t["route_enabled"] and t["target_enabled"] and observed.get(t["channel_id"])
            for t in v
        )
    }
    unsupported_keys = set(legacy_gaps)
    old_ids = {k for k, v in channels.items() if v["status"] == 1}
    active_ids = {k for k, v in observed.items() if v}
    legacy_groups = {g for g, m in source_pairs}
    summary = {
        "legacy_channels_total": len(channels),
        "legacy_enabled_channels": len(old_ids),
        "native_upstreams": len(observed),
        "native_enabled_upstreams": len(active_ids),
        "enabled_legacy_ids_missing_in_native": sorted(old_ids - set(observed)),
        "native_ids_missing_in_legacy": sorted(set(observed) - set(channels)),
        "enabled_native_ids_disabled_in_source": sorted(
            (active_ids & channels.keys()) - old_ids
        ),
        "source_pairs_with_exact_target_channel_model_set": sum(
            bool(p["source_declared_ids"])
            and not p["missing_native_targets"]
            and not p["extra_native_targets"]
            for p in pairs
        ),
        "active_source_models_without_historical_normalization": sum(
            1 for model in {m for g, m in source_pairs} if model not in resolved
        ),
        "native_name_bindings": len(names),
        "enabled_source_name_status": dict(
            Counter(r["name_status"] for r in results if r["id"] in old_ids)
        ),
        "source_groups": sorted(legacy_groups),
        "native_groups": groups,
        "source_model_group_pairs": len(source_pairs),
        "source_distinct_models": len({m for g, m in source_pairs}),
        "native_rule_count": len(pricing["rules"]),
        "source_pairs_with_native_price": len(source_pairs & native_priced),
        "source_pairs_without_price": len(source_pairs - native_priced),
        "native_only_price_pairs": len(native_priced - source_pairs),
        "source_pairs_with_enabled_native_route": len(source_pairs & actually_routed),
        "source_pairs_without_enabled_native_route": len(
            source_pairs - actually_routed
        ),
        "source_ability_channel_sets_differ": sum(
            declared[k] != abilities[k] for k in set(declared) | set(abilities)
        ),
        "source_unpriced_with_historical_reason": len(
            (source_pairs - native_priced) & unsupported_keys
        ),
        "prior_option_changes": sorted(
            k
            for k in PRICE_OPTIONS
            if k in prior_options and not equivalent(prior_options[k], options.get(k))
        ),
        "options_without_archived_baseline": sorted(set(options) - set(prior_options)),
        "routing_version": active,
        "price_version": cfg["price_version"],
    }
    if expected:
        baseline = read_json(expected.read_bytes())
        summary["native_price_rules_equal_frozen_reference"] = (
            baseline["rules"] == pricing["rules"]
        )
    return {
        "source_sha256": sha256(source),
        "target_sha256": sha256(target),
        "historical_price_reference_sha256": sha256(expected) if expected else None,
        "historical_resolver_sha256": sha256(historical_resolver)
        if historical_resolver
        else None,
        "pricing_scope": "Source options are preserved as original text. Native rules are actual stored rules. "
        "Missing per-model overrides are unknown here. Historical resolution is not a new provider quote. "
        "Rule presence and enabled targets do not establish settlement or upstream availability.",
        "summary": summary,
        "channels": results,
        "model_group_pairs": pairs,
        "source_pricing_options": options,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--legacy-dump", required=True, type=Path)
    parser.add_argument("--legacy-schema", required=True)
    parser.add_argument("--core-dump", required=True, type=Path)
    parser.add_argument("--reference-prices", type=Path)
    parser.add_argument(
        "--historical-resolver",
        type=Path,
        help="Optional frozen normalized-model and completion-ratio observations",
    )
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    report = audit(
        args.legacy_dump,
        args.core_dump,
        args.legacy_schema,
        args.reference_prices,
        args.historical_resolver,
    )
    with os.fdopen(
        os.open(args.output, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600), "w"
    ) as f:
        json.dump(report, f, ensure_ascii=False, indent=2, default=str)
        f.write("\n")
    print(json.dumps(report["summary"], ensure_ascii=False))


if __name__ == "__main__":
    main()
