//! Turning the server's item metadata into what the player shows.

use plex_core::library::Item;

use super::state::{ChapterMark, MarkerKind, MarkerSpan, MediaFacts, NextUp, SubFile};

fn ms_to_secs(ms: Option<u64>) -> Option<f64> {
    ms.map(|ms| ms as f64 / 1000.0)
}

/// "S4 E2" for an episode with both numbers.
fn episode_code(item: &Item) -> Option<String> {
    Some(format!("S{} E{}", item.parent_index?, item.index?))
}

/// The two title lines: show + "S4 E2 · Episode" for episodes, title + year otherwise.
#[must_use]
pub fn titles(item: &Item, fallback: &str) -> (String, Option<String>) {
    let own = item.title.clone().unwrap_or_else(|| fallback.to_owned());
    if item.kind.as_deref() == Some("episode") {
        if let Some(show) = item.grandparent_title.clone() {
            let line = match episode_code(item) {
                Some(code) => format!("{code} · {own}"),
                None => own,
            };
            return (show, Some(line));
        }
    }
    (own, item.year.map(|y| y.to_string()))
}

/// Intro and credits ranges, in seconds.
#[must_use]
pub fn markers(item: &Item) -> Vec<MarkerSpan> {
    item.markers
        .iter()
        .filter_map(|m| {
            let kind = match m.marker_type.as_deref() {
                Some("intro") => MarkerKind::Intro,
                Some("credits") => MarkerKind::Credits,
                _ => return None,
            };
            let start = ms_to_secs(m.start_offset_ms)?;
            let end = ms_to_secs(m.end_offset_ms)?;
            (end > start).then_some(MarkerSpan { kind, start, end })
        })
        .collect()
}

/// Chapter starts with their preview thumbnails.
#[must_use]
pub fn chapters(item: &Item) -> Vec<ChapterMark> {
    item.chapters
        .iter()
        .filter_map(|c| {
            Some(ChapterMark {
                start: ms_to_secs(c.start_offset_ms)?,
                thumb: c.thumb.clone(),
            })
        })
        .collect()
}

/// Formats mpv cannot load from one URL (VobSub needs its `.idx` and `.sub`).
const UNLOADABLE_SUBTITLES: [&str; 1] = ["vobsub"];

/// The first version's sidecar subtitle files, as URLs on `base`.
#[must_use]
pub fn sidecars(item: &Item, base: &str) -> Vec<SubFile> {
    let Some(part) = item.media.first().and_then(|m| m.parts.first()) else {
        return Vec::new();
    };
    part.streams
        .iter()
        .filter(|s| s.stream_type == Some(3))
        .filter(|s| {
            !s.codec
                .as_deref()
                .is_some_and(|c| UNLOADABLE_SUBTITLES.contains(&c))
        })
        .filter_map(|s| {
            let key = s.key.as_deref().filter(|k| k.starts_with('/'))?;
            Some(SubFile {
                url: format!("{}{key}", base.trim_end_matches('/')),
                lang: s.language_tag.clone().or_else(|| s.language.clone()),
                title: s.title.clone(),
                forced: s.forced,
            })
        })
        .collect()
}

/// The first version's file facts.
#[must_use]
pub fn facts(item: &Item) -> MediaFacts {
    let Some(media) = item.media.first() else {
        return MediaFacts::default();
    };
    MediaFacts {
        container: media.container.clone(),
        video_codec: media.video_codec.clone(),
        video_profile: media.video_profile.clone(),
        resolution: media.video_resolution.clone(),
        bitrate_kbps: media.bitrate,
        audio_codec: media.audio_codec.clone(),
        audio_channels: media.audio_channels,
        size_bytes: media.parts.first().and_then(|p| p.size),
    }
}

/// The episode after `item` among `siblings` (its season's episodes), if any.
#[must_use]
pub fn next_in(item: &Item, siblings: &[Item]) -> Option<NextUp> {
    let index = item.index?;
    siblings
        .iter()
        .filter(|s| s.index.is_some_and(|i| i > index))
        .min_by_key(|s| s.index)
        .and_then(next_up)
}

