//! Executable contracts for the desktop shell layer.
//!
//! Guards the shell decision:
//! egui + egui-winit + egui_glow + glow + glutin + glutin-winit + winit, and no
//! eframe/wgpu path, which is also what keeps the offline crate cache complete.

#![allow(clippy::unwrap_used, clippy::expect_used)] // product code keeps these lints on

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Crates that must stay out of the native, shipped dependency graph on top of
/// the shell list: the aws-lc crypto backend (rustls is pinned to the `ring`
/// provider on purpose, see Cargo.toml) .
const NATIVE_GRAPH_EXTRA_FORBIDDEN: &[&str] = &["aws-lc-rs", "aws-lc-sys"];

const FORBIDDEN_SHELL_CRATES: &[&str] = &[
    "eframe",
    "egui-wgpu",
    "wgpu",
    "pollster",
    "wasm-bindgen",
    "js-sys",
    "web-sys",
    "wasm-bindgen-futures",
    "enumn",
    "ash",
];

const REQUIRED_SHELL_CRATES: &[&str] = &[
    "egui",
    "epaint",
    "egui-winit",
    "egui_glow",
    "glow",
    "glutin",
    "glutin-winit",
    "winit",
];

fn dependency_keys(manifest: &PathBuf) -> Vec<String> {
    let text = std::fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
    let mut keys = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            let section = line.trim_matches(|c| c == '[' || c == ']');
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
        }
    }
    keys
}

fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..6 {
        if dir.join("Cargo.lock").exists() {
            return dir;
        }
        dir = dir
            .parent()
            .expect("workspace root above CARGO_MANIFEST_DIR")
            .to_path_buf();
    }
    panic!("workspace root not found");
}

#[test]
fn shell_uses_the_researched_egui_glow_stack_only() {
    let keys = dependency_keys(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    for expected in REQUIRED_SHELL_CRATES {
        assert!(
            keys.contains(&(*expected).to_owned()),
            "plex-app must wire the researched shell crate {expected}"
        );
    }
    for key in &keys {
        assert!(
            !FORBIDDEN_SHELL_CRATES.contains(&key.as_str()),
            "eframe/wgpu/pollster/enumn paths are out of scope for this client ({key})"
        );
    }
}

#[test]
fn lockfile_excludes_the_wgpu_and_eframe_paths() {
    let lock = std::fs::read_to_string(workspace_root().join("Cargo.lock"))
        .expect("Cargo.lock must be committed");
    for forbidden in FORBIDDEN_SHELL_CRATES.iter().copied() {
        // wasm-bindgen/js-sys/web-sys/wasm-bindgen-futures ARE in Cargo.lock as
        // cross-compile entries of winit 0.30.13, glow 0.17.0, uuid and web-time (via
        // egui/epaint). They are in no native manifest here; this lock-text check is
        // only a cheap guard for the non-wasm names. The native graph itself is
        // enforced in-repo by native_dependency_tree_resolves_no_forbidden_crates below.
        if matches!(
            forbidden,
            "wasm-bindgen" | "js-sys" | "web-sys" | "wasm-bindgen-futures"
        ) {
            continue;
        }
        assert!(
            !lock.contains(&format!("name = \"{forbidden}\"")),
            "{forbidden} must not enter the locked offline graph"
        );
    }
}

/// Executable native-graph gate: runs the dependency resolution
/// (`cargo tree --offline --locked -e normal --target x86_64-unknown-linux-gnu`)
/// so the "no wasm/eframe/wgpu/aws-lc edge" claim is enforced by the test suite
/// instead of being deferred to a command nobody runs.
#[test]
fn native_dependency_tree_resolves_no_forbidden_crates() {
    let cargo = std::env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"));
    let root = workspace_root();
    let output = std::process::Command::new(&cargo)
        .current_dir(&root)
        .arg("tree")
        .args([
            "--offline",
            "--locked",
            "-e",
            "normal",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--prefix",
            "none",
        ])
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "`{} tree` could not be run from {}: {e}",
                cargo.display(),
                root.display()
            )
        });
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "`cargo tree --offline --locked -e normal --target x86_64-unknown-linux-gnu` failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let names: BTreeSet<String> = stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect();
    assert!(
        names.len() >= 50,
        "the native tree resolved only {} packages; this gate is vacuous (stdout {name:?})",
        names.len(),
        name = stdout
    );
    for forbidden in FORBIDDEN_SHELL_CRATES
        .iter()
        .chain(NATIVE_GRAPH_EXTRA_FORBIDDEN.iter())
    {
        assert!(
            !names.contains(*forbidden),
            "{forbidden} is reachable in the native (normal-edge, x86_64-unknown-linux-gnu) dependency graph"
        );
    }
}

#[test]
fn version_and_license_stay_pinned() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.2");
    assert_eq!(env!("CARGO_PKG_LICENSE"), "GPL-3.0-or-later");
}
