#!/usr/bin/env python3
"""Validate a source-release request against the checked-out registry dependency."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def unique(pairs):
    result = dict(pairs)
    if len(result) != len(pairs):
        raise ValueError("duplicate release request fields")
    return result


def validate(root):
    request = json.loads((root / ".github/source-release-request.json").read_text(), object_pairs_hook=unique)
    if not isinstance(request, dict) or set(request) != {"version", "zconv"}:
        raise ValueError("release request requires exactly version and zconv")
    for field in ("version", "zconv"):
        if not isinstance(request[field], str) or not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", request[field]):
            raise ValueError(f"invalid stable version in {field}")
    if (root / "VERSION").read_text().strip() != request["version"]:
        raise ValueError("request version differs from VERSION")
    manifest = tomllib.loads((root / "services/core/Cargo.toml").read_text())
    dependency = manifest["dependencies"]["zconv"]
    if dependency != "=" + request["zconv"] or manifest.get("patch") or manifest.get("replace"):
        raise ValueError("Core must pin the requested registry zconv without source overrides")
    lock = tomllib.loads((root / "services/core/Cargo.lock").read_text())
    packages = [package for package in lock["package"] if package["name"] == "zconv"]
    if len(packages) != 1:
        raise ValueError("expected one resolved zconv package")
    package = packages[0]
    if package["version"] != request["zconv"] or package.get("source") != REGISTRY or not re.fullmatch(r"[a-f0-9]{64}", package.get("checksum", "")):
        raise ValueError("lockfile must resolve the exact registry zconv archive")
    return request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    request = validate(ROOT)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    result = {**request, "revision": revision}
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            for key, value in result.items():
                output.write(f"{key}={value}\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
