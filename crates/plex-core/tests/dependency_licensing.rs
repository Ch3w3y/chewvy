//! Dependency/licensing gate (accepted plan: Phase 1 "Dependency + licensing
//! gate").
//!
//! What is checked here, and what is *not*:
//!
//! * `Cargo.lock` is the authoritative version record for the offline build, but
//!   the Cargo 1.99 lock format (`version = 4`) carries no license field. The
//!   license column of docs/DEPENDENCIES.md is therefore the SPDX expression that
//!   each crate version declares in its own `Cargo.toml`, read offline from the
//!   crate cache when the generator ran (see scripts/gen_dependencies.py).
//! * This test can re-derive *coverage* (every locked package is recorded) and
//!   *license policy* (no copyleft-only crate) from tracked files alone, so those
//!   are hard assertions.
//! * Re-reading the crate cache is done only when a crate cache is present in
//!   the current `CARGO_HOME`; when a build uses another `CARGO_HOME` the license
//!   cross-check is skipped and reported, never faked.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
#![allow(clippy::collapsible_if, clippy::len_zero, clippy::manual_let_else)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const MEMBERS: [&str; 5] = [
    "plex-core",
    "jellyfin-core",
    "plex-player",
    "plex-app",
    "plex-cli",
];
/// Declared by a member but deliberately not compiled by the default feature
/// set; each entry must be justified in docs/DEPENDENCIES.md.
///
/// `windows-sys` is not optional: it is `plex-app`'s Windows-target-only
/// dependency (the startup failure dialog), so the host-target `cargo tree` that
/// Table A/B is generated from never compiles it, while the Windows cross-target
/// gate does.
const ALLOWED_OFF_BY_DEFAULT: [&str; 2] = ["bindgen", "windows-sys"];
/// Crates whose license expression contains a copyleft component. They are only
/// acceptable because the expression offers a permissive option, and each one has
/// to be named in the "License notes" section of docs/DEPENDENCIES.md.
const BEGIN_BUILT: &str = "<!-- deps:begin:built -->";
const END_BUILT: &str = "<!-- deps:end:built -->";
const BEGIN_UNUSED: &str = "<!-- deps:begin:locked-unused -->";
const END_UNUSED: &str = "<!-- deps:end:locked-unused -->";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    name: String,
    version: String,
    license: String,
    role: String,
}

fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..6 {
        if dir.join("Cargo.lock").exists() && dir.join("docs").join("DEPENDENCIES.md").exists() {
            return dir;
        }
        let Some(parent) = dir.parent() else { break };
        dir = parent.to_path_buf();
    }
    panic!(
        "could not find Cargo.lock + docs/DEPENDENCIES.md above {}",
        env!("CARGO_MANIFEST_DIR")
    );
}

/// name|version|source for every locked package.
fn locked_packages(root: &Path) -> Vec<(String, String, String)> {
    let text =
        std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock must be readable");
    let mut out = Vec::new();
    let mut name = String::new();
    let mut version = String::new();
    let mut source = String::new();
    let flush = |name: &mut String,
                 version: &mut String,
                 source: &mut String,
                 out: &mut Vec<(String, String, String)>| {
        if !name.is_empty() && !version.is_empty() {
            out.push((name.clone(), version.clone(), source.clone()));
        }
        name.clear();
        version.clear();
        source.clear();
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line == "[[package]]" {
            flush(&mut name, &mut version, &mut source, &mut out);
            continue;
        }
        if line.starts_with('[') {
            flush(&mut name, &mut version, &mut source, &mut out);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "name" => name = value.trim().trim_matches('"').to_owned(),
            "version" => version = value.trim().trim_matches('"').to_owned(),
            "source" => source = value.trim().trim_matches('"').to_owned(),
            _ => {}
        }
    }
    flush(&mut name, &mut version, &mut source, &mut out);
    out
}

