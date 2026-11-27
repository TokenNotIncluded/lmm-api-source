#!/usr/bin/env python3
"""Exercise CI provenance selection and byte-level rejection with synthetic artifacts."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


fetch = load("fetch-core-ci-material")
material = load("core-ci-material")
REVISION = "3" * 40


class CoreCIReuseTests(unittest.TestCase):
    def test_exact_main_attempt_with_a_unique_unexpired_digest(self):
        run = dict(path=fetch.WORKFLOW, head_branch="main", head_sha=REVISION,
                   repository={"full_name": fetch.REPO}, head_repository={"full_name": fetch.REPO},
                   event="workflow_dispatch", status="completed", conclusion="success", id=12,
                   run_number=5, run_attempt=2)
        artifact = dict(name=f"qualified-core-amd64-{REVISION}-12-2", expired=False,
                        digest="sha256:" + "a" * 64, id=42)
        def select(runs, artifacts):
            def api(endpoint):
                if endpoint.startswith("actions/workflows/"):
                    return {"workflow_runs": runs}
                self.assertIn("actions/runs/12/artifacts", endpoint)
                return {"artifacts": artifacts}
            return fetch.select(REVISION, api)
        self.assertEqual(select([run], [artifact]), artifact)
        for delta in ({"event": "pull_request"}, {"head_repository": {"full_name": "other/repo"}},
                      {"path": ".github/workflows/other.yml"}, {"head_sha": "4" * 40}):
            self.assertIsNone(select([{**run, **delta}], [artifact]))
        self.assertIsNone(select([run, {**run, "run_attempt": 3, "conclusion": "failure"}], [artifact]))
        self.assertIsNone(select([run], [{**artifact, "expired": True}]))
        with self.assertRaises(ValueError):
            select([run], [artifact, artifact])

    def test_archive_contents_and_build_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "archive.zip"
            with zipfile.ZipFile(archive, "w") as bundle:
                for name in fetch.MEMBERS:
                    content = {"schema.json": {"scope": "compiled-core-schema-contract", "components": ["fixture"]},
                               "sbom.spdx.json": {"spdxVersion": "SPDX-2.3", "packages": [{"name": "fixture"}]}}
                    bundle.writestr(name, json.dumps(content.get(name, name)))
            digest = "sha256:" + hashlib.sha256(archive.read_bytes()).hexdigest()
            out = root / "unpacked"
            with self.assertRaisesRegex(ValueError, "digest"):
                fetch.unpack(archive, "sha256:" + "0" * 64, out)
            fetch.unpack(archive, digest, out)
            frozen = material.manifest(out, REVISION)
            self.assertEqual(frozen["revision"], REVISION)
            self.assertEqual(frozen["platform"], "linux-amd64")
            self.assertIn("services/core/rust-toolchain.toml", frozen["build_inputs"])
            (out / "lmm-core").write_bytes(b"changed")
            self.assertNotEqual(material.manifest(out, REVISION), frozen)
            (out / "sbom.spdx.json").write_text('{"spdxVersion":"SPDX-2.3","packages":[]}')
            with self.assertRaisesRegex(ValueError, "SPDX"):
                material.manifest(out, REVISION)


if __name__ == "__main__":
    unittest.main()
