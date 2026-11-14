"""Pure resource-runner gates; never start a product process or fixture."""
import contextlib
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))
import verify_model_resources as runner


class ResourceSelectionTests(unittest.TestCase):
    def setUp(self):
        self.discovered = sorted(runner.v.PROCESS_TESTS | runner.v.PROCESS_SUPPORT_TESTS | set(runner.CASES.values()))

    def summary(self, passed=1, failed=0, ignored=0, measured=0, filtered=None):
        if filtered is None:
            filtered = len(self.discovered) - 1
        return (f"test result: ok. {passed} passed; {failed} failed; "
                f"{ignored} ignored; {measured} measured; {filtered} filtered out; finished in 1s\n")

    def test_freeze_binds_negative_test_source(self):
        self.assertEqual(runner.inputs()["resource_runner_tests_sha256"],
                         runner.v.sha(Path(__file__).resolve()))

    def test_embedded_inputs_invalidate_their_binding(self):
        cases = (
            ('services/extensions/internal/modules/toolmarket/builtin_catalog.json', 'product'),
            ('deployment/postgres/grant-tool-market-credit.sql', 'tests'),
            ('contracts/core/v1/funding-cases.json', 'tests'),
            ('contracts/assistant-tools-v1.json', 'tests'),
            ('deployment/postgres/verify-channel-observations-owner.sql', 'tests'),
        )
        for embedded_path, scope in cases:
            with self.subTest(path=embedded_path), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                for name in (*runner.v.PRODUCT_TREES, *runner.v.TEST_TREES):
                    (root / name).mkdir(parents=True, exist_ok=True)
                for name in (*runner.v.PRODUCT_FILES, *runner.v.TEST_FILES):
                    path = root / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text('fixture\n')
                embedded = root / embedded_path
                embedded.parent.mkdir(parents=True, exist_ok=True)
                embedded.write_text('original\n')
                with patch.object(runner.v, 'REPO', root):
                    before = runner.v.inputs()
                    self.assertIn(embedded_path, before[scope])
                    self.assertEqual(runner.v.is_product(embedded_path), scope == 'product')
                    embedded.write_text('changed\n')
                    after = runner.v.inputs()
                    self.assertEqual(runner.v.delta(before[scope], after[scope]), {
                        'modified': [embedded_path], 'missing': [], 'added': [],
                    })
                    other = 'tests' if scope == 'product' else 'product'
                    self.assertEqual(before[other], after[other])
                    embedded.unlink()
                    with self.assertRaises(FileNotFoundError):
                        runner.v.inputs()

    def test_reviewed_discovery_accepts_exact_current_list(self):
        runner.reviewed_tests(self.discovered)
        self.assertEqual(len(self.discovered), 12)

    def test_shared_hex_support_case_is_explicit_and_required(self):
        support = "hex_codec::tests::preserves_hex_representation_and_input_errors"
        self.assertEqual(runner.v.PROCESS_SUPPORT_TESTS, {support})
        self.assertEqual(len(runner.v.PROCESS_TESTS), 7)
        self.assertNotIn(support, runner.v.PROCESS_TESTS)
        for names in (
            [name for name in self.discovered if name != support],
            [name if name != support else support + "_renamed" for name in self.discovered],
        ):
            with self.subTest(names=names), self.assertRaises(runner.FixtureError):
                runner.reviewed_tests(names)

    def test_missing_extra_old_duplicate_and_malformed_discovery_rejected(self):
        variants = [self.discovered[:-1], self.discovered + ["foreign_case"],
                    sorted(runner.v.PROCESS_TESTS | {runner.CASE}),
                    self.discovered + [self.discovered[0]], None, {},
                    self.discovered[:-1] + [None]]
        for names in variants:
            with self.subTest(names=names), self.assertRaises(runner.FixtureError):
                runner.reviewed_tests(names)

    def test_unknown_case_rejected_by_command_and_cli(self):
        with self.assertRaises(runner.FixtureError):
            runner.selected_command("sealed-test", "unknown")
        argv = ["verify_model_resources", "run", "--output", "/nonexistent",
                "--process-root", "/nonexistent", "--prepared-sha256", "0" * 64,
                "--case", "unknown"]
        with patch.object(sys, "argv", argv), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as caught:
                runner.main()
        self.assertEqual(caught.exception.code, 2)

    def test_socket_command_selects_only_socket_case(self):
        command = runner.selected_command("sealed-test", "socket-backpressure")
        self.assertEqual(command, ["sealed-test", runner.CASES["socket-backpressure"],
                         "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        self.assertNotIn(runner.CASE, command)
        for selection in ("socket-pressure", "socket-h2"):
            selected = runner.selected_command("sealed-test", selection)
            self.assertEqual(selected[1], runner.CASES[selection])
            self.assertEqual(selected[2:], command[2:])
            self.assertNotIn(runner.CASE, selected)

    def test_sustained_command_preserves_existing_case(self):
        self.assertEqual(runner.selected_command("sealed-test", "sustained")[1], runner.CASE)

    def test_exact_one_selected_pass_accepted(self):
        runner.require_selected_success(self.summary(), self.discovered)

    def test_zero_ignored_failure_extra_pass_or_wrong_filter_rejected(self):
        variants = [self.summary(passed=0), self.summary(passed=0, ignored=1),
                    self.summary(ignored=1), self.summary(failed=1),
                    self.summary(passed=2), self.summary(measured=1),
                    self.summary(filtered=7), self.summary(filtered=10), self.summary(filtered=12), "",
                    self.summary() + self.summary(), self.summary().replace("ok.", "FAILED.")]
        for text in variants:
            with self.subTest(text=text), self.assertRaises(runner.FixtureError):
                runner.require_selected_success(text, self.discovered)


if __name__ == "__main__":
    unittest.main()
