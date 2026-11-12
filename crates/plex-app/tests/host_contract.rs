//! Window-host contract test.
//!
//! What this checks is the window host's **public surface and its run policy**:
//! the two things a single harness run cannot pin (a run only shows one
//! configuration).
//!
//! What this does NOT do: create a window, build an event loop, or call
//! [`plex_app::shell::host::run_host`] / [`plex_app::shell::host::run_host_report`].
//! Calling them here would depend on whether the machine running `cargo test`
//! has a compositor, and a host that cannot make progress blocks until its
//! deadline - neither is deterministic. Progress
//! counter semantics (`HostProgress`) are unit-tested in
//! `crates/plex-app/src/shell/host.rs` because `HostProgress::new` is private.

use plex_app::shell::host::{
    swap_interval_label, swap_interval_policy, watchdog_ms, UserEvent, HOST_SMOKE_WATCHDOG_MS,
    HOST_WATCHDOG_MAX_MS, HOST_WINDOW_TITLE,
};

/// The window host's error and return shapes, named so the assertions below
/// stay readable (and so `clippy::type_complexity` does not object to an
/// inline `fn(...) -> Result<_, Box<dyn Error>>` annotation).
type HostError = Box<dyn std::error::Error>;
type RunHost = fn(Option<u32>) -> Result<(), HostError>;
type RunHostReport = fn(Option<u32>) -> Result<u32, HostError>;

/// The entry points `crates/plex-app/src/main.rs` calls must keep the shape its
/// `--smoke-frames N` JSON report is built from. Binding the fn items to typed
/// function pointers **is** the assertion: a signature change stops this file
/// from compiling. Neither function is called here (see the module comment).
#[test]
fn window_host_entry_points_keep_the_reported_shape() {
    let run: RunHost = plex_app::shell::host::run_host;
    let run_report: RunHostReport = plex_app::shell::host::run_host_report;
    // The smoke report states the title it used; the window must not be nameless.
    assert!(!HOST_WINDOW_TITLE.is_empty());
    assert_eq!(HOST_WINDOW_TITLE, "Chewvy");
    let _referenced_but_not_called = (run, run_report);
}

/// The two pacing cases the harness measured, in one place: a bounded smoke
/// run must not ask for vsync-blocking swaps (in the headless test environment that presented
/// one frame and then made no further progress), while an interactive run keeps
/// the upstream example's pacing. `PLEX_HOST_SWAP_INTERVAL` overrides both.
///
/// Environment variables are process-global, so this is the only test in this
/// file that touches `PLEX_HOST_SWAP_INTERVAL` and it does so in one order.
#[test]
fn bounded_runs_are_paced_so_the_deadline_can_be_answered() {
    use std::env::{remove_var, set_var};
    remove_var("PLEX_HOST_SWAP_INTERVAL");
    assert_eq!(
        swap_interval_label(swap_interval_policy(Some(8))),
        "dontwait"
    );
    assert_eq!(swap_interval_label(swap_interval_policy(None)), "wait1");
    set_var("PLEX_HOST_SWAP_INTERVAL", "1");
    assert_eq!(swap_interval_label(swap_interval_policy(Some(8))), "wait1");
    set_var("PLEX_HOST_SWAP_INTERVAL", "0");
    assert_eq!(swap_interval_label(swap_interval_policy(None)), "dontwait");
    // An unparsable value is not a silent 0: it falls back to the policy.
    set_var("PLEX_HOST_SWAP_INTERVAL", "nonsense");
    assert_eq!(
        swap_interval_label(swap_interval_policy(Some(8))),
        "dontwait"
    );
    remove_var("PLEX_HOST_SWAP_INTERVAL");
}

/// The deadline bound the harness wall clock is sized from (the harness gives
/// `timeout` watchdog + 20 s) so a stuck run can report itself before it is
/// killed. The bounds are asserted; no run is measured here.
#[test]
fn watchdog_stays_inside_the_bound_the_harness_allows() {
    use std::env::{remove_var, set_var};
    remove_var("PLEX_HOST_WATCHDOG_MS");
    assert_eq!(watchdog_ms(), HOST_SMOKE_WATCHDOG_MS);
    set_var("PLEX_HOST_WATCHDOG_MS", "1");
    assert!(
        (500..=HOST_WATCHDOG_MAX_MS).contains(&watchdog_ms()),
        "clamped below the harness floor"
    );
    set_var("PLEX_HOST_WATCHDOG_MS", "99999999");
    assert_eq!(watchdog_ms(), HOST_WATCHDOG_MAX_MS);
    set_var("PLEX_HOST_WATCHDOG_MS", "not-a-number");
    assert_eq!(watchdog_ms(), HOST_SMOKE_WATCHDOG_MS);
    remove_var("PLEX_HOST_WATCHDOG_MS");
}

/// `UserEvent` is the event loop's own vocabulary: the repaint delay egui asks
/// for, and the deadline that turns an unfinished bounded run into a reported
/// failure instead of a hang.
#[test]
fn user_events_are_the_two_things_the_loop_can_be_asked_for() {
    match UserEvent::Redraw(std::time::Duration::from_millis(16)) {
        UserEvent::Redraw(d) => assert!(d < std::time::Duration::from_secs(1)),
        UserEvent::Deadline => panic!("expected the Redraw variant"),
    }
    assert!(matches!(UserEvent::Deadline, UserEvent::Deadline));
}
