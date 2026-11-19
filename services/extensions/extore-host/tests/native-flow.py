#!/usr/bin/env python3
"""Exercise the actual Go client and native host against isolated Extore OAuth."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time


def sha256(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--extore-source", required=True, type=Path)
    parser.add_argument("--extore-binary", required=True, type=Path)
    parser.add_argument("--host", required=True, type=Path)
    parser.add_argument("--plugin", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[4]
    extore_source = args.extore_source.resolve()
    gitlink = subprocess.check_output(
        ["git", "ls-files", "--stage", "third_party/extore"], cwd=root, text=True).split()
    extore_revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=extore_source, text=True).strip()
    extore_dirty = subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=extore_source, text=True).strip()
    if len(gitlink) < 2 or gitlink[0] != "160000" or gitlink[1] != extore_revision or extore_dirty:
        raise RuntimeError("native fixture requires the clean pinned Extore source")
    input_manifest = extore_source / "docs/verification/native-plugin-inputs.json"
    extore_inputs = json.loads(input_manifest.read_text())
    for entry in extore_inputs["files"]:
        source = extore_source / entry["path"]
        if source.stat().st_size != entry["bytes"] or sha256(source) != entry["sha256"]:
            raise RuntimeError("pinned Extore source differs from its verified input closure")
    fixture_path = extore_source / "scripts/native_plugin_fixture.py"
    spec = importlib.util.spec_from_file_location("extore_native_fixture", fixture_path)
    fixture_hash = sha256(fixture_path)
    extore_binary_hash = sha256(args.extore_binary)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    command = ["mise", "exec", "--", "go", "test", "./internal/extore",
               "-run", "^TestExtoreNativePlugin$", "-count=1", "-v"]
    started = time.monotonic()
    with module.ExtoreFixture(args.extore_binary.resolve(),
                              origin="https://extore-plugin.test") as fixture:
        # The provisioned copies have the same reviewed bytes and private ownership.
        code = fixture.root / "code"
        code.mkdir(mode=0o700)
        host = code / "lmm-extore-host"
        plugin = code / "libextore_zplugin.so"
        shutil.copyfile(args.host.resolve(), host)
        shutil.copyfile(args.plugin.resolve(), plugin)
        host.chmod(0o500)
        plugin.chmod(0o400)
        fixture.data_dir.chmod(0o700)
        host_hash, plugin_hash = sha256(host), sha256(plugin)
        payload = {
            "config": {"Origin": fixture.origin, "Host": str(host), "Library": str(plugin),
                       "SHA256": plugin_hash, "DataDir": str(fixture.data_dir)},
            "http_origin": fixture.http_origin,
            "a": fixture.authorize(fixture.shop_a),
            "b": fixture.authorize(fixture.shop_b),
        }
        manifest = fixture.root / "lmm-fixture.json"
        with manifest.open("x") as file:
            os.chmod(manifest, 0o600)
            json.dump(payload, file)
        env = os.environ.copy()
        env["LMM_EXTORE_PLUGIN_FIXTURE"] = str(manifest)
        with (output / "native-flow.log").open("w") as log:
            child = subprocess.Popen(command, cwd=root / "services/extensions", env=env,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = child.wait(timeout=180)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
                code = 124
    source_files = list((root / "services/extensions/internal/extore").glob("*.go"))
    source_files += list((root / "services/extensions/extore-host/src").glob("*.rs"))
    source_files += [
        root / ".gitmodules",
        root / "services/extensions/extore-host/Cargo.toml",
        root / "services/extensions/extore-host/Cargo.lock",
        Path(__file__).resolve(),
        root / "services/extensions/internal/app/configuration.go",
        root / "services/extensions/internal/app/configuration_extore_plugin.go",
        root / "services/extensions/internal/app/run.go",
        root / "services/extensions/internal/composition/runtime.go",
        root / "services/extensions/internal/composition/extore_plugin.go",
        root / "services/extensions/internal/modules/store/extore.go",
    ]
    receipt = {
        "schema": "lmm.extore-native-selftest.v1",
        "source_sha256": {str(path.relative_to(root)): sha256(path)
                          for path in sorted(source_files)},
        "exit_code": code, "elapsed_seconds": round(time.monotonic() - started, 2),
        "command": command, "host_sha256": host_hash,
        "plugin_sha256": plugin_hash, "extore_binary_sha256": extore_binary_hash,
        "fixture_sha256": fixture_hash, "log": str(output / "native-flow.log"),
        "source_revision": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "extore_revision": extore_revision,
        "extore_gitlink": gitlink[1],
        "extore_source_inputs_sha256": extore_inputs["sha256"],
        "extore_input_manifest_sha256": sha256(input_manifest),
        "extore_verified_source_files": len(extore_inputs["files"]),
    }
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
