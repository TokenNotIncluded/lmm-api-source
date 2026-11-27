#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
"""Exercise deployment baseline decisions with isolated GitHub API responses."""

import importlib.util
from io import BytesIO
import json
from pathlib import Path
import unittest
from zipfile import ZipFile


script = Path(__file__).with_name("resolve-deployment-web-base.py")
spec = importlib.util.spec_from_file_location("deployment_web_base", script)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

REPOSITORY = "example/lmm-api-source"
INITIAL = "a" * 40
REVISION = "b" * 40
NAME = module.artifact_name("production", '[{"name":"origin"}]')
LIST = f"repos/{REPOSITORY}/actions/artifacts?name={NAME}&per_page=10&direction=desc"
RUN = f"repos/{REPOSITORY}/actions/runs/7"
DOWNLOAD = f"repos/{REPOSITORY}/actions/artifacts/8/zip"


def receipt(revision=REVISION, attempt=2):
    stream = BytesIO()
    with ZipFile(stream, "w") as archive:
        archive.writestr(
            "baseline.json", json.dumps({"run_id": 7, "attempt": attempt, "revision": revision})
        )
    return stream.getvalue()


class DeploymentBaselineTests(unittest.TestCase):
    def setUp(self):
        self.artifact = {"workflow_run": {"id": 7}, "id": 8, "name": NAME,
                         "expired": False, "size_in_bytes": 400}
        self.artifacts = [self.artifact]
        self.run = {"path": ".github/workflows/deploy.yml", "repository": {"full_name": REPOSITORY},
                    "conclusion": "success", "run_attempt": 2}
        self.receipt = receipt()
        self.calls = []

    def read(self, endpoint):
        self.calls.append(endpoint)
        return {
            LIST: json.dumps({"artifacts": self.artifacts}).encode(),
            RUN: json.dumps(self.run).encode(),
            DOWNLOAD: self.receipt,
        }[endpoint]

    def resolve(self, initial=INITIAL):
        return module.resolve(REPOSITORY, NAME, initial, self.read)

    def test_bootstrap_requires_a_full_sha_and_no_prior_receipt(self):
        self.artifacts.clear()
        self.assertEqual(self.resolve(), INITIAL)
        self.assertEqual(self.resolve("untrusted\nother=value"), "")
        self.assertEqual(self.calls, [LIST, LIST])

    def test_successful_full_deployment_supersedes_the_bootstrap(self):
        self.assertEqual(self.resolve(), REVISION)
        self.assertEqual(self.calls, [LIST, RUN, DOWNLOAD])

    def test_partial_backend_deployment_invalidates_the_baseline(self):
        self.receipt = receipt(revision="")
        self.assertEqual(self.resolve(), "")

    def test_failed_workflow_or_wrong_origin_does_not_download_evidence(self):
        for field, value in [("conclusion", "failure"), ("path", ".github/workflows/ci.yml"),
                             ("repository", {"full_name": "fork/lmm-api-source"})]:
            with self.subTest(field=field):
                original = self.run[field]
                self.run[field] = value
                self.assertEqual(self.resolve(), "")
                self.run[field] = original
        self.assertNotIn(DOWNLOAD, self.calls)

    def test_expired_and_prior_attempt_evidence_do_not_reuse_bootstrap(self):
        self.artifact["expired"] = True
        self.assertEqual(self.resolve(), "")
        self.assertEqual(self.calls, [LIST])
        self.artifact["expired"] = False
        self.receipt = receipt(attempt=1)
        self.assertEqual(self.resolve(), "")

    def test_name_canonicalizes_json_but_binds_the_targets_and_environment(self):
        self.assertEqual(module.artifact_name("production", '[ { "name": "origin" } ]'), NAME)
        self.assertNotEqual(module.artifact_name("shadow", '[{"name":"origin"}]'), NAME)
        self.assertNotEqual(module.artifact_name("production", '[{"name":"other"}]'), NAME)


if __name__ == "__main__":
    unittest.main()
