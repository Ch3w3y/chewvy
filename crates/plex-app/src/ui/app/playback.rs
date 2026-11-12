//! The playing screen's side of [`Ui`]: starting an item, the overlay frame,
//! and the hand-off to the next episode or another quality.

use std::time::{Duration, Instant};

use crate::player::{self, quality, PlayRequest};
use crate::ui::player::{Action, Overlay};
use crate::ui::theme;
use crate::watch::Cmd;

use super::Ui;

/// How often the playing screen hands the worker a position. A server keeps an
/// item on "Continue Watching" from the last position it heard, so this is how
/// much of an item a crash can lose; a quit the app gets to see also reports
/// where it stopped. Five seconds is short next to a mistake and long next to
/// a frame.
const PROGRESS_TICK: Duration = Duration::from_secs(5);

impl Ui {
    pub(super) fn begin_playback(&mut self, request: PlayRequest, ctx: &egui::Context) {
        self.handoff = None;
        // A new item starts its own reporting: the first position the player
        // reaches is the report that tells a server the item began.
        self.progress_at = None;
        match player::start(
            request,
            self.settings.hardware_decoding,
            self.hdr.output,
            ctx,
        ) {
            Ok(()) => {
                self.overlay = Some(Overlay::default());
                self.status.clear();
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// The playing screen: [`Overlay`] over the video the host renders behind it.
    pub(super) fn draw_playback(&mut self, root: &mut egui::Ui) {
        self.report_progress(root.ctx());
        let overlay = self.overlay.get_or_insert_with(Overlay::default);
        match overlay.show(root, &mut self.artwork) {
            Action::Stay => self.auto_hdr(root.ctx()),
            Action::Output { hdr, at } => self.request_output(hdr, at, root.ctx()),
            Action::Back(failure) => {
                self.end_playback();
                self.reset_output();
                if failure.is_some() {
                    self.error = failure;
                }
            }
            Action::Unplayable(why) => self.fall_back(why, root.ctx()),
            Action::Next(next) => {
                self.end_playback();
                self.handoff = Some(format!("Up next · {}", next.label));
                self.send(Cmd::Play {
                    rating_key: next.rating_key,
                    title: next.label,
                    start: None,
                    quality: None,
                    wakeup: root.ctx().clone(),
                });
            }
            Action::Requality { index, at } => {
                let Some((rating_key, title)) =
                    player::with(|p| (p.request().rating_key.clone(), p.request().title.clone()))
                else {
                    return;
                };
                self.end_playback();
                self.handoff = Some(format!("Switching to {}…", quality::quality(index).label));
                self.send(Cmd::Quality(index));
                self.send(Cmd::Play {
                    rating_key,
                    title,
                    start: Some(at),
                    quality: None,
                    wakeup: root.ctx().clone(),
                });
            }
        }
        self.draw_hdr_prompt(root.ctx());
    }

    /// The playing item could not be played as the server sent it: reopen it
    /// one step down the ladder, at the position it reached, or show `why`
    /// when no step is left. The step is for this item only; the quality the
    /// user chose still opens the next one.
    fn fall_back(&mut self, why: String, ctx: &egui::Context) {
        let Some((rating_key, title, index, fell_back, at)) = player::with(|p| {
            let r = p.request();
            (
                r.rating_key.clone(),
                r.title.clone(),
                r.quality,
                r.fell_back,
                p.resume_point(),
            )
        }) else {
            return;
        };
        self.end_playback();
        let Some((next, line)) = quality::fallback(index) else {
            self.reset_output();
            self.error = Some(if fell_back {
                format!(
                    "{why}, after falling back to {}",
                    quality::quality(index).label
                )
            } else {
                why
            });
            return;
        };
        log::warn!("{why}; {line}");
        self.handoff = Some(line.to_owned());
        self.send(Cmd::Play {
            rating_key,
            title,
            start: at,
            quality: Some(next),
            wakeup: ctx.clone(),
        });
    }

    /// One position per [`PROGRESS_TICK`], for the worker to report. The screen
    /// does not know which source is playing or whether the user wants positions
    /// reported at all: it says where the player is, when it was last said, and
    /// what the Settings switch says now.
    fn report_progress(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self
            .progress_at
            .is_some_and(|at| now.duration_since(at) < PROGRESS_TICK)
        {
            return;
        }
        // A file that has not reported a position yet has no position to report,
        // and the tick belongs to it, not to the clock: this returns without
        // starting the interval, so the first position after a load is reported
        // as soon as it exists.
        let Some((position, paused)) = player::position() else {
            return;
        };
        self.progress_at = Some(now);
        self.send(Cmd::JellyfinProgress {
            time_ms: (position * 1000.0) as u64,
            paused,
            enabled: self.settings.progress_sync,
        });
        // The app idles its redraws, and a tick the UI sleeps through is a
        // position nobody reported.
        ctx.request_repaint_after(PROGRESS_TICK);
    }

    /// Stop the player, say where it stopped, and end its server stream
    /// session, if it had one.
    pub(super) fn end_playback(&mut self) {
        let session = player::with(|p| p.request().session.clone()).flatten();
        let last = player::position();
        player::stop();
        self.overlay = None;
        // One position more than the paced report: this is the one a server
        // writes down, and the player still knows where it stopped.
        if let Some((position, paused)) = last {
            self.send(Cmd::JellyfinProgress {
                time_ms: (position * 1000.0) as u64,
                paused,
                enabled: self.settings.progress_sync,
            });
        }
        self.send(Cmd::StopStream(session));
    }
}

/// Black with one line of text while the next item resolves.
pub(super) fn handoff_screen(root: &mut egui::Ui, text: &str) {
    let rect = root.max_rect();
    root.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
    root.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        theme::h2(),
        theme::SECONDARY,
    );
}
