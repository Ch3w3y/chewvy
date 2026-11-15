//! What the hero and the detail page draw over full-bleed art: the title art
//! or title, the rating and format marks, and the primary and frosted buttons.

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{pos2, vec2, CornerRadius, Galley, Id, Painter, Pos2, Rect, Response, Sense};
use plex_core::library::Item;

use crate::ui::marks::icon::{self, Icon, Size};
use crate::ui::{cards, controls, detail, glass, images, legible, marks, theme};

/// A button's height, padding, icon slot and the gap between its parts. The
/// Play button's icon is drawn larger than its slot and centred on it.
pub const BUTTON: [f32; 4] = [44.0, 20.0, icon::SIZE, 10.0];
/// The meta row's height: its text and its marks centre on it.
pub const META_HEIGHT: f32 = 22.0;
/// A format logo's height in a meta row.
const MARK_HEIGHT: f32 = 12.0;
/// A BBFC symbol's height in a meta row.
const RATING_HEIGHT: f32 = 20.0;
/// The primary button's progress track.
const TRACK: [f32; 2] = [64.0, 4.0];
/// A `clearLogo` request's size.
const LOGO_ART: [u32; 2] = [800, 280];
/// The strength of the soft copy under title art.
const ART_SHADOW: u8 = 150;
/// The strength of the line around a format tag.
const CHIP_LINE: u8 = 89;
/// The strength of the frosted button's wash while pointed at.
const SECONDARY_HOVER: u8 = 48;
/// The strength of the frosted button's wash while pressed.
const SECONDARY_PRESSED: u8 = 70;

/// A page heading: the server's title art, or the title set large.
pub enum Heading {
    Logo(egui::TextureId, egui::Vec2),
    Title(Arc<Galley>),
}

impl Heading {
    /// The title art at `logo` fitted in `logo_box`, or `title` wrapped in
    /// `width` while there is none.
    pub fn new(
        painter: &Painter,
        title: &str,
        logo: Option<&str>,
        artwork: &mut images::Artwork,
        logo_box: [f32; 2],
        width: f32,
    ) -> Self {
        let fitted = logo.and_then(|logo| {
            artwork
                .texture(Some(logo), LOGO_ART[0], LOGO_ART[1])
                .map(|(texture, tex)| {
                    (
                        texture,
                        cards::fit_size(tex, vec2(logo_box[0], logo_box[1])),
                    )
                })
        });
        match fitted {
            Some((texture, size)) => Heading::Logo(texture, size),
            None => {
                let mut job = LayoutJob::simple(
                    title.to_owned(),
                    theme::extrabold(44.0),
                    theme::palette().text,
                    width,
                );
                job.wrap.max_rows = 2;
                Heading::Title(painter.layout_job(job))
            }
        }
    }

    pub fn height(&self) -> f32 {
        self.size().y
    }

    pub fn size(&self) -> egui::Vec2 {
        match self {
            Heading::Logo(_, size) => *size,
            Heading::Title(galley) => galley.rect.size(),
        }
    }

    pub fn paint(self, painter: &Painter, origin: Pos2) {
        match self {
            Heading::Logo(texture, size) => {
                let rect = Rect::from_min_size(origin, size);
                painter.image(
                    texture,
                    rect.translate(vec2(0.0, 4.0)),
                    cards::FULL_UV,
                    theme::palette().scrim(ART_SHADOW),
                );
                painter.image(texture, rect, cards::FULL_UV, theme::UNTINTED);
            }
            Heading::Title(galley) => legible::on_art(painter, origin, galley),
        }
    }
}

