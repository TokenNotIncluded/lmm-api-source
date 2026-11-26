#!/usr/bin/env python3
"""Turn environment-owned deployment targets into a build plan and inventory."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
COMPONENTS = ("core", "extensions", "web")


def selected_targets():
    targets = json.loads(os.environ["DEPLOY_TARGETS"])
    if not isinstance(targets, list) or not targets:
        raise ValueError("DEPLOY_TARGETS must be a nonempty JSON array")
    names = set()
    wanted = os.environ.get("SELECT_TARGETS", "all") or "all"
    wanted = set(wanted.split(",")) if wanted != "all" else None
    components = os.environ.get("SELECT_COMPONENTS", "all") or "all"
    components = set(components.split(",")) if components != "all" else set(COMPONENTS)
    if not components or not components <= set(COMPONENTS):
        raise ValueError("unknown selected component")
    selected = []
    for target in targets:
        name = target["name"]
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*", name) or name in names:
            raise ValueError("target names must be unique inventory identifiers")
        names.add(name)
        if target.get("arch", "amd64") not in ("amd64", "arm64"):
            raise ValueError("unsupported target architecture")
        root = Path(target["root"])
        if not root.is_absolute() or ".." in root.parts or len(root.parts) < 3:
            raise ValueError("each target requires its own absolute deployment root")
        if not target["host"] or not 1 <= int(target.get("port", 22)) <= 65535:
            raise ValueError("invalid SSH destination")
        if not isinstance(target.get("allow_rollback", True), bool):
            raise ValueError("allow_rollback must be a JSON boolean")
        configured = target["components"]
        if not configured or not set(configured) <= set(COMPONENTS):
            raise ValueError("unknown or empty target components")
        for component, config in configured.items():
            if not config["health_url"].startswith(("http://", "https://")):
                raise ValueError("each component requires an HTTP health check")
            if component != "web":
                if not re.fullmatch(r"[A-Za-z0-9@_.-]+", config["service"]):
                    raise ValueError("invalid service name")
                if not Path(config["config"]).is_absolute() or not config["user"]:
                    raise ValueError("backend user and existing absolute config are required")
        chosen = {c: configured[c] for c in COMPONENTS if c in configured and c in components}
        if chosen and (wanted is None or name in wanted):
            selected.append({**target, "components": chosen})
    if wanted is not None and not wanted <= names:
        raise ValueError("selected target does not exist")
    if not selected:
        raise ValueError("selection has no deployable components")
    return selected


def plan():
    targets = selected_targets()
    matrix = {}
    for target in targets:
        for component in target["components"]:
            arch = "any" if component == "web" else target.get("arch", "amd64")
            matrix[component, arch] = {
                "component": component, "arch": arch,
                "runner": "ubuntu-24.04-arm" if arch == "arm64" else "ubuntu-24.04",
            }
    version = os.environ.get("DEPLOY_VERSION", "")
    if not version:
        tag = os.environ.get("GITHUB_REF_NAME", "") if os.environ.get("GITHUB_REF_TYPE") == "tag" else ""
        version = re.sub(r"^(?:shadow-)?v", "", tag) if tag else "0.0.0-preview." + os.environ.get("GITHUB_RUN_NUMBER", "0")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9][A-Za-z0-9.-]*)?", version):
        raise ValueError("version must be a semantic version")
    result = {"matrix": json.dumps({"include": list(matrix.values())}, separators=(",", ":")), "version": version}
    with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
        for key, value in result.items():
            stream.write(f"{key}={value}\n")
    print(json.dumps({"targets": [t["name"] for t in targets], **result}))


def inventory(artifacts, output):
    spec = importlib.util.spec_from_file_location("distribution", ROOT / "scripts/build-distribution.py")
    distribution = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(distribution)
    revision = os.environ["DEPLOY_REVISION"]
    web_base = os.environ.get("DEPLOY_WEB_BASE", "")
    if web_base and (
        not re.fullmatch(r"[0-9a-f]{40}", web_base)
        or os.environ.get("SELECT_COMPONENTS") != "web"
        or os.environ.get("SELECT_TARGETS", "all") != "all"
    ):
        raise ValueError("Web-only deployment requires a verified base and all configured targets")
    backend_by_target = {
        target["name"]: [
            {"name": name, **config} for name, config in target["components"].items()
            if name != "web"
        ]
        for target in json.loads(os.environ["DEPLOY_TARGETS"])
    } if web_base else {}

    archives = {}
    for archive in artifacts.rglob("*.tar.gz"):
        manifest = distribution.verify(archive, revision)
        key = manifest["component"], manifest["platform"]
        if key in archives:
            raise ValueError("duplicate component archive")
        with archive.open("rb") as stream:
            checksum = hashlib.file_digest(stream, "sha256").hexdigest()
        archives[key] = {
            "archive": str(archive.resolve()),
            "sha256": checksum,
            "expanded_bytes": sum(item["size"] for item in manifest["files"].values()) + 1024 * 1024,
            "compressed_bytes": archive.stat().st_size,
            "files": manifest["files"],
        }
    hosts = {}
    for target in selected_targets():
        components = []
        for name, config in target["components"].items():
            platform = "any" if name == "web" else "linux-" + target.get("arch", "amd64")
            components.append({"name": name, **config, **archives[name, platform]})
        hosts[target["name"]] = {
            "ansible_host": target["host"], "ansible_port": int(target.get("port", 22)),
            "ansible_user": target.get("user", "root"),
            "ansible_python_interpreter": target.get("python", "/usr/bin/python3"),
            "ansible_remote_tmp": target["root"] + "/.ansible-tmp",
            "deploy_root": target["root"], "deploy_components": components,
            "deploy_min_free_bytes": int(target.get("min_free_bytes", 256 * 1024 * 1024)),
            "deploy_revision": revision,
            "deploy_web_base": web_base,
            "deploy_web_backends": backend_by_target.get(target["name"], []),
            "deploy_allow_rollback": target.get("allow_rollback", True),
        }
    if web_base and not any(host["deploy_web_backends"] for host in hosts.values()):
        raise ValueError("Web-only deployment has no co-located backend to verify")
    if web_base and set(hosts) != set(backend_by_target):
        raise ValueError("Web-only deployment cannot omit a configured host")
    output.write_text(json.dumps({"all": {"hosts": hosts}}, indent=2) + "\n")
    print(json.dumps({"inventory": str(output), "targets": list(hosts)}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("plan")
    inv = sub.add_parser("inventory")
    inv.add_argument("--artifacts", type=Path, required=True)
    inv.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "plan":
        plan()
    else:
        inventory(args.artifacts, args.output)
