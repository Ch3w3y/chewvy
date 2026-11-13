//! The item detail page: one movie, or one show / season with its episodes.
//!
//! [`show`] draws the page from what the worker already read — the item's own
//! `/library/metadata/{ratingKey}` answer plus its `/children` answer — and
//! answers with one [`Action`]: what to put in the player, or which season's
//! episodes to read next. Every label here comes from a field a captured answer
//! actually names: `year`, `contentRating`, `duration`, `audienceRating`,
//! `Genre[].tag`, `Role[]`, `Image[]` (`background`, `clearLogo`) and the
//! version facts of `Media[]` and its `Part[].Stream[]`. A field an answer does
//! not name is left out of the page rather than guessed at.

use plex_core::library::{Item, Media};

use crate::ui::home;

/// The page's drawing below its head: season chips, episode rows and cast.
pub(crate) mod drawing;
/// The page's head: art, title, meta, summary, play buttons and versions.
mod head;

pub(crate) use drawing::show;

#[cfg(test)]
mod tests;

/// The 16:9 thumbnail of one episode row.
pub const EPISODE_THUMB: [f32; 2] = [240.0, 135.0];
/// A cast member's round thumb.
pub const CAST_THUMB: f32 = 72.0;
/// Summary lines an episode row shows before the rest of its meta.
pub const EPISODE_SUMMARY_ROWS: usize = 2;
/// The play pill's smallest width and its height.
const PILL: [f32; 2] = [190.0, 36.0];
/// The progress bar along an episode thumbnail's bottom edge.
const BAR_HEIGHT: f32 = 4.0;
/// Width of one cast entry's column.
const CAST_COLUMN: f32 = 110.0;

/// What the page asked for: the player, or the worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Play this item, at `start` seconds when the server named a resume point.
    Play(Target),
    /// Read this season's episodes into the show page.
    Season { rating_key: String },
    /// Open another item's page: an episode's own, or the show or season
    /// above this one.
    Open { rating_key: String, title: String },
}

/// One thing the page can put in the player.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub rating_key: String,
    /// The title the player window shows.
    pub title: String,
    /// Seconds to open at, from the server's own `viewOffset`.
    pub start: Option<f64>,
}

/// The page's content: the item whose page this is, and the `/children`
/// answers read for it.
#[derive(Clone, Debug)]
pub struct Page {
    pub item: Item,
    /// A show's seasons, in the order its own `/children` answer listed them.
    /// Empty for a movie, an episode or a season.
    pub seasons: Vec<Item>,
    /// The episodes the page lists: a season item's own children, or the
    /// episodes of the season a show page has selected.
    pub episodes: Vec<Item>,
    /// The ratingKey `episodes` belong to — a show page's selected season, or
    /// the season item this page is itself.
    pub season_key: Option<String>,
    /// A show's next-up episode, fixed from the season the page opened on, so
    /// browsing other seasons never changes what the hero's Play continues.
    pub next_up: Option<Item>,
}

impl Page {
    /// One item with its own `/children` answer filed by kind. A show keeps its
    /// seasons and opens on the season [`default_season`] names; a season keeps
    /// its episodes.
    #[must_use]
    pub fn open(item: Item, children: &[Item]) -> Self {
        let seasons: Vec<Item> = seasons(children).into_iter().cloned().collect();
        let episodes: Vec<Item> = episodes(children).into_iter().cloned().collect();
        let season_key = if !episodes.is_empty() {
            item.rating_key.clone()
        } else {
            default_season(children).and_then(|season| season.rating_key.clone())
        };
        let next_up = next_episode(&episodes).cloned();
        Self {
            item,
            seasons,
            episodes,
            season_key,
            next_up,
        }
    }

    /// The rating key whose `/children` answer this page is still waiting for:
    /// the season a show page opens on, or a season page's own key when it has
    /// no episodes. Nothing once the page lists episodes, so a page that has
    /// been read is never read again.
    #[must_use]
    pub fn pending(&self) -> Option<&str> {
        if !self.episodes.is_empty() {
            return None;
        }
        match self.item.kind.as_deref() {
            Some("season") => self.item.rating_key.as_deref(),
            Some("show") => self.season_key.as_deref(),
            _ => None,
        }
    }

    /// A season chip's selection, taken in place: the season's own episodes
    /// replace the ones in view, so nothing the user was reading moves.
    pub fn select_season(&mut self, rating_key: &str) {
        self.season_key = Some(rating_key.to_owned());
        self.episodes.clear();
    }

