//! The bottom control band: seek bar, transport row, stream badges, up-next card.

use egui::{pos2, vec2, Align2, Color32, CursorIcon, Pos2, Rect, Sense, Shape, Stroke};

use crate::player::{clock, MarkerKind, NextUp, Player};
use crate::ui::images::Artwork;
use crate::ui::theme;

use super::icons::Icon;
use super::{icon_button, Menu, Overlay, GLASS};

/// Height of the gradient band behind the controls.
const BAND: f32 = 190.0;
/// Side margin of the bar and the rows.
const SIDE: f32 = 32.0;
/// Room for the time labels either side of the seek bar.
const TIME_W: f32 = 76.0;
/// Hit and icon size of the transport buttons.
const BTN: f32 = 40.0;

/// The whole bottom band.
pub(super) fn bottom(
    ov: &mut Overlay,
    root: &mut egui::Ui,
    screen: Rect,
    p: &mut Player,
    artwork: &mut Artwork,
) {
    let band = Rect::from_min_max(pos2(screen.left(), screen.bottom() - BAND), screen.max);
    root.painter().add(Shape::gradient_rect(
        band,
        egui::epaint::Direction::BottomUp,
        [Color32::from_black_alpha(215), Color32::TRANSPARENT],
    ));
    let row_y = screen.bottom() - 44.0;
    let bar_y = row_y - 46.0;
    seek_row(ov, root, screen, bar_y, p, artwork);
    transport(ov, root, screen, row_y, p);
}

fn seek_row(
    ov: &mut Overlay,
    root: &mut egui::Ui,
    screen: Rect,
    y: f32,
    p: &mut Player,
    artwork: &mut Artwork,
) {
    let snap = p.snapshot();
    let duration = snap.duration.unwrap_or(0.0);
    let position = ov.scrub.or(snap.position).unwrap_or(0.0);
    let painter = root.painter();
    painter.text(
        pos2(screen.left() + SIDE, y),
        Align2::LEFT_CENTER,
        clock(position),
        theme::meta(),
        theme::TEXT,
    );
    let remaining = format!("-{}", clock((duration - position).max(0.0)));
    painter.text(
        pos2(screen.right() - SIDE, y),
        Align2::RIGHT_CENTER,
        remaining,
        theme::meta(),
        theme::SECONDARY,
    );
    let track = Rect::from_min_max(
        pos2(screen.left() + SIDE + TIME_W, y - 10.0),
        pos2(screen.right() - SIDE - TIME_W, y + 10.0),
    );
    seek_bar(ov, root, track, p, artwork);
}

fn x_to_time(track: Rect, x: f32, duration: f64) -> f64 {
    let t = ((x - track.left()) / track.width()).clamp(0.0, 1.0);
    f64::from(t) * duration
}

fn time_to_x(track: Rect, secs: f64, duration: f64) -> f32 {
    if duration <= 0.0 {
        return track.left();
    }
    track.left() + track.width() * (secs / duration).clamp(0.0, 1.0) as f32
}

fn seek_bar(
    ov: &mut Overlay,
    root: &mut egui::Ui,
    hit: Rect,
    p: &mut Player,
    artwork: &mut Artwork,
) {
    let duration = p.snapshot().duration.unwrap_or(0.0);
    let resp = root
        .interact(hit, root.id().with("seek"), Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::PointingHand);
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.drag_started() {
            ov.scrub = Some(x_to_time(hit, pos.x, duration));
        }
        if resp.clicked() {
            p.seek_to(x_to_time(hit, pos.x, duration), true);
        }
    }
    if resp.drag_stopped() {
        if let Some(to) = ov.scrub.take() {
            p.seek_to(to, true);
        }
    }
    let active = resp.hovered() || ov.scrub.is_some();
    paint_track(root, hit, p, ov.scrub, active);
    if let Some(hover) = resp.hover_pos().or_else(|| {
        ov.scrub
            .map(|s| pos2(time_to_x(hit, s, duration), hit.center().y))
    }) {
        if duration > 0.0 {
            preview(
                root,
                hit,
                x_to_time(hit, hover.x, duration),
                hover.x,
                p,
                artwork,
            );
        }
    }
}

