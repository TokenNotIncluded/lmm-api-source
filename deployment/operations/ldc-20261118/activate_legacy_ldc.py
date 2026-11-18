#!/usr/bin/env python3
"""One-shot disabled-LDC configuration activation; no database migration.

Default: read-only preflight. --prepare-only installs an unused private Core
base/key and runs the real read-only check-db command. --activate additionally
stops Core/Extensions, selects the candidates and checks readiness. The fixed
release, source hashes and actual file ownership/modes are mandatory gates.

Run only after the standard release is live and the private stage was reviewed.
This script never enables payments, prints keys, or invokes a provider. After a
Core start may have occurred, failures retain configuration and require recovery.
"""
import argparse
import contextlib
import dataclasses
import fcntl
import hashlib
import json
import os
import pathlib
import pwd
import re
import signal
import stat
import subprocess
import time
import tomllib
import urllib.request
import uuid

Path = pathlib.Path
REVISION = "723925770caae52cc33a331bbfe2d758729fdc28"
ROOT_UID = 0
BASE = Path("/srv/lmm-shadow")
CORE_MAIN = Path("/etc/lmm.conf.d/native69-20261115/core-final.toml")
CORE_SOURCE = Path("/etc/lmm.conf.d/core-payments.toml")
GO_MAIN = Path("/etc/lmm.conf.d/extensions.toml")
GO_SOURCE = Path("/etc/lmm.conf.d/extensions-payments.toml")
CORE_TARGET = Path("/var/lib/lmm/core/config/payments.toml")
KEY_TARGET = CORE_TARGET.parent / "legacy-ldc-5004.secret"
PREVIEW = CORE_TARGET.parent / "legacy-ldc-5004-check.toml"
LOCK = Path("/run/lock/lmm-native-update.lock")
UNITS = {"core": "lmm-core.service", "extensions": "lmm-extensions.service"}
ENTRIES = {"core": "bin/lmm-core", "extensions": "bin/lmm-extensions",
           "web": "dist/index.html"}
CHANNEL = 5004
MAX_BYTES = 64 * 1024


class GateError(Exception):
    """Only fixed machine codes are allowed in this exception."""
    pass


def require(ok, code):
    if not ok:
        raise GateError(code)


def sha(value):
    return hashlib.sha256(value).hexdigest()


@dataclasses.dataclass(frozen=True)
class Snapshot:
    data: bytes
    uid: int
    gid: int
    mode: int
    device: int
    inode: int

    def evidence(self):
        return {"sha256": sha(self.data), "uid": self.uid, "gid": self.gid,
                "mode": oct(self.mode), "device": self.device, "inode": self.inode}


def read_file(path, limit=MAX_BYTES):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as handle:
        info = os.fstat(handle.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1
                and info.st_size <= limit, "UNSAFE_OR_OVERSIZED_FILE")
        value = handle.read(limit + 1)
        require(len(value) == info.st_size, "FILE_CHANGED_DURING_READ")
        return Snapshot(value, info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode),
                        info.st_dev, info.st_ino)


def exists(path):
    return os.path.lexists(path)


def private_folder(path, uid):
    info = path.lstat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == uid
            and stat.S_IMODE(info.st_mode) == 0o700, "PRIVATE_DIRECTORY_REQUIRED")


def private_file(path, uid, limit=MAX_BYTES):
    value = read_file(path, limit)
    require(value.uid == uid and value.mode == 0o600, "PRIVATE_FILE_REQUIRED")
    return value


def fsync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def write_new(path, value, uid, gid, mode):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    with os.fdopen(fd, "wb") as handle:
        os.fchown(handle.fileno(), uid, gid)
        os.fchmod(handle.fileno(), mode)
        handle.write(value)
        handle.flush()
        os.fsync(handle.fileno())
    fsync_directory(path.parent)


def unchanged(path, expected):
    require(read_file(path) == expected, "SOURCE_CHANGED")


