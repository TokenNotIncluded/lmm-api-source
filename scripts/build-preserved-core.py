#!/usr/bin/env python3
"""Build the immutable Core executors used by frozen upgrade regressions."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN = "1.99.0"
FAMILIES = {
    "pancake67": {
        "revision": "0510c2455dabcb63e33c719ba52ac189d62612bc",
        "target": "subscription_pancake_language_target.json",
        "count": 67,
        "last_component": "subscription-pancake-language-optional",
        "environment": "PANCAKE67",
    },
    "early58": {
        "revision": "2a0c7bd380f58a553dc903d399255821c1a65d0f",
        "target": "commerce_display_rate_target.json",
        "count": 58,
        "last_component": "display-exchange-rate",
        "environment": "EARLY58",
    },
}


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def output(command, *, cwd=ROOT, env=None):
    return subprocess.run(
        command, cwd=cwd, env=env, check=True, capture_output=True, timeout=60
    ).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--family", choices=FAMILIES, default="pancake67")
    parser.add_argument("--output", required=True, type=Path, help="new private evidence directory")
    parser.add_argument("--offline", action="store_true", help="require the existing Cargo cache")
    args = parser.parse_args()
    spec = FAMILIES[args.family]
    revision_pin = spec["revision"]
    destination = args.output.absolute()
    if "\n" in str(destination) or "\r" in str(destination):
        parser.error("output path cannot contain line breaks")
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    destination = destination.resolve()
    rustc = output(["rustc", "--version"]).decode().strip()
    cargo = output(["cargo", "--version"]).decode().strip()
    if rustc.split()[1] != TOOLCHAIN or cargo.split()[1] != TOOLCHAIN:
        raise SystemExit("run with Rust 1.99.0 (mise exec rust@1.99.0 or rustup run 1.99.0)")
    revision = output(["git", "rev-parse", "--verify", revision_pin + "^{commit}"]).decode().strip()
    if revision != revision_pin:
        raise SystemExit("preserved Core Git object does not match the immutable source pin")
    archive = destination / "source.tar.gz"
    subprocess.run(
        ["git", "archive", "--format=tar.gz", "--output", str(archive), revision_pin],
        cwd=ROOT, check=True, timeout=60
    )
    source = destination / "source"
    source.mkdir()
    with tarfile.open(archive, "r:gz") as bundle:
        bundle.extractall(source, filter="data")
    manifest_path = source / "services/core/Cargo.toml"
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(destination / "target")
    environment["GIT_CEILING_DIRECTORIES"] = str(source)
    environment.setdefault("CARGO_BUILD_JOBS", "2")
    command = ["cargo", "build", "--locked", "--manifest-path", str(manifest_path), "--bin", "lmm-core"]
    if args.offline:
        command.append("--offline")
    report = {
        "family": args.family,
        "revision": revision,
        "source": str(source),
        "source_archive_sha256": digest(archive),
        "cargo_lock_sha256": digest(source / "services/core/Cargo.lock"),
        "toolchain": {"rustc": rustc, "cargo": cargo},
        "command": command,
        "state": "BUILDING",
    }
    receipt = destination / "provenance.json"
    receipt.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    with (destination / "build.log").open("wb") as log:
        result = subprocess.run(
            command, cwd=source, env=environment, stdout=log, stderr=subprocess.STDOUT,
            timeout=2400
        )
    report["build_exit"] = result.returncode
    if result.returncode:
        report["state"] = "BUILD_FAILED"
        receipt.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        raise SystemExit("preserved Core build failed; see " + str(destination / "build.log"))
    binary = (destination / "target/debug/lmm-core").resolve(strict=True)
    probe = subprocess.run(
        [str(binary), "schema-manifest"], env={}, stdin=subprocess.DEVNULL,
        capture_output=True, check=True, timeout=60
    )
    if probe.stderr:
        raise SystemExit("preserved Core schema-manifest wrote unexpected diagnostics")
    target_file = ROOT / "services/core/src/core_schema/upgrade" / spec["target"]
    target = json.loads(target_file.read_text(encoding="utf-8"))
    manifest = json.loads(probe.stdout)
    manifest_digest = hashlib.sha256(probe.stdout).hexdigest()
    if manifest_digest != target["manifest_sha256"]:
        raise SystemExit("preserved Core manifest differs from the frozen captured target")
    if (len(manifest["components"]) != spec["count"]
            or manifest["components"][-1]["name"] != spec["last_component"]):
        raise SystemExit("preserved Core has the wrong schema family")
    (destination / "schema-manifest.json").write_bytes(probe.stdout)
    binary_digest = digest(binary)
    report.update({
        "state": "BUILD_AND_MANIFEST_VERIFIED",
        "binary": str(binary),
        "binary_sha256": binary_digest,
        "schema_manifest_stdout_sha256": manifest_digest,
        "catalog_sha256": target["catalog_sha256"],
    })
    receipt.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    environment_prefix = "LMM_PRESERVED_" + spec["environment"]
    (destination / "executor.env").write_text(
        environment_prefix + "_CORE_BIN=" + str(binary) + "\n"
        + environment_prefix + "_CORE_SHA256=" + binary_digest + "\n",
        encoding="utf-8"
    )
    print(json.dumps({"receipt": str(receipt), "environment": str(destination / "executor.env")}))


if __name__ == "__main__":
    main()
