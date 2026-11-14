//! This device's estimate of the group's clock.
//!
//! A group schedules its commands on the server's clock, so each member keeps
//! the offset between that clock and its own. The samples come from a time
//! request to the server; this only weighs them.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Seconds on the group's clock (the server's), as this device estimates it.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct GroupTime(pub f64);

/// Samples weighed together: the burst taken on joining, then one a minute, so
/// a sample ages out after a few minutes and the estimate follows a clock
/// that drifts.
const KEPT: usize = 8;

/// One measurement of the group's clock against this device's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Group time minus local time, in seconds.
    pub offset: f64,
    /// Time the request spent on the network, in seconds.
    pub round_trip: f64,
}

/// The offset between this device's monotonic clock and the group's.
///
/// "Local" time is seconds on whatever monotonic clock the samples' send and
/// receive times were read from; [`GroupClock::group_time`] takes the same.
#[derive(Clone, Debug, Default)]
pub struct GroupClock {
    samples: VecDeque<Sample>,
}

impl GroupClock {
    /// Weigh in a new sample, dropping the oldest beyond the last few.
    pub fn add(&mut self, sample: Sample) {
        if !sample.offset.is_finite() || !sample.round_trip.is_finite() {
            return;
        }
        // Millisecond timestamps can make a near-instant exchange come out
        // a hair below zero; it is still the best sample there is.
        let round_trip = sample.round_trip.max(0.0);
        if self.samples.len() == KEPT {
            self.samples.pop_front();
        }
        self.samples.push_back(Sample {
            offset: sample.offset,
            round_trip,
        });
    }

    /// The sample with the shortest round trip: the less time a request spent
    /// in flight, the less room there was for an uneven path to skew it.
    fn best(&self) -> Option<&Sample> {
        self.samples
            .iter()
            .min_by(|a, b| a.round_trip.total_cmp(&b.round_trip))
    }

    /// Group time minus local time, once there is a sample.
    #[must_use]
    pub fn offset(&self) -> Option<f64> {
        self.best().map(|s| s.offset)
    }

    /// The round trip of the sample the offset comes from.
    #[must_use]
    pub fn round_trip(&self) -> Option<f64> {
        self.best().map(|s| s.round_trip)
    }

    /// The group's time at local time `local`, once there is a sample.
    #[must_use]
    pub fn group_time(&self, local: f64) -> Option<GroupTime> {
        self.offset().map(|offset| GroupTime(local + offset))
    }
}

/// Seconds on this device's clock: the system time when first asked, moved on
/// by a monotonic clock, so a clock change during a session cannot jump the
/// group's time. Every sample and every reading uses this one clock.
#[must_use]
pub fn local_now() -> f64 {
    static ANCHOR: OnceLock<(Instant, f64)> = OnceLock::new();
    let (instant, system) = ANCHOR.get_or_init(|| {
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64());
        (Instant::now(), since)
    });
    system + instant.elapsed().as_secs_f64()
}
