#!/usr/bin/env python3
"""Only the new transport/claim boundary. Never run SSH or either business helper."""
import ast
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import tempfile
import types
import unittest
from unittest import mock

import runner
import server


class OperationsTests(unittest.TestCase):
    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.tmp = Path(self.stack.enter_context(tempfile.TemporaryDirectory(prefix="ldc-ops-boundary-")))
        self.ops = self.tmp / "ops"
        self.ops.mkdir(mode=0o700)
        self.stage = self.tmp / "stage"
        self.stack.enter_context(mock.patch.multiple(
            server, BASE=self.tmp, OPS=self.ops, STAGE=self.stage, ROOT_UID=os.getuid()))
        self.stack.enter_context(mock.patch.object(server, "release_verified", lambda: None))
        self.commands = []
        self.stack.enter_context(mock.patch.object(server, "command", self.command))
        for name in server.HASHES:
            shutil.copyfile(Path(__file__).parent / name, self.ops / name)
            (self.ops / name).chmod(0o600)

    def command(self, argv, timeout=30):
        self.commands.append(argv)
        if argv[0] == "systemctl":
            return b"ActiveState=active\nSubState=exited\nResult=success\n"
        return b""

    def test_only_fixed_target_and_actions_are_accepted(self):
        good = {"name": "arch-shadow", "host": "server.example.test", "port": 22,
                "root": "/srv/lmm-shadow", "user": "root"}
        self.assertEqual(runner.selected_target(json.dumps([good]))["host"], good["host"])
        for changed in (dict(good, name="other"), dict(good, root="/tmp/other"),
                        dict(good, user="new-user"), dict(good, host="host; injected")):
            with self.assertRaises(runner.Refused):
                runner.selected_target(json.dumps([changed]))
        with self.assertRaisesRegex(runner.Refused, "OPERATION_INVALID"):
            runner.execute("shell")
        self.assertEqual(runner.MODES, ("verify", "stage", "prepare", "activate"))

    def test_frozen_helpers_are_copied_without_changing_bytes(self):
        files = runner.checked_files(Path(__file__).parent)
        self.assertEqual(runner.HASHES, server.HASHES)
        self.assertEqual(set(files), {*runner.HASHES, "server.py"})
        ast.parse(runner.INSTALLER)
        self.assertIn("marker.read_text().strip()=='lmm-deployment-v1'", runner.INSTALLER)

    def test_claim_is_durable_before_unit_launch_and_prevents_replay(self):
        def launch(argv, timeout=30):
            self.assertTrue(server.claim_path("stage").is_file())
            return self.command(argv, timeout)
        with mock.patch.object(server, "command", launch):
            result = server.start("stage")
        self.assertEqual(result["status"], "claimed")
        args = self.commands[-1]
        self.assertEqual(args[0], "systemd-run")
        self.assertIn("--unit=lmm-ldc-20261118-stage.service", args)
        self.assertIn("--no-block", args)
        self.assertIn("--property=RemainAfterExit=yes", args)
        self.assertEqual(args[-2:], ["worker", "stage"])
        with self.assertRaisesRegex(server.Refused, "ALREADY_CLAIMED_NEVER_REPLAY"):
            server.start("stage")
        self.assertEqual(len(self.commands), 1)

    def test_uncertain_launch_keeps_claim_and_blocks_later_phases(self):
        with mock.patch.object(server, "command", side_effect=server.Refused("SERVER_COMMAND_FAILED")):
            with self.assertRaises(server.Refused):
                server.start("stage")
        self.assertTrue(server.claim_path("stage").exists())
        self.assertFalse(server.receipt_path("stage").exists())
        with self.assertRaisesRegex(server.Refused, "ALREADY_CLAIMED_NEVER_REPLAY"):
            server.start("stage")
        with self.assertRaisesRegex(server.Refused, "PRIOR_OPERATION_UNRESOLVED"):
            server.start("prepare")

    def test_worker_is_once_only_and_private_receipt_drops_extra_output(self):
        server.start("stage")
        response = {"channel_id": 5004, "core_enabled": False, "active_files_changed": False,
                    "merchant": "synthetic-secret-must-not-return",
                    "stderr": "synthetic-secret-must-not-return"}
        fake = types.SimpleNamespace(returncode=0, stdout=json.dumps(response).encode(), stderr=b"private")
        with mock.patch.object(server.subprocess, "run", return_value=fake) as invoke:
            result = server.worker("stage")
            with self.assertRaisesRegex(server.Refused, "WORKER_STATE_INVALID"):
                server.worker("stage")
        self.assertEqual(invoke.call_count, 1)
        self.assertEqual(result["status"], "completed")
        self.assertEqual(invoke.call_args.args[0][-2:], ["--stage-to", str(self.stage)])
        receipt = server.receipt_path("stage")
        self.assertNotIn(b"synthetic-secret", receipt.read_bytes())
        self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
        self.assertTrue((self.ops / "stage.started.json").exists())

    def test_unknown_activation_worker_result_requires_manual_recovery(self):
        server.durable_new(server.claim_path("activate"), {"operation": "activate"})
        fake = types.SimpleNamespace(returncode=1, stdout=b"not-json", stderr=b"private-value")
        with mock.patch.object(server.subprocess, "run", return_value=fake):
            result = server.worker("activate")
        self.assertEqual(result["status"], "failed")
        self.assertTrue(result["manual_review_required"])
        self.assertTrue(result["recovery_required"])
        self.assertNotIn(b"private-value", server.receipt_path("activate").read_bytes())

    def test_default_verify_has_no_server_writes(self):
        before = {p.name: p.read_bytes() for p in self.ops.iterdir()}
        result = server.verify()
        self.assertTrue(result["release_verified"])
        after = {p.name: p.read_bytes() for p in self.ops.iterdir()}
        self.assertEqual(before, after)
        self.assertFalse(self.stage.exists())
        self.assertEqual(self.commands, [])

    def test_unreadable_or_unresolved_observations_fail_closed(self):
        base = {"release_verified": True, "operations": {}}
        self.assertTrue(runner.observed_ok(base))
        self.assertFalse(runner.observed_ok(dict(base, stage_state_unreadable=True)))
        self.assertFalse(runner.observed_ok(dict(base, activation_recovery_required=True)))
        for entry in ({"receipt_unreadable": True}, {"unit_observation_unavailable": True},
                      {"claimed": True}, {"claimed": True, "receipt": {"status": "failed"}},
                      {"receipt": {"status": "completed", "manual_review_required": True}},
                      {"receipt": {"status": "completed", "recovery_required": True}}):
            with self.subTest(entry=entry):
                self.assertFalse(runner.observed_ok(dict(base, operations={"stage": entry})))

    def test_ssh_uses_existing_strict_hostkey_without_logging_private_output(self):
        key, hosts = self.tmp / "key", self.tmp / "known_hosts"
        for file in (key, hosts):
            file.write_text("synthetic-only")
            file.chmod(0o600)
        with mock.patch.dict(os.environ, {"LDC_SSH_KEY_FILE": str(key),
                                        "LDC_KNOWN_HOSTS_FILE": str(hosts)}):
            argv = runner.ssh_arguments({"host": "host.example.test", "port": 22, "user": "root"})
        self.assertIn("StrictHostKeyChecking=yes", argv)
        self.assertIn("IdentitiesOnly=yes", argv)
        self.assertIn("BatchMode=yes", argv)
        self.assertIn("UserKnownHostsFile=" + str(hosts), argv)
        fake = types.SimpleNamespace(returncode=255, stdout=b"", stderr=b"secret-transport-output")
        with mock.patch.object(runner.subprocess, "run", return_value=fake):
            with self.assertRaisesRegex(runner.Refused, "^SSH_RESULT_UNCERTAIN_USE_VERIFY$"):
                runner.remote(argv, ["/usr/bin/python3", "-", "verify"], b"fixed-public-code")

    def test_workflow_preserves_main_environment_and_fixed_choice_boundary(self):
        root = Path(__file__).resolve().parents[3]
        text = (root / ".github/workflows/ldc-operations-20261118.yml").read_text()
        for part in ("workflow_dispatch:", "default: verify",
                     "options: [verify, stage, prepare, activate]",
                     "if: github.ref == 'refs/heads/main'",
                     "git merge-base --is-ancestor HEAD origin/main",
                     "environment: \u0024{{ vars.DEPLOY_ENVIRONMENT || 'shadow' }}",
                     "group: deploy-\u0024{{ vars.DEPLOY_ENVIRONMENT || 'shadow' }}",
                     "cancel-in-progress: false", "contents: read",
                     "persist-credentials: false", "secrets.DEPLOY_SSH_PRIVATE_KEY",
                     "vars.DEPLOY_KNOWN_HOSTS", "if: always()"):
            self.assertIn(part, text)
        self.assertNotIn("id-token:", text)
        self.assertNotIn("contents: write", text)
        self.assertNotIn("inputs.targets", text)
        self.assertNotIn("inputs.command", text)
        self.assertNotIn("inputs.script", text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
