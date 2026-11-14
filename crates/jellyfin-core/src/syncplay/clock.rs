//! Instants on the wire, and how far this device's clock is from the
//! server's.
//!
//! SyncPlay schedules every command on the server's clock: an `Unpause` says
//! "be playing at this position when the server's clock reads `When`". A
//! device whose clock is a second off would start a second early or late, so
//! the client measures the difference with `GET /GetUtcTime` and converts.
//!
//! The arithmetic is the usual four-timestamp one: the device notes when it
//! sent the request (`t0`) and when the answer arrived (`t3`), the server
//! reports when it received the request (`t1`) and when it answered (`t2`).
//!
//! ```text
//! offset     = ((t1 - t0) + (t2 - t3)) / 2     server clock minus device clock
//! round trip = (t3 - t0) - (t2 - t1)           time on the wire, both ways
//! ```
//!
//! The sample with the smallest round trip has the least room for an
//! asymmetric delay to skew its offset, so that is the one to keep
//! ([`best`]).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::JfError;
use crate::ticks::TICKS_PER_SECOND;

/// Ticks in one day.
const TICKS_PER_DAY: i64 = 86_400 * TICKS_PER_SECOND;

/// One instant, as 100 ns ticks since 1970-01-01T00:00:00Z.
///
/// The server writes instants as ISO 8601 UTC with up to seven fractional
/// digits (`2026-11-12T22:04:12.8632449Z`), which is exactly one tick of
/// resolution, so a round trip through [`UtcTime::parse`] and
/// [`UtcTime::to_wire`] loses nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTime(pub i64);

impl UtcTime {
    /// This device's clock now.
    #[must_use]
    pub fn now() -> Self {
        Self::from_system(SystemTime::now())
    }

    /// Convert a [`SystemTime`]; one before 1970 counts backwards.
    #[must_use]
    pub fn from_system(time: SystemTime) -> Self {
        match time.duration_since(UNIX_EPOCH) {
            Ok(after) => Self(duration_ticks(after)),
            Err(before) => Self(-duration_ticks(before.duration())),
        }
    }

    /// Parse the server's `YYYY-MM-DDTHH:MM:SS[.fffffff]Z`.
    ///
    /// # Errors
    ///
    /// [`JfError::ResponseShape`] for anything else, including a local
    /// offset: the server writes every SyncPlay instant in UTC.
    pub fn parse(text: &str) -> Result<Self, JfError> {
        parse_iso(text.as_bytes()).ok_or(JfError::ResponseShape)
    }

    /// The instant in the server's own format, with all seven digits.
    #[must_use]
    pub fn to_wire(self) -> String {
        let days = self.0.div_euclid(TICKS_PER_DAY);
        let in_day = self.0.rem_euclid(TICKS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        let seconds = in_day / TICKS_PER_SECOND;
        let fraction = in_day % TICKS_PER_SECOND;
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{fraction:07}Z",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    }

    /// This instant moved by `ticks` (negative moves it back).
    #[must_use]
    pub fn add_ticks(self, ticks: i64) -> Self {
        Self(self.0.saturating_add(ticks))
    }

    /// `self - earlier`, in ticks.
    #[must_use]
    pub fn ticks_since(self, earlier: Self) -> i64 {
        self.0.saturating_sub(earlier.0)
    }

    /// `self - earlier`, in seconds.
    #[must_use]
    pub fn seconds_since(self, earlier: Self) -> f64 {
        ticks_to_seconds(self.ticks_since(earlier))
    }
}

impl serde::Serialize for UtcTime {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_wire())
    }
}

impl<'de> serde::Deserialize<'de> for UtcTime {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Self::parse(&text).map_err(|_| serde::de::Error::custom("not a UTC instant"))
    }
}

/// One `GET /GetUtcTime` exchange, reduced to what it says about the clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// Server clock minus device clock, in ticks.
    pub offset_ticks: i64,
    /// Time the exchange spent between the two machines, in ticks.
    pub round_trip_ticks: i64,
}

