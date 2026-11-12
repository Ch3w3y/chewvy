#!/usr/bin/env python3
"""Regenerate the dependency tables inside docs/DEPENDENCIES.md.

Reads the committed Cargo.lock (authoritative version + license record) and the
workspace member manifests, and asks `cargo tree` which crates the default-feature
build actually compiles. Only the region between the sentinels is rewritten; the
prose in docs/DEPENDENCIES.md is hand-written and preserved.

Usage:
  source scripts/env.sh
  python3 scripts/gen_dependencies.py --write docs/DEPENDENCIES.md
  python3 scripts/gen_dependencies.py --stdout

Requires cargo on PATH (scripts/env.sh sets it up).
"""

from __future__ import annotations

import argparse
import io
import os
import re
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MEMBERS = ["plex-core", "jellyfin-core", "plex-player", "plex-app", "plex-cli"]
# scripts/env.sh exports CARGO_HOME when a project-local crate cache exists;
# fall back to that location for runs that did not source it.
CARGO_HOME = Path(os.environ.get("CARGO_HOME") or ROOT / ".cargo")
SRC_DIR = CARGO_HOME / "registry" / "src"
CRATE_DIR = CARGO_HOME / "registry" / "cache"
PKG_RE = re.compile(r"^\[\[package\]\]$")
BEGAN = "<!-- deps:begin:{} -->"
ENDED = "<!-- deps:end:{} -->"
SECTION_KEYS = ("built", "locked-unused")


def parse_lock(path: Path) -> dict[str, dict[str, str]]:
    """name|version -> {license, source} for every locked package."""
    packages: dict[str, dict[str, str]] = {}
    cur: dict[str, str] = {}

    def flush() -> None:
        if "name" in cur and "version" in cur:
            key = f"{cur['name']}|{cur['version']}"
            packages[key] = {
                "license": cur.get("license", ""),
                "source": cur.get("source", ""),
            }

    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if PKG_RE.match(line):
            flush()
            cur = {}
            continue
        if line.startswith("["):
            flush()
            cur = {}
            continue
        if "=" in line:
            key, _, value = line.partition("=")
            cur[key.strip()] = value.strip().strip('"')
    flush()
    return packages


def dep_sections(manifest: Path) -> dict[str, list[str]]:
    """dependency-section-name -> crate keys declared there."""
    out: dict[str, list[str]] = {}
    section = None
    for raw in manifest.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if line.startswith("[") and line.endswith("]"):
            name = line[1:-1].strip("'")
            section = name if name.endswith("dependencies") or name.endswith("dependencies.*") else None
            continue
        if section and "=" in line:
            key = line.split("=", 1)[0].strip().strip('"')
            if key:
                out.setdefault(section, []).append(key)
    return out


def direct_deps() -> dict[str, list[tuple[str, str]]]:
    """crate -> [(member, section), ...] for crates declared by a workspace member."""
    direct: dict[str, list[tuple[str, str]]] = {}
    for member in MEMBERS:
        for section, keys in dep_sections(ROOT / "crates" / member / "Cargo.toml").items():
            for key in keys:
                if key in MEMBERS:
                    continue
                direct.setdefault(key, []).append((member, section))
    return direct


def built_set() -> set[str]:
    """name|version pairs that `cargo tree` says the default-feature build compiles."""
    cmd = [
        "cargo",
        "tree",
        "--offline",
        "--locked",
        "--edges",
        "normal,build,dev",
        "--prefix",
        "none",
    ]
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.exit(
            "cargo tree failed (is scripts/env.sh sourced and the lock current?):\n"
            + (proc.stderr or proc.stdout)
        )
    built: set[str] = set()
    line_re = re.compile(r"^([A-Za-z0-9_.-]+) v([^ ]+)")
    for line in proc.stdout.splitlines():
        match = line_re.match(line.strip())
        if match:
            built.add(f"{match.group(1)}|{match.group(2)}")
    return built


def package_license(manifest_text: str, inherit: str = "") -> str:
    """license / license-file declared in the [package] table of a Cargo.toml.

    `license.workspace = true` inherits the `[workspace.package]` value, so the
    caller passes that value in as `inherit`.
    """
    section = "package"
    for raw in manifest_text.splitlines():
        line = raw.strip()
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1].strip("'")
            continue
        if section not in ("package", "workspace.package") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        if key in ("license", "license-file"):
            return f"{key}: {value.strip().strip('\"')}"
        if key.startswith("license") and key.endswith(".workspace"):
            return inherit
    return ""


