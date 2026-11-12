//! One item, end to end against a mock 12.4 server: the profile the client
//! sends, the source it picks out of the answer, and the three session reports
//! that move the item on the server's "Continue Watching" row.
//!
//! The token's place is asserted here too: it is in every `Authorization`
//! header and in no path, so it can never reach mpv's command line or a log.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::Duration;

use jellyfin_core::browse::Client;
use jellyfin_core::playback::{
    self, CodecProfile, Delivery, DeviceProfile, DirectPlayProfile, DirectPlayRefusal, PlayTarget,
    ProfileCondition, ProfileKind, ProfileProperty, SessionBody, TranscodingProfile,
};
use jellyfin_core::{DeviceInfo, Http, Item, SecretString, UserData};

mod mock;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/live-12.4/");
const USER_ID: &str = "70eb2746aca443c098159c5d6e66d886";
const MOVIE_ID: &str = "5197d80dc0c848286caeb3d345930689";
const SESSION_ID: &str = "50ea4d36ddc843118b7851a0ab3c4453";
const TOKEN: &str = "c8f0a1d2e3b4456789abcdef01234567";
/// The MPEG-2 transport stream the captured server would not hand over as it
/// sits, and the play session its answer named.
const OLD_MOVIE_ID: &str = "359d901ce7e39b79fc3d691180c2be21";
const STREAM_SESSION_ID: &str = "d03b05610bdc4d03ba9d8c6eaa1ee80f";

/// The mock, its transport and the signed-in user, so a test reads as the
/// sequence of calls a player would make.
struct Server {
    mock: mock::MockJfServer,
    http: Http,
    device: DeviceInfo,
    token: SecretString,
}

impl Server {
    fn new() -> Self {
        let mock = mock::MockJfServer::start();
        let http = jellyfin_core::Http::with_timeouts(
            &mock.url(),
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .expect("mock base must normalise");
        Self {
            mock,
            http,
            device: DeviceInfo {
                app: "Chewvy".to_owned(),
                app_version: "0.1.0".to_owned(),
                device: "test-machine".to_owned(),
                device_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned(),
            },
            token: SecretString::new(TOKEN),
        }
    }

    fn route(&self, method_path: &str, status: u16, body: impl Into<String>) {
        self.mock
            .route(method_path, mock::Answer::json(status, body));
    }

    fn client(&self) -> Client<'_> {
        Client::new(&self.http, &self.device, &self.token, USER_ID).expect("a usable user id")
    }

    /// `METHOD /path` per request, with any query dropped.
    fn calls(&self) -> Vec<String> {
        self.mock
            .requests()
            .iter()
            .map(|request| {
                let path = request.path_and_query.split('?').next().unwrap();
                format!("{} {path}", request.method)
            })
            .collect()
    }

    /// The body of one recorded request, read as JSON.
    fn body(&self, index: usize) -> serde_json::Value {
        serde_json::from_str(&self.mock.requests()[index].body).expect("a JSON body")
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}{name}.json")).expect("fixture present")
}

/// The server's answer carries an `mkv` with an H.264 video stream, so a
/// profile that names exactly that is the one that gets it offered.
fn profile() -> DeviceProfile {
    DeviceProfile {
        name: Some("Chewvy (test)".to_owned()),
        max_static_bitrate: Some(100_000_000),
        max_streaming_bitrate: Some(100_000_000),
        direct_play_profiles: vec![
            DirectPlayProfile {
                container: "mkv,mp4".to_owned(),
                kind: ProfileKind::Video,
                video_codec: Some("h264,hevc".to_owned()),
                audio_codec: Some("aac,ac3".to_owned()),
            },
            DirectPlayProfile {
                container: "mp3,flac".to_owned(),
                kind: ProfileKind::Audio,
                video_codec: None,
                audio_codec: None,
            },
        ],
        codec_profiles: vec![CodecProfile {
            kind: ProfileKind::Video,
            codec: Some("h264".to_owned()),
            container: Some("mkv".to_owned()),
            conditions: vec![
                ProfileCondition::at_most(ProfileProperty::Width, 3840),
                ProfileCondition::at_most(ProfileProperty::VideoFramerate, "60.000000"),
            ],
        }],
        transcoding_profiles: vec![TranscodingProfile::hls(
            "ts",
            "h264,hevc",
            "aac,ac3,eac3",
            6,
        )],
    }
}

