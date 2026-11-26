#!/usr/bin/env python3
"""Prove an update only changes Web files relative to an installed release."""

import argparse
import re
import subprocess


def git(*args):
    return subprocess.run(
        ["git", *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        check=False,
    )


def is_web_only(base, revision):
    for commit in (base, revision):
        if not re.fullmatch(r"[0-9a-f]{40}", commit):
            return False
        resolved = git("rev-parse", "--verify", f"{commit}^{{commit}}")
        if resolved.returncode or resolved.stdout.strip() != commit.encode():
            return False
    if git("merge-base", "--is-ancestor", base, revision).returncode:
        return False
    changed = git("diff", "--no-renames", "--name-only", "-z", base, revision)
    paths = changed.stdout.rstrip(b"\0").split(b"\0")
    return changed.returncode == 0 and all(
        path.startswith(b"apps/web/") for path in paths
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    return 0 if is_web_only(args.base, args.revision) else 1


if __name__ == "__main__":
    raise SystemExit(main())
