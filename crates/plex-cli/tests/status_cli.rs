//! Real behavioural tests of the CLI: the report is produced by the built
//! binary, with no display server reachable in the test environment, and the
//! native-readiness gate behaves as documented.

// Product code keeps unwrap_used/expect_used/indexing_slicing on; the tests index
// the JSON report on purpose so a missing key fails loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::process::Command;

fn run(args: &[&str]) -> (Option<i32>, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_chewvy-cli"));
    cmd.args(args);
    // Prove the report path needs no display server or Wayland/X11 session.
    cmd.env_remove("WAYLAND_DISPLAY");
    cmd.env_remove("DISPLAY");
    cmd.env_remove("XDG_SESSION_TYPE");
    cmd.env_remove("MIR_SOCKET");
    cmd.env_remove("QT_QPA_PLATFORM");
    let out = cmd.output().expect("chewvy-cli binary should be runnable");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    )
}

#[test]
fn status_prints_a_machine_readable_skeleton_report_without_a_display() {
    let (code, stdout, stderr) = run(&["status"]);
    assert_eq!(code, Some(0), "status failed, stderr: {stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("status must print JSON on stdout, got {stdout:?}: {e}"));
    assert_eq!(report["tool"], "chewvy-cli");
    assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["skeleton"], true);
    assert_eq!(report["product_logic"], false);
    assert_eq!(report["requires_display"], false);
    let layers = report["links_layers"]
        .as_array()
        .expect("links_layers array");
    assert!(layers.contains(&serde_json::json!("plex-core")));
    assert!(layers.contains(&serde_json::json!("plex-player")));
    // No plex-app dependency: the CLI must stay display-free even later.
    assert!(!layers.contains(&serde_json::json!("plex-app")));
    assert!(
        stderr.is_empty(),
        "status must not warn on stderr: {stderr}"
    );
}

#[test]
fn status_reports_probed_native_versions_or_explicit_missing_entries() {
    let (_, stdout, _) = run(&["status"]);
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("status prints JSON");
    let native = report["native"].as_object().expect("native object");
    for lib in ["mpv", "egl", "gl", "wayland_client", "xkbcommon"] {
        let value = native[lib].as_str().expect("native entry is a string");
        assert!(
            !value.is_empty(),
            "{lib} must be reported as a version or missing:<lib>"
        );
        if !value.starts_with("missing:") {
            let mut parts = value.split('.');
            let major: u64 = parts
                .next()
                .expect("major version")
                .parse()
                .expect("numeric major");
            assert!(major >= 1, "{lib} reported implausible version {value}");
        }
    }
    let missing = report["native_missing"]
        .as_array()
        .expect("native_missing array");
    assert_eq!(
        missing.len(),
        native
            .values()
            .filter(|v| v.as_str().unwrap_or_default().starts_with("missing:"))
            .count(),
        "native_missing must agree with the native entries"
    );
}

#[test]
fn require_native_exits_3_exactly_when_a_probe_is_missing() {
    let (_, stdout, _) = run(&["status"]);
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("status prints JSON");
    let missing = report["native_missing"]
        .as_array()
        .expect("native_missing array")
        .len();
    let (code, _, _) = run(&["status", "--require-native"]);
    assert_eq!(code, Some(if missing == 0 { 0 } else { 3 }));
}

#[test]
fn version_and_argument_behaviour_is_machine_readable() {
    let (code, stdout, _) = run(&["--version"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "unexpected --version output: {stdout}"
    );
    // No subcommand is an honest usage error (clap exit code 2), not a silent zero.
    let (code, _, stderr) = run(&[]);
    assert_eq!(code, Some(2), "expected a usage error, stderr: {stderr}");
    assert!(
        stderr.contains("status"),
        "usage error should name the subcommand"
    );
}
