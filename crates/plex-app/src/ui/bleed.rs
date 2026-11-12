//! Full-bleed art for the Home hero and the detail page: the item's backdrop
//! across the whole window (under the sidebar, whose glass blurs it), faded
//! out by a mask, with a left-hand scrim under the text.
//!
//! Painted into the backdrop's slot just above the photograph, so the page's
//! cards and the sidebar's glass both sit over it.

use egui::epaint::{Mesh, Vertex};
use egui::{pos2, vec2, Color32, Pos2, Rect, Shape, TextureId};

use crate::ui::{backdrop, theme};

/// The mask's centre, as fractions of the art's width and height: the part of
/// the picture that stays at full strength sits up and to the right.
const MASK_CENTER: [f32; 2] = [0.82, 0.22];
/// The mask's radii, as fractions of the art's width and height.
const MASK_RADII: [f32; 2] = [1.0, 1.1];
/// Where along the radius the art starts fading, and where it is gone.
const MASK_FADE: [f32; 2] = [0.40, 0.80];
/// Which part of the picture a crop keeps. Backdrop art puts heads close to its
/// top edge, so a window wider than the art loses height only from the bottom,
/// which the mask fades out anyway; sideways, faces sit right of centre.
const FOCUS: [f32; 2] = [0.60, 0.0];
/// The scrims' stops: (fraction of the window width, window-colour alpha).
const HERO_SCRIM: [(f32, f32); 3] = [(0.0, 0.70), (0.40, 0.35), (0.62, 0.0)];
const DETAIL_SCRIM: [(f32, f32); 4] = [(0.0, 0.92), (0.42, 0.60), (0.75, 0.10), (1.0, 0.10)];
/// Mask grid resolution; the mask is smooth enough at this many cells.
const GRID: [usize; 2] = [32, 20];

/// How the art is faded out.
#[derive(Clone, Copy)]
pub enum Mask {
    /// Home: kept up and to the right, faded left and down, so the shelves
    /// start on the plain backdrop.
    Hero,
    /// Detail: the full width at the top, faded straight down, with a deeper
    /// scrim under the longer text column.
    Detail,
}

impl Mask {
    /// Where down the art the straight bottom fade starts, so its bottom edge
    /// never shows as a line across what follows.
    fn bottom_fade(self) -> f32 {
        match self {
            Mask::Hero => 0.6,
            Mask::Detail => 0.45,
        }
    }

    /// The scrim's stops.
    #[must_use]
    pub fn scrim(self) -> &'static [(f32, f32)] {
        match self {
            Mask::Hero => &HERO_SCRIM,
            Mask::Detail => &DETAIL_SCRIM,
        }
    }

    /// How much of the art shows at `(u, v)` (0..1 across the art).
    #[must_use]
    pub fn alpha(self, u: f32, v: f32) -> f32 {
        let radial = match self {
            Mask::Hero => {
                let dx = (u - MASK_CENTER[0]) / MASK_RADII[0];
                let dy = (v - MASK_CENTER[1]) / MASK_RADII[1];
                let d = (dx * dx + dy * dy).sqrt();
                ((MASK_FADE[1] - d) / (MASK_FADE[1] - MASK_FADE[0])).clamp(0.0, 1.0)
            }
            Mask::Detail => 1.0,
        };
        let start = self.bottom_fade();
        let t = ((v - start) / (1.0 - start)).clamp(0.0, 1.0);
        radial * (1.0 - t * t * (3.0 - 2.0 * t))
    }
}

/// Paint `texture` (`tex` pixels) masked across `rect`, then the scrim over it.
/// `None` paints only the scrim, so the text has the same ground while the art
/// is still loading.
pub fn paint(ctx: &egui::Context, rect: Rect, art: Option<(TextureId, egui::Vec2)>, mask: Mask) {
    let mut shapes: Vec<Shape> = Vec::with_capacity(2);
    if let Some((texture, tex)) = art {
        shapes.push(masked(texture, rect, art_uv(tex, rect.size()), mask).into());
    }
    shapes.push(scrim(rect, mask).into());
    backdrop::paint_above(ctx, Shape::Vec(shapes));
}

/// The part of a `tex`-sized picture shown across an art rect of `size`.
#[must_use]
pub fn art_uv(tex: egui::Vec2, size: egui::Vec2) -> Rect {
    backdrop::cover_uv(tex, size, vec2(FOCUS[0], FOCUS[1]))
}

/// A textured grid whose vertex alpha follows `mask`.
fn masked(texture: TextureId, rect: Rect, uv: Rect, mask: Mask) -> Mesh {
    let mut mesh = Mesh::with_texture(texture);
    let [nx, ny] = GRID;
    for j in 0..=ny {
        for i in 0..=nx {
            let (u, v) = (i as f32 / nx as f32, j as f32 / ny as f32);
            let alpha = mask.alpha(u, v);
            mesh.vertices.push(Vertex {
                pos: pos2(
                    rect.min.x + u * rect.width(),
                    rect.min.y + v * rect.height(),
                ),
                uv: pos2(uv.min.x + u * uv.width(), uv.min.y + v * uv.height()),
                color: Color32::WHITE.gamma_multiply(alpha),
            });
        }
    }
    let row = (nx + 1) as u32;
    for j in 0..ny as u32 {
        for i in 0..nx as u32 {
            let a = j * row + i;
            mesh.add_triangle(a, a + 1, a + row);
            mesh.add_triangle(a + 1, a + row + 1, a + row);
        }
    }
    mesh
}

/// The left-hand scrim: one band per stop pair, coloured per vertex, fading
/// out over the art's lower part as the art itself does.
fn scrim(rect: Rect, mask: Mask) -> Mesh {
    let mut mesh = Mesh::default();
    let x = |u: f32| rect.min.x + u * rect.width();
    let rows = [(0.0, 1.0), (mask.bottom_fade(), 1.0), (1.0, 0.0)];
    for pair in mask.scrim().windows(2) {
        let &[(u0, a0), (u1, a1)] = pair else {
            continue;
        };
        for band in rows.windows(2) {
            let &[(v0, k0), (v1, k1)] = band else {
                continue;
            };
            let y = |v: f32| rect.min.y + v * rect.height();
            let base = mesh.vertices.len() as u32;
            for (p, a) in [
                (pos2(x(u0), y(v0)), a0 * k0),
                (pos2(x(u1), y(v0)), a1 * k0),
                (pos2(x(u0), y(v1)), a0 * k1),
                (pos2(x(u1), y(v1)), a1 * k1),
            ] {
                mesh.colored_vertex(p, theme::WINDOW.gamma_multiply(a));
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
    }
    mesh
}

/// The art's box: the whole window width, from `top` down `height`.
#[must_use]
pub fn rect(window: Rect, top: f32, height: f32) -> Rect {
    Rect::from_min_size(Pos2::new(window.min.x, top), vec2(window.width(), height))
}