def atomic_replace(path, value, uid, gid, mode, expected=None):
    if expected is not None:
        unchanged(path, expected)
    temporary = path.parent / (".ldc-activation-" + uuid.uuid4().hex + ".tmp")
    # Never remove Core overlays or immutable key files, even after an error.
    write_new(temporary, value, uid, gid, mode)
    if expected is not None:
        unchanged(path, expected)
    os.replace(temporary, path)
    fsync_directory(path.parent)
    actual = read_file(path)
    require(actual.data == value and (actual.uid, actual.gid, actual.mode) == (uid, gid, mode),
            "REPLACEMENT_UNVERIFIED")
    return actual


def command(args, code, timeout=45, env=None):
    try:
        result = subprocess.run(args, capture_output=True, timeout=timeout, env=env)
    except (OSError, subprocess.TimeoutExpired):
        raise GateError(code) from None
    require(result.returncode == 0, code)
    return result.stdout


def unit_state(component):
    raw = command(["systemctl", "show", UNITS[component], "--property=ActiveState",
                   "--property=MainPID"], "SERVICE_OBSERVATION_FAILED")
    fields = dict(line.split("=", 1) for line in raw.decode().splitlines() if "=" in line)
    return fields.get("ActiveState"), int(fields.get("MainPID", "-1"))


def service(action, component):
    command(["systemctl", action, UNITS[component]], "SERVICE_" + action.upper() + "_FAILED",
            timeout=60)


def wait_ready(component):
    port = 3101 if component == "core" else 8081
    # Explicitly ignore proxy environment variables for private readiness.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    deadline = time.monotonic() + 40
    while time.monotonic() < deadline:
        try:
            with opener.open(f"http://127.0.0.1:{port}/health/ready", timeout=2) as response:
                if response.status == 200:
                    return
        except (OSError, ValueError):
            pass
        time.sleep(0.5)
    raise GateError("READINESS_FAILED")


def artifact(component):
    root = (BASE / component).resolve(strict=True)
    require(root.is_dir(), "ARTIFACT_DIRECTORY_MISSING")
    manifest_file = read_file(root / "MANIFEST.json", 4 * 1024 * 1024)
    manifest = json.loads(manifest_file.data)
    require(manifest.get("schema_version") == 1 and manifest.get("component") == component
            and manifest.get("revision") == REVISION, "RELEASE_REVISION_MISMATCH")
    entry = ENTRIES[component]
    expected = manifest["files"][entry]
    path = root / entry
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_size == expected["size"],
            "ARTIFACT_ENTRY_INVALID")
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(block)
    require(hasher.hexdigest() == expected["sha256"], "ARTIFACT_HASH_MISMATCH")
    return {"root": str(root), "manifest_sha256": sha(manifest_file.data),
            "entry_sha256": hasher.hexdigest(), "revision": REVISION}


def verify_release(running=True):
    result = {name: artifact(name) for name in ENTRIES}
    if running:
        for component in UNITS:
            state, pid = unit_state(component)
            require(state == "active" and pid > 1, "SERVICE_NOT_RUNNING")
            binary = Path(result[component]["root"]) / ENTRIES[component]
            require(os.path.samefile(Path(f"/proc/{pid}/exe"), binary),
                    "RUNNING_EXECUTABLE_MISMATCH")
            args = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
            config = CORE_MAIN if component == "core" else GO_MAIN
            require(args.count(b"--config") == 1
                    and args[args.index(b"--config") + 1] == os.fsencode(config),
                    "RUNNING_CONFIG_PATH_MISMATCH")
            result[component]["pid"] = pid
    return result


def release_fingerprint(release):
    return {name: {key: value for key, value in data.items() if key != "pid"}
            for name, data in release.items()}


def core_owner():
    account = pwd.getpwnam("lmm-core")
    return account.pw_uid, account.pw_gid


