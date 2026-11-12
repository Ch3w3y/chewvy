//! The detail screen: what the shell does with the page the worker read.
//!
//! [`crate::ui::detail::show`] draws the page and answers with what the user
//! clicked; this file is only the exchange around it — ask the worker for the
//! page and its seasons' episodes, file the answers, and turn the page's answer
//! into the same [`Cmd::Play`] every other play affordance sends.

use plex_core::library::Item;

use crate::player;
use crate::ui::detail::{self, Action};
use crate::watch::Cmd;

use super::Ui;

impl Ui {
    /// Draw the page for the selected [`crate::ui::rail::Entry::Detail`], or the
    /// wait while its item and children are read.
    pub(super) fn draw_detail(&mut self, ui: &mut egui::Ui) {
        // The page is taken out so `show` can ask the artwork loader while the
        // shell still owns it.
        let Some(mut page) = self.detail.take() else {
            ui.spinner();
            ui.label(&self.status);
            return;
        };
        let action = detail::show(
            ui,
            &mut page,
            &mut self.artwork,
            player::is_active(),
            crate::source::Servers {
                plex: &self.server,
                jellyfin: &self.sources.jellyfin,
            },
        );
        self.detail = Some(page);
        match action {
            Some(Action::Play(target)) => {
                self.status = format!("Opening “{}”…", target.title);
                self.send(Cmd::Play {
                    rating_key: target.rating_key,
                    title: target.title,
                    start: target.start,
                    quality: None,
                    wakeup: ui.ctx().clone(),
                });
            }
            Some(Action::Season { rating_key }) => {
                if let Some(page) = &mut self.detail {
                    page.select_season(&rating_key);
                }
                self.status = "Loading…".into();
                self.send(Cmd::Season { rating_key });
            }
            Some(Action::Open { rating_key, title }) => {
                self.go(crate::ui::rail::Entry::Detail {
                    id: rating_key,
                    title,
                });
            }
            None => {}
        }
    }

    /// File one `/children` answer with the page that asked for it: an answer
    /// for another season fills nothing.
    pub(super) fn fill_episodes(&mut self, season_key: &str, items: &[Item]) {
        let Some(page) = &mut self.detail else {
            return;
        };
        if page.season_key.as_deref() != Some(season_key) {
            return;
        }
        page.fill(season_key, items);
    }
}
