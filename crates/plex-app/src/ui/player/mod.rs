//! The playing screen: video behind, a transparent control overlay on top.
//!
//! The video itself is drawn by [`crate::player::render_behind`] before egui
//! paints, so nothing here may fill the window: every shape is a gradient,
//! a translucent pill or a popover. The overlay hides after a few idle seconds
//! (and takes the cursor with it) and comes back on any pointer motion or key.

mod bar;
pub(crate) mod hdr;
mod icons;
mod menus;
pub(crate) mod together;

use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align2, Color32, CursorIcon, Key, Rect, Sense, Shape};

use crate::player::{self, marker_at, Ended, MarkerKind, NextUp, Player};
use crate::ui::images::Artwork;
use crate::ui::theme;

use icons::Icon;

/// What the app does after this frame.
pub enum Action {
    /// Keep playing.
    Stay,
    /// Leave the player; `Some` carries a failure to show in the browser.
    Back(Option<String>),
    /// mpv could not play what the server sent: the reason, for the screen to
    /// show unless another delivery of the item is left to try.
    Unplayable(String),
    /// Play this episode next.
    Next(NextUp),
    /// Reopen the same item at `at` seconds with quality `index`.
    Requality { index: usize, at: f64 },
    /// Reopen the same item at `at` seconds in the HDR window (`hdr`) or back
    /// in this one.
    Output { hdr: bool, at: f64 },
    /// A choice in the watch-together panel.
    Group(together::Pick),
}

/// Which popover is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Menu {
    Audio,
    Subtitles,
    Speed,
    Chapters,
    Quality,
    Settings,
    Together,
}

/// Choices mpv does not report back, so the overlay remembers them.
#[derive(Default)]
pub(crate) struct Prefs {
    pub aspect: usize,
    pub sub_pos: Option<i64>,
    /// Pause at this instant (the sleep timer), with its label.
    pub sleep: Option<(Instant, u64)>,
}

/// Overlay state for one playing item.
pub struct Overlay {
    last_activity: Instant,
    pointer: Option<egui::Pos2>,
    pub(crate) menu: Option<Menu>,
    pub(crate) info: bool,
    pub(crate) prefs: Prefs,
    /// When the item finished with an episode queued: autoplay deadline.
    autoplay_at: Option<Instant>,
    /// The user dismissed the up-next card for this item.
    up_next_dismissed: bool,
    /// The up-next line last shown in the HDR window, so each is sent once.
    hdr_up_next: Option<String>,
    /// The seek bar is being dragged to this position.
    pub(crate) scrub: Option<f64>,
    /// A quality picked in the settings popover, applied after this frame.
    pub(crate) requality: Option<usize>,
    /// The watch-together group the player follows, as the app last saw it.
    pub(crate) group: Option<together::Badge>,
    /// What was chosen in the group panel this frame.
    pub(crate) pick: Option<together::Pick>,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            last_activity: Instant::now(),
            pointer: None,
            menu: None,
            info: false,
            prefs: Prefs::default(),
            autoplay_at: None,
            up_next_dismissed: false,
            hdr_up_next: None,
            scrub: None,
            requality: None,
            group: None,
            pick: None,
        }
    }
}

/// Idle time before the controls fade out.
const HIDE_AFTER: Duration = Duration::from_secs(3);
/// Countdown before the next episode starts by itself.
const AUTOPLAY: Duration = Duration::from_secs(10);
/// Show the up-next card this long before the end when there is no credits marker.
const UP_NEXT_LEAD: f64 = 30.0;
/// Translucent fill for pills, buttons and popovers.
pub(crate) const GLASS: Color32 = Color32::from_rgba_premultiplied(14, 15, 18, 215);

impl Overlay {
    /// Draw one frame of the player screen and say what happens next.
    pub fn show(&mut self, root: &mut egui::Ui, artwork: &mut Artwork) -> Action {
        let ctx = root.ctx().clone();
        let screen = root.max_rect();
        let action = player::with(|p| {
            p.tick();
            self.frame(root, screen, p, artwork)
        });
        // Fullscreen is the window's, not this item's: the next episode, another
        // quality and the screen the player returns to all keep it.
        let action = action.unwrap_or(Action::Back(None));
        ctx.request_repaint_after(Duration::from_millis(250));
        action
    }

