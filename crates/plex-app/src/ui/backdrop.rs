//! The aurora photograph every screen sits on (`assets/backdrop`, credited in
//! Settings › About). Sign-in screens show the light-blur copy; browsing shows
//! the heavier, dimmer copy so posters and text stay in front of it.
//!
//! It is painted on egui's background layer, so whatever a screen draws later,
//! and every glass panel's blur, sits over it whatever order the panels ran in.
//! The panels share that layer, so a slot is kept just above the photograph
//! for a page's full-bleed art ([`crate::ui::bleed`]), which a page decides on
//! only after the sidebar has drawn.

use std::sync::Mutex;

use egui::{pos2, vec2, Color32, LayerId, Rect, TextureHandle};

use crate::ui::theme;

const SHARP: &[u8] = include_bytes!("../../assets/backdrop/aurora.jpg");
const SOFT: &[u8] = include_bytes!("../../assets/backdrop/aurora-soft.jpg");

/// Which copy of the photograph a screen shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Sign-in, first run and the server picker: the scene itself.
    Sharp,
    /// Browsing screens: blurred and dimmed behind the content.
    Soft,
}

static TEXTURES: Mutex<[Option<TextureHandle>; 2]> = Mutex::new([None, None]);

fn texture(ctx: &egui::Context, kind: Kind) -> Option<TextureHandle> {
    let mut slots = TEXTURES.lock().ok()?;
    let [sharp, soft] = &mut *slots;
    let (slot, bytes, name) = match kind {
        Kind::Sharp => (sharp, SHARP, "backdrop-sharp"),
        Kind::Soft => (soft, SOFT, "backdrop-soft"),
    };
    if slot.is_none() {
        let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).ok()?;
        let rgba = decoded.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        *slot = Some(ctx.load_texture(name, image, egui::TextureOptions::LINEAR));
    }
    slot.clone()
}

/// Drop the sign-in copy once browsing starts; it is only shown before then.
pub fn release_sharp() {
    if let Ok(mut slots) = TEXTURES.lock() {
        let [sharp, _] = &mut *slots;
        *sharp = None;
    }
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

/// Paint the photograph over the whole of `rect`, then `dim` (0..1) of the
/// window colour over it.
pub fn paint(ctx: &egui::Context, rect: Rect, kind: Kind, dim: f32) {
    let painter = ctx.layer_painter(LayerId::background());
    painter.rect_filled(rect, 0.0, theme::WINDOW);
    if let Some(texture) = texture(ctx, kind) {
        let focus = match kind {
            Kind::Sharp => vec2(0.5, 0.4),
            Kind::Soft => vec2(0.5, 0.7),
        };
        let uv = cover_uv(texture.size_vec2(), rect.size(), focus);
        painter.image(texture.id(), rect, uv, Color32::WHITE);
    }
    if dim > 0.0 {
        painter.rect_filled(rect, 0.0, theme::WINDOW.gamma_multiply(dim.clamp(0.0, 1.0)));
    }
    let slot = painter.add(egui::Shape::Noop);
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(slot_id(), (pass, slot)));
}

fn slot_id() -> egui::Id {
    egui::Id::new("backdrop-art-slot")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_copies_decode_at_full_hd_or_more() {
        for bytes in [SHARP, SOFT] {
            let image = image::load_from_memory(bytes).unwrap_or_else(|e| panic!("backdrop: {e}"));
            assert!(image.width() >= 1920 && image.height() >= 1080);
        }
    }

    #[test]
    fn cover_keeps_the_aspect_and_stays_inside_the_image() {
        let uv = cover_uv(vec2(2048.0, 1152.0), vec2(1280.0, 1000.0), vec2(0.5, 0.7));
        assert!(uv.min.x >= 0.0 && uv.max.x <= 1.0 + 1e-6);
        assert!(uv.min.y >= 0.0 && uv.max.y <= 1.0 + 1e-6);
        let drawn = (uv.width() * 2048.0) / (uv.height() * 1152.0);
        assert!((drawn - 1.28).abs() < 0.01);
    }
}