    /// File one `/children` answer: seasons with their show, episodes with the
    /// season they belong to.
    pub fn fill(&mut self, rating_key: &str, children: &[Item]) {
        let listed_seasons = seasons(children);
        let listed_episodes = episodes(children);
        if !listed_episodes.is_empty() {
            self.season_key = Some(rating_key.to_owned());
            self.episodes = listed_episodes.into_iter().cloned().collect();
            // The first season filed is the one the page opened on.
            if self.next_up.is_none() {
                self.next_up = next_episode(&self.episodes).cloned();
            }
        } else if !listed_seasons.is_empty() {
            self.seasons = listed_seasons.into_iter().cloned().collect();
        }
    }
}

/// Whether this kind is played rather than browsed into.
#[must_use]
fn leaf(kind: Option<&str>) -> bool {
    matches!(
        kind,
        Some("movie") | Some("episode") | Some("clip") | Some("video")
    )
}

/// Whether this kind's `/children` answer holds the next level down.
#[must_use]
pub fn browsable(item: &Item) -> bool {
    matches!(item.kind.as_deref(), Some("show") | Some("season"))
}

/// Seasons in the order the server listed them.
#[must_use]
pub fn seasons(children: &[Item]) -> Vec<&Item> {
    children
        .iter()
        .filter(|item| item.kind.as_deref() == Some("season"))
        .filter(|item| item.rating_key.is_some())
        .collect()
}

/// Episodes in the order the server listed them.
///
/// A page is only ever given one season's `children`, so no `parentIndex` filter
/// is applied here.
#[must_use]
pub fn episodes(children: &[Item]) -> Vec<&Item> {
    children
        .iter()
        .filter(|item| item.kind.as_deref() == Some("episode"))
        .filter(|item| item.rating_key.is_some())
        .collect()
}

/// The season a show page opens with: the first one with an episode left to
/// watch, else the first season the server listed.
#[must_use]
pub fn default_season(children: &[Item]) -> Option<&Item> {
    seasons(children)
        .into_iter()
        .find(|season| unwatched_leaves(season).is_some_and(|left| left > 0))
        .or_else(|| seasons(children).into_iter().next())
}

/// The season whose episodes the page lists, for the `Season N` line above
/// them.
#[must_use]
pub fn selected_season(page: &Page) -> Option<&Item> {
    let key = page.season_key.as_deref()?;
    if page.item.rating_key.as_deref() == Some(key) {
        return Some(&page.item);
    }
    seasons(&page.seasons)
        .into_iter()
        .find(|season| season.rating_key.as_deref() == Some(key))
}

/// `leafCount` less `viewedLeafCount`, when the answer names both.
#[must_use]
pub fn unwatched_leaves(item: &Item) -> Option<u64> {
    let leaves = item.leaf_count?;
    let viewed = item.viewed_leaf_count?;
    Some(leaves.saturating_sub(viewed))
}

/// The episode to play next: one the server has a resume point for and has not
/// counted as watched, else the first it counts as unwatched. A `viewOffset`
/// alone does not mark an episode as the one in progress: the captured answers
/// hold no leaf with both `viewOffset` and `viewCount`, so a fully watched
/// episode still carrying a stale offset must not win over an unwatched one.
#[must_use]
pub fn next_episode(children: &[Item]) -> Option<&Item> {
    let list = episodes(children);
    list.iter()
        .copied()
        .find(|item| home::resume_start(item).is_some() && !watched(item))
        .or_else(|| list.iter().copied().find(|item| !watched(item)))
}

/// The server's own watch count, read as watched-or-not and nothing more.
#[must_use]
pub fn watched(item: &Item) -> bool {
    item.view_count.is_some_and(|count| count > 0)
}

/// `E{index}`, or an empty string when the answer named no `index`.
#[must_use]
pub fn episode_label(item: &Item) -> String {
    item.index
        .map(|index| format!("E{index}"))
        .unwrap_or_default()
}

/// One episode row's heading: `E{index} · title`, or the title alone when the
/// answer named no `index`.
#[must_use]
pub fn row_caption(item: &Item) -> String {
    let label = episode_label(item);
    let title = item.title.as_deref().unwrap_or("(untitled)");
    match label.is_empty() {
        true => title.to_owned(),
        false => format!("{label} · {title}"),
    }
}

