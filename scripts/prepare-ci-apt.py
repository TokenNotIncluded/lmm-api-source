#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Prepare the apt sources used by ephemeral Ubuntu CI runners."""
import argparse
import os
import re
from email.parser import Parser
from pathlib import Path
from urllib.parse import urlsplit

RUNNER_UBUNTU_MIRRORS = "mirror+file:/etc/apt/apt-mirrors.txt"
UBUNTU_ARCHIVE = "https://archive.ubuntu.com/ubuntu/"


def unused_runner_source(uri):
    parsed = urlsplit(uri)
    if parsed.scheme not in {"http", "https"}:
        return False
    path = parsed.path.rstrip("/")
    return (
        parsed.hostname == "dl.google.com"
        and path in {"/linux/chrome/deb", "/linux/chrome-stable/deb"}
    ) or (
        parsed.hostname == "packages.microsoft.com"
        and path == "/ubuntu/24.04/prod"
    )


def canonical_ubuntu_uris(block):
    lines = []
    in_uris = False
    for line in block.splitlines(keepends=True):
        if not line.lstrip().startswith("#"):
            field = re.match(r"^URIs:", line, re.I)
            if field:
                in_uris = True
            elif not line[:1].isspace():
                in_uris = False
            if in_uris:
                start = field.end() if field else 0
                line = line[:start] + re.sub(
                    r"\S+", lambda match: UBUNTU_ARCHIVE if match[0] == RUNNER_UBUNTU_MIRRORS else match[0], line[start:]
                )
        lines.append(line)
    return "".join(lines)


def prepare(directory):
    changed = []
    for path in sorted(directory.iterdir()):
        if path.is_symlink() or not path.is_file() or path.suffix not in {".list", ".sources"}:
            continue
        original = path.read_text()
        if path.suffix == ".list":
            lines = []
            for line in original.splitlines(keepends=True):
                match = re.match(r"^\s*deb(?:-src)?\s+(?:\[[^]]*\]\s+)?(\S+)", line)
                if match and unused_runner_source(match[1]):
                    line = "# Unused by LMM CI: " + line
                elif match and match[1] == RUNNER_UBUNTU_MIRRORS:
                    line = line[:match.start(1)] + UBUNTU_ARCHIVE + line[match.end(1):]
                lines.append(line)
            result = "".join(lines)
        else:
            blocks = re.split(r"(\n[ \t]*\n)", original)
            for index in range(0, len(blocks), 2):
                block = blocks[index]
                uris = " ".join(Parser().parsestr(block).get_all("URIs", [])).split()
                if not any(unused_runner_source(uri) for uri in uris):
                    continue
                if not all(unused_runner_source(uri) for uri in uris):
                    raise ValueError("refusing to disable a mixed-source stanza: " + path.name)
                if re.search(r"^Enabled:", block, re.M | re.I):
                    block = re.sub(r"^Enabled:.*$", "Enabled: no", block, flags=re.M | re.I)
                else:
                    block = block.rstrip("\n") + "\nEnabled: no\n"
                blocks[index] = block
            result = canonical_ubuntu_uris("".join(blocks))
        if result != original:
            path.write_text(result)
            changed.append(path.name)
    return changed


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--sources-dir", type=Path, default=Path("/etc/apt/sources.list.d"))
    args = parser.parse_args()
    if os.environ.get("GITHUB_ACTIONS") != "true":
        parser.error("this preparation is limited to GitHub Actions")
    for name in prepare(args.sources_dir):
        print("Prepared CI apt source:", name)
