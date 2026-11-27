"""Read an inactive LMM-written provider from the official desktop accessibility tree."""
import contextlib
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import time

import pyatspi

APP, CLI, TEST = [Path(arg).resolve(strict=True) for arg in sys.argv[1:4]]
HOME = Path(os.environ["HOME"]).resolve(strict=True)
DB = HOME / ".cc-switch" / "cc-switch.db"
OUTPUT = Path(os.environ["RUNNER_TEMP"]) / "cc-switch-desktop-output"
GATE = "oauth::applications::tests::desktop_fixture_adds_one_synthetic_provider_without_activation"
VERSION = "4.0.22"


def window_tree():
    nodes = []
    pending = list(pyatspi.Registry.getDesktop(0))
    while pending and len(nodes) < 3500:
        item = pending.pop()
        try:
            role, name = item.getRoleName(), item.name or ""
            if not name and role == "heading":
                label = item.queryText()
                name = label.getText(0, label.characterCount)
            nodes.append((role, name.strip(), item))
            pending.extend(list(item))
        except (RuntimeError, NotImplementedError):
            continue
    return nodes


def click(item):
    action = item.queryAction()
    if action.nActions and action.doAction(0):
        return
    raise RuntimeError("Visible desktop control has no invokable action")


def provider_page(process, expected=None):
    deadline = time.monotonic() + 65
    selected = False
    while time.monotonic() < deadline:
        assert process.poll() is None, "Official desktop exited before readback"
        tree = window_tree()
        buttons = [(n, obj) for r, n, obj in tree if r in {"push button", "button"}]
        notice = next((obj for n, obj in buttons if n in {"Got it", "我知道了", "知道了"}), None)
        if notice is not None:
            click(notice)
            time.sleep(0.4)
            continue
        selector = next((obj for n, obj in buttons if n == "Codex" or n.startswith("Codex ·")), None)
        if selector is not None and not selected:
            click(selector)
            selected = True
            time.sleep(0.6)
            continue
        # WebKitGTK exposes each card's visible Edit control under its full provider
        # name even on builds that omit the h3 from AT-SPI's heading collection.
        cards = {n.removeprefix("Edit ") for n, _ in buttons if n.startswith("Edit ")}
        if selected and expected and expected in cards:
            return cards
        if selected and not expected and DB.is_file():
            with sqlite3.connect(f"file:{DB}?mode=ro", uri=True) as conn:
                seeded = {row[0] for row in conn.execute(
                    "select name from providers where app_type='codex'"
                )}
            if seeded & cards:
                return cards
        time.sleep(0.4)
    # Only isolated synthetic names, app navigation labels and roles are collected.
    visible = ", ".join(sorted({n for r, n, _ in window_tree()
                               if r in {"heading", "push button", "button"} and n})[:65])
    raise RuntimeError(f"Expected native Codex provider card was unavailable; controls={visible[:1400]}")


@contextlib.contextmanager
def launch():
    process = subprocess.Popen([APP], start_new_session=True)
    try:
        yield process
    finally:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        time.sleep(0.8)


def snapshot():
    with sqlite3.connect(f"file:{DB}?mode=ro", uri=True) as conn:
        assert conn.execute("pragma user_version").fetchone() == (24,)
        return set(conn.execute("select app_type, id from providers where is_current != 0"))


assert HOME.is_relative_to(Path(os.environ["RUNNER_TEMP"]).resolve()) and not DB.exists()
assert [x["ifname"] for x in json.loads(subprocess.check_output(["ip", "-j", "link"]))] == ["lo"]
os.environ.update(CC_SWITCH_TEST_HOME=str(HOME), LMM_CC_SWITCH_CLI=str(CLI),
                  XDG_CONFIG_HOME=str(HOME/".config"), XDG_DATA_HOME=str(HOME/".local/share"),
                  XDG_STATE_HOME=str(HOME/".local/state"), XDG_CACHE_HOME=str(HOME/".cache"),
                  GDK_BACKEND="x11", LANGUAGE="en", LC_ALL="C.UTF-8",
                  NO_AT_BRIDGE="0", GTK_MODULES="atk-bridge",
                  WEBKIT_DISABLE_DMABUF_RENDERER="1", LIBGL_ALWAYS_SOFTWARE="1")
OUTPUT.mkdir()
with launch() as desktop:
    before_cards = provider_page(desktop)
before_selection = snapshot()
test_run = subprocess.run([TEST, "--ignored", "--exact", GATE], check=True, text=True,
                          stdout=subprocess.PIPE, timeout=30)
assert f"test {GATE} ... ok" in test_run.stdout.splitlines(), "Exact native add case did not run"
fixture = json.loads((HOME/"desktop-fixture.json").read_text())
assert fixture["provider"] not in before_cards and snapshot() == before_selection
readback = subprocess.run([CLI, "receipt", "cc-switch", "--instance", str(DB.parent),
                           "--issuer", fixture["issuer"], "--installation-id", fixture["installation"],
                           "--change", fixture["change"], "--json"], check=True, text=True,
                          stdout=subprocess.PIPE, timeout=20)
receipt = json.loads(readback.stdout)["receipt"]
assert receipt["provider_present"] and receipt["provider_id"] == fixture["id"]
assert receipt["change_id"] == fixture["change"] and not receipt["selection_changed"]
assert not receipt["live_application_changed"] and not receipt["authorization_changed"]
with launch() as desktop:
    provider_page(desktop, fixture["provider"])
    subprocess.run(["import", "-window", "root", str(OUTPUT/"native-codex-provider.png")],
                   check=True, timeout=15)
assert snapshot() == before_selection
assert ("codex", fixture["id"]) not in before_selection
result = {"application":"CC Switch", "version":VERSION, "schema":24, "ui":"Linux X11 / AT-SPI",
          "observed_provider_control":"Edit " + fixture["provider"], "provider_id":fixture["id"],
          "lmm_change_id":fixture["change"], "selection_preserved":True,
          "official_process_restarted":True, "isolated_network_interfaces":["lo"],
          "catalog_source":"synthetic test input to LMM preview_add and apply",
          "cli_receipt_readback":True, "oauth_exercised":False, "model_invocations":0}
(OUTPUT/"receipt.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result))