/// The runtime as `1h 41m`, or `41m` under an hour. `None` when the answer
/// named no `duration` (a show's `duration` is one episode's length, which is
/// not this page's runtime, so callers pass a leaf's own duration).
#[must_use]
pub fn runtime(duration_ms: Option<u64>) -> Option<String> {
    let ms = duration_ms?;
    let minutes = ms / 60_000;
    if minutes == 0 {
        return None;
    }
    if minutes < 60 {
        return Some(format!("{minutes}m"));
    }
    Some(format!("{}h {:02}m", minutes / 60, minutes % 60))
}

/// The version badges the page shows, in the order it shows them: picture,
/// then sound.
///
/// Only the first `Media` version is read: that is the version the play path
/// addresses ([`plex_core::library::Item::parts`] is that version's parts
/// flattened, and the player asks for part 0 of it), so a badge never describes
/// a version the user could not open. An answer with no version shows no
/// badges.
///
/// The picture badge comes from the video stream's own `displayTitle`
/// (`"4K HDR10"`, `"1080p"`); only when it names none does the page read the
/// version's `videoResolution` (`"4k"`, `"1080"`).
#[must_use]
pub fn badges(item: &Item) -> Vec<String> {
    item.media.first().map(version_badges).unwrap_or_default()
}

/// One version's badges, picture then sound, as [`badges`] reads them.
#[must_use]
pub fn version_badges(media: &Media) -> Vec<String> {
    let mut badges: Vec<String> = Vec::new();
    let mut push = |badge: &str| {
        let badge = badge.to_owned();
        if !badge.is_empty() && !badges.contains(&badge) {
            badges.push(badge);
        }
    };
    let video = media
        .parts
        .iter()
        .flat_map(|part| &part.streams)
        .find(|stream| stream.stream_type == Some(1) && (stream.selected || stream.default));
    match video.and_then(|stream| stream.display_title.as_deref()) {
        Some(title) => {
            // Plex writes a dual-layer version as `4K DoVi/HDR10`.
            for word in title.split(|c: char| c.is_whitespace() || c == '/') {
                // A word the page has no name for (a codec's own `displayTitle`
                // tail) is left out rather than shown verbatim.
                if let Some(badge) = picture_badge(word) {
                    push(badge);
                }
            }
        }
        None => {
            if let Some(badge) = resolution_badge(media.video_resolution.as_deref()) {
                push(badge);
            }
            // A 10-bit HEVC `videoProfile` is the only HDR hint a version can
            // give when it names no video stream `displayTitle`.
            if media.video_codec.as_deref() == Some("hevc")
                && media.video_profile.as_deref() == Some("main 10")
            {
                push("HDR");
            }
        }
    }
    if let Some(badge) = video_codec_badge(media.video_codec.as_deref()) {
        push(badge);
    }
    if let Some(badge) = audio_badge(media) {
        push(&badge);
    }
    badges
}

/// One word of a video stream's `displayTitle` as a badge, or `None` when the
/// word names nothing the page has a name for.
fn picture_badge(word: &str) -> Option<&'static str> {
    match word.to_ascii_lowercase().as_str() {
        "4k" | "uhd" => Some("4K"),
        "1080p" | "1080" => Some("1080p"),
        "720p" | "720" => Some("720p"),
        "sd" | "480p" | "576p" => Some("SD"),
        "hdr10+" => Some("HDR10+"),
        "hdr10" | "hdr12" | "hdr" => Some("HDR"),
        "dolby" | "vision" | "dovi" => Some("Dolby Vision"),
        "sdr" => Some("SDR"),
        _ => None,
    }
}

/// `videoResolution` as the page names it: `"4k"` becomes `4K`, `1080` becomes
/// `1080p`. A value already carrying a `p` is used as the answer names it.
fn resolution_badge(value: Option<&str>) -> Option<&'static str> {
    match value?.to_ascii_lowercase().as_str() {
        "4k" => Some("4K"),
        "1080" => Some("1080p"),
        "720" => Some("720p"),
        "sd" | "576" | "480" => Some("SD"),
        _ => None,
    }
}

/// `videoCodec` as the tag the page shows.
fn video_codec_badge(value: Option<&str>) -> Option<&'static str> {
    match value?.to_ascii_lowercase().as_str() {
        "h264" => Some("H.264"),
        "hevc" | "h265" => Some("HEVC"),
        "av1" => Some("AV1"),
        "vp9" => Some("VP9"),
        "mpeg2video" => Some("MPEG-2"),
        "vc1" => Some("VC-1"),
        _ => None,
    }
}

