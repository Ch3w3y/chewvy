//! The day's picks for a theme, and which one is on screen at a moment.

use std::collections::HashSet;
use std::num::NonZeroU32;

use super::calendar::LocalTime;
use super::image::image_score;
use super::theme::{Feed, ThemeId};
use super::title::{Candidate, ImageKey, ImageRef, TitleKey};

/// How many picks [`rank`] keeps.
pub const DEFAULT_PICKS: usize = 20;

const TITLE_WEIGHT: f32 = 0.6;
const IMAGE_WEIGHT: f32 = 0.4;

/// One title chosen for a theme, with the image to show for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    /// The title.
    pub title: TitleKey,
    /// Its best image.
    pub image: ImageKey,
    /// Title merit and image quality combined, `0..=1`.
    pub score: f32,
    /// The feed that admitted it.
    pub feed: Feed,
    /// True when it is a canon film.
    pub canon: bool,
}

/// The best [`DEFAULT_PICKS`] titles for `theme`. See [`rank_n`].
pub fn rank(theme: ThemeId, candidates: &[Candidate]) -> Vec<Pick> {
    rank_n(theme, candidates, DEFAULT_PICKS)
}

/// The best `n` titles for `theme`, best first. A title matched by an earlier
/// feed of the theme comes first, then canon films, then by score; ties break
/// on the title key, so the result does not depend on the input order. A
/// title held by two servers (a shared provider id, or the same title and
/// year) appears once. Titles that are low rated, unrated outside the canon,
/// or have no showable image are never picked.
pub fn rank_n(theme: ThemeId, candidates: &[Candidate], n: usize) -> Vec<Pick> {
    let mut ranked: Vec<(usize, Pick, &Candidate)> = candidates
        .iter()
        .filter_map(|c| assess(theme, c).map(|(feed_at, pick)| (feed_at, pick, c)))
        .collect();
    ranked.sort_by(|(fa, a, _), (fb, b, _)| {
        fa.cmp(fb)
            .then(b.canon.cmp(&a.canon))
            .then(b.score.total_cmp(&a.score))
            .then_with(|| a.title.cmp(&b.title))
    });
    let mut seen = HashSet::new();
    ranked
        .into_iter()
        .filter(|(_, _, c)| {
            let ids = c.identities();
            let fresh = !ids.iter().any(|id| seen.contains(id));
            seen.extend(ids);
            fresh
        })
        .take(n)
        .map(|(_, pick, _)| pick)
        .collect()
}

fn assess(theme: ThemeId, c: &Candidate) -> Option<(usize, Pick)> {
    let canon = c.canon();
    let (feed_at, feed) = c.feed_for(theme, canon)?;
    let merit = c.merit(canon.is_some())?;
    let (image, quality) = best_image(&c.images)?;
    let pick = Pick {
        title: c.key.clone(),
        image: image.key.clone(),
        score: TITLE_WEIGHT * merit + IMAGE_WEIGHT * quality,
        feed,
        canon: canon.is_some(),
    };
    Some((feed_at, pick))
}

/// The highest-scoring showable image; the server's earlier image wins a tie.
fn best_image(images: &[ImageRef]) -> Option<(&ImageRef, f32)> {
    images
        .iter()
        .filter_map(|i| image_score(i.size, i.look.as_ref()).map(|s| (i, s)))
        .fold(None, |best, (i, s)| match best {
            Some((_, b)) if b >= s => best,
            _ => Some((i, s)),
        })
}

/// The pick on screen at `now`, changing every `every` minutes. Each local day
/// shuffles the picks into its own order, so the same day always shows the
/// same sequence and the next day a different one. `None` with no picks.
pub fn showing(picks: &[Pick], now: LocalTime, every: NonZeroU32) -> Option<&Pick> {
    let day = now.day_number();
    let mut order: Vec<&Pick> = picks.iter().collect();
    order.sort_by(|a, b| {
        shuffle_key(day, &a.title)
            .cmp(&shuffle_key(day, &b.title))
            .then_with(|| a.title.cmp(&b.title))
    });
    let slot = usize::try_from(now.minute_of_day() / every.get()).ok()?;
    let at = slot.checked_rem(order.len())?;
    order.get(at).copied()
}