fn markdown_rows(text: &str, begin: &str, end: &str) -> Vec<Row> {
    let Some((_before, rest)) = text.split_once(begin) else {
        panic!("missing {begin} sentinel in docs/DEPENDENCIES.md");
    };
    let region = match rest.split_once(end) {
        Some((region, _)) => region,
        None => panic!("missing {end} sentinel in docs/DEPENDENCIES.md"),
    };
    let mut rows = Vec::new();
    for raw in region.lines() {
        let line = raw.trim();
        if !line.starts_with("|") || !line.ends_with("|") {
            continue;
        }
        let cells: Vec<String> = line
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_owned())
            .collect();
        if cells.len() < 5 {
            continue;
        }
        if cells[0] == "crate" || cells[0].chars().all(|c| c == '-') || cells[0].is_empty() {
            continue;
        }
        rows.push(Row {
            name: cells[0].clone(),
            version: cells[1].clone(),
            license: cells[2].clone(),
            role: cells[3].clone(),
        });
    }
    rows
}

fn key(row: &Row) -> String {
    format!("{}|{}", row.name, row.version)
}

/// Crates declared in a dependency section of a workspace member manifest.
fn declared_by_members(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for member in MEMBERS {
        let manifest = root.join("crates").join(member).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
        let mut section: Option<String> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if line.starts_with('[') && line.ends_with(']') {
                let name = line[1..line.len() - 1].trim_matches('\'').to_owned();
                section = if name.ends_with("dependencies") || name.ends_with("dependencies.*") {
                    Some(name)
                } else {
                    None
                };
                continue;
            }
            if section.is_none() {
                continue;
            }
            let Some((key, _)) = line.split_once('=') else {
                continue;
            };
            let declared = key.trim().trim_matches('"').to_owned();
            if !declared.is_empty() && !MEMBERS.contains(&declared.as_str()) {
                out.insert(declared);
            }
        }
    }
    out
}

/// SPDX-style expressions are accepted when at least one disjunctive option is
/// free of a copyleft marker.
fn permissive_option_count(expression: &str) -> usize {
    expression
        .split(" OR ")
        .filter(|option| {
            let lower = option.to_lowercase();
            !(lower.contains("gpl")
                || lower.contains("agpl")
                || lower.contains("lgpl")
                || lower.contains("sspl"))
        })
        .count()
}

fn copyleft_options(expression: &str) -> Vec<String> {
    expression
        .split(" OR ")
        .filter(|option| {
            let lower = option.to_lowercase();
            lower.contains("gpl")
                || lower.contains("agpl")
                || lower.contains("lgpl")
                || lower.contains("sspl")
        })
        .map(|option| option.to_owned())
        .collect()
}

fn notes_section(text: &str) -> &str {
    let (_, rest) = text
        .split_once("## License notes")
        .expect("docs/DEPENDENCIES.md must have a '## License notes' section");
    match rest.find("\n## ") {
        Some(offset) => &rest[..offset],
        None => rest,
    }
}

/// The hand-maintained native (non-crate) table only.
///
/// Table A/Table B also contain rows named `wayland-client`, `libmpv` etc., but
/// those rows record crate licenses, not the verification status of a native
/// component, so the verification-scope assertion must not match them.
fn native_section(text: &str) -> &str {
    let (_, rest) = text
        .split_once("## Native (non-crate) components")
        .expect("docs/DEPENDENCIES.md must have a '## Native (non-crate) components' section");
    match rest.find("\n## ") {
        Some(offset) => &rest[..offset],
        None => rest,
    }
}

fn all_recorded(text: &str) -> (Vec<Row>, Vec<Row>) {
    (
        markdown_rows(text, BEGIN_BUILT, END_BUILT),
        markdown_rows(text, BEGIN_UNUSED, END_UNUSED),
    )
}

/// License declared in the `[package]` table of a cached crate manifest,
/// `<CARGO_HOME>/registry/<index>/<name>-<version>/Cargo.toml`.
fn cached_license(src_dir: &Path, name: &str, version: &str) -> Option<String> {
    let stem = format!("{name}-{version}");
    for registry in std::fs::read_dir(src_dir).ok()? {
        let Ok(registry) = registry else { continue };
        let manifest = registry.path().join(&stem).join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let mut section = String::from("package");
        for raw in text.lines() {
            let line = raw.trim();
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim_matches('\'').to_owned();
                continue;
            }
            if section != "package" {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "license" {
                    return Some(value.trim().trim_matches('"').to_owned());
                }
            }
        }
    }
    None
}