    fn frame(
        &mut self,
        root: &mut egui::Ui,
        screen: Rect,
        p: &mut Player,
        artwork: &mut Artwork,
    ) -> Action {
        match p.ended() {
            Some(Ended::Failed(why)) => return Action::Back(Some(why.clone())),
            Some(Ended::Unplayable(why)) => return Action::Unplayable(why.clone()),
            // The HDR window was closed: the user left the item there.
            Some(Ended::Stopped) => return Action::Back(None),
            _ => {}
        }
        if let Some(action) = self.keys(root.ctx(), p) {
            return action;
        }
        if p.take_next_asked() {
            if let Some(next) = self.next_up(p) {
                return Action::Next(next);
            }
        }
        self.track_activity(root.ctx());
        self.sleep_timer(p);

        let hidden = !self.controls_visible(p);
        let surface = root.interact(screen, root.id().with("video"), Sense::click());
        if surface.double_clicked() {
            self.toggle_fullscreen(root.ctx());
        } else if surface.clicked() {
            // A click where hidden controls sit only brings them back.
            let on_chrome = surface
                .interact_pointer_pos()
                .is_some_and(|at| at.y < screen.top() + 140.0 || at.y > screen.bottom() - 190.0);
            if !(hidden && on_chrome) && self.menu.take().is_none() && !self.info {
                p.toggle_pause();
            }
            self.info = false;
            self.last_activity = Instant::now();
        }

        let visible = self.controls_visible(p);
        // A source that is still opening has no position yet.
        if p.snapshot().buffering || p.snapshot().position.is_none() {
            spinner(root, screen.center());
        }
        let ended = self.ending(root, screen, p, artwork, visible);
        if !matches!(ended, Action::Stay) {
            return ended;
        }
        if let Some(action) = self.skip_pill(root, screen, p, visible) {
            return action;
        }
        if p.output().is_hdr_window() {
            if let Some(action) = hdr::card(root, screen, p) {
                return action;
            }
        }
        if !visible {
            root.ctx().set_cursor_icon(CursorIcon::None);
            return Action::Stay;
        }
        if self.top_bar(root, screen, p) {
            return Action::Back(None);
        }
        if let Some(action) = hdr::button(root, screen, p) {
            return action;
        }
        bar::bottom(self, root, screen, p, artwork);
        if let Some(menu) = self.menu {
            menus::popover(self, root, p, menu);
        }
        if let Some(pick) = self.pick.take() {
            if pick == together::Pick::Leave {
                self.menu = None;
            }
            return Action::Group(pick);
        }
        if self.info {
            menus::info_sheet(root, p);
        }
        if let Some(at) = p.take_restart() {
            let index = p.request().quality;
            return Action::Requality { index, at };
        }
        match self.requality.take() {
            Some(index) => Action::Requality {
                index,
                at: p.snapshot().position.unwrap_or(0.0),
            },
            None => Action::Stay,
        }
    }

    fn track_activity(&mut self, ctx: &egui::Context) {
        let (pos, any) = ctx.input(|i| {
            let any = i.pointer.any_down() || !i.events.is_empty() && i.pointer.is_moving();
            (i.pointer.hover_pos(), any)
        });
        if pos != self.pointer || any {
            self.pointer = pos;
            self.last_activity = Instant::now();
        }
    }

    fn controls_visible(&self, p: &Player) -> bool {
        p.snapshot().paused
            || self.menu.is_some()
            || self.info
            || self.scrub.is_some()
            || self.last_activity.elapsed() < HIDE_AFTER
    }

    fn sleep_timer(&mut self, p: &mut Player) {
        if let Some((at, _)) = self.prefs.sleep {
            if Instant::now() >= at {
                self.prefs.sleep = None;
                if !p.snapshot().paused {
                    p.toggle_pause();
                }
            }
        }
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        toggle_fullscreen(ctx);
    }