fn paint_track(root: &egui::Ui, hit: Rect, p: &Player, scrub: Option<f64>, active: bool) {
    let snap = p.snapshot();
    let duration = snap.duration.unwrap_or(0.0);
    let thick = if active { 6.0 } else { 4.0 };
    let line = Rect::from_center_size(hit.center(), vec2(hit.width(), thick));
    let x = |secs: f64| time_to_x(line, secs, duration);
    let painter = root.painter();
    painter.rect_filled(line, thick / 2.0, Color32::from_white_alpha(60));
    let played = snap.position.unwrap_or(0.0);
    if let Some(ahead) = snap.cached_ahead {
        let r = Rect::from_x_y_ranges(line.left()..=x(played + ahead), line.y_range());
        painter.rect_filled(r, thick / 2.0, Color32::from_white_alpha(110));
    }
    for m in &p.request().markers {
        let tint = match m.kind {
            MarkerKind::Intro => Color32::from_rgba_unmultiplied(120, 170, 255, 150),
            MarkerKind::Credits => Color32::from_rgba_unmultiplied(200, 140, 255, 150),
        };
        let r = Rect::from_x_y_ranges(x(m.start)..=x(m.end), line.y_range());
        painter.rect_filled(r, 0.0, tint);
    }
    let shown = scrub.unwrap_or(played);
    let r = Rect::from_x_y_ranges(line.left()..=x(shown), line.y_range());
    painter.rect_filled(r, thick / 2.0, theme::ACCENT);
    // Gaps at chapter starts, dropped when they would crowd closer than 10 pt.
    let mut last = f32::NEG_INFINITY;
    for (start, _) in snap.chapters.iter().skip(1) {
        let cx = x(*start);
        if cx - last < 10.0 {
            continue;
        }
        last = cx;
        painter.line_segment(
            [pos2(cx, line.top()), pos2(cx, line.bottom())],
            Stroke::new(1.5, Color32::from_black_alpha(150)),
        );
    }
    let knob = if active { 8.0 } else { 0.0 };
    if knob > 0.0 {
        painter.circle_filled(pos2(x(shown), line.center().y), knob, theme::TEXT);
    }
}

/// The hover bubble: chapter thumbnail when the server has one, chapter title, time.
fn preview(root: &egui::Ui, track: Rect, secs: f64, x: f32, p: &Player, artwork: &mut Artwork) {
    let snap = p.snapshot();
    let chapter = snap.chapters.iter().rev().find(|(start, _)| *start <= secs);
    let thumb = p
        .request()
        .chapters
        .iter()
        .rev()
        .find(|c| c.start <= secs)
        .and_then(|c| c.thumb.as_deref());
    let image = artwork.texture(thumb, 320, 180);
    let width = if image.is_some() { 224.0 } else { 140.0 };
    let height = if image.is_some() { 126.0 + 46.0 } else { 46.0 };
    let half = width / 2.0;
    let cx = x.clamp(track.left() + half, track.right() - half);
    let rect = Rect::from_center_size(
        pos2(cx, track.top() - 14.0 - height / 2.0),
        vec2(width, height),
    );
    let painter = root.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        root.id().with("preview"),
    ));
    painter.rect_filled(rect, 10.0, GLASS);
    let mut text_top = rect.top() + 6.0;
    if let Some((id, _)) = image {
        let img = Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(width - 8.0, 126.0 - 8.0));
        painter.image(
            id,
            img,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        text_top = img.bottom() + 6.0;
    }
    if let Some((_, title)) = chapter.filter(|_| snap.chapters.len() > 1) {
        painter.text(
            pos2(rect.center().x, text_top),
            Align2::CENTER_TOP,
            title,
            theme::caption(),
            theme::SECONDARY,
        );
        text_top += 16.0;
    }
    painter.text(
        pos2(rect.center().x, text_top),
        Align2::CENTER_TOP,
        clock(secs),
        theme::body(),
        theme::TEXT,
    );
}

/// Left: play, ±10 s, next, volume. Right: chapters, audio, subtitles, speed, settings, info, fullscreen.
fn transport(ov: &mut Overlay, root: &mut egui::Ui, screen: Rect, y: f32, p: &mut Player) {
    let at = |x: f32| Rect::from_center_size(pos2(x, y), vec2(BTN, BTN));
    let mut x = screen.left() + SIDE + BTN / 2.0;
    let paused = p.snapshot().paused;
    let play = if paused { Icon::Play } else { Icon::Pause };
    if icon_button(root, at(x).expand(4.0), play, "Play/Pause (Space)", false) {
        p.toggle_pause();
    }
    x += BTN + 12.0;
    if icon_button(root, at(x), Icon::Back10, "Back 10 s (←)", false) {
        p.seek_by(-10.0);
    }
    x += BTN + 8.0;
    if icon_button(root, at(x), Icon::Forward10, "Forward 10 s (→)", false) {
        p.seek_by(10.0);
    }
    if let Some(next) = p.request().next.clone() {
        x += BTN + 8.0;
        let label = format!("Next: {}", next.label);
        if icon_button(root, at(x), Icon::Next, &label, false) {
            ov.autoplay_now(p);
        }
    }
    x += BTN + 12.0;
    volume(root, at(x), p);
    right_buttons(ov, root, screen, y, p);
}

