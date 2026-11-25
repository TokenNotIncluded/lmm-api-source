"""Local Git + file publication tests. LMM_CHANNEL_CORE_BIN enables the real Core integration."""
import argparse
import copy
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
import threading
import tomllib
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("channel-change.py")
loader = importlib.machinery.SourceFileLoader("channel_change", str(SCRIPT))
spec = importlib.util.spec_from_loader(loader.name, loader)
change = importlib.util.module_from_spec(spec)
loader.exec_module(change)


def model_inputs(priced=7):
    """Priced IDs have active primaries in both groups. The remainder have neither a route nor a price."""
    names = ["Enterprise", "国产"]
    source = """# Preserve this operator comment and reference-only credential inventory.
version=1
input_token_ceiling=100
lease_seconds=120
timeout_seconds=30
max_output_tokens=20
credentials=[{reference={id=1,revision=1},secret_file='/run/secrets/channel-a'},{reference={id=2,revision=1},secret_file='/run/secrets/channel-b'}]
key_groups=[]
[routing]
version=12
groups=[{name='Enterprise',enabled=true},{name='国产',enabled=true}]
upstreams=[{id=1,endpoint='http://127.0.0.1:8001',protocol='openai_chat',credential={id=1,revision=1},enabled=true},{id=2,endpoint='http://127.0.0.1:8002',protocol='openai_chat',credential={id=2,revision=1},enabled=true}]
aliases=[]
"""
    selection = {"expected_version": {"routing": 12, "pricing": 4}, "channel_id": 2,
                 "attachments": [], "new_routes": []}
    rules = []
    for group_no, group in enumerate(names):
        for model_no in range(1, 10):
            route = 100 + group_no * 10 + model_no
            model = f"development-model-{model_no}"
            selection["attachments"].append({"route_id": route, "upstream_model": model,
                "placement": "backup", "speeds": ["standard"], "priority": 0, "weight": 1,
                "timeout_ms": 30000, "payload": "adapt"})
            if model_no > priced:
                selection["new_routes"].append({"id": route, "model": model, "group": group, "speeds": ["standard"]})
                continue
            source += f"""
[[routing.routes]]
id={route}
model='{model}'
group='{group}'
enabled=true
speeds=['standard']
targets=[{{id={route},upstream_id=1,upstream_model='{model}',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=30000}}]
failover={{max_attempts=1,retry_on=[],allow_replay_after_send=false}}
"""
            rules.append(f"{{id={route},model='{model}',group='{group}',speed='standard',rates={{input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0}},group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000}}")
    return (source + "\n[prices]\nversion=4\nrules=[" + ",\n".join(rules) + "]\n").encode(), selection


