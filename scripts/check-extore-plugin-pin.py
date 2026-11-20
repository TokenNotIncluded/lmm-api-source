#!/usr/bin/env python3
"""Verify the initialized Extore dependency matches its packaged source record."""
import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
pin = json.loads((ROOT / "packaging/extore-plugin.json").read_text())
dependency = ROOT / pin["path"]
revision = subprocess.check_output(
    ["git", "-C", str(dependency), "rev-parse", "HEAD"], text=True).strip()
if revision != pin["revision"]:
    raise SystemExit("Extore checkout does not match the fixed plugin source revision")
dirty = subprocess.check_output(
    ["git", "-C", str(dependency), "status", "--porcelain", "--",
     pin["workspace"], "LICENSE"], text=True)
if dirty:
    raise SystemExit("Extore plugin source contains uncommitted changes")
workspace = tomllib.loads((dependency / pin["workspace"] / "Cargo.toml").read_text())
if workspace["workspace"]["dependencies"]["zplugin"]["version"] != "=" + pin["zplugin"]:
    raise SystemExit("Extore zplugin dependency does not match the packaged source record")
print("Extore plugin source pin verified:", revision)
