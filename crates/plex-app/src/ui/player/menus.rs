//! Popovers above the right-hand buttons, and the stream info sheet.

use std::time::{Duration, Instant};

use egui::{vec2, Align2, Color32, CornerRadius, Margin, RichText, Sense, Stroke};

use crate::player::prefs::{self, Channels};
use crate::player::quality::{self, QUALITIES};
use crate::player::{clock, codec_name, Player};
use crate::ui::theme;

use super::icons::{self, Icon};
use super::{Menu, Overlay, GLASS, SPEEDS};

/// The sleep timer choices, in minutes (0 = off).
const SLEEP: [u64; 6] = [0, 15, 30, 45, 60, 90];
/// Aspect overrides: label and mpv value.
const ASPECTS: [(&str, &str); 5] = [
    ("Auto", "no"),
    ("16:9", "16:9"),
    ("4:3", "4:3"),
    ("2.35:1", "2.35:1"),
    ("1.85:1", "1.85:1"),
];

fn frame() -> egui::Frame {
    egui::Frame::new()
        .fill(GLASS)
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(40)))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(10))
}

/// The open popover, anchored above the bottom-right buttons.
pub(super) fn popover(ov: &mut Overlay, root: &egui::Ui, p: &mut Player, menu: Menu) {
    egui::Area::new(root.id().with("player-menu"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-32.0, -116.0))
        .order(egui::Order::Foreground)
        .show(root.ctx(), |ui| {
            frame().show(ui, |ui| {
                let visuals = ui.visuals_mut();
                visuals.widgets.inactive.bg_fill = Color32::from_white_alpha(46);
                visuals.selection.bg_fill = theme::ACCENT;
                ui.spacing_mut().slider_width = 150.0;
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
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(
        RichText::new(text)
            .font(theme::caption())
            .color(theme::SECONDARY),
    );
    ui.add_space(2.0);
}

/// A labelled on/off switch.
fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *on = !*on;
    }
    let track = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 40.0, rect.center().y - 10.0),
        vec2(36.0, 20.0),
    );
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let fill = if *on {
        theme::ACCENT
    } else {
        Color32::from_white_alpha(if resp.hovered() { 70 } else { 46 })
    };
    let painter = ui.painter();
    painter.rect_filled(track, 10.0, fill);
    let knob = egui::pos2(
        egui::lerp(track.left() + 10.0..=track.right() - 10.0, t),
        track.center().y,
    );
    painter.circle_filled(knob, 8.0, theme::TEXT);
    painter.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        label,
        theme::body(),
        theme::TEXT,
    );
}

/// One full-width choice with a check when selected; `true` when clicked.
fn row(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 8.0, Color32::from_white_alpha(26));
    }
    let check =
        egui::Rect::from_center_size(rect.left_center() + vec2(16.0, 0.0), vec2(16.0, 16.0));
    if selected {
        icons::paint(ui.painter(), Icon::Check, check, theme::ACCENT);
    }
    let color = if selected {
        theme::TEXT
    } else {
        theme::SECONDARY
    };
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), theme::body(), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 42.0);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let elided = galley.elided;
    let pos = rect.left_center() + vec2(34.0, -galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    let resp = if elided {
        resp.on_hover_text(label)
    } else {
        resp
    };
    resp.clicked()
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
    ui.separator();
    picture(ui, ov, p);
    ui.separator();
    subtitle_style(ui, ov, p);
    ui.separator();
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
    ui.horizontal_wrapped(|ui| {
        for m in SLEEP {
            let on = ov.prefs.sleep.map_or(m == 0, |(_, chosen)| chosen == m);
            let label = if m == 0 {
                "Off".to_owned()
            } else {
                format!("{m}m")
            };
            if ui.selectable_label(on, label).clicked() {
                ov.prefs.sleep = (m > 0).then(|| (Instant::now() + Duration::from_secs(m * 60), m));
            }
        }
    });
}

fn picture(ui: &mut egui::Ui, ov: &mut Overlay, p: &Player) {
    heading(ui, "PICTURE");
    let mut zoom = (2f64.powf(p.snapshot().zoom) * 100.0).round();
    if ui
        .add(
            egui::Slider::new(&mut zoom, 50.0..=200.0)
                .suffix("%")
                .text("Zoom"),
        )
        .changed()
    {
        p.set_zoom((zoom / 100.0).log2());
    }
    ui.horizontal_wrapped(|ui| {
        for (i, (label, value)) in ASPECTS.iter().enumerate() {
            if ui.selectable_label(ov.prefs.aspect == i, *label).clicked() {
                ov.prefs.aspect = i;
                p.set_aspect(value);
            }
        }
    });
    let mut sticky = prefs::get();
    toggle(ui, &mut sticky.deinterlace, "Deinterlace");
    toggle(ui, &mut sticky.smooth, "Smooth motion");
    remember(p, sticky);
}

fn subtitle_style(ui: &mut egui::Ui, ov: &mut Overlay, p: &Player) {
    heading(ui, "SUBTITLES");
    let snap = p.snapshot();
    let mut size = (snap.sub_scale * 100.0).round();
    if ui
        .add(
            egui::Slider::new(&mut size, 50.0..=200.0)
                .suffix("%")
                .text("Size"),
        )
        .changed()
    {
        p.set_sub_scale(size / 100.0);
    }
    let mut pos = ov.prefs.sub_pos.unwrap_or(100);
    if ui
        .add(
            egui::Slider::new(&mut pos, 50..=100)
                .text("Position")
                .trailing_fill(true),
        )
        .changed()
    {
        ov.prefs.sub_pos = Some(pos);
        p.set_sub_pos(pos);
    }
    let mut delay = snap.sub_delay;
    if ui
        .add(
            egui::Slider::new(&mut delay, -10.0..=10.0)
                .step_by(0.1)
                .suffix(" s")
                .text("Delay"),
        )
        .changed()
    {
        p.set_sub_delay(delay);
    }
}

fn sound(ui: &mut egui::Ui, p: &Player) {
    heading(ui, "AUDIO");
    let mut delay = p.snapshot().audio_delay;
    if ui
        .add(
            egui::Slider::new(&mut delay, -5.0..=5.0)
                .step_by(0.05)
                .suffix(" s")
                .text("Sync offset"),
        )
        .changed()
    {
        p.set_audio_delay(delay);
    }
    let mut sticky = prefs::get();
    toggle(ui, &mut sticky.normalize, "Night mode (even out loudness)");
    toggle(ui, &mut sticky.passthrough, "Passthrough to AV receiver");
    ui.horizontal(|ui| {
        ui.label(theme::secondary("Channels"));
        for c in Channels::ALL {
            if ui
                .selectable_label(sticky.channels == c, c.label())
                .clicked()
            {
                sticky.channels = c;
            }
        }
    });
    remember(p, sticky);
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
            frame().show(ui, |ui| {
                ui.set_width(340.0);
                egui::Grid::new("player-info-grid")
                    .num_columns(2)
                    .spacing(vec2(14.0, 6.0))
                    .show(ui, |ui| {
                        for (section, rows) in info_rows(p) {
                            ui.label(
                                RichText::new(section)
                                    .font(theme::caption())
                                    .color(theme::ACCENT),
                            );
                            ui.end_row();
                            for (k, v) in rows {
                                ui.label(theme::secondary(k));
                                ui.label(RichText::new(v).color(theme::TEXT));
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
