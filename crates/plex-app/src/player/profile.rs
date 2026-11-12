//! The `DeviceProfile` this install sends, and the header the player sends with
//! it — the two claims a Jellyfin server is allowed to hold this client to.
//!
//! A `POST /Items/{id}/PlaybackInfo` answer is a fact about the pair (item,
//! profile): `SupportsDirectPlay` says what *the sent profile* can take. So the
//! profile is not a formality — it is this player's capability matrix in the
//! shape a server reads, and every number in it has to be something this player
//! has been seen doing.
//!
//! Every claim is a measurement of mpv on the development host (RX 9070 XT,
//! Mesa vaapi): H.264, HEVC Main10, AV1 and VP9 decode in hardware at up to
//! 3840x2160 with room to spare over 24-60 fps, inside the 4096-wide frames
//! every one of those decoders accepts. H.264 is claimed at 8-bit and the others
//! at 10-bit, the depths the decoders list. MPEG-2, MPEG-4 part 2 and VP8 are
//! decoded in software and claimed at the frame each was generated and played
//! at. Audio is decoded by libavcodec in software.
//!
//! Deliberately **not** claimed, because nothing here has measured them: frames
//! larger than 4096x2160, VC-1/WMV3, MJPEG and VVC. A file in one of those
//! shapes is answered `SupportsDirectPlay=false` for this profile, and the
//! server's own stream of it plays instead of handing the file to mpv on
//! faith. Raising a ceiling is a measurement to make, not a number to guess.
//!
//! That stream is HLS in MPEG-TS segments holding H.264 or HEVC with AAC, AC-3
//! or E-AC-3: codecs from the measured lists above, in the one stream shape
//! mpv was seen playing from a 12.4 server, copied and encoded alike.

// The lints product code keeps on are relaxed for the tests, which read a
// serialized profile the way a reader would.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "tests may unwrap; product code keeps these lints on"
    )
)]

use jellyfin_core::device::DeviceInfo;
use jellyfin_core::playback::{
    CodecProfile, DeviceProfile, DirectPlayProfile, ProfileCondition, ProfileKind, ProfileProperty,
    TranscodingProfile,
};
use jellyfin_core::{media_browser_header, SecretString};

/// The widest frame claimed: cinema 4K. UHD (3840x2160) was measured decoding in
/// hardware for all four codecs, and 4096 is the smallest maximum width any of
/// their decoders reports.
const MAX_WIDTH: u32 = 4096;
/// The tallest frame claimed: the 2160 lines the 4K measurements used.
const MAX_HEIGHT: u32 = 2160;

/// Containers and the video codecs each was measured holding, in the names a
/// server gives them. A row matches a source when its container and its codecs
/// appear together, so a codec is only claimed where a file of that pairing
/// family was played: transport streams (`.ts` and `.m2ts` are both `ts` to
/// the server), DVD-style program streams and AVI get rows of their own.
const VIDEO_ROWS: [(&str, &str); 4] = [
    ("mkv,mp4,mov,webm", "h264,hevc,av1,vp9,mpeg2video,vp8"),
    ("ts", "h264,hevc,mpeg2video"),
    ("mpeg", "mpeg2video"),
    ("avi", "mpeg4"),
];
/// The audio codecs this libavcodec was measured decoding. No ceiling is
/// attached to them: software audio decode is nowhere near the limit these
/// measurements came close to. `dts` was measured as DTS core 5.1; DTS-HD
/// rides on the same codec name and no legal generator for it exists here.
const AUDIO_CODECS: &str = "aac,mp3,flac,ac3,eac3,opus,vorbis,mp2,dts,truehd,pcm_s24le,alac";
/// Audio in the containers that hold nothing but audio. A blank `AudioCodec`
/// would claim *every* audio codec, which is wider than the matrix was, so these
/// rows name their codecs too.
const AUDIO_CONTAINERS: &str = "mp3,flac,m4a,ogg,opus";

