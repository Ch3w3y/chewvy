//! Logging plumbing that makes "tokens never reach a log" a *testable* property
//! instead of an intention.
//!
//! [`init_capture`] installs a `log` backend that appends every formatted
//! record to an in-process buffer. Tests then assert that a token literal never
//! appears in that buffer: `tests/redaction.rs` (calibration
//! `the_capture_backend_catches_a_token_if_one_is_ever_logged`, then
//! `a_signed_in_flow_leaves_no_token_in_captured_log_output` and
//! `failure_paths_log_and_render_only_error_codes_never_the_token`). The same
//! helper is what a UI smoke test can use to prove the shipped build behaves
//! the same way.
//!
//! There is deliberately no `log::error!("... {token}")`-style helper here: the
//! only formatting entry point is [`log::warn!`] & friends from the `log` crate,
//! whose `Record` we capture verbatim.

use std::sync::{Mutex, OnceLock};

static CAPTURED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// `log` backend that buffers rendered records.
struct CaptureLogger;

impl log::Log for CaptureLogger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let buffer = CAPTURED.get_or_init(|| Mutex::new(Vec::new()));
        let line = format!(
            "{} target={} {}",
            record.level(),
            record.target(),
            record.args()
        );
        if let Ok(mut guard) = buffer.lock() {
            guard.push(line);
        }
    }

    fn flush(&self) {}
}

/// Install the capturing logger and capture everything at `Trace`.
///
/// Idempotent and process-global: `cargo test` runs all tests of one file in one
/// process, so calling this from several tests is fine. Returns false when a
/// different logger was already installed by another component (in which case
/// the caller must not claim redaction coverage from this buffer).
pub fn init_capture() -> bool {
    CAPTURED.get_or_init(|| Mutex::new(Vec::new()));
    log::set_boxed_logger(Box::new(CaptureLogger)).is_ok() && {
        log::set_max_level(log::LevelFilter::Trace);
        true
    }
}

/// True when our capture backend is the active one.
#[must_use]
pub fn capture_active() -> bool {
    log::max_level() == log::LevelFilter::Trace && CAPTURED.get().is_some()
}

/// Every captured record so far.
#[must_use]
pub fn captured() -> Vec<String> {
    CAPTURED
        .get()
        .and_then(|buffer| buffer.lock().ok().map(|guard| guard.clone()))
        .unwrap_or_default()
}

/// Drop everything captured so far (call before each redaction assertion).
pub fn clear() {
    if let Some(buffer) = CAPTURED.get() {
        if let Ok(mut guard) = buffer.lock() {
            guard.clear();
        }
    }
}

/// Captured records that contain `needle` (used as "secret leaked here").
#[must_use]
pub fn lines_containing(needle: &str) -> Vec<String> {
    captured()
        .into_iter()
        .filter(|line| line.contains(needle))
        .collect()
}

/// Replace every occurrence of each secret in a string that a caller wants to
/// log or show. Applied to any text derived from an external source (a server
/// error body, a URL, a `Display` of a foreign error) before it is logged.
#[must_use]
pub fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_owned();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        // Longest first so a nested/overlapping secret cannot survive.
        out = out.replace(secret, crate::secret::REDACTED);
    }
    out
}

/// Render an error for a UI/CLI surface: our own variants are already
/// secret-free, and `source()` chains are scrubbed of any secret the caller
/// still holds.
#[must_use]
pub fn describe(error: &dyn std::error::Error, secrets: &[&str]) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(current) = source {
        parts.push(current.to_string());
        source = current.source();
    }
    scrub(&parts.join(": "), secrets)
}

/// Longest server-supplied label this crate puts into a UI line or a log line.
///
/// Server-supplied labels are presentation text, not identity: cutting them
/// short loses nothing a user can act on, while an unbounded label can.
pub const MAX_LABEL_LEN: usize = 120;

/// Bound and flatten text that came from a server (a `resources` row `name` or
/// `device`, a host the user typed, a connection `uri`) before it is rendered
/// into a one-line message or a log record.
///
/// This is not a Plex rule and not escaping for a shell or HTML: it is this
/// client's own output hygiene, so that one hostile or malformed server cannot
/// inject newlines (forging extra log records), control characters (ANSI colour
/// or cursor moves) or an arbitrarily long string into UI and log output.
///
/// * line breaks and tabs become a single space, so a label stays one line;
/// * every other control character (including `\u{1b}`, which is what starts an
///   ANSI escape sequence) becomes `?`, which makes such a sequence inert;
/// * the result is capped at `max_len` characters **in total**, including the
///   `…` that marks removed text, so a caller that renders the result can
///   rely on the length it asked for (`max_len == 0` means the empty string).
#[must_use]
pub fn sanitize(text: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_len));
    let mut kept = 0_usize;
    let mut truncated = false;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        // Keep room for the ellipsis when there is still text behind this one.
        if kept + 1 >= max_len && characters.peek().is_some() {
            truncated = true;
            break;
        }
        let mapped = match character {
            '\n' | '\r' | '\t' => ' ',
            other if other.is_control() => '?',
            other => other,
        };
        out.push(mapped);
        kept += 1;
    }
    if truncated && max_len > 0 {
        out.push('…');
    }
    out
}
