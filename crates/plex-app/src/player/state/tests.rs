use std::collections::HashMap;

use super::{
    clock, language_name, marker_at, track_label, MarkerKind, MarkerSpan, Snapshot, TrackKind,
};

fn getter(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name: &str| map.get(name).cloned()
}

#[test]
fn clock_formats_minutes_and_hours() {
    assert_eq!(clock(0.0), "0:00");
    assert_eq!(clock(65.4), "1:05");
    assert_eq!(clock(3_725.0), "1:02:05");
    assert_eq!(clock(-3.0), "0:00");
}

#[test]
fn audio_labels_name_language_codec_and_layout() {
    let label = track_label(
        TrackKind::Audio,
        Some("eng"),
        Some("Commentary"),
        Some("eac3"),
        Some("6"),
        false,
        false,
    );
    assert_eq!(label, "English · Commentary · EAC3 5.1");
    let truehd = track_label(
        TrackKind::Audio,
        Some("jpn"),
        None,
        Some("truehd"),
        Some("8"),
        false,
        false,
    );
    assert_eq!(truehd, "Japanese · TrueHD 7.1");
}

#[test]
fn subtitle_labels_mark_forced_and_name_the_format() {
    let label = track_label(
        TrackKind::Subtitle,
        Some("en"),
        None,
        Some("hdmv_pgs_subtitle"),
        None,
        true,
        false,
    );
    assert_eq!(label, "English · Forced · PGS");
    assert_eq!(language_name("xx"), "xx");
    assert_eq!(language_name("ja-JP"), "Japanese");
    assert_eq!(language_name("pt-BR"), "Portuguese (Brazil)");
    assert_eq!(language_name("es-419"), "Spanish (Latin America)");
    let signs = track_label(
        TrackKind::Subtitle,
        Some("en-US"),
        Some("Forced (Signs only)"),
        Some("ass"),
        None,
        true,
        false,
    );
    assert_eq!(signs, "English · Forced (Signs only) · ASS");
}

#[test]
fn a_snapshot_reads_tracks_chapters_and_hdr_from_properties() {
    let get = getter(&[
        ("time-pos", "12.5"),
        ("duration", "100"),
        ("pause", "no"),
        ("track-list/count", "3"),
        ("track-list/0/type", "video"),
        ("track-list/0/id", "1"),
        ("track-list/1/type", "audio"),
        ("track-list/1/id", "1"),
        ("track-list/1/lang", "eng"),
        ("track-list/1/codec", "truehd"),
        ("track-list/1/demux-channel-count", "8"),
        ("track-list/1/selected", "yes"),
        ("track-list/2/type", "sub"),
        ("track-list/2/id", "2"),
        ("track-list/2/lang", "fre"),
        ("track-list/2/codec", "subrip"),
        ("chapter-list/count", "2"),
        ("chapter-list/0/time", "0"),
        ("chapter-list/1/time", "60.5"),
        ("chapter-list/1/title", "The Heist"),
        ("video-params/gamma", "pq"),
    ]);
    let mut snap = Snapshot::default();
    snap.refresh_from(&get, true);
    assert_eq!(snap.position, Some(12.5));
    assert!((snap.progress() - 0.125).abs() < 1e-6);
    assert_eq!(snap.audio_tracks().count(), 1);
    assert!(snap.audio_tracks().all(|t| t.selected));
    let subs: Vec<_> = snap.subtitle_tracks().collect();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs.first().map(|t| t.label.as_str()), Some("French · SRT"));
    assert_eq!(
        snap.chapters,
        vec![
            (0.0, "Chapter 1".to_owned()),
            (60.5, "The Heist".to_owned())
        ]
    );
    assert_eq!(snap.video.hdr_label(), Some("HDR (PQ)"));
}

#[test]
fn a_fast_refresh_keeps_the_slow_fields() {
    let mut snap = Snapshot::default();
    snap.refresh_from(
        &getter(&[("chapter-list/count", "1"), ("chapter-list/0/time", "5")]),
        true,
    );
    snap.refresh_from(&getter(&[("time-pos", "9")]), false);
    assert_eq!(snap.chapters.len(), 1);
    assert_eq!(snap.position, Some(9.0));
}