def core_ld_path(pid):
    for part in Path(f"/proc/{pid}/environ").read_bytes().split(b"\0"):
        if part.startswith(b"LD_LIBRARY_PATH="):
            return part.split(b"=", 1)[1].decode()
    return None


def run_check_db(release, library_path):
    binary = Path(release["core"]["root"]) / ENTRIES["core"]
    args = ["runuser", "-u", "lmm-core", "--", "env"]
    if library_path:
        args.append("LD_LIBRARY_PATH=" + library_path)
    args.extend([str(binary), "--config", str(PREVIEW), "check-db"])
    clean_env = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"}
    report = json.loads(command(args, "CORE_CHECK_DB_FAILED", timeout=60, env=clean_env))
    require(report.get("protocol") == "core.check-db.v1"
            and report.get("status") == "validated"
            and report.get("execution", {}).get("executable_sha256")
            == release["core"]["entry_sha256"], "CORE_CHECK_DB_UNVERIFIED")
    # check-db checks schema/runtime authority; it does not load the full payment
    # provider config. No non-existent Go check-config command is invoked.


def history_clear():
    tables = ("core_payments.intents", "core_payments.legacy_bases",
              "core_payments.accepted_proofs", "core_payments.provider_events",
              "core_payments.provider_refunds", "core_payments.refund_dispatch_bindings",
              "core_subscriptions.external_prices", "core_subscriptions.external_orders")
    sql = "SELECT " + " OR ".join(
        f"EXISTS(SELECT FROM {table} WHERE channel_id={CHANNEL})" for table in tables)
    result = command(["runuser", "-u", "postgres", "--", "env",
                      "PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=10000",
                      "psql", "-X", "-A", "-t", "-v", "ON_ERROR_STOP=1",
                      "-d", "lmm_core_native_20261115", "-c", sql], "HISTORY_CHECK_FAILED")
    require(result.strip() == b"f", "CHANNEL_HAS_HISTORY")


def validate_candidates(source, candidate, plan, key):
    require(plan.get("contract") == "lmm.disabled-ldc-migration.v1", "STAGE_CONTRACT_INVALID")
    require(plan.get("targets") == {"core": str(CORE_TARGET), "key": str(KEY_TARGET),
                                   "go": str(GO_SOURCE), "main": str(CORE_MAIN)},
            "STAGE_TARGET_MISMATCH")
    require(16 <= len(key) <= 4096 and all(33 <= c <= 126 for c in key),
            "STAGED_KEY_INVALID")
    before = {name: tomllib.loads(value.data.decode()) for name, value in source.items()}
    after = {name: tomllib.loads(value.decode()) for name, value in candidate.items()}
    for name in ("core", "go", "main"):
        require(sha(source[name].data) == plan["source_sha256"][name]
                and sha(candidate[name]) == plan["candidate_sha256"][name],
                "STAGE_SOURCE_OR_CANDIDATE_CHANGED")
    for name in ("core", "go"):
        old, new = before[name], after[name]
        require(old.get("version") == 2 and new.get("version") == 2
                and 0 < len(old["channels"]) < 16
                and all(row["id"] != CHANNEL and row["provider"] != "epay"
                        for row in old["channels"])
                and new["channels"][:-1] == old["channels"]
                and len(new["channels"]) == len(old["channels"]) + 1
                and {k: v for k, v in new.items() if k != "channels"}
                == {k: v for k, v in old.items() if k != "channels"},
                "EXISTING_PAYMENT_CONFIGURATION_CHANGED")
    merchant = after["core"]["channels"][-1].get("merchant", "")
    require(re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", merchant) is not None,
            "STAGED_MERCHANT_INVALID")
    identity = {"id": CHANNEL, "provider": "epay", "merchant": merchant,
                "environment": "live", "currency": "LDC", "enabled": False,
                "min_amount_minor": 10000, "payment_type": "epay",
                "endpoint": "https://credit.linux.do/epay",
                "notify_url": "https://native.api.lmm.best/callbacks/5004",
                "return_url": "https://native.api.lmm.best/wallet"}
    require(after["core"]["channels"][-1] == dict(
        identity, credits_per_minor=500, max_amount_minor=9_000_000_000_000,
        webhook_secret_file=str(KEY_TARGET)), "CORE_CANDIDATE_POLICY_INVALID")
    require(after["go"]["channels"][-1] == dict(identity, subscription_enabled=False),
            "GO_CANDIDATE_POLICY_INVALID")
    require(before["main"]["payments"]["config_file"] == str(CORE_SOURCE)
            and after["main"] == dict(before["main"], payments=dict(
                before["main"]["payments"], config_file=str(CORE_TARGET))),
            "MAIN_CANDIDATE_CHANGED_UNRELATED_SETTINGS")
    require(before["core"].get("permissions", {}).get("read_user") is True,
            "SAFE_PROJECTION_PERMISSION_REQUIRED")
    require(tomllib.loads(read_file(GO_MAIN).data.decode())["payments"]["config_file"]
            == str(GO_SOURCE), "GO_PAYMENT_PATH_MISMATCH")


