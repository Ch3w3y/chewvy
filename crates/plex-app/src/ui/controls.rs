//! The app's own controls, the only place under `ui` that builds an
//! interactive widget out of parts: buttons in three kinds, the text field,
//! the slider, the switch, a row that picks one option, a segmented choice,
//! the hairline rule, the working spinner, the dialog, and the focus ring
//! every focusable control draws.
//!
//! # The data shape
//!
//! A control is a [`Response`] and the [`Look`] it is in: pointed at,
//! pressed, holding the keyboard focus, or turned off. Every control paints
//! itself from that look in the theme's palette, so one state reads the same
//! everywhere: brightness and fill say "pointed at" and "pressed", the
//! [`focus_ring`] says "the keyboard is here", and nothing uses a hue.
//!
//! A button is one of three [`Kind`]s: the one main action a screen offers
//! ([`Kind::Primary`], a near-white plate), the actions beside it
//! ([`Kind::Secondary`], a lifted translucent plate), and the quiet text
//! actions under them ([`Kind::Quiet`], text that lifts and underlines when
//! pointed at).
//!
//! egui's stock widgets (`ui.button`, `Slider`, `selectable_label`,
//! `separator`, a framed `TextEdit`, `Spinner`, `Modal`'s own frame) do not
//! appear on any product screen; a test holds `ui` to that, so a new screen
//! reaches for these instead.

mod choice;
mod field;

#[cfg(test)]
mod tests;

pub use choice::{option_row, option_row_sized, segmented, slider, switch_row};
pub use field::Field;

use egui::{pos2, vec2, CornerRadius, CursorIcon, Painter, Rect, Response, Sense, Stroke};

use crate::ui::theme;

/// A button's and a field's height.
pub const HEIGHT: f32 = 40.0;
/// The room a button keeps on each side of its label.
const PAD_X: f32 = 18.0;
/// The widest a button that fills its row grows.
const WIDE: f32 = 420.0;
/// The focus ring's width, and how far outside the control it is drawn.
pub const RING: [f32; 2] = [2.0, 2.0];
/// The strengths of the wash a secondary button is filled with: at rest,
/// pointed at and pressed, and of its rim.
const SECONDARY: [u8; 4] = [31, 52, 20, 40];
/// How far a pressed primary button darkens towards the shadow ink.
const PRIMARY_PRESSED: f32 = 0.12;
/// The spinner's diameter.
const SPINNER: f32 = 18.0;

/// What state a control is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// The pointer is over it.
    pub hovered: bool,
    /// A pointer button is held down on it.
    pub pressed: bool,
    /// It holds the keyboard focus.
    pub focused: bool,
}

impl Look {
    /// The look `response` asks for.
    #[must_use]
    pub fn of(response: &Response) -> Self {
        Self {
            hovered: response.hovered(),
            pressed: response.is_pointer_button_down_on(),
            focused: response.has_focus(),
        }
    }
}

/// The ring round a control that holds the keyboard focus: drawn outside
/// `rect`, following its corners, in the theme's focus colour.
pub fn focus_ring(painter: &Painter, rect: Rect, radius: f32) {
    ringed(painter.ctx());
    let [width, gap] = RING;
    let outer = rect.expand(gap);
    painter.rect_stroke(
        outer,
        CornerRadius::same((radius + gap).clamp(0.0, 255.0) as u8),
        Stroke::new(width, theme::palette().focus),
        egui::StrokeKind::Outside,
    );
}

/// [`focus_ring`] round `rect` while `response` holds the keyboard focus.
pub fn ring_when_focused(painter: &Painter, response: &Response, rect: Rect, radius: f32) {
    if response.has_focus() {
        focus_ring(painter, rect, radius);
    }
}

/// [`focus_ring`] round a round control centred on `center`.
pub fn round_ring_when_focused(painter: &Painter, response: &Response, center: egui::Pos2, r: f32) {
    if response.has_focus() {
        ringed(painter.ctx());
        let [width, gap] = RING;
        painter.circle_stroke(
            center,
            r + gap + width * 0.5,
            Stroke::new(width, theme::palette().focus),
        );
    }
}

/// Where this frame's [`ringed`] marker is kept.
fn ringed_id() -> egui::Id {
    egui::Id::new("controls-focus-ringed")
}