impl ClockSample {
    /// The sample from the four timestamps of one exchange: sent (`t0`),
    /// received by the server (`t1`), answered by the server (`t2`) and
    /// received here (`t3`).
    #[must_use]
    pub fn new(
        sent: UtcTime,
        server_received: UtcTime,
        server_sent: UtcTime,
        received: UtcTime,
    ) -> Self {
        let there = server_received.ticks_since(sent);
        let back = server_sent.ticks_since(received);
        let round_trip = received
            .ticks_since(sent)
            .saturating_sub(server_sent.ticks_since(server_received));
        Self {
            offset_ticks: there.saturating_add(back) / 2,
            // A server that answers faster than its own clock reports cannot
            // make the wire take negative time.
            round_trip_ticks: round_trip.max(0),
        }
    }

    /// Server clock minus device clock, in seconds.
    #[must_use]
    pub fn offset_seconds(&self) -> f64 {
        ticks_to_seconds(self.offset_ticks)
    }

    /// The round trip, as `/SyncPlay/Ping` wants it.
    #[must_use]
    pub fn round_trip(&self) -> Duration {
        Duration::from_nanos(
            u64::try_from(self.round_trip_ticks)
                .unwrap_or(0)
                .saturating_mul(100),
        )
    }

    /// What the server's clock reads when this device's reads `local`.
    #[must_use]
    pub fn to_server(&self, local: UtcTime) -> UtcTime {
        local.add_ticks(self.offset_ticks)
    }

    /// What this device's clock reads when the server's reads `server`.
    #[must_use]
    pub fn to_local(&self, server: UtcTime) -> UtcTime {
        server.add_ticks(-self.offset_ticks)
    }
}

/// The sample with the smallest round trip, if there is one.
#[must_use]
pub fn best(samples: &[ClockSample]) -> Option<ClockSample> {
    samples
        .iter()
        .copied()
        .min_by_key(|sample| sample.round_trip_ticks)
}

/// Ticks as seconds.
#[must_use]
pub fn ticks_to_seconds(ticks: i64) -> f64 {
    // Exact for any span shorter than about 28 years.
    ticks as f64 / TICKS_PER_SECOND as f64
}

fn duration_ticks(duration: Duration) -> i64 {
    i64::try_from(duration.as_nanos() / 100).unwrap_or(i64::MAX)
}

