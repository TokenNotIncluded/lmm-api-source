#!/usr/bin/env python3
"""Link the small test harness only after the owner supplies a matching freeze."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
INPUTS = (
    "services/core/Cargo.lock",
    "services/core/schema/routing.sql",
    "services/core/src/routing/config.rs",
    "services/core/src/routing/engine.rs",
    "services/core/src/routing/group_name.rs",
    "services/core/src/routing/mod.rs",
    "services/core/src/routing/pricing.rs",
    "services/core/src/routing/tiered.rs",
)
STORE = "services/core/src/routing/store.rs"


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def checked_sha(path: Path, expected: str) -> None:
    actual = sha(path)
    if actual != expected:
        raise SystemExit(f"SHA mismatch for {path}: expected {expected}, got {actual}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", type=Path, required=True)
    parser.add_argument("--rlib-sha256", required=True)
    parser.add_argument("--pins-sha256", required=True)
    parser.add_argument("--config-sha256", required=True)
    parser.add_argument("--cache-pins-sha256", required=True)
    parser.add_argument("--source-store-sha256", required=True,
                        help="current store.rs used by the source harness")
    parser.add_argument("--out", type=Path, required=True, help="new private output directory")
    args = parser.parse_args()
    for value in (args.rlib_sha256, args.pins_sha256, args.config_sha256, args.cache_pins_sha256,
                  args.source_store_sha256):
        if re.fullmatch(r"[0-9a-f]{64}", value) is None:
            parser.error("each expected SHA-256 must be 64 lowercase hexadecimal digits")
    freeze = args.freeze.resolve(strict=True)
    pins_path = freeze / "source-pins.json"
    config_path = freeze / "compile-config.json"
    cache_pins_path = freeze / "cache-pins.json"
    rlib = freeze / "liblmm_core.rlib"
    checked_sha(pins_path, args.pins_sha256)
    checked_sha(config_path, args.config_sha256)
    checked_sha(cache_pins_path, args.cache_pins_sha256)
    checked_sha(rlib, args.rlib_sha256)
    pins = json.loads(pins_path.read_text(encoding="utf-8"))
    for relative, pinned_sha in pins.items():
        source = (freeze / "source" / relative).resolve(strict=True)
        if not source.is_relative_to((freeze / "source").resolve(strict=True)):
            raise SystemExit(f"source pin escapes freeze: {relative}")
        checked_sha(source, pinned_sha)
    for relative in INPUTS:
        if relative not in pins:
            raise SystemExit(f"missing source pin: {relative}")
        checked_sha(ROOT / relative, pins[relative])
    if STORE not in pins:
        raise SystemExit(f"missing source pin: {STORE}")
    checked_sha(ROOT / STORE, args.source_store_sha256)

    config = json.loads(config_path.read_text(encoding="utf-8"))
    cache_pins = json.loads(cache_pins_path.read_text(encoding="utf-8"))
    cache_files = [config["compiler"]]
    cache_files.extend(
        config["externs"][index + 1].split("=", 1)[1]
        for index, item in enumerate(config["externs"])
        if item == "--extern"
    )
    for name in cache_files:
        if name not in cache_pins:
            raise SystemExit(f"missing compiler/dependency cache pin: {name}")
        checked_sha(Path(name), cache_pins[name])
    test_inputs = {
        name: sha(HERE / name)
        for name in ("publish_harness.rs", "publish_tests.rs", "publish_compile.py", "publish_run.py")
    }
    out = args.out.absolute()
    out.mkdir(mode=0o700, parents=False, exist_ok=False)
    binary = out / "publish-harness"
    command = [
        config["compiler"],
        "--edition=2024",
        "--test",
        str(HERE / "publish_harness.rs"),
        "--extern",
        f"lmm_core={rlib}",
        *config["externs"],
        *config["search"],
        *config["link"],
        "-o",
        str(binary),
    ]
    env = os.environ.copy()
    env["CARGO_MANIFEST_DIR"] = str(ROOT / "services/core")
    env["OUT_DIR"] = config["out_dir"]
    with (out / "compile.log").open("xb") as log:
        result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT, check=False)
    binding_intact = (
        sha(rlib) == args.rlib_sha256
        and sha(pins_path) == args.pins_sha256
        and sha(config_path) == args.config_sha256
        and sha(cache_pins_path) == args.cache_pins_sha256
        and all(sha(Path(name)) == cache_pins[name] for name in cache_files)
        and all(sha(ROOT / name) == pins[name] for name in INPUTS)
        and sha(ROOT / STORE) == args.source_store_sha256
        and all(sha(HERE / name) == digest for name, digest in test_inputs.items())
    )
    evidence = {
        "freeze": str(freeze),
        "rlib_sha256": args.rlib_sha256,
        "source_pins_sha256": args.pins_sha256,
        "compile_config_sha256": args.config_sha256,
        "cache_pins_sha256": args.cache_pins_sha256,
        "verified_compiler_and_externs": len(cache_files),
        "source_inputs": {name: pins[name] for name in INPUTS},
        "frozen_store_sha256": pins[STORE],
        "source_store_sha256": args.source_store_sha256,
        "frozen_store_matches_tested_source": pins[STORE] == args.source_store_sha256,
        "scope": "source-included transaction entry; frozen library public publish only",
        "test_inputs": test_inputs,
        "binding_intact": binding_intact,
        "compile_exit": result.returncode,
    }
    if result.returncode == 0:
        evidence["binary_sha256"] = sha(binary)
    (out / "compile-result.json").write_text(
        json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(evidence, ensure_ascii=False))
    if result.returncode:
        raise SystemExit(result.returncode)
    if not binding_intact:
        raise SystemExit("inputs changed during compilation; evidence is invalid")


if __name__ == "__main__":
    main()
