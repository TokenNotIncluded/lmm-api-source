#!/usr/bin/env python3
"""Install the preview APK and exercise its native catalog in an isolated container."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import pwd
import secrets
import subprocess
import sys
import tempfile
import time
import urllib.request


def probe_receipt(report, host_sha256, plugin_sha256):
    """Export only reviewed acceptance facts from the private probe report."""
    def confirmed(actual, expected):
        for key, value in expected.items():
            if type(actual.get(key)) is not type(value) or actual[key] != value:
                raise ValueError(f"unconfirmed native acceptance field: {key}")
        return {key: actual[key] for key in expected}

    receipt = confirmed(report, {
        "status": "passed", "consumer_exit_code": 0,
        "host_library_sha256": host_sha256, "plugin_library_sha256": plugin_sha256,
        "actual_rust_health": True, "isolated_network_namespace": True,
        "tls_certificate_verification": True, "authorized_catalog": True,
        "cross_shop_http_detail_denied": True, "revoked_token_denied": True,
        "catalog_calls_did_not_issue_cards": True, "actual_sqlite_grant_usage_recorded": True,
    })
    receipt["consumer"] = confirmed(report["consumer"], {
        "status": "passed", "host_abi": 1, "interface": "extore.commerce",
        "real_extore_https_sqlite": True, "native_digest_path_binding": True,
        "native_repeated_open_call_release_close": True, "store_sync_persisted": True,
        "http_catalog_fallback_calls": 0, "secret_values_logged": False,
        "production_accounts_created": False,
        "store_repository": "transactional memory test repository",
        "core_authority": "existing Store test double; independent CoreSession acceptance not run",
    })
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=("amd64", "arm64"), required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--parent-netns", required=True)
    parser.add_argument("--package-dir", type=Path, default=Path("/packages"))
    parser.add_argument("--consumer", type=Path, default=Path("/probe/store-native.test"))
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if (os.environ.get("LMM_PACKAGING_ISOLATED") != "1" or os.geteuid() != 0
            or not Path("/.dockerenv").exists()
            or os.readlink("/proc/self/ns/net") == args.parent_netns):
        raise SystemExit("this check requires a disposable container with its own network namespace")
    candidates = list(args.package_dir.glob(f"lmm-extensions-*-1-{args.arch}.apk"))
    if len(candidates) != 1:
        raise SystemExit("expected exactly one release-1 extensions APK")
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    # This unsigned preview is installed only in the disposable container.
    subprocess.run(["apk", "add", "--no-network", "--allow-untrusted", str(candidates[0])], check=True)
    info = json.loads(Path("/usr/share/doc/lmm-extensions/BUILDINFO.json").read_text())
    if (info["source_revision"] != args.revision or info["format"] != "apk"
            or info["source_platform"] != f"linux-musl-{args.arch}"):
        raise SystemExit("installed package identity does not match the frozen musl input")
    for name, expected in info["files"].items():
        path = Path(name)
        if (hashlib.sha256(path.read_bytes()).hexdigest() != expected["sha256"]
                or path.stat().st_mode & 0o777 != expected["mode"]):
            raise SystemExit("installed payload differs from its package build record")
    account = pwd.getpwnam("lmm-extensions")
    if account.pw_uid == 0 or account.pw_gid == 0:
        raise SystemExit("extensions service account must be unprivileged")
    with tempfile.TemporaryDirectory(prefix="lmm-native-package-") as temporary:
        root = Path(temporary)
        root.chmod(0o755)
        state = root / "private"
        state.mkdir(mode=0o700)
        os.chown(state, account.pw_uid, account.pw_gid)
        token = state / "token"
        token.write_text(secrets.token_hex(32))
        token.chmod(0o600)
        os.chown(token, account.pw_uid, account.pw_gid)
        config = root / "extensions.toml"
        config.write_text('version = 1\n[host]\nlisten = "127.0.0.1:18081"\n'
                          f'token_file = "{token}"\nmodules = ["none"]\nweb_compat = false\n')
        config.chmod(0o644)
        native_check = root / "native-check.toml"
        host = "/usr/lib/lmm/extensions/extore/libextore_plugin_host.so"
        plugin = "/usr/lib/lmm/extensions/extore/libextore_commerce_plugin.so"
        native_check.write_text(
            'version = 1\n[host]\n'
            f'token_file = "{state / "absent-host-token"}"\n'
            'modules = ["store"]\nweb_compat = false\n[store]\n'
            'database_name = "lmm_native_load_only"\n'
            f'database_url_file = "{state / "absent-store-url"}"\n'
            f'extore_key_file = "{state / "absent-extore-key"}"\n'
            'extore_redirect_uri = "https://api.lmm.best/store/manage"\n'
            '[store.extore_plugin]\n'
            f'host_library = "{host}"\nplugin_library = "{plugin}"\n'
            f'host_sha256 = "{info["files"][host]["sha256"]}"\n'
            f'plugin_sha256 = "{info["files"][plugin]["sha256"]}"\n')
        native_check.chmod(0o644)
        preflight = subprocess.run(
            ["/usr/bin/lmm-extensions", "--check-extore-plugin", "--config", str(native_check)],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30,
            user=account.pw_uid, group=account.pw_gid, extra_groups=[])
        if preflight.returncode != 0 or preflight.stdout.strip() != (
                "Extore native plugin loaded: host ABI 1, extore.commerce v1"):
            raise RuntimeError("installed load-only native check failed: " + preflight.stdout)
        if any(state.glob("absent-*")):
            raise RuntimeError("load-only check created a business credential")
        with (root / "extensions.log").open("wb") as log:
            process = subprocess.Popen(["/usr/bin/lmm-extensions", "--config", str(config)],
                                       stdout=log, stderr=subprocess.STDOUT,
                                       user=account.pw_uid, group=account.pw_gid, extra_groups=[])
            try:
                ready = False
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                for _ in range(100):
                    if process.poll() is not None:
                        raise RuntimeError("installed extensions exited before its default health endpoint")
                    try:
                        with opener.open("http://127.0.0.1:18081/health/live", timeout=1) as response:
                            ready = response.status == 200
                    except OSError:
                        pass
                    if ready:
                        break
                    time.sleep(0.1)
                if not ready:
                    raise RuntimeError("installed extensions did not start without native configuration")
                # Procfs checks credentials; container root cannot inspect another UID.
                identity = subprocess.run(
                    ["readlink", "-n", f"/proc/{process.pid}/exe"],
                    check=True, stdout=subprocess.PIPE, timeout=5, user=account.pw_uid,
                    group=account.pw_gid, extra_groups=[])
                if identity.stdout != b"/usr/bin/lmm-extensions":
                    raise RuntimeError("default process is not the installed program")
            finally:
                process.terminate()
                try:
                    status = process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                    raise RuntimeError("installed extensions did not stop after SIGTERM")
                if status not in (0, -15):
                    raise RuntimeError(f"installed extensions exited with status {status}")
    args.report.parent.mkdir(parents=True, exist_ok=True)
    receipt = {
        "status": "passed", "source_revision": args.revision,
        "source_platform": info["source_platform"],
        "apk_sha256": hashlib.sha256(candidates[0].read_bytes()).hexdigest(),
        "installed_payload_verified": True,
        "installed_native_load_only_without_business_credentials": True,
        "installed_default_process_without_plugin": True,
        "service_uid": account.pw_uid, "service_gid": account.pw_gid,
        "default_process_stopped": True,
    }
    with tempfile.TemporaryDirectory(prefix="extore-private-probe-") as directory:
        private_report = Path(directory) / "probe.json"
        command = [sys.executable, "/work/tests/native_plugin_probe.py",
                   "--inside-netns", "--parent-netns", args.parent_netns,
                   "--binary", "/opt/extore/extore-server",
                   "--host-library", host, "--plugin-library", plugin,
                   "--report", str(private_report), "--consumer", str(args.consumer),
                   "-test.run", "^TestNativePluginCatalogStore$", "-test.v"]
        subprocess.run(command, check=True)
        accepted = probe_receipt(json.loads(private_report.read_text()),
                                 info["files"][host]["sha256"], info["files"][plugin]["sha256"])
    args.report.write_text(json.dumps(accepted, indent=2) + "\n")
    Path(str(args.report) + ".package.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