/// The first episode of a later season's listing.
#[must_use]
pub fn first_of(episodes: &[Item]) -> Option<NextUp> {
    episodes.iter().min_by_key(|e| e.index).and_then(next_up)
}

/// The season that follows `season_index` among a show's seasons.
#[must_use]
pub fn next_season(season_index: u64, seasons: &[Item]) -> Option<&Item> {
    seasons
        .iter()
        .filter(|s| s.index.is_some_and(|i| i > season_index))
        .min_by_key(|s| s.index)
}

fn next_up(episode: &Item) -> Option<NextUp> {
    let (_, line) = titles(episode, "");
    Some(NextUp {
        rating_key: episode.rating_key.clone()?,
        label: line.or_else(|| episode.title.clone())?,
        thumb: episode.thumb.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::{next_in, next_season, sidecars, titles};
    use plex_core::library::{Item, Media, MediaPart, Stream};

    fn episode(key: &str, season: u64, index: u64, title: &str) -> Item {
        Item {
            rating_key: Some(key.to_owned()),
            kind: Some("episode".to_owned()),
            title: Some(title.to_owned()),
            index: Some(index),
            parent_index: Some(season),
            grandparent_title: Some("Show".to_owned()),
            ..Item::default()
        }
    }

    #[test]
    fn an_episode_titles_as_show_then_code_and_name() {
        let (title, line) = titles(&episode("1", 4, 2, "Blue Rafter"), "x");
        assert_eq!(title, "Show");
        assert_eq!(line.as_deref(), Some("S4 E2 · Blue Rafter"));
    }

    #[test]
    fn a_movie_titles_with_its_year() {
        let movie = Item {
            title: Some("Dune".to_owned()),
            year: Some(2021),
            kind: Some("movie".to_owned()),
            ..Item::default()
        };
        assert_eq!(
            titles(&movie, "x"),
            ("Dune".to_owned(), Some("2021".to_owned()))
        );
    }

    #[test]
    fn next_up_is_the_following_index_not_the_list_order() {
        let season = vec![
            episode("c", 1, 3, "Three"),
            episode("a", 1, 1, "One"),
            episode("b", 1, 2, "Two"),
        ];
        let next = next_in(&episode("a", 1, 1, "One"), &season);
        assert_eq!(next.map(|n| n.rating_key), Some("b".to_owned()));
        assert!(next_in(&episode("c", 1, 3, "Three"), &season).is_none());
    }

    #[test]
    fn the_next_season_skips_specials_and_earlier_seasons() {
        let seasons: Vec<Item> = [0, 1, 2, 3]
            .into_iter()
            .map(|i| Item {
                index: Some(i),
                ..Item::default()
            })
            .collect();
        assert_eq!(next_season(1, &seasons).and_then(|s| s.index), Some(2));
        assert!(next_season(3, &seasons).is_none());
    }

    #[test]
    fn sidecars_are_external_text_subtitles_with_server_labels() {
        let stream = |key: Option<&str>, codec: &str| Stream {
            stream_type: Some(3),
            codec: Some(codec.into()),
            language_tag: Some("en".into()),
            title: Some("SDH".into()),
            key: key.map(Into::into),
            ..Stream::default()
        };
        let item = Item {
            media: vec![Media {
                parts: vec![MediaPart {
                    streams: vec![
                        stream(None, "ass"),
                        stream(Some("/library/streams/7"), "srt"),
                        stream(Some("/library/streams/8"), "vobsub"),
                    ],
                    ..MediaPart::default()
                }],
                ..Media::default()
            }],
            ..Item::default()
        };
        let subs = sidecars(&item, "https://tower:32400/");
        assert_eq!(subs.len(), 1);
        let sub = subs.first();
        assert_eq!(
            sub.map(|s| s.url.as_str()),
            Some("https://tower:32400/library/streams/7")
        );
        assert_eq!(sub.and_then(|s| s.lang.as_deref()), Some("en"));
        assert!(!sub.is_some_and(|s| s.url.contains("Token")));
    }
}
