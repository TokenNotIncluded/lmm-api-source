#!/usr/bin/env python3
"""Prepare, install, and verify a reviewed channel change in a local Git configuration repository.

propose creates an independent branch without switching or touching the checkout.
After reviewing the diff, fast-forward the release branch using the usual Git review path.
install takes its exact approved SHA and atomically replaces the configured model sidecar.
Run the installation's normal controlled Core restart, then use status against each instance.
"""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import runpy
import stat
import subprocess
import sys
import tempfile
import urllib.request

admin = runpy.run_path(str(Path(__file__).with_name("lmm-admin.py")))
Error = admin["ClientError"]
CONFIG = "/core/v1/admin/routing/configuration"
LIMIT = 16 * 1024 * 1024


def require(ok, message):
    if not ok:
        raise Error(message)


def pretty(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(argv, data=None, env=None):
    child_env = {**os.environ, **(env or {})}
    child_env.pop("LMM_ADMIN_SESSION", None)
    proc = subprocess.run(argv, input=data, capture_output=True, env=child_env)
    if proc.returncode:
        if argv[-1] == "plan-channel-model-change":
            # Native planner emits sanitized errors, never echoes input TOML.
            raise Error("Core rejected proposal: " + proc.stderr.decode(errors="replace").strip()[:400])
        raise Error("Git rejected the operation; verify the ref, paths, author identity and current base")
    return proc.stdout


def git(args, *argv, data=None, env=None):
    return run(["git", "-C", str(args.repo), *argv], data, env)


def branch(args, name):
    ref = "refs/heads/" + name
    git(args, "check-ref-format", ref)
    return ref


def resolve(args, ref):
    return git(args, "rev-parse", "--verify", "--end-of-options", ref + "^{commit}").decode().strip()


def config_file(args, commit, name):
    require(isinstance(name, str) and name and str(PurePosixPath(name)) == name
            and not name.startswith(("/", "-", ":")) and ".." not in PurePosixPath(name).parts
            and not any(ord(c) < 32 for c in name), "Invalid tracked model config path")
    metadata = git(args, "ls-tree", "-z", commit, "--", ":(literal)" + name)
    info, _, path = metadata.rstrip(b"\0").partition(b"\t")
    require(path == name.encode() and info.startswith(b"100644 blob "), "Model config must be a tracked regular nonexecutable file")
    raw = git(args, "cat-file", "blob", commit + ":" + name)
    require(len(raw) <= LIMIT, "Model config exceeds Core limit")
    return raw


def observed(args):
    origin = admin["origin_url"](args.instance, args.allow_insecure_loopback)
    token = admin["session_token"](args)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), admin["NoRedirect"]())
    request = urllib.request.Request(origin + CONFIG, headers={"Authorization": "Bearer " + token, "Accept": "application/json", "Cache-Control": "no-cache"})
    try:
        with opener.open(request, timeout=10) as response:
            require(response.headers.get_content_type() == "application/json", "Unexpected Core response")
            envelope = json.loads(admin["read_bounded"](response, LIMIT * 3, 10))
    except (OSError, ValueError) as exc:
        raise Error("Core readback unavailable; effective status is unknown") from exc
    require(isinstance(envelope, dict) and envelope.get("success") is True and isinstance(envelope.get("data"), dict), "Invalid Core readback")
    authority = envelope["data"].get("authority") or {}
    require(isinstance(authority, dict) and authority.get("mode") == "file" and authority.get("source") == "model.config_file",
            "Core must report model.config_file authority")
    version = authority.get("version")
    require(isinstance(version, dict) and version == envelope["data"].get("version")
            and all(type(version.get(k)) is int and version[k] > 0 for k in ("routing", "pricing")),
            "Core reported inconsistent active versions")
    proof = {"version": version}
    for field in ("routing_sha256", "pricing_sha256"):
        digest = authority.get(field)
        require(isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest),
                "Core did not report active routing and pricing digests")
        proof[field] = digest
    return proof


def plan(args, source, selection, local_http):
    payload = {"model_config": source.decode(), "plan": selection, "local_http": local_http}
    return json.loads(run([args.core, "plan-channel-model-change"], pretty(payload)))


