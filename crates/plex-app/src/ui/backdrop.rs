//! The photograph behind the glass: one of the bundled, freely licensed
//! photographs (`assets/backdrop`, credited in Settings › About) or a
//! backdrop from the user's own servers. Sign-in screens show the scene
//! itself; browsing shows a blurred copy so posters and text stay in front.
//!
//! # Data shape
//!
//! * [`Photo`] is one bundled photograph with its credit, and [`PHOTOS`]
//!   lists them in the theme picker's order; `grade.py` writes the table with
//!   the files.
//! * [`Showing`] is what the background is: a photo by its place in
//!   [`PHOTOS`], or a server backdrop by its artwork address.
//! * [`Stage`] paints it. It keeps what is on screen until the next picture
//!   has loaded, then cross-fades, and says what the picture needs over it
//!   through [`theme::set_backdrop`]: the dims are measured from each
//!   picture's own pixels ([`legibility`]), so a brighter one is dimmed more.
//!
//! It is painted on egui's background layer, so whatever a screen draws later,
//! and every glass panel's blur, sits over it whatever order the panels ran in.
//! The panels share that layer, so a slot is kept just above the photograph
//! for a page's full-bleed art ([`crate::ui::bleed`]), which a page decides on
//! only after the sidebar has drawn.

pub mod choice;
pub mod decode;
pub mod legibility;
mod photos;
mod stage;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests may unwrap; product code keeps these lints on"
)]
mod tests;

use egui::{pos2, vec2, LayerId, Rect};

use crate::theme_art::ThemeId;

pub use photos::PHOTOS;
pub use stage::Stage;

/// One bundled photograph.
pub struct Photo {
    /// The file's name without `.jpg`.
    pub id: &'static str,
    /// The theme it belongs to.
    pub theme: ThemeId,
    /// The graded JPEG.
    pub bytes: &'static [u8],
    /// Its size in pixels.
    pub size: [u32; 2],
    /// Which part stays when a window is not 16:9 (0..1 in each axis).
    pub focus: [f32; 2],
    /// What the photographer or the archive called it.
    pub title: &'static str,
    /// Who made it.
    pub author: &'static str,
    /// Its licence's short name.
    pub licence: &'static str,
    /// The licence's deed, empty for the public domain.
    pub licence_url: &'static str,
    /// Its page on Wikimedia Commons.
    pub page: &'static str,
}

impl Photo {
    /// The credit line: `“Eilean Donan castle” by Eusebius, CC BY 3.0`.
    #[must_use]
    pub fn credit(&self) -> String {
        let licence = match self.licence {
            "Public domain" => "public domain",
            other => other,
        };
        format!("“{}” by {}, {licence}", self.title, self.author)
    }
}

/// The places in [`PHOTOS`] of `theme`'s photographs.
pub fn photos_of(theme: ThemeId) -> impl Iterator<Item = usize> {
    PHOTOS
        .iter()
        .enumerate()
        .filter(move |(_, photo)| photo.theme == theme)
        .map(|(index, _)| index)
}

/// What the background shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Showing {
    /// A bundled photograph, by its place in [`PHOTOS`].
    Photo(usize),
    /// A server backdrop, by the artwork address the loader fetches.
    Library(String),
}

/// Which copy of the photograph a screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Sign-in, first run and the server picker: the scene itself.
    Sharp,
    /// Browsing screens: blurred and dimmed behind the content.
    Soft,
}

/// The part of a `tex`-sized image that covers `target` without stretching,
/// anchored at `focus` (0..1 in each axis: which part of the photo stays).
#[must_use]
pub fn cover_uv(tex: egui::Vec2, target: egui::Vec2, focus: egui::Vec2) -> Rect {
    let image_aspect = tex.x / tex.y.max(1.0);
    let target_aspect = target.x / target.y.max(1.0);
    let (w, h) = if target_aspect > image_aspect {
        (1.0, image_aspect / target_aspect)
    } else {
        (target_aspect / image_aspect, 1.0)
    };
    let min = pos2((1.0 - w) * focus.x, (1.0 - h) * focus.y);
    Rect::from_min_size(min, vec2(w, h))
}

fn slot_id() -> egui::Id {
    egui::Id::new("backdrop-art-slot")
}

/// Keep this pass's slot above the photograph for [`paint_above`].
fn keep_slot(ctx: &egui::Context) {
    let painter = ctx.layer_painter(LayerId::background());
    let slot = painter.add(egui::Shape::Noop);
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(slot_id(), (pass, slot)));
}

/// Put `shape` in this pass's slot above the photograph, under every panel;
/// on a pass that painted no backdrop it is simply added.
pub fn paint_above(ctx: &egui::Context, shape: egui::Shape) {
    let painter = ctx.layer_painter(LayerId::background());
    let pass = ctx.cumulative_pass_nr();
    let slot = ctx.data(|data| data.get_temp::<(u64, egui::layers::ShapeIdx)>(slot_id()));
    match slot {
        Some((at, slot)) if at == pass => painter.set(slot, shape),
        _ => {
            painter.add(shape);
        }
    }
}
