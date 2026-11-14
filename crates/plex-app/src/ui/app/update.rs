//! Updates on screen, one flow for every install: a notice says a release is
//! out; clicking it opens the update prompt; "Update now" downloads and
//! installs in the prompt, with its progress; the prompt then asks to restart,
//! and the restart opens the new version. The notice shows the same progress
//! in small while the prompt is put away. Settings' "Check now" opens the same
//! prompt.

use std::f32::consts::TAU;

use egui::{pos2, vec2, Align2, Color32, CursorIcon, Margin, Rect, Sense, Stroke};

use crate::ui::rail::Entry;
use crate::ui::{chrome, marks, theme};
use crate::update::{Release, State, Updater, CURRENT};

use super::app_settings::{chip, section_panel, setting_row};
use super::Ui;

/// The prompt's width.
const PROMPT_WIDTH: f32 = 420.0;
/// The tallest the release notes run in the prompt before they scroll.
const NOTES_HEIGHT: f32 = 200.0;
/// The notice's round icon.
const ICON: f32 = 34.0;
/// A progress bar's height.
const BAR: f32 = 6.0;

/// What a click in the update screens asked for.
enum Ask {
    Open,
    Check,
    Install,
    Retry,
    Restart,
    Page(String),
    Close,
    Dismiss,
}

impl Ui {
    /// The check a start makes, once, when the settings allow it.
    pub(super) fn update_at_start(&mut self) {
        // Tests build the screens without a network to ask.
        if cfg!(test) || !self.settings.check_updates {
            return;
        }
        if matches!(self.updater.state(), State::Idle) {
            self.updater.check();
        }
    }

    /// Whether the update prompt is up, so the screens under it take no keys.
    pub(super) fn update_prompt_open(&self) -> bool {
        self.update_prompt
    }

    /// Settings' Updates section: the version, a line on where the updater is,
    /// the switch for the check at start, and the way into the prompt.
    pub(super) fn updates_section(&mut self, ui: &mut egui::Ui) {
        let mut ask = None;
        let state = self.updater.state().clone();
        section_panel(ui, "Updates", |ui| {
            ui.label(format!("Chewvy {CURRENT}"));
            ui.label(theme::secondary(summary(&state)));
            ui.add_space(theme::SPACING_BASE);
            let auto = self.settings.check_updates;
            if setting_row(
                ui,
                "Check for updates at start",
                "Ask GitHub for a newer release each time Chewvy opens",
                auto,
            ) {
                self.set_settings(|stored| stored.check_updates = !auto);
            }
            ui.horizontal_wrapped(|ui| {
                let offer = matches!(
                    state,
                    State::Available(_)
                        | State::Manual(..)
                        | State::Downloading { .. }
                        | State::Installing { .. }
                        | State::Ready(_)
                );
                if offer {
                    if chip(ui, "Open update", "See the update and install it") {
                        ask = Some(Ask::Open);
                    }
                } else if chip(ui, "Check now", "Ask GitHub for the latest release") {
                    ask = Some(Ask::Check);
                }
            });
        });
        self.answer_update(ui.ctx(), ask);
    }

    /// The notice over the browsing screens, and the prompt it opens.
    pub(super) fn update_overlays(&mut self, ctx: &egui::Context) {
        let state = self.updater.state().clone();
        let mut ask = None;
        if self.update_prompt {
            ask = prompt(ctx, &state, self.updater.offered());
        } else if !self.update_dismissed
            // Settings shows the same in its own section.
            && !matches!(self.current, Some(Entry::Settings))
            && notice_shown(&state, self.updater.offered())
        {
            ask = notice(ctx, &state);
        }
        self.answer_update(ctx, ask);
    }

    fn answer_update(&mut self, ctx: &egui::Context, ask: Option<Ask>) {
        match ask {
            Some(Ask::Open) => self.update_prompt = true,
            Some(Ask::Check) => {
                self.updater.check();
                self.update_prompt = true;
                self.update_dismissed = false;
            }
            Some(Ask::Install) => self.updater.install(),
            Some(Ask::Retry) => self.updater.retry(),
            Some(Ask::Restart) => {
                self.updater.restart();
                chrome::request_quit();
            }
            Some(Ask::Page(url)) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Some(Ask::Close) => self.update_prompt = false,
            Some(Ask::Dismiss) => self.update_dismissed = true,
            None => {}
        }
    }
}