def native_state(result, after):
    return result["after" if after else "before"]


def propose(args):
    current_ref = branch(args, args.release_branch)
    proposal_ref = branch(args, args.branch)
    base = resolve(args, current_ref)
    raw = config_file(args, base, args.config)
    if hasattr(args, "selection"):
        selection = args.selection
    else:
        with args.plan.open("rb") as handle:
            data = handle.read(LIMIT + 1)
        require(len(data) <= LIMIT, "Plan exceeds the Core input limit")
        selection = json.loads(data)
    result = plan(args, raw, selection, args.local_http)
    before, after = native_state(result, False), native_state(result, True)
    require(observed(args) == before, "Core has diverged from the Git base; recompute against its current source")
    candidate = result["model_config"].encode()
    preview = result["preview"]
    card = {"format": 1, "base_commit": base, "config": args.config, "local_http": args.local_http,
            "selection": selection, "before": before, "after": after, "preview": preview,
            "base_file_sha256": sha(raw), "candidate_file_sha256": sha(candidate)}
    receipt = "channel-changes/" + sha(pretty(card)) + ".json"
    with tempfile.TemporaryDirectory() as tmp:
        index = {"GIT_INDEX_FILE": str(Path(tmp) / "index")}
        git(args, "read-tree", base, env=index)
        for path, data in ((args.config, candidate), (receipt, pretty(card))):
            blob = git(args, "hash-object", "-w", "--stdin", data=data).decode().strip()
            git(args, "update-index", "--add", "--cacheinfo", "100644", blob, path, env=index)
        tree = git(args, "write-tree", env=index).decode().strip()
    commit = git(args, "commit-tree", tree, "-p", base, "-m", f"Plan channel {preview['channel_id']} model routing").decode().strip()
    instruction = f"start\nverify {current_ref} {base}\ncreate {proposal_ref} {commit}\nprepare\ncommit\n"
    git(args, "update-ref", "--stdin", data=instruction.encode())
    return {"state": "awaiting_review", "commit": commit, "branch": args.branch, "card": receipt,
            "priced_bindings": len(preview["candidates"]), "draft_bindings": preview["draft_bindings"]}


def proposal(args):
    commit = args.commit
    require(re.fullmatch("[0-9a-f]{40}|[0-9a-f]{64}", commit), "Supply the complete reviewed Git commit SHA")
    parents = git(args, "rev-list", "--parents", "-n", "1", commit).decode().split()
    require(len(parents) == 2 and parents[0] == commit, "Proposal must identify one exact parent")
    base = parents[1]
    files = git(args, "diff-tree", "--no-commit-id", "--name-only", "-z", "-r", base, commit).decode().rstrip("\0").split("\0")
    receipts = [f for f in files if re.fullmatch(r"channel-changes/[0-9a-f]{64}\.json", f)]
    require(len(receipts) == 1, "Missing or ambiguous channel proposal receipt")
    data = config_file(args, commit, receipts[0])
    card = json.loads(data)
    require(receipts[0] == "channel-changes/" + sha(data) + ".json" and card.get("format") == 1
            and card["base_commit"] == base and set(files) <= {receipts[0], card["config"]}, "Invalid proposal commit")
    source = config_file(args, base, card["config"])
    candidate = config_file(args, commit, card["config"])
    require(sha(source) == card["base_file_sha256"] and sha(candidate) == card["candidate_file_sha256"],
            "Sidecar file differs from proposal receipt")
    return card, source, candidate


def current_file(path):
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as handle:
        info = os.fstat(handle.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_size <= LIMIT, "Installed sidecar must be a bounded regular file")
        data = handle.read(LIMIT + 1)
        require(len(data) <= LIMIT, "Installed sidecar exceeds Core size limit")
        return info, data