fn parse_iso(bytes: &[u8]) -> Option<UtcTime> {
    let (date_time, fraction) = match bytes.strip_suffix(b"Z")? {
        rest if rest.len() > 19 => {
            let (whole, fraction) = rest.split_at(19);
            let fraction = fraction
                .strip_prefix(b".")
                .filter(|digits| !digits.is_empty());
            (whole, fraction?)
        }
        rest => (rest, &b""[..]),
    };
    if date_time.len() != 19
        || date_time.get(4) != Some(&b'-')
        || date_time.get(7) != Some(&b'-')
        || date_time.get(10) != Some(&b'T')
        || date_time.get(13) != Some(&b':')
        || date_time.get(16) != Some(&b':')
        || fraction.len() > 9
    {
        return None;
    }
    let field = |from: usize, to: usize| digits(date_time.get(from..to)?);
    let (year, month, day) = (field(0, 4)?, field(5, 7)?, field(8, 10)?);
    let (hour, minute, second) = (field(11, 13)?, field(14, 16)?, field(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    // Digits past the seventh are below one tick and are dropped.
    let mut fraction_ticks = 0;
    for position in 0..7 {
        let digit = match fraction.get(position) {
            Some(byte) if byte.is_ascii_digit() => i64::from(byte - b'0'),
            Some(_) => return None,
            None => 0,
        };
        fraction_ticks = fraction_ticks * 10 + digit;
    }
    if !fraction.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    Some(UtcTime(seconds * TICKS_PER_SECOND + fraction_ticks))
}

fn digits(bytes: &[u8]) -> Option<i64> {
    bytes.iter().try_fold(0_i64, |value, byte| {
        byte.is_ascii_digit()
            .then(|| value * 10 + i64::from(byte - b'0'))
    })
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_servers_instants_round_trip_to_the_tick() {
        for text in [
            "2026-11-12T22:04:12.8632449Z",
            "1970-01-01T00:00:00.0000000Z",
            "2000-02-29T23:59:59.9999999Z",
            "1969-12-31T23:59:59.5000000Z",
        ] {
            assert_eq!(
                UtcTime::parse(text).map(UtcTime::to_wire),
                Ok(text.to_owned())
            );
        }
    }

    #[test]
    fn short_and_long_fractions_are_read_as_decimals() {
        let whole = UtcTime::parse("2026-11-12T22:01:42Z").map(|time| time.0);
        let short = UtcTime::parse("2026-11-12T22:01:42.148856Z").map(|time| time.0);
        let long = UtcTime::parse("2026-11-12T22:01:42.148856123Z").map(|time| time.0);
        assert_eq!(short.map(|ticks| ticks - whole.unwrap_or(0)), Ok(1_488_560));
        assert_eq!(long, short.map(|ticks| ticks + 1));
    }

    #[test]
    fn anything_but_utc_iso_8601_is_refused() {
        for text in [
            "",
            "2026-11-12T22:01:42",
            "2026-11-12T22:01:42+01:00",
            "2026-11-12 22:01:42Z",
            "2026-13-12T22:01:42Z",
            "2026-11-12T24:01:42Z",
            "2026-11-12T22:01:42.Z",
            "2026-11-12T22:01:42.12a4Z",
            "2026-11-12T22:01:42.1234567890Z",
        ] {
            assert_eq!(UtcTime::parse(text), Err(JfError::ResponseShape), "{text}");
        }
    }

    #[test]
    fn the_unix_epoch_is_tick_zero() {
        assert_eq!(UtcTime::from_system(UNIX_EPOCH), UtcTime(0));
        assert_eq!(
            UtcTime::from_system(UNIX_EPOCH + Duration::from_millis(1500)),
            UtcTime(15_000_000)
        );
    }

    #[test]
    fn a_symmetric_exchange_measures_offset_and_round_trip() {
        // The server is 2 s ahead; 40 ms out, 5 ms inside the server, 40 ms back.
        let t0 = UtcTime(0);
        let t1 = UtcTime(20_400_000);
        let t2 = UtcTime(20_450_000);
        let t3 = UtcTime(850_000);
        let sample = ClockSample::new(t0, t1, t2, t3);
        assert_eq!(sample.offset_ticks, 20_000_000);
        assert_eq!(sample.round_trip_ticks, 800_000);
        assert_eq!(sample.round_trip(), Duration::from_millis(80));
        assert!((sample.offset_seconds() - 2.0).abs() < 1e-9);
        assert_eq!(sample.to_server(UtcTime(5)), UtcTime(20_000_005));
        assert_eq!(sample.to_local(UtcTime(20_000_005)), UtcTime(5));
    }

    #[test]
    fn a_server_behind_the_device_has_a_negative_offset() {
        let sample = ClockSample::new(UtcTime(100), UtcTime(-900), UtcTime(-900), UtcTime(300));
        assert_eq!(sample.offset_ticks, -1100);
        assert_eq!(sample.round_trip_ticks, 200);
    }

    #[test]
    fn the_round_trip_never_goes_negative() {
        let sample = ClockSample::new(UtcTime(0), UtcTime(0), UtcTime(50), UtcTime(10));
        assert_eq!(sample.round_trip_ticks, 0);
        assert_eq!(sample.round_trip(), Duration::ZERO);
    }

    #[test]
    fn the_fastest_exchange_is_the_one_kept() {
        let slow = ClockSample {
            offset_ticks: 900,
            round_trip_ticks: 5_000,
        };
        let fast = ClockSample {
            offset_ticks: 300,
            round_trip_ticks: 1_000,
        };
        assert_eq!(best(&[slow, fast, slow]), Some(fast));
        assert_eq!(best(&[]), None);
    }
}
