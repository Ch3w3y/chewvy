//! The Sources screen: every place this app can play from, one card each, and
//! the panels that add one. The first-run choice is drawn from here too, since
//! it offers the same two kinds and runs the same Jellyfin sign-in.
//!
//! The Plex card is there because a Plex token is on file, not because a screen
//! remembers signing in, so signing out anywhere takes it away. A Jellyfin card
//! belongs to this device alone and Remove forgets it here alone — forgetting a
//! token is not a request, which is why a server that is asleep is still
//! removable.

use egui::{
    Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, StrokeKind, TextEdit, Vec2,
};

use crate::add_source::{AddFlow, FlowInput, Stage};
use crate::source::Origin;
use crate::sources::JellyfinServer;
use crate::ui::rail::Entry;
use crate::ui::{marks, splash, theme};
use crate::watch::{Cmd, JfCmd, JfEv};

use super::app_settings::{chip, section_panel};
use super::{Screen, Ui};

/// The Plex badge's amber.
const PLEX_HUE: Color32 = Color32::from_rgb(0xE3, 0xB6, 0x5A);
/// The Jellyfin badge's violet.
const JELLYFIN_HUE: Color32 = Color32::from_rgb(0xA9, 0x8B, 0xFF);
/// A badge's box: wide enough for the Plex wordmark.
const BADGE: Vec2 = Vec2::new(46.0, 28.0);
/// The Plex wordmark's height inside a badge.
const BADGE_MARK: f32 = 11.0;
/// Where the add panel's text fields stop.
const FIELD: [f32; 2] = [280.0, 24.0];
/// What the footer says about where the secrets live, and what Remove costs.
const TOKEN_NOTE: &str = "Tokens live in the system keychain — Secret Service, Credential \
Manager, or Keychain. Removing a source deletes its token from this device only, so a server \
that is asleep or off-network can still be forgotten.";

impl Ui {
    /// With no Plex account linked, the shell opens on the Jellyfin servers on
    /// file; with none of those either, the first-run choice does.
    pub(super) fn open_without_plex(&mut self) {
        self.sections.clear();
        self.server.clear();
        self.server_choices.clear();
        self.history = super::History::default();
        self.current = None;
        self.error = None;
        self.status = String::new();
        if self.sources.jellyfin.is_empty() {
            self.screen = Screen::Welcome;
        } else {
            self.enter_shell();
        }
    }

    /// Show the browsing shell on its Home entry.
    pub(super) fn enter_shell(&mut self) {
        self.screen = Screen::Connected;
        self.go(Entry::Home);
    }

    /// Start adding a source from inside the shell: the Sources screen, asking
    /// which kind when both can be added. One Plex account is all the sign-in
    /// can hold, so with one linked the Jellyfin panel opens directly.
    pub(super) fn add_source(&mut self) {
        if !matches!(self.current, Some(Entry::Sources)) {
            self.go(Entry::Sources);
        }
        if self.sources.plex.is_some() {
            self.add_jellyfin();
        } else {
            self.choosing = true;
        }
    }

    /// Open the panel that adds a Jellyfin server and ask the network who is there.
    pub(super) fn add_jellyfin(&mut self) {
        self.choosing = false;
        self.add = Some(AddFlow::open());
        self.send(Cmd::Jf(JfCmd::Discover));
    }

    /// Ask plex.tv for a sign-in code. The pairing card takes the window once
    /// the code arrives.
    pub(super) fn add_plex(&mut self) {
        self.choosing = false;
        self.error = None;
        self.status = "Asking plex.tv for a code…".into();
        self.send(Cmd::Pair);
    }

    /// Draw the Sources entry.
    pub(super) fn draw_sources(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sources");
        ui.label(theme::secondary(
            "Accounts and servers this app can play from",
        ));
        // The cards and an open add panel are taller than a default window.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.sources_list(ui));
    }

