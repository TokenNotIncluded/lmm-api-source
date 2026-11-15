#!/usr/bin/env python3
"""Exercise environment selection and source-pinned deployment inventory."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("deployment_plan", ROOT / "scripts/deployment-plan.py")
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)
spec = importlib.util.spec_from_file_location("distribution", ROOT / "scripts/build-distribution.py")
distribution = importlib.util.module_from_spec(spec)
spec.loader.exec_module(distribution)


def target(name):
    return {"name": name, "host": name + ".example.test", "root": "/srv/application-shadow",
            "components": {"web": {"health_url": "http://127.0.0.1:18090/sign-in"}}}


class DeploymentPlanTests(unittest.TestCase):
    def test_inventory_is_not_limited_to_a_pair_of_servers(self):
        targets = [target("node" + str(i)) for i in range(9)]
        with tempfile.TemporaryDirectory() as temp, patch.dict(os.environ, {
            "DEPLOY_TARGETS": json.dumps(targets), "GITHUB_OUTPUT": str(Path(temp) / "output"),
            "GITHUB_REF_TYPE": "tag", "GITHUB_REF_NAME": "shadow-v1.2.3",
            "DEPLOY_VERSION": "", "SELECT_TARGETS": "all", "SELECT_COMPONENTS": "all",
        }):
            self.assertEqual(len(deploy.selected_targets()), 9)
            deploy.plan()
            output = (Path(temp) / "output").read_text()
            self.assertIn("version=1.2.3\n", output)
            matrix = json.loads(output.split("matrix=", 1)[1].splitlines()[0])
            self.assertEqual([x["component"] for x in matrix["include"]], ["web"])

    def test_unknown_selection_and_duplicate_names_fail_before_any_ssh(self):
        with patch.dict(os.environ, {"DEPLOY_TARGETS": json.dumps([target("node")]),
                                    "SELECT_TARGETS": "missing", "SELECT_COMPONENTS": "all"}):
            with self.assertRaisesRegex(ValueError, "does not exist"):
                deploy.selected_targets()
        with patch.dict(os.environ, {"DEPLOY_TARGETS": json.dumps([target("node"), target("node")]),
                                    "SELECT_TARGETS": "all", "SELECT_COMPONENTS": "all"}):
            with self.assertRaisesRegex(ValueError, "unique"):
                deploy.selected_targets()

    def test_real_archive_is_checked_before_inventory_is_emitted(self):
        revision = "a" * 40
        with tempfile.TemporaryDirectory() as temp:
            work = Path(temp)
            (work / "web").mkdir()
            (work / "web/index.html").write_text("<!doctype html>new build")
            archive = distribution.build(ROOT, "web", "1.2.3", revision, "any", work / "web", work / "artifacts")
            with patch.dict(os.environ, {"DEPLOY_TARGETS": json.dumps([target("node")]),
                                        "SELECT_TARGETS": "all", "SELECT_COMPONENTS": "web",
                                        "DEPLOY_REVISION": revision}):
                deploy.inventory(work / "artifacts", work / "inventory.json")
                host = json.loads((work / "inventory.json").read_text())["all"]["hosts"]["node"]
                self.assertEqual(host["deploy_components"][0]["archive"], str(archive))
                self.assertGreater(host["deploy_components"][0]["expanded_bytes"], 0)
                os.environ["DEPLOY_REVISION"] = "b" * 40
                with self.assertRaises(ValueError):
                    deploy.inventory(work / "artifacts", work / "wrong.json")
                self.assertFalse((work / "wrong.json").exists())


if __name__ == "__main__":
    unittest.main()
