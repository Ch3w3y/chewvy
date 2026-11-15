//! The calendar side of themes: the user's local day, and which seasonal or
//! festive theme it falls in.

use super::theme::{Festival, Season, ThemeId};

const SECS_PER_DAY: i64 = 86_400;

/// A moment on the user's wall clock: Unix seconds shifted by the UTC offset
/// that was in force. Day boundaries, festive windows and rotation all follow
/// this, so a pick changes at local midnight rather than UTC midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocalTime {
    local_secs: i64,
}

impl LocalTime {
    /// `unix_secs` since the epoch, seen from a zone `utc_offset_secs` east of
    /// UTC (negative to the west).
    pub fn from_unix(unix_secs: i64, utc_offset_secs: i32) -> Self {
        Self {
            local_secs: unix_secs.saturating_add(i64::from(utc_offset_secs)),
        }
    }

    /// Whole local days since 1970-01-01.
    pub fn day_number(self) -> i64 {
        self.local_secs.div_euclid(SECS_PER_DAY)
    }

    /// Minutes since local midnight, `0..1440`.
    pub fn minute_of_day(self) -> u32 {
        let secs = self.local_secs.rem_euclid(SECS_PER_DAY);
        u32::try_from(secs / 60).unwrap_or(0)
    }

    /// The local calendar date.
    pub fn date(self) -> CivilDate {
        CivilDate::from_day_number(self.day_number())
    }
}

/// A proleptic Gregorian calendar date. Only valid dates can be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDate {
    year: i32,
    month: u8,
    day: u8,
}

impl CivilDate {
    /// The date, or `None` when the month or day does not exist.
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        let valid = (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month);
        valid.then_some(Self { year, month, day })
    }

    /// The year.
    pub fn year(self) -> i32 {
        self.year
    }

    /// The month, `1..=12`.
    pub fn month(self) -> u8 {
        self.month
    }

    /// The day of the month, `1..=31`.
    pub fn day(self) -> u8 {
        self.day
    }

    /// Howard Hinnant's `civil_from_days`, which is exact for every `i64` day
    /// whose year fits an `i32`.
    fn from_day_number(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year: i32::try_from(year).unwrap_or(i32::MAX),
            month: u8::try_from(month).unwrap_or(1),
            day: u8::try_from(day).unwrap_or(1),
        }
    }

    fn month_day(self) -> MonthDay {
        MonthDay(self.month, self.day)
    }
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Which half of the globe the user lives in. Seasons flip across the equator;
/// festive windows do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hemisphere {
    /// North of the equator.
    North,
    /// South of the equator.
    South,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MonthDay(u8, u8);

/// An inclusive window of the year; `from > to` wraps over New Year.
struct Window {
    from: MonthDay,
    to: MonthDay,
}

impl Window {
    fn contains(&self, d: MonthDay) -> bool {
        if self.from <= self.to {
            self.from <= d && d <= self.to
        } else {
            d >= self.from || d <= self.to
        }
    }
}

const FESTIVE: [(Festival, Window); 2] = [
    (
        Festival::Halloween,
        Window {
            from: MonthDay(10, 15),
            to: MonthDay(11, 1),
        },
    ),
    (
        Festival::Christmas,
        Window {
            from: MonthDay(12, 1),
            to: MonthDay(1, 6),
        },
    ),
];

/// The theme the calendar suggests for `date`: a festive window if one is
/// open, otherwise the meteorological season for the hemisphere.
pub fn seasonal(date: CivilDate, hemisphere: Hemisphere) -> ThemeId {
    let md = date.month_day();
    if let Some((festival, _)) = FESTIVE.iter().find(|(_, w)| w.contains(md)) {
        return ThemeId::Festival(*festival);
    }
    let north = match date.month {
        12 | 1 | 2 => Season::Winter,
        3..=5 => Season::Spring,
        6..=8 => Season::Summer,
        _ => Season::Autumn,
    };
    ThemeId::Season(match hemisphere {
        Hemisphere::North => north,
        Hemisphere::South => opposite(north),
    })
}

