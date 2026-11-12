//! Jellyfin media positions are 100 ns ticks (`.NET` `TimeSpan` units): ten
//! million ticks per second. Conversion is lossy below 100 ns by construction;
//! these helpers make that loss explicit instead of silent.

/// Ticks per second in Jellyfin's `TimeSpan` wire format.
pub const TICKS_PER_SECOND: i64 = 10_000_000;

/// Convert a position in ticks to a [`Duration`].
///
/// Negative ticks have no duration (a corrupt or absent position is sent as
/// `-1` in some Jellyfin payloads), so they yield `None`.
#[must_use]
pub fn ticks_to_duration(ticks: i64) -> Option<std::time::Duration> {
    u64::try_from(ticks)
        .ok()
        .map(|ticks| std::time::Duration::from_nanos(ticks.saturating_mul(100)))
}

/// Convert a [`Duration`] to ticks, saturating at `i64::MAX` (the position of
/// a media file will not get near it). Sub-tick fractions are truncated:
/// durations are always rounded down to the last whole 100 ns.
#[must_use]
pub fn duration_to_ticks(duration: std::time::Duration) -> i64 {
    let micros = duration.as_micros();
    let ticks = micros.saturating_mul(10);
    i64::try_from(ticks).unwrap_or(i64::MAX)
}
