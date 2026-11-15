//! The Settings entry: the account and server the app is on, what the user asks
//! of playback, and how the browsing looks — each edit written straight back to
//! `<config_dir>/settings.json`.

use egui::{Frame, Margin};

use crate::ui::controls::{self, Button, Kind};
use crate::ui::rail::Entry;
use crate::ui::{brand, chips, glass, settings, theme};

use super::Ui;

/// The widest the Settings and Sources columns grow: a row is one control,
/// and a row as wide as a large window is a target nobody means to hit.
pub(super) const COLUMN: f32 = 720.0;
/// A section card's corner radius.
pub(super) const SECTION_RADIUS: f32 = 12.0;

impl Ui {
    /// Draw the Settings entry.
    pub(super) fn draw_settings(&mut self, ui: &mut egui::Ui) {
        // The title scrolls with the sections, so none is cut off under it.
        egui::ScrollArea::vertical()
            .id_salt("settings-page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(COLUMN);
                crate::ui::legible::label(ui, egui::RichText::new("Settings").heading());
                self.sources_section(ui);
                self.playback_section(ui);
                self.hdr_section(ui);
                self.interface_section(ui);
                self.background_section(ui);
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
            if Button::new(Kind::Secondary, "Manage sources")
                .hint("Add a server, or sign one of them out")
                .show(ui)
                .clicked()
            {
                self.go(Entry::Sources);
            }
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
                ui.spacing_mut().item_spacing.x = chips::GAP;
                for size in settings::PosterSize::ALL {
                    let chosen = self.settings.poster_size == size;
                    let chip = chips::slot(ui, size.label());
                    chips::paint(ui, &chip, size.label(), chosen, false);
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
        // The credits read as one block, not as paragraphs.
        ui.spacing_mut().item_spacing.y = theme::SPACING_BASE;
        for credit in brand::CREDITS {
            ui.label(theme::secondary(credit).size(theme::CAPTION_SIZE));
        }
        photo_credits(ui);
        ui.label(
            theme::secondary("Free software under the GNU General Public License.")
                .size(theme::CAPTION_SIZE),
        );
    });
}

/// Every bundled photograph's credit, as its licence asks: title, author,
/// licence, and the source page a click opens.
fn photo_credits(ui: &mut egui::Ui) {
    ui.label(
        theme::secondary(
            "Background photographs from Wikimedia Commons, cropped, resized and \
             darkened; the browsing copies are blurred:",
        )
        .size(theme::CAPTION_SIZE),
    );
    for photo in &crate::ui::backdrop::PHOTOS {
        let licence = if photo.licence_url.is_empty() {
            String::new()
        } else {
            format!("\nLicence: {}", photo.licence_url)
        };
        let line = format!("{}: {}", photo.theme.name(), photo.credit());
        let response = ui
            .add(
                egui::Label::new(theme::secondary(line).size(theme::CAPTION_SIZE))
                    .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("{}{licence}", photo.page));
        if response.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(photo.page));
        }
    }
}

/// One Settings section: its title and its controls on a glass card, so the
/// sections read as separate cards in the app's own material.
pub(super) fn section_panel(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    glass_card(ui, |ui| {
        ui.label(theme::strong(title.to_owned()).size(theme::H2_SIZE));
        ui.add_space(theme::SPACING_BASE);
        contents(ui);
    });
}

/// A footnote on the same glass as the sections, so small text never sits
/// straight on the photograph.
pub(super) fn note_panel(ui: &mut egui::Ui, note: &str) {
    glass_card(ui, |ui| {
        ui.label(theme::secondary(note).size(theme::CAPTION_SIZE));
    });
}

fn glass_card(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(theme::GAP);
    let slot = glass::reserve(ui.painter());
    let card = Frame::NONE
        .inner_margin(Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), contents);
        });
    glass::fill(
        ui.painter(),
        slot,
        card.response.rect,
        SECTION_RADIUS,
        theme::current().glass.section,
    );
}

/// One setting as a row of its section: label and description at the left,
/// its switch at the right edge. The whole row is the control and answers
/// whether it was clicked.
pub(super) fn setting_row(ui: &mut egui::Ui, label: &str, note: &str, on: bool) -> bool {
    controls::switch_row(ui, label, Some(note), on).clicked()
}
