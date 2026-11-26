import copy
import importlib.util
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import Mock

ROOT = Path(__file__).resolve().parent
SCRIPT = ROOT / "prepare-openrouter-price-draft.py"
FEED = ROOT / "fixtures/openrouter-prices-mixed.json"
spec = importlib.util.spec_from_file_location("openrouter_price_draft", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def current():
    def rule(i, name):
        return {"id": i, "model": name, "group": "default", "speed": "standard",
                "rates": dict.fromkeys(mod.RATES.values(), 1), "group_multiplier_ppm": 1_000_000,
                "speed_multiplier_ppm": 1_000_000}
    return {"version": {"routing": 1, "pricing": 1},
            "prices": {"version": 1, "rules": [rule(1, "model-a"), rule(2, "model-b"), rule(3, "model-c")]}}


class PriceDraft(unittest.TestCase):
    def test_mixed_feed_preserves_unselected_and_unusable_rules_and_repeats_exactly(self):
        active = current()
        before = copy.deepcopy(active)
        plan = mod.prepare(FEED.read_bytes(), active, [3, 2, 1])
        self.assertEqual(active, before)
        self.assertEqual(plan, mod.prepare(FEED.read_bytes(), active, [1, 2, 3]))
        prices = plan["preview_request"]["prices"]
        self.assertEqual(prices["version"], 2)
        self.assertEqual(prices["rules"][0]["rates"],
                         {"input": 2500000, "output": 10000000, "cache_read": 250000,
                          "cache_write": 3125000, "request": 1000})
        self.assertEqual(prices["rules"][1:], active["prices"]["rules"][1:])
        self.assertEqual([row["reason"] for row in plan["skipped_rows"]],
                         ["structured_price:prompt", "missing_price:input_cache_read"])
        self.assertEqual(plan["feed_sha256"], mod.sha(FEED.read_bytes()))
        self.assertEqual(plan["preview_request"]["expected_version"], active["version"])
        active.update(prices=prices, version={"routing": 3, "pricing": 2})
        repeat = mod.prepare(FEED.read_bytes(), active, [1])
        self.assertIsNone(repeat["preview_request"])
        self.assertEqual(repeat["unchanged_rule_ids"], [1])
        transport = Mock()
        self.assertEqual(mod.review(repeat, transport, None, None), {"status": "no_changes"})
        transport.assert_not_called()

    def test_exact_decimal_no_binary_float_rounding_underflow_or_overflow(self):
        for raw, scale, want in [
            ("0", 12, 0), ("0.000000000001", 12, 1), ("1e-6", 6, 1),
            ("0.000001000", 6, 1), ("9007.199254740993", 12, 2**53 + 1),
            ("9223372.036854775807", 12, mod.MAX_INT),
        ]:
            self.assertEqual(mod.native_amount(raw, scale), want)
        for raw in ["9223372.036854775808", "1e-13", "9e-90000000", "1e90000000",
                    "-1", "NaN", "Infinity", "-Infinity", "", " 0", "0x10", {}, [], True, 1, 1.25]:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                mod.native_amount(raw, 12)

    def test_each_invalid_price_isolated_including_conditions_extras_and_ambiguity(self):
        good = json.loads(FEED.read_bytes())["data"][0]
        cases = [
            ({"prompt": ["0"]}, "structured_price:prompt"),
            ({"completion": float("nan")}, "nonfinite_price:completion"),
            ({"completion": float("inf")}, "nonfinite_price:completion"),
            ({"discount": "0"}, "unknown_price_dimension"),
            ({"completion": "-0.001"}, "negative_price:completion"),
            ({"image": "0.05"}, "unsupported_unit_price:image"),
            ({"overrides": [{"min_prompt_tokens": 200000, "prompt": "0.002"}]}, "unsupported_conditional_pricing"),
        ]
        for edit, reason in cases:
            bad = {"id": "bad", "pricing": {**good["pricing"], **edit}}
            quotes, skipped = mod.feed_quotes(json.dumps({"data": [bad, good]}).encode())
            self.assertEqual(set(quotes), {"model-a"})
            self.assertEqual(skipped[0]["reason"], reason)
        mixed = json.dumps({"data": [good, good, {"id": "other", "pricing": good["pricing"]}]}).encode()
        quotes, skipped = mod.feed_quotes(mixed)
        self.assertEqual(set(quotes), {"other"})
        self.assertEqual([x["reason"] for x in skipped], ["duplicate_model"] * 2)
        for corrupt in [b'{"id":"good","id":"bad","pricing":{}}',
                        b'{"id":"bad","pricing":{"prompt":"0","prompt":"1"}}']:
            good_raw = json.dumps(good).encode()
            quotes, skipped = mod.feed_quotes(b'{"data":[' + good_raw + b',' + corrupt + b']}')
            self.assertEqual(set(quotes), {"model-a"})
            self.assertTrue(skipped)

    def test_explicit_zero_is_valid_and_rule_selection_preserves_other_groups(self):
        prices = dict.fromkeys(mod.RATES, "0")
        raw = json.dumps({"data": [{"id": "model-a", "pricing": prices}]}).encode()
        active = current()
        other = copy.deepcopy(active["prices"]["rules"][0])
        other.update(id=9, group="partner", group_multiplier_ppm=320000)
        active["prices"]["rules"].append(other)
        result = mod.prepare(raw, active, [9])
        proposed = result["preview_request"]["prices"]["rules"]
        self.assertEqual(proposed[:3], active["prices"]["rules"][:3])
        self.assertEqual(proposed[3], {**other, "rates": dict.fromkeys(mod.RATES.values(), 0)})
        self.assertEqual(result["changed"][0]["id"], 9)

    def test_scope_existing_tariffs_and_versions(self):
        for field, value in [("billing_mode","upstream_cost"), ("billing_mode","tiered_decimal"),
                             ("rounding","legacy_go_ratio_half_away_min1_v1")]:
            snapshot = current()
            snapshot["prices"]["rules"][0][field] = value
            result = mod.prepare(FEED.read_bytes(), snapshot, [1])
            self.assertIsNone(result["preview_request"])
            self.assertEqual(result["skipped_rules"][0]["reason"], "non_native_token_rule")
        for ids in [[], [0], [4], [1, 1]]:
            with self.assertRaises(ValueError):
                mod.prepare(FEED.read_bytes(), current(), ids)

    def test_preview_drift_and_draft_collision_do_not_write(self):
        p = mod.prepare(FEED.read_bytes(), current(), [1])
        preview = {"confirmation_digest": "a"*64, "saved": False, "after": p["preview_request"]["prices"],
                   "expected_version": p["base_version"]}
        for replies in [[{**preview, "after": {}}],
                        [preview, {"items": [], "version": {"routing": 2, "pricing": 1}}],
                        [preview, {"version": p["base_version"], "items": [{"stale": True, "draft": {"id": 3}}]}],
                        [preview, {"version": p["base_version"], "items": []}]]:
            call = Mock(side_effect=replies)
            with self.assertRaises(ValueError):
                mod.review(p, call, 3, "wrong")
            self.assertFalse(any(c.args[0] == "PUT" for c in call.call_args_list))

    def test_loopback_operator_cli_previews_saves_once_and_recovers_completed_save(self):
        active = current()
        requests, drafts = [], []
        class Core(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def handle_api(self):
                self_test.assertEqual(self.headers["Authorization"], "Bearer synthetic-admin")
                requests.append((self.command, self.path))
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"null")
                if (self.command, self.path) == ("GET", mod.PRICES):
                    data = active
                elif (self.command, self.path) == ("POST", mod.PRICES + "/preview"):
                    self_test.assertEqual(body["expected_version"], active["version"])
                    data = {"saved": False, "after": body["prices"], "expected_version": body["expected_version"],
                            "confirmation_digest": "b"*64}
                elif (self.command, self.path) == ("GET", mod.PRICES + "/drafts"):
                    data = {"items": drafts, "version": active["version"]}
                elif (self.command, self.path) == ("PUT", mod.PRICES + "/drafts/15"):
                    self_test.assertFalse(drafts)
                    self_test.assertEqual(body["expected_version"], active["version"])
                    drafts.append({"stale":False, "draft": {"id":15, "name":body["name"], "prices":body["prices"]}})
                    active["version"]["routing"] += 1
                    data = {"id":15, "published":False, "version":active["version"]}
                else:
                    self.send_error(404)
                    return
                msg = json.dumps({"success":True, "data":data}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(msg)))
                self.end_headers()
                self.wfile.write(msg)
            do_GET = do_POST = do_PUT = handle_api
        self_test = self
        with tempfile.TemporaryDirectory() as tmp, ThreadingHTTPServer(("127.0.0.1",0),Core) as server:
            worker=threading.Thread(target=server.serve_forever,daemon=True)
            worker.start()
            try:
                session = Path(tmp)/"session"
                session.write_text("synthetic-admin")
                session.chmod(0o600)
                cmd = [sys.executable, str(SCRIPT), "--feed", str(FEED), "--rule-id", "1",
                       "--core-url", "http://127.0.0.1:" + str(server.server_port), "--session-file", str(session)]
                read = json.loads(subprocess.check_output(cmd))
                self.assertEqual(read["core"]["status"], "previewed")
                save = cmd + ["--save-draft","15","--accept",read["plan_sha256"]]
                self.assertEqual(json.loads(subprocess.check_output(save))["core"]["status"], "saved")
                self.assertEqual(json.loads(subprocess.check_output(save))["core"]["status"], "already_saved")
                self.assertEqual(sum(method == "PUT" for method, _ in requests), 1)
                self.assertEqual(active["prices"], current()["prices"])
            finally:
                server.shutdown()
                worker.join()

if __name__ == "__main__":
    unittest.main()