    /// The Sources screen under its heading: one card per source, then the way
    /// to add another.
    fn sources_list(&mut self, ui: &mut egui::Ui) {
        let held = self.sources.jellyfin.clone();
        if self.sources.plex.is_some() {
            self.plex_card(ui);
        }
        for server in &held {
            self.jellyfin_card(ui, server);
        }
        if self.add.is_none() && !self.choosing {
            ui.add_space(theme::SPACING_BASE);
            ui.horizontal_wrapped(|ui| {
                let hover = if self.sources.plex.is_some() {
                    "Add a Jellyfin server"
                } else {
                    "Add a Plex account or a Jellyfin server"
                };
                if chip(ui, "+ Add a source", hover) {
                    self.add_source();
                }
            });
            if self.sources.plex.is_some() {
                ui.label(
                    RichText::new("One Plex account at a time: sign out of it to link another.")
                        .color(theme::MUTED)
                        .size(theme::CAPTION_SIZE),
                );
            }
        }
        self.kind_panel(ui);
        self.add_panel(ui);
        ui.add_space(theme::GAP);
        ui.label(
            RichText::new(TOKEN_NOTE)
                .color(theme::MUTED)
                .size(theme::CAPTION_SIZE),
        );
        ui.add_space(theme::GAP);
    }

    /// The panel that asks which kind of source to add.
    fn kind_panel(&mut self, ui: &mut egui::Ui) {
        if !self.choosing {
            return;
        }
        let mut closed = false;
        section_panel(ui, "Add a source", |ui| {
            self.kind_options(ui);
            ui.add_space(theme::GAP);
            if chip(ui, "Cancel", "Stop adding a source") {
                closed = true;
            }
        });
        if closed {
            self.choosing = false;
        }
    }

    /// The two kinds of source, each a row that starts its own sign-in.
    fn kind_options(&mut self, ui: &mut egui::Ui) {
        if option_row(
            ui,
            &Origin::Plex,
            "Plex account",
            "Every server your account can see. Sign in with a code at plex.tv/link.",
        ) {
            self.add_plex();
        }
        ui.add_space(theme::SPACING_BASE * 2.0);
        if option_row(
            ui,
            &Origin::Jellyfin(String::new()),
            "Jellyfin server",
            "A server on your network, or its address. Sign in with Quick Connect.",
        ) {
            self.add_jellyfin();
        }
    }

    /// The first-run card: nothing is on file, so it asks where the user's
    /// films and shows are, and runs the Jellyfin sign-in when that is the answer.
    pub(super) fn welcome(&mut self, ui: &mut egui::Ui, status: &str) {
        if self.add.is_some() {
            splash::heading(ui, "Add a Jellyfin server", "Find it, then sign in");
            self.add_panel(ui);
            return;
        }
        splash::heading(
            ui,
            "Where are your films and shows?",
            "Add one now. You can add the other later.",
        );
        self.kind_options(ui);
        if !status.trim().is_empty() {
            ui.add_space(theme::GAP);
            spinner(ui, status);
        }
    }

    /// The Plex card: a source because a Plex token is on file. Its account's
    /// servers are listed under it, the same ones Settings switched between.
    fn plex_card(&mut self, ui: &mut egui::Ui) {
        let servers = self.server_choices.clone();
        let meta = plex_meta(&self.server, &servers);
        source_card(ui, &Origin::Plex, "Plex account", &meta, |ui| {
            if chip(
                ui,
                "Refresh servers",
                "Ask this account which servers it has",
            ) {
                self.send(Cmd::Servers);
            }
            if chip(ui, "Sign out", "Forget this Plex account on this device") {
                self.send(Cmd::SignOut);
            }
            ui.add_space(theme::SPACING_BASE);
            if servers.is_empty() {
                ui.label(theme::secondary(
                    "Its servers are listed once you ask for them.",
                ));
            } else {
                ui.label(theme::secondary("Or switch to another of its servers"));
                ui.horizontal_wrapped(|ui| {
                    for (index, name) in servers.iter().enumerate() {
                        if chip(ui, name, "Connect to this server") {
                            self.status = format!("Connecting to {name}…");
                            self.send(Cmd::Connect(index));
                        }
                    }
                });
            }
        });
    }

