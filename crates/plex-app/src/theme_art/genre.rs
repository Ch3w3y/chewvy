//! Server genre strings, read as genre suites.
//!
//! Plex films carry its own agent's genres ("Science Fiction", "War",
//! "History"); Jellyfin carries TMDB's, whose television list merges some
//! ("Sci-Fi & Fantasy", "Action & Adventure", "War & Politics"). Both reach
//! here as [`normalise`]d text, so "Sci-Fi & Fantasy" and "sci fi and fantasy"
//! are one key.
//!
//! [`normalise`]: super::canon::normalise

use super::canon::normalise;
use super::theme::Genre;

const TABLE: &[(&str, &[Genre])] = &[
    ("fantasy", &[Genre::Fantasy]),
    ("fairy tale", &[Genre::Fantasy]),
    ("sword and sorcery", &[Genre::Fantasy]),
    ("science fiction", &[Genre::SciFi]),
    ("sci fi", &[Genre::SciFi]),
    ("scifi", &[Genre::SciFi]),
    ("sci fi and fantasy", &[Genre::SciFi, Genre::Fantasy]),
    (
        "science fiction and fantasy",
        &[Genre::SciFi, Genre::Fantasy],
    ),
    ("war", &[Genre::Epic]),
    ("history", &[Genre::Epic]),
    ("historical", &[Genre::Epic]),
    ("war and politics", &[Genre::Epic]),
    ("epic", &[Genre::Epic]),
    ("horror", &[Genre::Horror]),
    ("western", &[Genre::Western]),
    ("adventure", &[Genre::Adventure]),
    ("action and adventure", &[Genre::Adventure]),
    ("nature", &[Genre::Adventure]),
    ("travel", &[Genre::Adventure]),
];

/// The genre suites a server genre belongs to; empty when it is none of them
/// ("Drama", "Comedy", ...).
pub fn genre_suites(genre: &str) -> &'static [Genre] {
    let key = normalise(genre);
    TABLE
        .iter()
        .find(|(name, _)| *name == key)
        .map_or(&[], |(_, suites)| *suites)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plex_and_jellyfin_spellings_reach_the_same_suite() {
        assert_eq!(genre_suites("Science Fiction"), &[Genre::SciFi]);
        assert_eq!(genre_suites("Sci-Fi"), &[Genre::SciFi]);
        assert_eq!(
            genre_suites("Sci-Fi & Fantasy"),
            &[Genre::SciFi, Genre::Fantasy]
        );
        assert_eq!(genre_suites("War"), &[Genre::Epic]);
        assert_eq!(genre_suites("History"), &[Genre::Epic]);
        assert_eq!(genre_suites("War & Politics"), &[Genre::Epic]);
        assert_eq!(genre_suites("Action & Adventure"), &[Genre::Adventure]);
        assert_eq!(genre_suites("WESTERN"), &[Genre::Western]);
    }

    #[test]
    fn genres_outside_every_suite_map_to_nothing() {
        assert!(genre_suites("Drama").is_empty());
        assert!(genre_suites("Action").is_empty());
        assert!(genre_suites("").is_empty());
    }

    #[test]
    fn table_keys_are_normalised() {
        for (name, suites) in TABLE {
            assert_eq!(normalise(name), *name);
            assert!(!suites.is_empty());
        }
    }
}
