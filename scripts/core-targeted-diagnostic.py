#!/usr/bin/env python3
"""One bounded Core diagnostic; never release or full-CI qualification."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

BASE = "7b0832d37f0a0be0b87cceaf43f7c6a6dfe856bc"
REGRESSION = "unknown_classification_takes_conversation_lock_before_charge_row"
LOCK_CYCLE = "Unknown classification reversed the conversation/request lock order"
APP05 = "core_schema::upgrade::payment_discounts::tests::app05_upgrade_engine_66_to67_exact_replay_and_runtime_denial"
MANAGED70 = "core_schema::upgrade::managed_applications::tests::managed70_owner_upgrade_preserves_all_other_data_acl_and_commit_recovery"
ASSISTANT = [
    "answer_settles_once_and_replays_without_dispatch",
    "assistant_request_limits_share_user_budget_before_hold_and_dispatch",
    "budget_and_expiry_reject_without_leaving_a_charge",
    "cancelled_rpc_does_not_cancel_core_worker_or_replay_dispatch",
    "concurrent_reservation_never_dispatches_twice",
    "cross_owner_revocation_and_digest_fail_before_dispatch",
    "missing_usage_is_unknown_and_never_replayed_after_reconstruction",
    "real_runtime_role_executes_without_updating_immutable_authority_tables",
    "rpc_requires_single_host_protocol_and_original_personal_session",
    "token_limit_answer_settles_and_core_builds_ordered_context",
    "unknown_classification_takes_conversation_lock_before_charge_row",
    "usage_sources::usage_assistant_paid_and_unknown_sources_preserve_consumption_and_holds",
    "zero_balance_user_uses_bounded_treasury_and_real_gift_ledger"
]
TEST_FILE = "services/core/tests/assistant_model.rs"

os.umask(0o077)
control, candidate, baseline, evidence = map(lambda p: Path(p).resolve(), sys.argv[1:])
evidence.mkdir(mode=0o700, parents=True, exist_ok=True)
mode, requested = os.environ["DIAGNOSTIC_MODE"], os.environ["PRODUCT_SHA"]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def save(name, value):
    (evidence / name).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def execute(name, argv, cwd, timeout):
    record = {"argv": argv, "cwd": str(cwd),
              "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    path = evidence / (name + ".log")
    with path.open("wb") as output:
        try:
            result = subprocess.run(argv, cwd=cwd, stdout=output,
                                    stderr=subprocess.STDOUT, timeout=timeout, check=False)
            record["exit_code"] = result.returncode
        except subprocess.TimeoutExpired:
            record["exit_code"] = 124
            record["outer_diagnostic_timeout_seconds"] = timeout
    record.update(finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  log=path.name, log_sha256=sha(path), log_bytes=path.stat().st_size)
    save(name + ".command.json", record)
    print(name + ": exit " + str(record["exit_code"]), flush=True)
    return record, path.read_text(errors="replace")


def source_manifest(root):
    paths = subprocess.check_output(
        ["git", "-C", str(root), "ls-files", "-z", "--", "services/core",
         "contracts/proto", "deployment/postgres", "config/legal"]).decode().split("\0")
    return [{"path": path, "sha256": sha(root / path),
             "bytes": (root / path).stat().st_size} for path in sorted(filter(None, paths))]


def listed(text):
    return sorted(re.findall(r"^([A-Za-z0-9_:]+): test$", text, re.MULTILINE))


def test_run(label, executable, names, filters, cwd, negative=False):
    entry = {"expected_cases": names, "executable": str(executable),
             "executable_sha256": sha(executable)}
    list_command, listing = execute(label + "-list",
        [str(executable), *filters, "--list", "--format", "terse"], cwd, 60)
    ignored_command, ignored = execute(label + "-ignored",
        [str(executable), *filters, "--ignored", "--list", "--format", "terse"], cwd, 60)
    actual, ignored_names = listed(listing), listed(ignored)
    entry.update(actual_cases=actual, selected_count=len(actual), ignored_cases=ignored_names)
    valid = (list_command["exit_code"] == 0 and ignored_command["exit_code"] == 0
             and actual == sorted(names) and not ignored_names)
    if not valid:
        entry.update(accepted=False, status="SELECTION_CONTRACT_FAILED")
        save(label + "-result.json", entry)
        return entry
    command, output = execute(label,
        [str(executable), *filters, "--test-threads=1", "--nocapture",
         "--format", "pretty", "--color", "never"], cwd, 600)
    summaries = re.findall(
        r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;",
        output)
    running = re.findall(r"^running (\d+) tests?$", output, re.MULTILINE)
    counts = [int(v) for v in summaries[-1][1:]] if len(summaries) == 1 else None
    count_ok = (counts is not None and counts[0] + counts[1] == len(names)
                and counts[2:4] == [0, 0] and running == [str(len(names))])
    if negative:
        accepted = (count_ok and command["exit_code"] == 101 and counts[:2] == [0, 1]
                    and LOCK_CYCLE in output)
        status = "EXPECTED_LOCK_CYCLE_OBSERVED" if accepted else "COUNTEREXAMPLE_NOT_ESTABLISHED"
    else:
        accepted = (count_ok and command["exit_code"] == 0
                    and counts[:2] == [len(names), 0])
        status = "TARGETED_PASS" if accepted else "TARGETED_FAIL"
    entry.update(accepted=accepted, status=status, test_exit_code=command["exit_code"],
                 result_counts=counts, running_count=running,
                 lock_cycle_assertion_observed=LOCK_CYCLE in output)
    save(label + "-result.json", entry)
    return entry


def main():
    if mode not in ("candidate", "pre_fix", "managed70") or not re.fullmatch(r"[0-9a-f]{40}", requested):
        raise ValueError("invalid diagnostic inputs")
    if git(candidate, "rev-parse", "HEAD") != requested or git(candidate, "diff", "--name-only"):
        raise ValueError("candidate checkout does not match the immutable source input")
    root = candidate
    binding = {"scope": "TARGETED_DIAGNOSTIC_ONLY", "mode": mode,
               "candidate_sha": requested, "candidate_tree": git(candidate, "rev-parse", "HEAD^{tree}"),
               "control_sha": git(control, "rev-parse", "HEAD"),
               "control_workflow_sha256": sha(control / ".github/workflows/core-protocol.yml"),
               "control_script_sha256": sha(Path(__file__).resolve())}
    if mode == "pre_fix":
        if git(baseline, "rev-parse", "HEAD") != BASE or git(baseline, "diff", "--name-only"):
            raise ValueError("pre-fix checkout drift")
        root = baseline
        shutil.copyfile(candidate / TEST_FILE, baseline / TEST_FILE)
        if git(root, "diff", "--name-only") != TEST_FILE:
            raise ValueError("pre-fix must contain exactly the candidate test-file overlay")
        (evidence / "pre-fix-test-only-overlay.patch").write_text(git(root, "diff", "--", TEST_FILE) + "\n")
        binding["test_overlay"] = {"source_sha": requested, "path": TEST_FILE,
                                   "git_blob": git(candidate, "rev-parse", "HEAD:" + TEST_FILE),
                                   "sha256": sha(candidate / TEST_FILE)}
    binding.update(compiled_base_sha=git(root, "rev-parse", "HEAD"),
                   compiled_base_tree=git(root, "rev-parse", "HEAD^{tree}"),
                   source_root=str(root),
                   environment={k: os.environ[k] for k in (
                       "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "CARGO_PROFILE_DEV_DEBUG",
                       "CARGO_PROFILE_TEST_DEBUG", "RUST_TEST_THREADS", "GOWORK")})
    before = source_manifest(root)
    save("source-before.json", before)
    save("source-binding.json", binding)
    execute("rustc-version", ["rustc", "--version", "--verbose"], root / "services/core", 30)
    execute("cargo-version", ["cargo", "--version"], root / "services/core", 30)
    if mode == "managed70":
        targets = ["--lib"]
    else:
        targets = ["--test", "assistant_model"] if mode == "pre_fix" else ["--lib", "--test", "assistant_model"]
    command, output = execute("compile",
        ["cargo", "test", "--locked", *targets, "--no-run", "--message-format=json"],
        root / "services/core", 4200)
    results = []
    if command["exit_code"] == 0:
        executables = {}
        for line in output.splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get("reason") == "compiler-artifact" and item.get("executable") and item.get("profile", {}).get("test"):
                executables[item["target"]["name"]] = Path(item["executable"]).resolve()
        save("executables.json", {key: {"path": str(path), "sha256": sha(path)}
                                  for key, path in executables.items()})
        if mode == "managed70":
            results.append(test_run("managed70", executables["lmm_core"], [MANAGED70],
                                    [MANAGED70, "--exact"], root / "services/core"))
        else:
            assistant = executables["assistant_model"]
            cases = [REGRESSION] if mode == "pre_fix" else ASSISTANT
            filters = [REGRESSION, "--exact"] if mode == "pre_fix" else []
            results.append(test_run("assistant", assistant, cases, filters, root / "services/core",
                                    negative=mode == "pre_fix"))
            if mode == "candidate":
                results.append(test_run("app05", executables["lmm_core"], [APP05],
                                        [APP05, "--exact"], root / "services/core"))
    after = source_manifest(root)
    save("source-after.json", after)
    accepted = (command["exit_code"] == 0 and len(results) == (1 if mode in ("pre_fix", "managed70") else 2)
                and all(result["accepted"] for result in results) and before == after)
    save("result.json", {"scope": "TARGETED_DIAGNOSTIC_ONLY", "mode": mode,
                        "accepted": accepted, "compile_exit_code": command["exit_code"],
                        "source_unchanged": before == after, "results": results,
                        "full_ci_qualification": False})
    return 0 if accepted else 1


try:
    sys.exit(main())
except Exception as error:
    save("controller-error.json", {"type": type(error).__name__, "message": str(error)})
    raise