#[test]
fn every_locked_package_is_recorded_exactly_once() {
    let root = workspace_root();
    let text = std::fs::read_to_string(root.join("docs").join("DEPENDENCIES.md"))
        .expect("docs/DEPENDENCIES.md");
    let locked = locked_packages(&root);
    assert!(
        locked.len() > 200,
        "Cargo.lock looks truncated: {} packages",
        locked.len()
    );
    let (built, unused) = all_recorded(&text);
    let recorded: BTreeSet<String> = built.iter().chain(unused.iter()).map(key).collect();
    assert_eq!(
        recorded.len(),
        built.len() + unused.len(),
        "a crate/version appears in both tables (or a duplicate row was hand-edited in)"
    );
    let locked_keys: BTreeSet<String> = locked.iter().map(|(n, v, _)| format!("{n}|{v}")).collect();
    assert_eq!(
        locked_keys.len(),
        locked.len(),
        "Cargo.lock has a duplicated name+version entry"
    );
    let missing: Vec<&String> = locked_keys
        .iter()
        .filter(|k| !recorded.contains(*k))
        .collect();
    let extra: Vec<&String> = recorded
        .iter()
        .filter(|k| !locked_keys.contains(*k))
        .collect();
    assert!(missing.is_empty(), "{missing:?} are locked in Cargo.lock but absent from docs/DEPENDENCIES.md (regenerate with scripts/gen_dependencies.py)");
    assert!(
        extra.is_empty(),
        "{extra:?} are listed in docs/DEPENDENCIES.md but not locked in Cargo.lock"
    );
    assert!(
        !built.is_empty() && !unused.is_empty(),
        "both tables must be populated"
    );
}

#[test]
fn every_recorded_crate_states_an_spdx_expression() {
    let root = workspace_root();
    let text = std::fs::read_to_string(root.join("docs").join("DEPENDENCIES.md"))
        .expect("docs/DEPENDENCIES.md");
    let (built, unused) = all_recorded(&text);
    for row in built.iter().chain(unused.iter()) {
        assert!(
            row.license != "UNRESOLVED" && !row.license.is_empty(),
            "{} {} has no license record in docs/DEPENDENCIES.md",
            row.name,
            row.version
        );
        assert!(
            row.license.starts_with("license:")
                || row.license.contains("license-file")
                || MEMBERS.contains(&row.name.as_str()),
            "{} {} license cell {:?} is not a manifest-declared license",
            row.name,
            row.version,
            row.license
        );
    }
}

#[test]
fn no_copyleft_only_dependency_is_recorded_and_copyleft_options_are_documented() {
    let root = workspace_root();
    let text = std::fs::read_to_string(root.join("docs").join("DEPENDENCIES.md"))
        .expect("docs/DEPENDENCIES.md");
    let (built, unused) = all_recorded(&text);
    let notes = notes_section(&text);
    let mut documented = Vec::new();
    for row in built.iter().chain(unused.iter()) {
        if MEMBERS.contains(&row.name.as_str()) {
            // The workspace's own license is the reviewed decision recorded in the notes.
            assert_eq!(
                row.license, "GPL-3.0-or-later",
                "{} must keep the reviewed workspace license",
                row.name
            );
            assert!(
                notes.contains("GPL-3.0-or-later"),
                "license notes must record the workspace license decision"
            );
            continue;
        }
        let expression = row
            .license
            .strip_prefix("license: ")
            .unwrap_or(&row.license);
        assert!(
            permissive_option_count(expression) >= 1,
            "{} {} ({}) offers no non-copyleft option; rejected by the licensing policy",
            row.name,
            row.version,
            expression
        );
        let copyleft = copyleft_options(expression);
        if !copyleft.is_empty() {
            assert!(
                notes.contains(&format!("`{}`", row.name)),
                "{} ({}) has a copyleft option {:?}; docs/DEPENDENCIES.md 'License notes' must name it explicitly",
                row.name,
                row.version,
                copyleft
            );
            documented.push((row.name.clone(), copyleft.join("/")));
        }
    }
    assert!(
        documented.iter().any(|(name, _)| name == "self_cell") && documented.iter().any(|(name, _)| name == "r-efi"),
        "the gate expects the observed copyleft-option crates (self_cell, r-efi) to still be in the graph: {documented:?}"
    );
}

