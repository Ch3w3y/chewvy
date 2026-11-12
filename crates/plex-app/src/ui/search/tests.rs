//! Search grouping and captions read over the captured `GET /hubs/search`
//! answer and the captured episode `/metadata` answer: which hubs become a
//! section, and the one line an episode row is read as.

use plex_core::library::{Hub, Item};
use serde_json::Value;

use super::*;

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../plex-core/tests/fixtures/real/{name}"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn text(row: &Value, field: &str) -> Option<String> {
    row.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn number(row: &Value, field: &str) -> Option<u64> {
    row.get(field).and_then(Value::as_u64)
}

fn flag(row: &Value, field: &str) -> Option<bool> {
    row.get(field).and_then(Value::as_bool)
}

/// One `Metadata` row of a captured answer, as [`Item`] reads it.
fn wire_item(row: &Value) -> Item {
    Item {
        rating_key: text(row, "ratingKey"),
        key: text(row, "key"),
        kind: text(row, "type"),
        title: text(row, "title"),
        index: number(row, "index"),
        parent_index: number(row, "parentIndex"),
        parent_title: text(row, "parentTitle"),
        grandparent_title: text(row, "grandparentTitle"),
        thumb: text(row, "thumb"),
        duration: number(row, "duration"),
        ..Item::default()
    }
}

/// The `Hub` rows of a captured hub answer, with the names that capture uses.
fn wire_hubs(name: &str) -> Vec<Hub> {
    fixture(name)
        .get("MediaContainer")
        .and_then(|container| container.get("Hub"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| panic!("{name}: no Hub array"))
        .iter()
        .map(|hub| Hub {
            hub_id: text(hub, "hubIdentifier"),
            path: text(hub, "key"),
            item_keys: text(hub, "hubKey"),
            title: text(hub, "title"),
            kind: text(hub, "type"),
            available: number(hub, "size").map(|size| size as usize),
            context: text(hub, "context"),
            style: text(hub, "style"),
            more: flag(hub, "more"),
            promoted: flag(hub, "promoted"),
            items: hub
                .get("Metadata")
                .and_then(Value::as_array)
                .map(|rows| rows.iter().map(wire_item).collect())
                .unwrap_or_default(),
        })
        .collect()
}

fn metadata_row(name: &str) -> Item {
    fixture(name)
        .get("MediaContainer")
        .and_then(|container| container.get("Metadata"))
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .map(wire_item)
        .unwrap_or_else(|| panic!("{name}: no Metadata row"))
}

#[test]
fn a_search_answer_groups_the_hubs_that_answered_with_rows() {
    let groups = group(&wire_hubs("hubs_search.json"));
    let labels: Vec<(String, usize)> = groups
        .iter()
        .map(|group| (group.title.clone(), group.cards.len()))
        .collect();
    assert_eq!(
        labels,
        vec![("Movies".to_owned(), 3), ("Shows".to_owned(), 1),]
    );
}

#[test]
fn a_hub_the_server_found_nothing_for_draws_no_section() {
    // The captured answer names an `artist` hub with `size` 0 and no `Metadata`
    // array at all.
    let hubs = wire_hubs("hubs_search.json");
    let artist = hubs
        .iter()
        .find(|hub| hub.kind.as_deref() == Some("artist"))
        .unwrap_or_else(|| panic!("the capture names an artist hub"));
    assert!(artist.items.is_empty());
    assert!(group(&hubs).iter().all(|group| group.title != "Artists"));
}

#[test]
fn a_search_group_is_drawn_as_posters_except_for_a_people_hub() {
    let groups = group(&wire_hubs("hubs_search.json"));
    assert!(groups.iter().all(|group| !group.people));
    let people = Hub {
        kind: Some("artist".to_owned()),
        title: Some("Artists".to_owned()),
        items: vec![wire_item(
            &serde_json::json!({ "ratingKey": "1", "type": "artist", "title": "Yo-Yo Ma" }),
        )],
        ..Hub::default()
    };
    let grouped = group(&[people]);
    assert!(grouped.first().is_some_and(|group| group.people));
}

#[test]
fn an_episode_is_captioned_with_its_show_not_its_season() {
    let episode = metadata_row("episode_metadata.json");
    assert_eq!(caption(&episode), "The 404 · S1E1 · Pilot");
    assert_eq!(
        caption(&wire_item(
            &serde_json::json!({ "type": "movie", "title": "Heat" })
        )),
        "Heat"
    );
}

#[test]
fn a_caption_leaves_out_the_parts_an_answer_does_not_name() {
    let bare = wire_item(&serde_json::json!({ "type": "episode", "title": "Untitled" }));
    assert_eq!(caption(&bare), "Untitled");
}

#[test]
fn an_episode_row_in_a_search_section_carries_its_caption() {
    let season = metadata_row("season_children_episodes.json");
    let hub = Hub {
        kind: Some("episode".to_owned()),
        title: Some("Episodes".to_owned()),
        items: vec![season],
        ..Hub::default()
    };
    let cards = group(&[hub])
        .first()
        .map(|group| group.cards.clone())
        .unwrap_or_default();
    let card = cards
        .first()
        .unwrap_or_else(|| panic!("the section holds the row"));
    assert!(card.title.starts_with("The 404 · S1E1 · "));
    assert!(card.subtitle.is_empty());
    assert!(card.playable());
}

#[test]
fn a_people_circle_is_labelled_with_two_initials() {
    assert_eq!(initials("Milo Larkspur"), "ML");
    assert_eq!(initials("Cher"), "C");
    assert_eq!(initials(""), "");
}
