//! What is on screen behind the glass, and the cross-fade to what comes next.

use std::collections::{HashMap, HashSet};

use egui::{vec2, Context, LayerId, Rect, TextureHandle, TextureId, Vec2};

use super::decode::{Decoder, Job, Variant};
use super::{cover_uv, keep_slot, Kind, Showing, PHOTOS};
use crate::ui::images::Artwork;
use crate::ui::theme::{self, Backdrop};

/// How long one picture takes to give way to the next.
const FADE_SECS: f64 = 0.6;
/// Which part of a server backdrop stays when the window is narrower than
/// it: faces sit right of centre and near the top.
const LIBRARY_FOCUS: [f32; 2] = [0.6, 0.3];

/// One picture in one copy.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Layer {
    showing: Showing,
    kind: Kind,
}

/// A layer ready to draw.
#[derive(Clone, Copy)]
struct Drawn {
    texture: TextureId,
    size: Vec2,
    focus: Vec2,
    backdrop: Backdrop,
}

/// The background on screen, the one fading out, and the bundled photos'
/// textures.
#[derive(Default)]
pub struct Stage {
    shown: Option<Layer>,
    fading: Option<(Layer, f64)>,
    photos: Photos,
}

impl Stage {
    /// Paint `showing` over `rect` in the `kind` copy: until it has loaded the
    /// picture already on screen stays, and before anything has loaded
    /// `fallback` shows. A change cross-fades unless egui's animations are
    /// off. Answers what the background now shows.
    pub fn paint(
        &mut self,
        ctx: &Context,
        rect: Rect,
        kind: Kind,
        [showing, fallback]: [&Showing; 2],
        artwork: &mut Artwork,
    ) -> Option<Showing> {
        self.photos.drain(ctx);
        let now = ctx.input(|i| i.time);
        let target = Layer {
            showing: showing.clone(),
            kind,
        };
        let next = if self.resolve(&target, artwork).is_some() {
            Some(target)
        } else if let Some(shown) = self
            .shown
            .clone()
            .filter(|s| self.resolve(s, artwork).is_some())
        {
            Some(shown)
        } else {
            let fallback = Layer {
                showing: fallback.clone(),
                kind,
            };
            self.resolve(&fallback, artwork).map(|_| fallback)
        };
        if next.is_some() && next != self.shown {
            let animate = ctx.global_style().animation_time > 0.0;
            self.fading = self.shown.take().filter(|_| animate).map(|old| (old, now));
            self.shown = next;
            if self.fading.is_none() {
                self.photos.keep_only(self.shown.as_ref());
            }
        }
        let shown = self
            .shown
            .clone()
            .and_then(|layer| self.resolve(&layer, artwork));
        let fading = self.fading.clone().and_then(|(layer, start)| {
            let t = ((now - start) / FADE_SECS) as f32;
            (t < 1.0).then_some(())?;
            Some((self.resolve(&layer, artwork)?, t.max(0.0)))
        });
        if fading.is_none() && self.fading.take().is_some() {
            self.photos.keep_only(self.shown.as_ref());
        }
        draw(ctx, rect, kind, shown, fading);
        self.shown.as_ref().map(|layer| layer.showing.clone())
    }

    /// The picker's thumbnail of bundled photo `index`, once decoded.
    pub fn thumb(&mut self, ctx: &Context, index: usize) -> Option<TextureId> {
        self.photos.drain(ctx);
        self.photos.texture(index, Variant::Thumb)
    }

    fn resolve(&mut self, layer: &Layer, artwork: &mut Artwork) -> Option<Drawn> {
        match &layer.showing {
            Showing::Photo(index) => {
                let photo = PHOTOS.get(*index)?;
                let variant = match layer.kind {
                    Kind::Sharp => Variant::Sharp,
                    Kind::Soft => Variant::Soft,
                };
                let texture = self.photos.texture(*index, variant)?;
                let [width, height] = photo.size;
                Some(Drawn {
                    texture,
                    size: vec2(width as f32, height as f32),
                    focus: vec2(photo.focus[0], photo.focus[1]),
                    backdrop: *self.photos.backdrops.get(index)?,
                })
            }
            Showing::Library(address) => {
                let (texture, size, backdrop) = artwork.background(address)?;
                Some(Drawn {
                    texture,
                    size,
                    focus: vec2(LIBRARY_FOCUS[0], LIBRARY_FOCUS[1]),
                    backdrop,
                })
            }
        }
    }
}