def management_paths():
    parent = CORE_TARGET.parent
    return [CORE_TARGET.with_name(CORE_TARGET.name + ".epay.json"),
            CORE_TARGET.with_name(CORE_TARGET.name + ".epay.lock"),
            *parent.glob("epay-key-*.secret"), *parent.glob(".epay-state-*.tmp")]


def check_no_management_state():
    require(not any(exists(path) for path in management_paths())
            and not exists(CORE_SOURCE.with_name(CORE_SOURCE.name + ".epay.json"))
            and not exists(CORE_SOURCE.with_name(CORE_SOURCE.name + ".epay.lock")),
            "EXISTING_MANAGEMENT_STATE_REQUIRES_RECONCILIATION")


class Activation:
    def __init__(self, stage):
        self.stage = stage
        self.record_path = stage / "activation.json"
        self.backups = stage / "activation-backup"
        self.record = None
        self.record_snapshot = None
        self.recovery_indicated = False
        self.source = {}
        self.candidate = {}
        self.release = {}
        self.core_start_attempted = False
        self.stop_attempted = False
        self.switch_attempted = False
        self.mutation_started = False

    def save_record(self, phase, **fields):
        self.record.update(phase=phase, **fields)
        payload = (json.dumps(self.record, indent=2, sort_keys=True) + "\n").encode()
        if self.record_snapshot is not None:
            self.record_snapshot = atomic_replace(
                self.record_path, payload, ROOT_UID, os.getegid(), 0o600,
                self.record_snapshot)
        else:
            write_new(self.record_path, payload, ROOT_UID, os.getegid(), 0o600)
            self.record_snapshot = private_file(self.record_path, ROOT_UID)

    def preflight(self):
        require(os.geteuid() == ROOT_UID, "SERVER_ROOT_REQUIRED")
        require(self.stage.parent == BASE and self.stage.name.startswith(".epay-stage-")
                and self.stage.resolve(strict=True) == self.stage, "STAGE_PATH_INVALID")
        private_folder(self.stage, ROOT_UID)
        # A previous attempt may already have selected the candidate, so source
        # hashes can legitimately differ. Observe the durable recovery marker
        # BEFORE those gates; malformed/partial prior state is also fail-closed.
        if exists(self.record_path):
            self.recovery_indicated = True
            self.record_snapshot = private_file(self.record_path, ROOT_UID)
            record = json.loads(self.record_snapshot.data)
            require(isinstance(record, dict), "PREVIOUS_ATTEMPT_REQUIRES_RECOVERY")
            self.record = record
            self.core_start_attempted = bool(record.get("core_start_attempted"))
            require(record.get("contract") == "lmm.disabled-ldc-activation.v1"
                    and record.get("phase") == "prepared"
                    and record.get("revision") == REVISION
                    and not record.get("core_start_attempted")
                    and not record.get("recovery_required"),
                    "PREVIOUS_ATTEMPT_REQUIRES_RECOVERY")
            self.recovery_indicated = False
        self.source = {"core": read_file(CORE_SOURCE), "go": read_file(GO_SOURCE),
                       "main": read_file(CORE_MAIN)}
        plan = json.loads(private_file(self.stage / "plan.json", ROOT_UID).data)
        filenames = {"core": "core-payments.toml", "go": "extensions-payments.toml",
                     "main": "core-final.toml"}
        self.candidate = {name: private_file(self.stage / filename, ROOT_UID).data
                          for name, filename in filenames.items()}
        self.key = private_file(self.stage / KEY_TARGET.name, ROOT_UID, 4096).data
        validate_candidates(self.source, self.candidate, plan, self.key)
        self.uid, self.gid = core_owner()
        check_no_management_state()
        self.release = verify_release()
        self.library_path = core_ld_path(self.release["core"]["pid"])
        history_clear()
        self.stage_fingerprint = sha(private_file(self.stage / "plan.json", ROOT_UID).data)
        if self.record is not None:
            unchanged(self.record_path, self.record_snapshot)
            require(self.record.get("stage_plan_sha256") == self.stage_fingerprint,
                    "PREVIOUS_ATTEMPT_REQUIRES_RECOVERY")
            require(self.record.get("source") == {
                name: value.evidence() for name, value in self.source.items()},
                "SOURCE_METADATA_CHANGED_SINCE_PREPARE")
            self.verify_prepared()
        else:
            require(not any(exists(path) for path in (CORE_TARGET, KEY_TARGET, PREVIEW))
                    and not exists(self.backups), "UNRECOGNIZED_PREPARED_FILES")
        self.verify_sources()
        return {"phase": "preflight_passed", "revision": REVISION, "channel_id": CHANNEL,
                "disabled": True, "old_channels_and_permissions_preserved": True,
                "go_metadata_preserved": {
                    "uid": self.source["go"].uid, "gid": self.source["go"].gid,
                    "mode": oct(self.source["go"].mode)},
                "core_management_writable_after_activation": True,
                "pricing_review_required_before_enabling": True}

    def verify_sources(self):
        for name, path in (("core", CORE_SOURCE), ("go", GO_SOURCE), ("main", CORE_MAIN)):
            unchanged(path, self.source[name])

    def verify_prepared(self):
        private_folder(CORE_TARGET.parent, self.uid)
        for path, expected in ((CORE_TARGET, self.candidate["core"]),
                               (KEY_TARGET, self.key), (PREVIEW, self.candidate["main"])):
            actual = private_file(path, self.uid)
            require(actual.data == expected and actual.gid == self.gid,
                    "PREPARED_CONFIGURATION_CHANGED")
        check_no_management_state()
        require(self.record.get("prepared_sha256") == {
            "core": sha(self.candidate["core"]), "key": sha(self.key),
            "preview": sha(self.candidate["main"])}, "PREPARED_RECORD_CHANGED")
        private_folder(self.backups, ROOT_UID)
        for name, expected in self.source.items():
            require(private_file(self.backups / (name + ".toml"), ROOT_UID).data == expected.data,
                    "BACKUP_CHANGED")

    def prepare(self):
        if self.record is not None:
            self.verify_prepared()
            run_check_db(self.release, self.library_path)
            return
        self.mutation_started = True
        self.record = {"contract": "lmm.disabled-ldc-activation.v1", "revision": REVISION,
                       "channel_id": CHANNEL, "stage_plan_sha256": self.stage_fingerprint,
                       "source": {name: value.evidence() for name, value in self.source.items()},
                       "release": release_fingerprint(self.release),
                       "core_start_attempted": False, "recovery_required": False}
        self.save_record("preparing")
        self.backups.mkdir(mode=0o700)
        fsync_directory(self.stage)
        for name, snapshot in self.source.items():
            write_new(self.backups / (name + ".toml"), snapshot.data,
                      ROOT_UID, os.getegid(), 0o600)
        parent = CORE_TARGET.parent
        if exists(parent):
            private_folder(parent, self.uid)
        else:
            require(parent.parent.resolve(strict=True) == parent.parent,
                    "MANAGED_PARENT_PATH_UNSAFE")
            parent.mkdir(mode=0o700)
            os.chown(parent, self.uid, self.gid)
            os.chmod(parent, 0o700)
            fsync_directory(parent.parent)
        check_no_management_state()
        # The main pointer remains unchanged until BOTH key and base are durable.
        write_new(KEY_TARGET, self.key, self.uid, self.gid, 0o600)
        write_new(CORE_TARGET, self.candidate["core"], self.uid, self.gid, 0o600)
        write_new(PREVIEW, self.candidate["main"], self.uid, self.gid, 0o600)
        self.record["prepared_sha256"] = {"core": sha(self.candidate["core"]),
                                         "key": sha(self.key),
                                         "preview": sha(self.candidate["main"])}
        self.verify_prepared()
        run_check_db(self.release, self.library_path)
        self.verify_sources()
        self.save_record("prepared", core_check_db="validated",
                         core_check_db_scope="schema_and_runtime_authority",
                         go_config_check="source_contract_and_TOML_preflight_only")

    def activate(self):
        self.prepare()
        self.verify_sources()
        self.verify_prepared()
        require(verify_release() == self.release, "LIVE_RELEASE_CHANGED")
        self.mutation_started = True
        self.stop_attempted = True
        self.save_record("stopping_services")
        for name in ("extensions", "core"):
            service("stop", name)
            require(unit_state(name) == ("inactive", 0), "SERVICE_DID_NOT_STOP")
        self.verify_sources()
        self.verify_prepared()
        history_clear()
        require(release_fingerprint(verify_release(running=False))
                == release_fingerprint(self.release), "LIVE_ARTIFACT_CHANGED")
        self.switch_attempted = True
        self.save_record("selecting_configuration")
        for name, path in (("main", CORE_MAIN), ("go", GO_SOURCE)):
            old = self.source[name]
            atomic_replace(path, self.candidate[name], old.uid, old.gid, old.mode, old)
        # Durable BEFORE calling systemctl: even a timeout may mean Core started
        # and acknowledged an owner save. No automatic config rollback after here.
        self.save_record("core_start_attempted", core_start_attempted=True,
                         recovery_required=True)
        self.core_start_attempted = True
        service("start", "core")
        wait_ready("core")
        service("start", "extensions")
        wait_ready("extensions")
        current = verify_release()
        require(release_fingerprint(current) == release_fingerprint(self.release),
                "RELEASE_CHANGED_AFTER_START")
        for name, path in (("main", CORE_MAIN), ("go", GO_SOURCE)):
            actual = read_file(path)
            original = self.source[name]
            require(actual.data == self.candidate[name]
                    and (actual.uid, actual.gid, actual.mode)
                    == (original.uid, original.gid, original.mode),
                    "SELECTED_CONFIGURATION_CHANGED")
        # Do not insist that management sidecars are absent after start: a real
        # owner may have just saved a valid update. Never remove or overwrite it.
        self.save_record("activated", recovery_required=False,
                         services_ready=True, pids={name: current[name]["pid"] for name in UNITS})
        return {"phase": "activated", "revision": REVISION, "channel_id": CHANNEL,
                "core_and_extensions_ready": True, "disabled": True,
                "funds_permissions_unchanged": True, "database_rollback_attempted": False,
                "pricing_review_required_before_enabling": True}

    def failure(self, code):
        result = {"phase": "failed", "code": code, "recovery_required": False,
                  "core_start_may_have_occurred": bool(self.core_start_attempted
                      or (self.record and self.record.get("core_start_attempted"))),
                  "management_files_deleted": False, "database_rollback_attempted": False}
        if self.record is None or not self.mutation_started:
            result["recovery_required"] = bool(
                self.recovery_indicated or result["core_start_may_have_occurred"])
            return result
        try:
            if self.core_start_attempted or self.record.get("core_start_attempted"):
                result["recovery_required"] = True
            elif self.stop_attempted:
                # An unexpected process after pointer selection is also an
                # uncertain start, even if this script did not issue start.
                if self.switch_attempted:
                    require(all(unit_state(name) == ("inactive", 0) for name in UNITS),
                            "UNEXPECTED_RUNNING_PROCESS")
                current = {}
                for name, path in (("main", CORE_MAIN), ("go", GO_SOURCE)):
                    actual = read_file(path)
                    old = self.source[name]
                    require(actual.data in (old.data, self.candidate[name])
                            and (actual.uid, actual.gid, actual.mode)
                            == (old.uid, old.gid, old.mode), "RECOVERY_SOURCE_CHANGED")
                    current[name] = actual
                for name, path in (("main", CORE_MAIN), ("go", GO_SOURCE)):
                    old = self.source[name]
                    if current[name].data != old.data:
                        atomic_replace(path, old.data, old.uid, old.gid, old.mode, current[name])
                # Only the pre-start case can restore the original main and Go
                # files. Prepared Core base/key and all sidecars remain intact.
                self.save_record("restored_before_core_start", recovery_required=True)
                service("start", "core")
                wait_ready("core")
                service("start", "extensions")
                wait_ready("extensions")
                result["original_configuration_restored"] = True
            self.save_record("failed", error_code=code, **{
                key: value for key, value in result.items() if key not in ("phase", "code")})
        except BaseException:
            result["recovery_required"] = True
            try:
                self.save_record("recovery_required", error_code=code, recovery_required=True)
            except BaseException:
                result["recovery_record_write_failed"] = True
        return result


