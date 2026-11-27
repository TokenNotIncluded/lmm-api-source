#!/usr/bin/env python3
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "static_retention", Path(__file__).with_name("retain-web-static.py")
)
retention = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retention)


class RetainStatic(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.fallback = self.root / "static-retained"
        self.static = self.fallback / "static"
        (self.static / "js").mkdir(parents=True)

    def release(self, revision, name, body):
        slot = self.root / revision[0]
        source = slot / "dist/static" / name
        source.parent.mkdir(parents=True)
        source.write_bytes(body)
        manifest = {"component": "web", "revision": revision,
                    "files": {"dist/static/" + name: {"sha256": hashlib.sha256(body).hexdigest()}}}
        (slot / "MANIFEST.json").write_text(json.dumps(manifest))
        return slot, source

    def test_keeps_one_managed_generation_and_preserves_existing_fallback(self):
        legacy = self.static / "js/legacy.abcdef12.js"
        legacy.write_bytes(b"pre-existing fallback")
        first, original = self.release("a" * 40, "js/chunk.abcdef12.js", b"first")
        old = self.static / "js/chunk.abcdef12.js"
        self.assertEqual(retention.retain(first, self.fallback, check_only=True)["would_add"], 1)
        self.assertFalse(old.exists())
        self.assertEqual(retention.retain(first, self.fallback)["added"], 1)
        self.assertEqual(original.stat().st_ino, old.stat().st_ino)
        original.unlink()
        self.assertEqual(old.read_bytes(), b"first")
        second, _ = self.release("b" * 40, "js/chunk.fedcba91.js.LICENSE.txt", b"second")
        self.assertEqual(retention.retain(second, self.fallback)["removed"], 1)
        self.assertFalse(old.exists())
        self.assertEqual(legacy.read_bytes(), b"pre-existing fallback")
        self.assertEqual(retention.retain(second, self.fallback)["added"], 0)

    def test_refuses_conflicting_content_before_modifying_fallback(self):
        name = "js/chunk.abcdef12.js"
        slot, _ = self.release("a" * 40, name, b"source")
        (self.static / name).write_bytes(b"conflict")
        with self.assertRaisesRegex(ValueError, "collision"):
            retention.retain(slot, self.fallback)
        self.assertEqual((self.static / name).read_bytes(), b"conflict")
        self.assertFalse((self.fallback / "ci-web-retained.json").exists())

    def test_replays_recorded_ownership_after_an_interrupted_link(self):
        name = "js/chunk.abcdef12.js"
        slot, _ = self.release("a" * 40, name, b"source")
        retention.write_receipt(self.fallback / "ci-web-retained.json",
                                {name: hashlib.sha256(b"source").hexdigest()})
        result = retention.retain(slot, self.fallback)
        self.assertEqual((result["added"], result["managed"]), (1, 1))

    def test_rejects_a_fallback_subdirectory_that_redirects_links(self):
        slot, _ = self.release("a" * 40, "js/chunk.abcdef12.js", b"source")
        elsewhere = self.root / "outside"
        elsewhere.mkdir()
        (self.static / "js").rmdir()
        (self.static / "js").symlink_to(elsewhere, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symbolic link"):
            retention.retain(slot, self.fallback)
        self.assertEqual(list(elsewhere.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
