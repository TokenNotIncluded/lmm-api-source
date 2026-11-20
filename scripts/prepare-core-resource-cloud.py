#!/usr/bin/env python3
"""Prepare the fixed 120 Core resource driver in CI; never run acceptance cases."""
from __future__ import annotations

import argparse
import hashlib
import importlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import time
import zipfile

REVISION = "120e0401555c35b7e42eae89d44bab77b2b38087"
TREE = "f669d9c4d1bb6495a58c81a0b5bb69fdf4c7f7d2"
REPOSITORY = "TokenNotIncluded/lmm-api-source"
RUN = 44654769695
JOB = 136018774884
ARTIFACT = 14593054127
ZIP_SIZE = 24058188
ZIP_SHA = "fa7076ccbbdeea69b6b99aee2fd83caaaade8b1e7053ce65909bb96eb0dd517d"
ARCHIVE = "lmm-core-0.1.27-preview.20261120.120e0401-linux-amd64.tar.gz"
ARCHIVE_SHA = "2699590189259cb9f0bb4306c9dcc8c87cc318cef92f667d57ddc6d4f0f88501"
MANIFEST_SHA = "05d4fea8398230491a8fdde741e38f1e85f99c147afb7b84be68469414d57f4d"
CORE_SHA = "f38c49fbaeb2661561d1df169dbc76a1d1b2a46d76d7164f94a2d2f276f01823"
CONTROL = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def record(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def git(source, *arguments):
    return subprocess.check_output(
        ["git", "-C", str(source), *arguments], text=True, timeout=30).strip()


def canonical(path):
    require(path.is_absolute() and path.resolve() == path,
            "paths must be absolute and contain no symlinks")
    return path


def create_private(path):
    canonical(path)
    path.mkdir(mode=0o700, exist_ok=False)


def run(command, cwd, log, environment=None, timeout=120):
    started = time.monotonic()
    with log.open("xb") as stream:
        result = subprocess.run(command, cwd=cwd, env=environment, stdout=stream,
                                stderr=subprocess.STDOUT, timeout=timeout)
    record(log.with_suffix(log.suffix + ".json"), {
        "command": command, "exit_code": result.returncode,
        "seconds": time.monotonic() - started, "log_sha256": sha(log)})
    require(result.returncode == 0, "command failed; inspect " + log.name)
    return log.read_text() if log.stat().st_size < 100000 else ""


def api(path, destination):
    with destination.open("xb") as stream:
        subprocess.run(["gh", "api", "--method", "GET",
                        "repos/" + REPOSITORY + path],
                       stdout=stream, check=True, timeout=120)
    return json.loads(destination.read_text())


def official_core(source, output):
    evidence = output / "official"
    downloads = output / "downloads"
    create_private(evidence)
    create_private(downloads)
    run(["gh", "--version"], source, evidence / "gh-version.log")
    help_text = run(["gh", "api", "--help"], source, evidence / "gh-api-help.log")
    allow_escapes = "--allow-escape-sequences" in help_text
    run_data = api(f"/actions/runs/{RUN}", evidence / "run.json")
    jobs = api(f"/actions/runs/{RUN}/jobs?per_page=100", evidence / "jobs.json")
    artifact = api(f"/actions/artifacts/{ARTIFACT}", evidence / "artifact.json")
    require(run_data["id"] == RUN and run_data["head_sha"] == REVISION
            and run_data["run_attempt"] == 1 and run_data["status"] == "completed"
            and run_data["conclusion"] == "success"
            and run_data["path"] == ".github/workflows/microservice-distribution.yml",
            "official distribution run identity or completion differs")
    selected = [job for job in jobs["jobs"] if job["id"] == JOB]
    require(len(selected) == 1 and selected[0]["status"] == "completed"
            and selected[0]["conclusion"] == "success", "official Core job did not succeed")
    require(artifact["id"] == ARTIFACT and artifact["expired"] is False
            and artifact["name"] == "lmm-preview-core-" + REVISION
            and artifact["size_in_bytes"] == ZIP_SIZE
            and artifact["digest"] == "sha256:" + ZIP_SHA
            and artifact["workflow_run"]["id"] == RUN
            and artifact["workflow_run"]["head_sha"] == REVISION,
            "official artifact metadata differs")
    archive_zip = downloads / "official-core.zip"
    download_command = ["gh", "api", "--method", "GET",
                        f"repos/{REPOSITORY}/actions/artifacts/{ARTIFACT}/zip"]
    if allow_escapes:
        download_command.append("--allow-escape-sequences")
    # Direct binary output; one download, then the same mandatory digest check.
    with archive_zip.open("xb") as stream:
        subprocess.run(download_command, stdout=stream, check=True, timeout=300)
    require(archive_zip.stat().st_size == ZIP_SIZE and sha(archive_zip) == ZIP_SHA,
            "downloaded ZIP size or digest differs")
    with zipfile.ZipFile(archive_zip) as archive:
        members = archive.infolist()
        require(len(members) == 2 and {item.filename for item in members}
                == {ARCHIVE, ARCHIVE + ".sha256"}, "unexpected ZIP inventory")
        for item in members:
            mode = item.external_attr >> 16
            require(not item.is_dir() and not stat.S_ISLNK(mode)
                    and stat.S_IFMT(mode) in (0, stat.S_IFREG)
                    and 0 < item.file_size <= 25000000, "unsafe ZIP member")
            with archive.open(item) as src, (downloads / item.filename).open("xb") as dst:
                shutil.copyfileobj(src, dst)
    package = downloads / ARCHIVE
    require(sha(package) == ARCHIVE_SHA, "official tar digest differs")
    require((downloads / (ARCHIVE + ".sha256")).read_text().split()
            == [ARCHIVE_SHA, ARCHIVE], "official checksum sidecar differs")
    spec = importlib.util.spec_from_file_location(
        "fixed_distribution", source / "scripts/build-distribution.py")
    distribution = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = distribution
    spec.loader.exec_module(distribution)
    manifest = distribution.verify(package, REVISION)
    require(manifest["component"] == "core" and manifest["platform"] == "linux-amd64"
            and len(manifest["files"]) == 7, "official package identity differs")
    record(evidence / "archive-manifest.json", manifest)
    extracted = downloads / "core"
    create_private(extracted)
    # The unchanged verifier above rejects unsafe names, links, types and modes.
    with tarfile.open(package, "r:gz") as archive:
        for item in archive:
            destination = extracted / item.name
            destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            with archive.extractfile(item) as src, destination.open("xb") as dst:
                shutil.copyfileobj(src, dst)
            destination.chmod(item.mode)
    require(sha(extracted / "MANIFEST.json") == MANIFEST_SHA
            and sha(extracted / "bin/lmm-core") == CORE_SHA,
            "extracted official files differ")
    record(evidence / "verification.json", {
        "run": RUN, "job": JOB, "artifact": ARTIFACT, "source_revision": REVISION,
        "gh_allow_escape_sequences_supported": allow_escapes,
        "gh_version_sha256": sha(evidence / "gh-version.log"),
        "gh_api_help_sha256": sha(evidence / "gh-api-help.log"),
        "download_command": download_command,
        "source_tree": TREE, "zip_sha256": sha(archive_zip),
        "archive_sha256": sha(package), "manifest_sha256": MANIFEST_SHA,
        "binary_sha256": sha(extracted / "bin/lmm-core"),
        "verification": "Original fixed-source distribution verifier and actual file digests"})
    return extracted / "bin/lmm-core"


def package_evidence(output, resource, requests):
    files = {}
    for path in sorted(output.rglob("*")):
        relative = path.relative_to(output)
        if (relative.parts[0] == "downloads" or "compile-tmp" in relative.parts
                or path.name in {"core-resource-prepared.tar.gz",
                                 "core-resource-prepared.tar.gz.sha256", "export-manifest.json"}):
            continue
        require(not path.is_symlink(), "evidence contains a symlink")
        if path.is_file():
            files[str(relative)] = path
    for label, directory in (("resource", resource), ("requests", requests)):
        if directory.is_dir():
            for path in sorted(directory.rglob("*.json")):
                require(path.is_file() and not path.is_symlink(), "unsafe lease receipt")
                files["lease/" + label + "/" + str(path.relative_to(directory))] = path
    record(output / "export-manifest.json", {
        "format": "lmm-core-resource-cloud-export-v1",
        "scope": "Original prepare output and provenance; no acceptance execution",
        "files": {name: {"sha256": sha(path), "size": path.stat().st_size,
                         "mode": stat.S_IMODE(path.stat().st_mode)}
                  for name, path in sorted(files.items())}})
    files["export-manifest.json"] = output / "export-manifest.json"
    bundle = output / "core-resource-prepared.tar.gz"
    with tarfile.open(bundle, "w:gz", compresslevel=1) as archive:
        for name, path in sorted(files.items()):
            archive.add(path, arcname=name, recursive=False)
    (output / "core-resource-prepared.tar.gz.sha256").write_text(
        sha(bundle) + "  " + bundle.name + "\n")


def prepare(args, receipt):
    source = canonical(args.source)
    require(os.environ.get("GITHUB_ACTIONS") == "true", "this helper is CI-only")
    require(os.environ.get("GITHUB_REPOSITORY") == REPOSITORY, "unexpected repository")
    require(git(source, "rev-parse", "HEAD") == REVISION
            and git(source, "rev-parse", "HEAD^{tree}") == TREE
            and git(source, "status", "--porcelain", "--untracked-files=all") == "",
            "source checkout must be the clean fixed 120 revision")
    control = git(CONTROL, "rev-parse", "HEAD")
    require(control == os.environ.get("GITHUB_SHA")
            and git(CONTROL, "status", "--porcelain", "--untracked-files=all") == "",
            "control checkout must be the clean dispatched revision")
    receipt.update(source_revision=REVISION, source_tree=TREE, control_revision=control,
                   helper_sha256=sha(Path(__file__).resolve()),
                   workflow_sha256=sha(CONTROL / ".github/workflows/core-protocol.yml"))
    sys.path.insert(0, str(source / "scripts"))
    gate = importlib.import_module("verify_model_resources")
    lease = importlib.import_module("build_lease")
    frozen = gate.inputs()
    record(args.output / "source-inputs-before.json", frozen)
    core = official_core(source, args.output)
    binding = args.output / "source-binding.json"
    record(binding, {"files": frozen["product"]})
    reference = args.output / "official-build-reference.json"
    record(reference, {
        "format": "github-release-build-reference-v1",
        "reference_kind": "Verified existing official GitHub release-profile artifact; not a new local product build",
        "exit_code": 0, "workflow_run": RUN, "run_attempt": 1, "job_id": JOB,
        "workflow_url": f"https://github.com/{REPOSITORY}/actions/runs/{RUN}",
        "source_revision": REVISION, "source_tree": TREE, "binary_path": str(core),
        "binary_sha256": sha(core), "archive_sha256": ARCHIVE_SHA,
        "source_binding_sha256": sha(binding), "source_drift": [],
        "source_drift_basis": "Clean fixed source product closure and verified official artifact revision; reference validation, not observation of a new local product build",
        "evidence": {str(path.relative_to(args.output)): sha(path)
                     for path in sorted((args.output / "official").glob("*.json"))}})
    environment = gate.env()  # Original allowlist, offline mode and Rust policy.
    tools = args.output / "toolchain"
    create_private(tools)
    mise_version = run(["mise", "--version"], source, tools / "mise.log", environment)
    require(mise_version.split()[0] == "2026.11.10", "unexpected mise version")
    cargo = Path(subprocess.check_output(
        ["rustup", "which", "--toolchain", "1.100.0", "cargo"],
        text=True, env=environment, timeout=30).strip()).resolve()
    rust_version = run([str(cargo.parent / "rustc"), "-vV"], source,
                       tools / "rustc.log", environment)
    require("release: 1.100.0\n" in rust_version, "unexpected Rust version")
    run([str(cargo), "--version"], source, tools / "cargo.log", environment)
    run(["mise", "ls", "--json"], source, tools / "mise-installed.log", environment)
    target = source / "services/core/target"
    if not target.exists():
        target.mkdir(mode=0o700)
    require(target.resolve() == target and target.is_dir()
            and target.stat().st_uid == os.geteuid()
            and not target.stat().st_mode & 0o022, "unsafe standard Core target")
    canonical(args.resource)
    canonical(args.requests)
    for parent in {args.resource.parent, args.requests.parent}:
        if not parent.exists():
            create_private(parent)
        lease.private_directory(parent)
    lease.initialize(args.resource)
    create_private(args.requests)
    prepared_root = args.output / "prepare"
    create_private(prepared_root)
    command = [sys.executable, "-B", str(source / "scripts/verify_model_resources.py"),
               "prepare", "--output", str(prepared_root),
               "--resource", str(args.resource), "--requests", str(args.requests),
               "--core-binary", str(core), "--source-binding", str(binding),
               "--build-receipt", str(reference)]
    receipt["prepare_command"] = command
    receipt["original_limits"] = {"compile_timeout_seconds": 1800,
                                 "lease_wait_timeout_seconds": 1200}
    # Original prepare controls its 1800s compilation timeout and 1200s lease wait.
    run(command, source, args.output / "prepare.log", timeout=1900)
    after = gate.inputs()
    record(args.output / "source-inputs-after.json", after)
    require(after == frozen and git(source, "status", "--porcelain",
                                  "--untracked-files=all") == "", "source inputs drifted")
    prepared = json.loads((prepared_root / "prepared.json").read_text())
    require(prepared["format"] == "lmm-resource-prepared-v1"
            and prepared["inputs"] == frozen
            and prepared["source_binding_sha256"] == sha(binding)
            and prepared["build_receipt_sha256"] == sha(reference)
            and prepared["artifacts"]["artifacts/lmm-core"] == CORE_SHA,
            "original prepared receipt differs from actual inputs")
    gate.artifacts_check(prepared_root, prepared["artifacts"])
    gate.reviewed_tests(prepared["test_list"])
    finished = list(args.requests.glob("*/finished.json"))
    require(len(finished) == 1 and json.loads(finished[0].read_text())["exit_code"] == 0
            and (finished[0].parent / "released.json").is_file(),
            "private build lease did not finish and release")
    receipt.update(status="PREPARED_NOT_ACCEPTANCE", source_drift=[],
                   prepared_sha256=sha(prepared_root / "prepared.json"),
                   source_binding_sha256=sha(binding), build_receipt_sha256=sha(reference),
                   artifacts=prepared["artifacts"], rustc=prepared["rustc"],
                   test_list=prepared["test_list"], lease_released=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "output", "resource", "requests"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    create_private(args.output)
    started = time.monotonic()
    receipt = {"format": "lmm-core-resource-cloud-preparation-v1", "status": "FAIL",
               "acceptance_executed": False, "official_run": RUN, "official_artifact": ARTIFACT,
               "cloud_run": os.environ.get("GITHUB_RUN_ID"),
               "cloud_run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT")}
    try:
        prepare(args, receipt)
    except Exception as error:
        receipt["error_type"] = type(error).__name__
        receipt["error"] = str(error)
    finally:
        receipt["seconds"] = time.monotonic() - started
        if not (args.output / "prepare.log").exists():
            (args.output / "prepare.log").write_text(
                "Original prepare was not reached; inspect cloud-preparation-receipt.json.\n")
        record(args.output / "cloud-preparation-receipt.json", receipt)
        try:
            package_evidence(args.output, args.resource, args.requests)
        except Exception as error:
            record(args.output / "export-failure.json", {
                "error_type": type(error).__name__, "error": str(error)})
            print("FAIL: evidence packaging; inspect export-failure.json", file=sys.stderr)
            return 1
    print(receipt["status"] + ": " + str(args.output / "cloud-preparation-receipt.json"))
    return 0 if receipt["status"] == "PREPARED_NOT_ACCEPTANCE" else 1


if __name__ == "__main__":
    raise SystemExit(main())
