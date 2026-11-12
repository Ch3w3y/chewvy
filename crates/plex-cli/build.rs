//! Display-free native-readiness probe.
//!
//! Probes the system libraries the researched stack needs (libmpv, EGL, GL,
//! Wayland, xkbcommon) and exposes the discovered versions to the crate through
//! `cargo:rustc-env`, so `chewvy-cli status` reports what this machine can actually
//! provide. Unlike `crates/plex-player/build.rs`, this probe is diagnostic: it
//! warns instead of failing, because a readiness report must be able to report a
//! missing library instead of dying before printing it.

const PROBES: &[(&str, Option<&str>)] = &[
    ("mpv", Some("2.5.0")),
    ("egl", None),
    ("gl", None),
    ("wayland-client", Some("1.20")),
    ("xkbcommon", Some("1.0")),
];

fn probe(name: &str, min: Option<&str>) -> String {
    let mut config = pkg_config::Config::new();
    if let Some(version) = min {
        config.atleast_version(version);
    }
    match config.probe(name) {
        Ok(library) => library.version,
        Err(err) => {
            println!("cargo:warning=native probe failed for {name}: {err}");
            format!("missing:{name}")
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH_x86_64_unknown_linux_gnu");
    println!("cargo:rerun-if-env-changed=HOST_PKG_CONFIG_PATH");
    // `.copied()` so `min` is `Option<&str>`, not `&Option<&str>` (E0308).
    for (name, min) in PROBES.iter().copied() {
        let key = name.to_uppercase().replace('-', "_");
        let version = probe(name, min);
        println!("cargo:rustc-env=PLEX_NATIVE_{key}={version}");
    }
    println!("cargo:rustc-env=PLEX_NATIVE_PROBED=1");
}
