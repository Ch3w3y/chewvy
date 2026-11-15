//! Popovers above the right-hand buttons, and the stream info sheet.

use std::time::{Duration, Instant};

use egui::{vec2, Align2, Margin, RichText};

use crate::player::prefs::{self, Channels};
use crate::player::quality::{self, QUALITIES};
use crate::player::{clock, codec_name, Player};
use crate::ui::{controls, glass, theme};

use super::{Menu, Overlay, SPEEDS};

/// The sleep timer choices, in minutes (0 = off).
const SLEEP: [u64; 6] = [0, 15, 30, 45, 60, 90];
/// A popover's corner radius: the app's panel radius.
const RADIUS: f32 = 12.0;
/// The room a popover keeps at its right for its scroll bar.
const SCROLL_GUTTER: i8 = 10;
/// Aspect overrides: label and mpv value.
const ASPECTS: [(&str, &str); 5] = [
    ("Auto", "no"),
    ("16:9", "16:9"),
    ("4:3", "4:3"),
    ("2.35:1", "2.35:1"),
    ("1.85:1", "1.85:1"),
];

/// A popover on the app's glass: `add` inside a padded frame, the glass
/// filled in under it once its size is known.
fn glass_frame(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let slot = glass::reserve(ui.painter());
    let inner = egui::Frame::NONE
        .inner_margin(Margin::same(12))
        .show(ui, add);
    glass::fill(
        ui.painter(),
        slot,
        inner.response.rect,
        RADIUS,
        theme::current().glass.panel,
    );
}

/// The open popover, anchored above the bottom-right buttons.
pub(super) fn popover(ov: &mut Overlay, root: &egui::Ui, p: &mut Player, menu: Menu) {
    egui::Area::new(root.id().with("player-menu"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-32.0, -116.0))
        .order(egui::Order::Foreground)
        .show(root.ctx(), |ui| {
            glass_frame(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.set_width(if menu == Menu::Settings { 340.0 } else { 300.0 });
                // The group panel changes while it is open (people join, an
                // Invite button appears), and a scroll area keeps the height
                // it opened with: this one panel sizes itself instead.
                if menu == Menu::Together {
                    if let Some(group) = &ov.group {
                        ov.pick = super::together::panel(ui, group);
                    }
                    return;
                }
                egui::ScrollArea::vertical()
                    .scroll_source(
                        egui::scroll_area::ScrollSource::SCROLL_BAR
                            | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
                    )
                    .max_height(560.0)
                    .show(ui, |ui| {
                        // Room for the scroll bar, so it never sits over a
                        // value or a switch at the row's end.
                        let gutter = Margin {
                            right: SCROLL_GUTTER,
                            ..Margin::ZERO
                        };
                        egui::Frame::NONE
                            .inner_margin(gutter)
                            .show(ui, |ui| match menu {
                                Menu::Audio => audio(ui, p),
                                Menu::Subtitles => subtitles(ui, p),
                                Menu::Speed => speed(ui, p),
                                Menu::Chapters => chapters(ui, p),
                                Menu::Quality => quality(ui, ov, p),
                                Menu::Settings => settings(ui, ov, p),
                                Menu::Together => {}
                            });
                    });
            });
        });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(
        RichText::new(text)
            .font(theme::caption())
            .color(theme::palette().secondary),
    );
    ui.add_space(2.0);
}

/// A labelled on/off switch.
fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str) {
    if controls::switch_row(ui, label, None, *on).clicked() {
        *on = !*on;
    }
}

/// One full-width choice with a check when selected; `true` when clicked.
fn row(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    controls::option_row(ui, label, selected).clicked()
}

fn audio(ui: &mut egui::Ui, p: &Player) {
    heading(ui, "AUDIO");
    let tracks: Vec<_> = p.snapshot().audio_tracks().cloned().collect();
    if tracks.is_empty() {
        ui.label(theme::secondary("No audio tracks"));
    }
    for t in tracks {
        if row(ui, &t.label, t.selected) {
            p.set_audio(t.id);
        }
    }
}

fn subtitles(ui: &mut egui::Ui, p: &Player) {
    heading(ui, "SUBTITLES");
    let tracks: Vec<_> = p.snapshot().subtitle_tracks().cloned().collect();
    let none = !tracks.iter().any(|t| t.selected);
    if row(ui, "Off", none) {
        p.set_subtitle(None);
    }
    for t in tracks {
        if row(ui, &t.label, t.selected) {
            p.set_subtitle(Some(t.id));
        }
    }
}

fn speed(ui: &mut egui::Ui, p: &Player) {
    heading(ui, "PLAYBACK SPEED");
    let current = p.snapshot().speed;
    for s in SPEEDS {
        let label = if (s - 1.0).abs() < 1e-9 {
            "Normal".to_owned()
        } else {
            format!("{s}×")
        };
        if row(ui, &label, (s - current).abs() < 0.01) {
            p.set_speed(s);
        }
    }
}