/// Whether the notice has something to say: an offer, the work on it, a
/// restart to make, or an install that failed. A check that found nothing,
/// or could not reach GitHub, stays quiet.
fn notice_shown(state: &State, offered: Option<&Release>) -> bool {
    match state {
        State::Available(_)
        | State::Manual(..)
        | State::Downloading { .. }
        | State::Installing { .. }
        | State::Ready(_) => true,
        State::Failed(_) => offered.is_some(),
        State::Idle | State::Checking | State::Current => false,
    }
}

/// One line for Settings on where the updater is.
fn summary(state: &State) -> String {
    match state {
        State::Idle => "Not checked yet.".into(),
        State::Checking => "Checking for updates…".into(),
        State::Current => "This is the latest version.".into(),
        State::Available(release) | State::Manual(release, _) => {
            format!("Chewvy {} is available.", release.version)
        }
        State::Downloading { release, .. } | State::Installing { release, .. } => {
            format!("Updating to Chewvy {}…", release.version)
        }
        State::Ready(release) => format!(
            "Chewvy {} is installed. Restart to use it.",
            release.version
        ),
        State::Failed(why) => why.clone(),
    }
}

// ------------------------------------------------------------------ notice

/// The small card in the bottom-right corner: an icon, two lines, and a ×.
/// A click anywhere else on it opens the prompt.
fn notice(ctx: &egui::Context, state: &State) -> Option<Ask> {
    let (title, line, progress) = match state {
        State::Available(release) | State::Manual(release, _) => (
            "Update available".to_owned(),
            format!("Chewvy {} · click to update", release.version),
            None,
        ),
        State::Downloading { done, total, .. } => {
            let fraction = fraction(*done, *total);
            (
                "Downloading update".to_owned(),
                format!("{}%", (fraction * 100.0).round()),
                Some(Some(fraction)),
            )
        }
        State::Installing { step, fraction, .. } => (
            "Installing update".to_owned(),
            (*step).to_owned(),
            Some(*fraction),
        ),
        State::Ready(release) => (
            "Restart to update".to_owned(),
            format!("Chewvy {} is installed", release.version),
            None,
        ),
        State::Failed(_) => (
            "Update failed".to_owned(),
            "Click for details".to_owned(),
            None,
        ),
        _ => return None,
    };
    let icon = match state {
        State::Ready(_) => marks::icon::CHECK,
        _ => marks::icon::DOWNLOAD,
    };
    let mut ask = None;
    egui::Area::new(egui::Id::new("update-notice"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-24.0, -24.0))
        .show(ctx, |ui| {
            let frame = egui::Frame::new()
                .fill(theme::CARD)
                .corner_radius(theme::CARD_RADIUS)
                .inner_margin(Margin::symmetric(12, 10))
                .stroke(Stroke::new(theme::HAIRLINE, theme::CARD_HOVER))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::hover());
                        round_icon(ui, rect, icon, progress);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(theme::strong(title));
                            ui.label(theme::secondary(line).size(theme::CAPTION_SIZE));
                        });
                        ui.add_space(8.0);
                        let close = ui
                            .add(egui::Button::new("×").frame(false))
                            .on_hover_text("Hide until Chewvy next opens");
                        if close.clicked() {
                            ask = Some(Ask::Dismiss);
                        }
                    });
                });
            let card = ui
                .interact(frame.response.rect, ui.id().with("open"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            if card.clicked() && ask.is_none() {
                ask = Some(Ask::Open);
            }
        });
    if matches!(state, State::Downloading { .. } | State::Installing { .. }) {
        ctx.request_repaint();
    }
    ask
}

