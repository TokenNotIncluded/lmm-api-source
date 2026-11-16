#!/usr/bin/env python3
"""Build the immutable Core67 used by the frozen Pancake upgrade regression."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
REVISION = "0510c2455dabcb63e33c719ba52ac189d62612bc"
TOOLCHAIN = "1.99.0"
TARGET = ROOT / "services/core/src/core_schema/upgrade/subscription_pancake_language_target.json"


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
    parser.add_argument("--output", required=True, type=Path, help="new private evidence directory")
    parser.add_argument("--offline", action="store_true", help="require the existing Cargo cache")
    args = parser.parse_args()
    destination = args.output.absolute()
    if "\n" in str(destination) or "\r" in str(destination):
        parser.error("output path cannot contain line breaks")
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    destination = destination.resolve()
    rustc = output(["rustc", "--version"]).decode().strip()
    cargo = output(["cargo", "--version"]).decode().strip()
    if rustc.split()[1] != TOOLCHAIN or cargo.split()[1] != TOOLCHAIN:
        raise SystemExit("run with Rust 1.99.0 (mise exec rust@1.99.0 or rustup run 1.99.0)")
    revision = output(["git", "rev-parse", "--verify", REVISION + "^{commit}"]).decode().strip()
    if revision != REVISION:
        raise SystemExit("preserved Core67 Git object does not match the immutable source pin")
    archive = destination / "source.tar.gz"
    subprocess.run(
        ["git", "archive", "--format=tar.gz", "--output", str(archive), REVISION],
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
        "revision": revision,
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
        raise SystemExit("preserved Core67 build failed; see " + str(destination / "build.log"))
    binary = (destination / "target/debug/lmm-core").resolve(strict=True)
    probe = subprocess.run(
        [str(binary), "schema-manifest"], env={}, stdin=subprocess.DEVNULL,
        capture_output=True, check=True, timeout=60
    )
    if probe.stderr:
        raise SystemExit("preserved Core67 schema-manifest wrote unexpected diagnostics")
    target = json.loads(TARGET.read_text(encoding="utf-8"))
    manifest = json.loads(probe.stdout)
    manifest_digest = hashlib.sha256(probe.stdout).hexdigest()
    if manifest_digest != target["manifest_sha256"]:
        raise SystemExit("preserved Core67 manifest differs from the frozen captured target")
    if (len(manifest["components"]) != 67
            or manifest["components"][66]["name"] != "subscription-pancake-language-optional"):
        raise SystemExit("preserved Core67 has the wrong schema family")
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
    (destination / "executor.env").write_text(
        "LMM_PRESERVED_PANCAKE67_CORE_BIN=" + str(binary) + "\n"
        "LMM_PRESERVED_PANCAKE67_CORE_SHA256=" + binary_digest + "\n",
        encoding="utf-8"
    )
    print(json.dumps({"receipt": str(receipt), "environment": str(destination / "executor.env")}))


if __name__ == "__main__":
    main()
