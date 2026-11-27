#!/usr/bin/env python3
"""Bind the tested Linux Core, compiled schema and runtime SBOM to its build inputs."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
BUILD_INPUTS = (
    "deployment/docker/core.Dockerfile",
    "services/core/rust-toolchain.toml",
    "services/core/Cargo.toml",
    "services/core/Cargo.lock",
    "services/core/build.rs",
    "packaging/distribution.json",
    "config/core.example.toml",
)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def manifest(directory, revision):
    schema = json.loads((directory / "schema.json").read_bytes())
    sbom = json.loads((directory / "sbom.spdx.json").read_bytes())
    if schema.get("scope") != "compiled-core-schema-contract" or not schema.get("components"):
        raise ValueError("missing compiled schema")
    if not str(sbom.get("spdxVersion", "")).startswith("SPDX-2.") or not sbom.get("packages"):
        raise ValueError("missing runtime SPDX package inventory")
    return {
        "format": "lmm-core-ci.v1", "revision": revision, "platform": "linux-amd64",
        "build_inputs": {path: digest(ROOT / path) for path in BUILD_INPUTS},
        "files": {name: digest(directory / name) for name in ("lmm-core", "schema.json", "sbom.spdx.json")},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("record", "verify"))
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    directory = args.directory.resolve()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    record = manifest(directory, revision)
    path = directory / "build.json"
    if args.operation == "record":
        path.write_text(json.dumps(record, sort_keys=True, indent=2) + "\n")
    else:
        if json.loads(path.read_bytes()) != record:
            raise ValueError("Core binary, schema, SBOM or pinned build inputs differ")
        binary = directory / "lmm-core"
        binary.chmod(0o755)
        if subprocess.check_output([str(binary), "schema-manifest"]) != (directory / "schema.json").read_bytes():
            raise ValueError("compiled Core schema differs from CI receipt")


if __name__ == "__main__":
    main()
