#!/usr/bin/env python3
"""Run the unchanged 16 browser cases and cite six immutable earlier PASS receipts.

The prior failed run remains failed. No compiled input is rebuilt, no old process
or lease is adopted, and no production URL or credential is accepted.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
import time
import zipfile

os.umask(0o077)
sys.dont_write_bytecode = True
CONTROL = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("fixed_final_runtime", CONTROL / "scripts/final-runtime-acceptance.py")
base = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = base
spec.loader.exec_module(base)
require, sha, record = base.require, base.sha, base.record

ORIGIN = {"run_id": 44939623460, "run_attempt": 1,
          "control_revision": "9bd568c7d8929752924a39997850388bee76e385"}
PREPARED_ARTIFACT = 14721760645
PREPARED_ZIP_SHA = "a88bf01f274929fed94a6709e4b5ea7c1d4381aa3a02d72a7df0d5fe11138b98"
BUNDLE_SHA = "1044e749c60c48c5041a76b5133ad4d32797f0775ea7e64dd34bdaca909a0ce1"
MANIFEST_SHA = "4145061e94cf59240b6536074559a31e7e5e5f6e1e024130743d23fa3ad2c874"
PREPARATION_SHA = "1bc538de674f80d141423ea8d819e6ff8e6650ca039820006b05878cc571975f"
CORE_PREPARED_SHA = "0cf1b34d256b2f088e365b05ce6e9f30dc798a888806659989b224e946468de5"
RESULT_ARTIFACT = 14723002826
RESULT_ZIP_BYTES = 6852049
RESULT_ZIP_SHA = "5b541a433d507e51b25fbf68dab36f87ec1f57bed4e8d7b49a00a3bd446505c8"
EXPORT_INDEX_SHA = "c8d78bd716af53db90e7b955b6bbabebcb8fa30932c18d4e6ba42bb1dbc9fb9a"
RECEIPTS = {
    "shared540": "094213a0ca0194ffa45dbce3a736ca23531b5b59f02c61f43a675756a316fd91",
    "shared240": "dadb4419b623c27ae086f430e6c36ed8034f2b0407d221ac7f78ecb10c753436",
    "whole540": "8a47b94f955eb263cd4d3c71690f8a2427f27895e6eb52345108e2366035dd78",
    "whole240": "a48c6a24e61a254a8c2a6b7aba8272e07c3e96d6115ccb4ec9cba55b946d918e",
    "core540": "96804895c2645c48291e081af52bdcb93bae2002ce4b3c5e60c8a4ebb1605309",
    "payment302": "a803b8d7890c93dbe5bca8a8b1314fb437ec1dcaa6fc112bf9d4b17b38bb3daf",
}


def json_file(path):
    return json.loads(base.regular(path).read_text())


def safe_relative(name):
    path = PurePosixPath(name)
    require(not path.is_absolute() and str(path) == name and "\\" not in name
            and not any(part in {"", ".", ".."} for part in path.parts)
            and not any(ord(c) < 32 for c in name), "unsafe prior evidence path")
    return path


def unpack_prior(archive, output):
    """The complete pinned failed artifact is preserved; only its safe files unpack."""
    require(archive.stat().st_size == RESULT_ZIP_BYTES and sha(archive) == RESULT_ZIP_SHA,
            "prior result transport differs")
    base.private(output)
    with zipfile.ZipFile(archive) as zipped:
        entries = zipped.infolist()
        require(len(entries) == 365 and len({item.filename for item in entries}) == 365
                and sum(item.file_size for item in entries) == 86134068,
                "pinned prior result inventory differs")
        for item in entries:
            name = safe_relative(item.filename)
            require(not item.is_dir() and 0 < item.file_size <= 64 * 1024**2
                    and stat.S_IFMT(item.external_attr >> 16) in {0, stat.S_IFREG},
                    "unsafe prior result member")
            target = output / name
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            with zipped.open(item) as source, target.open("xb") as dest:
                shutil.copyfileobj(source, dest)


def prior_receipts(root):
    """Check unchanged public receipts against the pinned original export index."""
    index_file = root / "export-index.json"
    require(sha(index_file) == EXPORT_INDEX_SHA, "prior export index changed")
    index = json_file(index_file)
    require(index["runtime_revision"] == base.RUNTIME
            and index["control_revision"] == ORIGIN["control_revision"]
            and index["production_inputs_used"] is False
            and index["databases_configs_credentials_uploaded"] is False,
            "prior result source or data boundary differs")
    by_source, by_export = {}, {}
    for item in index["files"]:
        name = str(safe_relative(item["export_path"]))
        require(name not in by_export and item["source_path"] not in by_source,
                "duplicate prior evidence binding")
        require(sha(root / name) == item["export_sha256"], "prior exported file changed: " + name)
        by_source[item["source_path"]] = item
        by_export[name] = item

    def original(source_path, digest):
        item = by_source[source_path]
        require(item["original_sha256"] == item["export_sha256"] == digest
                and item["original_bytes_preserved"] is True and item["redactions"] == 0,
                "reused receipt is not the original byte sequence")
        return json_file(root / item["export_path"]), item

    sequence = json_file(root / "controller/sequence-result.json")
    require(sequence["result"] == "FAIL" and sequence["failure"] is not None
            and sequence["runtime_revision"] == base.RUNTIME
            and sequence["commerce_compile_revision"] == base.RUNTIME
            and sequence["control_revision"] == ORIGIN["control_revision"]
            and sequence["all_seven_original_gates_executed"] is False
            and sequence["first_failure_stops_sequence"] is True
            and sequence["source_unchanged_after_attempt"] is True
            and sequence["compiled_inputs_unchanged_after_attempt"] is True
            and [item["case"] for item in sequence["cases"]] == list(RECEIPTS),
            "prior six-pass/failed-browser sequence differs")
    values, references = {}, []
    for case in sequence["cases"]:
        name = case["case"]
        require(case["result"] == "PASS" and case["sha256"] == RECEIPTS[name],
                "prior case is not the fixed successful receipt")
        value, exported = original(case["receipt"], case["sha256"])
        require(value["result"] == "PASS" and value["cluster_removed"] is True,
                "prior case result or cleanup was incomplete")
        values[name] = value
        reference = {"case": name, "result": "PASS", "reused": True,
                     "original_run_id": ORIGIN["run_id"], "original_job_id": 136956454425,
                     "original_artifact_id": RESULT_ARTIFACT, "sha256": case["sha256"],
                     "original_source_path": case["receipt"], "export_path": exported["export_path"],
                     "executed_in_this_job": False}
        if name.startswith(("shared", "whole")):
            pg_in_budget = name.startswith("whole")
            require(value["external_core_sha256"] == base.CORE_SHA
                    and value["extension_sha256"] == value["external_extension_sha256"] == base.GO_SHA
                    and value["external_test_sha256"] == base.COMMERCE_SHA
                    and value["whole_budget"] is pg_in_budget and value["shared_resources"] is True
                    and value["mixed_load"] is name.endswith("540")
                    and value["mixed_slow_sockets"] is name.endswith("240")
                    and case["postgres_in_budget"] is pg_in_budget,
                    "prior capacity profile or exact compiled inputs differ")
            require(len(case["native_observations"]) >= 3, "prior native lifecycle is incomplete")
            observed_names = set()
            for path, digest in case["native_observations"].items():
                native, _ = original(path, digest)
                observed_names.add(PurePosixPath(path).name)
                require(native["format"] == "lmm-native-library-observation-v1"
                        and native["native_input_sha256"] == case["native_input_sha256"]
                        and {item["kind"]: item["sha256"] for item in native["libraries"]}
                            == {"host": base.HOST_SHA, "plugin": base.PLUGIN_SHA}
                        and all(item["maps"] for item in native["libraries"]),
                        "prior actual Go native mappings differ")
            require({"native-libraries.initial.json", "native-libraries.after-mixed.json",
                     "native-libraries.restart.json"} <= observed_names,
                    "prior native observations lack a required lifecycle point")
            reference.update(postgres_in_budget=pg_in_budget,
                             native_observations=case["native_observations"],
                             native_input_sha256=case["native_input_sha256"])
        elif name == "core540":
            require(value["case"] == "sustained" and value["prepared_sha256"] == CORE_PREPARED_SHA,
                    "prior Core capacity prepared binding differs")
        else:
            require(value["case"] == "accepted-proof-expiry" and value["full_payment_acceptance"] is False
                    and value["binary_sha256"]["core"] == base.CORE_SHA
                    and value["binary_sha256"]["go"] == base.GO_SHA
                    and value["product_bindings"]["product_source_drift"] == []
                    and value["product_bindings"]["go_reference_sha256"] == RECEIPTS["shared540"],
                    "prior selected payment scope or source binding differs")
            reference["scope"] = "Synthetic accepted-proof expiry/restart only; not full payment qualification"
        references.append(reference)
    binding = json_file(root / "controller/runtime-binding.json")
    require(binding["prepared_sha256"] == CORE_PREPARED_SHA
            and binding["preparation_sha256"] == PREPARATION_SHA
            and binding["runtime_revision"] == base.RUNTIME
            and binding["control_revision"] == ORIGIN["control_revision"],
            "prior sequence is not bound to the fixed prepared artifact")
    old_browser = json_file(root / "controller/browser-rounds/browser-final-fa57/browser/result.json")
    require(old_browser["overall"] == "FAIL", "prior failed browser result was replaced")
    summary = {"format": "lmm-prior-six-runtime-receipts-v1", "result": "VERIFIED_HISTORICAL_RECEIPTS",
               "runtime_revision": base.RUNTIME, "origin": ORIGIN, "result_artifact_id": RESULT_ARTIFACT,
               "result_zip_sha256": RESULT_ZIP_SHA, "export_index_sha256": EXPORT_INDEX_SHA,
               "sequence_sha256": sha(root / "controller/sequence-result.json"),
               "prior_sequence_result": "FAIL", "prior_browser_result": "FAIL",
               "prepared_artifact_id": PREPARED_ARTIFACT, "prepared_zip_sha256": PREPARED_ZIP_SHA,
               "preparation_sha256": PREPARATION_SHA, "core_prepared_sha256": CORE_PREPARED_SHA,
               "cases": references, "acceptance_executed_in_this_job": False,
               "old_pid_or_lease_reused": False, "production_deployment_authorized": False}
    return summary, values


def validate_restored(args):
    payload = args.output / "payload"
    require(sha(payload / "preparation.json") == PREPARATION_SHA
            and sha(payload / "core-prepared/prepared.json") == CORE_PREPARED_SHA,
            "historical prepared receipt bytes changed")
    base.verify_compiled_inputs(payload)
    base.verified_component_paths(args.source, payload)
    root = args.output / "prior-result/files/final-runtime-evidence/public"
    summary, values = prior_receipts(root)
    expected = json_file(payload / "source-before.json")
    require(base.source_snapshot(args.source) == expected
            and json_file(root / "controller/source-after-attempt.json") == expected,
            "new browser source does not equal the six-gate immutable source closure")
    _, commerce, payment, _ = base.input_modules(args.source)
    commerce_inputs, payment_inputs = commerce.sources(), payment.sources()
    require(commerce_inputs == json_file(payload / "commerce-runtime-inputs.json")
            and payment_inputs == json_file(payload / "payment-compiled-inputs.json"),
            "restored original driver input closure differs")
    for name in ("shared540", "shared240", "whole540", "whole240"):
        require(values[name]["source_sha256"] == commerce_inputs, "reused commerce source closure differs")
    require(values["payment302"]["source_sha256"] == payment_inputs
            and values["payment302"]["binary_sha256"]["test"] == sha(payload / "payments/payments_process")
            and values["payment302"]["binary_sha256"]["bootstrap"] == sha(payload / "payments/bootstrap"),
            "reused selected-payment compiled input closure differs")
    require(json_file(root / "prepared/prepared.json") == json_file(payload / "core-prepared/prepared.json"),
            "reused Core capacity driver binding differs")
    return summary


def restore(args):
    args.artifact_id, args.artifact_sha256 = PREPARED_ARTIFACT, PREPARED_ZIP_SHA
    args.bundle_sha256, args.manifest_sha256 = BUNDLE_SHA, MANIFEST_SHA
    base.restore(args, origin=ORIGIN)
    archive = base.download_artifact(args.output / "prior-result", RESULT_ARTIFACT, RESULT_ZIP_BYTES,
                                     RESULT_ZIP_SHA, ORIGIN["control_revision"], ORIGIN["run_id"],
                                     "final-runtime-results-" + str(ORIGIN["run_id"]))
    unpack_prior(archive, args.output / "prior-result/files")
    record(args.output / "prior-six.result.json", validate_restored(args))


def execute(args):
    require(not os.environ.get("GH_TOKEN") and not os.environ.get("GITHUB_TOKEN"),
            "download token reached browser execution")
    evidence, source, payload = args.evidence, args.source, args.output / "payload"
    prior = validate_restored(args)
    require(prior == json_file(args.output / "prior-six.result.json"), "historical receipt summary changed")
    snapshot = base.source_snapshot(source)
    result = {"format": "lmm-final-runtime-browser-resume-sequence-v1", "result": "NOT_RUN",
              "runtime_revision": base.RUNTIME, "control_revision": os.environ["GITHUB_SHA"],
              "cases": [], "reused_cases": prior["cases"], "prior_sequence_result": "FAIL",
              "prior_result_zip_sha256": RESULT_ZIP_SHA,
              "all_seven_original_gates_executed_in_this_job": False,
              "production_data_used": False, "production_deployment_authorized": False}
    paths, started = [], time.monotonic()
    try:
        browser_root = base.private(evidence / "browser-rounds")
        candidate, node, chromium, playwright = base.browser_candidate(source, payload, evidence)
        browser_env = {**os.environ, "LMM_ACCEPTANCE_ROOT": str(browser_root), "LMM_ACCEPTANCE_NODE": str(node),
                       "LMM_ACCEPTANCE_CHROMIUM": str(chromium), "LMM_ACCEPTANCE_PLAYWRIGHT": str(playwright)}
        paths.append(browser_root)
        base.command([sys.executable, "-B", CONTROL / "tests/final-runtime/browser/run-isolated.py",
                      "--round", "browser-final-fa57", "--candidate", candidate],
                     source, evidence / "browser16.private.log", 1860, browser_env)
        browser_result = browser_root / "browser-final-fa57/browser/result.json"
        launcher_result = browser_root / "browser-final-fa57-launcher/result.json"
        value, launcher = json_file(browser_result), json_file(launcher_result)
        original_cases = json_file(CONTROL / "tests/final-runtime/browser/preparation.json")["case_names"]
        require(value["overall"] == "PASS" and len(original_cases) == len(value["cases"]) == 16
                and set(value["cases"]) == set(original_cases)
                and all(status == "PASS" for status in value["cases"].values())
                and value["page_errors"] == [] and launcher["overall"] == "PASS"
                and launcher["surviving_registered_pids"] == []
                and launcher["fixture_cleanup"]["all_stopped"] is True,
                "original full browser gates or cleanup incomplete")
        namespace_root = browser_root / "browser-final-fa57-namespace"
        namespace_file, outer_file = namespace_root / "namespace-result.json", namespace_root / "outer-result.json"
        namespace, outer = json_file(namespace_file), json_file(outer_file)
        require(namespace["status"] == "PASS" and namespace["inner_exit_code"] == 0
                and namespace["surviving_namespace_processes"] == []
                and namespace["forced_cleanup_processes"] == []
                and outer["status"] == "PASS" and outer["candidate_unchanged"] is True
                and all(outer["parent_resolver_unchanged"].values()),
                "browser namespace cleanup or host isolation failed")
        result["cases"].append({"case": "browser16", "result": "PASS", "reused": False,
                                "executed_in_this_job": True, "receipt": str(browser_result),
                                "sha256": sha(browser_result), "launcher_receipt_sha256": sha(launcher_result),
                                "namespace_receipt_sha256": sha(namespace_file), "outer_receipt_sha256": sha(outer_file)})
        require(base.source_snapshot(source) == snapshot, "runtime source drift after browser")
        base.verified_component_paths(source, payload)
        base.verify_compiled_inputs(payload)
        result.update(result="PASS", source_drift=[], combined_seven_same_product_receipts=True)
    except BaseException as error:
        result.update(result="FAIL", failure={"type": type(error).__name__, "message": str(error)},
                      combined_seven_same_product_receipts=False)
        raise
    finally:
        result["seconds"] = time.monotonic() - started
        try:
            after = base.source_snapshot(source)
            result["source_unchanged_after_attempt"] = after == snapshot
            record(evidence / "source-after-attempt.json", after)
            require(after == snapshot, "source changed after browser attempt")
            base.verify_compiled_inputs(payload)
            result["compiled_inputs_unchanged_after_attempt"] = True
        except Exception as error:
            result["after_attempt_check_error"] = type(error).__name__
            result["result"] = "FAIL"
            result["combined_seven_same_product_receipts"] = False
        result["private_evidence_roots"] = [str(path) for path in paths]
        result["failure_retention_boundary"] = (
            "The old failed run remains immutable. New failed fixtures are retained privately on this disposable runner; "
            "this export is not a durable PostgreSQL backup.")
        record(evidence / "sequence-result.json", result)


def run(args):
    require(not os.environ.get("GH_TOKEN") and not os.environ.get("GITHUB_TOKEN"),
            "download token reached browser run")
    evidence = base.private(args.evidence)
    result = {"result": "NOT_RUN", "runtime_revision": base.RUNTIME,
              "control_revision": os.environ["GITHUB_SHA"], "scope": "Six cited original PASS gates plus new full16 browser",
              "all_seven_original_gates_executed_in_this_job": False,
              "production_deployment_authorized": False}
    try:
        prior = validate_restored(args)
        require(prior == json_file(args.output / "prior-six.result.json"), "historical receipt summary changed")
        record(evidence / "prior-six.result.json", prior)
        record(evidence / "runtime-binding.json", {
            "runtime_revision": base.RUNTIME, "control_revision": os.environ["GITHUB_SHA"],
            "origin": ORIGIN, "prepared_zip_sha256": PREPARED_ZIP_SHA,
            "prior_result_zip_sha256": RESULT_ZIP_SHA, "prior_six_sha256": sha(evidence / "prior-six.result.json"),
            "preparation_sha256": PREPARATION_SHA, "prepared_sha256": CORE_PREPARED_SHA,
            "execution_layout": base.execution_layout(args.source, args.output),
            "runtime_generation": base.identity(os.getpid()), "old_pid_or_lease_used_for_runtime_ownership": False})
        _, _, _, lease = base.input_modules(args.source)
        lease_parent = base.private(evidence / "runtime-lease")
        lease.initialize(lease_parent / "resource")
        requests = base.private(lease_parent / "requests")
        lock = lease_parent / "resource/heavy.lock"
        lock_before = (lock.stat().st_dev, lock.stat().st_ino)
        code = lease.run(lease_parent / "resource", requests, "S11", "frontend", sha(evidence / "runtime-binding.json"),
                         [sys.executable, "-B", str(Path(__file__).resolve()), "execute",
                          "--source", str(args.source), "--output", str(args.output), "--evidence", str(evidence)],
                         args.source, 1200)
        finished = list(requests.glob("*/finished.json"))
        require(len(finished) == 1 and (finished[0].parent / "released.json").is_file(),
                "fresh browser runtime lease did not release")
        finish, release = json_file(finished[0]), json_file(finished[0].parent / "released.json")
        require(finish["exit_code"] == code and release["holder"] == base.identity(os.getpid())
                and lock_before == (lock.stat().st_dev, lock.stat().st_ino),
                "fresh browser runtime lease generation or inode changed")
        result.update(exit_code=code, runtime_lease_finished_sha256=sha(finished[0]),
                      runtime_lease_released_sha256=sha(finished[0].parent / "released.json"),
                      runtime_lease_inode_unchanged=True)
        sequence = json_file(evidence / "sequence-result.json")
        require(code == 0 and sequence["result"] == "PASS" and len(sequence["cases"]) == 1
                and sequence["cases"][0]["case"] == "browser16" and len(sequence["reused_cases"]) == 6
                and sequence["combined_seven_same_product_receipts"] is True
                and sequence["all_seven_original_gates_executed_in_this_job"] is False,
                "new full browser gate failed; historical result remains unchanged")
        result.update(result="PASS", executed_cases=sequence["cases"], reused_cases=sequence["reused_cases"],
                      acceptance_executed=True, combined_seven_same_product_receipts=True)
    except BaseException as error:
        result.update(result="FAIL", error_type=type(error).__name__, error=str(error))
        raise
    finally:
        record(evidence / "run-result.json", result)
        base.export_evidence(args)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("restore", "run", "execute"))
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    require(os.environ.get("GITHUB_ACTIONS") == "true"
            and os.environ.get("GITHUB_REPOSITORY") == base.REPOSITORY,
            "browser resume requires its original cloud repository workflow")
    require(re.fullmatch(r"[0-9a-f]{40}", os.environ.get("GITHUB_SHA", "")) is not None
            and base.git(CONTROL, "rev-parse", "HEAD") == os.environ["GITHUB_SHA"]
            and base.git(CONTROL, "status", "--porcelain=v1", "--untracked-files=all") == "",
            "current browser controller is not clean and bound")
    base.execution_layout(args.source, args.output)
    if args.phase != "restore":
        require(args.evidence == Path(os.environ["RUNNER_TEMP"]) / "final-runtime-evidence",
                "unexpected browser evidence root")
    try:
        {"restore": restore, "run": run, "execute": execute}[args.phase](args)
    except BaseException as error:
        if args.output.is_dir():
            target = args.output / ("controller-failure-browser-resume-" + args.phase + ".json")
            if not target.exists():
                record(target, {"result": "FAIL", "phase": args.phase, "runtime_revision": base.RUNTIME,
                                "error_type": type(error).__name__, "message": str(error),
                                "product_acceptance_claimed": False})
        print("FAIL: browser resume " + args.phase + "; inspect preserved receipts", file=sys.stderr)
        return 1
    print("COMPLETE: browser resume " + args.phase + "; consult limited-scope receipts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
