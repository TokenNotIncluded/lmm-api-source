#!/usr/bin/env python3
"""Fetch exact Core bytes from the canonical successful main CI attempt, if available."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import zipfile

REPO = "TokenNotIncluded/lmm-api-source"
WORKFLOW = ".github/workflows/core-protocol.yml"
MEMBERS = {"lmm-core", "build.json", "schema.json", "sbom.spdx.json", "receipt.json", "SHA256SUMS"}


def get(endpoint):
    return json.loads(subprocess.check_output(["gh", "api", f"repos/{REPO}/{endpoint}"]))


def select(revision, api=get):
    runs = api(f"actions/workflows/core-protocol.yml/runs?branch=main&head_sha={revision}&per_page=100")
    matching = [run for run in runs["workflow_runs"] if (
        run["path"] == WORKFLOW and run["head_branch"] == "main" and run["head_sha"] == revision
        and run["repository"]["full_name"] == REPO and run["head_repository"]["full_name"] == REPO
        and run["event"] in ("workflow_dispatch", "push")
    )]
    if not matching:
        return None
    run = max(matching, key=lambda item: (item["run_number"], item["run_attempt"]))
    if run["status"] != "completed" or run["conclusion"] != "success":
        return None
    name = f"qualified-core-amd64-{revision}-{run['id']}-{run['run_attempt']}"
    artifacts = api(f"actions/runs/{run['id']}/artifacts?name={name}")
    found = [a for a in artifacts["artifacts"] if a["name"] == name and not a["expired"]]
    if not found:
        return None
    if len(found) != 1 or not re.fullmatch(r"sha256:[0-9a-f]{64}", found[0]["digest"]):
        raise ValueError("ambiguous Core artifact or missing trusted archive digest")
    return found[0]


def unpack(path, expected_digest, output):
    with path.open("rb") as archive:
        if "sha256:" + hashlib.file_digest(archive, "sha256").hexdigest() != expected_digest:
            raise ValueError("GitHub artifact digest mismatch")
    with zipfile.ZipFile(path) as bundle:
        if len(bundle.infolist()) != len(MEMBERS) or set(bundle.namelist()) != MEMBERS:
            raise ValueError("unexpected Core CI artifact contents")
        output.mkdir(parents=True)
        for name in MEMBERS:
            with bundle.open(name) as source, (output / name).open("xb") as target:
                shutil.copyfileobj(source, target)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    candidate = select(revision) if os.environ.get("GITHUB_REPOSITORY") == REPO else None
    if candidate is None:
        print("No eligible Core CI artifact; build the selected component.")
        return
    with tempfile.TemporaryDirectory() as work:
        path = Path(work) / "artifact.zip"
        with path.open("wb") as archive:
            subprocess.run(["gh", "api", f"repos/{REPO}/actions/artifacts/{candidate['id']}/zip"],
                           check=True, stdout=archive)
        unpack(path, candidate["digest"], args.directory)
    print(f"Verified GitHub artifact {candidate['id']} ({candidate['digest']}).")


if __name__ == "__main__":
    main()
