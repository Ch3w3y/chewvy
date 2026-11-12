//! Playback quality: the file as-is, a remux, or a server transcode.
//!
//! Direct Play streams the part file itself. Direct Stream and the transcode
//! presets ask the server's universal transcoder for HLS. The URL carries the
//! session and client identity but never the token, which mpv sends as a header.

/// How the server should deliver the item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The original file, untouched.
    DirectPlay,
    /// Original video and audio, repackaged by the server.
    DirectStream,
    /// Re-encoded to fit `resolution` and `kbps`.
    Transcode {
        /// Largest frame, as `WIDTHxHEIGHT`.
        resolution: &'static str,
        /// Video bitrate ceiling in kbit/s.
        kbps: u32,
    },
}

/// One choice in the quality menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quality {
    /// What the menu shows.
    pub label: &'static str,
    /// What the server is asked for.
    pub mode: Mode,
}

const fn transcode(label: &'static str, resolution: &'static str, kbps: u32) -> Quality {
    Quality {
        label,
        mode: Mode::Transcode { resolution, kbps },
    }
}

/// Direct Play: the default, and the fallback for an unknown index.
const ORIGINAL: Quality = Quality {
    label: "Original · Direct Play",
    mode: Mode::DirectPlay,
};

/// The menu, best first; index 0 is the default.
pub const QUALITIES: [Quality; 9] = [
    ORIGINAL,
    Quality {
        label: "Original · Direct Stream",
        mode: Mode::DirectStream,
    },
    transcode("1080p · 20 Mbps", "1920x1080", 20_000),
    transcode("1080p · 12 Mbps", "1920x1080", 12_000),
    transcode("1080p · 8 Mbps", "1920x1080", 8_000),
    transcode("720p · 4 Mbps", "1280x720", 4_000),
    transcode("720p · 2 Mbps", "1280x720", 2_000),
    transcode("480p · 1.5 Mbps", "854x480", 1_500),
    transcode("240p · 320 kbps", "426x240", 320),
];

/// Where [`QUALITIES`] keeps the server remux and its best transcode.
pub const REMUX: usize = 1;
const TRANSCODE: usize = 2;

/// What to try when the quality at `index` could not be played, with the line
/// shown while it opens: the server remux after Direct Play (a new container
/// around the same streams), the best transcode after the remux (new streams),
/// and nothing after a transcode.
#[must_use]
pub fn fallback(index: usize) -> Option<(usize, &'static str)> {
    match quality(index).mode {
        Mode::DirectPlay => Some((REMUX, "Direct play failed · trying a server remux…")),
        Mode::DirectStream => Some((
            TRANSCODE,
            "The server remux failed · trying a server transcode…",
        )),
        Mode::Transcode { .. } => None,
    }
}

/// The quality at `index`, falling back to Direct Play.
#[must_use]
pub fn quality(index: usize) -> Quality {
    QUALITIES.get(index).copied().unwrap_or(ORIGINAL)
}

/// The platform the server picks its base profile by: one it knows to stream HLS.
const PROFILE_PLATFORM: &str = "Chrome";
/// Lets the server copy HEVC and Dolby audio instead of re-encoding them.
const PROFILE_EXTRA: &str = "add-transcode-target(type=videoProfile&context=streaming&protocol=hls&container=mpegts&videoCodec=h264,hevc&audioCodec=aac,ac3,eac3)+add-transcode-target-audio-codec(type=videoProfile&context=streaming&protocol=hls&audioCodec=ac3,eac3)";

