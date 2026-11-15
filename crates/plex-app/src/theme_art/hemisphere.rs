//! Which half of the globe a time zone or a locale says the user lives in, so
//! the calendar can flip the seasons. Pure: the caller reads the zone name and
//! the locale from the system.
//!
//! The zone is asked first because it says where the computer is; a locale only
//! says how the user writes (an `en_AU` user in London lives in the north). A
//! zone that names no place (`UTC`, `Etc/GMT+3`) says nothing, and then the
//! locale's region decides. With neither, the answer is North, where most
//! users live.

use super::calendar::Hemisphere;

/// IANA zones south of the equator, as whole names or as `Area/` prefixes,
/// plus the Windows zone names of the same places. Zones within a few degrees
/// of the equator, where seasons are wet and dry rather than warm and cold,
/// are left out and so read as North.
const SOUTHERN_ZONES: &[&str] = &[
    "Australia/",
    "Antarctica/",
    "NZ",
    "NZ-CHAT",
    "Pacific/Auckland",
    "Pacific/Chatham",
    "Pacific/Fiji",
    "Pacific/Tongatapu",
    "Pacific/Apia",
    "Pacific/Noumea",
    "Pacific/Efate",
    "Pacific/Port_Moresby",
    "Pacific/Tahiti",
    "Pacific/Rarotonga",
    "Pacific/Easter",
    "Pacific/Norfolk",
    "America/Argentina/",
    "America/Buenos_Aires",
    "America/Sao_Paulo",
    "America/Santiago",
    "America/Punta_Arenas",
    "America/Montevideo",
    "America/Asuncion",
    "America/La_Paz",
    "America/Lima",
    "America/Bahia",
    "America/Recife",
    "America/Fortaleza",
    "America/Maceio",
    "America/Cuiaba",
    "America/Campo_Grande",
    "America/Porto_Velho",
    "America/Rio_Branco",
    "America/Araguaina",
    "America/Noronha",
    "Brazil/",
    "Chile/",
    "Africa/Johannesburg",
    "Africa/Maputo",
    "Africa/Harare",
    "Africa/Lusaka",
    "Africa/Windhoek",
    "Africa/Gaborone",
    "Africa/Maseru",
    "Africa/Mbabane",
    "Africa/Lubumbashi",
    "Africa/Luanda",
    "Africa/Blantyre",
    "Africa/Dar_es_Salaam",
    "Indian/Mauritius",
    "Indian/Reunion",
    "Indian/Antananarivo",
    "Indian/Mayotte",
    "Indian/Comoro",
    "Atlantic/St_Helena",
    "Atlantic/Stanley",
    "Atlantic/South_Georgia",
    "AUS Eastern Standard Time",
    "AUS Central Standard Time",
    "Cen. Australia Standard Time",
    "E. Australia Standard Time",
    "W. Australia Standard Time",
    "Tasmania Standard Time",
    "New Zealand Standard Time",
    "Chatham Islands Standard Time",
    "South Africa Standard Time",
    "Namibia Standard Time",
    "Mauritius Standard Time",
    "E. South America Standard Time",
    "Argentina Standard Time",
    "Pacific SA Standard Time",
    "Magallanes Standard Time",
    "Paraguay Standard Time",
    "Montevideo Standard Time",
    "Bahia Standard Time",
    "Tocantins Standard Time",
    "Central Brazilian Standard Time",
    "Fiji Standard Time",
    "Tonga Standard Time",
    "Easter Island Standard Time",
];

/// Zone names that name no place.
const PLACELESS_ZONES: &[&str] = &["UTC", "UCT", "GMT", "Zulu", "Universal", "Etc/"];

/// ISO 3166 regions south of the equator, as a locale names them (`en_AU`).
const SOUTHERN_REGIONS: &[&str] = &[
    "AU", "NZ", "ZA", "AR", "BR", "CL", "UY", "PY", "BO", "PE", "NA", "BW", "LS", "SZ", "MZ", "ZW",
    "ZM", "MW", "MG", "MU", "RE", "FJ", "TO", "PG",
];

/// The hemisphere `zone` (an IANA name such as `Australia/Sydney`, or a
/// Windows zone name) or else `locale` (`en_NZ.UTF-8`, `pt-BR`) places the
/// user in; North when neither names a place.
pub fn hemisphere(zone: Option<&str>, locale: Option<&str>) -> Hemisphere {
    zone.and_then(zone_hemisphere)
        .or_else(|| locale.and_then(locale_hemisphere))
        .unwrap_or(Hemisphere::North)
}

fn zone_hemisphere(zone: &str) -> Option<Hemisphere> {
    let zone = zone.trim().trim_start_matches(':');
    let placeless = zone.is_empty() || PLACELESS_ZONES.iter().any(|p| zone.starts_with(p));
    if placeless {
        return None;
    }
    let south = SOUTHERN_ZONES.iter().any(|z| match z.strip_suffix('/') {
        Some(_) => zone.starts_with(z),
        None => zone == *z,
    });
    Some(if south {
        Hemisphere::South
    } else {
        Hemisphere::North
    })
}

/// The region of `ll_RR`, `ll-RR` or `ll_RR.charset@modifier`; `None` for a
/// locale that names none (`C`, `POSIX`, `en`).
fn locale_hemisphere(locale: &str) -> Option<Hemisphere> {
    let tag = locale.split(['.', '@']).next()?;
    let region = tag.split(['_', '-']).nth(1)?;
    let region = region.to_ascii_uppercase();
    (region.len() == 2).then(|| {
        if SOUTHERN_REGIONS.contains(&region.as_str()) {
            Hemisphere::South
        } else {
            Hemisphere::North
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Hemisphere::{North, South};

    #[test]
    fn a_southern_zone_flips_the_seasons_and_a_northern_one_does_not() {
        assert_eq!(hemisphere(Some("Australia/Sydney"), None), South);
        assert_eq!(hemisphere(Some("Pacific/Auckland"), None), South);
        assert_eq!(
            hemisphere(Some("America/Argentina/Buenos_Aires"), None),
            South
        );
        assert_eq!(hemisphere(Some("AUS Eastern Standard Time"), None), South);
        assert_eq!(hemisphere(Some(":Africa/Johannesburg"), None), South);
        assert_eq!(hemisphere(Some("Europe/London"), None), North);
        assert_eq!(hemisphere(Some("America/New_York"), None), North);
        assert_eq!(
            hemisphere(Some("Pacific/Honolulu"), None),
            North,
            "only whole names match, not a Pacific/ prefix"
        );
    }

    #[test]
    fn the_zone_wins_over_the_locale_unless_it_names_no_place() {
        assert_eq!(
            hemisphere(Some("Europe/London"), Some("en_AU.UTF-8")),
            North
        );
        assert_eq!(hemisphere(Some("UTC"), Some("en_AU.UTF-8")), South);
        assert_eq!(hemisphere(Some("Etc/GMT-10"), Some("pt-BR")), South);
        assert_eq!(hemisphere(None, Some("en_NZ")), South);
        assert_eq!(hemisphere(None, Some("de_DE.UTF-8@euro")), North);
    }

    #[test]
    fn with_nothing_to_go_on_the_answer_is_north() {
        assert_eq!(hemisphere(None, None), North);
        assert_eq!(hemisphere(Some(""), Some("C.UTF-8")), North);
        assert_eq!(hemisphere(Some("UTC"), Some("POSIX")), North);
        assert_eq!(hemisphere(None, Some("en")), North);
    }
}