fn opposite(season: Season) -> Season {
    match season {
        Season::Winter => Season::Summer,
        Season::Spring => Season::Autumn,
        Season::Summer => Season::Winter,
        Season::Autumn => Season::Spring,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn on(year: i32, month: u8, day: u8, h: Hemisphere) -> ThemeId {
        seasonal(CivilDate::new(year, month, day).unwrap(), h)
    }

    const HALLOWEEN: ThemeId = ThemeId::Festival(Festival::Halloween);
    const CHRISTMAS: ThemeId = ThemeId::Festival(Festival::Christmas);

    #[test]
    fn halloween_runs_from_mid_october_to_all_saints() {
        use Hemisphere::North;
        assert_eq!(on(2026, 10, 14, North), ThemeId::Season(Season::Autumn));
        assert_eq!(on(2026, 10, 15, North), HALLOWEEN);
        assert_eq!(on(2026, 10, 31, North), HALLOWEEN);
        assert_eq!(on(2026, 11, 1, North), HALLOWEEN);
        assert_eq!(on(2026, 11, 2, North), ThemeId::Season(Season::Autumn));
    }

    #[test]
    fn christmas_runs_over_new_year_to_epiphany() {
        use Hemisphere::North;
        assert_eq!(on(2026, 11, 30, North), ThemeId::Season(Season::Autumn));
        assert_eq!(on(2026, 12, 1, North), CHRISTMAS);
        assert_eq!(on(2026, 12, 31, North), CHRISTMAS);
        assert_eq!(on(2027, 1, 1, North), CHRISTMAS);
        assert_eq!(on(2027, 1, 6, North), CHRISTMAS);
        assert_eq!(on(2027, 1, 7, North), ThemeId::Season(Season::Winter));
    }

    #[test]
    fn festive_windows_win_in_both_hemispheres() {
        assert_eq!(on(2026, 12, 25, Hemisphere::South), CHRISTMAS);
        assert_eq!(on(2026, 10, 31, Hemisphere::South), HALLOWEEN);
    }

    #[test]
    fn seasons_flip_across_the_equator() {
        let cases = [
            ((2026, 2, 10), Season::Winter, Season::Summer),
            ((2026, 4, 20), Season::Spring, Season::Autumn),
            ((2026, 7, 4), Season::Summer, Season::Winter),
            ((2026, 9, 30), Season::Autumn, Season::Spring),
        ];
        for ((y, m, d), north, south) in cases {
            assert_eq!(on(y, m, d, Hemisphere::North), ThemeId::Season(north));
            assert_eq!(on(y, m, d, Hemisphere::South), ThemeId::Season(south));
        }
    }

    #[test]
    fn impossible_dates_cannot_be_built() {
        assert!(CivilDate::new(2026, 2, 29).is_none());
        assert!(CivilDate::new(2028, 2, 29).is_some());
        assert!(CivilDate::new(1900, 2, 29).is_none());
        assert!(CivilDate::new(2000, 2, 29).is_some());
        assert!(CivilDate::new(2026, 13, 1).is_none());
        assert!(CivilDate::new(2026, 4, 31).is_none());
        assert!(CivilDate::new(2026, 1, 0).is_none());
    }

    #[test]
    fn local_time_follows_the_utc_offset_across_midnight() {
        // 2026-11-14T23:30:00Z.
        let unix = 1_794_699_000;
        let utc = LocalTime::from_unix(unix, 0);
        assert_eq!(utc.date(), CivilDate::new(2026, 11, 14).unwrap());
        assert_eq!(utc.minute_of_day(), 23 * 60 + 30);

        let paris = LocalTime::from_unix(unix, 3600);
        assert_eq!(paris.date(), CivilDate::new(2026, 11, 15).unwrap());
        assert_eq!(paris.minute_of_day(), 30);
        assert_eq!(paris.day_number(), utc.day_number() + 1);

        let before_epoch = LocalTime::from_unix(-1, 0);
        assert_eq!(before_epoch.date(), CivilDate::new(1969, 12, 31).unwrap());
        assert_eq!(before_epoch.minute_of_day(), 23 * 60 + 59);
    }
}