/// The sound badge one version names: `audioCodec` and `audioChannels`, with
/// the atmos flag a stream's `displayTitle` carries kept in the badge.
fn audio_badge(media: &Media) -> Option<String> {
    let codec = media.audio_codec.as_deref()?;
    let channels = media
        .audio_channels
        .and_then(channel_label)
        .unwrap_or("2.0");
    let atmos = media
        .parts
        .iter()
        .flat_map(|part| &part.streams)
        .any(|stream| {
            stream.stream_type == Some(2)
                && stream
                    .display_title
                    .as_deref()
                    .is_some_and(|title| title.to_ascii_lowercase().contains("atmos"))
        });
    let name = codec_name(codec);
    Some(if atmos {
        format!("{name} · Atmos")
    } else {
        format!("{name} · {channels}")
    })
}

/// `audioChannels` as the channel count the page shows: 6 is 5.1, 8 is 7.1.
fn channel_label(channels: u64) -> Option<&'static str> {
    match channels {
        1 => Some("1.0"),
        2 => Some("2.0"),
        3 => Some("2.1"),
        6 => Some("5.1"),
        8 => Some("7.1"),
        _ => None,
    }
}

/// The name the page shows for the codec ids the captured answers carry. A codec
/// id with no name here is shown as the server named it.
fn codec_name(codec: &str) -> String {
    match codec.to_ascii_lowercase().as_str() {
        "ac3" => "Dolby Digital".to_owned(),
        "eac3" => "Dolby Digital Plus".to_owned(),
        "ac4" => "Dolby AC-4".to_owned(),
        "dca" => "DTS".to_owned(),
        "dca-ma" => "DTS-HD MA".to_owned(),
        "dts-hd" => "DTS-HD".to_owned(),
        "opus" => "Opus".to_owned(),
        "vorbis" => "Vorbis".to_owned(),
        "flac" => "FLAC".to_owned(),
        "aac" => "AAC".to_owned(),
        "mp3" => "MP3".to_owned(),
        "truehd" => "Dolby TrueHD".to_owned(),
        other => other.to_owned(),
    }
}

/// One credited person as the cast row shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct Cast {
    pub name: String,
    pub role: Option<String>,
    pub thumb: Option<String>,
}

/// `Role[]` in the order the answer listed them.
#[must_use]
pub fn cast(item: &Item) -> Vec<Cast> {
    item.roles
        .iter()
        .filter_map(|role| {
            let name = role.tag.as_deref()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(Cast {
                name: name.to_owned(),
                role: role
                    .role
                    .as_deref()
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .map(str::to_owned),
                thumb: role.thumb.clone(),
            })
        })
        .collect()
}

/// The backdrop art: the answer's own `Image` of type `background`, else its
/// `art`. `None` draws the page with no backdrop band rather than a stretched
/// poster.
#[must_use]
pub fn backdrop(item: &Item) -> Option<String> {
    // An episode's `art` is its show's; its own still is the `thumb`.
    if item.kind.as_deref() == Some("episode") {
        if let Some(still) = item.thumb.as_deref() {
            return Some(still.to_owned());
        }
    }
    item.image_url("background")
        .or(item.art.as_deref())
        .map(str::to_owned)
}

/// The line an episode's page heads with under its show's logo or title:
/// `S1 E7 · Contents Under Pressure`. Nothing for other kinds.
#[must_use]
pub fn episode_heading(item: &Item) -> Option<String> {
    (item.kind.as_deref() == Some("episode")).then(|| home::episode_caption(item))
}

/// A page above this one: the show an episode or a season belongs to, or an
/// episode's season.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ancestor {
    pub rating_key: String,
    pub title: String,
}

