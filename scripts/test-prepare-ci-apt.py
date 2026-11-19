# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import importlib.util
from pathlib import Path
import tempfile
import unittest
import sys
sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location("prepare_ci_apt", Path(__file__).with_name("prepare-ci-apt.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PreparationTests(unittest.TestCase):
    def test_only_unused_chrome_sources_are_disabled(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ubuntu = "Types: deb\nURIs: https://archive.ubuntu.com/ubuntu\nSuites: noble\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"
            pg = "deb [signed-by=/keys/postgresql.gpg] https://apt.postgresql.org/pub/repos/apt noble-pgdg main\n"
            (root / "ubuntu.sources").write_text(ubuntu)
            (root / "postgres.list").write_text(pg)
            (root / "chrome.list").write_text("deb [arch=amd64] https://dl.google.com/linux/chrome-stable/deb stable main\n")
            (root / "chrome.sources").write_text("Types: deb\nURIs: https://dl.google.com/linux/chrome/deb\nSuites: stable\nEnabled: yes\n")
            self.assertEqual(module.prepare(root), ["chrome.list", "chrome.sources"])
            self.assertEqual((root / "ubuntu.sources").read_text(), ubuntu)
            self.assertEqual((root / "postgres.list").read_text(), pg)
            self.assertTrue((root / "chrome.list").read_text().startswith("#"))
            self.assertIn("Enabled: no", (root / "chrome.sources").read_text())
            self.assertEqual(module.prepare(root), [])

    def test_mixed_sources_fail_without_changing_the_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "mixed.sources"
            content = "Types: deb\nURIs: https://dl.google.com/linux/chrome/deb https://archive.ubuntu.com/ubuntu\n"
            path.write_text(content)
            with self.assertRaises(ValueError): module.prepare(path.parent)
            self.assertEqual(path.read_text(), content)

    def test_runner_ubuntu_deb822_keeps_suites_components_and_signature(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ubuntu.sources"
            content = (
                "# Ubuntu archive and security sources\n"
                "Types: deb\n"
                "URIs: mirror+file:/etc/apt/apt-mirrors.txt\n"
                "Suites: noble noble-updates noble-backports\n"
                "Components: main restricted universe multiverse\n"
                "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"
                "\n"
                "Types: deb\n"
                "URIs: mirror+file:/etc/apt/apt-mirrors.txt\n"
                "Suites: noble-security\n"
                "Components: main restricted universe multiverse\n"
                "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"
            )
            path.write_text(content)
            self.assertEqual(module.prepare(path.parent), ["ubuntu.sources"])
            self.assertEqual(path.read_text(), content.replace(
                "mirror+file:/etc/apt/apt-mirrors.txt", "https://archive.ubuntu.com/ubuntu/"
            ))
            self.assertEqual(module.prepare(path.parent), [])

    def test_only_exact_runner_uri_changes_in_list_entries(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ubuntu.list"
            original_uri = "mirror+file:/etc/apt/apt-mirrors.txt"
            entry = "deb [arch=amd64 signed-by=/keys/ubuntu.gpg] " + original_uri + " noble main\n"
            unchanged = (
                "# deb " + original_uri + " noble main\n"
                "deb mirror+file:/etc/apt/other-mirrors.txt noble main\n"
                "deb " + original_uri + ".backup noble main\n"
                "deb https://apt.postgresql.org/pub/repos/apt noble-pgdg main\n"
            )
            path.write_text(entry + unchanged)
            self.assertEqual(module.prepare(path.parent), ["ubuntu.list"])
            self.assertEqual(path.read_text(), entry.replace(
                original_uri, "https://archive.ubuntu.com/ubuntu/"
            ) + unchanged)
            self.assertEqual(module.prepare(path.parent), [])

    def test_deb822_continuation_preserves_other_fields_and_unknown_uris(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ubuntu.sources"
            prefix = (
                "Types: deb\n"
                "URIs: mirror+file:/etc/apt/other-mirrors.txt\n"
                " mirror+file:/etc/apt/apt-mirrors.txt.backup\n"
            )
            suffix = (
                "Suites: noble\n"
                "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"
                "Enabled: no\n"
                "# mirror+file:/etc/apt/apt-mirrors.txt\n"
                "X-Comment: mirror+file:/etc/apt/apt-mirrors.txt\n"
            )
            path.write_text(prefix + " mirror+file:/etc/apt/apt-mirrors.txt\n" + suffix)
            self.assertEqual(module.prepare(path.parent), ["ubuntu.sources"])
            self.assertEqual(path.read_text(), prefix + " https://archive.ubuntu.com/ubuntu/\n" + suffix)
            self.assertEqual(module.prepare(path.parent), [])


if __name__ == "__main__": unittest.main()
