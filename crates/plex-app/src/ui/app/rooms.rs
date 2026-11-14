//! The Plex side of watching together, as the shell runs it: the rooms
//! panel a Plex title or the sidebar opens, and what its choices ask of the
//! worker. The room this device is in is a group like any other
//! ([`super::together`]).

use crate::together::follow::Command;
use crate::together::{Ask, Room};
use crate::ui::together::{Choice, Invite, PanelView};
use crate::watch::Cmd;

use super::together::{Group, Panel, Subject, Together};
use super::Ui;

impl Together {
    /// Where the group's last command puts the player now.
    pub(super) fn follower_position(&self) -> Option<f64> {
        match self.command? {
            Command::PlayAt { at, position } => {
                let now = self.now()?;
                Some(position + (now.0 - at.0).max(0.0))
            }
            Command::PauseAt { position } | Command::SeekTo { position } => Some(position),
            Command::Stop => None,
        }
    }

    /// The rooms listed so far; empty before the first list.
    fn listed(&self) -> &[Room] {
        match &self.rooms {
            Some(Ok(rooms)) => rooms,
            _ => &[],
        }
    }
}

impl Ui {
    /// How many Plex rooms the account is in, for the sidebar.
    pub(super) fn room_count(&self) -> usize {
        self.together.listed().len()
    }

    /// The rating key of the Plex room this device is in.
    pub(super) fn room_title(&self) -> Option<&str> {
        let id = self.together.group.as_ref()?.room.as_deref()?;
        self.together
            .listed()
            .iter()
            .find(|room| room.id == id)?
            .rating_key
            .as_deref()
    }

    /// Open the rooms panel for a Plex title, or for every room, under
    /// `anchor`; the list and the people are read again.
    pub(super) fn open_rooms(&mut self, subject: Subject, anchor: egui::Rect) {
        let for_title = matches!(subject, Subject::Title { .. });
        self.together.panel = Some(Panel {
            subject,
            anchor,
            groups: None,
            refusal: None,
        });
        self.send(Cmd::Together(Ask::Rooms));
        if for_title || self.together.group.is_some() {
            self.send(Cmd::Together(Ask::People));
        }
    }

    /// The panel for a Plex title, or for every room.
    pub(super) fn rooms_view<'a>(
        &'a self,
        subject: &'a Subject,
        refusal: Option<&'a str>,
    ) -> PanelView<'a> {
        let together = &self.together;
        match subject {
            Subject::Title { rating_key, .. } => PanelView::Title {
                people: together.people.as_ref(),
                ticked: &together.ticked,
                rooms: together
                    .listed()
                    .iter()
                    .filter(|room| room.rating_key.as_deref() == Some(rating_key.as_str()))
                    .collect(),
                refusal,
            },
            Subject::Rooms | Subject::Jellyfin(_) => PanelView::Rooms {
                rooms: together.rooms.as_ref(),
                refusal,
            },
        }
    }

    /// Who the Plex room this device is in can still invite.
    pub(super) fn invite_view<'a>(&'a self, group: &Group) -> Option<Invite<'a>> {
        let id = group.room.as_deref()?;
        let invited = self
            .together
            .listed()
            .iter()
            .find(|room| room.id == id)
            .map_or(&[][..], |room| &room.ids);
        Some(Invite {
            people: self.together.people.as_ref(),
            ticked: &self.together.ticked,
            invited,
        })
    }

    /// A choice in a rooms panel.
    pub(super) fn room_choice(&mut self, choice: Choice) {
        let together = &mut self.together;
        match choice {
            Choice::Tick(id) => {
                if let Some(at) = together.ticked.iter().position(|ticked| *ticked == id) {
                    together.ticked.remove(at);
                } else {
                    together.ticked.push(id);
                }
            }
            Choice::Create => {
                let Some(Subject::Title { rating_key, title }) =
                    together.panel.as_ref().map(|panel| panel.subject.clone())
                else {
                    return;
                };
                let people = std::mem::take(&mut together.ticked);
                self.send(Cmd::Together(Ask::Create {
                    rating_key,
                    title,
                    people,
                }));
            }
            Choice::Enter(room) => {
                // On another of the account's servers: the worker connects
                // to it first, as the switcher would.
                let elsewhere = together
                    .listed()
                    .iter()
                    .find(|listed| listed.id == room)
                    .and_then(|listed| listed.elsewhere.clone());
                if let Some(server) = &elsewhere {
                    self.status = format!("Connecting to {} for the room…", server.name);
                }
                let server = elsewhere.map(|server| server.machine);
                self.send(Cmd::Together(Ask::Enter { room, server }));
            }
            Choice::Remove(room) => {
                if let Some(Ok(rooms)) = together.rooms.as_mut() {
                    rooms.retain(|listed| listed.id != room);
                }
                self.send(Cmd::Together(Ask::Remove { room }));
            }
            Choice::Invite => {
                let Some(room) = together.group.as_ref().and_then(|g| g.room.clone()) else {
                    return;
                };
                let people = std::mem::take(&mut together.ticked);
                self.send(Cmd::Together(Ask::Invite { room, people }));
            }
            Choice::Start | Choice::Join(_) | Choice::Leave | Choice::Close => {}
        }
    }
}
