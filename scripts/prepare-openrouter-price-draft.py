#!/usr/bin/env python3
"""Prepare native token sale-price drafts from a locally captured OpenRouter /api/v1/models feed.

Choose existing rule IDs explicitly. Supply the full GET /core/v1/admin/routing/prices
response with --prices-file for an offline plan, or --core-url and --session-file for
Core's authorized preview. Review the result, then repeat with --save-draft ID and
--accept PLAN_SHA256 to save a new draft. Publish separately in native pricing management.
The accepted public pricing shape is described at https://openrouter.ai/docs/guides/overview/models .
All five native rates must be explicit. Missing request/cache amounts, nonzero
unrepresented dimensions and conditional overrides leave the whole model unchanged.
The ordinary token contract supports input, output, and cache reads. Native text
request fees require a request-only quote. The report lists incompatible rules
with their retained prices. Token rates must convert exactly to ledger credits
after applying the selected rule's multipliers. The maximum native rate is 2^63-1. The feed SHA-256
identifies the source version in the saved draft name and review report.
"""
import argparse
from collections import Counter
from copy import deepcopy
from decimal import Decimal, InvalidOperation
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import sys
from urllib.error import URLError
from urllib.parse import urlsplit
from urllib.request import build_opener, HTTPRedirectHandler, ProxyHandler, Request

PRICES = "/core/v1/admin/routing/prices"
LIMIT = 32 * 1024 * 1024
MAX_INT = 2**63 - 1  # PriceSnapshot::compile and the PostgreSQL release schema
RATES = {"prompt": "input", "completion": "output", "input_cache_read": "cache_read",
         "input_cache_write": "cache_write", "request": "request"}
ZERO_ONLY = {"image", "web_search", "internal_reasoning"}
MODEL = re.compile(r"[A-Za-z0-9._:/@~-]{1,200}", re.ASCII)
DECIMAL = re.compile(r"(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?", re.ASCII)


class FeedObject(dict):
    """Preserve duplicate selectors/fields so ambiguous rows cannot win by ordering."""
    def __init__(self, pairs):
        super().__init__(pairs)
        self.ambiguous = len(self) != len(pairs)
        self.ids = {value for key, value in pairs if key == "id" and isinstance(value, str)}


def bounded(stream):
    raw = stream.read(LIMIT + 1)
    if len(raw) > LIMIT:
        raise ValueError("JSON document exceeds size limit")
    return raw


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def decimal_price(raw):
    if isinstance(raw, (dict, list)):
        raise ValueError("structured_price")
    if isinstance(raw, str) and raw.lower().lstrip("+-") in {"nan", "snan", "inf", "infinity"}:
        raise ValueError("nonfinite_price")
    if isinstance(raw, str) and raw.startswith("-") and DECIMAL.fullmatch(raw[1:]):
        raise ValueError("negative_price")
    if not isinstance(raw, str) or len(raw) > 128 or not DECIMAL.fullmatch(raw):
        raise ValueError("invalid_decimal")
    try:
        amount = Decimal(raw)
    except InvalidOperation:
        raise ValueError("invalid_decimal") from None
    if not amount.is_finite():
        raise ValueError("invalid_decimal")
    return amount


def native_amount(raw, places):
    # JSON prices on the Models API are decimal strings. Native tokens are
    # micro-USD per million tokens (10^12), requests are micro-USD each (10^6).
    # Refuse fractional native units instead of rounding a published price.
    amount = decimal_price(raw)
    _, digits, exponent = amount.as_tuple()
    if amount.is_zero():
        return 0
    shift = exponent + places
    if shift < 0:
        if -shift >= len(digits) or any(digits[shift:]):
            raise ValueError("fractional_native_unit")
        digits, shift = digits[:shift], 0
    if len(digits) + shift > 19:
        raise ValueError("native_rate_overflow")
    value = int("".join(map(str, digits))) * 10**shift
    if value > MAX_INT:
        raise ValueError("native_rate_overflow")
    return value


def rates_for(row):
    prices = row.get("pricing")
    if row.ambiguous:
        raise ValueError("duplicate_model_field")
    if not isinstance(prices, FeedObject):
        raise ValueError("missing_or_invalid_pricing_object")
    if prices.ambiguous:
        raise ValueError("duplicate_pricing_field")
    for field in prices:
        if field not in RATES and field not in ZERO_ONLY and field != "overrides":
            raise ValueError("unknown_price_dimension")
    if "overrides" in prices and prices["overrides"] != []:
        raise ValueError("unsupported_conditional_pricing")
    for field in sorted(ZERO_ONLY.intersection(prices)):
        try:
            amount = decimal_price(prices[field])
        except ValueError as error:
            raise ValueError(f"{error}:{field}") from None
        if not amount.is_zero():
            raise ValueError("unsupported_unit_price:" + field)
    rates = {}
    for field, target in RATES.items():
        if field not in prices:
            raise ValueError("missing_price:" + field)
        try:
            rates[target] = native_amount(prices[field], 6 if field == "request" else 12)
        except ValueError as error:
            raise ValueError(f"{error}:{field}") from None
    return rates