fn chapters(ui: &mut egui::Ui, p: &mut Player) {
    heading(ui, "CHAPTERS");
    let snap = p.snapshot();
    let pos = snap.position.unwrap_or(0.0);
    let list = snap.chapters.clone();
    let current = list.iter().rposition(|(start, _)| *start <= pos);
    for (i, (start, title)) in list.iter().enumerate() {
        if row(
            ui,
            &format!("{}  ·  {title}", clock(*start)),
            current == Some(i),
        ) {
            p.set_chapter(i);
        }
    }
}

fn settings(ui: &mut egui::Ui, ov: &mut Overlay, p: &Player) {
    sleep_timer(ui, ov);
    controls::rule(ui);
    picture(ui, ov, p);
    controls::rule(ui);
    subtitle_style(ui, ov, p);
    controls::rule(ui);
    sound(ui, p);
}

fn quality(ui: &mut egui::Ui, ov: &mut Overlay, p: &Player) {
    heading(ui, "QUALITY");
    let current = p.request().quality;
    for (index, q) in QUALITIES.iter().enumerate() {
        if row(ui, q.label, index == current) && index != current {
            ov.requality = Some(index);
        }
    }
}

fn sleep_timer(ui: &mut egui::Ui, ov: &mut Overlay) {
    let left = ov
        .prefs
        .sleep
        .map(|(at, _)| at.saturating_duration_since(Instant::now()).as_secs() / 60 + 1);
    heading(
        ui,
        &match left {
            Some(m) => format!("SLEEP TIMER · pauses in {m} min"),
            None => "SLEEP TIMER".to_owned(),
        },
    );
    let labels: Vec<String> = SLEEP
        .iter()
        .map(|m| {
            if *m == 0 {
                "Off".to_owned()
            } else {
                format!("{m}m")
            }
        })
        .collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let chosen = SLEEP
        .iter()
        .position(|m| ov.prefs.sleep.map_or(*m == 0, |(_, chosen)| chosen == *m));
    if let Some(m) = controls::segmented(ui, &labels, chosen).and_then(|i| SLEEP.get(i)) {
        let m = *m;
        ov.prefs.sleep = (m > 0).then(|| (Instant::now() + Duration::from_secs(m * 60), m));
    }
}

fn picture(ui: &mut egui::Ui, ov: &mut Overlay, p: &Player) {
    heading(ui, "PICTURE");
    let mut zoom = (2f64.powf(p.snapshot().zoom) * 100.0).round();
    if controls::slider(ui, "Zoom", &mut zoom, 50.0..=200.0, 1.0, percent).changed() {
        p.set_zoom((zoom / 100.0).log2());
    }
    let labels: Vec<&str> = ASPECTS.iter().map(|(label, _)| *label).collect();
    let picked = controls::segmented(ui, &labels, Some(ov.prefs.aspect));
    if let Some((i, (_, value))) = picked.and_then(|i| ASPECTS.get(i).map(|a| (i, a))) {
        ov.prefs.aspect = i;
        p.set_aspect(value);
    }
    let mut sticky = prefs::get();
    toggle(ui, &mut sticky.deinterlace, "Deinterlace");
    toggle(ui, &mut sticky.smooth, "Smooth motion");
    remember(p, sticky);
}

fn subtitle_style(ui: &mut egui::Ui, ov: &mut Overlay, p: &Player) {
    heading(ui, "SUBTITLES");
    let snap = p.snapshot();
    let mut size = (snap.sub_scale * 100.0).round();
    if controls::slider(ui, "Size", &mut size, 50.0..=200.0, 1.0, percent).changed() {
        p.set_sub_scale(size / 100.0);
    }
    let mut pos = ov.prefs.sub_pos.unwrap_or(100) as f64;
    if controls::slider(ui, "Position", &mut pos, 50.0..=100.0, 1.0, percent).changed() {
        let pos = pos.round().clamp(50.0, 100.0) as i64;
        ov.prefs.sub_pos = Some(pos);
        p.set_sub_pos(pos);
    }
    let mut delay = snap.sub_delay;
    if controls::slider(ui, "Delay", &mut delay, -10.0..=10.0, 0.1, seconds).changed() {
        p.set_sub_delay(delay);
    }
}

