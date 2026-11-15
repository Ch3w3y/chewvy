//! The screens before the browsing shell (starting, the first-run choice,
//! the Plex pairing code, an account's servers), and the messages the app
//! shows over a screen: the error strip under a sign-in card and the status
//! toast over the shell.

use egui::{pos2, vec2, Align2, Rect};

use crate::ui::controls::{self, Button, Kind};
use crate::ui::{glass, splash, theme};
use crate::watch::Cmd;

use super::{Screen, Ui};

/// The widest a message strip or toast grows, and its height.
const STRIP: [f32; 2] = [640.0, 40.0];
/// How far a toast floats above the window's bottom edge.
const TOAST_BOTTOM: f32 = 24.0;
/// The room a message keeps on each side of its text.
const STRIP_PAD: f32 = 20.0;

impl Ui {
    /// The sign-in screens: the backdrop, and a glass card that says what the
    /// app is doing (an orbit while it works), shows the pairing code, or
    /// lists the account's servers.
    pub(super) fn sign_in(&mut self, ui: &mut egui::Ui) {
        let area = ui.max_rect();
        let ctx = ui.ctx().clone();
        self.paint_background(&ctx, ctx.content_rect(), crate::ui::backdrop::Kind::Sharp);
        let screen = match &self.screen {
            Screen::Pairing { code, url } => Screen::Pairing {
                code: code.clone(),
                url: url.clone(),
            },
            Screen::Servers(names) => Screen::Servers(names.clone()),
            Screen::Welcome => Screen::Welcome,
            _ => Screen::Starting,
        };
        let error = self.error.clone();
        let status = self.status.clone();
        splash::card(ui, area, |ui| match &screen {
            Screen::Pairing { code, url } => self.pairing_card(ui, code, url),
            Screen::Welcome => self.welcome(ui, &status),
            Screen::Servers(names) => {
                let subtitle = if names.is_empty() {
                    "No servers found on this account"
                } else {
                    "Choose a server to browse"
                };
                splash::heading(ui, "Your servers", subtitle);
                for (index, name) in names.iter().enumerate() {
                    if splash::server_row(ui, name) {
                        self.send(Cmd::Connect(index));
                    }
                }
            }
            _ => {
                splash::masthead(ui);
                ui.label(theme::secondary("Your libraries, beautifully").font(theme::body()));
                ui.add_space(24.0);
                splash::orbit(ui);
                ui.add_space(16.0);
                let line = if status.trim().is_empty() {
                    "Starting…".to_owned()
                } else {
                    status.clone()
                };
                ui.label(theme::secondary(line).font(theme::body()));
            }
        });
        splash::not_affiliated(ui, area);
        if let Some(error) = error {
            let centre = pos2(area.center().x, area.max.y - 56.0);
            message(ui.painter(), centre, area.width(), &error, true);
        }
    }

    /// The Plex pairing card: the code as tiles, the way to enter it, and the
    /// two ways out.
    fn pairing_card(&mut self, ui: &mut egui::Ui, code: &str, url: &str) {
        splash::heading(ui, "Link this device", "Sign in with your Plex account");
        ui.label(theme::secondary("Go to plex.tv/link and enter this code").font(theme::body()));
        ui.add_space(16.0);
        splash::code_tiles(ui, code);
        ui.add_space(24.0);
        if Button::new(Kind::Primary, "Open plex.tv/link")
            .wide()
            .show(ui)
            .clicked()
        {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
        ui.add_space(4.0);
        if controls::button(ui, Kind::Quiet, "Get a new code").clicked() {
            self.send(Cmd::Pair);
        }
        if controls::button(ui, Kind::Quiet, "Back").clicked() {
            self.send(Cmd::CancelPair);
            self.open_without_plex();
        }
        ui.add_space(12.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            controls::spinner(ui);
            ui.label(theme::secondary("Waiting for the code to be entered").font(theme::meta()));
        });
    }

    /// What the shell has to say (an error, or what it is doing) as a glass
    /// toast floating over the bottom of the page, so it never takes the
    /// page's height or moves what is on it.
    pub(super) fn status_toast(&self, ctx: &egui::Context) {
        let (failed, text) = match &self.error {
            Some(error) => (true, error.as_str()),
            None if !self.status.trim().is_empty() => (false, self.status.as_str()),
            None => return,
        };
        let screen = ctx.content_rect();
        let page = Rect::from_min_max(pos2(crate::ui::rail::WIDTH, screen.min.y), screen.max);
        egui::Area::new(egui::Id::new("status-toast"))
            .order(egui::Order::Foreground)
            .interactable(false)
            .fixed_pos(page.min)
            .show(ctx, |ui| {
                let centre = pos2(page.center().x, page.max.y - TOAST_BOTTOM - STRIP[1] / 2.0);
                message(ui.painter(), centre, page.width(), text, failed);
            });
    }
}

/// One message on a glass strip centred on `centre`: as wide as its text, at
/// most [`STRIP`] wide and never wider than `room` allows, the text ending in
/// `…` when it is longer than that.
fn message(painter: &egui::Painter, centre: egui::Pos2, room: f32, text: &str, failed: bool) {
    let palette = theme::palette();
    let ink = if failed { palette.danger } else { palette.text };
    let max = STRIP[0].min(room - 32.0).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), theme::body(), ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width((max - 2.0 * STRIP_PAD).max(0.0));
    let galley = painter.layout_job(job);
    let strip = Rect::from_center_size(centre, vec2(galley.size().x + 2.0 * STRIP_PAD, STRIP[1]));
    glass::panel(
        painter,
        strip,
        STRIP[1] / 2.0,
        theme::current().glass.status_strip,
    );
    painter.galley(
        Align2::CENTER_CENTER
            .anchor_size(strip.center(), galley.size())
            .min,
        galley,
        ink,
    );
}
