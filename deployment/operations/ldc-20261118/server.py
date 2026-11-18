#!/usr/bin/env python3
"""Fixed one-off server operations. No credential data is returned to the runner."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import time

REVISION = "723925770caae52cc33a331bbfe2d758729fdc28"
ROOT_UID = 0
BASE = Path("/srv/lmm-shadow")
OPS = BASE / ".ldc-operations-20261118"
STAGE = BASE / ".epay-stage-runner-20261118"
MODES = ("stage", "prepare", "activate")
HASHES = {
    "stage_legacy_ldc.py": "654e648bee5bcf8cce0a92397c652d48748ef1f5afc54b0693a28acae7ef9480",
    "activate_legacy_ldc.py": "181b457598de365d65eda5128479faaa9af7623a938f7cd30a0d002664697de9",
}


class Refused(Exception):
    pass


def require(ok, code):
    if not ok:
        raise Refused(code)


def digest(value):
    return hashlib.sha256(value).hexdigest()


def private_read(path):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == ROOT_UID
            and stat.S_IMODE(info.st_mode) == 0o600 and info.st_size <= 65536,
            "PRIVATE_STATE_INVALID")
    return path.read_bytes()


def private_directory():
    info = OPS.lstat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == ROOT_UID
            and stat.S_IMODE(info.st_mode) == 0o700, "PRIVATE_DIRECTORY_INVALID")


def durable_new(path, value):
    raw = (json.dumps(value, sort_keys=True) + "\n").encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(raw)
        handle.flush()
        os.fsync(handle.fileno())
    fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def command(args, timeout=30):
    result = subprocess.run(args, capture_output=True, timeout=timeout)
    require(result.returncode == 0, "SERVER_COMMAND_FAILED")
    return result.stdout


def release_verified():
    for name, entry in (("core", "bin/lmm-core"), ("extensions", "bin/lmm-extensions"),
                        ("web", "dist/index.html")):
        root = (BASE / name).resolve(strict=True)
        manifest = json.loads((root / "MANIFEST.json").read_bytes())
        require(manifest.get("revision") == REVISION and manifest.get("component") == name
                and manifest.get("schema_version") == 1, "RELEASE_REVISION_MISMATCH")
        item = manifest["files"][entry]
        path = root / entry
        with path.open("rb") as handle:
            actual = hashlib.file_digest(handle, "sha256").hexdigest()
        require(actual == item["sha256"] and path.stat().st_size == item["size"],
                "RELEASE_FILE_MISMATCH")
        if name != "web":
            pid = int(command(["systemctl", "show", "lmm-" + name + ".service",
                               "--property=MainPID", "--value"]).strip())
            require(pid > 1 and os.path.samefile(f"/proc/{pid}/exe", path),
                    "RUNNING_EXECUTABLE_MISMATCH")


def unit(mode):
    return "lmm-ldc-20261118-" + mode + ".service"


def receipt_path(mode):
    return OPS / (mode + ".result.json")


def claim_path(mode):
    return OPS / (mode + ".claim.json")


def safe_result(value):
    """Only fixed enums, integers, booleans and reviewed hashes may leave the host."""
    require(isinstance(value, dict), "RESULT_INVALID")
    mode = value.get("operation")
    require(mode in MODES and value.get("status") in ("completed", "failed"), "RESULT_INVALID")
    out = {"operation": mode, "status": value["status"], "revision": REVISION}
    for name in ("recovery_required", "manual_review_required"):
        out[name] = value.get(name) is True
    if type(value.get("helper_exit_code")) is int:
        out["helper_exit_code"] = value["helper_exit_code"]
    phase = value.get("helper_phase")
    if phase in ("staged", "prepared", "activated", "failed"):
        out["helper_phase"] = phase
    code = value.get("code")
    if isinstance(code, str) and re.fullmatch(r"[A-Z_]{1,80}", code):
        out["code"] = code
    return out


def verify():
    require(os.geteuid() == ROOT_UID, "ROOT_REQUIRED")
    result = {"operation": "verify", "revision": REVISION, "release_verified": False,
              "stage_present": STAGE.exists(), "operations": {}}
    if OPS.exists():
        private_directory()
        for mode in MODES:
            entry = {"claimed": claim_path(mode).exists(),
                     "worker_started": (OPS / (mode + ".started.json")).exists()}
            if receipt_path(mode).exists():
                try:
                    entry["receipt"] = safe_result(json.loads(private_read(receipt_path(mode))))
                except Exception:
                    entry["receipt_unreadable"] = True
            if entry["claimed"]:
                try:
                    raw = command(["systemctl", "show", unit(mode),
                                   "--property=ActiveState", "--property=SubState",
                                   "--property=Result"]).decode()
                    states = dict(line.split("=", 1) for line in raw.splitlines() if "=" in line)
                    entry["unit_state"] = {
                        key: value for key, value in states.items()
                        if key in ("ActiveState", "SubState", "Result")
                        and re.fullmatch(r"[a-z-]{1,40}", value)}
                except Exception:
                    entry["unit_observation_unavailable"] = True
            result["operations"][mode] = entry
    if STAGE.exists():
        try:
            result["stage_plan_sha256"] = digest(private_read(STAGE / "plan.json"))
            record = STAGE / "activation.json"
            if record.exists():
                state = json.loads(private_read(record))
                result["core_start_attempted"] = state.get("core_start_attempted") is True
                result["activation_recovery_required"] = state.get("recovery_required") is True
                phase = state.get("phase")
                if phase in ("preparing", "prepared", "stopping_services",
                             "selecting_configuration", "core_start_attempted", "activated",
                             "restored_before_core_start", "failed", "recovery_required"):
                    result["activation_phase"] = phase
        except Exception:
            result["stage_state_unreadable"] = True
    try:
        release_verified()
        result["release_verified"] = True
    except Exception:
        result["release_code"] = "RELEASE_OBSERVATION_UNVERIFIED"
    return result


def start(mode):
    require(os.geteuid() == ROOT_UID and mode in MODES, "OPERATION_INVALID")
    private_directory()
    lock = os.open(OPS / "dispatch.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for name, expected in HASHES.items():
            require(digest(private_read(OPS / name)) == expected, "HELPER_HASH_MISMATCH")
        require(not claim_path(mode).exists(), "ALREADY_CLAIMED_NEVER_REPLAY")
        for previous in MODES:
            if claim_path(previous).exists():
                require(receipt_path(previous).exists(), "PRIOR_OPERATION_UNRESOLVED")
                prior = safe_result(json.loads(private_read(receipt_path(previous))))
                require(prior["status"] == "completed", "PRIOR_OPERATION_FAILED")
        if mode != "stage":
            predecessor = "stage" if mode == "prepare" else "prepare"
            require(receipt_path(predecessor).exists(), "PREVIOUS_PHASE_REQUIRED")
            prior = safe_result(json.loads(private_read(receipt_path(predecessor))))
            require(prior["status"] == "completed", "PREVIOUS_PHASE_FAILED")
        release_verified()
        durable_new(claim_path(mode), {"operation": mode, "revision": REVISION,
                                      "claimed_at": int(time.time())})
        # A persistent system-manager unit owns the process. SSH/runner loss does
        # not terminate it. A failed launch retains the claim and is never retried.
        command(["systemd-run", "--unit=" + unit(mode), "--no-block",
                 "--property=Type=oneshot", "--property=RemainAfterExit=yes",
                 "--property=UMask=0077", "--property=TimeoutStartSec=900",
                 "--property=TimeoutStopSec=90", "--property=SendSIGKILL=no",
                 "/usr/bin/python3", str(OPS / "server.py"), "worker", mode])
        return {"operation": mode, "status": "claimed", "unit": unit(mode), "revision": REVISION}
    finally:
        os.close(lock)


def worker(mode):
    require(os.geteuid() == ROOT_UID and mode in MODES, "OPERATION_INVALID")
    private_directory()
    require(claim_path(mode).exists() and not receipt_path(mode).exists(),
            "WORKER_STATE_INVALID")
    durable_new(OPS / (mode + ".started.json"), {"operation": mode, "started_at": int(time.time())})
    result = {"operation": mode, "status": "failed", "revision": REVISION,
              "manual_review_required": True, "recovery_required": mode != "stage"}
    try:
        release_verified()
        filename = "stage_legacy_ldc.py" if mode == "stage" else "activate_legacy_ldc.py"
        require(digest(private_read(OPS / filename)) == HASHES[filename], "HELPER_HASH_MISMATCH")
        if mode == "stage":
            args = ["/usr/bin/python3", str(OPS / filename), "--stage-to", str(STAGE)]
        else:
            args = ["/usr/bin/python3", str(OPS / filename), "--stage", str(STAGE),
                    "--prepare-only" if mode == "prepare" else "--activate"]
        executed = subprocess.run(args, capture_output=True)
        value = json.loads(executed.stdout)
        expected = ("staged" if mode == "stage" else
                    "prepared" if mode == "prepare" else "activated")
        if mode == "stage":
            successful = (executed.returncode == 0 and value.get("channel_id") == 5004
                          and value.get("core_enabled") is False
                          and value.get("active_files_changed") is False)
        else:
            successful = executed.returncode == 0 and value.get("phase") == expected
        result.update(status="completed" if successful else "failed",
                      helper_phase=expected if successful else "failed",
                      helper_exit_code=executed.returncode,
                      recovery_required=value.get("recovery_required") is True,
                      manual_review_required=not successful)
        if isinstance(value.get("code"), str) and re.fullmatch(r"[A-Z_]{1,80}", value["code"]):
            result["code"] = value["code"]
    except Exception:
        result["code"] = "WORKER_RESULT_UNKNOWN"
    safe = safe_result(result)
    durable_new(receipt_path(mode), safe)
    return safe


def main():
    os.umask(0o077)
    try:
        args = sys.argv[1:]
        if args == ["verify"]:
            result = verify()
        elif len(args) == 2 and args[0] in ("start", "worker") and args[1] in MODES:
            result = start(args[1]) if args[0] == "start" else worker(args[1])
        else:
            raise Refused("OPERATION_INVALID")
        print(json.dumps(result, sort_keys=True))
        return 1 if result.get("status") == "failed" else 0
    except BaseException as error:
        code = str(error) if isinstance(error, Refused) else "SERVER_OPERATION_UNCERTAIN"
        print(json.dumps({"status": "refused", "code": code,
                          "manual_review_required": True, "revision": REVISION}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