@contextmanager
def reviewed_ref(args):
    ref = branch(args, args.release_branch)
    # Keep Git's verification lock through the file replacement. A concurrent merge
    # cannot advance the reviewed ref between its comparison and the installation.
    env = os.environ.copy()
    env.pop("LMM_ADMIN_SESSION", None)
    with subprocess.Popen(["git", "-C", str(args.repo), "update-ref", "--stdin"], env=env,
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True) as proc:
        try:
            proc.stdin.write(f"start\nverify {ref} {args.commit}\nprepare\n")
            proc.stdin.flush()
            require(proc.stdout.readline() == "start: ok\n" and proc.stdout.readline() == "prepare: ok\n",
                    "Release branch must point to the exact human-reviewed commit")
            yield
            proc.stdin.write("commit\n")
            proc.stdin.flush()
            require(proc.stdout.readline() == "commit: ok\n", "Git release ref verification failed")
        finally:
            proc.stdin.close()



def install(args):
    card, source, candidate = proposal(args)
    require(candidate != source, "Draft-only or unchanged proposal has no installable routing changes")
    result = plan(args, source, card["selection"], card["local_http"])
    require(result["model_config"].encode() == candidate and result["preview"] == card["preview"]
            and native_state(result, False) == card["before"] and native_state(result, True) == card["after"],
            "Current Core validator differs from the reviewed proposal; recompute")
    path = args.installed
    require(path.is_absolute(), "Pass the absolute configured model sidecar installation path")
    directory = os.open(path.parent, os.O_DIRECTORY)
    try:
        fcntl.flock(directory, fcntl.LOCK_EX)
        with reviewed_ref(args):
            info, installed = current_file(path)
            live = observed(args)
            if installed == candidate:
                require(live in (card["before"], card["after"]), "Core release has diverged; recompute")
            else:
                require(installed == source and live == card["before"], "Installed or running base has changed; recompute")
                with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".channel-", delete=False) as temp:
                    temp_path = temp.name
                    try:
                        temp.write(candidate)
                        temp.flush()
                        if (os.geteuid(), os.getegid()) != (info.st_uid, info.st_gid):
                            os.fchown(temp.fileno(), info.st_uid, info.st_gid)
                        os.fchmod(temp.fileno(), stat.S_IMODE(info.st_mode))
                        os.fsync(temp.fileno())
                        os.replace(temp_path, path)
                        os.fsync(directory)
                    finally:
                        Path(temp_path).unlink(missing_ok=True)
    finally:
        os.close(directory)
    return {"state": "published", "commit": args.commit, "expected": card["after"]}


def status(args):
    card, source, candidate = proposal(args)
    head = resolve(args, branch(args, args.release_branch))
    reviewed = head == args.commit
    installed = current_file(args.installed)[1]
    published = reviewed and source != candidate and installed == candidate
    result = {"commit": args.commit, "instance": args.instance, "reviewed": reviewed,
              "published": published, "effective": None, "expected": card["after"]}
    try:
        actual = observed(args)
    except (Error, OSError, ValueError, KeyError):
        result["state"] = "unknown"
        return result
    result["observed"] = actual
    result["effective"] = published and actual == card["after"]
    if (head not in (args.commit, card["base_commit"]) or installed not in (source, candidate)
            or (not reviewed and actual != card["before"]) or actual not in (card["before"], card["after"])):
        result["state"] = "stale"
    elif not reviewed:
        result["state"] = "awaiting_review"
    elif candidate == source:
        result["state"] = "draft"
    elif result["effective"]:
        result["state"] = "effective"
    else:
        result["state"] = "published" if published else "reviewed"
    return result


