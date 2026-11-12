//! The screens' own tests: back history, the paging offset a next page is
//! asked at, and the answers a connected screen files.

use std::sync::mpsc;

use plex_core::library::Item;

use super::*;

fn section(id: &str, title: &str) -> Entry {
    Entry::Section {
        id: id.to_owned(),
        title: title.to_owned(),
    }
}

#[test]
fn back_returns_to_the_entry_the_user_left_last() {
    let mut history = History::default();
    history.leave(&section("11", "Movies"));
    history.leave(&Entry::Search);
    assert_eq!(history.back(), Some(Entry::Search));
    assert_eq!(history.back(), Some(section("11", "Movies")));
    assert_eq!(history.back(), None);
}

#[test]
fn staying_on_one_entry_does_not_fill_the_history() {
    let mut history = History::default();
    for _ in 0..5 {
        history.leave(&Entry::Home);
    }
    assert_eq!(history.stack.len(), 1);
}

fn item(kind: &str, title: &str) -> Item {
    Item {
        rating_key: Some("42".to_owned()),
        kind: Some(kind.to_owned()),
        title: Some(title.to_owned()),
        duration: Some(7_380_000),
        ..Item::default()
    }
}

fn cards(n: usize) -> Vec<grid::Card> {
    (0..n)
        .map(|_| grid::Card::from_item(&item("movie", "Heat")))
        .collect()
}

fn ask(start: usize) -> Ask {
    Ask::Section {
        section: "11".to_owned(),
        start,
        sort: Some("titleSort".to_owned()),
        unwatched: false,
    }
}

/// A [`Ui`] with no worker: its sends go nowhere and nothing arrives.
fn ui_without_worker() -> Ui {
    let (cmd, _cmd_rx) = mpsc::channel();
    let (_ev_tx, ev) = mpsc::channel();
    Ui::new(cmd, ev)
}

#[test]
fn the_next_page_starts_where_the_served_page_stopped() {
    let mut ui = ui_without_worker();
    ui.asked = Some(ask(0));
    ui.apply_cards(cards(60), false, true, Some(1_654), ask(0));
    assert_eq!(ui.card_start, 60);
    ui.asked = Some(ask(60));
    // The server served 23 rows where [`grid::PAGE`] were asked for, so the
    // next ask continues at the served offset plus those 23, not at 60 + PAGE
    // and not at the rows still in view after an answer was dropped.
    ui.apply_cards(cards(23), true, true, Some(1_654), ask(60));
    assert_eq!(ui.card_start, 83);
    assert_eq!(ui.cards.len(), 83);
    assert_ne!(ui.card_start, 60 + grid::PAGE);
    assert_eq!(ui.card_total, Some(1_654));
    // An answer is measured against the ask it answers: its own offset plus
    // the rows it filled, not the number of rows left in view.
    ui.asked = Some(ask(200));
    ui.apply_cards(cards(20), true, true, Some(1_654), ask(200));
    assert_eq!(ui.card_start, 220);
    assert_eq!(ui.cards.len(), 103);
}

#[test]
fn a_failed_page_ends_the_page_in_flight() {
    let mut ui = ui_without_worker();
    ui.card_paging = true;
    ui.apply(Ev::Error("browse failed".to_owned()));
    assert!(!ui.card_paging);
}

#[test]
fn an_answer_for_another_ask_fills_nothing() {
    let mut ui = ui_without_worker();
    ui.asked = Some(ask(0));
    ui.apply_cards(cards(60), false, true, Some(1_654), ask(120));
    assert!(ui.cards.is_empty());
    assert_eq!(ui.card_start, 0);
}

/// Settings' server switch lists the account's servers without leaving the
/// connected shell, so the pre-connect picker is not the only way to name them.
#[test]
fn a_connected_server_lists_the_account_servers_without_leaving_its_shell() {
    let mut ui = ui_without_worker();
    ui.apply(Ev::Sections {
        server: "Family Plex".to_owned(),
        sections: vec![("4".to_owned(), "Movies".to_owned())],
    });
    ui.apply(Ev::Servers(vec![
        "Family Plex".to_owned(),
        "Study Plex".to_owned(),
    ]));
    assert!(matches!(ui.screen, Screen::Connected));
    assert_eq!(ui.server_choices, vec!["Family Plex", "Study Plex"]);
}

/// A registry holding one Jellyfin server and, when asked, a Plex account.
fn sources(plex: bool) -> crate::sources::Sources {
    let mut held = crate::sources::Sources::default();
    held.set_jellyfin(crate::sources::JellyfinServer {
        server_id: "loft".to_owned(),
        name: "Loft".to_owned(),
        ..Default::default()
    });
    if plex {
        held.record_plex(None);
    }
    held
}

#[test]
fn with_nothing_on_file_the_first_run_choice_opens() {
    let mut ui = ui_without_worker();
    ui.sources = crate::sources::Sources::default();
    ui.apply(Ev::NoPlex);
    assert!(matches!(ui.screen, Screen::Welcome));
}

#[test]
fn a_jellyfin_server_on_file_opens_the_shell_without_a_plex_account() {
    let mut ui = ui_without_worker();
    ui.sources = sources(false);
    ui.apply(Ev::NoPlex);
    assert!(matches!(ui.screen, Screen::Connected));
    assert_eq!(ui.current, Some(Entry::Home));
}

#[test]
fn signing_out_of_plex_keeps_browsing_what_is_left() {
    let mut ui = ui_without_worker();
    ui.sources = sources(false);
    ui.apply(Ev::Sections {
        server: "Attic".to_owned(),
        sections: vec![("1".to_owned(), "Films".to_owned())],
    });
    ui.apply(Ev::SignedOut);
    assert!(matches!(ui.screen, Screen::Connected));
    assert!(ui.sections.is_empty(), "the Plex libraries leave the rail");
    assert!(ui.server.is_empty());

    ui.sources = crate::sources::Sources::default();
    ui.apply(Ev::SignedOut);
    assert!(matches!(ui.screen, Screen::Welcome));
}

#[test]
fn removing_the_last_source_brings_the_choice_back() {
    let mut ui = ui_without_worker();
    ui.sources = sources(false);
    ui.apply(Ev::NoPlex);
    ui.apply(Ev::Sources(crate::sources::Sources::default()));
    assert!(matches!(ui.screen, Screen::Welcome));
}

#[test]
fn adding_a_source_asks_which_kind_only_while_no_plex_account_is_linked() {
    let mut ui = ui_without_worker();
    ui.sources = sources(false);
    ui.apply(Ev::NoPlex);
    ui.add_source();
    assert_eq!(ui.current, Some(Entry::Sources));
    assert!(ui.choosing && ui.add.is_none());

    let mut ui = ui_without_worker();
    ui.sources = sources(true);
    ui.apply(Ev::NoPlex);
    ui.add_source();
    assert!(!ui.choosing, "one Plex account is all the sign-in holds");
    assert!(ui.add.is_some(), "so the Jellyfin panel opens directly");
}

#[test]
fn a_sign_in_code_takes_the_window_and_back_leaves_it() {
    let mut ui = ui_without_worker();
    ui.sources = crate::sources::Sources::default();
    ui.apply(Ev::NoPlex);
    ui.apply(Ev::Pairing {
        code: "K7QF".to_owned(),
        url: "https://plex.tv/link".to_owned(),
    });
    assert!(matches!(ui.screen, Screen::Pairing { .. }));
    ui.open_without_plex();
    assert!(matches!(ui.screen, Screen::Welcome));
}
