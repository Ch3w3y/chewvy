//! The Settings entry: the account and server the app is on, what the user asks
//! of playback, and how the browsing looks — each edit written straight back to
//! `<config_dir>/settings.json`.

use egui::{CursorIcon, Margin, Rect, Sense, Stroke, Vec2};

use crate::ui::rail::Entry;
use crate::ui::{brand, chips, settings, theme};

use super::Ui;

/// A settings row's height: its label and its one-line description stacked.
const ROW_HEIGHT: f32 = 48.0;
/// The switch a settings row ends with, at the panel's right edge.
const SWITCH: Vec2 = Vec2::new(40.0, 22.0);
/// The room a row keeps between its text and its switch.
const SWITCH_GAP: f32 = 12.0;
/// The gap between a row's two lines of text.
const LINE_GAP: f32 = 4.0;
/// The room a switch keeps its knob off its own edge by.
const SWITCH_PAD: f32 = 3.0;

impl Ui {
    /// Draw the Settings entry.
    pub(super) fn draw_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        // The sections are taller than a default-sized window, and a bar that
        // is always drawn says so.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.sources_section(ui);
                self.playback_section(ui);
                self.hdr_section(ui);
                self.interface_section(ui);
                self.updates_section(ui);
                about_section(ui);
                ui.add_space(theme::GAP);
            });
    }

    /// Record one Settings edit and write it back at once, so a restart keeps
    /// what the user last chose.
    pub(super) fn set_settings(&mut self, edit: impl FnOnce(&mut settings::Settings)) {
        edit(&mut self.settings);
        settings::save(self.settings);
    }

    /// What this app can play from, in one line, and the screen that manages it.
    /// The list itself belongs to Sources: one screen shows the sources, one
    /// screen changes them.
    fn sources_section(&mut self, ui: &mut egui::Ui) {
        let summary = self.sources.summary();
        section_panel(ui, "Sources", |ui| {
            ui.label(theme::secondary(summary));
            ui.horizontal_wrapped(|ui| {
                if chip(
                    ui,
                    "Manage sources",
                    "Add a server, or sign one of them out",
                ) {
                    self.go(Entry::Sources);
                }
            });
        });
    }

    /// The HDR window: whether HDR and Dolby Vision videos move there by
    /// themselves, and whether it opens full screen. Without the first switch
    /// the player offers a button instead.
    fn hdr_section(&mut self, ui: &mut egui::Ui) {
        section_panel(ui, "HDR & Dolby Vision", |ui| {
            ui.label(theme::secondary(
                "HDR and Dolby Vision play with correct colours in a separate HDR window, \
                 which sends real HDR to your display. Turn HDR on for the display in your \
                 system settings first.",
            ));
            let auto = self.settings.hdr_auto;
            if setting_row(
                ui,
                "Open in the HDR window",
                "Move HDR and Dolby Vision videos there automatically",
                auto,
            ) {
                self.set_settings(|stored| stored.hdr_auto = !auto);
            }
            let full = self.settings.hdr_fullscreen;
            if setting_row(
                ui,
                "Start full screen",
                "Open the HDR window full screen; F toggles it either way",
                full,
            ) {
                self.set_settings(|stored| stored.hdr_fullscreen = !full);
            }
        });
    }

    /// What the user asks of playback, written to the settings file at once.
    fn playback_section(&mut self, ui: &mut egui::Ui) {
        section_panel(ui, "Playback", |ui| {
            let sync = self.settings.progress_sync;
            if setting_row(
                ui,
                "Progress sync",
                "Keep what this client has watched in step with the server",
                sync,
            ) {
                self.set_settings(|stored| stored.progress_sync = !sync);
            }
            let hw = self.settings.hardware_decoding;
            if setting_row(
                ui,
                "Hardware decoding",
                "Prefer hardware decoding where it is available",
                hw,
            ) {
                self.set_settings(|stored| stored.hardware_decoding = !hw);
            }
        });
    }

    /// How wide the library grid lays its posters out — the setting the browsing
    /// screens read.
    fn interface_section(&mut self, ui: &mut egui::Ui) {
        section_panel(ui, "Interface", |ui| {
            ui.label(theme::secondary("Poster size"));
            ui.horizontal_wrapped(|ui| {
                for size in settings::PosterSize::ALL {
                    let chosen = self.settings.poster_size == size;
                    let chip = chips::slot(ui, size.label());
                    chips::paint(ui, chip.rect, size.label(), chosen, false);
                    if chip.response.clicked() && !chosen {
                        self.set_settings(|settings| settings.poster_size = size);
                    }
                }
            });
        });
    }
}

