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

    def git_candidate(self):
        self.manifest.write_text(
            '[dependencies]\n'
            'zconv = { version = "=0.4.0", git = "https://github.com/LIghtJUNction/zconv.git" }\n'
        )
        self.lock.write_text(
            'version = 4\n[[package]]\nname = "zconv"\nversion = "0.4.0"\n'
            'source = "git+https://github.com/LIghtJUNction/zconv.git#' + "a" * 40 + '"\n'
        )

    def test_exact_canonical_git_candidate_is_accepted(self):
        self.git_candidate()
        self.assertEqual(release.validate(self.root), {"version": "0.1.20", "zconv": "0.4.0"})

    def test_git_manifest_requires_exact_canonical_url(self):
        for url in (
            "https://github.com/other/zconv.git",
            "http://github.com/LIghtJUNction/zconv.git",
            "https://github.com/LIghtJUNction/zconv.git/",
            "https://github.com/LIghtJUNction/zconv",
        ):
            with self.subTest(url=url):
                self.git_candidate()
                self.manifest.write_text(
                    '[dependencies]\nzconv = { version = "=0.4.0", git = ' + json.dumps(url) + ' }\n'
                )
                with self.assertRaises(ValueError):
                    release.validate(self.root)

    def test_git_manifest_rejects_extra_keys(self):
        for extra in (
            'branch = "main"', 'tag = "v0.4.0"', 'rev = "' + "a" * 40 + '"',
            'path = "../local"', 'package = "zconv"', 'features = []',
            'default-features = false',
        ):
            with self.subTest(extra=extra):
                self.git_candidate()
                self.manifest.write_text(
                    self.manifest.read_text().replace(" }\n", ", " + extra + " }\n")
                )
                with self.assertRaises(ValueError):
                    release.validate(self.root)

    def test_git_lock_requires_exact_canonical_source(self):
        for source in (
            "registry+https://github.com/rust-lang/crates.io-index",
            "git+https://github.com/other/zconv.git#" + "a" * 40,
            "git+http://github.com/LIghtJUNction/zconv.git#" + "a" * 40,
            "git+https://github.com/LIghtJUNction/zconv.git?branch=main#" + "a" * 40,
            "git+https://github.com/LIghtJUNction/zconv.git?tag=v0.4.0#" + "a" * 40,
            "git+https://github.com/LIghtJUNction/zconv.git?rev=" + "a" * 40 + "#" + "a" * 40,
            0,
        ):
            with self.subTest(source=source):
                self.git_candidate()
                self.lock.write_text(
                    'version = 4\n[[package]]\nname = "zconv"\nversion = "0.4.0"\n'
                    'source = ' + json.dumps(source) + '\n'
                )
                with self.assertRaises(ValueError):
                    release.validate(self.root)

    def test_git_lock_requires_full_lowercase_commit(self):
        for revision in ("", "a" * 7, "a" * 39, "a" * 41, "A" * 40, "g" * 40, "a" * 40 + "/"):
            with self.subTest(revision=revision):
                self.git_candidate()
                self.lock.write_text(self.lock.read_text().replace("a" * 40, revision))
                with self.assertRaises(ValueError):
                    release.validate(self.root)

    def test_git_manifest_and_lock_versions_must_match_request(self):
        for path in (self.manifest, self.lock):
            with self.subTest(path=path.name):
                self.git_candidate()
                path.write_text(path.read_text().replace("0.4.0", "0.4.1"))
                with self.assertRaises(ValueError):
                    release.validate(self.root)

    def test_git_lock_requires_one_zconv_package(self):
        self.git_candidate()
        self.lock.write_text(self.lock.read_text() + self.lock.read_text().split("\n", 1)[1])
        with self.assertRaises(ValueError):
            release.validate(self.root)

    def test_git_lock_rejects_any_checksum_field(self):
        for checksum in ("a" * 64, "", 0, False):
            with self.subTest(checksum=checksum):
                self.git_candidate()
                self.lock.write_text(self.lock.read_text() + "checksum = " + json.dumps(checksum) + "\n")
                with self.assertRaises(ValueError):
                    release.validate(self.root)

    def test_git_dependency_rejects_source_overrides(self):
        for override in (
            '\n[patch.crates-io]\nzconv = { path = "../local" }\n',
            '\n[replace]\n"zconv:0.4.0" = { path = "../local" }\n',
        ):
            with self.subTest(override=override):
                self.git_candidate()
                self.manifest.write_text(self.manifest.read_text() + override)
                with self.assertRaises(ValueError):
                    release.validate(self.root)


if __name__ == "__main__":
    unittest.main()