    /// Keyboard shortcuts; `Some` when a key leaves the player.
    fn keys(&mut self, ctx: &egui::Context, p: &mut Player) -> Option<Action> {
        let pressed: Vec<Key> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => Some(*key),
                    _ => None,
                })
                .collect()
        });
        if ctx.egui_wants_keyboard_input() {
            return None;
        }
        for key in pressed {
            self.last_activity = Instant::now();
            if let Some(action) = self.key(ctx, p, key) {
                return Some(action);
            }
        }
        None
    }

    fn key(&mut self, ctx: &egui::Context, p: &mut Player, key: Key) -> Option<Action> {
        let snap = p.snapshot();
        let (volume, muted, speed) = (snap.volume, snap.muted, snap.speed);
        match key {
            Key::Space | Key::K => p.toggle_pause(),
            Key::ArrowLeft | Key::J => p.seek_by(-10.0),
            Key::ArrowRight | Key::L => p.seek_by(10.0),
            Key::ArrowUp => p.set_volume(volume + 5.0),
            Key::ArrowDown => p.set_volume(volume - 5.0),
            Key::M => p.set_mute(!muted),
            Key::F => self.toggle_fullscreen(ctx),
            Key::I => self.info = !self.info,
            // A group sets the pace while in one.
            Key::OpenBracket if self.group.is_none() => p.set_speed(step_speed(speed, -1)),
            Key::CloseBracket if self.group.is_none() => p.set_speed(step_speed(speed, 1)),
            Key::C => cycle_subtitles(p),
            Key::A => cycle_audio(p),
            Key::S => skip_marker(p),
            Key::N => return self.next_up(p).map(Action::Next),
            Key::Escape => {
                if self.menu.take().is_some() || std::mem::take(&mut self.info) {
                    return None;
                }
                if is_fullscreen(ctx) {
                    self.toggle_fullscreen(ctx);
                    return None;
                }
                return Some(Action::Back(None));
            }
            _ => {}
        }
        None
    }

    /// Back, the two title lines and the stream badges; `true` when Back was clicked.
    fn top_bar(&mut self, root: &mut egui::Ui, screen: Rect, p: &Player) -> bool {
        let band = Rect::from_min_size(screen.min, vec2(screen.width(), 140.0));
        root.painter().add(Shape::gradient_rect(
            band,
            egui::epaint::Direction::BottomUp,
            [Color32::TRANSPARENT, Color32::from_black_alpha(190)],
        ));
        let back = Rect::from_min_size(screen.min + vec2(28.0, 28.0), vec2(44.0, 44.0));
        let clicked = round_button(root, back, Icon::ArrowLeft, "Back (Esc)");
        let req = p.request();
        let x = back.right() + 16.0;
        let painter = root.painter();
        painter.text(
            pos2(x, back.top() + 2.0),
            Align2::LEFT_TOP,
            &req.title,
            egui::FontId::proportional(22.0),
            theme::TEXT,
        );
        if let Some(sub) = &req.subtitle {
            painter.text(
                pos2(x, back.top() + 30.0),
                Align2::LEFT_TOP,
                sub,
                theme::body(),
                theme::SECONDARY,
            );
        }
        bar::badges(root, pos2(screen.right() - 28.0, back.center().y), p);
        clicked
    }

    /// "Skip Intro" / "Skip Credits" while a marker plays; `Some` when skipping
    /// credits moves straight to the next episode.
    fn skip_pill(
        &mut self,
        root: &mut egui::Ui,
        screen: Rect,
        p: &mut Player,
        visible: bool,
    ) -> Option<Action> {
        if self.menu.is_some() || self.info {
            return None; // the popovers sit where the pill would
        }
        let pos = p.snapshot().position?;
        let span = marker_at(&p.request().markers, pos)?.clone();
        if span.kind == MarkerKind::Credits && p.request().next.is_some() {
            return None; // the up-next card covers credits
        }
        let label = match span.kind {
            MarkerKind::Intro => "Skip Intro",
            MarkerKind::Credits => "Skip Credits",
        };
        let lift = if visible { 150.0 } else { 56.0 };
        let size = vec2(148.0, 46.0);
        let rect = Rect::from_min_size(
            screen.right_bottom() - vec2(40.0 + size.x, lift + size.y),
            size,
        );
        let resp = root.interact(rect, root.id().with("skip"), Sense::click());
        let fill = if resp.hovered() {
            theme::PRIMARY_BG
        } else {
            GLASS
        };
        let text = if resp.hovered() {
            theme::PRIMARY_TEXT
        } else {
            theme::TEXT
        };
        let painter = root.painter();
        painter.rect_filled(rect, 23.0, fill);
        painter.rect_stroke(
            rect,
            23.0,
            egui::Stroke::new(1.0, Color32::from_white_alpha(90)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            theme::body(),
            text,
        );
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            p.seek_to(span.end, false);
        }
        None
    }

    /// The up-next card near the end and the autoplay countdown after it.
    fn ending(
        &mut self,
        root: &mut egui::Ui,
        screen: Rect,
        p: &mut Player,
        artwork: &mut Artwork,
        visible: bool,
    ) -> Action {
        let finished = p.ended() == Some(&Ended::Finished);
        let Some(next) = self.next_up(p) else {
            // In a group the last frame stays up: the group may seek back.
            return if finished && self.group.is_none() {
                Action::Back(None)
            } else {
                Action::Stay
            };
        };
        if !finished {
            self.autoplay_at = None;
        }
        let snap = p.snapshot();
        let near_end = snap.position.zip(snap.duration).is_some_and(|(pos, dur)| {
            let credits = p
                .request()
                .markers
                .iter()
                .find(|m| m.kind == MarkerKind::Credits);
            match credits {
                Some(c) => pos >= c.start,
                None => dur - pos <= UP_NEXT_LEAD,
            }
        });
        if finished {
            let at = *self
                .autoplay_at
                .get_or_insert_with(|| Instant::now() + AUTOPLAY);
            if Instant::now() >= at && !self.up_next_dismissed {
                return Action::Next(next);
            }
        }
        if (!near_end && !finished) || self.up_next_dismissed {
            return if finished && self.up_next_dismissed {
                Action::Back(None)
            } else {
                Action::Stay
            };
        }
        let countdown = self
            .autoplay_at
            .map(|at| at.saturating_duration_since(Instant::now()).as_secs() + 1);
        if p.output().is_hdr_window() {
            self.up_next_in_hdr_window(p, &next, countdown);
        }
        match bar::up_next(root, screen, &next, countdown, artwork, visible) {
            Some(true) => Action::Next(next),
            Some(false) => {
                self.up_next_dismissed = true;
                Action::Stay
            }
            None => Action::Stay,
        }
    }
}