#[test]
fn the_skip_button_shows_inside_a_marker_but_not_in_its_last_second() {
    let markers = vec![MarkerSpan {
        kind: MarkerKind::Intro,
        start: 30.0,
        end: 90.0,
    }];
    assert!(marker_at(&markers, 29.0).is_none());
    assert!(marker_at(&markers, 30.0).is_some());
    assert!(marker_at(&markers, 88.0).is_some());
    assert!(marker_at(&markers, 89.5).is_none());
}

#[test]
fn the_active_track_ids_win_over_a_stale_selected_flag() {
    let get = getter(&[
        ("track-list/count", "2"),
        ("track-list/0/type", "audio"),
        ("track-list/0/id", "1"),
        ("track-list/0/selected", "yes"),
        ("track-list/1/type", "sub"),
        ("track-list/1/id", "1"),
        ("track-list/1/selected", "no"),
        ("aid", "no"),
        ("sid", "1"),
    ]);
    let mut snap = Snapshot::default();
    snap.refresh_from(&get, true);
    assert!(snap.audio_tracks().all(|t| !t.selected));
    assert!(snap.subtitle_tracks().all(|t| t.selected));
}

#[test]
fn a_title_that_restates_the_language_is_left_out() {
    let label = |lang, title| {
        track_label(
            TrackKind::Subtitle,
            Some(lang),
            Some(title),
            Some("ass"),
            None,
            false,
            false,
        )
    };
    assert_eq!(
        label("es-419", "Latin American"),
        "Spanish (Latin America) · ASS"
    );
    assert_eq!(label("eng", "English"), "English · ASS");
    assert_eq!(label("spa", "Castilian"), "Spanish · Castilian · ASS");
    assert_eq!(language_name("fil-PH"), "Filipino");
}

#[test]
fn a_sidecar_track_takes_the_servers_language_and_title() {
    let get = getter(&[
        ("track-list/count", "1"),
        ("track-list/0/type", "sub"),
        ("track-list/0/id", "3"),
        ("track-list/0/codec", "subrip"),
        ("track-list/0/external", "yes"),
        (
            "track-list/0/external-filename",
            "https://tower/library/streams/7",
        ),
    ]);
    let mut snap = Snapshot {
        sidecars: vec![super::SubFile {
            url: "https://tower/library/streams/7".into(),
            lang: Some("fr".into()),
            title: Some("SDH".into()),
            forced: false,
        }],
        ..Snapshot::default()
    };
    snap.refresh_from(&get, true);
    let label = snap.subtitle_tracks().next().map(|t| t.label.clone());
    assert_eq!(label.as_deref(), Some("French · SDH · External · SRT"));
}

#[test]
fn a_listed_stream_mpv_deselected_is_reported_as_undecoded() {
    let tracks = [
        ("track-list/count", "2"),
        ("track-list/0/type", "video"),
        ("track-list/1/type", "audio"),
    ];
    let with = |vid: &'static str, aid: &'static str| {
        let mut pairs = tracks.to_vec();
        pairs.extend([("vid", vid), ("aid", aid)]);
        let mut snapshot = Snapshot::default();
        snapshot.refresh_from(&getter(&pairs), true);
        snapshot.undecoded
    };
    assert_eq!(with("1", "1"), None);
    assert_eq!(with("no", "1"), Some("video"));
    assert_eq!(with("1", "no"), Some("audio"));
    // Before a file loads both are `auto`, with nothing listed yet.
    assert_eq!(with("auto", "auto"), None);
}

#[test]
fn a_file_with_no_such_stream_or_only_cover_art_is_not_missing_it() {
    let mut snapshot = Snapshot::default();
    let music = getter(&[
        ("track-list/count", "2"),
        ("track-list/0/type", "audio"),
        ("track-list/1/type", "video"),
        ("track-list/1/image", "yes"),
        ("vid", "no"),
        ("aid", "1"),
    ]);
    snapshot.refresh_from(&music, true);
    assert_eq!(snapshot.undecoded, None);
}
