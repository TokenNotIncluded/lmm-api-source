#!/usr/bin/env python3
"""One bounded Core diagnostic; never release or full-CI qualification."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile

BASE = "7b0832d37f0a0be0b87cceaf43f7c6a6dfe856bc"
REGRESSION = "unknown_classification_takes_conversation_lock_before_charge_row"
LOCK_CYCLE = "Unknown classification reversed the conversation/request lock order"
APP05 = "core_schema::upgrade::payment_discounts::tests::app05_upgrade_engine_66_to67_exact_replay_and_runtime_denial"
MANAGED70 = "core_schema::upgrade::managed_applications::tests::managed70_owner_upgrade_preserves_all_other_data_acl_and_commit_recovery"
PROVIDER78_CAPTURE = "core_schema::upgrade::provider_oauth_clients::tests::capture_provider_oauth_clients78_targets"
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
CONTINUITY = [
    ("continuity-a5",
     "model_http::responses_continuity_independent::responses_continuity_independent_actual_kill_restart_same_database"),
    ("continuity-tristate",
     "model_http::responses_continuity_independent::responses_continuity_independent_store_tristate_and_exact_refusal"),
    ("native-performance-success",
     "model_http::input_api::tests::input_api_pg_native_key_restricted_runtime_settles_exact_usage_once"),
    ("native-performance-unknown",
     "model_http::input_api::tests::input_api_pg_unknown_usage_keeps_original_hold_and_never_replays"),
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


def capture_provider78(executable, compile_output, cwd, capture_file):
    entry = {"scope": "CATALOG_MEASUREMENT_ONLY", "accepted": False,
             "expected_cases": [PROVIDER78_CAPTURE],
             "ordinary_provider_upgrade_qualification": False,
             "full_ci_qualification": False}
    try:
        artifacts = []
        for line in compile_output.splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if (item.get("reason") == "compiler-artifact"
                    and item.get("target", {}).get("name") == "lmm_core"
                    and item.get("profile", {}).get("test") and item.get("executable")):
                artifacts.append(item)
        if (len(artifacts) != 1 or artifacts[0]["target"]["kind"] != ["lib"]
                or Path(artifacts[0]["target"]["src_path"]).resolve() != cwd / "src/lib.rs"
                or Path(artifacts[0]["executable"]).resolve() != executable):
            raise ValueError("capture requires the unique current Core lib-test compiler artifact")
        save("provider78-compiler-artifact.json", artifacts[0])
        entry.update(executable=str(executable), executable_sha256=sha(executable))
        selected, listing = execute("provider78-capture-list",
            [str(executable), PROVIDER78_CAPTURE, "--exact", "--list", "--format", "terse"], cwd, 60)
        ignored, ignored_listing = execute("provider78-capture-ignored",
            [str(executable), PROVIDER78_CAPTURE, "--exact", "--ignored", "--list", "--format", "terse"], cwd, 60)
        entry.update(actual_cases=listed(listing), ignored_cases=listed(ignored_listing))
        if (selected["exit_code"] != 0 or ignored["exit_code"] != 0
                or entry["actual_cases"] != [PROVIDER78_CAPTURE]
                or entry["ignored_cases"] != [PROVIDER78_CAPTURE]):
            raise ValueError("capture selection must be exactly the one explicitly ignored measurement")
        if capture_file.exists() or capture_file.is_symlink():
            raise ValueError("capture output must not exist before the only invocation")
        command, output = execute("provider78-capture",
            [str(executable), PROVIDER78_CAPTURE, "--exact", "--ignored", "--nocapture",
             "--test-threads=1", "--format", "pretty", "--color", "never"], cwd, 600)
        summaries = re.findall(
            r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;",
            output)
        running = re.findall(r"^running (\d+) tests?$", output, re.MULTILINE)
        counts = [int(v) for v in summaries[-1][1:]] if len(summaries) == 1 else None
        entry.update(test_exit_code=command["exit_code"], result_counts=counts, running_count=running)
        # Retain the original bounded private output even if libtest subsequently
        # reports failure. Its existence is never accepted as a successful capture.
        destination = evidence / "provider-oauth-clients78.json"
        if capture_file.exists() or capture_file.is_symlink():
            info = capture_file.lstat()
            if (not stat.S_ISREG(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o600
                    or info.st_uid != os.getuid() or info.st_size > 1024 * 1024):
                raise ValueError("capture output must be a bounded private regular file owned by this job")
            raw = capture_file.read_bytes()
            with destination.open("xb") as output_file:
                output_file.write(raw)
            if sha(destination) != sha(capture_file):
                raise ValueError("retained capture bytes differ from the original private output")
            entry.update(capture_output=destination.name, capture_sha256=sha(destination), capture_bytes=len(raw))
        if (command["exit_code"] != 0 or counts is None or counts[:4] != [1, 0, 0, 0]
                or running != ["1"] or summaries[0][0] != "ok"):
            raise ValueError("the only capture invocation did not complete exactly one measurement")
        if "capture_output" not in entry:
            raise ValueError("the measurement did not create its required private output")
        measured = json.loads(raw)
        target = measured["provider_target"]
        fields = {"manifest_sha256", "catalog_sha256", "oauth_predecessor_catalog_sha256",
                  "oauth_catalog_sha256", "raw_schema_sha256"}
        if (measured.get("schema") != "core-provider-oauth-clients-capture.v1"
                or measured.get("source_sha") != requested
                or measured.get("server_version_num") != 180006
                or set(target) != fields
                or any(not isinstance(target[k], str) or not re.fullmatch(r"[0-9a-f]{64}", target[k]) for k in fields)
                or measured.get("database_transactions") != "both_rolled_back"
                or measured.get("fresh_and_incremental_catalog_equal") is not True
                or measured["predecessor"]["component_count"] != 77
                or measured["fresh_target"]["component_count"] != 78
                or measured["fresh_target"]["manifest_sha256"] != target["manifest_sha256"]
                or measured["fresh_target"]["catalog_sha256"] != target["catalog_sha256"]
                or hashlib.sha256(measured["compiled_manifest_utf8"].encode()).hexdigest() != target["manifest_sha256"]
                or sha(cwd / "schema/provider_oauth_clients.sql") != target["raw_schema_sha256"]):
            raise ValueError("capture receipt does not satisfy its immutable source and rollback contract")
        historical = {name: sha(cwd / ("schema/" + filename)) for name, filename in (
            ("oauth-grants", "oauth_grants.sql"),
            ("managed-applications", "managed_applications.sql"),
            ("charging", "charging.sql"),
            ("connection-history-import", "connection_history_import.sql"))}
        if measured["historical_schema_sha256"] != historical:
            raise ValueError("capture historical DDL hashes differ from the compiled source")
        entry.update(accepted=True, status="CATALOG_MEASUREMENT_COMPLETED")
    except Exception as error:
        entry.update(status="CATALOG_MEASUREMENT_FAILED", error_type=type(error).__name__, error=str(error))
    save("provider78-capture-result.json", entry)
    return entry


def main():
    if mode not in ("candidate", "pre_fix", "managed70", "continuity", "provider78_capture") or not re.fullmatch(r"[0-9a-f]{40}", requested):
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
    if mode == "continuity":
        private_artifacts = evidence.parent / "core-targeted-continuity-private"
        private_artifacts.mkdir(mode=0o700)
        os.environ["RESPONSES_CONTINUITY_INDEPENDENT_ARTIFACTS"] = str(private_artifacts)
        binding["continuity_artifacts"] = {
            "private_directory": str(private_artifacts),
            "retained_evidence": "worker .log files and a5/a8 result JSON only; no private config or ready files"}
    if mode == "provider78_capture":
        private_base = Path.home() / ".local/state/lmm-api"
        private_base.mkdir(mode=0o700, parents=True, exist_ok=True)
        private_capture = Path(tempfile.mkdtemp(prefix="provider78-capture-", dir=private_base))
        if stat.S_IMODE(private_capture.stat().st_mode) != 0o700:
            raise ValueError("capture directory must have mode0700")
        capture_file = private_capture / "provider-oauth-clients78.json"
        os.environ["LMM_PROVIDER_OAUTH_CAPTURE_FILE"] = str(capture_file)
        os.environ["LMM_PROVIDER_OAUTH_CAPTURE_SOURCE_SHA"] = requested
        binding.update(scope="CATALOG_MEASUREMENT_ONLY", capture_private_directory=str(private_capture),
                       capture_output_file=str(capture_file), exact_ignored_case=PROVIDER78_CAPTURE,
                       ordinary_provider_upgrade_qualification=False)
    before = source_manifest(root)
    save("source-before.json", before)
    save("source-binding.json", binding)
    rustc_command, rustc_output = execute("rustc-version", ["rustc", "--version", "--verbose"], root / "services/core", 30)
    cargo_command, cargo_output = execute("cargo-version", ["cargo", "--version"], root / "services/core", 30)
    if mode == "provider78_capture" and (rustc_command["exit_code"] != 0 or cargo_command["exit_code"] != 0
            or not re.search(r"^rustc 1\.100\.0\s", rustc_output)
            or not re.search(r"^cargo 1\.100\.0\s", cargo_output)):
        raise ValueError("provider78 capture requires the original Rust and Cargo1.100.0 toolchain")
    if mode in ("managed70", "continuity", "provider78_capture"):
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
        if mode == "provider78_capture":
            results.append(capture_provider78(executables["lmm_core"], output,
                                              root / "services/core", capture_file))
        elif mode == "continuity":
            try:
                for label, name in CONTINUITY:
                    results.append(test_run(label, executables["lmm_core"], [name],
                                            [name, "--exact"], root / "services/core"))
            finally:
                retained = evidence / "continuity-process"
                retained.mkdir(mode=0o700)
                paths = list(private_artifacts.glob("*.log"))
                paths += [private_artifacts / name for name in
                          ("a5-process-restart.json", "a8-store-exact.json")]
                for path in paths:
                    if path.is_file():
                        shutil.copyfile(path, retained / path.name)
        elif mode == "managed70":
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
    expected_results = 4 if mode == "continuity" else (1 if mode in ("pre_fix", "managed70", "provider78_capture") else 2)
    accepted = (command["exit_code"] == 0 and len(results) == expected_results
                and all(result["accepted"] for result in results) and before == after)
    save("result.json", {"scope": binding["scope"], "mode": mode,
                        "accepted": accepted, "compile_exit_code": command["exit_code"],
                        "source_unchanged": before == after, "results": results,
                        "full_ci_qualification": False})
    return 0 if accepted else 1


try:
    sys.exit(main())
except Exception as error:
    if mode == "provider78_capture" and not (evidence / "source-after.json").exists():
        try:
            save("source-after.json", source_manifest(candidate))
        except Exception as snapshot_error:
            save("capture-source-after-error.json", {"type": type(snapshot_error).__name__,
                                                     "message": str(snapshot_error)})
    save("controller-error.json", {"type": type(error).__name__, "message": str(error)})
    raise
