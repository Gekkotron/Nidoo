#!/usr/bin/env python3
"""Bumps the workspace patch version: 0.1.4 -> 0.1.5.

Rewrites `[workspace.package] version` in Cargo.toml and the version of every
workspace crate in Cargo.lock and satellite/mac/Cargo.lock (the macOS shell
pulls nidoo-satellite and nidoo-core by path). Prints the new version.
Run from the repo root.
"""

import re
import sys
from pathlib import Path

LOCKS = [Path("Cargo.lock"), Path("satellite/mac/Cargo.lock")]


def bump_manifest(manifest: str) -> tuple[str, str, str]:
    """The manifest with its [workspace.package] patch version bumped, and (old, new)."""
    section = re.search(r"^\[workspace\.package\]\n(.*?)(?=^\[|\Z)", manifest, re.M | re.S)
    if not section:
        sys.exit("bump_version: no [workspace.package] in Cargo.toml")
    found = re.search(r'^version = "(\d+)\.(\d+)\.(\d+)"$', section.group(1), re.M)
    if not found:
        sys.exit("bump_version: no x.y.z version in [workspace.package]")
    major, minor, patch = found.groups()
    old = f"{major}.{minor}.{patch}"
    new = f"{major}.{minor}.{int(patch) + 1}"
    start = section.start(1) + found.start()
    end = section.start(1) + found.end()
    return manifest[:start] + f'version = "{new}"' + manifest[end:], old, new


def member_names() -> set[str]:
    names = set()
    for manifest in Path("crates").glob("*/Cargo.toml"):
        name = re.search(r'^name = "([^"]+)"$', manifest.read_text(), re.M)
        if name:
            names.add(name.group(1))
    return names


def bump_lock(text: str, names: set[str], old: str, new: str) -> str:
    """Sets `new` on each [[package]] of `names` at `old` with no `source` (a path crate)."""
    blocks = text.split("\n[[package]]\n")
    for i, block in enumerate(blocks):
        name = re.search(r'^name = "([^"]+)"$', block, re.M)
        if not name or name.group(1) not in names or re.search(r"^source = ", block, re.M):
            continue
        blocks[i] = block.replace(f'\nversion = "{old}"\n', f'\nversion = "{new}"\n', 1)
    return "\n[[package]]\n".join(blocks)


def main() -> None:
    manifest_path = Path("Cargo.toml")
    manifest, old, new = bump_manifest(manifest_path.read_text())
    manifest_path.write_text(manifest)
    names = member_names()
    for lock in LOCKS:
        if lock.exists():
            lock.write_text(bump_lock(lock.read_text(), names, old, new))
    print(new)


if __name__ == "__main__":
    main()