impl Overlay {
    /// Skip to the end so [`Overlay::ending`] hands the next episode over.
    fn autoplay_now(&mut self, p: &mut Player) {
        if let Some(d) = p.snapshot().duration {
            p.seek_to(d, false);
        }
        self.autoplay_at = Some(std::time::Instant::now());
    }
}

fn volume(root: &mut egui::Ui, icon_rect: Rect, p: &Player) {
    let snap = p.snapshot();
    let (vol, muted) = (snap.volume, snap.muted);
    let icon = if muted || vol <= 0.0 {
        Icon::Muted
    } else {
        Icon::Volume
    };
    if icon_button(root, icon_rect, icon, "Mute (M)", false) {
        p.set_mute(!muted);
    }
    let track = Rect::from_min_size(
        pos2(icon_rect.right() + 6.0, icon_rect.center().y - 10.0),
        vec2(110.0, 20.0),
    );
    let resp = root
        .interact(track, root.id().with("volume"), Sense::click_and_drag())
        .on_hover_text(format!("Volume {vol:.0}%"));
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let v = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0) * 150.0;
            p.set_volume(f64::from(v));
            if muted {
                p.set_mute(false);
            }
        }
    }
    let line = Rect::from_center_size(track.center(), vec2(track.width(), 4.0));
    let painter = root.painter();
    painter.rect_filled(line, 2.0, Color32::from_white_alpha(60));
    let level = if muted {
        0.0
    } else {
        (vol / 150.0).clamp(0.0, 1.0) as f32
    };
    let fill = Rect::from_min_size(line.min, vec2(line.width() * level, line.height()));
    painter.rect_filled(fill, 2.0, theme::TEXT);
    // 100% marker: past it mpv amplifies.
    let mark = line.left() + line.width() * (100.0 / 150.0);
    painter.line_segment(
        [
            pos2(mark, line.top() - 3.0),
            pos2(mark, line.bottom() + 3.0),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(120)),
    );
    if resp.hovered() || resp.dragged() {
        painter.circle_filled(
            pos2(line.left() + line.width() * level, line.center().y),
            6.0,
            theme::TEXT,
        );
    }
}

fn right_buttons(ov: &mut Overlay, root: &mut egui::Ui, screen: Rect, y: f32, p: &Player) {
    let snap = p.snapshot();
    let mut entries: Vec<(Icon, &str, Option<Menu>)> = Vec::new();
    if snap.chapters.len() > 1 {
        entries.push((Icon::Chapters, "Chapters", Some(Menu::Chapters)));
    }
    entries.push((Icon::Audio, "Audio (A)", Some(Menu::Audio)));
    entries.push((Icon::Subtitles, "Subtitles (C)", Some(Menu::Subtitles)));
    entries.push((Icon::Speed, "Speed ([ / ])", Some(Menu::Speed)));
    entries.push((Icon::Quality, "Quality", Some(Menu::Quality)));
    entries.push((Icon::Settings, "Settings", Some(Menu::Settings)));
    entries.push((Icon::Info, "Info (I)", None));
    let mut x = screen.right() - SIDE - BTN / 2.0;
    let fs_icon = if ov.fullscreen {
        Icon::ExitFullscreen
    } else {
        Icon::Fullscreen
    };
    if icon_button(
        root,
        Rect::from_center_size(pos2(x, y), vec2(BTN, BTN)),
        fs_icon,
        "Fullscreen (F)",
        false,
    ) {
        ov.toggle_fullscreen(root.ctx());
    }
    for (icon, hint, menu) in entries.into_iter().rev() {
        x -= BTN + 8.0;
        let rect = Rect::from_center_size(pos2(x, y), vec2(BTN, BTN));
        let on = match menu {
            Some(m) => ov.menu == Some(m),
            None => ov.info,
        };
        if icon_button(root, rect, icon, hint, on) {
            match menu {
                Some(m) => ov.menu = if on { None } else { Some(m) },
                None => {
                    ov.info = !ov.info;
                    ov.menu = None;
                }
            }
        }
    }
}