/// FNV-1a over the key, mixed with the day by the splitmix64 finaliser. A
/// keyed hash rather than a seeded shuffle, so adding one title to the
/// library moves only that title in the day's order.
fn shuffle_key(day: i64, key: &TitleKey) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.0.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut z = h ^ (day as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::theme_art::image::tests::{paint, rich, H, W};
    use crate::theme_art::image::Size;
    use crate::theme_art::title::tests::film;
    use crate::theme_art::{look, Genre, Kind, Score10, Thumbnail};

    const SCIFI: ThemeId = ThemeId::Genre(Genre::SciFi);

    fn keys(picks: &[Pick]) -> Vec<&str> {
        picks.iter().map(|p| p.title.0.as_str()).collect()
    }

    fn measured(key: &str, width: u32, px: &[u8]) -> ImageRef {
        ImageRef {
            key: ImageKey(key.to_owned()),
            size: Some(Size {
                width,
                height: width * 9 / 16,
            }),
            look: Some(look(&Thumbnail::new(W, H, px).unwrap())),
        }
    }

    #[test]
    fn a_low_rated_film_is_never_picked() {
        let flop = film("flop", "Space Flop", 2020, &["Science Fiction"], 3.9);
        assert!(rank(SCIFI, std::slice::from_ref(&flop)).is_empty());
        let mut with_great_art = flop;
        with_great_art.images = vec![measured("art", 3840, &rich())];
        let good = film("good", "Good Space", 2020, &["Science Fiction"], 7.2);
        assert_eq!(keys(&rank(SCIFI, &[with_great_art, good])), ["good"]);
    }

    #[test]
    fn canon_films_rank_above_better_rated_ordinary_ones() {
        let picks = rank(
            SCIFI,
            &[
                film("hit", "Some Space Hit", 2019, &["Science Fiction"], 9.1),
                film(
                    "bladerunner",
                    "Blade Runner 2049",
                    2017,
                    &["Science Fiction"],
                    7.0,
                ),
            ],
        );
        assert_eq!(keys(&picks), ["bladerunner", "hit"]);
        assert!(picks[0].canon);
    }

    #[test]
    fn a_title_outside_the_theme_is_not_picked() {
        let drama = film("d", "Quiet Drama", 2020, &["Drama"], 8.8);
        assert!(rank(SCIFI, &[drama]).is_empty());
        assert!(rank(ThemeId::Aurora, &[film("x", "X", 2020, &["Fantasy"], 9.0)]).is_empty());
    }

    #[test]
    fn a_1280_wide_backdrop_is_never_shown() {
        let mut only_small = film("small", "Small Art", 2020, &["Science Fiction"], 8.0);
        only_small.images = vec![measured("small/art", 1280, &rich())];
        assert!(rank(SCIFI, std::slice::from_ref(&only_small)).is_empty());

        only_small.images.push(measured(
            "big/art",
            2560,
            &paint(|_, y| [40, 60, (60 + y) as u8]),
        ));
        assert_eq!(rank(SCIFI, &[only_small])[0].image.0, "big/art");
    }

    #[test]
    fn the_rich_image_of_a_title_wins_over_a_dark_flat_one() {
        let mut f = film("f", "Far Planet", 2020, &["Science Fiction"], 8.0);
        f.images = vec![
            measured("dark", 3840, &paint(|_, _| [10, 10, 14])),
            measured("rich", 3840, &rich()),
        ];
        assert_eq!(rank(SCIFI, &[f])[0].image.0, "rich");
    }

    #[test]
    fn equal_titles_order_by_image_quality() {
        let mut dim = film("dim", "Dim Planet", 2020, &["Science Fiction"], 8.0);
        dim.images = vec![measured(
            "a",
            3840,
            &paint(|x, _| [70, 70, (90 + x / 4) as u8]),
        )];
        let mut bright = film("bright", "Bright Planet", 2020, &["Science Fiction"], 8.0);
        bright.images = vec![measured("b", 3840, &rich())];
        assert_eq!(keys(&rank(SCIFI, &[dim, bright])), ["bright", "dim"]);
    }

    #[test]
    fn films_are_preferred_to_equally_rated_shows() {
        let mut show = film("a-show", "Space Show", 2020, &["Sci-Fi & Fantasy"], 8.0);
        show.kind = Kind::Show;
        let movie = film("b-movie", "Space Movie", 2020, &["Science Fiction"], 8.0);
        assert_eq!(keys(&rank(SCIFI, &[show, movie])), ["b-movie", "a-show"]);
    }

    #[test]
    fn the_same_film_on_two_servers_is_picked_once() {
        let mut plex = film("plex-1", "Far Planet", 2020, &["Science Fiction"], 8.0);
        plex.provider_ids.insert("imdb".into(), "tt0000001".into());
        let mut jelly = film(
            "jellyfin:a:b",
            "Far Planet (Director's Cut)",
            2020,
            &["Science Fiction"],
            8.0,
        );
        jelly.provider_ids.insert("Imdb".into(), "TT0000001".into());
        let by_title = film("plex-2", "Far Planet", 2020, &["Science Fiction"], 8.0);
        assert_eq!(rank(SCIFI, &[plex, jelly, by_title]).len(), 1);
    }

    fn library() -> Vec<Candidate> {
        (0..30)
            .map(|i| {
                let mut c = film(
                    &format!("k{i:02}"),
                    &format!("Space {i}"),
                    2000,
                    &["Science Fiction"],
                    0.0,
                );
                c.audience = Score10::out_of_10(6.5 + f64::from(i % 7) * 0.5);
                c
            })
            .collect()
    }

    #[test]
    fn ranking_is_capped_and_independent_of_input_order() {
        let lib = library();
        let picks = rank(SCIFI, &lib);
        assert_eq!(picks.len(), DEFAULT_PICKS);
        let mut reversed = lib.clone();
        reversed.reverse();
        assert_eq!(rank(SCIFI, &reversed), picks);
        assert!(picks.windows(2).all(|w| w[0].score >= w[1].score));
    }

    fn at(day: i64, minute: i64) -> LocalTime {
        LocalTime::from_unix(day * 86_400 + minute * 60, 0)
    }

    fn day_sequence(picks: &[Pick], day: i64, every: NonZeroU32) -> Vec<String> {
        (0..1440)
            .step_by(every.get() as usize)
            .map(|m| showing(picks, at(day, m), every).unwrap().title.0.clone())
            .collect()
    }

    #[test]
    fn rotation_holds_a_pick_for_its_period_then_moves_on() {
        let picks = rank(SCIFI, &library());
        let every = NonZeroU32::new(30).unwrap();
        let day = 20_000;
        let first = showing(&picks, at(day, 60), every).unwrap();
        assert_eq!(showing(&picks, at(day, 89), every), Some(first));
        assert_ne!(showing(&picks, at(day, 90), every), Some(first));
        let seq = day_sequence(&picks, day, every);
        let distinct: HashSet<&String> = seq.iter().collect();
        assert_eq!(distinct.len(), picks.len(), "every pick is shown each day");
    }

    #[test]
    fn the_same_day_shows_the_same_sequence_and_other_days_differ() {
        let picks = rank(SCIFI, &library());
        let every = NonZeroU32::new(15).unwrap();
        let today = day_sequence(&picks, 20_000, every);
        assert_eq!(day_sequence(&rank(SCIFI, &library()), 20_000, every), today);
        let later: Vec<Vec<String>> = (1..=7)
            .map(|d| day_sequence(&picks, 20_000 + d, every))
            .collect();
        assert!(later.iter().all(|s| *s != today));
    }

    #[test]
    fn nothing_shows_without_picks() {
        assert!(showing(&[], at(1, 1), NonZeroU32::new(10).unwrap()).is_none());
    }
}