    /// A Jellyfin card: this device's own source, removable without asking the
    /// server, since forgetting a token is not a request.
    fn jellyfin_card(&mut self, ui: &mut egui::Ui, held: &JellyfinServer) {
        let server_id = held.server_id.clone();
        let name = held.name.clone();
        let meta = format!(
            "{} · {}",
            host_of(&held.base_url),
            held.user_name.as_deref().unwrap_or("signed in")
        );
        let origin = Origin::Jellyfin(server_id.clone());
        source_card(ui, &origin, &name, &meta, |ui| {
            if chip(
                ui,
                "Remove",
                "Forget this server on this device, its token included",
            ) {
                self.send(Cmd::Jf(JfCmd::Remove { server_id }));
            }
        });
    }

    /// The panel that adds a Jellyfin server, at whichever step it reached.
    ///
    /// The flow is taken out of the screen for the length of the panel so a
    /// button in it can ask the worker for a step. Both Cancel and `Added` close
    /// it, and both tell the worker the server it was working on is done with:
    /// otherwise a wait nobody is watching would run on to its own timeout.
    fn add_panel(&mut self, ui: &mut egui::Ui) {
        let Some(mut flow) = self.add.take() else {
            return;
        };
        let stage = flow.stage;
        let error = flow.error.clone();
        let mut closed = false;
        section_panel(ui, "Add a Jellyfin source", |ui| {
            match stage {
                Stage::Address => self.address_step(&mut flow, ui),
                Stage::Checking => self.checking_step(&mut flow, ui),
                Stage::Ready => self.ready_step(&mut flow, ui),
                Stage::Pairing => self.pairing_step(&mut flow, ui),
                Stage::Password => self.password_step(&mut flow, ui),
                Stage::SigningIn => {
                    ui.label(theme::secondary("Signing in\u{2026}"));
                    spinner(ui, "Exchanging the password for a token");
                }
                // The source is already in the list above; the panel is leaving.
                Stage::Added => {}
            }
            if let Some(failure) = error {
                ui.add_space(theme::SPACING_BASE);
                ui.label(
                    RichText::new(failure.message())
                        .color(theme::DANGER)
                        .size(theme::META_SIZE),
                );
            }
            ui.add_space(theme::GAP);
            ui.horizontal_wrapped(|ui| {
                let busy = stage.is_busy();
                let label = if busy { "Cancel anyway" } else { "Cancel" };
                if chip(ui, label, "Stop adding a source and forget it") {
                    closed = true;
                }
                if busy {
                    ui.label(theme::secondary("One step at a time."));
                }
            });
        });
        if closed || stage == Stage::Added {
            self.send(Cmd::Jf(JfCmd::Dismiss));
            self.add = None;
        } else {
            self.add = Some(flow);
        }
    }

    /// The step that asks where the server is: the neighbours the network named,
    /// and the address field for everything else.
    fn address_step(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        ui.label(theme::secondary(
            "Pick a server found nearby, or type its address.",
        ));
        ui.add_space(theme::SPACING_BASE);
        if flow.discovering {
            spinner(ui, "Asking the network\u{2026}");
        } else if chip(ui, "Look again", "Ask the network which servers answer") {
            self.act(flow, FlowInput::Discover);
        }
        let found = flow.discovered.clone();
        for found in &found {
            let address = found.address.clone();
            if chip(ui, &found.name, "Use this server") {
                self.act(flow, FlowInput::Address(address));
                self.act(flow, FlowInput::SubmitAddress);
            }
        }
        self.address_field(flow, ui);
    }

    /// The step that waits for an address to say what it is, still showing the
    /// field: typing over it is how that wait is left behind.
    fn checking_step(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        let address = flow.address.clone();
        ui.label(theme::secondary(format!("Checking {address}\u{2026}")));
        spinner(ui, "Getting in touch");
        ui.label(theme::secondary("Typing another address above asks again."));
        self.address_field(flow, ui);
    }

