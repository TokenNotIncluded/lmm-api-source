#!/usr/bin/env python3
"""Explicit cloud projection of the frozen 293 runtime's original local gates.

This controller does not qualify a source release or authorize deployment. It
never accepts a production URL. The product/test runners remain unmodified.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import zipfile

REPOSITORY = "TokenNotIncluded/lmm-api-source"
RUNTIME = "293e53f799b09f187893e9cdf0c3503973916676"
RUNTIME_TREE = "63454c389c91e7a202589bce8a43ff4a170d994b"
EXTORE = "8e3063690cf5595884b1e639535869fd593e0296"
DISTRIBUTION_RUN = 44798787579
COMMERCE_REVISION = "952d21c5d5d849507478d8443f603c47e0126880"
COMMERCE_TREE = "5c54b1576e61963e5baed635a4ccc22e8c9df637"
COMMERCE_RUN = 44802025200
COMMERCE_SHA = "9851d527400ffe97dbeb540cbcc97e10666be9a205e99de760800e5e5f7b5d77"
CORE_SHA = "ece6e14ff4772f3e6bf83095719c73ee62e3824075c0aefcdd4b79813b2f315d"
GO_SHA = "b190132c210f694256b3844e47cac0fed0d4640d6f9712fa672d85860c55a957"
HOST_SHA = "db346ba27cc7bdd121665c06cbaa5ff9099d4f4c6c6085cb6ae2cd1e91a5b125"
PLUGIN_SHA = "25c4928d879472c1f9ff11b9d7679875898a823c6cb042af7e59b510ef07980c"
CLI_SHA = "4ec8556d140c69eb913b0473ef9bc691b669bae8baf9c1ffa8df31cad01bd756"
PG_IMAGE = "postgres:18-alpine@sha256:77f585114c32fbca283dc835b0596f4e52b51b4c6662d7810b2f4084f60a1873"
COMPONENTS = {
    "core": (14661182046, 24074810,
             "f27e36a3f7aba0c4aa25244300a216957b90a914fa2faedde9d87a87333215a7",
             "lmm-core-0.1.0-preview-linux-amd64.tar.gz",
             "30c2a9edcde93e17670c8b31a76c9fd04c6a87d29ccc1727b8ac229a11ca4186"),
    "extensions": (14659991533, 13924517,
                   "1472a0ee530451898f4652f70e939197eeb9fca1d7380cfbebb90ca954112acd",
                   "lmm-extensions-0.1.0-preview-linux-amd64-extore.tar.gz",
                   "38a9c81b0032721f6cf44c4c64b099bd0f8f6f19112fa638948526f383e7f7a2"),
    "web": (14660130923, 19372579,
            "c2b7503a3c0b8f2f25e8ba4a6c8648de934a9499687620d2721b1a07ecb73881",
            "lmm-web-0.1.0-preview-any.tar.gz",
            "bfcef01dc4c480958e1ff34666c0a116e58350fb22a3bc812b0025a571a24968"),
    "cli": (14659413791, 5458470,
            "e6359091d4d5b6248417eacb63307768c5904b41dc4780933847a974b1c3c367",
            "lmm-0.1.0-preview-linux-amd64.tar.gz",
            "e199e99e833f242217defeb8b2911a9f08b406c96e7333a2101610e777f7b98b"),
}
COMMERCE_ARTIFACT = (14662920404, 41001325,
                     "3ef28d125a95229c35023d02402148ffebf7598686737664681b8a7b237079c0")
CONTROL = Path(__file__).resolve().parents[1]
os.umask(0o077)
sys.dont_write_bytecode = True


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def record(path, value):
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, sort_keys=True, indent=2, ensure_ascii=False)
        stream.write("\n")


def private(path):
    path = Path(path)
    require(path.is_absolute() and path.resolve() == path, "noncanonical private path")
    path.mkdir(mode=0o700)
    return path


def regular(path):
    path = Path(path)
    info = path.lstat()
    require(path.is_absolute() and path.resolve() == path
            and stat.S_ISREG(info.st_mode), "noncanonical regular input")
    return path


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def git(source, *args):
    return subprocess.check_output(["git", "-C", str(source), *args],
                                   text=True, timeout=60).strip()


def identity(pid):
    raw = Path(f"/proc/{pid}/stat").read_text()
    return {"pid": pid, "start_ticks": int(raw.rsplit(") ", 1)[1].split()[19]),
            "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip()}


def execution_layout(source, output):
    """The two hosted VMs must expose identical canonical input paths."""
    workspace = Path(os.environ["GITHUB_WORKSPACE"])
    temporary = Path(os.environ["RUNNER_TEMP"])
    require(workspace.is_absolute() and workspace.resolve() == workspace
            and temporary.is_absolute() and temporary.resolve() == temporary
            and source == workspace / "product" and CONTROL == workspace / "control"
            and output == temporary / "final-runtime-prepared", "unexpected canonical cloud layout")
    return {"workspace": str(workspace), "runner_temp": str(temporary), "source": str(source),
            "control": str(CONTROL), "output": str(output), "uid": os.geteuid(),
            "machine": os.uname().machine, "python": str(Path(sys.executable).resolve())}


def command(argv, cwd, log, timeout, env=None):
    """One invocation; preserve failure and terminate only its owned group."""
    started = time.monotonic()
    result = {"argv": list(map(str, argv)), "cwd": str(cwd), "timeout_seconds": timeout,
              "exit_code": None, "timed_out": False, "result": "NOT_RUN"}
    with Path(log).open("xb") as output:
        process = subprocess.Popen(list(map(str, argv)), cwd=cwd, env=env,
                                   stdin=subprocess.DEVNULL, stdout=output,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            result["owner"] = identity(process.pid)
        except FileNotFoundError:
            result["owner"] = {"pid": process.pid, "generation": "EXITED_BEFORE_OBSERVATION"}
        try:
            result["exit_code"] = process.wait(timeout=timeout)
        except BaseException as error:
            result["timed_out"] = isinstance(error, subprocess.TimeoutExpired)
            result["error_type"] = type(error).__name__
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
            result["exit_code"] = process.returncode
            raise
        finally:
            result["seconds"] = time.monotonic() - started
            result["result"] = "PASS" if result["exit_code"] == 0 and "error_type" not in result else "FAIL"
            result["log_sha256"] = sha(log)
            record(Path(str(log) + ".command.json"), result)
    require(result["exit_code"] == 0, "command failed; inspect its private log and receipt")
    return Path(log).read_text(errors="replace")


def source_snapshot(source):
    require(git(source, "rev-parse", "HEAD") == RUNTIME
            and git(source, "rev-parse", "HEAD^{tree}") == RUNTIME_TREE,
            "runtime source checkout identity differs")
    require(git(source, "status", "--porcelain=v1", "--untracked-files=all") == "",
            "runtime source is not clean")
    entries = subprocess.check_output(["git", "-C", str(source), "ls-tree", "-rz", "HEAD"], timeout=60)
    files, links = {}, {}
    for item in entries.split(b"\0"):
        if not item:
            continue
        definition, raw_name = item.split(b"\t", 1)
        mode, kind, blob = definition.decode().split()
        name = raw_name.decode()
        if kind == "commit":
            links[name] = blob
            continue
        require(kind == "blob" and mode in {"100644", "100755"}, "unexpected source Git mode")
        path = regular(source / name)
        data = path.read_bytes()
        require(hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest() == blob,
                "checked-out source bytes differ from immutable Git object: " + name)
        require(bool(path.stat().st_mode & 0o111) == (mode == "100755"), "source executable mode differs: " + name)
        files[name] = {"git_blob": blob, "sha256": hashlib.sha256(data).hexdigest(), "mode": mode}
    require(len(files) + len(links) == 5581 and len(links) == 20
            and links["third_party/extore"] == EXTORE, "fixed runtime tree inventory differs")
    extore = source / "third_party/extore"
    require(git(extore, "rev-parse", "HEAD") == EXTORE
            and git(extore, "status", "--porcelain=v1", "--untracked-files=all") == "",
            "fixed Extore fixture source differs")
    extore_files = {}
    for item in subprocess.check_output(["git", "-C", str(extore), "ls-tree", "-rz", "HEAD"], timeout=60).split(b"\0"):
        if not item:
            continue
        definition, raw_name = item.split(b"\t", 1)
        mode, kind, blob = definition.decode().split()
        name = raw_name.decode()
        require(kind == "blob" and mode in {"100644", "100755"}, "unexpected Extore tracked input")
        path = regular(extore / name)
        data = path.read_bytes()
        require(hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest() == blob
                and bool(path.stat().st_mode & 0o111) == (mode == "100755"), "Extore bytes or Git mode differ: " + name)
        extore_files[name] = {"git_blob": blob, "sha256": hashlib.sha256(data).hexdigest(), "mode": mode}
    return {"revision": RUNTIME, "tree": RUNTIME_TREE, "files": files, "gitlinks": links,
            "extore": {"revision": EXTORE, "tree": git(extore, "rev-parse", "HEAD^{tree}"), "files": extore_files}}


def fetch_json(endpoint, target):
    with target.open("xb") as output:
        subprocess.run(["gh", "api", "--method", "GET", endpoint],
                       stdout=output, check=True, timeout=90)
    return json.loads(target.read_text())


def download_artifact(root, artifact_id, size, digest, revision, run_id, name=None):
    """One metadata request and one binary GET; an existing claim prevents replay."""
    private(root)
    record(root / "download-claim.json", {"artifact_id": artifact_id, "expected_size": size,
           "expected_sha256": digest, "source_revision": revision, "run_id": run_id,
           "created_utc": datetime.now(timezone.utc).isoformat(), "attempts_allowed": 1})
    meta = fetch_json(f"repos/{REPOSITORY}/actions/artifacts/{artifact_id}", root / "metadata.json")
    require(meta["id"] == artifact_id and meta["expired"] is False
            and 0 < meta["size_in_bytes"] <= 1024**3
            and (size is None or meta["size_in_bytes"] == size) and meta["digest"] == "sha256:" + digest
            and meta["workflow_run"]["id"] == run_id
            and meta["workflow_run"]["head_sha"] == revision
            and (name is None or meta["name"] == name), "artifact API binding differs")
    size = meta["size_in_bytes"]
    help_text = subprocess.check_output(["gh", "api", "--help"], text=True, timeout=30)
    argv = ["gh", "api", "--method", "GET", f"repos/{REPOSITORY}/actions/artifacts/{artifact_id}/zip"]
    if "--allow-escape-sequences" in help_text:
        argv.append("--allow-escape-sequences")
    archive = root / "artifact.zip"
    with archive.open("xb") as output:
        subprocess.run(argv, stdout=output, check=True, timeout=600)
    require(archive.stat().st_size == size and sha(archive) == digest, "actual artifact ZIP differs")
    return archive


def unzip(archive, output, names, maximum_file=256 * 1024**2):
    private(output)
    with zipfile.ZipFile(archive) as zipped:
        entries = zipped.infolist()
        require(len(entries) == len(names) and {entry.filename for entry in entries} == set(names),
                "ZIP inventory differs")
        for entry in entries:
            mode = entry.external_attr >> 16
            require(not entry.is_dir() and PurePosixPath(entry.filename).name == entry.filename
                    and stat.S_IFMT(mode) in {0, stat.S_IFREG}
                    and 0 < entry.file_size <= maximum_file, "unsafe ZIP member")
            with zipped.open(entry) as src, (output / entry.filename).open("xb") as dst:
                shutil.copyfileobj(src, dst)


def official_components(source, root):
    private(root)
    run = fetch_json(f"repos/{REPOSITORY}/actions/runs/{DISTRIBUTION_RUN}", root / "run.json")
    jobs = fetch_json(f"repos/{REPOSITORY}/actions/runs/{DISTRIBUTION_RUN}/jobs?per_page=100", root / "jobs.json")
    require(run["head_sha"] == RUNTIME and run["run_attempt"] == 1
            and run["event"] == "workflow_dispatch" and run["conclusion"] == "success"
            and len(jobs["jobs"]) == 5 and all(j["conclusion"] == "success" for j in jobs["jobs"]),
            "official distribution was not the complete successful fixed run")
    distribution = load_module("fixed_final_distribution", source / "scripts/build-distribution.py")
    result = {}
    for component, (artifact, size, digest, archive_name, archive_sha) in COMPONENTS.items():
        d = root / component
        variant = "-extore" if component == "extensions" else ""
        zipped = download_artifact(d, artifact, size, digest, RUNTIME, DISTRIBUTION_RUN,
                                  f"lmm-preview-{component}-{RUNTIME}{variant}")
        unzip(zipped, d / "zip-content", {archive_name, archive_name + ".sha256"})
        package = d / "zip-content" / archive_name
        require(sha(package) == archive_sha and
                (d / "zip-content" / (archive_name + ".sha256")).read_text().split()
                == [archive_sha, archive_name], "component archive or sidecar differs")
        manifest = distribution.verify(package, RUNTIME)
        require(manifest["component"] == component and manifest["version"] == "0.1.0-preview"
                and manifest["preview"] is True and manifest["source_date_epoch"] == 1795200983,
                "original component manifest identity differs")
        content = private(d / "content")
        with tarfile.open(package, "r:gz") as tar:
            for entry in tar:
                # The unchanged original verifier rejects directories, links,
                # duplicates, modes, unknown fields and noncanonical names.
                destination = content / entry.name
                destination.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
                with tar.extractfile(entry) as src, destination.open("xb") as dst:
                    shutil.copyfileobj(src, dst)
                destination.chmod(entry.mode)
        inventory = {str(p.relative_to(content)): {"sha256": sha(p), "size": p.stat().st_size,
                      "mode": stat.S_IMODE(p.stat().st_mode)} for p in sorted(content.rglob("*")) if p.is_file()}
        record(d / "verified.json", {"result": "VERIFIED_ARTIFACT_NOT_ACCEPTANCE", "source_revision": RUNTIME,
               "artifact_id": artifact, "zip_sha256": digest, "archive_sha256": archive_sha,
               "manifest_sha256": sha(content / "MANIFEST.json"), "manifest": manifest,
               "files": inventory})
        result[component] = content
    return result


def commerce_driver(source, root, snapshot):
    archive = download_artifact(root, *COMMERCE_ARTIFACT, COMMERCE_REVISION, COMMERCE_RUN)
    content = root / "content"
    unzip(archive, content, {"commerce_process", "receipt.json", "compiler-artifact.json", "source-inputs.json"})
    receipt = json.loads((content / "receipt.json").read_text())
    compiler = json.loads((content / "compiler-artifact.json").read_text())
    inputs = json.loads((content / "source-inputs.json").read_text())
    names = subprocess.check_output(["git", "-C", str(source), "ls-files", "-z", "--",
        "services/core", "tests/commerce", "deployment/postgres", "contracts/proto",
        "services/extensions/internal/modules/store/schema.sql"], timeout=60).decode().split("\0")
    expected = {name: snapshot["files"][name]["sha256"] for name in names if name}
    require(len(inputs) == 1131 and inputs == expected, "952 compiler input closure differs from runtime293")
    require(receipt["format"] == "lmm-commerce-ci-driver-v1" and receipt["revision"] == COMMERCE_REVISION
            and receipt["source_tree"] == COMMERCE_TREE and str(receipt["run_id"]) == str(COMMERCE_RUN)
            and str(receipt["run_attempt"]) == "1" and receipt["binary"] == "commerce_process"
            and receipt["binary_sha256"] == sha(content / "commerce_process") == COMMERCE_SHA
            and receipt["source_inputs_sha256"] == sha(content / "source-inputs.json")
            and receipt["compiler_artifact_sha256"] == sha(content / "compiler-artifact.json"),
            "commerce driver receipt differs from actual bytes")
    require(compiler["reason"] == "compiler-artifact" and compiler["target"]["name"] == "commerce_process"
            and compiler["target"]["kind"] == ["test"] and compiler["executable"] == receipt["compiler_executable"]
            and compiler["profile"] == receipt["cargo_profile"]
            and receipt["cargo_profile"] == {"debug_assertions": True, "debuginfo": 0, "opt_level": "0",
                 "overflow_checks": True, "test": True}
            and "\nrelease: 1.100.0\n" in receipt["rustc_verbose"]
            and "\nhost: x86_64-unknown-linux-gnu\n" in receipt["rustc_verbose"],
            "commerce actual compiler identity differs")
    (content / "commerce_process").chmod(0o500)
    record(root / "verified.json", {"result": "VERIFIED_COMPILATION_ONLY", "source_revision": COMMERCE_REVISION,
           "runtime_revision": RUNTIME, "compile_inputs_count": len(inputs), "compile_inputs_equal": True,
           "runtime_sources_checked_separately": True, "binary_sha256": COMMERCE_SHA,
           "receipt_sha256": sha(content / "receipt.json")})
    return content / "commerce_process"


def input_modules(source):
    sys.path.insert(0, str(source / "scripts"))
    sys.path.insert(0, str(source / "tests/commerce"))
    resource = load_module("verify_model_resources", source / "scripts/verify_model_resources.py")
    commerce = load_module("final_commerce", source / "tests/commerce/verify_process.py")
    payment = load_module("final_payment", source / "tests/payments/verify_process.py")
    lease = load_module("final_build_lease", source / "tests/architecture_completion/build_lease.py")
    return resource, commerce, payment, lease


def copy_binary(source, target):
    regular(source)
    with Path(source).open("rb") as src, target.open("xb") as dst:
        shutil.copyfileobj(src, dst)
    target.chmod(0o500)
    require(sha(source) == sha(target), "binary changed while sealing")


def one_binary(root, name, digest):
    files = list(root.rglob(name))
    require(len(files) == 1 and sha(regular(files[0])) == digest, "official executable identity differs: " + name)
    return files[0]


def compiler_executable(log, target_name, source, target):
    messages = []
    for line in log.read_text().splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (value.get("reason") == "compiler-artifact" and value.get("executable")
                and value.get("target", {}).get("name") == target_name
                and value["target"]["kind"] == ["test"]):
            messages.append(value)
    require(len(messages) == 1, "compile did not produce exactly the selected test executable")
    path = regular(Path(messages[0]["executable"]).resolve())
    require(path.is_relative_to(target) and Path(messages[0]["target"]["src_path"]).is_relative_to(source),
            "compiler artifact escaped its fixed source/target")
    return path, messages[0]


def package_payload(payload, output, layout):
    files = {}
    for path in sorted(payload.rglob("*")):
        require(not path.is_symlink(), "prepared payload contains a symlink")
        relative = path.relative_to(payload)
        if "zip-content" in relative.parts or "compile-tmp" in relative.parts or path.name == "artifact.zip":
            continue
        if path.is_file():
            files[str(relative)] = {"sha256": sha(path), "size": path.stat().st_size,
                                    "mode": stat.S_IMODE(path.stat().st_mode)}
    require(len(files) <= 10000 and sum(v["size"] for v in files.values()) <= 2 * 1024**3,
            "prepared payload exceeds the bounded transport envelope")
    manifest = output / "export-manifest.json"
    record(manifest, {"format": "lmm-final-runtime-prepared-export-v1", "runtime_revision": RUNTIME,
           "control_revision": os.environ["GITHUB_SHA"], "cloud_run": os.environ["GITHUB_RUN_ID"],
           "cloud_run_attempt": os.environ["GITHUB_RUN_ATTEMPT"], "acceptance_executed": False,
           "execution_layout": layout, "files": files})
    bundle = output / "runtime-prepared.tar.gz"
    with tarfile.open(bundle, "w:gz", compresslevel=1) as archive:
        archive.add(manifest, arcname="export-manifest.json", recursive=False)
        for name in files:
            archive.add(payload / name, arcname=name, recursive=False)
    record(output / "bundle.json", {"sha256": sha(bundle), "size": bundle.stat().st_size,
           "manifest_sha256": sha(manifest), "prepared_result": "PREPARED_NOT_ACCEPTANCE"})
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        stream.write(f"bundle_sha256={sha(bundle)}\nmanifest_sha256={sha(manifest)}\n")


def acquire(args):
    """The only phase given GH_TOKEN; no compilation or product execution."""
    private(args.output)
    payload = private(args.output / "payload")
    snapshot = source_snapshot(args.source)
    layout = execution_layout(args.source, args.output)
    record(payload / "source-before.json", snapshot)
    official_components(args.source, payload / "official")
    commerce_driver(args.source, payload / "commerce", snapshot)
    require(source_snapshot(args.source) == snapshot, "source drifted while acquiring inputs")
    record(payload / "acquired.json", {"result": "VERIFIED_INPUTS_NOT_ACCEPTANCE", "runtime_revision": RUNTIME,
           "control_revision": os.environ["GITHUB_SHA"], "controller_sha256": sha(Path(__file__).resolve()),
           "source_before_sha256": sha(payload / "source-before.json"), "acceptance_executed": False,
           "execution_layout": layout, "commerce_files": {p.name: sha(p) for p in (payload / "commerce/content").iterdir()}})


def verified_component_paths(source, payload):
    distribution = load_module("recheck_fixed_distribution", source / "scripts/build-distribution.py")
    paths = {}
    for component, definition in COMPONENTS.items():
        d = payload / "official" / component
        content = d / "content"
        reviewed = json.loads((d / "verified.json").read_text())
        require(reviewed["source_revision"] == RUNTIME and reviewed["archive_sha256"] == definition[4],
                "stored official archive binding differs")
        actual_names = {str(p.relative_to(content)) for p in content.rglob("*") if p.is_file()}
        require(actual_names == set(reviewed["files"]), "component inventory drifted")
        for name, entry in reviewed["files"].items():
            path = regular(content / name)
            require(sha(path) == entry["sha256"] and path.stat().st_size == entry["size"]
                    and stat.S_IMODE(path.stat().st_mode) == entry["mode"], "component bytes or modes drifted")
        paths[component] = content
    # Load the unchanged verifier above as part of the source binding, even when
    # run-job transport intentionally omits the redundant original tarballs.
    require(distribution.ROOT == source, "wrong original archive verifier source")
    return paths


def prepare(args):
    source, output = args.source, args.output
    require(not os.environ.get("GH_TOKEN") and not os.environ.get("GITHUB_TOKEN"),
            "download token must not enter the prepare step")
    payload = output / "payload"
    receipt = {"format": "lmm-final-runtime-cloud-preparation-v1", "result": "NOT_RUN",
               "acceptance_executed": False, "runtime_revision": RUNTIME,
               "control_revision": os.environ["GITHUB_SHA"], "commerce_revision": COMMERCE_REVISION}
    started = time.monotonic()
    try:
        snapshot = source_snapshot(source)
        acquired = json.loads((payload / "acquired.json").read_text())
        require(acquired["result"] == "VERIFIED_INPUTS_NOT_ACCEPTANCE"
                and acquired["runtime_revision"] == RUNTIME
                and acquired["control_revision"] == os.environ["GITHUB_SHA"]
                and acquired["controller_sha256"] == sha(Path(__file__).resolve())
                and acquired["source_before_sha256"] == sha(payload / "source-before.json")
                and acquired["execution_layout"] == execution_layout(source, output)
                and acquired["commerce_files"] == {p.name: sha(p) for p in (payload / "commerce/content").iterdir()}
                and json.loads((payload / "source-before.json").read_text()) == snapshot,
                "acquired source/control binding differs")
        resource, commerce, payment, lease = input_modules(source)
        resource_inputs = resource.inputs()
        runtime_inputs = commerce.sources()
        payment_inputs = payment.sources()
        record(payload / "resource-inputs-before.json", resource_inputs)
        record(payload / "commerce-runtime-inputs.json", runtime_inputs)
        record(payload / "payment-compiled-inputs.json", payment_inputs)
        components = verified_component_paths(source, payload)
        driver = regular(payload / "commerce/content/commerce_process")
        require(sha(driver) == COMMERCE_SHA, "imported compiled commerce executable drifted")
        core = one_binary(components["core"], "lmm-core", CORE_SHA)
        one_binary(components["extensions"], "lmm-extensions", GO_SHA)
        one_binary(components["extensions"], "lmm-extore-host", HOST_SHA)
        one_binary(components["extensions"], "libextore_zplugin.so", PLUGIN_SHA)
        one_binary(components["cli"], "lmm", CLI_SHA)
        binding = payload / "core-source-binding.json"
        record(binding, {"files": resource_inputs["product"]})
        official_jobs = json.loads((payload / "official/jobs.json").read_text())["jobs"]
        core_jobs = [job for job in official_jobs if job["name"].lower() == "core"]
        require(len(core_jobs) == 1, "official Core job is not unique")
        reference = payload / "official-core-build-reference.json"
        record(reference, {"format": "github-release-build-reference-v1",
            "reference_kind": "Verified existing official GitHub release-profile artifact; not a new local product build",
            "exit_code": 0, "workflow_run": DISTRIBUTION_RUN, "run_attempt": 1,
            "job_id": core_jobs[0]["id"], "workflow_url": f"https://github.com/{REPOSITORY}/actions/runs/{DISTRIBUTION_RUN}",
            "source_revision": RUNTIME, "source_tree": RUNTIME_TREE, "binary_path": str(core),
            "binary_sha256": CORE_SHA, "archive_sha256": COMPONENTS["core"][4],
            "source_binding_sha256": sha(binding), "source_drift": [],
            "source_drift_basis": "Clean fixed product closure and verified official artifact revision; reference validation, not observation of a new local product build",
            "evidence": {str(p.relative_to(payload)): sha(p) for p in sorted((payload / "official").rglob("*.json"))
                         if "content" not in p.parts and "zip-content" not in p.parts}})
        tools = private(payload / "toolchain")
        environment = resource.env()
        mise = command(["mise", "--version"], source, tools / "mise.log", 30, environment)
        require(mise.split()[0] == "2026.11.10", "actual mise version differs")
        cargo = regular(Path(subprocess.check_output(["rustup", "which", "--toolchain", "1.100.0", "cargo"],
                                  env=environment, text=True, timeout=30).strip()).resolve())
        rustc = regular(cargo.parent / "rustc")
        version = command([rustc, "-vV"], source, tools / "rustc.log", 30, environment)
        require("\nrelease: 1.100.0\n" in version, "actual Rust version differs")
        command([cargo, "--version"], source, tools / "cargo.log", 30, environment)
        command(["mise", "ls", "--json"], source, tools / "mise-installed.log", 30, environment)
        record(tools / "binaries.json", {"cargo": {"path": str(cargo), "sha256": sha(cargo)},
               "rustc": {"path": str(rustc), "sha256": sha(rustc)}})
        target = source / "services/core/target"
        if not target.exists():
            target.mkdir(mode=0o700)
        require(target.resolve() == target and target.is_dir() and target.stat().st_uid == os.geteuid()
                and not target.stat().st_mode & 0o022, "unsafe original standard Cargo target")
        lease_parent = private(output / "build-lease")
        lease.initialize(lease_parent / "resource")
        private(lease_parent / "requests")
        prepared = private(payload / "core-prepared")
        argv = [sys.executable, "-B", source / "scripts/verify_model_resources.py", "prepare",
                "--output", prepared, "--resource", lease_parent / "resource",
                "--requests", lease_parent / "requests", "--core-binary", core,
                "--source-binding", binding, "--build-receipt", reference]
        # The unchanged original prepare retains its 1800s compile/1200s lease limits.
        command(argv, source, payload / "resource-prepare.log", 1900)
        prepared_data = json.loads((prepared / "prepared.json").read_text())
        require(prepared_data["format"] == "lmm-resource-prepared-v1"
                and prepared_data["inputs"] == resource_inputs
                and prepared_data["source_binding_sha256"] == sha(binding)
                and prepared_data["build_receipt_sha256"] == sha(reference)
                and prepared_data["artifacts"]["artifacts/lmm-core"] == CORE_SHA,
                "actual original resource prepared receipt differs")
        resource.artifacts_check(prepared, prepared_data["artifacts"])
        resource.reviewed_tests(prepared_data["test_list"])
        finished = list((lease_parent / "requests").glob("*/finished.json"))
        require(len(finished) == 1 and json.loads(finished[0].read_text())["exit_code"] == 0
                and (finished[0].parent / "released.json").is_file(), "original prepare lease did not release")
        saved_lease = private(payload / "build-lease-receipts")
        for p in sorted(lease_parent.rglob("*.json")):
            q = saved_lease / p.relative_to(lease_parent)
            q.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            shutil.copyfile(p, q)
        payments = private(payload / "payments")
        compile_env = {**os.environ, "RUSTUP_TOOLCHAIN": "1.100.0", "RUSTUP_AUTO_INSTALL": "0",
                       "RUSTC": str(rustc), "CARGO_NET_OFFLINE": "true", "SQLX_OFFLINE": "true",
                       "CARGO_BUILD_JOBS": "2", "CARGO_INCREMENTAL": "0",
                       "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0"}
        compile_env.pop("CARGO_TARGET_DIR", None)
        command([cargo, "test", "--locked", "--offline", "--test", "payments_process", "--no-run",
                 "--message-format=json"], source / "services/core", payments / "compile.jsonl", 1800, compile_env)
        executable, compiler = compiler_executable(payments / "compile.jsonl", "payments_process", source, target)
        copy_binary(executable, payments / "payments_process")
        record(payments / "compiler-artifact.json", compiler)
        command(["mise", "exec", "--", "go", "test", "-c", "-o", payments / "bootstrap",
                 "./internal/modules/payments"], source / "services/extensions", payments / "go-compile.log", 600,
                {**os.environ, "GOWORK": "off", "GOFLAGS": "-mod=readonly"})
        (payments / "bootstrap").chmod(0o500)
        full = command([payments / "payments_process", "--list"], source / "services/core", payments / "test-list.log", 10)
        ignored = command([payments / "payments_process", "--list", "--ignored"], source / "services/core", payments / "ignored-test-list.log", 10)
        payment.verify_discovery(full, ignored)
        bootstrap_list = command([payments / "bootstrap", "-test.list", "^TestRustDurableProofAcrossGoRestart$"],
                                 source / "services/extensions", payments / "bootstrap-list.log", 10)
        require(bootstrap_list.strip() == "TestRustDurableProofAcrossGoRestart", "bootstrap listing differs")
        fixtures = private(payload / "fixtures")
        extore_target = private(output / "extore-build-target")
        command(["mise", "exec", "--", "cargo", "build", "--locked", "--manifest-path",
                 source / "third_party/extore/Cargo.toml", "--package", "extore"], source,
                fixtures / "extore-compile.log", 1800,
                {**os.environ, "CARGO_TARGET_DIR": str(extore_target), "CARGO_BUILD_JOBS": "2",
                 "CARGO_INCREMENTAL": "0", "CARGO_PROFILE_DEV_DEBUG": "0"})
        copy_binary(extore_target / "debug/extore", fixtures / "extore")
        helper = source / "third_party/extore/scripts/native_plugin_fixture.py"
        require(sha(helper) == "fd44a30f6b8f50f6588098a81549802c19518a8f0f58ad6e5b80c1515571875e",
                "fixed8e fixture helper differs")
        static = source / "third_party/extore/extore/static"
        static_map = {str(p.relative_to(static)): sha(regular(p)) for p in sorted(static.rglob("*")) if p.is_file()}
        require(len(static_map) == 57, "fixed Extore static inventory differs")
        record(fixtures / "extore-static-binding.json", {"source_commit": EXTORE,
               "source_tree": git(source / "third_party/extore", "rev-parse", "HEAD^{tree}"), "files": static_map})
        record(fixtures / "fixture-tools.json", {"extore_source_commit": EXTORE,
               "extore_binary_sha256": sha(fixtures / "extore"), "helper_sha256": sha(helper),
               "scope": "Fixed8e browser fixture service; not an official Extore release"})
        require(resource.inputs() == resource_inputs and commerce.sources() == runtime_inputs
                and payment.sources() == payment_inputs, "original driver/runtime inputs drifted during compilation")
        after = source_snapshot(source)
        require(after == snapshot, "full source checkout drifted during preparation")
        record(payload / "source-after.json", after)
        receipt.update(result="PREPARED_NOT_ACCEPTANCE", source_drift=[],
                       execution_layout=execution_layout(source, output),
                       preparation_generation={"job": os.environ["GITHUB_JOB"], "controller": identity(os.getpid()),
                                               "role": "Historical preparation only; never a run-job ownership claim"},
                       prepared_sha256=sha(prepared / "prepared.json"), commerce_sha256=sha(driver),
                       payment_sha256=sha(payments / "payments_process"), bootstrap_sha256=sha(payments / "bootstrap"),
                       extore_fixture_sha256=sha(fixtures / "extore"))
        record(payload / "preparation.json", receipt)
        package_payload(payload, output, execution_layout(source, output))
    except BaseException as error:
        receipt.update(result="FAIL", error_type=type(error).__name__, error=str(error))
        raise
    finally:
        receipt["seconds"] = time.monotonic() - started
        record(output / "preparation-result.json", receipt)


def restore(args):
    private(args.output)
    for digest in (args.artifact_sha256, args.bundle_sha256, args.manifest_sha256):
        require(re.fullmatch(r"[0-9a-f]{64}", digest) is not None, "missing prepared transport digest")
    run_id = int(os.environ["GITHUB_RUN_ID"])
    archive = download_artifact(args.output / "transport", args.artifact_id, None, args.artifact_sha256,
                                os.environ["GITHUB_SHA"], run_id, f"final-runtime-prepared-{run_id}")
    unpacked = args.output / "transport/files"
    unzip(archive, unpacked, {"runtime-prepared.tar.gz", "export-manifest.json", "bundle.json"}, 1024**3)
    bundle = unpacked / "runtime-prepared.tar.gz"
    manifest_file = unpacked / "export-manifest.json"
    require(sha(bundle) == args.bundle_sha256 and sha(manifest_file) == args.manifest_sha256,
            "same-run prepared transport bytes differ from prepare-job outputs")
    manifest = json.loads(manifest_file.read_text())
    require(manifest["format"] == "lmm-final-runtime-prepared-export-v1"
            and manifest["runtime_revision"] == RUNTIME
            and manifest["control_revision"] == os.environ["GITHUB_SHA"]
            and str(manifest["cloud_run"]) == str(run_id)
            and str(manifest["cloud_run_attempt"]) == os.environ["GITHUB_RUN_ATTEMPT"]
            and manifest["execution_layout"] == execution_layout(args.source, args.output)
            and manifest["acceptance_executed"] is False, "prepared source/run binding differs")
    expected = manifest["files"]
    require(isinstance(expected, dict) and 0 < len(expected) <= 10000
            and sum(item["size"] for item in expected.values()) <= 2 * 1024**3,
            "prepared export exceeds transport limits")
    payload = private(args.output / "payload")
    with tarfile.open(bundle, "r:gz") as tar:
        members = tar.getmembers()
        names = [item.name for item in members]
        require(len(names) == len(set(names)) and set(names) == set(expected) | {"export-manifest.json"},
                "prepared tar inventory differs")
        for item in members:
            name = PurePosixPath(item.name)
            require(item.isfile() and not item.issym() and not item.islnk() and not name.is_absolute()
                    and str(name) == item.name and "\\" not in item.name
                    and not any(part in {"", ".", ".."} for part in name.parts)
                    and not any(ord(c) < 32 for c in item.name), "unsafe prepared tar path or type")
            if item.name == "export-manifest.json":
                require(item.size == manifest_file.stat().st_size
                        and hashlib.sha256(tar.extractfile(item).read()).hexdigest() == args.manifest_sha256,
                        "inner prepared manifest differs")
                continue
            definition = expected[item.name]
            require(type(definition["size"]) is int and item.size == definition["size"]
                    and 0 <= item.size <= 512 * 1024**2 and item.mode == definition["mode"]
                    and item.mode in {0o400, 0o444, 0o500, 0o555, 0o600, 0o644, 0o700, 0o755},
                    "prepared tar size or mode differs")
            target = payload / item.name
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            with tar.extractfile(item) as src, target.open("xb") as dst:
                shutil.copyfileobj(src, dst)
            target.chmod(item.mode)
            require(sha(target) == definition["sha256"], "prepared file hash differs")
    require(source_snapshot(args.source) == json.loads((payload / "source-before.json").read_text()),
            "fresh runner source differs from prepared source closure")
    prepared = json.loads((payload / "core-prepared/prepared.json").read_text())
    original = json.loads((payload / "preparation.json").read_text())
    require(original["result"] == "PREPARED_NOT_ACCEPTANCE"
            and sha(payload / "core-prepared/prepared.json") == original["prepared_sha256"],
            "original prepared.json changed during transfer")
    resource, _, _, _ = input_modules(args.source)
    resource.check(prepared["inputs"])
    resource.artifacts_check(payload / "core-prepared", prepared["artifacts"])
    resource.reviewed_tests(prepared["test_list"])
    verified_component_paths(args.source, payload)
    record(args.output / "restore-result.json", {"result": "VERIFIED_ORIGINAL_PREPARED_NOT_ACCEPTANCE",
           "runtime_revision": RUNTIME, "control_revision": os.environ["GITHUB_SHA"],
           "bundle_sha256": args.bundle_sha256, "manifest_sha256": args.manifest_sha256,
           "original_prepared_sha256": original["prepared_sha256"], "file_count": len(expected),
           "prepared_bytes_modified": False, "product_executed": False,
           "execution_layout": execution_layout(args.source, args.output),
           "preparation_generation": original["preparation_generation"],
           "run_job_generation": {"job": os.environ["GITHUB_JOB"], "controller": identity(os.getpid())},
           "old_lease_reused": False, "old_pid_used_for_cleanup": False})


def short_root(label):
    path = Path(tempfile.mkdtemp(prefix="lf-" + label + "-", dir="/tmp"))
    path.chmod(0o700)
    require(len(os.fsencode(path)) <= 35, "original process root length exceeded")
    return path


def verify_compiled_inputs(payload):
    original = json.loads((payload / "preparation.json").read_text())
    require(original["result"] == "PREPARED_NOT_ACCEPTANCE" and original["runtime_revision"] == RUNTIME,
            "missing original successful preparation")
    for name, expected in [("core-prepared/prepared.json", original["prepared_sha256"]),
                           ("commerce/content/commerce_process", COMMERCE_SHA),
                           ("payments/payments_process", original["payment_sha256"]),
                           ("payments/bootstrap", original["bootstrap_sha256"]),
                           ("fixtures/extore", original["extore_fixture_sha256"])]:
        require(sha(regular(payload / name)) == expected, "sealed driver/fixture bytes drifted: " + name)


def unique_json(root, name):
    paths = list(root.rglob(name))
    require(len(paths) == 1, "expected one original receipt: " + name)
    return paths[0], json.loads(paths[0].read_text())


def browser_candidate(source, payload, evidence):
    verify_compiled_inputs(payload)
    components = verified_component_paths(source, payload)
    fixtures = payload / "fixtures"
    dist = components["web"] / "dist"
    file_map = {str(p.relative_to(dist)): sha(regular(p)) for p in sorted(dist.rglob("*")) if p.is_file()}
    require("index.html" in file_map, "official Web index missing")
    web_binding = evidence / "web-binding.json"
    embedded = [name for name in file_map if name.endswith(".js") and RUNTIME.encode() in (dist / name).read_bytes()]
    record(web_binding, {"source_commit": RUNTIME, "embedded_build_revision": RUNTIME if embedded else None,
           "embedded_revision_files": embedded, "revision_binding": "Verified official archive MANIFEST.json",
           "official_manifest_sha256": sha(components["web"] / "MANIFEST.json"), "files": file_map})
    node = regular(Path(os.environ["LMM_ACCEPTANCE_NODE"]).resolve())
    playwright = regular(Path(os.environ["LMM_ACCEPTANCE_PLAYWRIGHT"]).resolve())
    package = json.loads((playwright.parent / "package.json").read_text())
    require(package["version"] == "1.65.1", "browser automation version differs")
    query = "import {pathToFileURL} from 'node:url';const p=await import(pathToFileURL(process.argv[1]).href);console.log(p.chromium.executablePath())"
    chromium = Path(subprocess.check_output([node, "--input-type=module", "-e", query, playwright], text=True, timeout=30).strip()).resolve()
    regular(chromium)
    node_version = subprocess.check_output([node, "--version"], text=True, timeout=30).strip()
    require(node_version == "v26.10.0", "original root-pinned Node differs")
    chrome_version = subprocess.check_output([chromium, "--version"], text=True, timeout=30).strip()
    record(evidence / "browser-tools.json", {"node": {"path": str(node), "sha256": sha(node), "version": node_version},
           "playwright": {"path": str(playwright), "sha256": sha(playwright), "version": package["version"]},
           "chromium": {"path": str(chromium), "sha256": sha(chromium), "version": chrome_version},
           "qualification": "Observed cloud browser tools; no acceptance result"})
    binary = fixtures / "extore"
    helper = source / "third_party/extore/scripts/native_plugin_fixture.py"
    static_binding = fixtures / "extore-static-binding.json"
    candidate = {"source": str(source), "source_repository": str(source), "source_commit": RUNTIME,
        "core": {"path": str(one_binary(components["core"], "lmm-core", CORE_SHA)), "sha256": CORE_SHA,
                 "source_commit": RUNTIME, "profile": "official distribution release"},
        "extensions": {"path": str(one_binary(components["extensions"], "lmm-extensions", GO_SHA)),
                       "sha256": GO_SHA, "source_commit": RUNTIME},
        "web": {"dist": str(dist), "binding": str(web_binding), "binding_sha256": sha(web_binding), "source_commit": RUNTIME},
        "extore": {"source_commit": EXTORE, "source_repository": str(source / "third_party/extore"),
            "binary": {"path": str(binary), "sha256": sha(binary), "compiled_source_commit": EXTORE},
            "library": {"path": str(one_binary(components["extensions"], "libextore_zplugin.so", PLUGIN_SHA)), "sha256": PLUGIN_SHA},
            "helper": {"path": str(helper), "sha256": sha(helper)},
            "static": {"root": str(source / "third_party/extore/extore/static"), "binding": str(static_binding),
                       "binding_sha256": sha(static_binding)}},
        "extore_host": {"path": str(one_binary(components["extensions"], "lmm-extore-host", HOST_SHA)),
            "sha256": HOST_SHA, "source_commit": RUNTIME, "extore_source_commit": EXTORE,
            "compiled_extore_source_commit": EXTORE},
        "fixture_overlays": {}, "test_harness_commit": os.environ["GITHUB_SHA"],
        "validation_class": "Official293 Core/Go/Web/native pair; fixed8e real fixture service; original16case browser projection",
        "official_distribution": {"run_id": DISTRIBUTION_RUN, "attempt": 1, "version": "0.1.0-preview"}}
    record(evidence / "browser-candidate.json", candidate)
    return evidence / "browser-candidate.json", node, chromium, playwright


def execute_sequence(args):
    """Only called under the original cloud-private heavy lease."""
    require(not os.environ.get("GH_TOKEN") and not os.environ.get("GITHUB_TOKEN"), "download token reached runtime gates")
    source, payload, evidence = args.source, args.output / "payload", args.evidence
    verify_compiled_inputs(payload)
    snapshot = source_snapshot(source)
    require(snapshot == json.loads((payload / "source-before.json").read_text()), "runtime source closure differs")
    components = verified_component_paths(source, payload)
    resource, commerce, payment, _ = input_modules(source)
    require(commerce.sources() == json.loads((payload / "commerce-runtime-inputs.json").read_text())
            and payment.sources() == json.loads((payload / "payment-compiled-inputs.json").read_text()),
            "runtime input closure differs from prepared drivers")
    core = one_binary(components["core"], "lmm-core", CORE_SHA)
    go = one_binary(components["extensions"], "lmm-extensions", GO_SHA)
    driver = payload / "commerce/content/commerce_process"
    require(sha(driver) == COMMERCE_SHA, "commerce executable drifted before execution")
    local = source / ".local-tests"
    if not local.exists():
        private(local)
    case_root = private(local / "final293-original-gates")
    result = {"format": "lmm-final-runtime-original-sequence-v1", "result": "NOT_RUN", "runtime_revision": RUNTIME,
              "control_revision": os.environ["GITHUB_SHA"], "commerce_compile_revision": COMMERCE_REVISION,
              "cases": [], "failure": None, "first_failure_stops_sequence": True,
              "production_data_used": False, "production_deployment_authorized": False}
    paths = []
    started = time.monotonic()
    shared_reference = None
    try:
        for name, flags, limit, pg_in_budget in [
            ("shared540", ["--shared-resources", "--mixed-load"], 1020, False),
            ("shared240", ["--shared-resources", "--mixed-slow-sockets"], 720, False),
            ("whole540", ["--shared-resources", "--mixed-load", "--whole-budget"], 1020, True),
            ("whole240", ["--shared-resources", "--mixed-slow-sockets", "--whole-budget"], 720, True),
        ]:
            output, proc = private(case_root / name), short_root(name)
            paths.extend([output, proc])
            argv = [sys.executable, "-B", source / "tests/commerce/verify_process.py",
                    "--output-root", output, "--process-root", proc,
                    "--core-binary", core, "--core-sha256", CORE_SHA,
                    "--extension-binary", go, "--extension-sha256", GO_SHA,
                    "--test-binary", driver, "--test-sha256", COMMERCE_SHA, *flags]
            record(evidence / (name + "-start.json"), {"case": name, "source_revision": RUNTIME,
                   "postgres_in_budget": pg_in_budget, "original_outer_seconds": limit,
                   "status": "RUNNING", "output_root": str(output), "process_root": str(proc)})
            command(argv, source, evidence / (name + ".private.log"), limit)
            path, value = unique_json(output, "acceptance.json")
            require(value["result"] == "PASS" and value["cluster_removed"] is True
                    and value["external_core_sha256"] == CORE_SHA and value["external_test_sha256"] == COMMERCE_SHA
                    and value["extension_sha256"] == GO_SHA and value["whole_budget"] is pg_in_budget
                    and value["source_sha256"] == commerce.sources(), "original mixed acceptance or cleanup incomplete")
            result["cases"].append({"case": name, "result": "PASS", "receipt": str(path), "sha256": sha(path),
                                    "postgres_in_budget": pg_in_budget})
            if name == "shared540":
                shared_reference = path
        prepared_root = payload / "core-prepared"
        original_prepared = prepared_root / "prepared.json"
        prepared_hash = sha(original_prepared)
        prepared = json.loads(original_prepared.read_text())
        resource.check(prepared["inputs"])
        resource.artifacts_check(prepared_root, prepared["artifacts"])
        proc = short_root("core540")
        paths.extend([prepared_root, proc])
        command([sys.executable, "-B", source / "scripts/verify_model_resources.py", "run", "--output", prepared_root,
                 "--process-root", proc, "--prepared-sha256", prepared_hash, "--case", "sustained"],
                source, evidence / "core540.private.log", 1000)
        results = list(prepared_root.glob("run-*/result.json"))
        require(len(results) == 1, "missing original Core-only result")
        core_result = json.loads(results[0].read_text())
        require(core_result["result"] == "PASS" and core_result["cluster_removed"] is True
                and core_result["case"] == "sustained" and core_result["prepared_sha256"] == prepared_hash
                and sha(original_prepared) == prepared_hash, "original Core-only gate or cleanup incomplete")
        result["cases"].append({"case": "core540", "result": "PASS", "receipt": str(results[0]),
                                "sha256": sha(results[0]), "postgres_in_budget": False})
        output, proc = private(case_root / "payment302"), short_root("pay302")
        paths.extend([output, proc])
        payments = payload / "payments"
        argv = [sys.executable, "-B", source / "tests/payments/verify_process.py", "--case", "accepted-proof-expiry",
                "--output-root", output, "--process-root", proc, "--core-binding", payload / "core-source-binding.json",
                "--core-build-receipt", payload / "official-core-build-reference.json", "--go-reference", shared_reference,
                "--compiled-inputs", payload / "payment-compiled-inputs.json"]
        for kind, binary in [("core", core), ("go", go), ("test", payments / "payments_process"), ("bootstrap", payments / "bootstrap")]:
            argv += ["--" + kind + "-binary", binary, "--" + kind + "-sha256", sha(binary)]
        # Original selected_command retains 480s, including the genuine >=302s wait.
        command(argv, source, evidence / "payment302.private.log", 600)
        path, value = unique_json(output, "acceptance.json")
        require(value["result"] == "PASS" and value["case"] == "accepted-proof-expiry"
                and value["cluster_removed"] is True and value["full_payment_acceptance"] is False,
                "original payment expiry gate or cleanup incomplete")
        result["cases"].append({"case": "payment302", "result": "PASS", "receipt": str(path), "sha256": sha(path),
                                "scope": "Original synthetic accepted-proof expiry/restart case; not full payment qualification"})
        browser_root = private(evidence / "browser-rounds")
        candidate, node, chromium, playwright = browser_candidate(source, payload, evidence)
        browser_env = {**os.environ, "LMM_ACCEPTANCE_ROOT": str(browser_root), "LMM_ACCEPTANCE_NODE": str(node),
                       "LMM_ACCEPTANCE_CHROMIUM": str(chromium), "LMM_ACCEPTANCE_PLAYWRIGHT": str(playwright)}
        paths.append(browser_root)
        command([sys.executable, "-B", CONTROL / "tests/final-runtime/browser/run-round.py", "--round", "browser-final-293",
                 "--candidate", candidate], source, evidence / "browser16.private.log", 1800, browser_env)
        browser_result = browser_root / "browser-final-293/browser/result.json"
        launcher_result = browser_root / "browser-final-293-launcher/result.json"
        value = json.loads(browser_result.read_text())
        launcher = json.loads(launcher_result.read_text())
        require(value["overall"] == "PASS" and len(value["cases"]) == 16
                and all(status == "PASS" for status in value["cases"].values())
                and value["page_errors"] == [] and launcher["overall"] == "PASS"
                and launcher["surviving_registered_pids"] == [] and launcher["fixture_cleanup"]["all_stopped"] is True,
                "original full browser gates or cleanup incomplete")
        result["cases"].append({"case": "browser16", "result": "PASS", "receipt": str(browser_result),
                                "sha256": sha(browser_result), "launcher_receipt_sha256": sha(launcher_result)})
        require(source_snapshot(source) == snapshot, "runtime source drift after gates")
        verified_component_paths(source, payload)
        verify_compiled_inputs(payload)
        result.update(result="PASS", source_drift=[], all_seven_original_gates_executed=True)
    except BaseException as error:
        result.update(result="FAIL", failure={"type": type(error).__name__, "message": str(error)},
                      all_seven_original_gates_executed=False)
        raise
    finally:
        result["seconds"] = time.monotonic() - started
        try:
            after = source_snapshot(source)
            result["source_unchanged_after_attempt"] = after == snapshot
            record(evidence / "source-after-attempt.json", after)
            require(after == snapshot, "source changed after the attempt")
            verify_compiled_inputs(payload)
            result["compiled_inputs_unchanged_after_attempt"] = True
        except Exception as error:
            result["after_attempt_check_error"] = type(error).__name__
            result["result"] = "FAIL"
        result["private_evidence_roots"] = [str(path) for path in paths]
        result["failure_retention_boundary"] = "Original failed fixtures are not success-cleaned. This disposable runner does not provide a durable PostgreSQL backup."
        record(evidence / "sequence-result.json", result)


def export_evidence(args):
    """Export original non-secret receipts; never databases/config/credential files."""
    evidence, payload = args.evidence, args.output / "payload"
    destination = private(evidence / "public")
    sequence_file = evidence / "sequence-result.json"
    sequence = json.loads(sequence_file.read_text()) if sequence_file.is_file() else {}
    roots = [("controller", evidence), ("prepared", payload / "core-prepared"),
             ("runtime-lease", evidence / "runtime-lease"),
             ("cases", args.source / ".local-tests/final293-original-gates")]
    for index, raw in enumerate(sequence.get("private_evidence_roots", [])):
        p = Path(raw)
        if p.parent == Path("/tmp") and p.name.startswith("lf-"):
            roots.append(("process-" + str(index), p))
    allowed = {"result.json", "acceptance.json", "inputs.json", "input-binding.json", "prepared.json", "source-after-attempt.json",
               "sequence-result.json", "run-result.json", "runtime-binding.json", "browser-tools.json",
               "web-binding.json", "browser-candidate.json", "failure.json", "running.json",
               "mixed-result.json", "joint-socket-result.json", "joint-socket-offered.json",
               "joint-socket-retained-recovery.json", "resource-result.json", "samples.json",
               "resource-samples.json", "group-allocation.json", "group-start.json", "group-ready.json", "group-closed.json",
               "whole-budget-allocation.json", "whole-budget-start.json", "whole-budget-ready.json", "whole-budget-closed.json",
               "resource.json", "requested.json", "acquired.json", "supervising.json", "running.json",
               "finished.json", "released.json", "interrupted.json", "supervisor-failed.json",
               "artifact-recheck.json", "fixture-cleanup.json", "fixture-ready.json", "native-host-start.json",
               "transport-preflight.json", "community-install-receipt.json", "provenance.json",
               "main-recovered.json", "bootstrap-replay-recovered.json", "expiry-before-restart.json",
               "new-products-ready-before-first-credit.json", "mixed-runtime-drain.json",
               "mixed-restart-classification.json", "backlog-classification-plan.json",
               "backlog-classification-result.json"}
    private_parts = {"private", "state-private", "fixture-evidence", "compile-tmp", "artifacts", "public"}
    safe_prefixes = ("resource-unit-", "resource-identity-", "scope-", "expiry-observation-", "resources-")
    files, withheld, seen = [], [], set()
    total = 0
    for label, root in roots:
        if not root.is_dir():
            continue
        for directory, dirs, names in os.walk(root, followlinks=False):
            parent = Path(directory)
            dirs[:] = [name for name in dirs if name not in private_parts and not (parent / name).is_symlink()]
            for name in sorted(names):
                path = parent / name
                if path.is_symlink() or path in seen:
                    continue
                seen.add(path)
                relative = path.relative_to(root)
                # Browser JSON is already redacted by the original driver. Apply
                # the export scrub as a second boundary, preserving original hashes.
                browser_json = "browser" in relative.parts and path.suffix == ".json"
                # The original fixture's reviewed state projection contains only
                # mapping/version/expiry metadata, never token or ciphertext values.
                browser_state = (label == "controller" and len(relative.parts) == 3
                                 and relative.parts[:2] == ("browser-rounds", "browser-final-293")
                                 and re.fullmatch(r"state-[a-z0-9-]{1,60}\.json", name) is not None)
                admitted = (name in allowed or name.endswith(".command.json") or name.endswith(".result.json")
                            or (name.startswith(safe_prefixes) and path.suffix == ".json") or browser_json or browser_state)
                admitted = admitted or (label.startswith("process-") and ".identity." in name and name.endswith(".json"))
                telemetry = (label.startswith("process-") and name in {
                    "resource-samples.jsonl", "backlog-classification-observations.jsonl", "joint-socket-samples.jsonl"})
                image = "browser" in relative.parts and path.suffix == ".png"
                listing = name in {"test-list.log", "ignored-test-list.log", "bootstrap-list.log"}
                if not (admitted or telemetry or image or listing):
                    if path.suffix in {".log", ".jsonl"} and path.stat().st_size <= 128 * 1024**2:
                        withheld.append({"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size,
                                         "reason": "Original private raw log retained on runner; not uploaded"})
                    continue
                require(path.stat().st_size <= 64 * 1024**2, "evidence member exceeds bounded export")
                raw = path.read_bytes()
                original_sha = hashlib.sha256(raw).hexdigest()
                redactions = 0
                if admitted or telemetry:
                    if telemetry:
                        rows = raw.splitlines()
                        require(0 < len(rows) <= 10000, "original telemetry exceeds bounded line count")
                        value = [json.loads(row) for row in rows]
                    else:
                        value = json.loads(raw)

                    def scrub(item, key=""):
                        nonlocal redactions
                        if re.fullmatch(r"(?:password|secret|token|access_token|refresh_token|authorization|credential|dsn|database_url|cookie|controlkey|adminpassword|extoresession)", key, re.I):
                            redactions += 1
                            return "[PRIVATE]"
                        if isinstance(item, dict):
                            return {k: scrub(v, k) for k, v in item.items()}
                        if isinstance(item, list):
                            return [scrub(v) for v in item]
                        if isinstance(item, str):
                            cleaned = re.sub(r"postgres(?:ql)?://[^\s\"']+", "[PRIVATE_DSN]", item)
                            cleaned = re.sub(r"\bBearer [A-Za-z0-9._~-]+", "Bearer [PRIVATE]", cleaned)
                            cleaned = re.sub(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+", "[PRIVATE_TOKEN]", cleaned)
                            cleaned = re.sub(r"lmmk_[A-Za-z0-9_-]+", "[PRIVATE_KEY]", cleaned)
                            if cleaned != item:
                                redactions += 1
                            return cleaned
                        return item

                    filtered = scrub(value)
                    if redactions:
                        raw = (("\n".join(json.dumps(row, sort_keys=True) for row in filtered) + "\n") if telemetry
                               else (json.dumps(filtered, sort_keys=True, indent=2) + "\n")).encode()
                total += len(raw)
                require(total <= 256 * 1024**2 and len(files) < 5000, "evidence export exceeds envelope")
                target = destination / label / relative
                target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                with target.open("xb") as stream:
                    stream.write(raw)
                files.append({"source_path": str(path), "export_path": str(target.relative_to(destination)),
                              "original_sha256": original_sha, "export_sha256": sha(target),
                              "redactions": redactions, "original_bytes_preserved": redactions == 0})
    record(destination / "export-index.json", {"format": "lmm-final-runtime-evidence-export-v1",
           "runtime_revision": RUNTIME, "control_revision": os.environ["GITHUB_SHA"],
           "files": files, "private_raw_logs_not_uploaded": withheld,
           "databases_configs_credentials_uploaded": False, "production_inputs_used": False,
           "retention": "Failed fixtures remain private on this disposable runner, not a durable backup"})


def run(args):
    require(not os.environ.get("GH_TOKEN") and not os.environ.get("GITHUB_TOKEN"), "download token reached runtime step")
    evidence = private(args.evidence)
    payload = args.output / "payload"
    prepared = json.loads((payload / "preparation.json").read_text())
    require(prepared["execution_layout"] == execution_layout(args.source, args.output), "run layout differs from preparation")
    record(evidence / "runtime-binding.json", {"runtime_revision": RUNTIME, "control_revision": os.environ["GITHUB_SHA"],
           "commerce_compile_revision": COMMERCE_REVISION, "prepared_sha256": prepared["prepared_sha256"],
           "preparation_sha256": sha(payload / "preparation.json"), "execution_layout": prepared["execution_layout"],
           "runtime_generation": identity(os.getpid()), "preparation_generation": prepared["preparation_generation"],
           "old_pid_or_lease_used_for_runtime_ownership": False,
           "original_helpers": {str(path.relative_to(args.source)): sha(path) for path in [
               args.source / "scripts/verify_model_resources.py", args.source / "tests/commerce/verify_process.py",
               args.source / "tests/payments/verify_process.py", args.source / "tests/architecture_completion/build_lease.py"]}})
    _, _, _, lease = input_modules(args.source)
    lease_parent = private(evidence / "runtime-lease")
    lease.initialize(lease_parent / "resource")
    requests = private(lease_parent / "requests")
    lock = lease_parent / "resource/heavy.lock"
    lock_before = (lock.stat().st_dev, lock.stat().st_ino)
    result = {"result": "NOT_RUN", "runtime_revision": RUNTIME, "control_revision": os.environ["GITHUB_SHA"],
              "native_machine_used": False, "scope": "Frozen293 preview runtime original synthetic gates"}
    try:
        code = lease.run(lease_parent / "resource", requests, "S11", "frontend", sha(evidence / "runtime-binding.json"),
                         [sys.executable, "-B", str(Path(__file__).resolve()), "execute", "--source", str(args.source),
                          "--output", str(args.output), "--evidence", str(evidence)], args.source, 1200)
        finished = list(requests.glob("*/finished.json"))
        require(len(finished) == 1 and (finished[0].parent / "released.json").is_file(), "original runtime lease not released")
        finish = json.loads(finished[0].read_text())
        release = json.loads((finished[0].parent / "released.json").read_text())
        require(finish["exit_code"] == code and release["holder"] == identity(os.getpid())
                and lock_before == (lock.stat().st_dev, lock.stat().st_ino), "runtime lease generation or inode changed")
        result.update(exit_code=code, runtime_lease_finished_sha256=sha(finished[0]),
                      runtime_lease_released_sha256=sha(finished[0].parent / "released.json"),
                      runtime_lease_inode_unchanged=True)
        sequence = json.loads((evidence / "sequence-result.json").read_text())
        require(code == 0 and sequence["result"] == "PASS" and len(sequence["cases"]) == 7
                and sequence["all_seven_original_gates_executed"] is True, "first required original gate failed")
        result.update(result="PASS", gates=sequence["cases"], acceptance_executed=True)
    except BaseException as error:
        result.update(result="FAIL", error_type=type(error).__name__, error=str(error))
        raise
    finally:
        record(evidence / "run-result.json", result)
        export_evidence(args)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("acquire", "prepare", "restore", "run", "execute"))
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--artifact-id", type=int)
    for name in ("artifact-sha256", "bundle-sha256", "manifest-sha256"):
        parser.add_argument("--" + name)
    args = parser.parse_args()
    require(os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("GITHUB_REPOSITORY") == REPOSITORY,
            "cloud-only controller requires its original repository workflow")
    require(re.fullmatch(r"[0-9a-f]{40}", os.environ.get("GITHUB_SHA", "")) is not None
            and git(CONTROL, "rev-parse", "HEAD") == os.environ["GITHUB_SHA"]
            and git(CONTROL, "status", "--porcelain=v1", "--untracked-files=all") == "", "controller source is not clean and bound")
    execution_layout(args.source, args.output)
    if args.phase in {"run", "execute"}:
        require(args.evidence == Path(os.environ["RUNNER_TEMP"]) / "final-runtime-evidence", "unexpected runtime evidence root")
    if args.phase == "restore":
        require(args.artifact_id and all((args.artifact_sha256, args.bundle_sha256, args.manifest_sha256)), "missing transport identities")
    try:
        {"acquire": acquire, "prepare": prepare, "restore": restore, "run": run, "execute": execute_sequence}[args.phase](args)
    except BaseException as error:
        if args.output.is_dir():
            path = args.output / ("controller-failure-" + args.phase + ".json")
            if not path.exists():
                record(path, {"result": "FAIL", "phase": args.phase, "runtime_revision": RUNTIME,
                       "error_type": type(error).__name__, "message": str(error), "product_acceptance_claimed": False})
        print("FAIL: " + args.phase + "; inspect preserved receipts", file=sys.stderr)
        return 1
    print("COMPLETE: " + args.phase + "; consult the phase receipt for its limited scope")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
