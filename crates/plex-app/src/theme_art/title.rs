//! One library title in a server-neutral form, and what makes it worth
//! showing behind a theme.

use std::collections::BTreeMap;

use super::canon::{self, normalise, CanonEntry};
use super::genre::genre_suites;
use super::image::{Look, Size};
use super::theme::{Feed, ThemeId};

/// The app's own id for a title (a Plex rating key or a `jellyfin:` id),
/// opaque here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TitleKey(pub String);

/// The app's own address for one image of a title (a Plex art path or a
/// Jellyfin backdrop tag route), opaque here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageKey(pub String);

/// A film or a series. Shows are allowed but rank below films.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A film.
    Film,
    /// A series.
    Show,
}

/// A rating on a 0 to 10 scale. Built only from a declared source scale, so a
/// percentage can never be read as a 10-point score.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Score10(f32);

impl Score10 {
    /// A 10-point value (Jellyfin `CommunityRating`, Plex `audienceRating`).
    /// `None` when it is not finite or outside `0..=10`.
    pub fn out_of_10(value: f64) -> Option<Self> {
        (value.is_finite() && (0.0..=10.0).contains(&value)).then_some(Self(value as f32))
    }

    /// A 100-point value (Jellyfin `CriticRating`, a Tomatometer).
    pub fn out_of_100(value: f64) -> Option<Self> {
        Self::out_of_10(value / 10.0)
    }

    /// The value, `0..=10`.
    pub fn get(self) -> f32 {
        self.0
    }
}

/// One image of a title.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    /// Where to fetch it.
    pub key: ImageKey,
    /// Its full size, when the server reported one.
    pub size: Option<Size>,
    /// What a decoded thumbnail of it looks like, once measured with
    /// [`look`](super::look).
    pub look: Option<Look>,
}

/// One library title, built by the Plex or Jellyfin adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The app's id for it.
    pub key: TitleKey,
    /// Film or series.
    pub kind: Kind,
    /// Display title.
    pub title: String,
    /// Original-language title, when the server has one.
    pub original_title: Option<String>,
    /// Release year.
    pub year: Option<u16>,
    /// Genres as the server spells them.
    pub genres: Vec<String>,
    /// Collections (Plex) or box sets (Jellyfin) it belongs to.
    pub collections: Vec<String>,
    /// Subject keywords (Jellyfin's `Tags`, which its TMDb metadata fills
    /// with TMDb's keywords: `christmas`, `snow`, `heist`).
    pub keywords: Vec<String>,
    /// Critics' score.
    pub critic: Option<Score10>,
    /// Audience or community score.
    pub audience: Option<Score10>,
    /// Times this user has watched it.
    pub view_count: u64,
    /// External ids by provider, keys lower case (`imdb`, `tmdb`, `tvdb`).
    /// Used only to see that two servers hold the same title.
    pub provider_ids: BTreeMap<String, String>,
    /// Background images, in the server's order.
    pub images: Vec<ImageRef>,
}

/// Below this a title is never picked, unless the canon vouches for it.
pub const MIN_RATING: f32 = 6.5;
/// A canon match rated below this is more likely a different film of the same
/// name than a classic.
const CANON_FLOOR: f32 = 5.0;
/// The rating a canon film with no rating at all is taken to have.
const CANON_PRIOR: f32 = 7.5;
const SHOW_WEIGHT: f32 = 0.85;
const VIEWS_BONUS: f32 = 0.05;

