#!/usr/bin/env python3
"""Release metadata cannot substitute a path dependency or a different source version."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("source_release", Path(__file__).with_name("source-release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / ".github").mkdir()
        (self.root / "services/core").mkdir(parents=True)
        self.request = self.root / ".github/source-release-request.json"
        self.request.write_text(json.dumps({"version": "0.1.20", "zconv": "0.4.0"}))
        (self.root / "VERSION").write_text("0.1.20\n")
        self.manifest = self.root / "services/core/Cargo.toml"
        self.manifest.write_text('[dependencies]\nzconv = "=0.4.0"\n')
        self.lock = self.root / "services/core/Cargo.lock"
        self.lock.write_text('version = 4\n[[package]]\nname = "zconv"\nversion = "0.4.0"\nsource = "' + release.REGISTRY + '"\nchecksum = "' + "a" * 64 + '"\n')

    def test_exact_registry_candidate_is_accepted(self):
        self.assertEqual(release.validate(self.root), {"version": "0.1.20", "zconv": "0.4.0"})

    def test_different_project_or_dependency_version_fails(self):
        (self.root / "VERSION").write_text("0.1.19\n")
        with self.assertRaises(ValueError):
            release.validate(self.root)
        (self.root / "VERSION").write_text("0.1.20\n")
        self.manifest.write_text('[dependencies]\nzconv = "=0.3.0"\n')
        with self.assertRaises(ValueError):
            release.validate(self.root)

    def test_path_or_git_dependency_does_not_qualify(self):
        original = self.lock.read_text()
        for source in ["", 'source = "git+https://example.test/zconv#1234"\n']:
            self.lock.write_text('version = 4\n[[package]]\nname = "zconv"\nversion = "0.4.0"\n' + source)
            with self.assertRaises(ValueError):
                release.validate(self.root)
        self.lock.write_text(original)
        self.manifest.write_text('[dependencies]\nzconv = { version = "=0.4.0", path = "../local" }\n')
        with self.assertRaises(ValueError):
            release.validate(self.root)

    def test_duplicate_unknown_or_shell_values_fail(self):
        for value in [
            '{"version":"0.1.20","version":"0.1.20","zconv":"0.4.0"}',
            '{"version":"0.1.20","zconv":"0.4.0","skip_tests":true}',
            '{"version":"0.1.20; echo invalid","zconv":"0.4.0"}',
        ]:
            self.request.write_text(value)
            with self.assertRaises(ValueError):
                release.validate(self.root)


if __name__ == "__main__":
    unittest.main()
