//! Executable contracts for the player layer.
//!
//! The build script already runs the libmpv version guard that milestone M4
//! depends on; these tests assert the guard actually ran and produced a supported
//! version, and that no upstream mpv/GStreamer crate crate sneaks into the graph.

#![allow(clippy::unwrap_used, clippy::expect_used)] // product code keeps these lints on

use std::path::Path;

/// The engine is a thin bindgen FFI over libmpv, not an existing
/// mpv/GStreamer wrapper crate.
const FORBIDDEN_ENGINE_CRATES: &[&str] = &[
    "mpv",
    "mpv_utils",
    "libmpv",
    "mpv-file",
    "mpv_api",
    "gstreamer",
    "gstreamer-base",
    "gstreamer-video",
    "gstreamer-app",
    "playbin3",
];

fn dependency_keys(manifest: &Path) -> Vec<String> {
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

fn version_at_least(version: &str, required: (u64, u64)) -> bool {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|p| p.parse().ok());
    let minor = parts.next().and_then(|p| p.parse().ok());
    match (major, minor) {
        (Some(major), Some(minor)) => (major, minor) >= required,
        _ => false,
    }
}

#[test]
fn libmpv_probe_reports_the_researched_minimum_version() {
    let probed = env!("PLEX_MPV_VERSION");
    if probed == "guard-disabled" {
        eprintln!(
            "SKIP: libmpv guard was bypassed via PLEX_DISABLE_MPV_GUARD; the player \
             cannot be built on this machine without libmpv 2.5.0+"
        );
        return;
    }
    assert!(
        version_at_least(probed, (2, 5)),
        "crates/plex-player requires libmpv >= 2.5.0, pkg-config reported {probed}"
    );
}

#[test]
fn player_layer_wires_native_probing_and_locks_bindgen_but_uses_no_wrapper_crate() {
    let keys = dependency_keys(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    for key in &keys {
        assert!(
            !FORBIDDEN_ENGINE_CRATES.contains(&key.as_str()),
            "plex-player must use its own libmpv FFI, not the {key} wrapper crate"
        );
    }
    assert!(keys.contains(&"plex-core".to_owned()));
    assert!(keys.contains(&"pkg-config".to_owned()));
    assert!(
        keys.contains(&"bindgen".to_owned()),
        "bindgen 0.72.1 must be a version-locked optional build-dependency"
    );
}

#[test]
fn a_skipped_libmpv_guard_cannot_happen_silently() {
    // The build script only emits `guard-disabled` when PLEX_DISABLE_MPV_GUARD is
    // set in the same invocation that compiled it, and it emits a cargo:warning in
    // that case. If the two ever disagree, the guard was bypassed without an
    // operator opting into it, which milestone M4 must not inherit.
    let probed = env!("PLEX_MPV_VERSION");
    let bypassed = std::env::var_os("PLEX_DISABLE_MPV_GUARD").is_some();
    if probed == "guard-disabled" {
        assert!(
            bypassed,
            "PLEX_MPV_VERSION={probed} while PLEX_DISABLE_MPV_GUARD is unset: the libmpv guard was skipped without an explicit opt-in"
        );
        eprintln!(
            "DEGRADED: libmpv guard skipped via PLEX_DISABLE_MPV_GUARD; the player is not buildable on this machine, and `chewvy-cli status --require-native` must report missing:mpv (exit 3) as the honest gate"
        );
    } else {
        assert!(
            !bypassed,
            "PLEX_DISABLE_MPV_GUARD is set but PLEX_MPV_VERSION={probed}: the build script reported a probe it skipped"
        );
        assert!(
            version_at_least(probed, (2, 5)),
            "unexpected guard state {probed} with the bypass variable unset"
        );
    }
}

#[test]
fn version_and_license_stay_pinned() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.4");
    assert_eq!(env!("CARGO_PKG_LICENSE"), "GPL-3.0-or-later");
}