/// A round icon, with a progress ring round it while there is work under way:
/// `Some(Some(f))` fills it to `f`, `Some(None)` turns a short arc.
fn round_icon(ui: &egui::Ui, rect: Rect, icon: marks::Logo, progress: Option<Option<f32>>) {
    let painter = ui.painter();
    let center = rect.center();
    let radius = rect.width() / 2.0;
    painter.circle_filled(center, radius, theme::ACCENT_TINT);
    let size = rect.width() * 0.45;
    let left = center.x - marks::width_at(icon, size) / 2.0;
    marks::paint_tinted(painter, icon, left, center.y, size, theme::ACCENT);
    let Some(progress) = progress else {
        return;
    };
    let (start, sweep) = match progress {
        Some(fraction) => (-TAU / 4.0, TAU * fraction.clamp(0.0, 1.0)),
        None => (ui.input(|i| i.time) as f32 * 4.0, TAU * 0.25),
    };
    let points: Vec<egui::Pos2> = (0..=48)
        .map(|i| {
            let a = start + sweep * i as f32 / 48.0;
            center + vec2(a.cos(), a.sin()) * (radius - 1.5)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(3.0, theme::ACCENT)));
}

// ------------------------------------------------------------------ prompt

/// The update prompt, which follows the updater from the offer to the
/// restart. While work is under way it can only be hidden, not cancelled.
fn prompt(ctx: &egui::Context, state: &State, offered: Option<&Release>) -> Option<Ask> {
    let mut ask = None;
    let frame = egui::Frame::new()
        .fill(theme::CARD)
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(Margin::same(20))
        .stroke(Stroke::new(theme::HAIRLINE, theme::CARD_HOVER));
    let modal = egui::Modal::new(egui::Id::new("update-prompt"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(PROMPT_WIDTH);
            ui.spacing_mut().item_spacing.y = 10.0;
            ask = prompt_body(ui, state, offered);
        });
    if modal.should_close() && ask.is_none() {
        ask = Some(Ask::Close);
    }
    if matches!(
        state,
        State::Checking | State::Downloading { .. } | State::Installing { .. }
    ) {
        ctx.request_repaint();
    }
    ask
}

fn prompt_body(ui: &mut egui::Ui, state: &State, offered: Option<&Release>) -> Option<Ask> {
    match state {
        State::Idle | State::Checking => {
            heading(ui, "Checking for updates");
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::secondary("Asking GitHub for the latest release…"));
            });
            buttons(ui, None, Some(("Close", Ask::Close)))
        }
        State::Current => {
            heading(ui, "You are up to date");
            ui.label(theme::secondary(format!(
                "Chewvy {CURRENT} is the latest version."
            )));
            buttons(ui, None, Some(("Close", Ask::Close)))
        }
        State::Available(release) => {
            heading(ui, "Update available");
            ui.label(format!("Chewvy {CURRENT}  →  {}", release.version));
            notes(ui, release);
            buttons(
                ui,
                Some(("Update now", Ask::Install)),
                Some(("Later", Ask::Close)),
            )
        }
        State::Manual(release, why) => {
            heading(ui, "Update available");
            ui.label(format!("Chewvy {CURRENT}  →  {}", release.version));
            ui.label(theme::secondary(why.as_str()));
            notes(ui, release);
            let page = Updater::release_page(&release.version);
            buttons(
                ui,
                Some(("Open the release page", Ask::Page(page))),
                Some(("Close", Ask::Close)),
            )
        }
        State::Downloading {
            release,
            done,
            total,
        } => {
            heading(ui, &format!("Updating to Chewvy {}", release.version));
            let fraction = fraction(*done, *total);
            ui.label(theme::secondary(format!(
                "Downloading…  {} of {}",
                megabytes(*done),
                megabytes(*total)
            )));
            bar(ui, Some(fraction));
            buttons(ui, None, Some(("Hide", Ask::Close)))
        }
        State::Installing {
            release,
            step,
            fraction,
        } => {
            heading(ui, &format!("Updating to Chewvy {}", release.version));
            ui.label(theme::secondary(format!("Installing…  {step}")));
            bar(ui, *fraction);
            buttons(ui, None, Some(("Hide", Ask::Close)))
        }
        State::Ready(release) => {
            heading(ui, "Update installed");
            ui.label(theme::secondary(format!(
                "Restart Chewvy to start using version {}.",
                release.version
            )));
            bar(ui, Some(1.0));
            buttons(
                ui,
                Some(("Restart now", Ask::Restart)),
                Some(("Later", Ask::Close)),
            )
        }
        State::Failed(why) => {
            heading(ui, "The update did not finish");
            ui.label(egui::RichText::new(why.as_str()).color(theme::DANGER));
            let again = if offered.is_some() {
                Ask::Retry
            } else {
                Ask::Check
            };
            buttons(ui, Some(("Try again", again)), Some(("Close", Ask::Close)))
        }
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(theme::strong(text.to_owned()).size(theme::H2_SIZE));
}

