"""Offline checks for the real runner's Rust test discovery command.

Only discovery and validation execute; no database or payment process is created.
"""
import ast
import os
import re
from pathlib import Path
import subprocess
from types import SimpleNamespace
import unittest
from unittest import mock

SOURCE = Path(os.environ.get(
    "LMM_TEST_PAYMENT_RUNNER", Path(__file__).with_name("verify_live_process.py")
))
TREE = ast.parse(SOURCE.read_text())
MAIN = next(node for node in TREE.body if isinstance(node, ast.FunctionDef) and node.name == "main")
CASE = next(
    ast.literal_eval(node.value) for node in TREE.body
    if isinstance(node, ast.Assign) and any(
        isinstance(target, ast.Name) and target.id == "CASE" for target in node.targets
    )
)
TRY = next(node for node in MAIN.body if isinstance(node, ast.Try))
INDEX = next(
    index for index, node in enumerate(TRY.body)
    if isinstance(node, ast.Assign) and any(
        isinstance(target, ast.Name) and target.id == "listed" for target in node.targets
    )
)
DISCOVERY = compile(ast.Module(body=TRY.body[INDEX:INDEX + 2], type_ignores=[]), str(SOURCE), "exec")


class FailureDiagnosticsTests(unittest.TestCase):
    def location(self, text):
        helper = next(node for node in TREE.body if isinstance(node, ast.FunctionDef) and node.name == "failure_location")
        namespace = {"re": re}
        exec(compile(ast.Module(body=[helper], type_ignores=[]), str(SOURCE), "exec"), namespace)
        return namespace["failure_location"](text)

    def test_only_static_source_location_survives_private_panic_details(self):
        raw = "thread 'payment' (1234) panicked at tests/payments_live_process.rs:412:7:\n" \
              "postgresql://user:synthetic-private-password@127.0.0.1/db\nBearer synthetic-token"
        self.assertEqual(self.location(raw), {"file": "tests/payments_live_process.rs", "line": 412, "column": 7})

    def test_no_location_or_unknown_file_does_not_echo_private_text(self):
        for raw in ("postgresql://user:secret@localhost/db", "panicked at /private/credential:1:2:", "panicked at tests/payments_live_process.rs:0:2:"):
            with self.subTest(raw=raw):
                self.assertIsNone(self.location(raw))


class DiscoveryTests(unittest.TestCase):
    def run_discovery(self, run):
        environment = {"PATH": "/synthetic/bin"}
        exec(DISCOVERY, {
            "binaries": {"test": {"path": "/sealed/payments_live_process"}},
            "env": environment,
            "CASE": CASE,
            "subprocess": SimpleNamespace(run=run),
        })
        return environment

    @staticmethod
    def discovery_output(names):
        lines = "\n".join(name + ": test" for name in names)
        noun = "test" if len(names) == 1 else "tests"
        return SimpleNamespace(stdout=f"{lines}\n\n{len(names)} {noun}, 0 benchmarks\n")

    def test_shared_helper_does_not_hide_the_exact_payment_case(self):
        names = ["hex_codec::tests::preserves_hex_representation_and_input_errors", CASE]
        def libtest(command, **_kwargs):
            selected = [name for name in names if name == command[-1]] if "--exact" in command else names
            return self.discovery_output(selected)
        run = mock.Mock(side_effect=libtest)
        environment = self.run_discovery(run)
        run.assert_called_once_with(
            ["/sealed/payments_live_process", "--list", "--exact", CASE],
            env=environment, capture_output=True, text=True, timeout=10, check=True,
        )

    def test_missing_payment_case_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "unexpected Rust test discovery"):
            self.run_discovery(mock.Mock(return_value=self.discovery_output([])))

    def test_similarly_named_case_is_not_accepted(self):
        with self.assertRaisesRegex(RuntimeError, "unexpected Rust test discovery"):
            self.run_discovery(mock.Mock(return_value=self.discovery_output([CASE + "_different"])))

    def test_unfiltered_extra_test_output_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "unexpected Rust test discovery"):
            self.run_discovery(mock.Mock(return_value=self.discovery_output([CASE, "helper"])))

    def test_discovery_command_failure_is_not_silenced(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_discovery(mock.Mock(side_effect=subprocess.CalledProcessError(1, "libtest")))


if __name__ == "__main__":
    unittest.main()
