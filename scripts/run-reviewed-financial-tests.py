#!/usr/bin/env python3
"""Run current financial Python tests against the sealed reviewed Core69 source.

The historical source is a fixture, not admission of the current Core schema.
No review pin, importer rule, test selection, or PostgreSQL opt-in is changed.
A missing Git object or any source mismatch fails before unittest discovery.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
SCOPE = "financial_current_python_against_reviewed69"
SOURCE_REVISION = "8f1b6b837ace7027f2a62f15168843790ce6de28"
REVIEW_PATH = "scripts/reviewed-fresh69-successor.json"
REVIEW_SHA256 = "ed6d7738ead1f836058df9ff2a5788216d8128809e01742f68ca0341f55b6794"
COMPILED_MANIFEST_SHA256 = "9d10056fbd0bdbdfcbcfad4f800a2f74ae86b92eda3250c47f71977184c2a165"
REVIEWED_FILE_COUNT = 1004
COMMAND = [
    sys.executable, "-B", "-m", "unittest", "discover",
    "-s", "scripts", "-p", "test*financial*.py", "-v",
]


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write_json(path: Path, value: dict) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("x", encoding="utf-8") as output:
        os.chmod(temporary, 0o600)
        json.dump(value, output, indent=2, sort_keys=True)
        output.write("\n")
    temporary.replace(path)


def git_output(*arguments: str) -> str:
    result = subprocess.run(
        ["git", "--no-replace-objects", "-C", str(ROOT), *arguments],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if result.returncode:
        raise ValueError(
            "required local Git source is unavailable; fetch the fixed revision "
            f"{SOURCE_REVISION} before running this fixture"
        )
    return result.stdout.decode("utf-8").strip()


def review_files() -> dict[str, str]:
    path = ROOT / REVIEW_PATH
    if path.is_symlink() or not path.is_file() or digest(path) != REVIEW_SHA256:
        raise ValueError("sealed reviewed69 review file is missing or changed")
    review = json.loads(path.read_bytes())
    if (
        review.get("source_schema_revision") != SOURCE_REVISION
        or review.get("compiled_manifest_sha256") != COMPILED_MANIFEST_SHA256
        or review.get("approval_state") != "reviewed-native-fresh69-v1"
    ):
        raise ValueError("reviewed69 source identity does not match the fixed runner")
    files = review.get("files")
    if not isinstance(files, dict) or len(files) != REVIEWED_FILE_COUNT:
        raise ValueError("reviewed69 source closure is incomplete")
    for name, expected in files.items():
        relative = PurePosixPath(name)
        if (
            relative.is_absolute() or ".." in relative.parts
            or str(relative) != name or "\\" in name
            or relative.parts[0] == "scripts"
            or not isinstance(expected, str)
            or re.fullmatch(r"[0-9a-f]{64}", expected) is None
        ):
            raise ValueError("reviewed69 source closure contains an invalid member")
    return files


def verify_source(fixture: Path, expected: dict[str, str]) -> None:
    for name, checksum in expected.items():
        path = fixture / name
        if (
            path.is_symlink() or not path.is_file()
            or any((fixture / parent).is_symlink() for parent in Path(name).parents)
            or digest(path) != checksum
        ):
            raise ValueError(f"reviewed69 source member is missing or changed: {name}")


def script_hashes(directory: Path) -> dict[str, str]:
    result = {}
    for path in sorted(directory.rglob("*")):
        relative = path.relative_to(directory)
        if "__pycache__" in relative.parts or path.suffix in (".pyc", ".pyo"):
            continue
        if path.is_symlink():
            raise ValueError(f"current scripts contain a symlink: {relative.as_posix()}")
        if path.is_file():
            result["scripts/" + relative.as_posix()] = digest(path)
    return result


def prepare(output: Path, evidence: dict) -> Path:
    reviewed = review_files()
    actual_revision = git_output("rev-parse", "--verify", SOURCE_REVISION + "^{commit}")
    if actual_revision != SOURCE_REVISION:
        raise ValueError("Git did not resolve the exact reviewed69 source revision")
    evidence["current_worktree_revision"] = git_output("rev-parse", "--verify", "HEAD")
    archive = output / "reviewed-source.tar"
    with archive.open("xb") as target:
        os.chmod(archive, 0o600)
        result = subprocess.run(
            ["git", "--no-replace-objects", "-C", str(ROOT),
             "archive", "--format=tar", SOURCE_REVISION],
            stdout=target, stderr=subprocess.PIPE, check=False,
        )
    if result.returncode:
        raise ValueError("could not archive the fixed reviewed69 source")
    evidence["git_archive_sha256"] = digest(archive)
    fixture = output / "fixture"
    fixture.mkdir(mode=0o700)
    with tarfile.open(archive, "r:") as source:
        source.extractall(fixture, filter="data")
    verify_source(fixture, reviewed)
    write_json(output / "verified-source-files.json", reviewed)
    evidence["verified_source_files"] = len(reviewed)

    scripts = script_hashes(ROOT / "scripts")
    if scripts.get(REVIEW_PATH) != REVIEW_SHA256:
        raise ValueError("sealed review changed while preparing current Python")
    destination = fixture / "scripts"
    shutil.rmtree(destination)
    shutil.copytree(
        ROOT / "scripts", destination,
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "*.pyo"),
    )
    if script_hashes(destination) != scripts:
        raise ValueError("copied current scripts do not match their captured hashes")
    # The overlay is disjoint from all 1004 reviewed source members.
    verify_source(fixture, reviewed)
    write_json(output / "current-scripts-sha256.json", scripts)
    evidence["current_python_sha256"] = {
        name: checksum for name, checksum in scripts.items() if name.endswith(".py")
    }
    evidence["current_scripts_manifest_sha256"] = digest(output / "current-scripts-sha256.json")
    evidence["discovery_files"] = sorted(
        path.name for path in destination.glob("test*financial*.py") if path.is_file()
    )
    if not evidence["discovery_files"]:
        raise ValueError("current financial unittest discovery has no matching files")
    return fixture


def run_tests(fixture: Path, output: Path) -> tuple[int, dict]:
    summary = {}
    with (output / "unittest.log").open("xb") as log:
        os.chmod(output / "unittest.log", 0o600)
        # Inherit all original PG switches, fixture receipts and tool paths.
        # Their existing test owners still decide which PostgreSQL modes run.
        with subprocess.Popen(
            COMMAND, cwd=fixture, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        ) as process:
            assert process.stdout is not None
            for line in process.stdout:
                log.write(line)
                log.flush()
                sys.stdout.buffer.write(line)
                sys.stdout.buffer.flush()
                text = line.decode("utf-8", errors="replace").strip()
                count = re.fullmatch(r"Ran ([0-9]+) tests? in ([0-9.]+)s", text)
                if count:
                    summary["tests_run"] = int(count.group(1))
                    summary["elapsed_seconds"] = float(count.group(2))
                if text == "OK" or text.startswith(("OK (", "FAILED (")):
                    summary["outcome"] = text
                    for key, value in re.findall(
                        r"(failures|errors|skipped|expected failures|unexpected successes)=([0-9]+)",
                        text,
                    ):
                        summary[key.replace(" ", "_")] = int(value)
            code = process.wait()
    if code == 0 and summary.get("tests_run", 0) == 0:
        raise ValueError("unittest reported success without running any discovered tests")
    return code, summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output", type=Path,
        help="new private evidence directory; must not already exist (default: a fresh directory under out/)",
    )
    arguments = parser.parse_args()
    output = None
    evidence = {
        "format_version": 1,
        "scope": SCOPE,
        "current_core_source_admitted": False,
        "migration_or_deployment_authorized": False,
        "source_revision": SOURCE_REVISION,
        "review_path": REVIEW_PATH,
        "review_sha256": REVIEW_SHA256,
        "compiled_manifest_sha256": COMPILED_MANIFEST_SHA256,
        "command": COMMAND,
        "environment_inherited": True,
        "postgres_opt_ins": {
            "LMM_FINANCIAL_PG_TEST": os.environ.get("LMM_FINANCIAL_PG_TEST"),
            "LMM_FINANCIAL_PG_FIXTURE_RECEIPT_supplied": bool(
                os.environ.get("LMM_FINANCIAL_PG_FIXTURE_RECEIPT")
            ),
        },
        "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "state": "PREPARING",
    }
    started = time.monotonic()
    print(SCOPE + ": current Python against the fixed reviewed Core69 fixture.", flush=True)
    print("This run does not admit current Core source or authorize migration/deployment.", flush=True)
    try:
        if arguments.output is None:
            parent = ROOT / "out"
            parent.mkdir(exist_ok=True)
            output = Path(tempfile.mkdtemp(prefix="reviewed-financial-", dir=parent))
        else:
            candidate = arguments.output.expanduser().absolute()
            candidate.mkdir(mode=0o700)
            output = candidate
        print(f"{SCOPE}: evidence={output}", flush=True)
        write_json(output / "evidence.json", evidence)
        fixture = prepare(output, evidence)
        evidence["state"] = "RUNNING"
        write_json(output / "evidence.json", evidence)
        code, summary = run_tests(fixture, output)
        evidence.update(
            state="PASS" if code == 0 else "FAIL",
            exit_code=code,
            unittest=summary,
        )
    except (OSError, ValueError, tarfile.TarError) as error:
        evidence.update(state="FAIL", exit_code=1, error=str(error))
        print(f"{SCOPE}: FAIL: {error}", file=sys.stderr, flush=True)
        code = 1
    evidence["finished_at_utc"] = datetime.now(timezone.utc).isoformat()
    evidence["elapsed_seconds"] = round(time.monotonic() - started, 3)
    if output is not None and output.is_dir():
        write_json(output / "evidence.json", evidence)
    print(f"{SCOPE}: {evidence['state']}; evidence={output}", flush=True)
    return code if code >= 0 else 128 - code


if __name__ == "__main__":
    raise SystemExit(main())