/// The release's own notes, scrolling when they are long.
fn notes(ui: &mut egui::Ui, release: &Release) {
    if release.notes.is_empty() {
        return;
    }
    egui::ScrollArea::vertical()
        .max_height(NOTES_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for line in note_lines(&release.notes) {
                match line {
                    Note::Heading(text) => {
                        ui.add_space(4.0);
                        ui.label(theme::strong(text));
                    }
                    Note::Text(text) => {
                        ui.label(theme::secondary(text));
                    }
                }
            }
        });
}

/// One line of release notes, as the prompt sets it.
#[derive(Debug, PartialEq, Eq)]
enum Note {
    Heading(String),
    Text(String),
}

/// Release notes are written in Markdown for the release page. The prompt
/// keeps their shape (headings, bullets, paragraphs) and drops the marks:
/// `**`, backticks, and links reduced to their text. Blank lines go.
fn note_lines(notes: &str) -> Vec<Note> {
    let mut lines = Vec::new();
    for raw in notes.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let heading = line.trim_start_matches('#');
        if heading.len() < line.len() && heading.starts_with(' ') {
            lines.push(Note::Heading(inline(heading.trim())));
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            lines.push(Note::Text(format!("•  {}", inline(item))));
        } else {
            lines.push(Note::Text(inline(line)));
        }
    }
    lines
}