impl Candidate {
    /// The canon film this is. Series are never canon.
    pub(super) fn canon(&self) -> Option<&'static CanonEntry> {
        match self.kind {
            Kind::Film => canon::find(&self.title, self.original_title.as_deref(), self.year),
            Kind::Show => None,
        }
    }

    /// The first of `theme`'s feeds this title satisfies, with its position.
    pub(super) fn feed_for(
        &self,
        theme: ThemeId,
        canon: Option<&CanonEntry>,
    ) -> Option<(usize, Feed)> {
        theme
            .feeds()
            .iter()
            .enumerate()
            .find(|(_, feed)| self.satisfies(**feed, theme, canon))
            .map(|(i, feed)| (i, *feed))
    }

    fn satisfies(&self, feed: Feed, theme: ThemeId, canon: Option<&CanonEntry>) -> bool {
        match feed {
            Feed::Bundled => false,
            Feed::Canon => canon.is_some_and(|c| c.suits(theme)),
            Feed::Genre(g) => {
                canon.is_some_and(|c| c.suits(ThemeId::Genre(g)))
                    || self.genres.iter().any(|s| genre_suites(s).contains(&g))
            }
            Feed::Festive(words) => {
                let names = std::iter::once(self.title.as_str())
                    .chain(self.original_title.as_deref())
                    .chain(self.collections.iter().map(String::as_str))
                    .chain(self.genres.iter().map(String::as_str));
                uses_any(names, words.names)
                    || uses_any(self.keywords.iter().map(String::as_str), words.keywords)
            }
        }
    }

    /// The mean of the ratings it has.
    fn rating(&self) -> Option<f32> {
        let scores: Vec<f32> = [self.critic, self.audience]
            .into_iter()
            .flatten()
            .map(Score10::get)
            .collect();
        (!scores.is_empty()).then(|| scores.iter().sum::<f32>() / scores.len() as f32)
    }

    /// How much this title deserves the background, `0..=1`, or `None` when it
    /// must never be picked. Ratings dominate; a film outweighs a show; titles
    /// the user has watched get a small lift.
    pub(super) fn merit(&self, canon: bool) -> Option<f32> {
        let rating = match (self.rating(), canon) {
            (Some(r), false) if r >= MIN_RATING => r,
            (Some(r), true) if r >= CANON_FLOOR => r,
            (None, true) => CANON_PRIOR,
            _ => return None,
        };
        let kind = match self.kind {
            Kind::Film => 1.0,
            Kind::Show => SHOW_WEIGHT,
        };
        let views = ((1.0 + self.view_count as f32).log10()).min(1.0) * VIEWS_BONUS;
        Some((rating / 10.0 * kind + views).min(1.0))
    }

    /// Ids that say two candidates are the same title on different servers.
    pub(super) fn identities(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .provider_ids
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("{}:{}", k.to_ascii_lowercase(), v.to_ascii_lowercase()))
            .collect();
        if let Some(year) = self.year {
            ids.push(format!("title:{}:{year}", normalise(&self.title)));
        }
        ids
    }
}

/// Whether any of `texts` uses any of `words` as whole words.
fn uses_any<'a>(mut texts: impl Iterator<Item = &'a str>, words: &[&str]) -> bool {
    texts.any(|text| {
        let text = padded(text);
        words.iter().any(|w| text.contains(&format!(" {w} ")))
    })
}