#[test]
fn declared_dependencies_are_compiled_unless_explicitly_off_by_default() {
    let root = workspace_root();
    let text = std::fs::read_to_string(root.join("docs").join("DEPENDENCIES.md"))
        .expect("docs/DEPENDENCIES.md");
    let (built, unused) = all_recorded(&text);
    let built_names: BTreeSet<String> = built.iter().map(|r| r.name.clone()).collect();
    let unused_names: BTreeSet<String> = unused.iter().map(|r| r.name.clone()).collect();
    for declared in declared_by_members(&root) {
        if ALLOWED_OFF_BY_DEFAULT.contains(&declared.as_str()) {
            assert!(
                unused_names.contains(&declared),
                "{declared} is declared but off by default: it must stay recorded in the locked-unused table"
            );
            continue;
        }
        assert!(
            built_names.contains(&declared),
            "{declared} is declared by a workspace member but is not in the compiled-dependency table (docs/DEPENDENCIES.md is stale or a member lost the pin)"
        );
    }
    for allowed in ALLOWED_OFF_BY_DEFAULT {
        assert!(
            unused_names.contains(allowed),
            "{allowed} is expected to stay optional/off by default (see docs/DEPENDENCIES.md)"
        );
    }
}

#[test]
fn no_source_is_git_or_vendored_path_outside_the_workspace() {
    let root = workspace_root();
    for (name, version, source) in locked_packages(&root) {
        if source.is_empty() {
            assert!(
                MEMBERS.contains(&name.as_str()),
                "{name} {version} is a path dependency outside the workspace members"
            );
            continue;
        }
        assert!(
            source.starts_with("registry+"),
            "{name} {version} must come from the crates.io registry index, got {source}"
        );
    }
}

#[test]
fn native_components_are_recorded_with_their_licence() {
    let root = workspace_root();
    let text = std::fs::read_to_string(root.join("docs").join("DEPENDENCIES.md"))
        .expect("docs/DEPENDENCIES.md");
    let native = native_section(&text);
    for component in ["libmpv", "EGL", "GL (", "wayland-client", "libxkbcommon"] {
        let row = native
            .lines()
            .find(|line| line.trim_start().starts_with("| ") && line.contains(component))
            .unwrap_or_else(|| {
                panic!("docs/DEPENDENCIES.md must record a native row for {component}")
            });
        assert!(
            row.split('|')
                .filter(|cell| !cell.trim().is_empty())
                .count()
                >= 4,
            "native row for {component} must state its role, licence and packaging, got {row}"
        );
    }
}

#[test]
fn cached_crate_manifests_are_rechecked_when_this_build_provides_a_cache() {
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from);
    let Some(cargo_home) = cargo_home else {
        eprintln!("NOTE: CARGO_HOME is unset; cached crate manifest license cross-check skipped");
        return;
    };
    let src_dir = cargo_home.join("registry").join("src");
    if !src_dir.is_dir() {
        eprintln!(
            "NOTE: no crate cache under {}; license cross-check against crate manifests skipped",
            src_dir.display()
        );
        return;
    }
    let root = workspace_root();
    let text = std::fs::read_to_string(root.join("docs").join("DEPENDENCIES.md"))
        .expect("docs/DEPENDENCIES.md");
    let (built, unused) = all_recorded(&text);
    let mut checked = 0usize;
    for row in built.iter().chain(unused.iter()) {
        if MEMBERS.contains(&row.name.as_str()) {
            continue;
        }
        let found = cached_license(&src_dir, &row.name, &row.version);
        let Some(declared) = found else { continue };
        let recorded = row
            .license
            .strip_prefix("license: ")
            .unwrap_or(&row.license);
        assert_eq!(
            declared, recorded,
            "{} {} license changed in the crate cache ({declared}) vs docs/DEPENDENCIES.md ({recorded})",
            row.name, row.version
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "the crate cache exists at {} but no license could be cross-checked",
        src_dir.display()
    );
    println!("cross-checked {checked} crate-manifest licenses against the offline cache");
}
