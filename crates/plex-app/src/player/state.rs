//! What the controls show: polled mpv state plus the server's facts about the item.
//!
//! Parsing takes a property getter instead of a live mpv handle, so every rule
//! here is unit-tested against plain strings.

use plex_player::mpv_link::FfiPlayer;

/// Which skippable range a marker covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    /// Opening titles.
    Intro,
    /// End credits.
    Credits,
}

/// One skippable range, in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerSpan {
    /// Intro or credits.
    pub kind: MarkerKind,
    /// Range start.
    pub start: f64,
    /// Range end (where "Skip" jumps to).
    pub end: f64,
}

/// One chapter start the server reported, with its preview image path.
#[derive(Clone, Debug, PartialEq)]
pub struct ChapterMark {
    /// Chapter start in seconds.
    pub start: f64,
    /// Server path of the chapter thumbnail (for the photo transcoder).
    pub thumb: Option<String>,
}

/// The episode that follows the playing one.
#[derive(Clone, Debug, PartialEq)]
pub struct NextUp {
    /// Its server key.
    pub rating_key: String,
    /// "S4 E3 · Title".
    pub label: String,
    /// Episode still path (for the photo transcoder).
    pub thumb: Option<String>,
}

/// A sidecar subtitle file the server keeps next to the media.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubFile {
    /// Full URL; the token travels in mpv's request headers, never here.
    pub url: String,
    /// Language tag as the server records it.
    pub lang: Option<String>,
    /// The stream's title.
    pub title: Option<String>,
    /// Forced (signs and foreign dialogue) only.
    pub forced: bool,
}

/// The server's description of the file, shown in the info sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaFacts {
    /// Container (`mkv`, `mp4`, …).
    pub container: Option<String>,
    /// Video codec as the server names it.
    pub video_codec: Option<String>,
    /// Codec profile (`main 10`, `high`, …).
    pub video_profile: Option<String>,
    /// `4k`, `1080`, …
    pub resolution: Option<String>,
    /// Overall bitrate in kbps.
    pub bitrate_kbps: Option<u64>,
    /// Audio codec as the server names it.
    pub audio_codec: Option<String>,
    /// Audio channel count.
    pub audio_channels: Option<u64>,
    /// File size in bytes.
    pub size_bytes: Option<u64>,
}

/// Audio or subtitle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    /// An audio stream.
    Audio,
    /// A subtitle stream.
    Subtitle,
}

/// One selectable stream, as mpv lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// mpv's own id (what `aid`/`sid` take).
    pub id: i64,
    /// Audio or subtitle.
    pub kind: TrackKind,
    /// The menu label: language, title, codec and channels.
    pub label: String,
    /// Currently selected.
    pub selected: bool,
    /// The language code the file names, if any.
    pub lang: Option<String>,
    /// A forced (signs and foreign-dialogue only) subtitle.
    pub forced: bool,
}

/// Everything the controls draw from, refreshed every UI frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Playback position in seconds.
    pub position: Option<f64>,
    /// Item length in seconds.
    pub duration: Option<f64>,
    /// Paused by the user.
    pub paused: bool,
    /// Waiting for the network cache.
    pub buffering: bool,
    /// Not advancing although it should: waiting for the cache, or a seek or
    /// the first frame not yet shown.
    pub stalled: bool,
    /// Seconds of media buffered ahead of the position.
    pub cached_ahead: Option<f64>,
    /// Played to the end (`keep-open` holds the last frame).
    pub eof: bool,
    /// Volume in percent.
    pub volume: f64,
    /// Muted.
    pub muted: bool,
    /// Playback speed.
    pub speed: f64,
    /// Subtitle delay in seconds.
    pub sub_delay: f64,
    /// Audio delay in seconds.
    pub audio_delay: f64,
    /// Subtitle scale factor.
    pub sub_scale: f64,
    /// Video zoom (log2).
    pub zoom: f64,
    /// Audio and subtitle tracks.
    pub tracks: Vec<Track>,
    /// Chapter titles and starts mpv read from the file.
    pub chapters: Vec<(f64, String)>,
    /// Facts about the decoded video, for the info sheet.
    pub video: VideoInfo,
    /// Sidecar files loaded with the item, to label their tracks.
    pub sidecars: Vec<SubFile>,
    /// A stream of the file ("video" or "audio") mpv is playing on without.
    pub undecoded: Option<&'static str>,
}

