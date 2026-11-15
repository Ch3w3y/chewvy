//! The theme picker: Automatic and every theme as a small tile of its
//! photograph, and the pinned title's row.

use egui::epaint::RectShape;
use egui::{pos2, vec2, CornerRadius, CursorIcon, Rect, Sense, Stroke, StrokeKind};

use crate::theme_art::ThemeId;
use crate::ui::backdrop::{photos_of, Stage};
use crate::ui::controls::{self, Button, Kind, Look};
use crate::ui::settings::Background;
use crate::ui::theme;

/// A tile's picture, 16:9.
const PICTURE: [f32; 2] = [152.0, 86.0];
/// The label under the picture.
const LABEL: f32 = 24.0;
/// Between tiles.
const GAP: f32 = 12.0;
const RADIUS: f32 = 8.0;
/// Wash over a picture: pointed at, and over every tile while a title is
/// pinned, since no theme shows then.
const HOVER_WASH: u8 = 40;
const PINNED_SHADE: f32 = 0.45;

/// Draw the tiles and answer the choice a click made.
pub(super) fn show(
    ui: &mut egui::Ui,
    stage: &mut Stage,
    chosen: Background,
    seasonal: ThemeId,
    pinned: bool,
) -> Option<Background> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        let automatic = format!("Automatic · {}", seasonal.name());
        let choices = std::iter::once((Background::Automatic, seasonal, automatic)).chain(
            ThemeId::ALL
                .into_iter()
                .map(|theme| (Background::Theme(theme), theme, theme.name().to_owned())),
        );
        for (background, theme, label) in choices {
            let selected = !pinned && chosen == background;
            if tile(ui, stage, theme, &label, selected, pinned) && !selected {
                picked = Some(background);
            }
        }
    });
    picked
}

/// One tile; answers whether it was clicked.
fn tile(
    ui: &mut egui::Ui,
    stage: &mut Stage,
    theme: ThemeId,
    label: &str,
    selected: bool,
    pinned: bool,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(PICTURE[0], PICTURE[1] + LABEL), Sense::click());
    let response = response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!("Show the {} background", theme.name()));
    let look = Look::of(&response);
    let palette = theme::palette();
    let picture = Rect::from_min_size(rect.min, vec2(PICTURE[0], PICTURE[1]));
    let painter = ui.painter();
    let texture = photos_of(theme)
        .next()
        .and_then(|index| stage.thumb(ui.ctx(), index));
    match texture {
        Some(texture) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.add(
                RectShape::filled(picture, CornerRadius::from(RADIUS), theme::UNTINTED)
                    .with_texture(texture, uv),
            );
        }
        None => {
            painter.rect_filled(picture, RADIUS, palette.card);
        }
    }
    if pinned {
        painter.rect_filled(picture, RADIUS, palette.window.gamma_multiply(PINNED_SHADE));
    } else if look.hovered {
        painter.rect_filled(picture, RADIUS, palette.wash(HOVER_WASH));
    }
    let rim = if selected {
        Stroke::new(2.0, palette.text)
    } else {
        Stroke::new(theme::HAIRLINE, palette.outline)
    };
    painter.rect_stroke(picture, RADIUS, rim, StrokeKind::Inside);
    let ink = if selected {
        palette.text
    } else {
        palette.secondary
    };
    let font = if selected {
        theme::semibold(theme::CAPTION_SIZE)
    } else {
        theme::medium(theme::CAPTION_SIZE)
    };
    painter.text(
        pos2(picture.left() + 2.0, picture.bottom() + 6.0),
        egui::Align2::LEFT_TOP,
        label,
        font,
        ink,
    );
    controls::ring_when_focused(painter, &response, picture, RADIUS);
    response.clicked()
}

/// `Pinned: Dune (2021) from Living Room` with its Unpin button; answers
/// whether Unpin was clicked.
pub(super) fn pinned_row(ui: &mut egui::Ui, credit: &str) -> bool {
    ui.horizontal(|ui| {
        ui.label(theme::strong(format!("Pinned: {credit}")));
        ui.add_space(theme::SPACING_BASE);
        Button::new(Kind::Secondary, "Unpin")
            .hint("Go back to the theme chosen above")
            .show(ui)
            .clicked()
    })
    .inner
}
