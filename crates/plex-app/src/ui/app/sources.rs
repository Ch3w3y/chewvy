//! The Sources screen: every place this app can play from, one card each, and
//! the panels that add one. The first-run choice is drawn from here too, since
//! it offers the same two kinds and runs the same Jellyfin sign-in.
//!
//! The Plex card is there because a Plex token is on file, not because a screen
//! remembers signing in, so signing out anywhere takes it away. A Jellyfin card
//! belongs to this device alone and Remove forgets it here alone — forgetting a
//! token is not a request, which is why a server that is asleep is still
//! removable.

use egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::add_source::{AddFlow, FlowInput, Stage};
use crate::source::Origin;
use crate::sources::JellyfinServer;
use crate::ui::controls::{self, Button, Field, Kind};
use crate::ui::rail::Entry;
use crate::ui::{glass, marks, splash, theme};
use crate::watch::{Cmd, JfCmd, JfEv};

use super::app_settings::{note_panel, section_panel};
use super::{Screen, Ui};

/// A badge's box: wide enough for the Plex wordmark to read at 1x.
const BADGE: Vec2 = Vec2::new(56.0, 36.0);
/// The Plex wordmark's height inside a badge.
const BADGE_MARK: f32 = 15.0;
/// The strengths of the wash a badge is filled and edged in, and of the
/// fill of an option row at rest and pointed at.
const BADGE_FILL: u8 = 18;
const BADGE_EDGE: u8 = 60;
const OPTION_FILL: [u8; 2] = [12, 40];
/// The keychain this build keeps tokens in, by the name its platform uses.
const KEYCHAIN: &str = if cfg!(target_os = "windows") {
    "Windows Credential Manager"
} else if cfg!(target_os = "macos") {
    "macOS Keychain"
} else {
    "system keyring (Secret Service)"
};

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
        // The title scrolls with the cards, so nothing is cut off under it.
        egui::ScrollArea::vertical()
            .id_salt("sources-page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(super::app_settings::COLUMN);
                crate::ui::legible::label(ui, RichText::new("Sources").heading());
                crate::ui::legible::label(
                    ui,
                    theme::secondary("Accounts and servers this app can play from"),
                );
                self.sources_list(ui);
            });
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
            ui.add_space(theme::GAP);
            let hover = if self.sources.plex.is_some() {
                "Add a Jellyfin server"
            } else {
                "Add a Plex account or a Jellyfin server"
            };
            if Button::new(Kind::Secondary, "Add a source")
                .hint(hover)
                .show(ui)
                .clicked()
            {
                self.add_source();
            }
        }
        self.kind_panel(ui);
        self.add_panel(ui, true);
        let mut note = format!(
            "Tokens live in the {KEYCHAIN}. Removing a source deletes its token from this \
             device only, so a server that is asleep or off-network can still be forgotten."
        );
        if self.sources.plex.is_some() {
            note.insert_str(
                0,
                "One Plex account at a time: sign out of it to link another. ",
            );
        }
        note_panel(ui, &note);
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
            ui.add_space(theme::SPACING_BASE);
            if Button::new(Kind::Quiet, "Cancel")
                .hint("Stop adding a source")
                .show(ui)
                .clicked()
            {
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
            self.add_panel(ui, false);
            return;
        }
        splash::masthead(ui);
        splash::heading(
            ui,
            "Where are your films?",
            "Add one now. You can add the other later.",
        );
        self.kind_options(ui);
        if !status.trim().is_empty() {
            ui.add_space(theme::GAP);
            controls::working(ui, status);
        }
    }

    /// The Plex card: a source because a Plex token is on file. Its account's
    /// servers are listed under it, the same ones Settings switched between.
    fn plex_card(&mut self, ui: &mut egui::Ui) {
        let servers = self.server_choices.clone();
        let meta = plex_meta(&self.server, &servers);
        source_card(ui, &Origin::Plex, "Plex account", &meta, |ui| {
            ui.horizontal_wrapped(|ui| {
                if Button::new(Kind::Secondary, "Refresh servers")
                    .hint("Ask this account which servers it has")
                    .show(ui)
                    .clicked()
                {
                    self.send(Cmd::Servers);
                }
                if Button::new(Kind::Secondary, "Sign out")
                    .hint("Forget this Plex account on this device")
                    .show(ui)
                    .clicked()
                {
                    self.send(Cmd::SignOut);
                }
            });
            ui.add_space(theme::SPACING_BASE);
            if servers.is_empty() {
                ui.label(theme::secondary(
                    "Its servers are listed once you ask for them.",
                ));
            } else {
                ui.label(theme::secondary("Or switch to another of its servers"));
                for (index, name) in servers.iter().enumerate() {
                    if splash::server_row(ui, name) {
                        self.status = format!("Connecting to {name}…");
                        self.send(Cmd::Connect(index));
                    }
                }
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
            if Button::new(Kind::Secondary, "Remove")
                .hint("Forget this server on this device, its token included")
                .show(ui)
                .clicked()
            {
                self.send(Cmd::Jf(JfCmd::Remove { server_id }));
            }
        });
    }

    /// The panel that adds a Jellyfin server, at whichever step it reached:
    /// a glass section on the Sources screen (`framed`), or straight on the
    /// first-run card, which already says what it is for.
    ///
    /// The flow is taken out of the screen for the length of the panel so a
    /// button in it can ask the worker for a step. Both Cancel and `Added` close
    /// it, and both tell the worker the server it was working on is done with:
    /// otherwise a wait nobody is watching would run on to its own timeout.
    fn add_panel(&mut self, ui: &mut egui::Ui, framed: bool) {
        let Some(mut flow) = self.add.take() else {
            return;
        };
        let stage = flow.stage;
        let mut closed = false;
        let mut steps = |ui: &mut egui::Ui| {
            self.add_steps(&mut flow, ui);
            ui.add_space(theme::SPACING_BASE);
            if Button::new(Kind::Quiet, "Cancel")
                .hint("Stop adding a source and forget it")
                .show(ui)
                .clicked()
            {
                closed = true;
            }
        };
        if framed {
            section_panel(ui, "Add a Jellyfin server", steps);
        } else {
            steps(ui);
        }
        if closed || stage == Stage::Added {
            self.send(Cmd::Jf(JfCmd::Dismiss));
            self.add = None;
        } else {
            self.add = Some(flow);
        }
    }

    /// The step the flow is at, then its error if the last step failed.
    fn add_steps(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        let error = flow.error.clone();
        match flow.stage {
            Stage::Address => self.address_step(flow, ui),
            Stage::Checking => self.checking_step(flow, ui),
            Stage::Ready => self.ready_step(flow, ui),
            Stage::Pairing => self.pairing_step(flow, ui),
            Stage::Password => self.password_step(flow, ui),
            Stage::SigningIn => controls::working(ui, "Signing in\u{2026}"),
            // The source is already in the list above; the panel is leaving.
            Stage::Added => {}
        }
        if let Some(failure) = error {
            ui.add_space(theme::SPACING_BASE);
            ui.label(
                RichText::new(failure.message())
                    .color(theme::palette().danger)
                    .size(theme::META_SIZE),
            );
        }
    }

    /// The step that asks where the server is: the neighbours the network named,
    /// and the address field for everything else.
    fn address_step(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        ui.label(theme::secondary(
            "Pick a server found nearby, or type its address.",
        ));
        ui.add_space(theme::SPACING_BASE);
        let found = flow.discovered.clone();
        for found in &found {
            let address = found.address.clone();
            if splash::server_row(ui, &found.name) {
                self.act(flow, FlowInput::Address(address));
                self.act(flow, FlowInput::SubmitAddress);
            }
        }
        if flow.discovering {
            controls::working(ui, "Asking the network\u{2026}");
        } else if Button::new(Kind::Quiet, "Look on the network again")
            .hint("Ask the network which servers answer")
            .show(ui)
            .clicked()
        {
            self.act(flow, FlowInput::Discover);
        }
        self.address_field(flow, ui, true);
    }

    /// The step that waits for an address to say what it is, still showing the
    /// field: typing over it is how that wait is left behind.
    fn checking_step(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui) {
        let address = flow.address.clone();
        controls::working(ui, &format!("Checking {address}\u{2026}"));
        ui.label(theme::secondary("Typing another address asks again."));
        self.address_field(flow, ui, false);
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
        ui.label(theme::secondary("How do you want to sign in?"));
        ui.add_space(theme::SPACING_BASE);
        ui.spacing_mut().item_spacing.y = theme::SPACING_BASE * 3.0;
        if Button::new(Kind::Primary, "Sign in with a code")
            .wide()
            .show(ui)
            .clicked()
        {
            self.act(flow, FlowInput::ChooseQuickConnect);
        }
        if Button::new(Kind::Secondary, "Username and password")
            .wide()
            .show(ui)
            .clicked()
        {
            self.act(flow, FlowInput::ChoosePassword);
        }
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
            controls::working(ui, "Getting the code");
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
            controls::working(ui, "Waiting for it to be approved");
        }
        ui.add_space(theme::SPACING_BASE);
        if Button::new(Kind::Quiet, "Use a username instead")
            .show(ui)
            .clicked()
        {
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
        // Each label sits over its field, both on the card's left edge, so
        // the two fields line up and no row is wider than the card.
        let entered = ui
            .with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = theme::SPACING_BASE * 1.5;
                ui.label(theme::secondary("Username"));
                Field::new(&mut flow.username, "someone", "jf-username").show(ui);
                ui.add_space(theme::SPACING_BASE);
                ui.label(theme::secondary("Password"));
                Field::new(&mut flow.password, "their password", "jf-password")
                    .password(&mut flow.show_password)
                    .show(ui)
                    .submitted
            })
            .inner;
        ui.add_space(theme::SPACING_BASE * 2.0);
        let asked = Button::new(Kind::Primary, "Sign in")
            .wide()
            .show(ui)
            .clicked();
        if asked || entered {
            self.act(flow, FlowInput::SubmitPassword);
        }
    }

    /// The add panel's address field. Typing records what it holds — which is
    /// also how a check still in flight gets left behind — and Enter or `Check`
    /// asks the worker what the address is.
    fn address_field(&mut self, flow: &mut AddFlow, ui: &mut egui::Ui, check: bool) {
        let before = flow.address.clone();
        let answer =
            Field::new(&mut flow.address, "http://192.168.1.20:8096", "jf-address").show(ui);
        if flow.address != before {
            let text = flow.address.clone();
            self.act(flow, FlowInput::Address(text));
        }
        let asked = check
            && Button::new(Kind::Primary, "Check")
                .wide()
                .show(ui)
                .clicked();
        if answer.submitted || asked {
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

/// One source on a glass card: its kind, its name, what makes it work, and
/// what can be done with it from here.
fn source_card(
    ui: &mut egui::Ui,
    origin: &Origin,
    name: &str,
    meta: &str,
    actions: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(theme::GAP);
    let slot = glass::reserve(ui.painter());
    let card = Frame::NONE
        .inner_margin(Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                badge(ui, origin);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = theme::SPACING_BASE;
                    ui.label(theme::title(RichText::new(name).size(theme::H2_SIZE)));
                    ui.label(theme::meta_text(meta));
                    ui.add_space(theme::SPACING_BASE * 2.0);
                    actions(ui);
                });
            });
        });
    glass::fill(
        ui.painter(),
        slot,
        card.response.rect,
        super::app_settings::SECTION_RADIUS,
        theme::current().glass.section,
    );
}

