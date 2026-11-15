//! The copies a background is drawn from, derived when a photograph is
//! loaded rather than shipped: the blurred browsing copy, the picker's
//! thumbnail, and the dims its pixels ask for. A bundled photograph is decoded
//! here on a thread of its own; a server's backdrop goes through the same
//! [`derive`] on the artwork loader's threads.

use std::sync::mpsc::{channel, Receiver, Sender};

use egui::ColorImage;
use image::RgbImage;

use super::legibility;
use super::photos::PHOTOS;
use crate::ui::theme::{self, Backdrop};

/// The browsing copy's width: it is blurred, so more pixels would not show.
pub const SOFT_WIDTH: u32 = 960;
/// The browsing copy's blur, in pixels of [`SOFT_WIDTH`]: about the blur the
/// shipped aurora copy had at 2048 pixels.
const SOFT_SIGMA: f32 = 4.0;
/// The theme picker's tile width.
pub const THUMB_WIDTH: u32 = 320;

/// A photograph's derived copies, as RGB.
pub struct Derived {
    /// The browsing copy.
    pub soft: RgbImage,
    /// The sign-in copy at [`SOFT_WIDTH`], unblurred: what the dims are
    /// measured on and what the thumbnail is cut from.
    pub small: RgbImage,
    /// What the photograph needs over it.
    pub backdrop: Backdrop,
}

/// The browsing copy and dims of a decoded photograph.
#[must_use]
pub fn derive(image: &RgbImage) -> Derived {
    let small = shrink(image, SOFT_WIDTH);
    let soft = image::imageops::fast_blur(&small, SOFT_SIGMA);
    let backdrop = legibility::backdrop_for(&theme::SMOKE.palette, pixels(&small), pixels(&soft));
    Derived {
        soft,
        small,
        backdrop,
    }
}

/// `image` scaled down to `width`, keeping its aspect; never enlarged.
#[must_use]
pub fn shrink(image: &RgbImage, width: u32) -> RgbImage {
    let (w, h) = image.dimensions();
    if w <= width {
        return image.clone();
    }
    let height = (u64::from(h) * u64::from(width) / u64::from(w.max(1))).max(1);
    image::imageops::thumbnail(image, width, u32::try_from(height).unwrap_or(1))
}

fn pixels(image: &RgbImage) -> &[[u8; 3]] {
    let raw: &[u8] = image.as_raw();
    let (chunks, _) = raw.as_chunks::<3>();
    chunks
}

/// An opaque RGB image as egui takes it.
#[must_use]
pub fn color_image(image: &RgbImage) -> ColorImage {
    let size = [image.width() as usize, image.height() as usize];
    ColorImage::from_rgb(size, image.as_raw())
}

/// Which copies of a bundled photograph to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    /// The scene itself, at its full size: sign-in screens.
    Sharp,
    /// Blurred: browsing screens.
    Soft,
    /// The theme picker's tile.
    Thumb,
}

/// One bundled photograph to decode and the copy wanted of it; the thumbnail
/// and the dims come with every answer.
pub struct Job {
    pub index: usize,
    pub variant: Variant,
}

/// What a job made.
pub struct Decoded {
    pub index: usize,
    pub variant: Variant,
    /// The asked copy, unless it was the thumbnail.
    pub image: Option<ColorImage>,
    pub thumb: ColorImage,
    pub backdrop: Backdrop,
}

/// The thread bundled photographs are decoded on, started on first use.
#[derive(Default)]
pub struct Decoder {
    channel: Option<(Sender<Job>, Receiver<Decoded>)>,
}

impl Decoder {
    /// Queue `job`.
    pub fn ask(&mut self, job: Job) {
        if self.channel.is_none() {
            self.channel = start();
        }
        if let Some((jobs, _)) = &self.channel {
            let _ = jobs.send(job);
        }
    }

    /// The jobs finished since the last call.
    pub fn finished(&mut self) -> Vec<Decoded> {
        self.channel
            .as_ref()
            .map(|(_, done)| done.try_iter().collect())
            .unwrap_or_default()
    }
}

fn start() -> Option<(Sender<Job>, Receiver<Decoded>)> {
    let (jobs, inbox) = channel::<Job>();
    let (outbox, done) = channel();
    let spawned = std::thread::Builder::new()
        .name("backdrop-decode".to_owned())
        .spawn(move || {
            let mut pending: Vec<Job> = Vec::new();
            while let Some(job) = next_job(&inbox, &mut pending) {
                match decode(&job) {
                    Some(decoded) => {
                        if outbox.send(decoded).is_err() {
                            return;
                        }
                        crate::watch::wake();
                    }
                    None => log::warn!(
                        "bundled backdrop {} did not decode",
                        PHOTOS.get(job.index).map_or("?", |photo| photo.id)
                    ),
                }
            }
        });
    match spawned {
        Ok(_) => Some((jobs, done)),
        Err(error) => {
            log::warn!("backdrop decoder did not start: {error}");
            None
        }
    }
}

/// The next job to decode, `None` once the app has gone. A full-size copy
/// goes ahead of picker tiles: the background waits on it, and behind a
/// queue of thirteen tiles it would hold the old picture for seconds.
pub(super) fn next_job(inbox: &Receiver<Job>, pending: &mut Vec<Job>) -> Option<Job> {
    if pending.is_empty() {
        pending.push(inbox.recv().ok()?);
    }
    pending.extend(inbox.try_iter());
    let at = pending
        .iter()
        .position(|job| job.variant != Variant::Thumb)
        .unwrap_or(0);
    Some(pending.remove(at))
}

fn decode(job: &Job) -> Option<Decoded> {
    let photo = PHOTOS.get(job.index)?;
    let image = image::load_from_memory_with_format(photo.bytes, image::ImageFormat::Jpeg)
        .ok()?
        .to_rgb8();
    let derived = derive(&image);
    let made = match job.variant {
        Variant::Sharp => Some(color_image(&image)),
        Variant::Soft => Some(color_image(&derived.soft)),
        Variant::Thumb => None,
    };
    Some(Decoded {
        index: job.index,
        variant: job.variant,
        image: made,
        thumb: color_image(&shrink(&derived.small, THUMB_WIDTH)),
        backdrop: derived.backdrop,
    })
}