/// Bitrate ceiling sent for static and streamed sources alike, in bits per second.
const MAX_BITRATE: u64 = 1_000_000_000;

/// The segment container of the server's stream, and what it may hold. The
/// server copies a source stream in one of these codecs and encodes to the
/// first of each list otherwise.
const STREAM_CONTAINER: &str = "ts";
const STREAM_VIDEO_CODECS: &str = "h264,hevc";
const STREAM_AUDIO_CODECS: &str = "aac,ac3,eac3";
/// 5.1: the widest layout those three audio codecs were measured in.
const STREAM_AUDIO_CHANNELS: u8 = 6;

/// The claim to send with every `PlaybackInfo` request.
#[must_use]
pub fn mpv_device_profile(device: &DeviceInfo) -> DeviceProfile {
    let audio = || Some(AUDIO_CODECS.to_owned());
    let mut direct_play_profiles: Vec<DirectPlayProfile> = VIDEO_ROWS
        .iter()
        .map(|(containers, codecs)| DirectPlayProfile {
            container: (*containers).to_owned(),
            kind: ProfileKind::Video,
            video_codec: Some((*codecs).to_owned()),
            audio_codec: audio(),
        })
        .collect();
    direct_play_profiles.push(DirectPlayProfile {
        container: AUDIO_CONTAINERS.to_owned(),
        kind: ProfileKind::Audio,
        video_codec: None,
        audio_codec: audio(),
    });
    DeviceProfile {
        // The name a server records this client's profile under, in its log and
        // its session list — which is where a wrong claim becomes traceable.
        name: Some(format!("{} (mpv)", device.app)),
        // Not a measured link speed: an upper bound this client asks for, so the
        // server does not narrow a source it has no reason to narrow. Jellyfin's
        // own `DeviceProfile` defaults to 8 Mbit/s, which would refuse the
        // 14.5 Mbit/s file of the captured fixture on bandwidth alone.
        max_static_bitrate: Some(MAX_BITRATE),
        // Without this a 12.4 server refuses direct play of every source.
        max_streaming_bitrate: Some(MAX_BITRATE),
        direct_play_profiles,
        codec_profiles: vec![
            // 8-bit because that is where `vainfo` stopped for H.264.
            ceiling("h264", (MAX_WIDTH, MAX_HEIGHT), 8),
            // Main10 is the profile the matrix measured.
            ceiling("hevc", (MAX_WIDTH, MAX_HEIGHT), 10),
            // `vainfo` measured AV1 Profile 0 and 2: 8-bit and 10-bit.
            ceiling("av1", (MAX_WIDTH, MAX_HEIGHT), 10),
            // VP9 Profile 0 and 2 again.
            ceiling("vp9", (MAX_WIDTH, MAX_HEIGHT), 10),
            // Software decode, each at the largest frame a clip was played at.
            ceiling("mpeg2video", (1920, 1080), 8),
            ceiling("mpeg4", (1280, 720), 8),
            ceiling("vp8", (1280, 720), 8),
        ],
        transcoding_profiles: vec![TranscodingProfile::hls(
            STREAM_CONTAINER,
            STREAM_VIDEO_CODECS,
            STREAM_AUDIO_CODECS,
            STREAM_AUDIO_CHANNELS,
        )],
    }
}

/// The frame and bit-depth ceiling one measured codec carries: the largest
/// frame this host was seen decoding it at, and the deepest sample measured.
fn ceiling(codec: &str, (max_width, max_height): (u32, u32), max_bit_depth: u8) -> CodecProfile {
    CodecProfile {
        kind: ProfileKind::Video,
        codec: Some(codec.to_owned()),
        container: None,
        conditions: vec![
            ProfileCondition::at_most(ProfileProperty::Width, max_width),
            ProfileCondition::at_most(ProfileProperty::Height, max_height),
            ProfileCondition::at_most(ProfileProperty::VideoBitDepth, max_bit_depth),
        ],
    }
}

