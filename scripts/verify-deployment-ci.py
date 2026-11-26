#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction
# Licensed under the GNU Affero General Public License v3.0; see LICENSE.
"""Require successful canonical CI for the exact configured deployment revision."""

import argparse
import json
import os
import re
import subprocess
import sys

REPOSITORY = "TokenNotIncluded/lmm-api-source"
WORKFLOW_PATH = ".github/workflows/ci.yml"
QUALITY_GATE = "Implemented component checks"


def github_api(path, *, paginate=False):
    command = ["gh", "api", "--hostname", "github.com", "--method", "GET"]
    if paginate:
        command.extend(["--paginate", "--slurp"])
    command.append(f"repos/{REPOSITORY}/{path}")
    return json.loads(subprocess.check_output(command, text=True, timeout=30))


def verify(revision, api=github_api, *, web_only=False):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("deployment revision must be a complete commit SHA")
    workflow = api("actions/workflows/ci.yml")
    if workflow["path"] != WORKFLOW_PATH or type(workflow["id"]) is not int or workflow["id"] <= 0:
        raise ValueError("canonical CI workflow identity differs")

    pages = api(
        f"actions/workflows/{workflow['id']}/runs?head_sha={revision}&branch=main&per_page=100",
        paginate=True,
    )
    runs = [
        run
        for page in pages
        for run in page["workflow_runs"]
        if run["workflow_id"] == workflow["id"]
        and run["path"] == WORKFLOW_PATH
        and run["head_sha"] == revision
        and run["head_branch"] == "main"
        and run["event"] in {"push", "workflow_dispatch"}
        and run["repository"]["full_name"] == REPOSITORY
        and run["head_repository"]["full_name"] == REPOSITORY
    ]
    if not runs:
        raise ValueError("no canonical main CI run exists for the deployment revision")
    if any(
        type(run[field]) is not int or run[field] <= 0
        for run in runs
        for field in ("id", "run_number", "run_attempt")
    ):
        raise ValueError("CI run or attempt identity is invalid")
    run = max(runs, key=lambda item: (item["run_number"], item["run_attempt"]))
    if run["status"] != "completed" or run["conclusion"] != "success":
        raise ValueError(
            f"latest CI run {run['id']} attempt {run['run_attempt']} is "
            f"{run['status']}/{run['conclusion']}; wait for successful CI before deploying"
        )

    pages = api(
        f"actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs?per_page=100",
        paginate=True,
    )
    gates = [job for page in pages for job in page["jobs"] if job["name"] == QUALITY_GATE]
    if len(gates) != 1:
        raise ValueError("the selected CI attempt must contain one implemented-component gate")
    gate = gates[0]
    if (
        gate["run_id"] != run["id"]
        or gate["run_attempt"] != run["run_attempt"]
        or gate["head_sha"] != revision
        or gate["status"] != "completed"
        or gate["conclusion"] != "success"
    ):
        raise ValueError("implemented-component gate did not succeed in the selected CI attempt")
    fast = [step for step in gate.get("steps", []) if step["name"] == "Require Web-only checks"]
    if len(fast) > 1 or (len(fast) == 1 and fast[0]["conclusion"] == "success") != web_only:
        raise ValueError("CI component coverage differs from the requested deployment scope")
    return {
        "repository": REPOSITORY,
        "revision": revision,
        "workflow_id": workflow["id"],
        "workflow_path": WORKFLOW_PATH,
        "run_id": run["id"],
        "run_attempt": run["run_attempt"],
        "quality_gate_job_id": gate["id"],
        "workflow_run": f"https://github.com/{REPOSITORY}/actions/runs/{run['id']}",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--web-only", action="store_true")
    args = parser.parse_args()
    try:
        if os.environ.get("GITHUB_REPOSITORY") != REPOSITORY:
            raise ValueError("deployment CI must be checked in the canonical source repository")
        checked_out = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        if checked_out != args.revision:
            raise ValueError("checked-out source differs from the deployment revision")
        print(json.dumps(verify(args.revision, web_only=args.web_only), sort_keys=True))
    except (ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"deployment CI qualification failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
