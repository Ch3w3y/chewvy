#!/usr/bin/env python3
"""Collect the licence texts of the Rust crates a build links.

Usage:
  python3 scripts/gen_third_party_licenses.py --target x86_64-unknown-linux-gnu --out FILE

Asks `cargo tree` which crates the two binaries compile for that target, then
copies each crate's own licence, copying and notice files out of the local
crate sources (CARGO_HOME/registry/src). Identical texts are printed once, with
the crates that ship them. Packages include the result so the notices the
MIT, BSD, Apache and similar licences require travel with the binaries.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CARGO_HOME = Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")
PACKAGES = ["plex-app", "plex-cli"]
LICENCE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE|COPYRIGHT)", re.IGNORECASE)
TREE_LINE = re.compile(r"^(\S+) v(\S+)( \(.*\))?$")


def compiled_crates(target: str) -> list[tuple[str, str]]:
    """(name, version) of every registry crate linked into the binaries."""
    cmd = ["cargo", "tree", "--locked", "--target", target, "--edges", "normal,build",
           "--prefix", "none", "--format", "{p}"]
    for package in PACKAGES:
        cmd += ["-p", package]
    out = subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, text=True).stdout
    crates = set()
    for line in out.splitlines():
        match = TREE_LINE.match(line.strip())
        # A path in the trailing parentheses marks a workspace member.
        if match and "/" not in (match.group(3) or ""):
            crates.add((match.group(1), match.group(2)))
    return sorted(crates)


def source_dir(name: str, version: str) -> Path | None:
    for registry in sorted((CARGO_HOME / "registry" / "src").glob("*")):
        candidate = registry / f"{name}-{version}"
        if candidate.is_dir():
            return candidate
    return None


def declared_licence(crate_dir: Path) -> str:
    manifest = (crate_dir / "Cargo.toml").read_text(encoding="utf-8", errors="replace")
    match = re.search(r'^license\s*=\s*"([^"]*)"', manifest, re.MULTILINE)
    return match.group(1) if match else "see the crate's licence file"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--target", required=True, help="Rust target triple of the build")
    parser.add_argument("--out", required=True, type=Path, help="file to write")
    args = parser.parse_args()

    texts: dict[str, tuple[str, list[str]]] = {}
    index, missing = [], []
    for name, version in compiled_crates(args.target):
        crate_dir = source_dir(name, version)
        if crate_dir is None:
            missing.append(f"{name} {version}")
            continue
        index.append(f"{name} {version}: {declared_licence(crate_dir)}")
        for path in sorted(crate_dir.iterdir()):
            if path.is_file() and LICENCE_FILE.match(path.name):
                text = path.read_text(encoding="utf-8", errors="replace").strip()
                key = hashlib.sha256(" ".join(text.split()).encode()).hexdigest()
                texts.setdefault(key, (text, []))[1].append(f"{name} {version} ({path.name})")
    if missing:
        print("crate sources not found under", CARGO_HOME, "for:", ", ".join(missing), file=sys.stderr)
        return 1

    lines = ["Chewvy: licences of the Rust crates linked into this build",
             f"Target: {args.target}", "",
             "Each crate is listed with the licence its manifest declares, followed by",
             "the licence, copying and notice texts the crates ship. A text shared by",
             "several crates is printed once.", "", "CRATES", "------", *index, ""]
    for text, owners in sorted(texts.values(), key=lambda item: item[1][0]):
        lines += ["=" * 78, "Shipped by: " + "; ".join(owners), "=" * 78, "", text, ""]
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines), encoding="utf-8")
    print(f"{args.out}: {len(index)} crates, {len(texts)} distinct texts")
    return 0


if __name__ == "__main__":
    sys.exit(main())