/// Say that the control holding the focus drew its own ring this frame.
pub fn ringed(ctx: &egui::Context) {
    let frame = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(ringed_id(), frame));
}

/// The ring for a focused control that did not draw one itself this frame:
/// round its whole box, over everything, so wherever the Tab key lands the
/// focus shows. Called once, after every screen has drawn.
pub fn ring_the_rest(ctx: &egui::Context) {
    // As `Response::has_focus`: a window in the background shows no focus.
    let Some(id) = ctx
        .memory(|m| m.focused())
        .filter(|_| ctx.input(|i| i.focused))
    else {
        return;
    };
    let frame = ctx.cumulative_pass_nr();
    if ctx.data(|d| d.get_temp::<u64>(ringed_id())) == Some(frame) {
        return;
    }
    let Some(response) = ctx.read_response(id) else {
        return;
    };
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("focus-ring"),
    ));
    let painter = painter.with_clip_rect(response.interact_rect.expand(RING[0] + RING[1]));
    focus_ring(&painter, response.rect, f32::from(theme::CARD_RADIUS_U8));
}

/// Which of the three buttons a screen's action is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The one main action on a screen or a card.
    Primary,
    /// An action beside the main one.
    Secondary,
    /// A quiet text action under the others.
    Quiet,
}

/// A button about to be added: its kind and label, and what else it carries.
#[must_use]
pub struct Button<'a> {
    kind: Kind,
    label: &'a str,
    wide: bool,
    hint: Option<&'a str>,
}

impl<'a> Button<'a> {
    /// A `kind` button reading `label`.
    pub fn new(kind: Kind, label: &'a str) -> Self {
        Self {
            kind,
            label,
            wide: false,
            hint: None,
        }
    }

    /// As wide as the row allows, up to a reading width.
    pub fn wide(mut self) -> Self {
        self.wide = true;
        self
    }

    /// What a tooltip says about it.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Lay the button out where `ui` puts its next widget.
    pub fn show(self, ui: &mut egui::Ui) -> Response {
        let galley = label_galley(ui.painter(), self.kind, self.label);
        let pad = if self.kind == Kind::Quiet { 6.0 } else { PAD_X };
        let natural = pad + galley.size().x + pad;
        let width = if self.wide {
            ui.available_width().min(WIDE).max(natural)
        } else {
            natural
        };
        let height = if self.kind == Kind::Quiet {
            32.0
        } else {
            HEIGHT
        };
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        self.paint(ui.painter(), rect, &response, galley);
        let response = response.on_hover_cursor(CursorIcon::PointingHand);
        match self.hint {
            Some(hint) => response.on_hover_text(hint),
            None => response,
        }
    }

    fn paint(
        &self,
        painter: &Painter,
        rect: Rect,
        response: &Response,
        galley: std::sync::Arc<egui::Galley>,
    ) {
        let look = Look::of(response);
        let palette = theme::palette();
        let radius = f32::from(theme::PILL_RADIUS_U8);
        let ink = match self.kind {
            Kind::Primary => {
                let fill = if look.pressed {
                    theme::mix(palette.primary_hover, palette.shadow, PRIMARY_PRESSED)
                } else if look.hovered {
                    palette.primary_hover
                } else {
                    palette.primary_bg
                };
                painter.rect_filled(rect, radius, fill);
                palette.primary_text
            }
            Kind::Secondary => {
                let [rest, hover, pressed, rim] = SECONDARY;
                let wash = if look.pressed {
                    pressed
                } else if look.hovered {
                    hover
                } else {
                    rest
                };
                painter.rect_filled(rect, radius, palette.wash(wash));
                painter.rect_stroke(
                    rect,
                    radius,
                    Stroke::new(theme::HAIRLINE, palette.wash(rim)),
                    egui::StrokeKind::Inside,
                );
                palette.text
            }
            Kind::Quiet if look.pressed => palette.muted,
            Kind::Quiet if look.hovered || look.focused => palette.text,
            Kind::Quiet => palette.secondary,
        };
        let text = Rect::from_center_size(rect.center(), galley.size());
        painter.galley(text.min, galley, ink);
        if self.kind == Kind::Quiet && look.hovered {
            let y = text.bottom() + 1.0;
            painter.line_segment(
                [pos2(text.left(), y), pos2(text.right(), y)],
                Stroke::new(theme::HAIRLINE, ink),
            );
        }
        if look.focused {
            focus_ring(painter, rect, radius);
        }
    }
}

