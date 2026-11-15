//! The theme vocabulary shared by the theme picker, the palette and the art
//! chooser.

/// A genre suite: art from films of one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Genre {
    /// Fantasy worlds, myth and fairy tale.
    Fantasy,
    /// Science fiction and space.
    SciFi,
    /// Historical, war and period epics.
    Epic,
    /// Horror.
    Horror,
    /// Westerns, classic and modern.
    Western,
    /// Nature, exploration and journeys.
    Adventure,
}

/// A meteorological season.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Season {
    /// December to February in the north.
    Winter,
    /// March to May in the north.
    Spring,
    /// June to August in the north.
    Summer,
    /// September to November in the north.
    Autumn,
}

/// A festive window on the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Festival {
    /// December 1 to January 6.
    Christmas,
    /// October 15 to November 1.
    Halloween,
}

/// One theme suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ThemeId {
    /// The bundled aurora photo, the default.
    Aurora,
    /// A genre suite.
    Genre(Genre),
    /// A season.
    Season(Season),
    /// A festive window.
    Festival(Festival),
}

/// Where a theme's background art may come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    /// A freely licensed photo shipped with the app, with its credit.
    Bundled,
    /// Library films that match a canon entry tagged with this theme itself.
    /// Genre suites do not need it: [`Feed::Genre`] already takes canon films
    /// tagged with the genre.
    Canon,
    /// Library titles whose genres map to this genre suite, and library films
    /// that match a canon entry tagged with it.
    Genre(Genre),
    /// Library titles that name a festival: see [`FestiveWords`].
    Festive(&'static FestiveWords),
}

/// What marks a title as one festival's.
#[derive(Debug, PartialEq, Eq)]
pub struct FestiveWords {
    /// Normalised words any of whose whole-word use in a title, original
    /// title, collection or genre admits it.
    pub names: &'static [&'static str],
    /// The narrower words a subject keyword (TMDb's, as Jellyfin's `Tags`)
    /// must use. Keywords are many and loose, so a word that also names
    /// something else ("elf" on a Tolkien film, "pumpkin" on a farm drama)
    /// is left out here.
    pub keywords: &'static [&'static str],
}

const CHRISTMAS: FestiveWords = FestiveWords {
    names: CHRISTMAS_WORDS,
    keywords: &["christmas", "xmas", "santa claus", "nativity"],
};

const HALLOWEEN: FestiveWords = FestiveWords {
    names: HALLOWEEN_WORDS,
    keywords: &["halloween"],
};

const CHRISTMAS_WORDS: &[&str] = &[
    "christmas",
    "xmas",
    "santa",
    "santa claus",
    "grinch",
    "nativity",
    "nutcracker",
    "scrooge",
    "mistletoe",
    "jingle",
    "sleigh",
    "elf",
];

const HALLOWEEN_WORDS: &[&str] = &["halloween", "pumpkin", "trick r treat", "hocus pocus"];

impl ThemeId {
    /// Every theme, in picker order: the default, genres, festivals, seasons.
    pub const ALL: [ThemeId; 13] = [
        ThemeId::Aurora,
        ThemeId::Genre(Genre::Fantasy),
        ThemeId::Genre(Genre::SciFi),
        ThemeId::Genre(Genre::Epic),
        ThemeId::Genre(Genre::Horror),
        ThemeId::Genre(Genre::Western),
        ThemeId::Genre(Genre::Adventure),
        ThemeId::Festival(Festival::Christmas),
        ThemeId::Festival(Festival::Halloween),
        ThemeId::Season(Season::Winter),
        ThemeId::Season(Season::Spring),
        ThemeId::Season(Season::Summer),
        ThemeId::Season(Season::Autumn),
    ];