fn movie() -> Item {
    serde_json::from_str(&fixture("item_movie")).expect("item fixture")
}

/// `POST PlaybackInfo` → resolve → `Playing` → `Progress` → `Stopped`, with the
/// server's own captured answer and its own `204`s.
#[test]
fn a_resolved_direct_play_item_reports_a_whole_session() {
    let server = Server::new();
    server.route(
        &format!("POST /Items/{MOVIE_ID}/PlaybackInfo"),
        200,
        fixture("playback_info"),
    );
    for path in [
        "/Sessions/Playing",
        "/Sessions/Playing/Progress",
        "/Sessions/Playing/Stopped",
    ] {
        server.route(&format!("POST {path}"), 204, "");
    }
    let client = server.client();
    let item = movie();

    let info = client
        .playback_info(MOVIE_ID, &profile(), Delivery::ServerChoice)
        .expect("the captured PlaybackInfo answer");
    let target = playback::resolve(&item, &info, Delivery::ServerChoice)
        .expect("the server offers a direct source");

    // What the player is asked to open: the static source route, and the
    // resume point — which this fixture does not carry, so it stays None.
    assert_eq!(
        target.stream_url(server.mock.url().as_str()),
        format!(
            "{}/Videos/{MOVIE_ID}/stream?static=true&mediaSourceId={MOVIE_ID}",
            server.mock.url()
        )
    );
    assert_eq!(target.media_source_id, MOVIE_ID);
    assert_eq!(target.play_session_id.as_deref(), Some(SESSION_ID));
    assert_eq!(target.duration, Some(Duration::from_secs(30)));
    assert_eq!(target.resume_at, None);

    let session = |position: Duration, paused: bool| SessionBody {
        item_id: &target.item_id,
        media_source_id: Some(&target.media_source_id),
        play_session_id: target.play_session_id.as_deref(),
        position,
        paused,
        duration: target.duration,
        transcoding: target.transcoding,
    };
    client
        .report_playing(&session(Duration::ZERO, false))
        .expect("204");
    client
        .report_progress(&session(Duration::from_secs(10), true))
        .expect("204");
    client
        .report_stopped(&session(Duration::from_secs(30), false))
        .expect("204");

    assert_eq!(
        server.calls(),
        vec![
            format!("POST /Items/{MOVIE_ID}/PlaybackInfo"),
            "POST /Sessions/Playing".to_owned(),
            "POST /Sessions/Playing/Progress".to_owned(),
            "POST /Sessions/Playing/Stopped".to_owned(),
        ],
        "the whole session, in its order, with nothing else asked"
    );

    // The token travels in the header on every call and in no path.
    for recorded in server.mock.requests() {
        assert!(
            !recorded.path_and_query.contains(TOKEN),
            "{}",
            recorded.path_and_query
        );
        assert!(
            recorded
                .authorization
                .as_deref()
                .is_some_and(|header| header.contains(TOKEN)),
            "every call authorizes: {recorded:?}"
        );
    }

    let playing = server.body(1);
    assert_eq!(playing["ItemId"], MOVIE_ID);
    assert_eq!(playing["PlayMethod"], "DirectPlay");
    assert_eq!(playing["MediaSourceId"], MOVIE_ID);
    assert_eq!(playing["PlaySessionId"], SESSION_ID);
    assert_eq!(playing["PositionTicks"], 0);
    assert_eq!(playing["IsPlaying"], true);
    assert_eq!(playing["EventName"], "Playing");
    assert!(
        playing.get("Completed").is_none(),
        "a start does not claim an end: {playing}"
    );

    let progress = server.body(2);
    assert_eq!(progress["PositionTicks"], 100_000_000);
    assert_eq!(progress["IsPaused"], true);
    assert_eq!(progress["IsPlaying"], false);
    assert_eq!(progress["CanSeek"], true);
    assert_eq!(progress["RuntimeTicks"], 300_000_000);

    // 30 s of a 30 s item: only the client knows it reached the end.
    let stopped = server.body(3);
    assert_eq!(stopped["PositionTicks"], 300_000_000);
    assert_eq!(stopped["Completed"], true);
}