/// What mpv reports about the video it is decoding.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoInfo {
    /// Codec description.
    pub codec: Option<String>,
    /// Decoded size.
    pub size: Option<(u32, u32)>,
    /// Frame rate.
    pub fps: Option<f64>,
    /// Hardware decoder in use (`no` when software).
    pub hwdec: Option<String>,
    /// Transfer function (`pq` and `hlg` mean HDR).
    pub gamma: Option<String>,
    /// Colour primaries (`bt.2020` for wide gamut).
    pub primaries: Option<String>,
    /// Colour matrix; `dolbyvision` when mpv decoded Dolby Vision metadata.
    pub matrix: Option<String>,
    /// Pixel format (`yuv420p10` etc.).
    pub pixel_format: Option<String>,
    /// Audio codec description.
    pub audio_codec: Option<String>,
    /// Audio channel layout.
    pub audio_channels: Option<String>,
    /// Current video bitrate in bits per second.
    pub video_bitrate: Option<f64>,
    /// Frames dropped by the output so far.
    pub dropped: Option<u64>,
}

impl VideoInfo {
    /// "HDR10 / PQ", "HLG" or `None` for SDR.
    #[must_use]
    pub fn hdr_label(&self) -> Option<&'static str> {
        match self.gamma.as_deref() {
            Some("pq") => Some("HDR (PQ)"),
            Some("hlg") => Some("HDR (HLG)"),
            _ => None,
        }
    }
}

impl Snapshot {
    /// Re-read mpv's properties; `slow` also refreshes tracks, chapters and info.
    pub fn refresh(&mut self, mpv: &FfiPlayer, slow: bool) {
        self.refresh_from(&|name| mpv.get_str(name), slow);
    }

    /// [`Snapshot::refresh`] over any property getter (unit-testable).
    pub fn refresh_from(&mut self, get: &dyn Fn(&str) -> Option<String>, slow: bool) {
        let num = |name: &str| get(name).and_then(|v| v.trim().parse::<f64>().ok());
        let flag = |name: &str| get(name).as_deref() == Some("yes");
        self.position = num("time-pos");
        self.duration = num("duration");
        self.paused = flag("pause");
        self.buffering = flag("paused-for-cache");
        // `seeking` stays on until a seek's (or the file's) first frame is
        // shown, paused or not; `core-idle` cannot help, as it is on whenever
        // the player is paused.
        self.stalled = self.buffering || flag("seeking");
        self.cached_ahead = num("demuxer-cache-duration");
        self.eof = flag("eof-reached");
        self.volume = num("volume").unwrap_or(100.0);
        self.muted = flag("mute");
        self.speed = num("speed").unwrap_or(1.0);
        if !slow {
            return;
        }
        self.sub_delay = num("sub-delay").unwrap_or(0.0);
        self.audio_delay = num("audio-delay").unwrap_or(0.0);
        self.sub_scale = num("sub-scale").unwrap_or(1.0);
        self.zoom = num("video-zoom").unwrap_or(0.0);
        self.tracks = read_tracks(get, &self.sidecars);
        self.undecoded = dropped_stream(get);
        self.chapters = read_chapters(get);
        self.video = VideoInfo {
            codec: get("video-format").or_else(|| get("video-codec")),
            size: num("video-params/w")
                .zip(num("video-params/h"))
                .map(|(w, h)| (w as u32, h as u32)),
            fps: num("container-fps").or_else(|| num("estimated-vf-fps")),
            hwdec: get("hwdec-current"),
            gamma: get("video-params/gamma"),
            primaries: get("video-params/primaries"),
            matrix: get("video-params/colormatrix"),
            pixel_format: get("video-params/hw-pixelformat")
                .or_else(|| get("video-params/pixelformat")),
            audio_codec: get("audio-codec-name"),
            audio_channels: get("audio-params/channels"),
            video_bitrate: num("video-bitrate"),
            dropped: num("frame-drop-count").map(|n| n as u64),
        };
    }

    /// Audio tracks, in mpv's order.
    pub fn audio_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.kind == TrackKind::Audio)
    }

    /// Subtitle tracks, in mpv's order.
    pub fn subtitle_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.kind == TrackKind::Subtitle)
    }

    /// Fraction of the item played, 0..=1.
    #[must_use]
    pub fn progress(&self) -> f32 {
        match (self.position, self.duration) {
            (Some(p), Some(d)) if d > 0.0 => (p / d).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        }
    }
}

/// The stream mpv lists for the file but has deselected. mpv does that to a
/// track whose decoder would not start, then plays the rest without a word:
/// sound with no picture, or the reverse. Nothing in this app deselects video
/// or audio, so "none selected, one listed" means a decoder failed.
fn dropped_stream(get: &dyn Fn(&str) -> Option<String>) -> Option<&'static str> {
    let count = get("track-list/count")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let lists = |kind: &str| {
        (0..count).any(|i| {
            get(&format!("track-list/{i}/type")).as_deref() == Some(kind)
                // Cover art is a picture, not a stream that should be playing.
                && get(&format!("track-list/{i}/image")).as_deref() != Some("yes")
        })
    };
    let off = |prop: &str| get(prop).as_deref() == Some("no");
    if off("vid") && lists("video") {
        Some("video")
    } else if off("aid") && lists("audio") {
        Some("audio")
    } else {
        None
    }
}