impl Overlay {
    /// The episode this player would hand over to. None while it follows a
    /// group: the group starts its titles together, and a hand-off here
    /// would take this player out of step with it.
    pub(super) fn next_up(&self, p: &Player) -> Option<NextUp> {
        p.request().next.clone().filter(|_| self.group.is_none())
    }

    /// The up-next card is in this window, which the HDR window may cover:
    /// say the same over the picture there, once per line.
    fn up_next_in_hdr_window(&mut self, p: &Player, next: &NextUp, countdown: Option<u64>) {
        let (line, time) = match countdown {
            Some(seconds) => (
                format!("Up next in {seconds} · {} · N plays it now", next.label),
                Duration::from_millis(1500),
            ),
            None => (
                format!("Up next · {} · N plays it now", next.label),
                Duration::from_secs(6),
            ),
        };
        if self.hdr_up_next.as_deref() != Some(line.as_str()) {
            p.show_text(&line, time);
            self.hdr_up_next = Some(line);
        }
    }
}

/// Whether the window is fullscreen, as the window itself last reported. Read
/// from the window rather than kept here, so a new overlay (the next episode,
/// another quality) and the browsing screens agree with what is on screen.
pub(crate) fn is_fullscreen(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
}

/// Enter or leave fullscreen.
pub(crate) fn toggle_fullscreen(ctx: &egui::Context) {
    let on = is_fullscreen(ctx);
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!on));
}