/// The pages above this one, outermost first: an episode's show and season, a
/// season's show. Only the ones the answer gave a key for, so every entry can
/// be opened.
#[must_use]
pub fn ancestors(item: &Item) -> Vec<Ancestor> {
    let step = |key: &Option<String>, title: Option<String>| {
        key.clone()
            .zip(title)
            .map(|(rating_key, title)| Ancestor { rating_key, title })
    };
    match item.kind.as_deref() {
        Some("episode") => {
            let season = item
                .parent_title
                .clone()
                .or_else(|| item.parent_index.map(|n| format!("Season {n}")));
            let show = step(&item.grandparent_rating_key, item.grandparent_title.clone());
            show.into_iter()
                .chain(step(&item.parent_rating_key, season))
                .collect()
        }
        Some("season") => step(&item.parent_rating_key, item.parent_title.clone())
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// The title logo: the answer's own `Image` of type `clearLogo`.
#[must_use]
pub fn logo(item: &Item) -> Option<String> {
    item.image_url("clearLogo").map(str::to_owned)
}

/// The title as the page names it, with the show an episode belongs to.
#[must_use]
pub fn play_title(item: &Item) -> String {
    match item.kind.as_deref() {
        Some("episode") => {
            let caption = home::episode_caption(item);
            match item
                .grandparent_title
                .as_deref()
                .or(item.parent_title.as_deref())
            {
                Some(show) => format!("{show} - {caption}"),
                None => caption,
            }
        }
        _ => item
            .title
            .clone()
            .unwrap_or_else(|| "(untitled)".to_owned()),
    }
}

/// What the white pill plays: a leaf plays itself, a show or season plays the
/// episode the server says is next. `None` when the page has nothing to play.
#[must_use]
pub fn play_target(page: &Page) -> Option<Target> {
    let item = play_item(page)?;
    let rating_key = item.rating_key.clone()?;
    Some(Target {
        rating_key,
        title: play_title(item),
        start: home::resume_start(item),
    })
}

/// The item the page's pill puts in the player, which is the page's own item
/// for a leaf and the next episode for a show or season.
#[must_use]
pub fn play_item(page: &Page) -> Option<&Item> {
    if leaf(page.item.kind.as_deref()) {
        return Some(&page.item);
    }
    if page.item.kind.as_deref() == Some("show") {
        if let Some(item) = page.next_up.as_ref() {
            return Some(item);
        }
    }
    next_episode(&page.episodes)
}

/// The pill's own text: `Play`, `Resume`, or `S2 E4` for the next episode
/// of a show that has no resume point, with `N min left` when the server named
/// a resume point partway in.
#[must_use]
pub fn pill_text(page: &Page) -> Option<String> {
    let item = play_item(page)?;
    let minutes = home::minutes_left(item.view_offset, item.duration);
    let base = match (item.kind.as_deref(), minutes) {
        // A show or season page names the episode its pill starts; an
        // episode's own page already heads with it.
        (Some("episode"), None) if !leaf(page.item.kind.as_deref()) => home::episode_caption(item),
        _ => home::resume_label(home::resume_start(item)).to_owned(),
    };
    match minutes {
        Some(left) => Some(format!("{base} · {left} min left")),
        None => Some(base),
    }
}

/// One episode row's own play / resume label.
#[must_use]
pub fn episode_pill_text(item: &Item) -> String {
    let minutes = home::minutes_left(item.view_offset, item.duration);
    let base = home::resume_label(home::resume_start(item)).to_owned();
    match minutes {
        Some(left) => format!("{base} · {left} min left"),
        None => base,
    }
}

/// The season chip's text: its own title, or `Season 2` from `index` when the
/// answer named no title (`(season)` when it named neither). Progress is
/// [`season_progress`], shown apart from the chip.
#[must_use]
pub fn season_label(season: &Item) -> String {
    let name = season
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| season.index.map(|index| format!("Season {index}")))
        .unwrap_or_else(|| "(season)".to_owned());
    name
}

/// The IMDb name search for a cast or crew member: the only thing it carries
/// is the person's name, percent-encoded.
#[must_use]
pub fn imdb_search(name: &str) -> String {
    let mut query = String::with_capacity(name.len() * 3);
    for byte in name.trim().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                query.push(char::from(byte));
            }
            b' ' => query.push('+'),
            other => query.push_str(&format!("%{other:02X}")),
        }
    }
    format!("https://www.imdb.com/find/?q={query}&s=nm")
}

/// How far through a season the server counts the user: `5 episodes` before
/// any is watched, `2 of 5 watched` part way, `Watched` once all are.
#[must_use]
pub fn season_progress(season: &Item) -> Option<String> {
    let leaves = season.leaf_count?;
    let viewed = season.viewed_leaf_count.unwrap_or(0);
    Some(match viewed {
        0 if leaves == 1 => "1 episode".to_owned(),
        0 => format!("{leaves} episodes"),
        v if v >= leaves => "Watched".to_owned(),
        v => format!("{v} of {leaves} watched"),
    })
}

/// The runtime text one episode row shows on its meta line.
#[must_use]
pub fn episode_runtime(item: &Item) -> Option<String> {
    runtime(item.duration)
}

/// The meta line of an episode row: its runtime and, when the answer named no
/// `duration`, nothing.
#[must_use]
pub fn episode_meta(item: &Item) -> String {
    episode_runtime(item).unwrap_or_default()
}