/// The `Authorization: MediaBrowser …` header line mpv sends for a Jellyfin stream.
///
/// A Jellyfin server authenticates the *request*, and a static stream URL
/// carries no token of its own, so the token has to travel in this header. It is
/// a [`SecretString`] on the way out for the same reason it is one everywhere
/// else: the value outlives the call that built it, sitting in the player until
/// the next file replaces it.
#[must_use]
pub fn stream_header(device: &DeviceInfo, token: &SecretString) -> SecretString {
    // mpv takes whole `Name: value` lines; the bare value would reach the
    // server as a malformed header and the stream as unauthorised.
    SecretString::new(format!(
        "Authorization: {}",
        media_browser_header(device, Some(token))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> DeviceInfo {
        DeviceInfo {
            app: "Chewvy".to_owned(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            device: "test-machine".to_owned(),
            device_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned(),
        }
    }

    fn body() -> serde_json::Value {
        serde_json::to_value(mpv_device_profile(&device())).expect("a profile that serializes")
    }

    /// The profile's shape is the server's, not the player's: PascalCase keys,
    /// comma-separated codec lists, and a `Type` on every row — an omitted
    /// `Type` would bind as the enum's first entry and quietly mis-describe it.
    #[test]
    fn the_profile_is_the_wire_shape_with_its_codecs_and_ceilings() {
        let body = body();
        assert_eq!(body["Name"], "Chewvy (mpv)");
        assert_eq!(body["MaxStaticBitrate"], 1_000_000_000);
        assert_eq!(body["MaxStreamingBitrate"], 1_000_000_000);
        assert!(
            body.get("EnableTranscoding").is_none(),
            "which methods the client accepts is the request's business: {body}"
        );

        let rows = body["DirectPlayProfiles"].as_array().expect("rows");
        assert_eq!(rows.len(), VIDEO_ROWS.len() + 1);
        for (row, (containers, codecs)) in rows.iter().zip(VIDEO_ROWS) {
            assert_eq!(row["Type"], "Video");
            assert_eq!(row["Container"], containers);
            assert_eq!(row["VideoCodec"], codecs);
            assert_eq!(row["AudioCodec"], AUDIO_CODECS);
        }
        let audio = &rows[VIDEO_ROWS.len()];
        assert_eq!(audio["Type"], "Audio");
        assert!(
            audio.get("VideoCodec").is_none(),
            "an audio row that names video codecs claims video it does not play: {audio}"
        );

        // Every measured codec carries its frame and depth ceiling, and every
        // condition says it does not disqualify a probe that reported no such
        // property.
        let codecs: Vec<&str> = body["CodecProfiles"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|row| row["Codec"].as_str().expect("a codec per row"))
            .collect();
        assert_eq!(
            codecs,
            ["h264", "hevc", "av1", "vp9", "mpeg2video", "mpeg4", "vp8"]
        );
        for row in body["CodecProfiles"].as_array().expect("rows") {
            assert_eq!(row["Type"], "Video");
            let conditions = row["Conditions"].as_array().expect("conditions");
            assert_eq!(conditions.len(), 3, "{row}");
            for condition in conditions {
                assert_eq!(condition["Condition"], "LessThanEqual");
                assert_eq!(
                    condition["IsRequired"], false,
                    "a probe that reports no frame size is not an unplayable source"
                );
            }
        }
    }

    /// The one stream the server may make: HLS, in codecs the direct rows
    /// already claim, so a copied stream is never one the player has not met.
    #[test]
    fn the_server_stream_is_hls_in_codecs_the_direct_rows_claim() {
        let body = body();
        let rows = body["TranscodingProfiles"].as_array().expect("rows");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row["Protocol"], "hls");
        assert_eq!(row["Container"], "ts");
        assert_eq!(row["Context"], "Streaming");
        assert_eq!(row["Type"], "Video");
        assert_eq!(row["MaxAudioChannels"], "6");
        // The stream is a transport stream, so its codecs are held to the
        // row that claims transport streams as files.
        let (_, ts_video) = VIDEO_ROWS
            .iter()
            .find(|(containers, _)| *containers == "ts")
            .expect("a transport-stream row");
        for (stream, direct) in [
            (row["VideoCodec"].as_str(), *ts_video),
            (row["AudioCodec"].as_str(), AUDIO_CODECS),
        ] {
            for codec in stream.expect("a codec list").split(',') {
                assert!(direct.split(',').any(|claimed| claimed == codec), "{codec}");
            }
        }
    }

    /// What the profile refuses to claim, in the two directions that matter:
    /// unmeasured codecs are absent, and the measured ones stop at the frame and
    /// depth they were measured at.
    #[test]
    fn unmeasured_codecs_are_absent_and_measured_ones_keep_their_ceiling() {
        let body = body();
        let rows = body["DirectPlayProfiles"].as_array().expect("rows");
        for row in rows {
            let claimed = row["VideoCodec"].as_str().unwrap_or_default();
            for unclaimed in ["vc1", "wmv3", "wmv2", "mjpeg", "vvc", "mpeg1video"] {
                assert!(
                    !claimed.split(',').any(|codec| codec == unclaimed),
                    "{unclaimed} is not claimed: the WMV family that matters and the \
                     rest were never measured"
                );
            }
            for container in row["Container"].as_str().expect("a container").split(',') {
                assert!(
                    container != "asf" && container != "wmv" && container != "flv",
                    "{container} is a container whose usual contents were never measured"
                );
            }
        }
        // A codec is claimed only in the containers it was played from: MPEG-4
        // part 2 in AVI and nowhere else, and nothing but it there.
        let holds = |container: &str| {
            rows.iter()
                .find(|row| row["Container"] == container)
                .and_then(|row| row["VideoCodec"].as_str())
                .expect("a row for the container")
        };
        assert_eq!(holds("avi"), "mpeg4");
        assert_eq!(holds("mpeg"), "mpeg2video");
        assert!(!holds("ts").contains("mpeg4"));

        let depths: Vec<(&str, &str)> = body["CodecProfiles"]
            .as_array()
            .expect("rows")
            .iter()
            .flat_map(|row| {
                let codec = row["Codec"].as_str().expect("a codec");
                row["Conditions"]
                    .as_array()
                    .expect("conditions")
                    .iter()
                    .filter(|condition| condition["Property"] == "VideoBitDepth")
                    .map(move |condition| (codec, condition["Value"].as_str().expect("a depth")))
            })
            .collect();
        assert_eq!(
            depths,
            [
                ("h264", "8"),
                ("hevc", "10"),
                ("av1", "10"),
                ("vp9", "10"),
                ("mpeg2video", "8"),
                ("mpeg4", "8"),
                ("vp8", "8")
            ]
        );
        let width_of = |index: usize| {
            body["CodecProfiles"][index]["Conditions"]
                .as_array()
                .expect("conditions")
                .iter()
                .find(|condition| condition["Property"] == "Width")
                .and_then(|condition| condition["Value"].as_str())
                .expect("a width ceiling")
                .to_owned()
        };
        assert_eq!(width_of(0), "4096");
        assert_eq!(width_of(4), "1920");
        assert_eq!(width_of(5), "1280");
    }

    /// The header carries the token — which is why it is a secret the moment it
    /// is built — and names the device the server pairs the session with.
    #[test]
    fn the_header_carries_the_token_and_the_device() {
        let token = SecretString::new("c8f0a1d2e3b4456789abcdef01234567");
        let header = stream_header(&device(), &token);

        assert!(header.expose().starts_with("Authorization: MediaBrowser "));
        assert!(header.expose().contains("Token=\"c8f0a1d2"));
        assert!(header.expose().contains("DeviceId=\"0a1b2c3d"));
        // Four commas in one header value is why it reaches mpv as one
        // `change-list … append` item rather than a comma-split list value.
        assert_eq!(header.expose().matches(',').count(), 4);
        assert!(!format!("{header:?}").contains("c8f0a1d2"));
    }
}
