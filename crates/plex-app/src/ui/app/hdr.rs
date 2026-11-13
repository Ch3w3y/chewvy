//! Moving a playing item between this window and the HDR window, and the
//! explanation shown before the first move.
//!
//! Either move reopens the item at the position it had reached, through the
//! same worker resolve a quality switch uses, so Plex and Jellyfin items move
//! the same way. Coming back plays the item here, tone-mapped, which is the
//! only form this window's surface can show.

use egui::{vec2, Id, RichText};

use crate::player::{self, hdr::Output};
use crate::ui::player::hdr::offered;
use crate::ui::theme;
use crate::watch::Cmd;

use super::Ui;

/// What the HDR side of the playing screen remembers between frames.
#[derive(Default)]
pub(super) struct HdrState {
    /// Where the next item opens: kept across a move so the reopened item
    /// lands in the window that was asked for.
    pub output: Output,
    /// The explanation is open; the move waits for it at this position.
    prompt: Option<f64>,
    /// The item the user chose to keep here, so automatic mode leaves it be.
    declined: Option<String>,
}

impl Ui {
    /// Ask for a move to the HDR window (`hdr`) or back here, at `at` seconds.
    /// The first move to the HDR window waits for the explanation.
    pub(super) fn request_output(&mut self, hdr: bool, at: f64, ctx: &egui::Context) {
        if hdr && !self.settings.hdr_explained {
            self.hdr.prompt = Some(at);
            return;
        }
        if !hdr {
            self.hdr.declined = player::with(|p| p.request().rating_key.clone());
        }
        self.move_output(hdr, at, ctx);
    }

    /// In automatic mode, an HDR video playing here moves by itself, once.
    pub(super) fn auto_hdr(&mut self, ctx: &egui::Context) {
        if !self.settings.hdr_auto || self.hdr.prompt.is_some() {
            return;
        }
        let at = player::with(|p| {
            let declined = self.hdr.declined.as_deref() == Some(p.request().rating_key.as_str());
            (offered(p) && !declined).then(|| p.snapshot().position.unwrap_or(0.0))
        })
        .flatten();
        if let Some(at) = at {
            self.request_output(true, at, ctx);
        }
    }

    /// Leaving the player resets where the next item opens.
    pub(super) fn reset_output(&mut self) {
        self.hdr = HdrState::default();
    }

    /// The explanation, while it is open.
    pub(super) fn draw_hdr_prompt(&mut self, ctx: &egui::Context) {
        let Some(at) = self.hdr.prompt else {
            return;
        };
        let mut choice = None;
        egui::Modal::new(Id::new("hdr-window-explainer")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.label(RichText::new("Open the HDR window?").font(theme::h2()));
            ui.add_space(8.0);
            ui.label(
                "This video is HDR. Chewvy's own window can only show it tone-mapped, \
                 so HDR and Dolby Vision play in a separate window that sends real HDR \
                 to your display and applies Dolby Vision's colour processing itself.",
            );
            ui.add_space(6.0);
            ui.label(theme::secondary(
                "Your TV shows HDR, not its own Dolby Vision mode. Turn HDR on for the \
                 display in your system settings. In that window: Space pauses, F toggles \
                 full screen, N plays the next episode, Q closes it. Chewvy shows a card with a button to close it \
                 and keep playing here.",
            ));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new("Open HDR window").min_size(vec2(160.0, 34.0)))
                    .clicked()
                {
                    choice = Some(true);
                }
                if ui
                    .add(egui::Button::new("Keep playing here").min_size(vec2(160.0, 34.0)))
                    .clicked()
                {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => {
                self.hdr.prompt = None;
                self.set_hdr_explained();
                self.move_output(true, at, ctx);
            }
            Some(false) => {
                self.hdr.prompt = None;
                self.hdr.declined = player::with(|p| p.request().rating_key.clone());
            }
            None => {}
        }
    }

    fn set_hdr_explained(&mut self) {
        self.settings.hdr_explained = true;
        crate::ui::settings::save(self.settings);
    }

    /// Reopen the playing item at `at` in the other window.
    fn move_output(&mut self, hdr: bool, at: f64, ctx: &egui::Context) {
        let Some((rating_key, title)) =
            player::with(|p| (p.request().rating_key.clone(), p.request().title.clone()))
        else {
            return;
        };
        self.end_playback();
        self.hdr.output = if hdr {
            Output::HdrWindow {
                fullscreen: self.settings.hdr_fullscreen,
            }
        } else {
            Output::Embedded
        };
        self.handoff = Some(
            if hdr {
                "Opening the HDR window…"
            } else {
                "Playing here…"
            }
            .to_owned(),
        );
        self.send(Cmd::Play {
            rating_key,
            title,
            start: Some(at),
            quality: None,
            wakeup: ctx.clone(),
        });
    }
}
