//! Text with no surface of its own under it: over an item's artwork, or
//! straight on the photograph behind every screen. Each line is drawn over
//! the dark copies of itself the theme names ([`theme::Theme::art_shadow`],
//! [`theme::Backdrop::text_shadow`]), so it holds where the picture under it
//! is bright.

use std::sync::Arc;

use egui::text_selection::LabelSelectionState;
use egui::{vec2, Color32, Galley, Painter, Pos2, Response, Shape, Stroke, WidgetText};

use crate::ui::marks::icon::{self, Icon, Size};
use crate::ui::theme::{self, TextShadow};

/// How many copies stand in for one blurred shadow, spread round a circle.
const RING: usize = 8;
/// How many of those copies cover a point just outside a glyph's stroke: the
/// ones offset towards it.
const OVERLAP: f32 = 3.0;

/// `galley` at `pos` over an item's artwork, in the palette's text colour
/// where the galley names none.
pub fn on_art(painter: &Painter, pos: Pos2, galley: Arc<Galley>) {
    let theme = theme::current();
    paint(painter, pos, &galley, &theme.art_shadow);
    painter.galley(pos, galley, theme.palette.text);
}

/// `glyph` centred on `center` over an item's artwork in `tint`, over the
/// same dark copies a line of text there gets.
pub fn icon_on_art(painter: &Painter, glyph: Icon, center: Pos2, tint: Color32) {
    for (offset, alpha) in copies(&theme::current().art_shadow) {
        let ink = theme::palette().scrim(alpha);
        icon::paint(painter, glyph, Size::Regular, center + offset, ink);
    }
    icon::paint(painter, glyph, Size::Regular, center, tint);
}

/// `galley` at `pos` straight on the backdrop; `fallback` colours what the
/// galley left to the painter.
pub fn on_backdrop(painter: &Painter, pos: Pos2, galley: Arc<Galley>, fallback: Color32) {
    paint(
        painter,
        pos,
        &galley,
        &theme::current().backdrop.text_shadow,
    );
    painter.galley(pos, galley, fallback);
}

/// A label straight on the backdrop: laid out, selectable and coloured as
/// `ui.label` would, over the theme's shadows.
pub fn label(ui: &mut egui::Ui, text: impl Into<WidgetText>) -> Response {
    let shadows = theme::current().backdrop.text_shadow;
    let (pos, galley, response) = egui::Label::new(text).layout_in_ui(ui);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), galley.text())
    });
    if !ui.is_rect_visible(response.rect) {
        return response;
    }
    paint(ui.painter(), pos, &galley, &shadows);
    let color = ui.visuals().text_color();
    if ui.style().interaction.selectable_labels {
        LabelSelectionState::label_text_selection(ui, &response, pos, galley, color, Stroke::NONE);
    } else {
        ui.painter().galley(pos, galley, color);
    }
    response
}

fn paint(painter: &Painter, pos: Pos2, galley: &Arc<Galley>, shadows: &[TextShadow]) {
    let shapes: Vec<Shape> = copies(shadows)
        .into_iter()
        .map(|(offset, alpha)| {
            let ink = theme::palette().scrim(alpha);
            egui::epaint::TextShape::new(pos + offset, galley.clone(), ink)
                .with_override_text_color(ink)
                .into()
        })
        .collect();
    if !shapes.is_empty() {
        painter.add(Shape::Vec(shapes));
    }
}

/// Where each dark copy of a line goes, and how strong it is. A hard shadow
/// is one copy; a blurred one is [`RING`] weaker copies round a circle of its
/// blur radius, which reach its strength just outside each stroke, where
/// [`OVERLAP`] of them lie over one another.
fn copies(shadows: &[TextShadow]) -> Vec<(egui::Vec2, u8)> {
    let mut out = Vec::new();
    for shadow in shadows.iter().filter(|shadow| shadow.alpha > 0) {
        let down = vec2(0.0, shadow.drop);
        if shadow.blur <= 0.0 {
            out.push((down, shadow.alpha));
            continue;
        }
        let whole = f32::from(shadow.alpha) / 255.0;
        let each = 1.0 - (1.0 - whole).powf(1.0 / OVERLAP);
        let alpha = (each * 255.0).round().clamp(1.0, 255.0) as u8;
        for step in 0..RING {
            let angle = std::f32::consts::TAU * step as f32 / RING as f32;
            out.push((down + shadow.blur * vec2(angle.cos(), angle.sin()), alpha));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_with_no_shadows_draws_no_copies() {
        assert!(copies(&[TextShadow::NONE; 3]).is_empty());
    }

    #[test]
    fn a_hard_shadow_is_one_copy_at_its_drop() {
        let hard = TextShadow {
            drop: 2.0,
            blur: 0.0,
            alpha: 70,
        };
        assert_eq!(copies(&[hard]), vec![(vec2(0.0, 2.0), 70)]);
    }

    #[test]
    fn a_blurred_shadow_rings_the_line_and_reaches_its_strength_beside_a_stroke() {
        let soft = TextShadow {
            drop: 1.0,
            blur: 2.0,
            alpha: 153,
        };
        let ring = copies(&[soft]);
        assert_eq!(ring.len(), RING);
        for (offset, _) in &ring {
            assert!(((*offset - vec2(0.0, 1.0)).length() - 2.0).abs() < 1e-4);
        }
        let left: f32 = ring
            .iter()
            .take(OVERLAP as usize)
            .map(|(_, alpha)| 1.0 - f32::from(*alpha) / 255.0)
            .product();
        let reached = (1.0 - left) * 255.0;
        assert!((reached - 153.0).abs() < 3.0, "{reached}");
    }
}