def feed_quotes(raw):
    # Non-standard JSON NaN/Infinity values are marked invalid within their
    # individual price row. They can never enter a Core request.
    data = json.loads(raw, object_pairs_hook=FeedObject, parse_constant=str,
                      parse_float=lambda _: None, parse_int=lambda _: None)
    if not isinstance(data, FeedObject) or data.ambiguous or not isinstance(data.get("data"), list):
        raise ValueError("invalid_feed_envelope")
    rows = data["data"]
    ids = Counter(model for row in rows if isinstance(row, FeedObject) for model in row.ids)
    quotes, skipped = {}, []
    for index, row in enumerate(rows):
        if not isinstance(row, FeedObject) or len(row.ids) != 1:
            skipped.append({"row": index, "reason": "missing_or_ambiguous_model_id"})
            continue
        model = next(iter(row.ids))
        if not MODEL.fullmatch(model):
            skipped.append({"row": index, "reason": "invalid_model_id"})
            continue
        try:
            if ids[model] != 1:
                raise ValueError("duplicate_model")
            quotes[model] = rates_for(row)
        except ValueError as error:
            skipped.append({"row": index, "model": model, "reason": str(error)})
    return quotes, skipped


def valid_id(value):
    return type(value) is int and 0 < value <= MAX_INT


def charging_issue(rule, rates):
    # Mirrors Core's credit_price_with_calendar for non-tiered native rules.
    if rates["request"] and any(rates[k] for k in ("input", "output", "cache_read", "cache_write")):
        return "request_fee_mixed_with_token_prices"
    if rates["cache_write"]:
        return "unsupported_cache_write_price"
    # 500,000 credits/USD, 1,000,000 micro-USD/USD and two ppm multipliers.
    denominator = 2_000_000_000_000
    multiplier = rule["group_multiplier_ppm"] * rule["speed_multiplier_ppm"]
    if rates["request"]:
        # TextRequestPrice permits whole-credit ceiling per completed request.
        if rates["request"] * multiplier > MAX_INT * denominator:
            return "credit_rate_overflow:request"
    else:
        for field in ("input", "output", "cache_read"):
            credits, remainder = divmod(rates[field] * multiplier, denominator)
            if remainder:
                return "fractional_credit_rate:" + field
            if credits > MAX_INT:
                return "credit_rate_overflow:" + field
    return None


