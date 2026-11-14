//! Executable workspace contracts for milestone M1 (no product logic yet).
//!
//! These tests guard properties that must stay true through every later
//! milestone: GUI-free cores, a single pinned workspace version, a committed
//! version-locked Cargo.lock, and the researched crate versions actually being
//! the locked ones. They are real file/graph assertions, not placeholders.

#![allow(clippy::unwrap_used, clippy::expect_used)] // product code keeps these lints on

use std::path::{Path, PathBuf};

/// Crates that must never appear in a dependency section of a display-free layer.
const DISPLAY_OR_FFI_CRATES: &[&str] = &[
    "egui",
    "epaint",
    "egui-winit",
    "egui_glow",
    "egui-wgpu",
    "eframe",
    "glow",
    "glow-sys",
    "glutin",
    "glutin-winit",
    "winit",
    "wgpu",
    "pollster",
    "wayland-client",
    "wayland-protocols",
    "x11rb",
    "arboard",
    "raw-window-handle",
    "bindgen",
    "pkg-config",
    "ash",
];

/// Collects dependency keys from every `[...dependencies]` section of a manifest.
/// Section-aware, so prose that mentions a crate name is not mistaken for a
/// dependency declaration.
fn dependency_keys(manifest: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
    let mut keys = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            let section = line
                .trim_matches(|c| c == '[' || c == ']')
                .trim_matches('\'')
                .to_owned();
            in_deps = section.ends_with("dependencies") || section.ends_with("dependencies.*");
            continue;
        }
        if in_deps {
            if let Some((key, _)) = line.split_once('=') {
                let key = key.trim().trim_matches('"').to_owned();
                if !key.is_empty() && !keys.contains(&key) {
                    keys.push(key);
                }
            }
        } else if !line.is_empty() && !line.starts_with('#') && !line.contains('=') {
            // Anything else in a non-dependency table is ignored.
        }
    }
    keys
}

fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..6 {
        if dir.join("Cargo.lock").exists() && dir.join("Cargo.toml").exists() {
            return dir;
        }
        let parent = match dir.parent() {
            Some(parent) => parent.to_path_buf(),
            None => break,
        };
        dir = parent;
    }
    panic!(
        "could not locate the workspace root above {}",
        env!("CARGO_MANIFEST_DIR")
    );
}

/// Every version locked for `crate_name`. Cargo.lock legitimately locks more than
/// one version of 31 crates here (`thiserror` 1.0.69 via calloop/smithay-client-toolkit/
/// vello_common while `plex-core` pins 2.0.20), so a first-match lookup is unreliable:
/// a pin is satisfied when the pinned version is one of the locked name+version pairs.
fn locked_versions(lock: &str, crate_name: &str) -> Vec<String> {
    let needle = format!("name = \"{crate_name}\"");
    let mut versions = Vec::new();
    let mut lines = lock.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() == needle {
            let version = lines
                .next()
                .unwrap_or_else(|| panic!("no version line after {crate_name} in Cargo.lock"));
            versions.push(
                version
                    .trim()
                    .trim_start_matches("version = ")
                    .trim_matches('"')
                    .to_owned(),
            );
        }
    }
    assert!(!versions.is_empty(), "{crate_name} is not in Cargo.lock");
    versions
}

#[test]
fn workspace_version_and_license_are_pinned() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.3.0");
    assert_eq!(env!("CARGO_PKG_LICENSE"), "GPL-3.0-or-later");
}

#[test]
fn plex_core_has_no_display_or_ffi_dependency() {
    let keys = dependency_keys(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    assert!(
        !keys.is_empty(),
        "expected declared dependencies in plex-core"
    );
    for key in &keys {
        assert!(
            !DISPLAY_OR_FFI_CRATES.contains(&key.as_str()),
            "plex-core must stay display-free, but depends on {key}"
        );
    }
    // The researched core stack has to be wired, not just available.
    for expected in ["ureq", "serde", "serde_json", "thiserror", "zbus", "rustls"] {
        assert!(
            keys.contains(&expected.to_owned()),
            "missing {expected} in plex-core"
        );
    }
}

#[test]
fn cargo_lock_is_committed_and_pins_the_researched_versions() {
    let root = workspace_root();
    let lock = std::fs::read_to_string(root.join("Cargo.lock"))
        .unwrap_or_else(|e| panic!("Cargo.lock must be committed at the workspace root: {e}"));
    assert!(
        lock.contains("[[package]]"),
        "Cargo.lock is not a Cargo-generated lock file"
    );
    assert!(
        lock.contains("name = \"plex-core\""),
        "workspace members must be part of the locked graph"
    );
    for (name, version) in [
        ("ureq", "3.3.0"),
        ("url", "2.5.8"),
        ("serde", "1.0.229"),
        ("serde_json", "1.0.151"),
        ("thiserror", "2.0.20"),
        ("rustls", "0.23.44"),
        ("zbus", "5.19.0"),
        ("tiny_http", "0.12.0"),
        ("egui", "0.36.1"),
        ("glow", "0.17.0"),
        ("glutin", "0.32.3"),
        ("glutin-winit", "0.5.0"),
        ("winit", "0.30.13"),
        ("clap", "4.6.6"),
        ("bindgen", "0.72.1"),
        ("pkg-config", "0.3.34"),
    ] {
        let locked = locked_versions(&lock, name);
        assert!(
            locked.iter().any(|v| v.as_str() == version),
            "Cargo.lock must pin {name} = {version} (locked: {locked:?})"
        );
    }
}

#[test]
fn no_aws_lc_sys_provider_is_locked_in() {
    // ureq's rustls feature selects the ring provider; aws-lc-sys would be an
    // unbuilt, cache-unverified native build, so its absence is a contract.
    let lock = std::fs::read_to_string(workspace_root().join("Cargo.lock")).unwrap_or_default();
    assert!(
        !lock.contains("name = \"aws-lc"),
        "aws-lc-rs/aws-lc-sys must not enter the offline dependency graph"
    );
    assert!(
        lock.contains("name = \"ring\""),
        "ring must be the locked TLS provider"
    );
}
