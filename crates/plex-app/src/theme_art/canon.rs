//! The cinematography canon: films renowned for their photography and scale,
//! and the matcher from a library title to one of them.
//!
//! The table ([`ENTRIES`]) is curated by hand from Academy, BAFTA and ASC
//! cinematography winners and nominees plus the large fantasy, science
//! fiction and historical epics everyone knows. It keys on title and year
//! only, because a guessed provider id would be worse than none: servers agree
//! on a film's title and year far more often than an invented id would.

use super::theme::ThemeId;

pub use super::canon_table::ENTRIES;

/// One canon film.
#[derive(Debug)]
pub struct CanonEntry {
    /// Normalised titles: the English release title first, then the original
    /// title or a common alternative a server might carry.
    pub titles: &'static [&'static str],
    /// Release year.
    pub year: u16,
    /// The themes this film's imagery suits. Never empty.
    pub tags: &'static [ThemeId],
}

impl CanonEntry {
    /// True when this film suits `theme`.
    pub fn suits(&self, theme: ThemeId) -> bool {
        self.tags.contains(&theme)
    }
}

/// The canon film a library item is, if any. `year` may differ from the
/// canon's by one, because servers disagree on festival and release years.
/// An item without a year never matches: a title alone collides with remakes.
pub fn find(
    title: &str,
    original_title: Option<&str>,
    year: Option<u16>,
) -> Option<&'static CanonEntry> {
    let year = year?;
    let names: Vec<String> = std::iter::once(title)
        .chain(original_title)
        .map(normalise)
        .filter(|n| !n.is_empty())
        .collect();
    ENTRIES
        .iter()
        .filter(|e| e.year.abs_diff(year) <= 1)
        .filter(|e| e.titles.iter().any(|t| names.iter().any(|n| n == t)))
        .min_by_key(|e| e.year.abs_diff(year))
}

/// A title reduced to the form two servers agree on: lower case, Latin
/// diacritics folded to ASCII, `&` read as "and", apostrophes and full stops
/// dropped, other punctuation read as a space, and a leading or trailing
/// (sort-title) "the" removed.
pub fn normalise(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for c in title.chars().flat_map(char::to_lowercase) {
        match c {
            '\'' | '\u{2019}' | '\u{2018}' | '`' | '\u{b4}' | '.' => {}
            '\u{300}'..='\u{36f}' => {}
            '&' => out.push_str(" and "),
            c if c.is_ascii_alphanumeric() => out.push(c),
            c => match fold(c) {
                Some(ascii) => out.push_str(ascii),
                None if c.is_alphanumeric() => out.push(c),
                None => out.push(' '),
            },
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    let words = match words.as_slice() {
        ["the", rest @ ..] if !rest.is_empty() => rest,
        [rest @ .., "the"] if !rest.is_empty() => rest,
        all => all,
    };
    words.join(" ")
}

/// ASCII for the lower-case letters of Latin-1 Supplement and Latin
/// Extended-A, which cover the European titles a library carries.
fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĳ' => "ij",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::theme_art::{Genre, Season};

    #[test]
    fn every_entry_is_normalised_tagged_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        assert!((120..=250).contains(&ENTRIES.len()), "{}", ENTRIES.len());
        for e in ENTRIES {
            assert!(!e.tags.is_empty(), "{:?} has no tags", e.titles);
            assert!((1900..=2030).contains(&e.year), "{:?}", e.titles);
            for t in e.titles {
                assert_eq!(normalise(t), *t, "not normalised");
                assert!(seen.insert((*t, e.year)), "duplicate {t} {}", e.year);
            }
        }
    }

    #[test]
    fn matches_despite_case_punctuation_and_articles() {
        let lotr = find(
            "The Lord of the Rings: The Fellowship of the Ring",
            None,
            Some(2001),
        );
        assert!(lotr.unwrap().suits(ThemeId::Genre(Genre::Fantasy)));
        assert!(find(
            "lord of the rings - the fellowship of the ring",
            None,
            Some(2001)
        )
        .is_some());
        assert!(find("Revenant, The", None, Some(2015)).is_some());
        assert!(find("2001: A Space Odyssey", None, Some(1968)).is_some());
        assert!(find("E.T. the Extra-Terrestrial", None, Some(1982)).is_some());
        assert!(find("Harry Potter and the Philosopher’s Stone", None, Some(2001)).is_some());
    }

    #[test]
    fn ampersand_and_and_are_the_same_title() {
        let amp = find("McCabe & Mrs. Miller", None, Some(1971)).unwrap();
        let and = find("McCabe and Mrs Miller", None, Some(1971)).unwrap();
        assert!(std::ptr::eq(amp, and));
        assert!(amp.suits(ThemeId::Genre(Genre::Western)));
    }

    #[test]
    fn diacritics_fold_in_both_composed_and_decomposed_form() {
        assert!(find("Nausicaä of the Valley of the Wind", None, Some(1984)).is_some());
        assert!(find(
            "Nausica\u{61}\u{308} of the Valley of the Wind",
            None,
            Some(1984)
        )
        .is_some());
        assert_eq!(
            normalise("Låt den rätte komma in"),
            "lat den ratte komma in"
        );
    }

    #[test]
    fn original_title_matches_when_the_display_title_is_localised() {
        let found = find(
            "So finster die Nacht",
            Some("Låt den rätte komma in"),
            Some(2008),
        );
        assert!(found.unwrap().suits(ThemeId::Season(Season::Winter)));
    }

    #[test]
    fn year_may_be_off_by_one_but_no_more_and_must_be_present() {
        assert!(find("Blade Runner 2049", None, Some(2018)).is_some());
        assert!(find("Blade Runner 2049", None, Some(2016)).is_some());
        assert!(find("Blade Runner 2049", None, Some(2019)).is_none());
        assert!(find("Blade Runner 2049", None, None).is_none());
    }

    #[test]
    fn a_remake_far_from_the_canon_year_does_not_match() {
        assert!(find("Dune", None, Some(2021)).is_some());
        assert!(find("Dune", None, Some(1984)).is_none());
    }

    #[test]
    fn a_title_that_only_contains_a_canon_title_does_not_match() {
        assert!(find("Alien", None, Some(1979)).is_some());
        assert!(find("Alien Resurrection", None, Some(1979)).is_none());
        assert!(find("The", None, Some(1979)).is_none());
        assert!(find("", None, Some(1979)).is_none());
    }
}