fn label_galley(painter: &Painter, kind: Kind, label: &str) -> std::sync::Arc<egui::Galley> {
    let font = match kind {
        Kind::Quiet => theme::medium(theme::BODY_SIZE - 1.0),
        Kind::Primary | Kind::Secondary => theme::semibold(theme::BODY_SIZE),
    };
    painter.layout_no_wrap(label.to_owned(), font, egui::Color32::PLACEHOLDER)
}

/// A `kind` button reading `label`, laid out where `ui` puts its next widget.
pub fn button(ui: &mut egui::Ui, kind: Kind, label: &str) -> Response {
    Button::new(kind, label).show(ui)
}

/// A hairline across the row, between groups of controls.
pub fn rule(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    let y = ui.painter().round_to_pixel_center(rect.center().y);
    ui.painter().line_segment(
        [pos2(rect.left(), y), pos2(rect.right(), y)],
        Stroke::new(theme::HAIRLINE, theme::palette().rule()),
    );
}

/// The working mark: a short arc turning on a faint ring, placed where `ui`
/// puts its next widget (so a centred column centres it).
pub fn spinner(ui: &mut egui::Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(SPINNER, SPINNER), Sense::hover());
    paint_spinner(ui, rect.center());
    response
}

fn paint_spinner(ui: &egui::Ui, center: egui::Pos2) {
    let painter = ui.painter();
    let radius = SPINNER / 2.0 - 1.5;
    painter.circle_stroke(center, radius, Stroke::new(2.0, theme::palette().wash(36)));
    let start = ui.input(|i| i.time) as f32 * 5.0;
    let points: Vec<egui::Pos2> = (0..=24)
        .map(|i| {
            let a = start + std::f32::consts::TAU * 0.28 * i as f32 / 24.0;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new(2.0, theme::palette().accent),
    ));
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
}

/// [`spinner`] with what the app is waiting for beside it, on one row
/// aligned the way `ui` aligns its widgets: centred on a centred card, at
/// the left edge of a section.
pub fn working(ui: &mut egui::Ui, what: &str) {
    let gap = 10.0;
    let text =
        ui.painter()
            .layout_no_wrap(what.to_owned(), theme::meta(), theme::palette().secondary);
    let (row, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), SPINNER.max(text.size().y)),
        Sense::hover(),
    );
    let width = SPINNER + gap + text.size().x;
    let left = match ui.layout().horizontal_align() {
        egui::Align::Center => row.center().x - width / 2.0,
        egui::Align::Max => row.max.x - width,
        egui::Align::Min => row.min.x,
    };
    paint_spinner(ui, pos2(left + SPINNER / 2.0, row.center().y));
    let top = row.center().y - text.size().y / 2.0;
    ui.painter().galley(
        pos2(left + SPINNER + gap, top),
        text,
        theme::palette().secondary,
    );
}

/// A dialog over the page: the page dims, and `add` is drawn on a glass card
/// `width` wide in the window's middle. Answers what `add` did, and whether a
/// click outside the card or Escape asked to close it.
pub fn dialog<R>(
    ctx: &egui::Context,
    id: egui::Id,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    let modal = egui::Modal::new(id)
        .frame(egui::Frame::NONE)
        .backdrop_color(theme::palette().scrim(150))
        .show(ctx, |ui| {
            let slot = crate::ui::glass::reserve(ui.painter());
            let inner = egui::Frame::NONE
                .inner_margin(egui::Margin::same(24))
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    add(ui)
                });
            crate::ui::glass::fill(
                ui.painter(),
                slot,
                inner.response.rect,
                DIALOG_RADIUS,
                theme::current().glass.panel,
            );
            inner.inner
        });
    let close = modal.should_close();
    (modal.inner, close)
}

/// A dialog's corner radius.
const DIALOG_RADIUS: f32 = 16.0;

/// A dialog's or a card's row of buttons, right-aligned with the main action
/// last, so it sits under the reader's eye where the text ends.
pub fn actions<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(6.0);
    let row = vec2(ui.available_width(), HEIGHT);
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(row, layout, |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        add(ui)
    })
    .inner
}