/// `meta` text, then the rating (its BBFC symbol, or a chip) and the top
/// format marks, in [`META_HEIGHT`] rows from `origin` that wrap at `width`.
/// Answers the height drawn.
pub fn meta_row(
    painter: &Painter,
    origin: Pos2,
    width: f32,
    meta: Arc<Galley>,
    item: &Item,
) -> f32 {
    let mut cy = origin.y + META_HEIGHT / 2.0;
    let mut x = origin.x;
    if meta.rect.width() > 0.0 {
        x += meta.rect.width() + 12.0;
        legible::on_art(painter, pos2(origin.x, cy - meta.rect.height() / 2.0), meta);
    }
    let rating = item
        .content_rating
        .as_deref()
        .map(|rating| match marks::rating(rating) {
            Some(symbol) => (marks::Mark::Logo(symbol), RATING_HEIGHT),
            None => (marks::Mark::Text(marks::rating_text(rating)), MARK_HEIGHT),
        });
    let rest = top_marks(item).into_iter().map(|mark| (mark, MARK_HEIGHT));
    for (mark, height) in rating.into_iter().chain(rest) {
        let w = match &mark {
            marks::Mark::Logo(logo) => marks::width_at(*logo, height),
            marks::Mark::Text(text) => chip_width(painter, text),
        };
        if x > origin.x && x + w > origin.x + width {
            x = origin.x;
            cy += META_HEIGHT + 6.0;
        }
        match mark {
            marks::Mark::Logo(logo) => {
                marks::paint_tinted(painter, logo, x, cy, height, theme::UNTINTED);
            }
            marks::Mark::Text(text) => {
                chip(painter, x, cy, &text);
            }
        }
        x += w + 10.0;
    }
    cy + META_HEIGHT / 2.0 - origin.y
}

fn chip_galley(painter: &Painter, text: &str) -> Arc<Galley> {
    painter.layout_no_wrap(
        text.to_owned(),
        theme::extrabold(11.5),
        theme::palette().text,
    )
}

fn chip_width(painter: &Painter, text: &str) -> f32 {
    chip_galley(painter, text).rect.width() + 12.0
}

/// The marks a heading names: the picture size and the top picture and sound
/// formats. Codecs, and a base layer under a better one (HDR10 under Dolby
/// Vision, TrueHD under Atmos), are left to the version list.
#[must_use]
pub fn top_marks(item: &Item) -> Vec<marks::Mark> {
    let all: Vec<marks::Mark> = detail::badges(item)
        .iter()
        .flat_map(|label| marks::marks(label))
        .collect();
    let has = |name: &str| {
        all.iter()
            .any(|mark| matches!(mark, marks::Mark::Logo(logo) if logo.name == name))
    };
    let covered = |name: &str| match name {
        "hdr10" | "hdr10_plus" => has("dolby_vision"),
        "dolby_truehd" => has("dolby_atmos"),
        _ => false,
    };
    all.iter()
        .filter(|mark| match mark {
            marks::Mark::Text(text) => matches!(text.as_str(), "4K" | "1080p" | "720p" | "SD"),
            marks::Mark::Logo(logo) => !covered(logo.name),
        })
        .cloned()
        .collect()
}

/// A small outlined tag (`4K`, `TV-MA`); answers its width.
fn chip(painter: &Painter, left: f32, cy: f32, text: &str) -> f32 {
    let galley = chip_galley(painter, text);
    let rect = Rect::from_min_size(
        pos2(left, cy - 10.0),
        vec2(galley.rect.width() + 12.0, 20.0),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        (1.0, theme::palette().wash(CHIP_LINE)),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        rect.center() - galley.size() * 0.5,
        galley,
        theme::palette().text,
    );
    rect.width()
}

/// How far into the item the primary button's track says the viewer is, and
/// the time left it names.
pub struct Progress {
    pub fraction: f32,
    pub left: String,
}

fn sense(enabled: bool) -> Sense {
    if enabled {
        Sense::click()
    } else {
        Sense::hover()
    }
}

