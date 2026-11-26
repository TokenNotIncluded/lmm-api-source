#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction
# Licensed under the GNU Affero General Public License v3.0; see LICENSE.
"""Exercise deployment admission against recorded GitHub API response shapes."""

import copy
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import Mock, call, patch

spec = importlib.util.spec_from_file_location(
    "deployment_ci", Path(__file__).with_name("verify-deployment-ci.py")
)
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)
REVISION = "a" * 40


class DeploymentCITests(unittest.TestCase):
    def setUp(self):
        self.workflow = {"id": 399255954, "path": ci.WORKFLOW_PATH}
        self.run = {
            "id": 44509988084,
            "workflow_id": self.workflow["id"],
            "path": ci.WORKFLOW_PATH,
            "head_sha": REVISION,
            "head_branch": "main",
            "event": "push",
            "repository": {"full_name": ci.REPOSITORY},
            "head_repository": {"full_name": ci.REPOSITORY},
            "run_number": 101,
            "run_attempt": 2,
            "status": "completed",
            "conclusion": "success",
        }
        self.gate = {
            "id": 135547567385,
            "name": ci.QUALITY_GATE,
            "run_id": self.run["id"],
            "run_attempt": self.run["run_attempt"],
            "head_sha": REVISION,
            "status": "completed",
            "conclusion": "success",
        }

    def api(self, *, runs=None, jobs=None):
        return Mock(
            side_effect=[
                self.workflow,
                [{"workflow_runs": [self.run] if runs is None else runs}],
                [{"jobs": [self.gate] if jobs is None else jobs}],
            ]
        )

    def test_success_is_bound_to_the_exact_run_attempt_and_revision(self):
        api = self.api()
        result = ci.verify(REVISION, api)
        self.assertEqual(result["revision"], REVISION)
        self.assertEqual(result["run_id"], self.run["id"])
        self.assertEqual(result["run_attempt"], 2)
        self.assertEqual(result["quality_gate_job_id"], self.gate["id"])
        self.assertEqual(
            api.call_args_list,
            [
                call("actions/workflows/ci.yml"),
                call(
                    f"actions/workflows/399255954/runs?head_sha={REVISION}&branch=main&per_page=100",
                    paginate=True,
                ),
                call("actions/runs/44509988084/attempts/2/jobs?per_page=100", paginate=True),
            ],
        )

    def test_web_coverage_cannot_authorize_backend_deployment(self):
        web_gate = {**self.gate, "steps": [
            {"name": "Require Web-only checks", "conclusion": "success"},
            {"name": "Require every implemented component", "conclusion": "skipped"},
        ]}
        self.assertEqual(
            ci.verify(REVISION, self.api(jobs=[web_gate]), web_only=True)["run_id"],
            self.run["id"],
        )
        with self.assertRaisesRegex(ValueError, "component coverage"):
            ci.verify(REVISION, self.api(jobs=[web_gate]))
        with self.assertRaisesRegex(ValueError, "component coverage"):
            ci.verify(REVISION, self.api(), web_only=True)

    def test_pending_failed_cancelled_or_skipped_ci_cannot_deploy(self):
        for status, conclusion in [
            ("queued", None),
            ("in_progress", None),
            ("completed", "failure"),
            ("completed", "cancelled"),
            ("completed", "skipped"),
        ]:
            with self.subTest(status=status, conclusion=conclusion):
                run = {**self.run, "status": status, "conclusion": conclusion}
                api = self.api(runs=[run])
                with self.assertRaisesRegex(ValueError, "latest CI run"):
                    ci.verify(REVISION, api)
                self.assertEqual(api.call_count, 2)

    def test_wrong_revision_repository_workflow_or_event_cannot_qualify(self):
        changes = [
            {"head_sha": "b" * 40},
            {"workflow_id": 399255955},
            {"path": ".github/workflows/source-release.yml"},
            {"head_branch": "other"},
            {"event": "pull_request"},
            {"repository": {"full_name": "example/fork"}},
            {"head_repository": {"full_name": "example/fork"}},
        ]
        for change in changes:
            with self.subTest(change=change):
                with self.assertRaisesRegex(ValueError, "no canonical main CI run"):
                    ci.verify(REVISION, self.api(runs=[{**self.run, **change}]))

    def test_workflow_lookup_must_identify_ci_yml(self):
        self.workflow["path"] = ".github/workflows/source-release.yml"
        with self.assertRaisesRegex(ValueError, "workflow identity"):
            ci.verify(REVISION, self.api())

    def test_old_success_cannot_hide_a_new_failed_run_or_pending_attempt(self):
        for newer in [
            {**self.run, "id": self.run["id"] + 1, "run_number": 102, "conclusion": "failure"},
            {**self.run, "run_attempt": 3, "status": "in_progress", "conclusion": None},
        ]:
            with self.subTest(newer=newer):
                with self.assertRaisesRegex(ValueError, "latest CI run"):
                    ci.verify(REVISION, self.api(runs=[self.run, newer]))

    def test_missing_duplicate_failed_or_skipped_component_gate_blocks(self):
        for jobs in [
            [],
            [self.gate, self.gate],
            [{**self.gate, "status": "in_progress", "conclusion": None}],
            [{**self.gate, "conclusion": "failure"}],
            [{**self.gate, "conclusion": "skipped"}],
        ]:
            with self.subTest(jobs=jobs):
                with self.assertRaises(ValueError):
                    ci.verify(REVISION, self.api(jobs=jobs))

    def test_component_gate_from_another_sha_run_or_attempt_blocks(self):
        for change in [
            {"head_sha": "b" * 40},
            {"run_id": self.run["id"] - 1},
            {"run_attempt": 1},
        ]:
            with self.subTest(change=change):
                with self.assertRaisesRegex(ValueError, "selected CI attempt"):
                    ci.verify(REVISION, self.api(jobs=[{**self.gate, **change}]))

    def test_paginated_manual_main_ci_and_gate_are_accepted(self):
        unrelated = copy.deepcopy(self.run)
        unrelated.update(head_sha="b" * 40, run_number=999)
        manual = {**self.run, "event": "workflow_dispatch"}
        api = Mock(
            side_effect=[
                self.workflow,
                [{"workflow_runs": [unrelated]}, {"workflow_runs": [manual]}],
                [{"jobs": []}, {"jobs": [self.gate]}],
            ]
        )
        self.assertEqual(ci.verify(REVISION, api)["run_id"], manual["id"])

    def test_missing_run_or_short_revision_cannot_qualify(self):
        with self.assertRaisesRegex(ValueError, "no canonical main CI run"):
            ci.verify(REVISION, self.api(runs=[]))
        api = Mock()
        with self.assertRaisesRegex(ValueError, "complete commit SHA"):
            ci.verify("abc123", api)
        api.assert_not_called()

    @patch.object(ci.subprocess, "check_output", return_value="[]")
    def test_api_only_reads_the_canonical_github_repository(self, command):
        self.assertEqual(ci.github_api("actions/runs/123/jobs", paginate=True), [])
        command.assert_called_once_with(
            [
                "gh", "api", "--hostname", "github.com", "--method", "GET",
                "--paginate", "--slurp",
                f"repos/{ci.REPOSITORY}/actions/runs/123/jobs",
            ],
            text=True,
            timeout=30,
        )


if __name__ == "__main__":
    unittest.main()
