#!/usr/bin/env python3
"""Keep the preceding Web release's immutable assets in an opted-in fallback."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile

REVISION = re.compile(r"[0-9a-f]{40}")
DIGEST = re.compile(r"[0-9a-f]{64}")
CHUNK = re.compile(r"[A-Za-z0-9_.-]+\.[0-9a-f]{8,}\.[A-Za-z0-9]+(?:\.LICENSE\.txt)?")
PART = re.compile(r"[A-Za-z0-9_-]+")


def asset(root, name):
    parts = name.split("/")
    if not all(PART.fullmatch(part) for part in parts[:-1]) or not CHUNK.fullmatch(parts[-1]):
        raise ValueError("invalid immutable asset path")
    path = root
    for part in parts:
        path = path / part
        if path.is_symlink():
            raise ValueError("asset path contains a symbolic link")
    return path


def digest(path):
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError("asset is not a regular file")
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_receipt(receipt, files):
    with tempfile.NamedTemporaryFile(mode="w", dir=receipt.parent, delete=False) as stream:
        try:
            json.dump(files, stream, sort_keys=True)
            stream.flush()
            os.fsync(stream.fileno())
            os.replace(stream.name, receipt)
        finally:
            Path(stream.name).unlink(missing_ok=True)


def retain(slot, fallback, check_only=False):
    if slot.is_symlink() or fallback.is_symlink() or not (fallback / "static").is_dir():
        raise ValueError("expected a real release slot and an existing fallback")
    static = fallback / "static"
    receipt = fallback / "ci-web-retained.json"
    if static.is_symlink() or receipt.is_symlink():
        raise ValueError("retention paths must not be symbolic links")
    manifest = json.loads((slot / "MANIFEST.json").read_text())
    revision = manifest["revision"]
    if manifest["component"] != "web" or not REVISION.fullmatch(revision):
        raise ValueError("expected a verified Web manifest")

    existing = json.loads(receipt.read_text()) if receipt.exists() else {}
    sources = slot / "dist" / "static"
    if sources.is_symlink() or sources.parent.is_symlink():
        raise ValueError("release asset root must not be a symbolic link")
    files = {name.removeprefix("dist/static/"): metadata["sha256"]
             for name, metadata in manifest["files"].items() if name.startswith("dist/static/")}
    if not files:
        raise ValueError("release has no immutable assets")

    # Verify every file and conflict before creating a link or pruning an older asset.
    additions = {}
    for name, expected in (existing | files).items():
        destination = asset(static, name)
        if not DIGEST.fullmatch(expected):
            raise ValueError("malformed asset digest")
        if name in files and digest(asset(sources, name)) != expected:
            raise ValueError("release asset differs from its manifest")
        if destination.exists():
            if digest(destination) != expected or existing.get(name, expected) != expected:
                raise ValueError("fallback asset collision or unexpected modification")
        elif name in files:
            additions[name] = expected

    managed = existing | additions
    stale = existing.keys() - files.keys()
    if check_only:
        return {"revision": revision, "available": len(files), "would_add": len(additions)}
    if additions:
        # Record ownership before linking so an interrupted attempt can be retried.
        write_receipt(receipt, managed)
    for name in additions:
        destination = asset(static, name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        os.link(asset(sources, name), destination, follow_symlinks=False)
    for name in stale:
        asset(static, name).unlink(missing_ok=True)
        del managed[name]
    if stale:
        write_receipt(receipt, managed)
    return {"revision": revision, "available": len(files), "added": len(additions),
            "removed": len(stale), "managed": len(managed)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("slot", type=Path)
    parser.add_argument("fallback", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.fallback.is_symlink() or not args.fallback.is_dir():
        parser.error("expected an existing fallback directory")
    if args.check:
        print(json.dumps(retain(args.slot, args.fallback, check_only=True)))
    else:
        lock = args.fallback / ".ci-web-retained.lock"
        with os.fdopen(os.open(lock, os.O_CREAT | os.O_NOFOLLOW | os.O_WRONLY, 0o600), "w") as stream:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            print(json.dumps(retain(args.slot, args.fallback)))
