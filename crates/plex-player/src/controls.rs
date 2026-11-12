//! Control-input validation: the requests the UI is allowed to issue.
//!
//! Everything here is pure Rust and *validated before* it reaches a link: an
//! obviously impossible control input (NaN seek target, volume outside the range
//! this wrapper exposes) is turned into [`PlayerError::UnsupportedItem`] rather
//! than handed to libmpv and reported later as an opaque command failure. The
//! command/property names used are the ones in [`crate::commands`] and carry
//! their own provenance label.

use crate::commands::{cmds, props, PropertyValue, Request, SeekMode};
use crate::events::PlayerError;

/// Highest volume this wrapper exposes (unity gain).
///
/// mpv accepts values above 100 (amplification); the client deliberately does
/// not, because the UI slider has no amplification semantics yet
/// (`docs/CAPABILITIES.md`).
pub const MAX_VOLUME: f64 = 100.0;

/// Build the `pause` set-property request.
///
/// `pause` is an unverified (non-header) property name; see
/// [`crate::commands::props::PAUSE`] and the `--probe-json` report.
#[must_use]
pub fn pause_request(paused: bool) -> Request {
    Request::SetProperty {
        name: props::PAUSE,
        value: PropertyValue::Flag(paused),
    }
}

/// Build the `volume` set-property request.
///
/// # Errors
///
/// [`PlayerError::UnsupportedItem`] for a non-finite value or one outside
/// `0.0..=MAX_VOLUME`.
pub fn volume_request(percent: f64) -> Result<Request, PlayerError> {
    if !percent.is_finite() {
        return Err(PlayerError::UnsupportedItem {
            item: format!("volume={percent}"),
            reason: "volume must be a finite number".to_owned(),
        });
    }
    if !(0.0..=MAX_VOLUME).contains(&percent) {
        return Err(PlayerError::UnsupportedItem {
            item: format!("volume={percent}"),
            reason: format!("volume must be within 0.0-{MAX_VOLUME}"),
        });
    }
    Ok(Request::SetProperty {
        name: props::VOLUME,
        value: PropertyValue::Double(percent),
    })
}

/// Build the `seek` command request (`seek <target> <flag>`).
///
/// # Errors
///
/// [`PlayerError::UnsupportedItem`] for a non-finite target, or for a negative
/// absolute target (a negative absolute position cannot be played).
pub fn seek_request(target: f64, mode: SeekMode) -> Result<Request, PlayerError> {
    if !target.is_finite() {
        return Err(PlayerError::UnsupportedItem {
            item: format!("seek={target}"),
            reason: "seek target must be a finite number of seconds".to_owned(),
        });
    }
    if target < 0.0 && mode != SeekMode::Relative {
        return Err(PlayerError::UnsupportedItem {
            item: format!("seek={target} mode={}", mode.flag()),
            reason: "negative seek target is only meaningful for a relative seek".to_owned(),
        });
    }
    Ok(Request::Command {
        name: cmds::SEEK,
        args: vec![format!("{target}"), mode.flag().to_owned()],
    })
}

/// Build the `loadfile` command request for an already-parsed item.
///
/// The mode argument (`replace`) is the upstream-documented form and is *not* a
/// header symbol; the `--probe-json` report records whether the installed libmpv
/// accepted this exact argument list in a real session.
///
/// # Errors
///
/// [`PlayerError::UnsupportedItem`] when the raw source is not playable here.
pub fn load_request(raw_source: &str) -> Result<Request, PlayerError> {
    let item = crate::commands::ItemSource::parse(raw_source)?;
    Ok(Request::Command {
        name: cmds::LOADFILE,
        args: vec![item.load_arg().to_owned(), "replace".to_owned()],
    })
}

/// Build the `stop` command request.
#[must_use]
pub fn stop_request() -> Request {
    Request::Command {
        name: cmds::STOP,
        args: Vec::new(),
    }
}