fn padded(text: &str) -> String {
    format!(" {} ", normalise(text))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(super) mod tests {
    use super::*;
    use crate::theme_art::{Festival, Genre};

    /// A well-rated film with one measured-unknown 4K backdrop.
    pub fn film(key: &str, title: &str, year: u16, genres: &[&str], rating: f64) -> Candidate {
        Candidate {
            key: TitleKey(key.to_owned()),
            kind: Kind::Film,
            title: title.to_owned(),
            original_title: None,
            year: Some(year),
            genres: genres.iter().map(|g| (*g).to_owned()).collect(),
            collections: Vec::new(),
            keywords: Vec::new(),
            critic: None,
            audience: Score10::out_of_10(rating),
            view_count: 0,
            provider_ids: BTreeMap::new(),
            images: vec![ImageRef {
                key: ImageKey(format!("{key}/art")),
                size: Some(Size {
                    width: 3840,
                    height: 2160,
                }),
                look: None,
            }],
        }
    }

    #[test]
    fn a_percentage_cannot_pass_for_a_ten_point_score() {
        assert!(Score10::out_of_10(87.0).is_none());
        assert_eq!(Score10::out_of_100(87.0).unwrap().get(), 8.7);
        assert!(Score10::out_of_10(f64::NAN).is_none());
        assert!(Score10::out_of_10(-0.5).is_none());
    }

    #[test]
    fn christmas_matches_whole_words_in_title_or_collection() {
        let christmas = ThemeId::Festival(Festival::Christmas);
        let carol = film("1", "A Christmas Carol", 2009, &["Animation"], 7.0);
        assert!(carol.feed_for(christmas, None).is_some());

        let mut in_set = film("2", "Home Alone", 1990, &["Comedy"], 7.4);
        assert!(in_set.feed_for(christmas, None).is_none());
        in_set.collections.push("Christmas Classics".to_owned());
        assert!(in_set.feed_for(christmas, None).is_some());

        let mut home_alone = film("4", "Home Alone", 1990, &["Comedy"], 7.4);
        home_alone.keywords = vec!["burglar".to_owned(), "christmas".to_owned()];
        assert!(home_alone.feed_for(christmas, None).is_some());
        let mut die_hard = film("5", "Die Hard", 1988, &["Action"], 8.6);
        die_hard.keywords = vec!["christmas party".to_owned()];
        assert!(die_hard.feed_for(christmas, None).is_some());
        let mut hobbit = film("6", "Middle Earth", 2012, &["Fantasy"], 7.8);
        hobbit.keywords = vec!["elf".to_owned(), "dwarf".to_owned()];
        assert!(
            hobbit.feed_for(christmas, None).is_none(),
            "a keyword that names something else does not count"
        );

        let shelf = film("3", "Shelf Life", 2001, &["Drama"], 7.0);
        assert!(
            shelf.feed_for(christmas, None).is_none(),
            "elf inside a word"
        );
    }

    #[test]
    fn halloween_prefers_its_own_keywords_over_plain_horror() {
        let halloween = ThemeId::Festival(Festival::Halloween);
        let night = film("1", "Halloween", 1978, &["Horror"], 7.5);
        let other = film("2", "The Conjuring", 2013, &["Horror"], 7.5);
        let (night_feed, _) = night.feed_for(halloween, night.canon()).unwrap();
        let (other_feed, _) = other.feed_for(halloween, other.canon()).unwrap();
        assert!(night_feed < other_feed);
    }

    #[test]
    fn canon_tags_admit_a_film_whose_server_genres_do_not() {
        let revenant = film("1", "The Revenant", 2015, &["Drama", "Thriller"], 7.6);
        let canon = revenant.canon();
        assert!(canon.is_some());
        assert!(revenant
            .feed_for(ThemeId::Genre(Genre::Western), canon)
            .is_some());
        assert!(revenant
            .feed_for(ThemeId::Genre(Genre::Western), None)
            .is_none());
    }

    #[test]
    fn series_are_never_canon() {
        let mut show = film("1", "Dune", 2021, &["Science Fiction"], 8.0);
        show.kind = Kind::Show;
        assert!(show.canon().is_none());
    }

    #[test]
    fn low_and_missing_ratings_are_never_merit() {
        assert!(film("1", "Bad", 2020, &["Horror"], 4.1)
            .merit(false)
            .is_none());
        let mut unrated = film("2", "Unknown", 2020, &["Horror"], 0.0);
        unrated.audience = None;
        assert!(unrated.merit(false).is_none());
        assert!(unrated.merit(true).is_some(), "the canon vouches for it");
        assert!(film("3", "Wrong", 2020, &[], 3.0).merit(true).is_none());
    }

    #[test]
    fn a_film_outranks_an_equally_rated_show() {
        let film_ = film("1", "A", 2020, &[], 8.0);
        let mut show = film("2", "B", 2020, &[], 8.0);
        show.kind = Kind::Show;
        assert!(film_.merit(false).unwrap() > show.merit(false).unwrap());
    }
}