/// The resume point belongs to the server: a row that says this user stopped
/// twelve minutes in is where the player starts. The fixture row says `0`, so
/// only the same row with a position can show the mapping.
#[test]
fn a_half_watched_row_supplies_where_the_player_starts() {
    let server = Server::new();
    server.route(
        &format!("POST /Items/{MOVIE_ID}/PlaybackInfo"),
        200,
        fixture("playback_info"),
    );
    let info = server
        .client()
        .playback_info(MOVIE_ID, &profile(), Delivery::ServerChoice)
        .expect("the captured PlaybackInfo answer");
    let mut half_watched = movie();
    half_watched
        .user_data
        .get_or_insert_with(UserData::default)
        .playback_position_ticks = Some(725 * jellyfin_core::ticks::TICKS_PER_SECOND);

    let target = playback::resolve(&half_watched, &info, Delivery::ServerChoice)
        .expect("the same source as ever");
    assert_eq!(target.resume_at, Some(Duration::from_secs(725)));
}

/// A source the server refuses for this profile is a refusal the user can read,
/// and it never reaches the session endpoints.
#[test]
fn a_server_that_offers_no_way_to_play_is_refused_before_any_report() {
    let server = Server::new();
    // The same answer with both flags off and no stream of the server's own.
    let mut answer: serde_json::Value =
        serde_json::from_str(&fixture("playback_info")).expect("playback fixture");
    let source = &mut answer["MediaSources"][0];
    source["SupportsDirectPlay"] = serde_json::Value::Bool(false);
    source["SupportsDirectStream"] = serde_json::Value::Bool(false);
    server.route(
        &format!("POST /Items/{MOVIE_ID}/PlaybackInfo"),
        200,
        answer.to_string(),
    );
    let client = server.client();

    let info = client
        .playback_info(MOVIE_ID, &profile(), Delivery::ServerChoice)
        .expect("the captured PlaybackInfo answer");
    let error =
        playback::resolve(&movie(), &info, Delivery::ServerChoice).expect_err("nothing is offered");

    assert_eq!(
        error,
        DirectPlayRefusal::NotOffered {
            container: Some("mkv".to_owned()),
            video_codec: Some("h264".to_owned()),
            audio_codec: None,
        }
    );
    assert_eq!(
        error.to_string(),
        "Jellyfin would have to convert h264 video in mkv to play it here, and did not \
         offer to: transcoding may be turned off for this user."
    );
    assert_eq!(server.calls().len(), 1, "no report was sent");
}