class Flow(unittest.TestCase):
    def setUp(self):
        self.tmp = TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        self.git("init", "-q", "-b", "reviewed")
        self.git("config", "user.name", "Local Operator")
        self.git("config", "user.email", "test@example.invalid")
        self.raw, self.intent = model_inputs()
        self.path = self.repo / "models.toml"
        self.installed = root / "installed.toml"
        self.path.write_bytes(self.raw)
        self.installed.write_bytes(self.raw)
        self.git("add", "models.toml")
        self.git("commit", "-qm", "Install base")
        self.selection = root / "selection.json"
        self.selection.write_text(json.dumps(self.intent))
        self.base = self.git("rev-parse", "HEAD")
        self.args = argparse.Namespace(repo=self.repo, release_branch="reviewed", branch="ai/channel-change",
            config="models.toml", installed=self.installed, plan=self.selection, local_http=True,
            core=os.environ.get("LMM_CHANNEL_CORE_BIN", "lmm-core"), token_stdin=False,
            credential_file=None, allow_insecure_loopback=True)
        self.actual = None
        flow = self

        class CoreReadback(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                if self.path != change.CONFIG or self.headers.get("Authorization") != "Bearer fixture-read-session":
                    self.send_error(403)
                    return
                if flow.actual is None:
                    self.send_error(503)
                    return
                data = {"version": flow.actual["version"], "authority":
                        {"mode": "file", "source": "model.config_file", **flow.actual}}
                body = json.dumps({"success": True, "data": data}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = HTTPServer(("127.0.0.1", 0), CoreReadback)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.thread.join)
        self.addCleanup(self.server.shutdown)
        self.args.instance = f"http://127.0.0.1:{self.server.server_port}"
        token = patch.dict(os.environ, {"LMM_ADMIN_SESSION": "fixture-read-session"})
        token.start()
        self.addCleanup(token.stop)

    def git(self, *argv):
        return subprocess.check_output(["git", "-C", self.repo, *argv], text=True).strip()

    def lifecycle(self, result):
        before = change.native_state(result, False)
        after = change.native_state(result, True)
        self.actual = before
        (self.repo / "unrelated").write_text("uncommitted")
        proposal = change.propose(self.args)
        self.args.commit = proposal["commit"]
        self.assertEqual(self.git("rev-parse", "HEAD"), self.base)
        self.assertEqual(self.path.read_bytes(), self.raw)
        self.assertEqual(self.git("status", "--porcelain"), "?? unrelated")
        self.assertEqual(change.status(self.args)["state"], "awaiting_review")
        with self.assertRaisesRegex(change.Error, "human-reviewed"):
            change.install(self.args)
        self.git("merge", "--ff-only", "-q", self.args.commit)
        self.assertEqual(change.status(self.args)["state"], "reviewed")
        self.actual = {**before, "routing_sha256": "0" * 64}
        with self.assertRaisesRegex(change.Error, "changed"):
            change.install(self.args)
        self.assertEqual(self.installed.read_bytes(), self.raw)
        self.actual = before
        self.assertEqual(change.install(self.args)["state"], "published")
        self.assertEqual(change.install(self.args)["state"], "published")
        self.assertEqual(change.status(self.args)["state"], "published")
        self.assertNotIn(b"fixture-read-session", self.path.read_bytes())
        self.actual = None
        self.assertIsNone(change.status(self.args)["effective"])
        self.actual = {**after, "pricing_sha256": "f" * 64}
        self.assertEqual(change.status(self.args)["state"], "stale")
        self.actual = after
        self.assertTrue(change.status(self.args)["effective"])
        return proposal

    def test_git_receipts_and_loopback_readback(self):
        # The transport test isolates Git and the installation lifecycle from native routing.
        candidate = self.raw.decode().replace("version=12", "version=13")
        result = {"model_config": candidate, "before": {"version": {"routing": 12, "pricing": 4}, "routing_sha256": "a" * 64, "pricing_sha256": "c" * 64},
                  "after": {"version": {"routing": 13, "pricing": 4}, "routing_sha256": "b" * 64, "pricing_sha256": "c" * 64},
                  "preview": {"channel_id": 2, "version": {"routing": 12, "pricing": 4},
                   "candidate_version": {"routing": 13, "pricing": 4}, "candidates": [], "draft_bindings": []}}
        with patch.object(change, "plan", return_value=result):
            self.lifecycle(result)

    @unittest.skipUnless(os.environ.get("LMM_CHANNEL_CORE_BIN"), "Set LMM_CHANNEL_CORE_BIN to a built Core")
    def test_all_nine_priced_become_backups(self):
        self.raw, self.intent = model_inputs(priced=9)
        self.path.write_bytes(self.raw)
        self.installed.write_bytes(self.raw)
        self.selection.write_text(json.dumps(self.intent))
        self.git("add", "models.toml")
        self.git("commit", "-qm", "Install all nine exact prices")
        self.base = self.git("rev-parse", "HEAD")
        result = change.plan(self.args, self.raw, self.intent, True)
        self.assertTrue(result["preview"]["ready"])
        receipt = self.lifecycle(result)
        self.assertEqual(receipt["priced_bindings"], 18)
        self.assertEqual(receipt["draft_bindings"], [])
        rows = tomllib.loads(self.installed.read_text())["routing"]["routes"]
        self.assertEqual(len(rows), 18)
        for row in rows:
            self.assertEqual([t["upstream_id"] for t in row["targets"]], [1, 2])
            self.assertGreater(row["targets"][1]["priority"], row["targets"][0]["priority"])

    @unittest.skipUnless(os.environ.get("LMM_CHANNEL_CORE_BIN"), "Set LMM_CHANNEL_CORE_BIN to a built Core")
    def test_real_native_nine_models_two_groups(self):
        result = change.plan(self.args, self.raw, self.intent, True)
        receipt = self.lifecycle(result)
        self.assertEqual(receipt["priced_bindings"], 14)
        self.assertEqual(len(receipt["draft_bindings"]), 4)
        self.assertEqual({b["model"] for b in receipt["draft_bindings"]}, {"development-model-8", "development-model-9"})
        self.assertEqual({b["group"] for b in receipt["draft_bindings"]}, {"Enterprise", "国产"})
        old = tomllib.loads(self.raw.decode())
        new = tomllib.loads(self.installed.read_text())
        self.assertEqual(old["prices"], new["prices"])
        self.assertEqual(old["credentials"], new["credentials"])
        self.assertEqual(len(new["routing"]["routes"]), 14)
        self.assertEqual(new["routing"]["version"], 13)
        self.assertTrue(self.installed.read_text().startswith("# Preserve this operator comment"))
        for row, previous in zip(new["routing"]["routes"], old["routing"]["routes"], strict=True):
            self.assertEqual(row["targets"][0], previous["targets"][0])
            self.assertEqual(row["targets"][1]["upstream_id"], 2)
            self.assertGreater(row["targets"][1]["priority"], row["targets"][0]["priority"])
            self.assertEqual(row["failover"]["max_attempts"], 2)
        short = copy.deepcopy(self.intent)
        short["attachments"][0]["timeout_ms"] = 100
        with self.assertRaisesRegex(change.Error, "timeout"):
            change.plan(self.args, self.raw, short, True)
        stale = copy.deepcopy(self.intent)
        stale["expected_version"]["pricing"] += 1
        with self.assertRaisesRegex(change.Error, "CHANNEL_VERSION_CONFLICT"):
            change.plan(self.args, self.raw, stale, True)


if __name__ == "__main__":
    unittest.main()