/// One kind of source as a row to click: its badge, its name, and one line on
/// what adding it involves. Answers whether it was clicked.
fn option_row(ui: &mut egui::Ui, origin: &Origin, title: &str, note: &str) -> bool {
    let fill = ui.painter().add(egui::Shape::Noop);
    let inner = Frame::NONE
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                badge(ui, origin);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(theme::title(RichText::new(title).size(theme::H2_SIZE)));
                    ui.label(theme::meta_text(note));
                });
            });
        });
    let response = ui
        .interact(
            inner.response.rect,
            ui.id().with(("source-kind", title)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let [rest, hover] = OPTION_FILL;
    let wash = if response.hovered() || response.has_focus() {
        hover
    } else {
        rest
    };
    ui.painter().set(
        fill,
        egui::epaint::RectShape::filled(
            response.rect,
            theme::CARD_RADIUS,
            theme::palette().wash(wash),
        ),
    );
    controls::ring_when_focused(
        ui.painter(),
        &response,
        response.rect,
        f32::from(theme::CARD_RADIUS_U8),
    );
    response.clicked()
}

/// A source's service as its own mark, centred in a quiet box: the mark is
/// the only thing on the page that carries the service's colours.
fn badge(ui: &mut egui::Ui, origin: &Origin) {
    let (rect, _) = ui.allocate_at_least(BADGE, Sense::hover());
    let radius = CornerRadius::same(8);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::palette().wash(BADGE_FILL));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(theme::HAIRLINE, theme::palette().wash(BADGE_EDGE)),
        StrokeKind::Inside,
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