fn read_tracks(get: &dyn Fn(&str) -> Option<String>, sidecars: &[SubFile]) -> Vec<Track> {
    let count = get("track-list/count")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    // mpv's per-track `selected` flag can lag a selection made while the file
    // loads, so the active ids come from `aid`/`sid` when mpv reports them.
    let active = |prop: &str| get(prop).and_then(|v| v.parse::<i64>().ok());
    let (aid, sid) = (
        get("aid").map(|_| active("aid")),
        get("sid").map(|_| active("sid")),
    );
    (0..count)
        .filter_map(|i| {
            let field = |f: &str| get(&format!("track-list/{i}/{f}"));
            let kind = match field("type").as_deref() {
                Some("audio") => TrackKind::Audio,
                Some("sub") => TrackKind::Subtitle,
                _ => return None,
            };
            let id = field("id")?.parse::<i64>().ok()?;
            // A sidecar's file name says nothing, so take the server's labels.
            let sidecar =
                field("external-filename").and_then(|name| sidecars.iter().find(|s| s.url == name));
            let lang = sidecar.map_or_else(|| field("lang"), |s| s.lang.clone());
            let forced =
                sidecar.map_or_else(|| field("forced").as_deref() == Some("yes"), |s| s.forced);
            let title = sidecar.map_or_else(|| field("title"), |s| s.title.clone());
            let label = track_label(
                kind,
                lang.as_deref(),
                title.as_deref(),
                field("codec").as_deref(),
                field("demux-channel-count").as_deref(),
                forced,
                field("external").as_deref() == Some("yes"),
            );
            Some(Track {
                id,
                kind,
                label,
                selected: match (kind, aid, sid) {
                    (TrackKind::Audio, Some(active), _)
                    | (TrackKind::Subtitle, _, Some(active)) => active == Some(id),
                    _ => field("selected").as_deref() == Some("yes"),
                },
                lang,
                forced,
            })
        })
        .collect()
}

/// "English · Commentary · EAC3 5.1" style menu label.
#[must_use]
pub fn track_label(
    kind: TrackKind,
    lang: Option<&str>,
    title: Option<&str>,
    codec: Option<&str>,
    channels: Option<&str>,
    forced: bool,
    external: bool,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let language = lang.map_or_else(|| "Unknown".to_owned(), language_name);
    let repeats = |t: &str| title_repeats_language(t, &language);
    parts.push(language.clone());
    if let Some(title) = title.filter(|t| !t.trim().is_empty() && !repeats(t)) {
        parts.push(title.trim().to_owned());
    }
    let titled_forced = title.is_some_and(|t| t.to_ascii_lowercase().contains("forced"));
    if forced && !titled_forced {
        parts.push("Forced".to_owned());
    }
    if external {
        parts.push("External".to_owned());
    }
    let codec = codec.map(codec_name);
    match (kind, codec, channels.and_then(|c| c.parse::<u32>().ok())) {
        (TrackKind::Audio, Some(c), Some(ch)) => parts.push(format!("{c} {}", channel_layout(ch))),
        (_, Some(c), _) => parts.push(c),
        _ => {}
    }
    parts.join(" · ")
}

fn channel_layout(channels: u32) -> String {
    match channels {
        1 => "Mono".to_owned(),
        2 => "Stereo".to_owned(),
        6 => "5.1".to_owned(),
        8 => "7.1".to_owned(),
        n => format!("{n}ch"),
    }
}

/// A codec's display name ("eac3" is "EAC3", "subrip" is "SRT").
#[must_use]
pub fn codec_name(codec: &str) -> String {
    match codec {
        "truehd" => "TrueHD".to_owned(),
        "eac3" => "EAC3".to_owned(),
        "ac3" => "AC3".to_owned(),
        "dts" => "DTS".to_owned(),
        "aac" => "AAC".to_owned(),
        "flac" => "FLAC".to_owned(),
        "opus" => "Opus".to_owned(),
        "vorbis" => "Vorbis".to_owned(),
        "subrip" => "SRT".to_owned(),
        "ass" => "ASS".to_owned(),
        "hdmv_pgs_subtitle" => "PGS".to_owned(),
        "dvd_subtitle" => "VobSub".to_owned(),
        "mov_text" => "Text".to_owned(),
        other => other.to_uppercase(),
    }
}