/// The window colour, the outgoing picture, the incoming one over it at `t`,
/// then the dim the two call for, mixed at the same `t`.
fn draw(ctx: &Context, rect: Rect, kind: Kind, shown: Option<Drawn>, fading: Option<(Drawn, f32)>) {
    let theme = theme::current();
    let painter = ctx.layer_painter(LayerId::background());
    painter.rect_filled(rect, 0.0, theme.palette.window);
    let image = |drawn: &Drawn, alpha: f32| {
        let uv = cover_uv(drawn.size, rect.size(), drawn.focus);
        painter.image(
            drawn.texture,
            rect,
            uv,
            theme::UNTINTED.gamma_multiply(alpha),
        );
    };
    let backdrop = match (shown, fading) {
        (Some(shown), Some((old, t))) => {
            image(&old, 1.0);
            image(&shown, t);
            ctx.request_repaint();
            blend(old.backdrop, shown.backdrop, t)
        }
        (Some(shown), None) => {
            image(&shown, 1.0);
            shown.backdrop
        }
        (None, _) => theme::SMOKE.backdrop,
    };
    let dim = match kind {
        Kind::Sharp => backdrop.sign_in_dim,
        Kind::Soft => backdrop.browse_dim,
    };
    if dim > 0.0 {
        painter.rect_filled(
            rect,
            0.0,
            theme.palette.window.gamma_multiply(dim.clamp(0.0, 1.0)),
        );
    }
    theme::set_backdrop(backdrop);
    keep_slot(ctx);
}

fn blend(from: Backdrop, to: Backdrop, t: f32) -> Backdrop {
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    Backdrop {
        sign_in_dim: lerp(from.sign_in_dim, to.sign_in_dim),
        browse_dim: lerp(from.browse_dim, to.browse_dim),
        text_shadow: to.text_shadow,
    }
}

/// The bundled photos' textures and dims, filled by the decoder thread.
#[derive(Default)]
struct Photos {
    decoder: Decoder,
    textures: HashMap<(usize, Variant), TextureHandle>,
    backdrops: HashMap<usize, Backdrop>,
    asked: HashSet<(usize, Variant)>,
}

impl Photos {
    fn drain(&mut self, ctx: &Context) {
        for done in self.decoder.finished() {
            self.asked.remove(&(done.index, done.variant));
            self.backdrops.insert(done.index, done.backdrop);
            self.textures
                .entry((done.index, Variant::Thumb))
                .or_insert_with(|| {
                    let name = format!("backdrop-{}-thumb", done.index);
                    ctx.load_texture(name, done.thumb, egui::TextureOptions::LINEAR)
                });
            if let Some(image) = done.image {
                let name = format!("backdrop-{}-{:?}", done.index, done.variant);
                let handle = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
                self.textures.insert((done.index, done.variant), handle);
            }
        }
    }

    /// The texture of one copy, asking the decoder for it when it is missing.
    fn texture(&mut self, index: usize, variant: Variant) -> Option<TextureId> {
        let key = (index, variant);
        if let Some(handle) = self.textures.get(&key) {
            return Some(handle.id());
        }
        if self.asked.insert(key) {
            self.decoder.ask(Job { index, variant });
        }
        None
    }

    /// Free every full-size copy but the one on screen; thumbnails stay.
    fn keep_only(&mut self, shown: Option<&Layer>) {
        let kept = shown.and_then(|layer| match (&layer.showing, layer.kind) {
            (Showing::Photo(index), Kind::Sharp) => Some((*index, Variant::Sharp)),
            (Showing::Photo(index), Kind::Soft) => Some((*index, Variant::Soft)),
            (Showing::Library(_), _) => None,
        });
        self.textures
            .retain(|key, _| key.1 == Variant::Thumb || Some(*key) == kept);
    }
}