/// A 44 pt glass circle with an icon; `true` when clicked.
pub(crate) fn round_button(root: &mut egui::Ui, rect: Rect, icon: Icon, hint: &str) -> bool {
    let resp = root
        .interact(rect, root.id().with(hint), Sense::click())
        .on_hover_text(hint)
        .on_hover_cursor(CursorIcon::PointingHand);
    let fill = if resp.hovered() {
        Color32::from_white_alpha(40)
    } else {
        GLASS
    };
    root.painter()
        .circle_filled(rect.center(), rect.width() / 2.0, fill);
    icons::paint(
        root.painter(),
        icon,
        rect.shrink(rect.width() * 0.22),
        theme::TEXT,
    );
    resp.clicked()
}

/// An icon with no fill until hovered; `true` when clicked.
pub(crate) fn icon_button(
    root: &mut egui::Ui,
    rect: Rect,
    icon: Icon,
    hint: &str,
    on: bool,
) -> bool {
    let resp = root
        .interact(rect, root.id().with(hint), Sense::click())
        .on_hover_text(hint)
        .on_hover_cursor(CursorIcon::PointingHand);
    if resp.hovered() || on {
        let alpha = if on { 46 } else { 30 };
        root.painter()
            .rect_filled(rect, 8.0, Color32::from_white_alpha(alpha));
    }
    let color = if on { theme::ACCENT } else { theme::TEXT };
    icons::paint(root.painter(), icon, rect.shrink(rect.width() * 0.2), color);
    resp.clicked()
}

fn spinner(root: &egui::Ui, center: egui::Pos2) {
    let t = root.input(|i| i.time) as f32;
    let start = t * 5.0;
    let pts: Vec<egui::Pos2> = (0..=28)
        .map(|i| {
            let a = start + i as f32 / 28.0 * std::f32::consts::PI * 1.4;
            center + vec2(a.cos(), a.sin()) * 26.0
        })
        .collect();
    root.painter().circle_filled(center, 40.0, GLASS);
    root.painter()
        .add(Shape::line(pts, egui::Stroke::new(4.0, theme::ACCENT)));
    root.ctx().request_repaint();
}

/// The speeds the menu and the `[`/`]` keys step through.
pub(crate) const SPEEDS: [f64; 11] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0, 8.0];

fn step_speed(current: f64, dir: i32) -> f64 {
    let idx = SPEEDS
        .iter()
        .position(|s| (s - current).abs() < 0.01)
        .unwrap_or(3) as i32;
    let next = (idx + dir).clamp(0, SPEEDS.len() as i32 - 1) as usize;
    SPEEDS.get(next).copied().unwrap_or(1.0)
}

fn cycle_subtitles(p: &Player) {
    let subs: Vec<_> = p.snapshot().subtitle_tracks().collect();
    let current = subs.iter().position(|t| t.selected);
    let next = match current {
        None => subs.first().map(|t| t.id),
        Some(i) => subs.get(i + 1).map(|t| t.id),
    };
    p.set_subtitle(next);
}

fn cycle_audio(p: &Player) {
    let tracks: Vec<_> = p.snapshot().audio_tracks().collect();
    if tracks.len() < 2 {
        return;
    }
    let current = tracks.iter().position(|t| t.selected).unwrap_or(0);
    if let Some(t) = tracks.get((current + 1) % tracks.len()) {
        p.set_audio(t.id);
    }
}

fn skip_marker(p: &mut Player) {
    let Some(pos) = p.snapshot().position else {
        return;
    };
    if let Some(end) = marker_at(&p.request().markers, pos).map(|m| m.end) {
        p.seek_to(end, false);
    }
}

#[cfg(test)]
mod tests {
    use super::step_speed;

    #[test]
    fn speed_steps_through_the_menu_values_and_stops_at_the_ends() {
        assert!((step_speed(1.0, 1) - 1.25).abs() < 1e-9);
        assert!((step_speed(1.0, -1) - 0.75).abs() < 1e-9);
        assert!((step_speed(8.0, 1) - 8.0).abs() < 1e-9);
        assert!((step_speed(0.25, -1) - 0.25).abs() < 1e-9);
    }
}