/// What this build is, whose it is not, and the attribution its bundled
/// artwork and font are licensed under.
fn about_section(ui: &mut egui::Ui) {
    section_panel(ui, "About", |ui| {
        ui.label(concat!("Chewvy ", env!("CARGO_PKG_VERSION")));
        ui.label(theme::secondary(brand::NOT_AFFILIATED));
        ui.add_space(theme::SPACING_BASE);
        for credit in brand::CREDITS {
            ui.label(theme::secondary(credit).size(theme::CAPTION_SIZE));
        }
        ui.label(
            theme::secondary("Free software under the GNU General Public License.")
                .size(theme::CAPTION_SIZE),
        );
    });
}

/// One Settings section: its title and its controls inside a rounded
/// [`theme::CARD`] panel, so the four sections read as separate cards on the
/// screen rather than one run of labels.
pub(super) fn section_panel(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(theme::GAP);
    egui::Frame::new()
        .fill(theme::CARD)
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(Margin::symmetric(16, 12))
        .stroke(Stroke::new(theme::HAIRLINE, theme::CARD_HOVER))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.label(theme::strong(title.to_owned()).size(theme::H2_SIZE));
                ui.add_space(theme::SPACING_BASE);
                contents(ui);
            });
        });
}

/// A control drawn as one of the screen's own chips rather than egui's default
/// button, which is a different shape from every other control in the app.
pub(super) fn chip(ui: &mut egui::Ui, label: &str, hover: &str) -> bool {
    let chip = chips::slot(ui, label);
    chips::paint(ui, chip.rect, label, false, false);
    chip.response.on_hover_text(hover).clicked()
}

/// One setting as a row of its panel: label and one-line description at the
/// left, its switch at the right edge. The whole row is the control and answers
/// whether it was clicked.
pub(super) fn setting_row(ui: &mut egui::Ui, label: &str, note: &str, on: bool) -> bool {
    let row = ui
        .allocate_response(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let rect = row.rect;
    if row.hovered() {
        ui.painter()
            .rect_filled(rect, theme::CARD_RADIUS, theme::CARD_HOVER);
    }
    // Both lines keep to one row of their own and end in `…` rather than
    // crossing into the switch or lifting the row.
    let max_width = (rect.width() - SWITCH.x - SWITCH_GAP).max(0.0);
    let title = ui
        .painter()
        .layout_job(chips::label_job(label, theme::TEXT, max_width));
    let note = ui
        .painter()
        .layout_job(chips::label_job(note, theme::SECONDARY, max_width));
    let (title_h, note_h) = (title.size().y, note.size().y);
    let left = rect.min.x + theme::SPACING_BASE;
    let top = rect.center().y - (title_h + LINE_GAP + note_h) * 0.5;
    ui.painter()
        .galley(egui::pos2(left, top), title, theme::TEXT);
    ui.painter().galley(
        egui::pos2(left, top + title_h + LINE_GAP),
        note,
        theme::SECONDARY,
    );
    switch(ui, switch_rect(rect), on);
    row.clicked()
}

/// The switch's box: [`SWITCH`] tall and centred on the row's right edge.
fn switch_rect(rect: Rect) -> Rect {
    let min = egui::pos2(
        rect.max.x - theme::SPACING_BASE - SWITCH.x,
        rect.center().y - SWITCH.y * 0.5,
    );
    Rect::from_min_size(min, SWITCH)
}

/// The switch itself: an accent track with its knob to the right when the
/// setting is on, a quiet track with its knob to the left when it is off.
fn switch(ui: &mut egui::Ui, rect: Rect, on: bool) {
    let radius = egui::CornerRadius::same(theme::PILL_RADIUS_U8);
    let (fill, edge) = if on {
        (theme::ACCENT, theme::ACCENT)
    } else {
        (theme::SURFACE, chips::OUTLINE)
    };
    ui.painter().rect_filled(rect, radius, fill);
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(theme::HAIRLINE, edge),
        egui::StrokeKind::Middle,
    );
    let knob = (rect.height() - 2.0 * SWITCH_PAD) * 0.5;
    let slide = rect.width() * 0.5 - knob - SWITCH_PAD;
    let center = rect.center() + egui::vec2(if on { slide } else { -slide }, 0.0);
    ui.painter().circle_filled(
        center,
        knob,
        if on {
            theme::PRIMARY_TEXT
        } else {
            theme::SECONDARY
        },
    );
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;