/// What the profile says about the client is the claim the server answers, so
/// it goes in the body of the one `POST` that reads.
#[test]
fn the_playback_info_body_carries_the_profile_and_starts_nothing() {
    let server = Server::new();
    server.route(
        &format!("POST /Items/{MOVIE_ID}/PlaybackInfo"),
        204,
        fixture("playback_info"),
    );

    let _ = server
        .client()
        .playback_info(MOVIE_ID, &profile(), Delivery::ServerChoice);
    let body = server.body(0);
    let sent = body["DeviceProfile"].clone();

    assert_eq!(sent["DirectPlayProfiles"][0]["Container"], "mkv,mp4");
    assert_eq!(sent["DirectPlayProfiles"][0]["Type"], "Video");
    assert_eq!(sent["DirectPlayProfiles"][0]["VideoCodec"], "h264,hevc");
    // An audio row says nothing about video codecs, because it plays no video.
    assert!(sent["DirectPlayProfiles"][1].get("VideoCodec").is_none());
    assert_eq!(sent["MaxStaticBitrate"], 100_000_000);
    // A ceiling only limits what it was measured against: one row per codec,
    // naming the property, the limit, and that a probe which reported no frame
    // rate does not disqualify the source.
    assert_eq!(sent["CodecProfiles"][0]["Codec"], "h264");
    assert_eq!(
        sent["CodecProfiles"][0]["Conditions"][0]["Property"],
        "Width"
    );
    assert_eq!(
        sent["CodecProfiles"][0]["Conditions"][0]["Condition"],
        "LessThanEqual"
    );
    assert_eq!(
        sent["CodecProfiles"][0]["Conditions"][0]["IsRequired"],
        false
    );
    assert_eq!(sent["TranscodingProfiles"][0]["Protocol"], "hls");
    assert_eq!(sent["TranscodingProfiles"][0]["Container"], "ts");
    assert_eq!(sent["TranscodingProfiles"][0]["Type"], "Video");
    assert_eq!(sent["TranscodingProfiles"][0]["Context"], "Streaming");
    assert_eq!(sent["TranscodingProfiles"][0]["MaxAudioChannels"], "6");
    // Which deliveries this client will accept is the request's business, not
    // the profile's. Left to the server, everything is allowed and nothing is
    // capped.
    assert_eq!(body["EnableDirectPlay"], true);
    assert_eq!(body["EnableDirectStream"], true);
    assert_eq!(body["EnableTranscoding"], true);
    assert_eq!(body["AllowVideoStreamCopy"], true);
    assert_eq!(body["AllowAudioStreamCopy"], true);
    assert!(body.get("MaxStreamingBitrate").is_none());
    for started in [
        "PlayMethod",
        "AutoOpenLiveStream",
        "NowPlayingQueue",
        "StartPositionTicks",
    ] {
        assert!(
            body.get(started).is_none(),
            "{started} would start something: {body}"
        );
    }
}

/// A target's own sentences: the route the player loads and the info-sheet
/// line. Neither names the token, and neither names the file's path.
#[test]
fn a_target_shows_what_plays_without_naming_the_token() {
    let server = Server::new();
    server.route(
        &format!("POST /Items/{MOVIE_ID}/PlaybackInfo"),
        200,
        fixture("playback_info"),
    );
    let info = server
        .client()
        .playback_info(MOVIE_ID, &profile(), Delivery::ServerChoice)
        .expect("the captured PlaybackInfo answer");
    let target: PlayTarget =
        playback::resolve(&movie(), &info, Delivery::ServerChoice).expect("a direct source");

    assert!(target
        .stream_url(&server.mock.url())
        .contains("static=true"));
    assert!(!target.stream_url(&server.mock.url()).contains(TOKEN));
    assert_eq!(target.codecs(), vec!["h264"]);
    assert_eq!(target.to_string(), "h264 in mkv");
    assert!(
        !target.to_string().contains("Aurora"),
        "the server's path for the file is not a name to show: {target}"
    );
}

fn old_movie() -> Item {
    serde_json::from_str(&fixture("item_movie_mpeg2")).expect("item fixture")
}

