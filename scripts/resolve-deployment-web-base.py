#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
"""Resolve the backend baseline published by the configured deployment workflow."""

import argparse
from hashlib import sha256
from io import BytesIO
import json
import os
import re
import subprocess
import sys
from zipfile import BadZipFile, ZipFile


def artifact_name(environment, targets):
    config = json.dumps([environment, json.loads(targets)], sort_keys=True, separators=(",", ":"))
    return "lmm-backend-baseline-v1-" + sha256(config.encode()).hexdigest()


def github(endpoint):
    return subprocess.run(
        ["gh", "api", endpoint], check=True, capture_output=True, timeout=10
    ).stdout


def resolve(repo, name, initial, read=github):
    result = json.loads(read(f"repos/{repo}/actions/artifacts?name={name}&per_page=10&direction=desc"))
    for artifact in result["artifacts"]:
        if artifact["expired"] or artifact["name"] != name:
            continue
        run_id = int(artifact["workflow_run"]["id"])
        run = json.loads(read(f"repos/{repo}/actions/runs/{run_id}"))
        if run["path"] != ".github/workflows/deploy.yml" or run["repository"]["full_name"] != repo:
            continue
        if run["conclusion"] != "success" or not 0 < artifact["size_in_bytes"] < 16_384:
            return ""
        artifact_id = int(artifact["id"])
        with ZipFile(BytesIO(read(f"repos/{repo}/actions/artifacts/{artifact_id}/zip"))) as archive:
            member = archive.getinfo("baseline.json")
            if not 0 < member.file_size < 4096:
                return ""
            receipt = json.loads(archive.read(member))
        if receipt["run_id"] != run_id or receipt["attempt"] != run["run_attempt"]:
            return ""
        # A successful partial backend deployment invalidates the common baseline.
        value = receipt["revision"]
        return value if isinstance(value, str) and re.fullmatch("[0-9a-f]{40}", value) else ""
    return initial if not result["artifacts"] and re.fullmatch("[0-9a-f]{40}", initial) else ""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-name", action="store_true")
    args = parser.parse_args()
    try:
        name = artifact_name(os.environ["DEPLOY_ENVIRONMENT_NAME"], os.environ["DEPLOY_TARGETS"])
        if args.artifact_name:
            print(name)
        else:
            print(resolve(os.environ["GITHUB_REPOSITORY"], name, os.environ.get("WEB_BASE", "")))
    except (BadZipFile, KeyError, OSError, TypeError, ValueError, subprocess.SubprocessError):
        if args.artifact_name:
            return 1
        print("No verified backend baseline could be read; use full qualification.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