@contextlib.contextmanager
def mutation_lock():
    fd = os.open(LOCK, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        require(stat.S_ISREG(os.fstat(fd).st_mode) and os.fstat(fd).st_uid == ROOT_UID,
                "DEPLOYMENT_LOCK_UNSAFE")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise GateError("DEPLOYMENT_LOCK_BUSY") from None
        yield
    finally:
        os.close(fd)


def interrupted(_signum, _frame):
    raise GateError("ACTIVATION_INTERRUPTED")


def failure_code(error):
    return str(error) if isinstance(error, GateError) else "ACTIVATION_INTERNAL_ERROR"


def perform(activation, args):
    """All mutation and recovery must remain inside the caller's deployment lock."""
    try:
        report = activation.preflight()
        if args.activate:
            report = activation.activate()
        elif args.prepare_only:
            activation.prepare()
            report.update(phase="prepared", active_files_changed=False,
                          services_restarted=False, core_check_db="validated")
        return report, 0
    except BaseException as error:
        return activation.failure(failure_code(error)), 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--prepare-only", action="store_true")
    mode.add_argument("--activate", action="store_true")
    args = parser.parse_args()
    os.umask(0o077)
    for signum in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(signum, interrupted)
    activation = Activation(args.stage)
    if args.prepare_only or args.activate:
        try:
            with mutation_lock():
                report, status = perform(activation, args)
        except BaseException as error:
            # Lock acquisition/release errors must not trigger unlocked recovery.
            report = {"phase": "failed", "code": failure_code(error),
                      "recovery_required": activation.mutation_started,
                      "management_files_deleted": False,
                      "database_rollback_attempted": False}
            status = 1
    else:
        report, status = perform(activation, args)
    # Never echo arbitrary exceptions, subprocess output, SQL DSN, merchant IDs,
    # key material or candidate TOML. Recovery completed before releasing the lock.
    print(json.dumps(report, sort_keys=True))
    return status


if __name__ == "__main__":
    raise SystemExit(main())