/// One line of Markdown without its inline marks.
fn inline(text: &str) -> String {
    let text = text.replace("**", "").replace('`', "");
    // `[words](address)` reads as `words`.
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(open) = rest.find('[') {
        let (before, after) = rest.split_at(open);
        let link = after
            .find("](")
            .and_then(|mid| after[mid..].find(')').map(|close| (mid, mid + close)));
        match link {
            Some((mid, close)) => {
                out.push_str(before);
                out.push_str(after.get(1..mid).unwrap_or_default());
                rest = after.get(close + 1..).unwrap_or_default();
            }
            None => {
                out.push_str(before);
                out.push('[');
                rest = after.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// The prompt's button row, right-aligned: the main action last.
fn buttons(
    ui: &mut egui::Ui,
    primary: Option<(&str, Ask)>,
    secondary: Option<(&str, Ask)>,
) -> Option<Ask> {
    let mut ask = None;
    ui.add_space(4.0);
    // A row of its own height: a layout that took the rest of the prompt
    // would keep it as tall as the tallest state it has shown.
    let row = vec2(ui.available_width(), 32.0);
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(row, layout, |ui| {
        if let Some((label, action)) = primary {
            let button = egui::Button::new(
                egui::RichText::new(label)
                    .color(theme::PRIMARY_TEXT)
                    .strong(),
            )
            .fill(theme::PRIMARY_BG)
            .corner_radius(theme::CARD_RADIUS)
            .min_size(vec2(0.0, 32.0));
            if ui
                .add(button)
                .on_hover_cursor(CursorIcon::PointingHand)
                .clicked()
            {
                ask = Some(action);
            }
        }
        if let Some((label, action)) = secondary {
            let button = egui::Button::new(label)
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(theme::HAIRLINE, theme::CARD_HOVER))
                .corner_radius(theme::CARD_RADIUS)
                .min_size(vec2(0.0, 32.0));
            if ui
                .add(button)
                .on_hover_cursor(CursorIcon::PointingHand)
                .clicked()
            {
                ask = ask.take().or(Some(action));
            }
        }
    });
    ask
}

/// A progress bar the prompt's width: filled to `fraction`, or a sliding
/// segment while a step cannot say how far it is.
fn bar(ui: &mut egui::Ui, fraction: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR), Sense::hover());
    let painter = ui.painter();
    let round = BAR / 2.0;
    painter.rect_filled(rect, round, theme::SURFACE);
    let fill = match fraction {
        Some(fraction) => {
            Rect::from_min_size(rect.min, vec2(rect.width() * fraction.clamp(0.0, 1.0), BAR))
        }
        None => {
            let t = (ui.input(|i| i.time) as f32 * 0.6).fract();
            let width = rect.width() * 0.3;
            let left = rect.left() - width + (rect.width() + width) * t;
            Rect::from_min_max(
                pos2(left.max(rect.left()), rect.top()),
                pos2((left + width).min(rect.right()), rect.bottom()),
            )
        }
    };
    if fill.width() > 0.0 {
        painter.rect_filled(fill, round, theme::ACCENT);
    }
}

fn fraction(done: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    (done as f32 / total as f32).clamp(0.0, 1.0)
}

/// `45.2 MB`.
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::{note_lines, notice, notice_shown, prompt, Note, State};
    use crate::update::Release;

    fn release() -> Release {
        Release {
            version: "0.3.0".to_owned(),
            notes: "Fixes.".to_owned(),
            package: None,
            sums: None,
        }
    }

    /// Every state the updater can be in, as the screens draw it.
    fn states() -> Vec<State> {
        let release = release();
        vec![
            State::Idle,
            State::Checking,
            State::Current,
            State::Available(release.clone()),
            State::Manual(release.clone(), "a development build".to_owned()),
            State::Downloading {
                release: release.clone(),
                done: 40,
                total: 100,
            },
            State::Installing {
                release: release.clone(),
                step: "Running the installer",
                fraction: None,
            },
            State::Installing {
                release: release.clone(),
                step: "Copying the new files",
                fraction: Some(0.5),
            },
            State::Ready(release),
            State::Failed("The download was cut short".to_owned()),
        ]
    }

    #[test]
    fn the_notice_and_the_prompt_draw_every_state_without_asking_anything() {
        let ctx = egui::Context::default();
        crate::ui::theme::apply(&ctx);
        let offer = release();
        for state in states() {
            let mut asked = (false, false);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let ctx = ui.ctx().clone();
                asked.0 = notice(&ctx, &state).is_some();
                asked.1 = prompt(&ctx, &state, Some(&offer)).is_some();
            });
            output.textures_delta.clear();
            assert_eq!(asked, (false, false), "{state:?}");
        }
    }

    #[test]
    fn the_notice_stays_quiet_unless_there_is_an_update_to_talk_about() {
        let offer = release();
        assert!(!notice_shown(&State::Current, None));
        assert!(!notice_shown(&State::Checking, None));
        // A check at start that could not reach GitHub says nothing.
        assert!(!notice_shown(&State::Failed("offline".to_owned()), None));
        assert!(notice_shown(
            &State::Failed("no room".to_owned()),
            Some(&offer)
        ));
        assert!(notice_shown(&State::Available(offer.clone()), Some(&offer)));
        assert!(notice_shown(&State::Ready(offer.clone()), Some(&offer)));
    }

    #[test]
    fn release_notes_keep_their_shape_and_lose_their_marks() {
        let notes = "Intro with **bold**.\n\n## What's new\n\n- **Watch together.** See [the docs](https://x/y) and `code`.\n* Second\n#hashtag stays";
        assert_eq!(
            note_lines(notes),
            vec![
                Note::Text("Intro with bold.".to_owned()),
                Note::Heading("What's new".to_owned()),
                Note::Text("•  Watch together. See the docs and code.".to_owned()),
                Note::Text("•  Second".to_owned()),
                Note::Text("#hashtag stays".to_owned()),
            ]
        );
    }
}