/// Whether a track title only restates its language ("Latin American" on
/// "Spanish (Latin America)"), so the label can leave it out.
fn title_repeats_language(title: &str, language: &str) -> bool {
    let language = language.to_lowercase();
    let mut words = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .peekable();
    words.peek().is_some()
        && words.all(|w| {
            let w = w.to_lowercase();
            language.contains(&w) || w.strip_suffix('n').is_some_and(|s| language.contains(s))
        })
}

/// Common ISO 639 codes as names; anything else is shown as given.
#[must_use]
pub fn language_name(code: &str) -> String {
    // BCP 47 tags ("ja-JP", "pt-BR", "es-419"): name the language, and the
    // region only where it changes what is spoken.
    let (primary, region) = code.split_once('-').unwrap_or((code, ""));
    let base =
        base_language(&primary.to_ascii_lowercase()).map_or_else(|| code.to_owned(), str::to_owned);
    match region.to_ascii_uppercase().as_str() {
        "BR" => format!("{base} (Brazil)"),
        "419" | "MX" | "LA" => format!("{base} (Latin America)"),
        "CA" if primary.eq_ignore_ascii_case("fr") => format!("{base} (Canada)"),
        "TW" | "HK" => format!("{base} (Traditional)"),
        _ => base,
    }
}

fn base_language(code: &str) -> Option<&'static str> {
    let name = match code {
        "en" | "eng" => "English",
        "ja" | "jpn" => "Japanese",
        "fr" | "fre" | "fra" => "French",
        "de" | "ger" | "deu" => "German",
        "es" | "spa" => "Spanish",
        "it" | "ita" => "Italian",
        "pt" | "por" => "Portuguese",
        "nl" | "dut" | "nld" => "Dutch",
        "sv" | "swe" => "Swedish",
        "no" | "nor" => "Norwegian",
        "da" | "dan" => "Danish",
        "fi" | "fin" => "Finnish",
        "pl" | "pol" => "Polish",
        "ru" | "rus" => "Russian",
        "zh" | "chi" | "zho" => "Chinese",
        "ko" | "kor" => "Korean",
        "ar" | "ara" => "Arabic",
        "hi" | "hin" => "Hindi",
        "tr" | "tur" => "Turkish",
        "fil" | "tl" | "tgl" => "Filipino",
        "id" | "ind" => "Indonesian",
        "ms" | "may" | "msa" => "Malay",
        "th" | "tha" => "Thai",
        "vi" | "vie" => "Vietnamese",
        "he" | "heb" => "Hebrew",
        "el" | "gre" | "ell" => "Greek",
        "cs" | "cze" | "ces" => "Czech",
        "sk" | "slo" | "slk" => "Slovak",
        "hu" | "hun" => "Hungarian",
        "ro" | "rum" | "ron" => "Romanian",
        "uk" | "ukr" => "Ukrainian",
        "bg" | "bul" => "Bulgarian",
        "hr" | "hrv" => "Croatian",
        "sr" | "srp" => "Serbian",
        "sl" | "slv" => "Slovenian",
        "ca" | "cat" => "Catalan",
        "fa" | "per" | "fas" => "Persian",
        "nb" | "nob" | "nn" | "nno" => "Norwegian",
        "is" | "ice" | "isl" => "Icelandic",
        "et" | "est" => "Estonian",
        "lv" | "lav" => "Latvian",
        "lt" | "lit" => "Lithuanian",
        "bn" | "ben" => "Bengali",
        "ta" | "tam" => "Tamil",
        "te" | "tel" => "Telugu",
        "ml" | "mal" => "Malayalam",
        "ur" | "urd" => "Urdu",
        "und" => "Unknown",
        _ => return None,
    };
    Some(name)
}

fn read_chapters(get: &dyn Fn(&str) -> Option<String>) -> Vec<(f64, String)> {
    let count = get("chapter-list/count")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    (0..count)
        .filter_map(|i| {
            let time = get(&format!("chapter-list/{i}/time"))?
                .parse::<f64>()
                .ok()?;
            let title = get(&format!("chapter-list/{i}/title"))
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| format!("Chapter {}", i + 1));
            Some((time, title))
        })
        .collect()
}

/// The marker whose range holds `position`, if any.
#[must_use]
pub fn marker_at(markers: &[MarkerSpan], position: f64) -> Option<&MarkerSpan> {
    markers
        .iter()
        .find(|m| position >= m.start && position < m.end - 1.0)
}

/// `h:mm:ss` or `m:ss`.
#[must_use]
pub fn clock(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests;