/// "4K · HEVC · HDR · TrueHD 7.1" pills at the top right, right-aligned at `right`.
pub(super) fn badges(root: &egui::Ui, right: Pos2, p: &Player) {
    let snap = p.snapshot();
    let facts = &p.request().media;
    let mut items: Vec<String> = Vec::new();
    if let Some(res) = facts.resolution.as_deref() {
        items.push(match res {
            "4k" => "4K".to_owned(),
            "sd" => "SD".to_owned(),
            other if other.chars().all(|c| c.is_ascii_digit()) => format!("{other}p"),
            other => other.to_uppercase(),
        });
    }
    if let Some(hdr) = snap.video.hdr_label() {
        items.push(
            if facts
                .video_profile
                .as_deref()
                .is_some_and(|v| v.contains("dv"))
            {
                "Dolby Vision".to_owned()
            } else {
                hdr.split(' ').next().unwrap_or("HDR").to_owned()
            },
        );
    }
    if let Some(v) = facts.video_codec.as_deref() {
        items.push(v.to_uppercase());
    }
    if snap
        .video
        .hwdec
        .as_deref()
        .is_some_and(|h| h != "no" && !h.is_empty())
    {
        items.push("HW".to_owned());
    }
    let painter = root.painter();
    let mut x = right.x;
    for label in items.iter().rev() {
        let galley = painter.layout_no_wrap(label.clone(), theme::caption(), theme::TEXT);
        let w = galley.size().x + 16.0;
        let rect = Rect::from_min_size(pos2(x - w, right.y - 12.0), vec2(w, 24.0));
        painter.rect_filled(rect, 6.0, GLASS);
        painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, Color32::from_white_alpha(60)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, theme::TEXT);
        x -= w + 8.0;
    }
}

/// The up-next card: `Some(true)` to play now, `Some(false)` to dismiss.
pub(super) fn up_next(
    root: &mut egui::Ui,
    screen: Rect,
    next: &NextUp,
    countdown: Option<u64>,
    artwork: &mut Artwork,
    visible: bool,
) -> Option<bool> {
    let size = vec2(360.0, 120.0);
    let lift = if visible { 150.0 } else { 48.0 };
    let card = Rect::from_min_size(
        screen.right_bottom() - vec2(40.0 + size.x, lift + size.y),
        size,
    );
    let painter = root.painter();
    painter.rect_filled(card, 14.0, GLASS);
    painter.rect_stroke(
        card,
        14.0,
        Stroke::new(1.0, Color32::from_white_alpha(50)),
        egui::StrokeKind::Inside,
    );
    let thumb = Rect::from_min_size(card.min + vec2(10.0, 10.0), vec2(178.0, 100.0));
    match artwork.texture(next.thumb.as_deref(), 356, 200) {
        Some((id, _)) => {
            painter.image(
                id,
                thumb,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(thumb, 8.0, theme::CARD);
        }
    }
    let x = thumb.right() + 12.0;
    let head = match countdown {
        Some(s) => format!("Up next in {s}"),
        None => "Up next".to_owned(),
    };
    painter.text(
        pos2(x, card.top() + 12.0),
        Align2::LEFT_TOP,
        head,
        theme::caption(),
        theme::ACCENT,
    );
    let galley = painter.layout(
        next.label.clone(),
        theme::meta(),
        theme::TEXT,
        card.right() - x - 10.0,
    );
    painter.galley(pos2(x, card.top() + 30.0), galley, theme::TEXT);
    let play = Rect::from_min_size(pos2(x, card.bottom() - 40.0), vec2(92.0, 30.0));
    let cancel = Rect::from_min_size(pos2(play.right() + 8.0, play.top()), vec2(66.0, 30.0));
    if pill(root, play, "Play now", true) {
        return Some(true);
    }
    if pill(root, cancel, "Cancel", false) {
        return Some(false);
    }
    None
}

fn pill(root: &mut egui::Ui, rect: Rect, label: &str, primary: bool) -> bool {
    let resp = root
        .interact(rect, root.id().with(("pill", label)), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let (fill, text) = match (primary, resp.hovered()) {
        (true, _) => (theme::PRIMARY_BG, theme::PRIMARY_TEXT),
        (false, true) => (Color32::from_white_alpha(50), theme::TEXT),
        (false, false) => (Color32::from_white_alpha(22), theme::TEXT),
    };
    let painter = root.painter();
    painter.rect_filled(rect, 15.0, fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::caption(),
        text,
    );
    resp.clicked()
}