fn sound(ui: &mut egui::Ui, p: &Player) {
    heading(ui, "AUDIO");
    let mut delay = p.snapshot().audio_delay;
    if controls::slider(ui, "Sync offset", &mut delay, -5.0..=5.0, 0.05, seconds).changed() {
        p.set_audio_delay(delay);
    }
    let mut sticky = prefs::get();
    toggle(ui, &mut sticky.normalize, "Night mode (even out loudness)");
    toggle(ui, &mut sticky.passthrough, "Passthrough to AV receiver");
    ui.label(theme::secondary("Channels").size(theme::META_SIZE));
    let labels: Vec<&str> = Channels::ALL.iter().map(|c| c.label()).collect();
    let chosen = Channels::ALL.iter().position(|c| *c == sticky.channels);
    if let Some(c) = controls::segmented(ui, &labels, chosen).and_then(|i| Channels::ALL.get(i)) {
        sticky.channels = *c;
    }
    remember(p, sticky);
}

/// A slider's value as a whole percentage.
fn percent(value: f64) -> String {
    format!("{value:.0}%")
}

/// A slider's value as signed seconds.
fn seconds(value: f64) -> String {
    format!("{value:+.2} s")
}

/// Keep `sticky` for later items and apply it now, when it changed.
fn remember(p: &Player, sticky: prefs::Sticky) {
    if sticky != prefs::get() {
        prefs::set(sticky);
        p.apply_sticky(sticky);
    }
}

/// The stream details panel on the right.
pub(super) fn info_sheet(root: &egui::Ui, p: &Player) {
    egui::Area::new(root.id().with("player-info"))
        .anchor(Align2::RIGHT_TOP, vec2(-32.0, 96.0))
        .order(egui::Order::Foreground)
        .show(root.ctx(), |ui| {
            glass_frame(ui, |ui| {
                ui.set_width(340.0);
                egui::Grid::new("player-info-grid")
                    .num_columns(2)
                    .spacing(vec2(14.0, 6.0))
                    .show(ui, |ui| {
                        for (section, rows) in info_rows(p) {
                            ui.label(
                                RichText::new(section)
                                    .font(theme::caption())
                                    .color(theme::palette().accent),
                            );
                            ui.end_row();
                            for (k, v) in rows {
                                ui.label(theme::secondary(k));
                                ui.label(RichText::new(v).color(theme::palette().text));
                                ui.end_row();
                            }
                        }
                    });
            });
        });
}

type Section = (&'static str, Vec<(&'static str, String)>);

fn info_rows(p: &Player) -> Vec<Section> {
    let snap = p.snapshot();
    let facts = &p.request().media;
    let v = &snap.video;
    let opt = |s: &Option<String>| s.clone().unwrap_or_else(|| "—".to_owned());
    let mut file = vec![(
        "Playback",
        quality::quality(p.request().quality).label.to_owned(),
    )];
    file.push(("Container", opt(&facts.container).to_uppercase()));
    if let Some(kbps) = facts.bitrate_kbps {
        file.push(("Bitrate", format!("{:.1} Mbps", kbps as f64 / 1000.0)));
    }
    if let Some(bytes) = facts.size_bytes {
        file.push(("Size", format!("{:.2} GB", bytes as f64 / 1e9)));
    }
    let mut video = vec![("Codec", opt(&v.codec).to_uppercase())];
    if let Some(profile) = &facts.video_profile {
        video.push(("Profile", profile.clone()));
    }
    if let Some((w, h)) = v.size {
        video.push(("Resolution", format!("{w}×{h}")));
    }
    if let Some(fps) = v.fps {
        video.push(("Frame rate", format!("{fps:.3} fps")));
    }
    video.push(("Dynamic range", v.hdr_label().unwrap_or("SDR").to_owned()));
    video.push((
        "Colour",
        format!("{} / {}", opt(&v.primaries), opt(&v.gamma)),
    ));
    video.push(("Pixel format", opt(&v.pixel_format)));
    video.push((
        "Decoder",
        match v.hwdec.as_deref() {
            Some(h) if h != "no" && !h.is_empty() => format!("Hardware ({h})"),
            _ => "Software".to_owned(),
        },
    ));
    if let Some(bps) = v.video_bitrate {
        video.push(("Live bitrate", format!("{:.1} Mbps", bps / 1e6)));
    }
    let audio = vec![
        (
            "Codec",
            v.audio_codec
                .as_deref()
                .map_or_else(|| "—".to_owned(), codec_name),
        ),
        (
            "Channels",
            v.audio_channels.as_deref().map_or_else(
                || "—".to_owned(),
                |c| c.split('(').next().unwrap_or(c).to_owned(),
            ),
        ),
    ];
    let mut stats = vec![(
        "Buffered",
        snap.cached_ahead
            .map_or_else(|| "—".to_owned(), |s| format!("{s:.0} s ahead")),
    )];
    stats.push((
        "Dropped frames",
        v.dropped.map_or_else(|| "—".to_owned(), |d| d.to_string()),
    ));
    vec![
        ("FILE", file),
        ("VIDEO", video),
        ("AUDIO", audio),
        ("PLAYBACK", stats),
    ]
}