/// Who is asking, as the server's session bookkeeping needs it.
pub struct StreamAsk<'a> {
    /// The connected server's base URL.
    pub base: &'a str,
    /// The item to stream.
    pub rating_key: &'a str,
    /// This stream's session id.
    pub session: &'a str,
    /// `X-Plex-Client-Identifier`.
    pub client_id: &'a str,
    /// `X-Plex-Product`.
    pub product: &'a str,
    /// Seconds into the item to start the stream at.
    pub offset: Option<f64>,
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The HLS URL for `mode`, or `None` for Direct Play (which uses the part URL).
#[must_use]
pub fn stream_url(ask: &StreamAsk<'_>, mode: Mode) -> Option<String> {
    let (direct_stream, limits) = match mode {
        Mode::DirectPlay => return None,
        Mode::DirectStream => ("1", Vec::new()),
        Mode::Transcode { resolution, kbps } => (
            "0",
            vec![
                ("videoResolution", resolution.to_owned()),
                ("maxVideoBitrate", kbps.to_string()),
            ],
        ),
    };
    let offset = ask.offset.unwrap_or(0.0).max(0.0);
    let mut query: Vec<(&str, String)> = vec![
        ("path", format!("/library/metadata/{}", ask.rating_key)),
        ("mediaIndex", "0".into()),
        ("partIndex", "0".into()),
        ("protocol", "hls".into()),
        ("fastSeek", "1".into()),
        ("copyts", "1".into()),
        ("directPlay", "0".into()),
        ("directStream", direct_stream.into()),
        ("videoQuality", "100".into()),
        ("subtitles", "none".into()),
        ("offset", format!("{offset:.0}")),
        ("session", ask.session.into()),
        ("X-Plex-Client-Identifier", ask.client_id.into()),
        ("X-Plex-Product", ask.product.into()),
        ("X-Plex-Platform", PROFILE_PLATFORM.into()),
        ("X-Plex-Client-Profile-Extra", PROFILE_EXTRA.into()),
    ];
    query.extend(limits);
    let query: Vec<String> = query
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect();
    Some(format!(
        "{}/video/:/transcode/universal/start.m3u8?{}",
        ask.base,
        query.join("&")
    ))
}

#[cfg(test)]
mod tests {
    use super::{fallback, quality, stream_url, Mode, StreamAsk, ORIGINAL, QUALITIES};

    fn ask() -> StreamAsk<'static> {
        StreamAsk {
            base: "https://tower.example:32400",
            rating_key: "123",
            session: "s-1",
            client_id: "c-1",
            product: "Chewvy",
            offset: Some(61.4),
        }
    }

    #[test]
    fn direct_play_has_no_stream_url() {
        assert!(stream_url(&ask(), Mode::DirectPlay).is_none());
    }

    #[test]
    fn a_transcode_url_names_the_item_limits_and_offset_but_no_token() {
        let url = stream_url(&ask(), quality(5).mode).unwrap_or_default();
        assert!(
            url.starts_with("https://tower.example:32400/video/:/transcode/universal/start.m3u8?")
        );
        assert!(url.contains("path=%2Flibrary%2Fmetadata%2F123"));
        assert!(url.contains("maxVideoBitrate=4000"));
        assert!(url.contains("videoResolution=1280x720"));
        assert!(url.contains("directStream=0"));
        assert!(url.contains("offset=61"));
        assert!(!url.to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn the_url_leaves_out_the_session_identifier_tower_rejects() {
        // Tower answers 400 to start.m3u8 when X-Plex-Session-Identifier is set.
        let url = stream_url(&ask(), Mode::DirectStream).unwrap_or_default();
        assert!(!url.contains("Session-Identifier"));
        assert!(url.contains("session=s-1"));
    }

    #[test]
    fn direct_stream_keeps_the_original_streams() {
        let url = stream_url(&ask(), Mode::DirectStream).unwrap_or_default();
        assert!(url.contains("directStream=1"));
        assert!(!url.contains("maxVideoBitrate"));
    }

    #[test]
    fn a_failed_play_steps_to_the_remux_then_the_best_transcode_then_stops() {
        let remux = fallback(0).map(|(index, _)| index);
        assert_eq!(remux.map(|i| quality(i).mode), Some(Mode::DirectStream));
        let transcode = remux.and_then(fallback).map(|(index, _)| index);
        assert_eq!(
            transcode.map(|i| quality(i).mode),
            Some(Mode::Transcode {
                resolution: "1920x1080",
                kbps: 20_000
            })
        );
        // No preset is retried at another preset: every one ends the ladder.
        for index in 2..QUALITIES.len() {
            assert!(fallback(index).is_none());
        }
    }

    #[test]
    fn an_unknown_index_falls_back_to_direct_play() {
        assert_eq!(quality(99), ORIGINAL);
    }
}
