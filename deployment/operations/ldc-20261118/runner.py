#!/usr/bin/env python3
"""One-off runner transport: fixed arch-shadow, fixed scripts, no remote shell input."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import stat
import subprocess
import sys
import time

TARGET = "arch-shadow"
BASE = "/srv/lmm-shadow"
OPS = BASE + "/.ldc-operations-20261118"
MODES = ("verify", "stage", "prepare", "activate")
HASHES = {
    "stage_legacy_ldc.py": "654e648bee5bcf8cce0a92397c652d48748ef1f5afc54b0693a28acae7ef9480",
    "activate_legacy_ldc.py": "181b457598de365d65eda5128479faaa9af7623a938f7cd30a0d002664697de9",
}

# This installer accepts only the three checkout-owned code files below. It
# never receives application configuration/credentials and never overwrites.
INSTALLER = r"""
import hashlib,json,os,pathlib,stat,sys
def check(value):
    if not value: raise ValueError('install gate failed')
try:
    check(os.geteuid()==0)
    base=pathlib.Path('/srv/lmm-shadow')
    root=base/'.ldc-operations-20261118'
    data=json.load(sys.stdin)
    expected={
      'stage_legacy_ldc.py':'654e648bee5bcf8cce0a92397c652d48748ef1f5afc54b0693a28acae7ef9480',
      'activate_legacy_ldc.py':'181b457598de365d65eda5128479faaa9af7623a938f7cd30a0d002664697de9',
      'server.py':data['server_sha256']}
    check(set(data['files'])==set(expected))
    info=base.lstat()
    check(stat.S_ISDIR(info.st_mode) and info.st_uid==0 and not info.st_mode&0o022)
    marker=base/'.managed'
    info=marker.lstat()
    check(stat.S_ISREG(info.st_mode) and info.st_uid==0 and not info.st_mode&0o022)
    check(marker.read_text().strip()=='lmm-deployment-v1')
    if not root.exists(): root.mkdir(mode=0o700)
    info=root.lstat()
    check(stat.S_ISDIR(info.st_mode) and info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o700)
    for name in expected:
        value=data['files'][name].encode()
        check(len(value)<=65536 and hashlib.sha256(value).hexdigest()==expected[name])
        path=root/name
        if os.path.lexists(path):
            info=path.lstat()
            check(stat.S_ISREG(info.st_mode) and info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o600)
            check(path.read_bytes()==value)
        else:
            fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
            with os.fdopen(fd,'wb') as f:
                f.write(value);f.flush();os.fsync(f.fileno())
    for directory in (root,base):
        fd=os.open(directory,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        os.fsync(fd);os.close(fd)
    print(json.dumps({'status':'code_installed'}))
except BaseException:
    print(json.dumps({'status':'refused','code':'REMOTE_INSTALL_REFUSED'}))
    sys.exit(1)
"""


class Refused(Exception):
    pass


def require(ok, code):
    if not ok:
        raise Refused(code)


def selected_target(raw):
    targets = json.loads(raw)
    require(isinstance(targets, list), "TARGET_CONFIGURATION_INVALID")
    matches = [item for item in targets if item.get("name") == TARGET]
    require(len(matches) == 1, "FIXED_TARGET_UNAVAILABLE")
    target = matches[0]
    require(target.get("root") == BASE and target.get("user", "root") == "root"
            and target.get("python", "/usr/bin/python3") == "/usr/bin/python3",
            "FIXED_TARGET_CONFIGURATION_MISMATCH")
    require(isinstance(target.get("host"), str)
            and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,252}", target["host"]),
            "TARGET_HOST_INVALID")
    port = int(target.get("port", 22))
    require(1 <= port <= 65535, "TARGET_PORT_INVALID")
    return {"host": target["host"], "port": port, "user": "root"}


def checked_files(directory):
    files = {}
    for name in (*HASHES, "server.py"):
        value = (directory / name).read_bytes()
        require(len(value) <= 65536, "CODE_FILE_TOO_LARGE")
        if name in HASHES:
            require(hashlib.sha256(value).hexdigest() == HASHES[name], "HELPER_HASH_MISMATCH")
        files[name] = value.decode()
    return files


def ssh_arguments(target):
    key, hosts = (Path(os.environ[name]) for name in
                  ("LDC_SSH_KEY_FILE", "LDC_KNOWN_HOSTS_FILE"))
    for path in (key, hosts):
        info = path.lstat()
        require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid()
                and not stat.S_IMODE(info.st_mode) & 0o077, "RUNNER_SSH_FILE_INVALID")
    return ["ssh", "-i", str(key), "-p", str(target["port"]),
            "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes",
            "-o", "StrictHostKeyChecking=yes", "-o", "UserKnownHostsFile=" + str(hosts),
            "-o", "ConnectTimeout=10", "-o", "ConnectionAttempts=1",
            "-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=2",
            "-o", "ControlMaster=no", "-o", "ControlPath=none",
            "--", target["user"] + "@" + target["host"]]


def remote(ssh, argv, stdin=b""):
    # OpenSSH sends a shell command remotely; shlex.join supplies actual POSIX
    # quoting. Neither argv nor stdin can be supplied through dispatch inputs.
    try:
        result = subprocess.run([*ssh, shlex.join(argv)], input=stdin,
                                capture_output=True, timeout=45)
    except (OSError, subprocess.TimeoutExpired):
        raise Refused("SSH_RESULT_UNCERTAIN_USE_VERIFY") from None
    require(len(result.stdout) <= 65536, "REMOTE_RESULT_INVALID")
    try:
        value = json.loads(result.stdout)
        require(isinstance(value, dict), "REMOTE_RESULT_INVALID")
    except (ValueError, TypeError):
        raise Refused("SSH_RESULT_UNCERTAIN_USE_VERIFY") from None
    if result.returncode != 0:
        # The server prints only fixed errors; never relay SSH stderr or raw text.
        code = value.get("code")
        raise Refused(code if isinstance(code, str)
                      and re.fullmatch(r"[A-Z_]{1,80}", code)
                      else "SSH_RESULT_UNCERTAIN_USE_VERIFY")
    return value


def observation(ssh, server_source):
    return remote(ssh, ["/usr/bin/python3", "-", "verify"], server_source.encode())


def observed_ok(value):
    if (value.get("release_verified") is not True
            or value.get("activation_recovery_required")
            or value.get("stage_state_unreadable")):
        return False
    for state in value.get("operations", {}).values():
        if state.get("receipt_unreadable") or state.get("unit_observation_unavailable"):
            return False
        receipt = state.get("receipt", {})
        if (state.get("claimed") and receipt.get("status") != "completed"
                or receipt.get("manual_review_required")
                or receipt.get("recovery_required")):
            return False
    return True


def execute(mode):
    require(mode in MODES, "OPERATION_INVALID")
    target = selected_target(os.environ["DEPLOY_TARGETS"])
    files = checked_files(Path(__file__).resolve().parent)
    ssh = ssh_arguments(target)
    before = observation(ssh, files["server.py"])
    if mode == "verify":
        print(json.dumps(before, sort_keys=True))
        return 0 if observed_ok(before) else 1
    require(observed_ok(before), "VERIFY_REQUIRED_BEFORE_OPERATION")
    payload = {"files": files, "server_sha256": hashlib.sha256(files["server.py"].encode()).hexdigest()}
    remote(ssh, ["/usr/bin/python3", "-c", INSTALLER],
           json.dumps(payload, ensure_ascii=True).encode())
    started = remote(ssh, ["/usr/bin/python3", OPS + "/server.py", "start", mode])
    print(json.dumps(started, sort_keys=True), flush=True)
    deadline = time.monotonic() + 900
    while time.monotonic() < deadline:
        state = observation(ssh, files["server.py"])
        receipt = state.get("operations", {}).get(mode, {}).get("receipt")
        if receipt is not None:
            print(json.dumps(state, sort_keys=True))
            return 0 if receipt.get("status") == "completed" and observed_ok(state) else 1
        time.sleep(3)
    raise Refused("RESULT_PENDING_USE_VERIFY_NEVER_REPLAY")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=MODES, default="verify")
    args = parser.parse_args()
    try:
        return execute(args.mode)
    except BaseException as error:
        code = str(error) if isinstance(error, Refused) else "RUNNER_OPERATION_UNCERTAIN_USE_VERIFY"
        print(json.dumps({"operation": args.mode, "status": "refused", "code": code,
                          "manual_review_required": True, "next_action": "verify"}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
