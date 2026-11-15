//! The system's own time zone and locale, read for
//! [`theme_art::hemisphere`](crate::theme_art::hemisphere), and the local
//! UTC offset the background calendar and rotation follow.

use crate::theme_art::{self, Hemisphere};

/// The UTC offset, in seconds east, the system's time zone has in force now.
///
/// On Linux and macOS: `TZ`, else `/etc/localtime`, read through jiff, which
/// follows the zone's rules past its last listed transition. On Windows: the
/// bias Windows reports with its daylight-saving state
/// ([`crate::gl::support::windows_zone`]). When neither answers, the offset is
/// 0 and the day follows UTC.
#[cfg(not(windows))]
#[must_use]
pub fn utc_offset_secs() -> i32 {
    jiff::tz::TimeZone::system()
        .to_offset(jiff::Timestamp::now())
        .seconds()
}

/// The UTC offset, in seconds east, Windows has in force now, with its
/// daylight-saving state applied ([`crate::gl::support::windows_zone`]).
#[cfg(windows)]
#[must_use]
pub fn utc_offset_secs() -> i32 {
    crate::gl::support::windows_zone().offset_secs
}

/// The hemisphere this computer's time zone, or else its locale, places the
/// user in; North when neither says (see [`theme_art::hemisphere`]).
///
/// The zone is `TZ` when set, else the `zoneinfo/` name `/etc/localtime`
/// links to (Linux and macOS) or `/etc/timezone` (Debian), and on Windows the
/// zone's registry key name (`AUS Eastern Standard Time`). The locale is the
/// first of `LC_ALL`, `LC_TIME` and `LANG` that is set.
#[must_use]
pub fn hemisphere() -> Hemisphere {
    let locale = ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
    theme_art::hemisphere(zone().as_deref(), locale.as_deref())
}

fn zone() -> Option<String> {
    if let Some(tz) = std::env::var("TZ").ok().filter(|v| !v.is_empty()) {
        return Some(zone_name(&tz).to_owned());
    }
    system_zone()
}

#[cfg(windows)]
fn system_zone() -> Option<String> {
    crate::gl::support::windows_zone().key_name
}

#[cfg(not(windows))]
fn system_zone() -> Option<String> {
    let linked = std::fs::read_link("/etc/localtime").ok();
    if let Some(name) = linked.as_ref().and_then(|p| p.to_str()).map(zone_name) {
        return Some(name.to_owned());
    }
    std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|text| text.trim().to_owned())
}

/// The zone name of a zoneinfo path (`/usr/share/zoneinfo/Australia/Sydney`,
/// `:Europe/Paris`), or the value itself when it is a bare name.
fn zone_name(value: &str) -> &str {
    let value = value.trim_start_matches(':');
    value
        .rsplit_once("zoneinfo/")
        .map_or(value, |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::zone_name;

    #[test]
    fn a_zoneinfo_path_names_its_zone() {
        assert_eq!(
            zone_name("/usr/share/zoneinfo/Australia/Sydney"),
            "Australia/Sydney"
        );
        assert_eq!(
            zone_name("/var/db/timezone/zoneinfo/Pacific/Auckland"),
            "Pacific/Auckland"
        );
        assert_eq!(zone_name(":Europe/Paris"), "Europe/Paris");
        assert_eq!(zone_name("UTC"), "UTC");
    }
}
