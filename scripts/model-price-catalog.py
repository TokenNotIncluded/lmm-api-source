#!/usr/bin/env python3
"""Maintain official model prices in TOML and prepare Core publication previews."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
import tempfile
import tomllib

FIELDS = {"rates", "billing_mode", "tiered", "source", "source_unit"}
REQUIRED = {"rates", "source", "source_unit"}
RATE_UNIT = "micro-USD/1M tokens; request: micro-USD/request"


def fraction_integers(value, writing=False, path=()):
    if isinstance(value, dict):
        return {key: fraction_integers(item, writing, (*path, key))
                for key, item in value.items()}
    if isinstance(value, list):
        return [fraction_integers(item, writing, path) for item in value]
    if ("tiered" in path and len(path) >= 3 and path[-3] == "rates"
            and path[-1] in {"numerator", "denominator"}):
        if writing and type(value) is int and value >= 2**63:
            return str(value)
        if not writing and isinstance(value, str) and re.fullmatch(r"0|[1-9][0-9]*", value):
            return int(value)
    return value



def validate_model(name, price):
    if not isinstance(name, str) or not name or any(ord(c) < 32 for c in name):
        raise ValueError("invalid model name")
    if not isinstance(price, dict) or not REQUIRED <= price.keys() <= FIELDS:
        raise ValueError(f"{name}: expected an official price without group or multiplier fields")
    return price


def quote(value):
    return json.dumps(value, ensure_ascii=False).replace(chr(127), r"\u007f")


def toml_value(value):
    if isinstance(value, str):
        return quote(value)
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        if not -(2**63) <= value < 2**63:
            raise ValueError("TOML integer is outside the signed 64-bit range")
        return str(value)
    if isinstance(value, dict):
        fields = (f"{quote(k)} = {toml_value(v)}" for k, v in value.items() if v is not None)
        return "{ " + ", ".join(fields) + " }"
    if isinstance(value, list):
        return "[" + ", ".join(map(toml_value, value)) + "]"
    raise ValueError(f"unsupported TOML value: {type(value).__name__}")


def read_config(path):
    data = json.loads(path.read_text())
    return data["data"] if data.get("success") is True else data


def export_catalog(config, destination):
    models = config["bindings"]["official_prices"]["models"]
    destination = destination.resolve()
    if destination.exists():
        raise FileExistsError(destination)
    with tempfile.TemporaryDirectory(prefix=".price-catalog-", dir=destination.parent) as temp:
        staged = Path(temp) / "catalog"
        (staged / "models").mkdir(parents=True)
        index = ["version = 1", "", "[models]"]
        for model, price in sorted(models.items()):
            validate_model(model, price)
            digest = hashlib.sha256(model.encode()).hexdigest()
            relative = f"models/{digest}.toml"
            index.append(f"{quote(model)} = {quote(relative)}")
            fields = {"model": model, "currency": "USD", "rate_unit": RATE_UNIT}
            fields.update({key: price[key] for key in sorted(price) if price[key] is not None})
            (staged / relative).write_text(
                "".join(f"{key} = {toml_value(value)}\n"
                        for key, value in fraction_integers(fields, writing=True).items())
            )
        (staged / "index.toml").write_text("\n".join(index) + "\n")
        staged.rename(destination)


def load_catalog(index_path):
    index = tomllib.loads(index_path.read_text())
    if set(index) != {"version", "models"} or type(index["version"]) is not int or index["version"] != 1:
        raise ValueError("expected a version 1 TOML index with [models]")
    if not isinstance(index["models"], dict):
        raise ValueError("[models] must map model names to paths")
    models = {}
    seen = set()
    root = index_path.parent.resolve()
    for model, relative in index["models"].items():
        if not isinstance(relative, str) or not re.fullmatch(r"models/[A-Za-z0-9_.-]+\.toml", relative):
            raise ValueError(f"{model}: expected a .toml file directly under models/")
        path = root / relative
        if path.resolve() != path or path in seen:
            raise ValueError(f"{model}: model files must be unique and cannot be symlinks")
        seen.add(path)
        fields = tomllib.loads(path.read_text())
        if (fields.pop("model", None) != model or fields.pop("currency", None) != "USD"
                or fields.pop("rate_unit", None) != RATE_UNIT):
            raise ValueError(f"{model}: model ID or pricing unit does not match the index")
        models[model] = validate_model(model, fraction_integers(fields))
    return models


def prepare_preview(config, models):
    version = config["version"]
    bindings = config["bindings"]
    book = bindings.get("official_prices")
    if book is None:
        groups, speeds = {}, {}
        for rule in config["prices"]["rules"]:
            for values, key, multiplier in (
                (groups, "group", "group_multiplier_ppm"),
                (speeds, "speed", "speed_multiplier_ppm"),
            ):
                name, ppm = rule[key], rule[multiplier]
                if values.setdefault(name, ppm) != ppm:
                    raise ValueError(f"inconsistent existing {key} multiplier: {name}")
        if speeds.get("standard", 1_000_000) != 1_000_000:
            raise ValueError("existing standard speed multiplier requires review")
        book = {
            "group_multipliers_ppm": groups,
            "speed_multipliers_ppm": {
                name: speeds.get(name, 1_000_000) for name in ("fast", "ultrafast")
            },
        }
    return {
        "expected_version": version,
        "routing": {**config["routing"], "version": version["routing"] + 1},
        "prices": {**config["prices"], "version": version["pricing"] + 1},
        "bindings": {**bindings, "official_prices": {**book, "models": models}},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    export = sub.add_parser("export", help="write a new directory from a Core configuration response")
    export.add_argument("configuration", type=Path)
    export.add_argument("directory", type=Path)
    preview = sub.add_parser("preview", help="emit a configuration preview request")
    preview.add_argument("configuration", type=Path)
    preview.add_argument("index", type=Path)
    args = parser.parse_args()
    try:
        config = read_config(args.configuration)
        if args.action == "export":
            export_catalog(config, args.directory)
        else:
            print(json.dumps(prepare_preview(config, load_catalog(args.index)), ensure_ascii=False, indent=2))
    except (KeyError, OSError, TypeError, ValueError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")


if __name__ == "__main__":
    main()