/// The captured answer for a file outside the profile: no direct flag, and a
/// stream of the server's own. That stream is what plays, its session is what
/// the reports and the stop call name, and the token the server wrote into
/// its URL is gone before anything can read it.
#[test]
fn a_source_the_server_must_convert_plays_as_its_stream_without_the_token() {
    let server = Server::new();
    server.route(
        &format!("POST /Items/{OLD_MOVIE_ID}/PlaybackInfo"),
        200,
        fixture("playback_info_transcode"),
    );
    server.route("POST /Sessions/Playing", 204, "");
    server.route("DELETE /Videos/ActiveEncodings", 204, "");
    assert!(fixture("playback_info_transcode").contains("ApiKey=REDACTED-TOKEN"));
    let client = server.client();

    let info = client
        .playback_info(OLD_MOVIE_ID, &profile(), Delivery::ServerChoice)
        .expect("the captured PlaybackInfo answer");
    assert!(!format!("{info:?}").contains("REDACTED-TOKEN"));
    let target = playback::resolve(&old_movie(), &info, Delivery::ServerChoice)
        .expect("the server offers its own stream");

    assert!(target.transcoding);
    assert!(target
        .path
        .starts_with("/videos/359d901c-e7e3-9b79-fc3d-691180c2be21/master.m3u8?"));
    assert!(!target.path.to_ascii_lowercase().contains("apikey"));
    assert!(target.path.contains("PlaySessionId=d03b0561"));
    assert_eq!(target.play_session_id.as_deref(), Some(STREAM_SESSION_ID));
    assert_eq!(target.codecs(), vec!["mpeg2video", "mp2"]);

    client
        .report_playing(&SessionBody {
            item_id: &target.item_id,
            media_source_id: Some(&target.media_source_id),
            play_session_id: target.play_session_id.as_deref(),
            position: Duration::ZERO,
            paused: false,
            duration: target.duration,
            transcoding: target.transcoding,
        })
        .expect("the report");
    assert_eq!(server.body(1)["PlayMethod"], "Transcode");

    client
        .stop_encoding(STREAM_SESSION_ID)
        .expect("the encode is told to stop");
    let stop = server.mock.requests()[2].clone();
    assert_eq!(stop.method, "DELETE");
    assert_eq!(
        stop.path_and_query,
        format!(
            "/Videos/ActiveEncodings?deviceId=0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d\
             &playSessionId={STREAM_SESSION_ID}"
        )
    );
    assert!(stop
        .authorization
        .is_some_and(|value| value.contains(TOKEN)));
}

/// A remux turns the file off and keeps copying on; a transcode turns copying
/// off too, names its bitrate in the request and its frame on the stream.
#[test]
fn the_delivery_asked_for_is_what_the_request_and_the_stream_say() {
    let server = Server::new();
    let transcode = Delivery::Transcode {
        max_bitrate: 8_000_000,
        max_width: 1920,
        max_height: 1080,
    };
    for _ in 0..2 {
        server.route(
            &format!("POST /Items/{OLD_MOVIE_ID}/PlaybackInfo"),
            200,
            fixture("playback_info_transcode"),
        );
    }
    let client = server.client();

    let remuxed = client
        .playback_info(OLD_MOVIE_ID, &profile(), Delivery::Remux)
        .expect("an answer");
    let body = server.body(0);
    assert_eq!(body["EnableDirectPlay"], false);
    assert_eq!(body["EnableDirectStream"], false);
    assert_eq!(body["EnableTranscoding"], true);
    assert_eq!(body["AllowVideoStreamCopy"], true);
    assert_eq!(body["AllowAudioStreamCopy"], true);
    assert!(body.get("MaxStreamingBitrate").is_none());
    let remux = playback::resolve(&old_movie(), &remuxed, Delivery::Remux).expect("a stream");
    assert!(!remux.path.contains("MaxWidth"));

    let encoded = client
        .playback_info(OLD_MOVIE_ID, &profile(), transcode)
        .expect("an answer");
    let body = server.body(1);
    assert_eq!(body["EnableDirectPlay"], false);
    assert_eq!(body["AllowVideoStreamCopy"], false);
    assert_eq!(body["AllowAudioStreamCopy"], false);
    assert_eq!(body["MaxStreamingBitrate"], 8_000_000);
    let target = playback::resolve(&old_movie(), &encoded, transcode).expect("a stream");
    assert!(target.path.ends_with("&MaxWidth=1920&MaxHeight=1080"));
}

/// A direct flag in the answer does not undo what was asked: a remux of a file
/// the server would also hand over as it sits still plays the server's stream.
#[test]
fn asking_for_the_stream_ignores_a_direct_source() {
    let mut answer: serde_json::Value =
        serde_json::from_str(&fixture("playback_info_transcode")).expect("playback fixture");
    answer["MediaSources"][0]["SupportsDirectPlay"] = serde_json::Value::Bool(true);
    let info = serde_json::from_value(answer).expect("an answer");

    let direct = playback::resolve(&old_movie(), &info, Delivery::ServerChoice).expect("a file");
    assert!(!direct.transcoding);
    assert!(direct.path.contains("static=true"));
    let stream = playback::resolve(&old_movie(), &info, Delivery::Remux).expect("a stream");
    assert!(stream.transcoding);
}