def prepare(raw, current, requested):
    quotes, skipped = feed_quotes(raw)
    if not isinstance(current, dict):
        raise ValueError("invalid_native_price_snapshot")
    version, active = current["version"], current["prices"]
    if (not isinstance(version, dict) or not isinstance(active, dict)
            or not valid_id(version["routing"]) or not valid_id(version["pricing"])
            or version["pricing"] != active["version"] or not isinstance(active["rules"], list)):
        raise ValueError("invalid_native_price_snapshot")
    if not requested or not all(valid_id(i) for i in requested) or len(requested) != len(set(requested)):
        raise ValueError("choose distinct existing rule IDs")
    prices = deepcopy(active)
    by_id = {rule["id"]: rule for rule in prices["rules"]}
    if len(by_id) != len(prices["rules"]) or not set(requested) <= by_id.keys():
        raise ValueError("unknown or duplicate native rule ID")
    changed, unchanged, skipped_rules = [], [], []
    for rule_id in sorted(requested):
        rule = by_id[rule_id]
        model = rule["model"]
        if rule.get("billing_mode", "tokens") != "tokens" or rule.get("rounding") or rule.get("tiered"):
            skipped_rules.append({"id": rule_id, "model": model, "reason": "non_native_token_rule"})
        elif model not in quotes:
            skipped_rules.append({"id": rule_id, "model": model, "reason": "no_complete_unique_quote"})
        elif rule["rates"] == quotes[model]:
            unchanged.append(rule_id)
        elif reason := charging_issue(rule, quotes[model]):
            skipped_rules.append({"id": rule_id, "model": model, "reason": reason,
                                  "retained_rates": rule["rates"]})
        else:
            changed.append({"id": rule_id, "model": model, "before": rule["rates"], "after": quotes[model]})
            rule["rates"] = quotes[model]
    body = None
    if changed:
        if version["pricing"] == MAX_INT:
            raise ValueError("native_price_version_exhausted")
        prices["version"] += 1
        body = {"expected_version": version, "prices": prices}
    report = {"contract": "openrouter-native-token-prices-v1", "feed_sha256": sha(raw),
              "base_version": version, "changed": changed, "unchanged_rule_ids": unchanged,
              "skipped_rows": skipped, "skipped_rules": skipped_rules, "preview_request": body}
    report["plan_sha256"] = sha(encoded(report))
    return report


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def core_transport(origin, session_file):
    parsed = urlsplit(origin)
    if not parsed.hostname or parsed.username is not None or parsed.password is not None or parsed.path not in ("", "/") or parsed.query or parsed.fragment:
        raise ValueError("expected a Core origin")
    if parsed.scheme != "https":
        try:
            loopback = ipaddress.ip_address(parsed.hostname).is_loopback
        except ValueError:
            loopback = parsed.hostname == "localhost"
        if parsed.scheme != "http" or not loopback:
            raise ValueError("Core requires HTTPS or loopback HTTP")
    with os.fdopen(os.open(session_file, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as f:
        mode = os.fstat(f.fileno()).st_mode
        if not stat.S_ISREG(mode) or mode & 0o077:
            raise ValueError("expected a private regular session file")
        token = f.read(8193)
    token = token.rstrip(b"\r\n")
    if not 0 < len(token) <= 8192 or not all(33 <= c <= 126 for c in token):
        raise ValueError("invalid session file")
    opener = build_opener(ProxyHandler({}), NoRedirect())
    def call(method, suffix="", body=None):
        req = Request(origin.rstrip("/") + PRICES + suffix, method=method,
                      headers={"Authorization": "Bearer " + token.decode("ascii"), "Content-Type": "application/json"},
                      data=None if body is None else encoded(body))
        with opener.open(req, timeout=30) as response:
            value = json.loads(bounded(response))
        if value.get("success") is not True:
            raise ValueError("Core rejected the request")
        return value["data"]
    return call


def review(report, call, draft_id, accepted):
    body = report["preview_request"]
    if body is None:
        return {"status": "no_changes"}
    preview = call("POST", "/preview", body)
    digest = preview["confirmation_digest"]
    if (preview.get("saved") is not False or preview.get("expected_version") != body["expected_version"]
            or preview.get("after") != body["prices"] or not isinstance(digest, str)
            or not re.fullmatch(r"[0-9a-f]{64}", digest)):
        raise ValueError("Core preview differs from reviewed snapshot")
    if draft_id is None:
        return {"status": "previewed", "confirmation_digest": digest}
    name = "OpenRouter " + report["feed_sha256"]
    listing = call("GET", "/drafts")
    if listing["version"] != body["expected_version"]:
        raise ValueError("Core version changed; preview again")
    for entry in listing["items"]:
        draft = entry["draft"]
        if draft["id"] == draft_id:
            if not entry["stale"] and draft["name"] == name and draft["prices"] == body["prices"]:
                return {"status": "already_saved", "draft_id": draft_id}
            raise ValueError("draft ID is occupied")
    if accepted != report["plan_sha256"]:
        raise ValueError("saving requires the current reviewed plan SHA-256")
    saved = call("PUT", "/drafts/" + str(draft_id), {**body, "name": name})
    if saved["id"] != draft_id or saved.get("published") is not False:
        raise ValueError("unexpected save receipt; inspect the Core draft before retrying")
    return {"status": "saved", "draft_id": draft_id, "version": saved["version"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--feed", type=Path, required=True)
    parser.add_argument("--rule-id", type=int, action="append", required=True)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--prices-file", type=Path, help="saved GET prices response, or - for stdin")
    source.add_argument("--core-url")
    parser.add_argument("--session-file", type=Path)
    parser.add_argument("--save-draft", type=int)
    parser.add_argument("--accept")
    args = parser.parse_args()
    if (bool(args.core_url) != bool(args.session_file)
            or (args.save_draft is None) != (args.accept is None)
            or args.save_draft is not None and (not args.core_url or not valid_id(args.save_draft))):
        parser.error("use a session file with --core-url and pair --save-draft with --accept in online mode")
    call = None
    if args.core_url:
        call = core_transport(args.core_url, args.session_file)
        current = call("GET")
    else:
        if str(args.prices_file) == "-":
            current = json.loads(bounded(sys.stdin.buffer))
        else:
            with args.prices_file.open("rb") as f:
                current = json.loads(bounded(f))
        if not isinstance(current, dict) or current.get("success") is not True:
            raise ValueError("expected successful native GET prices response")
        current = current["data"]
    with args.feed.open("rb") as f:
        report = prepare(bounded(f), current, args.rule_id)
    if call:
        report["core"] = review(report, call, args.save_draft, args.accept)
    print(json.dumps(report, ensure_ascii=True, allow_nan=False))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, RecursionError, URLError) as error:
        sys.exit("price draft failed: " + str(error))
