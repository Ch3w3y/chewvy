//! The text field: a rounded well holding egui's text editing with its own
//! frame turned off, so the well, its edge and its focus are the app's.

use egui::{pos2, vec2, Rect, Response, Sense, Stroke};

use crate::ui::marks::icon::{self, Icon, Size};
use crate::ui::theme;

/// The room a field keeps between its edge and its text.
const PAD_X: f32 = 12.0;
/// The room a leading icon takes, its gap included.
const LEAD: f32 = icon::SIZE + 10.0;
/// The strengths of the wash a field's well is filled with: at rest, pointed
/// at, and holding the focus; and of its edge at rest and pointed at.
const WELL: [u8; 3] = [14, 22, 26];
const EDGE: [u8; 2] = [34, 60];

/// A text field about to be added.
#[must_use]
pub struct Field<'a> {
    text: &'a mut String,
    hint: &'a str,
    id_salt: &'a str,
    leading: Option<Icon>,
    reveal: Option<&'a mut bool>,
    height: f32,
    selected: bool,
}

/// What a field answered this frame.
pub struct Answer {
    /// The text editor's own response: changed, focus, lost focus.
    pub response: Response,
    /// Enter committed the text.
    pub submitted: bool,
}

impl<'a> Field<'a> {
    /// A field editing `text`, saying `hint` while it is empty. `id_salt`
    /// keeps its caret and focus apart from every other field on screen.
    pub fn new(text: &'a mut String, hint: &'a str, id_salt: &'a str) -> Self {
        Self {
            text,
            hint,
            id_salt,
            leading: None,
            reveal: None,
            height: super::HEIGHT,
            selected: false,
        }
    }

    /// An icon at the field's left, before the text.
    pub fn leading(mut self, icon: Icon) -> Self {
        self.leading = Some(icon);
        self
    }

    /// A password field: dots unless `shown`, with a Show / Hide action at its
    /// right edge that flips `shown`.
    pub fn password(mut self, shown: &'a mut bool) -> Self {
        self.reveal = Some(shown);
        self
    }

    /// A field `height` tall instead of a button's height.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Drawn lit, as the screen in view (the rail's search field on Search).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Lay the field out across the row `ui` gives it.
    pub fn show(self, ui: &mut egui::Ui) -> Answer {
        let Self {
            text,
            hint,
            id_salt,
            leading,
            reveal,
            height,
            selected,
        } = self;
        let (well, well_response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::CLICK);
        let id = ui.id().with(("field", id_salt));
        let painter = ui.painter().clone();
        let lead = leading.map_or(PAD_X, |_| LEAD + PAD_X - 4.0);
        let toggle = reveal.as_ref().map(|shown| {
            let label = if **shown { "Hide" } else { "Show" };
            let galley = painter.layout_no_wrap(
                label.to_owned(),
                theme::semibold(theme::CAPTION_SIZE),
                egui::Color32::PLACEHOLDER,
            );
            let w = galley.size().x + 2.0 * PAD_X;
            (
                Rect::from_min_max(pos2(well.max.x - w, well.min.y), well.max),
                galley,
            )
        });
        let trail = toggle.as_ref().map_or(PAD_X, |(rect, _)| rect.width());
        let inner = Rect::from_min_max(
            pos2(well.min.x + lead, well.min.y),
            pos2(well.max.x - trail, well.max.y),
        );
        let masked = reveal.as_ref().is_some_and(|shown| !**shown);
        // The well's fill and edge go under the text, which `place` paints.
        let [well_slot, edge_slot] = [
            painter.add(egui::Shape::Noop),
            painter.add(egui::Shape::Noop),
        ];
        let response = ui.place(
            inner,
            egui::TextEdit::singleline(text)
                .id(id)
                .hint_text(egui::RichText::new(hint).color(theme::palette().muted))
                .font(theme::body())
                .text_color(theme::palette().text)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .vertical_align(egui::Align::Center)
                .password(masked)
                .desired_width(inner.width()),
        );
        if well_response.clicked() {
            response.request_focus();
        }
        let focused = response.has_focus();
        let hovered = well_response.hovered() || response.hovered();
        let palette = theme::palette();
        let [rest, hover, active] = WELL;
        let fill = if focused || selected {
            active
        } else if hovered {
            hover
        } else {
            rest
        };
        let radius = f32::from(theme::PILL_RADIUS_U8);
        let edge = if focused || selected {
            palette.focus.gamma_multiply(0.55)
        } else {
            palette.wash(if hovered { EDGE[1] } else { EDGE[0] })
        };
        painter.set(
            well_slot,
            egui::epaint::RectShape::filled(well, radius, palette.wash(fill)),
        );
        painter.set(
            edge_slot,
            egui::epaint::RectShape::stroke(
                well,
                radius,
                Stroke::new(theme::HAIRLINE, edge),
                egui::StrokeKind::Inside,
            ),
        );
        if let Some(glyph) = leading {
            let ink = if focused || selected {
                palette.text
            } else {
                palette.muted
            };
            icon::paint(
                &painter,
                glyph,
                Size::Regular,
                pos2(well.min.x + PAD_X - 2.0 + icon::SIZE / 2.0, well.center().y),
                ink,
            );
        }
        if focused {
            super::focus_ring(&painter, well, radius);
        }
        if let (Some(shown), Some((rect, galley))) = (reveal, toggle) {
            let flip = ui
                .interact(rect, id.with("reveal"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(if *shown {
                    "Hide the password"
                } else {
                    "Show the password"
                });
            let ink = if flip.hovered() || flip.has_focus() {
                palette.text
            } else {
                palette.secondary
            };
            painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
            super::ring_when_focused(&painter, &flip, rect.shrink(6.0), 6.0);
            if flip.clicked() {
                *shown = !*shown;
            }
        }
        let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        Answer {
            response,
            submitted,
        }
    }
}