def registry_license(name: str, version: str) -> tuple[str, str]:
    """(license, where it was read from) using only the offline crate cache."""
    stem = f"{name}-{version}"
    if SRC_DIR.is_dir():
        for crate_dir in SRC_DIR.glob(f"*/{stem}"):
            manifest = crate_dir / "Cargo.toml"
            if manifest.is_file():
                found = package_license(manifest.read_text(encoding="utf-8", errors="replace"))
                if found:
                    return found, "cached crate source"
    if CRATE_DIR.is_dir():
        for tarball in CRATE_DIR.glob(f"*/{stem}.crate"):
            with tarfile.open(tarball, "r:gz") as tar:
                member = f"{stem}/Cargo.toml"
                handle = tar.extractfile(member) if tar.getmember(member).isfile() else None
                if handle is not None:
                    found = package_license(io.TextIOWrapper(handle, encoding="utf-8", errors="replace").read())
                    if found:
                        return found, "cached .crate manifest"
    return "", "unresolved"


def source_kind(source: str) -> str:
    if not source:
        return "workspace member (path)"
    if source.startswith("registry+"):
        return "crates.io"
    return source


def table(rows: list[tuple[str, str, str, str, str]], header: tuple[str, ...]) -> list[str]:
    lines = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    for cells in rows:
        lines.append("| " + " | ".join(cells) + " |")
    return lines


def sorted_keys(packages: dict[str, dict[str, str]]) -> list[str]:
    return sorted(packages, key=lambda k: (k.split("|")[0], k.split("|")[1]))


def render(packages: dict[str, dict[str, str]], built: set[str], direct: dict) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    root_manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    workspace_license = package_license(root_manifest).split(": ", 1)[-1]

    built_rows = []
    for key in sorted_keys(packages):
        name, version = key.split("|")
        meta = packages[key]
        if key not in built:
            continue
        where = direct.get(name)
        if name in MEMBERS:
            who = "workspace member target"
        elif where:
            who = ", ".join(f"{m} ({s})" for m, s in sorted(where))
        else:
            who = "transitive"
        if meta["source"]:
            license_text, _origin = registry_license(name, version)
        else:
            manifest = ROOT / "crates" / name / "Cargo.toml"
            license_text = (
                package_license(manifest.read_text(encoding="utf-8"), inherit=workspace_license)
                if manifest.is_file()
                else ""
            )
        built_rows.append(
            (name, version, license_text or "UNRESOLVED", who, source_kind(meta["source"]))
        )
    out["built"] = table(
        built_rows,
        ("crate", "locked version", "license (Cargo.lock)", "declared by / graph role", "source"),
    )

    unused_rows = []
    for key in sorted_keys(packages):
        name, version = key.split("|")
        meta = packages[key]
        if key in built:
            continue
        if name in MEMBERS:
            reason = "workspace member declared in Cargo.lock"
        elif name in direct:
            reason = "declared by a member, optional/off by default: not compiled by the default build"
        else:
            reason = "locked via another crate's optional or dev edge: not compiled by the default build"
        license_text, _origin = registry_license(name, version)
        unused_rows.append(
            (name, version, license_text or "UNRESOLVED", reason, source_kind(meta["source"]))
        )
    out["locked-unused"] = table(
        unused_rows,
        ("crate", "locked version", "license (Cargo.lock)", "graph role", "source"),
    )
    return out


def splice(path: Path, sections: dict[str, list[str]]) -> None:
    text = path.read_text(encoding="utf-8")
    for key, rows in sections.items():
        begin, end = BEGAN.format(key), ENDED.format(key)
        if begin not in text or end not in text:
            sys.exit(f"missing sentinels {begin!r}/{end!r} in {path}")
        head, rest = text.split(begin, 1)
        _, tail = rest.split(end, 1)
        body = "\n" + "\n".join(rows) + "\n"
        text = head + begin + body + end + tail
    path.write_text(text, encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--write", metavar="MARKDOWN")
    ap.add_argument("--stdout", action="store_true")
    ap.add_argument("--summary", action="store_true")
    args = ap.parse_args()

    packages = parse_lock(ROOT / "Cargo.lock")
    built = built_set()
    direct = direct_deps()
    sections = render(packages, built, direct)
    unresolved = sum(1 for rows in sections.values() for line in rows if "UNRESOLVED" in line)
    if args.summary:
        print(f"locked={len(packages)} built={len(built)} locked_unused={len(packages) - len(built)} direct={len(direct)} unresolved_licenses={unresolved}")
    if args.write:
        splice(Path(args.write), sections)
        print(f"updated {args.write}")
    if args.stdout:
        for key in SECTION_KEYS:
            print(f"### {key}")
            print("\n".join(sections[key]))


if __name__ == "__main__":
    main()