/// The near-white play button from `origin`: the play icon, `label`, then the
/// progress track and time left when there is a resume point.
pub fn primary(
    ui: &mut egui::Ui,
    origin: Pos2,
    id: Id,
    label: &str,
    progress: Option<Progress>,
    enabled: bool,
) -> Response {
    let painter = ui.painter().clone();
    let palette = theme::palette();
    let [height, pad, slot, gap] = BUTTON;
    let label = painter.layout_no_wrap(
        label.to_owned(),
        theme::semibold(15.0),
        theme::palette().primary_text,
    );
    let left = progress.as_ref().map(|p| {
        painter.layout_no_wrap(p.left.clone(), theme::medium(15.0), palette.primary_muted)
    });
    let mut width = pad + slot + gap + label.rect.width() + pad;
    width += progress.as_ref().map_or(0.0, |_| gap + TRACK[0]);
    width += left.as_ref().map_or(0.0, |g| gap + g.rect.width());
    let rect = Rect::from_min_size(origin, vec2(width, height));
    let response = cards::pointer(ui.interact(rect, id, sense(enabled)));
    let fill = if response.is_pointer_button_down_on() {
        theme::mix(palette.primary_hover, palette.shadow, 0.12)
    } else if response.hovered() {
        palette.primary_hover
    } else {
        palette.primary_bg
    };
    painter.rect_filled(rect, CornerRadius::same(theme::PILL_RADIUS_U8), fill);
    let cy = rect.center().y;
    let mut x = rect.min.x + pad;
    icon::paint(
        &painter,
        Icon::Play,
        Size::Primary,
        pos2(x + slot / 2.0, cy),
        theme::palette().primary_text,
    );
    x += slot + gap;
    let label_width = label.rect.width();
    painter.galley(
        pos2(x, cy - label.rect.height() / 2.0),
        label,
        theme::palette().primary_text,
    );
    x += label_width + gap;
    if let Some(progress) = progress {
        let track = Rect::from_min_size(pos2(x, cy - TRACK[1] / 2.0), vec2(TRACK[0], TRACK[1]));
        painter.rect_filled(track, CornerRadius::same(2), palette.primary_track());
        let done = vec2(TRACK[0] * progress.fraction.clamp(0.0, 1.0), TRACK[1]);
        painter.rect_filled(
            Rect::from_min_size(track.min, done),
            CornerRadius::same(2),
            theme::palette().primary_text,
        );
        x += TRACK[0] + gap;
    }
    if let Some(galley) = left {
        painter.galley(
            pos2(x, cy - galley.rect.height() / 2.0),
            galley,
            palette.primary_muted,
        );
    }
    controls::ring_when_focused(&painter, &response, rect, f32::from(theme::PILL_RADIUS_U8));
    response
}

/// A frosted button from `origin` with an optional icon before `label`.
pub fn secondary(
    ui: &mut egui::Ui,
    origin: Pos2,
    id: Id,
    glyph: Option<Icon>,
    label: &str,
    enabled: bool,
) -> Response {
    let painter = ui.painter().clone();
    let [height, pad, slot, gap] = BUTTON;
    let label = painter.layout_no_wrap(
        label.to_owned(),
        theme::semibold(15.0),
        theme::palette().text,
    );
    let lead = glyph.map_or(0.0, |_| slot + gap);
    let rect = Rect::from_min_size(origin, vec2(pad + lead + label.rect.width() + pad, height));
    let response = cards::pointer(ui.interact(rect, id, sense(enabled)));
    let tint = if response.is_pointer_button_down_on() {
        theme::palette().wash(SECONDARY_PRESSED)
    } else if response.hovered() {
        theme::palette().wash(SECONDARY_HOVER)
    } else {
        theme::palette().secondary_bg()
    };
    glass::panel(&painter, rect, f32::from(theme::PILL_RADIUS_U8), tint);
    let cy = rect.center().y;
    if let Some(glyph) = glyph {
        icon::paint(
            &painter,
            glyph,
            Size::Regular,
            pos2(rect.min.x + pad + slot / 2.0, cy),
            theme::palette().text,
        );
    }
    let x = rect.min.x + pad + lead;
    painter.galley(
        pos2(x, cy - label.rect.height() / 2.0),
        label,
        theme::palette().text,
    );
    controls::ring_when_focused(&painter, &response, rect, f32::from(theme::PILL_RADIUS_U8));
    response
}