    /// The step a known server waits on: how the user wants to sign in to it.
    fn ready_step(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        let known = flow
            .server
            .as_ref()
            .map(|held| format!("{} is Jellyfin {}", host_of(&held.base_url), held.version));
        if let Some(known) = known {
            ui.label(theme::secondary(known));
        }
        ui.add_space(theme::SPACING_BASE);
        ui.label(theme::secondary("How do you want to sign in?"));
        ui.add_space(theme::SPACING_BASE);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(theme::primary(RichText::new("Sign in with a code")))
                .clicked()
            {
                self.act(flow, FlowInput::ChooseQuickConnect);
            }
            if ui.button("Username and password").clicked() {
                self.act(flow, FlowInput::ChoosePassword);
            }
        });
    }

    /// The step that shows the six digits and waits for another device to accept
    /// them — or, before the server has named them, waits for the code itself.
    fn pairing_step(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        let code = flow.code.clone();
        if code.is_empty() {
            ui.label(theme::secondary(
                "Asking that server for a code of its own.",
            ));
            ui.add_space(theme::SPACING_BASE);
            spinner(ui, "Getting the code");
        } else {
            let host = flow
                .server
                .as_ref()
                .map_or_else(String::new, |held| host_of(&held.base_url).to_owned());
            ui.label(theme::secondary(format!(
                "Enter this code on a device already signed in to {host}. It is good for a few \
                 minutes."
            )));
            ui.add_space(theme::SPACING_BASE);
            splash::code_tiles(ui, &code);
            ui.add_space(theme::GAP);
            spinner(ui, "Waiting for it to be approved");
        }
        ui.add_space(theme::SPACING_BASE);
        if ui.button("Use a username instead").clicked() {
            self.act(flow, FlowInput::ChoosePassword);
        }
    }

    /// The fallback that trades a username and password for a token. The password
    /// is this panel's only secret, and it leaves the flow when the step is sent.
    fn password_step(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        ui.label(theme::secondary(
            "An account on that server, sent over the connection just made.",
        ));
        ui.add_space(theme::SPACING_BASE);
        ui.horizontal(|ui| {
            ui.label(theme::secondary("Username"));
            ui.add_sized(
                FIELD,
                TextEdit::singleline(&mut flow.username).hint_text("someone"),
            );
        });
        let mut entered = false;
        ui.horizontal(|ui| {
            ui.label(theme::secondary("Password"));
            let response = ui.add_sized(
                FIELD,
                TextEdit::singleline(&mut flow.password)
                    .password(!flow.show_password)
                    .hint_text("their password"),
            );
            entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.toggle_value(&mut flow.show_password, "show");
        });
        ui.add_space(theme::SPACING_BASE);
        let asked = ui.add(theme::primary(RichText::new("Sign in"))).clicked();
        if asked || entered {
            self.act(flow, FlowInput::SubmitPassword);
        }
    }

    /// The add panel's address field. Typing records what it holds — which is
    /// also how a check still in flight gets left behind — and Enter or `Check`
    /// asks the worker what the address is.
    fn address_field(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        let before = flow.address.clone();
        let response = ui.add_sized(
            FIELD,
            TextEdit::singleline(&mut flow.address).hint_text("http://192.168.1.20:8096"),
        );
        if flow.address != before {
            let text = flow.address.clone();
            self.act(flow, FlowInput::Address(text));
        }
        let entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if entered || ui.button("Check").clicked() {
            self.act(flow, FlowInput::SubmitAddress);
        }
    }

    /// One input the panel offered, applied to its flow: the step the flow asked
    /// the worker for is sent; what the flow decided to show is the panel's own.
    fn act(&mut self, flow: &mut AddFlow, input: FlowInput) {
        if let Some(step) = flow.apply(input) {
            self.send(Cmd::Jf(step));
        }
    }

    /// One answer from the worker's Jellyfin desk, given to the panel that asked
    /// for it. An answer with no panel open belongs to a step the user already
    /// left, so it fills nothing and closes nothing.
    pub(super) fn apply_jellyfin(&mut self, answer: JfEv) {
        let input = match answer {
            JfEv::Found(found) => FlowInput::Discovered(found),
            JfEv::Validated(server) => FlowInput::Validated(server),
            JfEv::Code(code) => FlowInput::Code(code),
            JfEv::Added(source) => FlowInput::Added(source),
            JfEv::Failed(failure) => FlowInput::Failed(failure),
        };
        let added = matches!(input, FlowInput::Added(_));
        let Some(flow) = self.add.as_mut() else {
            return;
        };
        flow.apply(input);
        if added || flow.stage == Stage::Added {
            self.add = None;
            // Added from the first-run card: there is something to browse now.
            if matches!(self.screen, Screen::Welcome) {
                self.enter_shell();
            }
        }
    }
}

