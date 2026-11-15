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

/// Enter in the rail's field opens Search from any page, even when the field
/// still holds the query it last reported, and asks for that query at once.
#[test]
fn enter_in_the_rail_field_opens_search_with_an_unchanged_query() {
    let (cmd, sent) = mpsc::channel();
    let (_ev_tx, ev) = mpsc::channel();
    let mut ui = Ui::new(cmd, ev);
    ui.go(Entry::Sources);
    ui.rail_search("zzzz", true);
    assert_eq!(ui.current, Some(Entry::Search));
    let asked: Vec<String> = sent
        .try_iter()
        .filter_map(|cmd| match cmd {
            Cmd::SearchHubs(query) => Some(query),
            _ => None,
        })
        .collect();
    assert_eq!(asked, vec!["zzzz".to_owned()]);
}

/// A query stays in the rail only while Search, or a result opened from it,
/// is on screen: going anywhere else empties the field.
#[test]
fn leaving_search_empties_the_rail_field_unless_a_result_was_opened() {
    let (cmd, _sent) = mpsc::channel();
    let (_ev_tx, ev) = mpsc::channel();
    let mut ui = Ui::new(cmd, ev);
    ui.query = "lord".to_owned();
    ui.rail_search("lord", true);
    ui.go(Entry::Detail {
        id: "1".to_owned(),
        title: "The Lord of the Rings".to_owned(),
    });
    assert_eq!(ui.query, "lord");
    ui.go(Entry::Search);
    ui.go(Entry::Settings);
    assert!(ui.query.is_empty());
}

/// One Search frame: the repaint delay it asked for, and every text it drew.
fn search_frame(ui: &mut Ui, ctx: &egui::Context) -> (std::time::Duration, Vec<String>) {
    let mut output = ctx.run_ui(egui::RawInput::default(), |root| {
        ui.draw_search(root);
    });
    output.textures_delta.clear();
    let delay = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(std::time::Duration::MAX, |v| v.repaint_delay);
    let texts = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    (delay, texts)
}

fn searching(query: &str) -> Ui {
    let (cmd, _sent) = mpsc::channel();
    let (_ev_tx, ev) = mpsc::channel();
    let mut ui = Ui::new(cmd, ev);
    ui.query = query.to_owned();
    ui.rail_search(query, true);
    ui
}

/// A search the server could not answer says so, and the page stops
/// repainting: a spinner waiting on an answer that is not coming would
/// repaint every frame for as long as the page is open.
#[test]
fn a_failed_search_says_why_and_stops_spinning() {
    let ctx = egui::Context::default();
    crate::ui::theme::apply(&ctx);
    let mut ui = searching("dune");
    ui.apply(Ev::SearchFailed {
        query: "dune".to_owned(),
        error: "The server did not answer: 503".to_owned(),
    });
    assert!(!ui.search.waiting());
    for _ in 0..3 {
        search_frame(&mut ui, &ctx);
    }
    let (delay, texts) = search_frame(&mut ui, &ctx);
    assert!(
        delay > std::time::Duration::from_millis(16),
        "no spinner repaints the page: {delay:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("Could not search for")),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t.contains("503")), "{texts:?}");
}

/// A failure for a query the field no longer holds files nothing.
#[test]
fn a_failure_for_an_older_query_is_ignored() {
    let mut ui = searching("dune");
    ui.apply(Ev::SearchFailed {
        query: "dun".to_owned(),
        error: "gone".to_owned(),
    });
    assert!(ui.search.waiting());
}

/// While a newer query is on its way, the results on screen keep the name
/// of the query they answer.
#[test]
fn results_are_headed_by_the_query_they_answer() {
    let ctx = egui::Context::default();
    crate::ui::theme::apply(&ctx);
    let mut ui = searching("dune");
    let hub = Hub {
        kind: Some("movie".to_owned()),
        title: Some("Movies".to_owned()),
        items: vec![item("movie", "Dune")],
        ..Hub::default()
    };
    ui.apply(Ev::SearchHubs {
        query: "dune".to_owned(),
        hubs: vec![hub],
    });
    let (_, texts) = search_frame(&mut ui, &ctx);
    assert!(
        texts
            .iter()
            .any(|t| t == "Results for \u{201c}dune\u{201d}"),
        "{texts:?}"
    );
    ui.query = "dune 2".to_owned();
    let (_, texts) = search_frame(&mut ui, &ctx);
    assert!(
        texts
            .iter()
            .any(|t| t == "Results for \u{201c}dune\u{201d}"),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|t| t.contains("dune 2")), "{texts:?}");
}