    /// The name shown in the theme picker.
    pub const fn name(self) -> &'static str {
        match self {
            ThemeId::Aurora => "Aurora",
            ThemeId::Genre(Genre::Fantasy) => "Fantasy",
            ThemeId::Genre(Genre::SciFi) => "Science fiction",
            ThemeId::Genre(Genre::Epic) => "Epic",
            ThemeId::Genre(Genre::Horror) => "Horror",
            ThemeId::Genre(Genre::Western) => "Western",
            ThemeId::Genre(Genre::Adventure) => "Adventure",
            ThemeId::Festival(Festival::Christmas) => "Christmas",
            ThemeId::Festival(Festival::Halloween) => "Halloween",
            ThemeId::Season(Season::Winter) => "Winter",
            ThemeId::Season(Season::Spring) => "Spring",
            ThemeId::Season(Season::Summer) => "Summer",
            ThemeId::Season(Season::Autumn) => "Autumn",
        }
    }

    /// The theme's stable name in the settings file.
    pub const fn key(self) -> &'static str {
        match self {
            ThemeId::Aurora => "aurora",
            ThemeId::Genre(Genre::Fantasy) => "fantasy",
            ThemeId::Genre(Genre::SciFi) => "scifi",
            ThemeId::Genre(Genre::Epic) => "epic",
            ThemeId::Genre(Genre::Horror) => "horror",
            ThemeId::Genre(Genre::Western) => "western",
            ThemeId::Genre(Genre::Adventure) => "adventure",
            ThemeId::Festival(Festival::Christmas) => "christmas",
            ThemeId::Festival(Festival::Halloween) => "halloween",
            ThemeId::Season(Season::Winter) => "winter",
            ThemeId::Season(Season::Spring) => "spring",
            ThemeId::Season(Season::Summer) => "summer",
            ThemeId::Season(Season::Autumn) => "autumn",
        }
    }

    /// The theme a settings file names by [`ThemeId::key`].
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == key)
    }

    /// The sources that feed this theme, most specific first: a title matched
    /// by an earlier feed ranks above one matched only by a later feed. Every
    /// theme ends with its bundled photo, so a library with nothing that fits
    /// still has art.
    ///
    /// Spring, summer and autumn take only the bundled photo: no metadata a
    /// server keeps says a film looks like spring. Winter takes canon films
    /// tagged as snowbound. Halloween borrows the horror genre after its own
    /// keywords.
    pub const fn feeds(self) -> &'static [Feed] {
        match self {
            ThemeId::Aurora | ThemeId::Season(Season::Spring | Season::Summer | Season::Autumn) => {
                &[Feed::Bundled]
            }
            ThemeId::Season(Season::Winter) => &[Feed::Canon, Feed::Bundled],
            ThemeId::Genre(Genre::Fantasy) => &[Feed::Genre(Genre::Fantasy), Feed::Bundled],
            ThemeId::Genre(Genre::SciFi) => &[Feed::Genre(Genre::SciFi), Feed::Bundled],
            ThemeId::Genre(Genre::Epic) => &[Feed::Genre(Genre::Epic), Feed::Bundled],
            ThemeId::Genre(Genre::Horror) => &[Feed::Genre(Genre::Horror), Feed::Bundled],
            ThemeId::Genre(Genre::Western) => &[Feed::Genre(Genre::Western), Feed::Bundled],
            ThemeId::Genre(Genre::Adventure) => &[Feed::Genre(Genre::Adventure), Feed::Bundled],
            ThemeId::Festival(Festival::Christmas) => &[Feed::Festive(&CHRISTMAS), Feed::Bundled],
            ThemeId::Festival(Festival::Halloween) => &[
                Feed::Festive(&HALLOWEEN),
                Feed::Genre(Genre::Horror),
                Feed::Bundled,
            ],
        }
    }

    /// True when the user's library can feed this theme at all.
    pub fn uses_library(self) -> bool {
        self.feeds().iter().any(|f| *f != Feed::Bundled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme_art::normalise;

    #[test]
    fn every_theme_has_a_bundled_fallback_and_a_unique_name() {
        let mut names = std::collections::BTreeSet::new();
        for theme in ThemeId::ALL {
            assert_eq!(theme.feeds().last(), Some(&Feed::Bundled), "{theme:?}");
            assert!(
                names.insert(theme.name()),
                "duplicate name {}",
                theme.name()
            );
        }
    }

    #[test]
    fn every_theme_reads_back_from_its_key() {
        for theme in ThemeId::ALL {
            assert_eq!(ThemeId::from_key(theme.key()), Some(theme));
        }
        assert_eq!(ThemeId::from_key("violet"), None);
    }

    #[test]
    fn library_art_feeds_genre_and_festive_themes_but_not_the_default() {
        assert!(!ThemeId::Aurora.uses_library());
        assert!(!ThemeId::Season(Season::Summer).uses_library());
        assert!(ThemeId::Season(Season::Winter).uses_library());
        assert!(ThemeId::Genre(Genre::Western).uses_library());
        assert!(ThemeId::Festival(Festival::Christmas).uses_library());
    }

    #[test]
    fn festive_words_are_already_normalised() {
        for theme in ThemeId::ALL {
            for feed in theme.feeds() {
                if let Feed::Festive(words) = feed {
                    for w in words.names.iter().chain(words.keywords) {
                        assert_eq!(normalise(w), *w);
                    }
                }
            }
        }
    }
}