def assistant(args):
    data = sys.stdin.buffer.read(65537)
    require(len(data) <= 65536, "Assistant tool arguments exceed limit")
    request = json.loads(data)
    require(isinstance(request, dict), "Invalid assistant tool arguments")
    require(re.fullmatch(r"ai/channel/[0-9a-f]{32}/", args.namespace), "Invalid journal namespace")
    if args.command == "assistant-status":
        require(set(request) == {"channel_id", "commit"} and type(request.get("channel_id")) is int
                and request["channel_id"] > 0 and isinstance(request["commit"], str)
                and re.fullmatch("[0-9a-f]{40}|[0-9a-f]{64}", request["commit"]), "Invalid proposal commit")
        observed(args)
        args.commit = request["commit"]
        refs = git(args, "for-each-ref", "--format=%(refname)", "--points-at=" + args.commit,
                   "refs/heads/" + args.namespace).decode().splitlines()
        require(bool(refs), "Unknown channel proposal for this operator")
        card, _, _ = proposal(args)
        require(card["preview"]["channel_id"] == request["channel_id"], "Channel does not match the proposal")
        return {**status(args), "channel_id": request["channel_id"], "candidate_file_sha256": card["candidate_file_sha256"]}
    require(re.fullmatch("[0-9a-f]{64}", args.operation), "Invalid assistant journal operation")
    args.branch = args.namespace + args.operation
    ref = branch(args, args.branch)
    args.selection = request

    def recover():
        names = git(args, "for-each-ref", "--format=%(refname)", ref).decode().splitlines()
        if ref not in names:
            return None
        args.commit = resolve(args, ref)
        card, _, _ = proposal(args)
        require(card["selection"] == request and card["config"] == args.config
                and card["local_http"] == args.local_http, "Operation already belongs to another selection")
        preview = card["preview"]
        return {"state": "staged", "branch": args.branch, "commit": args.commit, "channel_id": preview["channel_id"],
                "candidate_file_sha256": card["candidate_file_sha256"],
                "priced_bindings": len(preview["candidates"]), "draft_binding_count": len(preview["draft_bindings"]),
                "before": card["before"], "expected": card["after"]}

    # Both fresh execution and post-crash readback require the original operator's live Session.
    observed(args)
    receipt = recover()
    if receipt:
        return receipt
    if args.command == "assistant-inspect":
        return {"state": "unknown"}
    try:
        propose(args)
    except Error:
        receipt = recover()
        return receipt if receipt is not None else {"state": "rejected"}
    receipt = recover()
    require(receipt is not None, "Staged Git receipt unavailable")
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    subs = parser.add_subparsers(dest="command", required=True)
    for name in ("propose", "install", "status", "assistant-stage", "assistant-inspect", "assistant-status"):
        sub = subs.add_parser(name)
        sub.add_argument("--repo", required=True, type=Path, help="Local configuration Git repository")
        sub.add_argument("--release-branch", required=True, help="Git branch accepting reviewed proposals")
        sub.add_argument("--instance", required=True, help="Origin of this Core instance")
        sub.add_argument("--allow-insecure-loopback", action="store_true")
        sub.add_argument("--credential-file", help="Private operator Session bearer file; otherwise reads LMM_ADMIN_SESSION")
        sub.set_defaults(token_stdin=False)
        if name.startswith("assistant-"):
            sub.add_argument("--namespace", required=True)
            if name == "assistant-status":
                sub.add_argument("--installed", required=True, type=Path)
            else:
                sub.add_argument("--core", required=True)
                sub.add_argument("--config", required=True)
                sub.add_argument("--operation", required=True)
                sub.add_argument("--local-http", action="store_true")
            continue
        if name != "status":
            sub.add_argument("--core", default="lmm-core", help="Installed native Core binary")
        if name == "propose":
            sub.add_argument("--branch", required=True, help="New independent proposal branch")
            sub.add_argument("--config", required=True, help="Tracked model sidecar TOML path")
            sub.add_argument("--plan", required=True, type=Path, help="Web-exported Core model selection JSON")
            sub.add_argument("--local-http", action="store_true", help="Model sidecar uses explicit numeric loopback provider mode")
        else:
            sub.add_argument("--commit", required=True, help="Full reviewed proposal SHA")
            sub.add_argument("--installed", required=True, type=Path, help="Existing installed model sidecar")
    args = parser.parse_args()
    action = assistant if args.command.startswith("assistant-") else {"propose": propose, "install": install, "status": status}[args.command]
    result = action(args)
    sys.stdout.buffer.write(pretty(result))
    return int(not args.command.startswith("assistant-") and result["state"] in ("unknown", "stale"))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (Error, OSError, ValueError, KeyError) as error:
        print("Channel change failed:", error if isinstance(error, Error) else type(error).__name__, file=sys.stderr)
        sys.exit(1)