/// One source: its kind, its name, what makes it work, and what can be done with
/// it from here.
fn source_card(
    ui: &mut egui::Ui,
    origin: &Origin,
    name: &str,
    meta: &str,
    actions: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(theme::SPACING_BASE);
    Frame::new()
        .fill(theme::CARD)
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(Margin::symmetric(16, 12))
        .stroke(Stroke::new(theme::HAIRLINE, theme::CARD_HOVER))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                badge(ui, origin);
                ui.vertical(|ui| {
                    ui.label(theme::title(RichText::new(name).size(theme::H2_SIZE)));
                    ui.add_space(theme::SPACING_BASE);
                    ui.label(theme::meta_text(meta));
                    ui.add_space(theme::SPACING_BASE * 2.0);
                    actions(ui);
                });
            });
        });
}

/// One kind of source as a row to click: its badge, its name, and one line on
/// what adding it involves. Answers whether it was clicked.
fn option_row(ui: &mut egui::Ui, origin: &Origin, title: &str, note: &str) -> bool {
    let hue = hue_of(origin);
    let inner = Frame::new()
        .fill(theme::CARD)
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(Margin::symmetric(14, 12))
        .stroke(Stroke::new(theme::HAIRLINE, theme::CARD_HOVER))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                badge(ui, origin);
                ui.vertical(|ui| {
                    ui.label(theme::title(RichText::new(title).size(theme::H2_SIZE)));
                    ui.label(theme::meta_text(note));
                });
            });
        });
    let response = inner
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_stroke(
            response.rect,
            theme::CARD_RADIUS,
            Stroke::new(theme::HAIRLINE, hue.gamma_multiply(0.7)),
            StrokeKind::Inside,
        );
    }
    response.clicked()
}

/// The hue a service's badge is edged in.
fn hue_of(origin: &Origin) -> Color32 {
    if origin.is_jellyfin() {
        JELLYFIN_HUE
    } else {
        PLEX_HUE
    }
}

/// A source's service as its own mark, centred in a box edged in its hue.
fn badge(ui: &mut egui::Ui, origin: &Origin) {
    let hue = hue_of(origin);
    let (rect, _) = ui.allocate_at_least(BADGE, Sense::hover());
    let radius = CornerRadius::same(8);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, hue.gamma_multiply(0.14));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(theme::HAIRLINE, hue.gamma_multiply(0.55)),
        StrokeKind::Middle,
    );
    let width = marks::source_width(origin, BADGE_MARK);
    marks::paint_source(
        painter,
        origin,
        rect.center().x - width / 2.0,
        rect.center().y,
        BADGE_MARK,
    );
}

/// The working mark, with the repaint that keeps it turning and a wait honest.
fn spinner(ui: &mut egui::Ui, what: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(16.0).color(theme::ACCENT));
        ui.label(theme::meta_text(what));
    });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(200));
}

/// The Plex card's second line: what the token on file makes true right now.
fn plex_meta(server: &str, servers: &[String]) -> String {
    match servers.len() {
        0 if server.is_empty() => "signed in".to_owned(),
        0 => format!("signed in · {server}"),
        1 => "signed in · 1 server".to_owned(),
        n => format!("signed in · {n} servers"),
    }
}

/// The host a base url names, without its scheme or trailing slash: the part a
/// reader recognises as their server's address.
fn host_of(base_url: &str) -> &str {
    base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest)
        .trim_end_matches('/')
}
