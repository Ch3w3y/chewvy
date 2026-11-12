//! Grid geometry and the values it browses with, read over the captured
//! section answers: columns per available width, the unwatched counts a show or
//! season row names, and the sort values a browse request is asked for.
//!
//! The wire names here are the names those captures use.

use plex_core::library::{self, Item, SortOption};
use serde_json::Value;

use super::*;
use crate::ui::{cards, rail};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../plex-core/tests/fixtures/real/{name}"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn rows(name: &str) -> Vec<Value> {
    fixture(name)
        .get("MediaContainer")
        .and_then(|container| container.get("Metadata"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| panic!("{name}: no Metadata array"))
}

fn text(row: &Value, field: &str) -> Option<String> {
    row.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn number(row: &Value, field: &str) -> Option<u64> {
    row.get(field).and_then(Value::as_u64)
}

/// One `Metadata` row of a captured section or children answer, as
/// [`Card::from_item`] reads it.
fn wire_item(row: &Value) -> Item {
    Item {
        rating_key: text(row, "ratingKey"),
        kind: text(row, "type"),
        title: text(row, "title"),
        thumb: text(row, "thumb"),
        index: number(row, "index"),
        parent_index: number(row, "parentIndex"),
        year: number(row, "year"),
        leaf_count: number(row, "leafCount"),
        viewed_leaf_count: number(row, "viewedLeafCount"),
        view_offset: number(row, "viewOffset"),
        ..Item::default()
    }
}

/// The sorts the captured films `/sorts` answer offers.
fn captured_sorts() -> Vec<SortOption> {
    fixture("films_sorts.json")
        .get("MediaContainer")
        .and_then(|container| container.get("Directory"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("sorts: no Directory array"))
        .iter()
        .map(|row| SortOption {
            key: text(row, "key").unwrap_or_else(|| panic!("sorts row with no key")),
            title: text(row, "title"),
            desc_key: text(row, "descKey"),
            default_direction: text(row, "defaultDirection"),
            first_character_path: text(row, "firstCharacterKey"),
        })
        .collect()
}

#[test]
fn column_count_follows_the_available_width() {
    assert_eq!(columns(800.0, PosterSize::Medium), 4);
    assert_eq!(columns(1280.0, PosterSize::Medium), 7);
    assert_eq!(columns(1920.0, PosterSize::Medium), 11);
    assert_eq!(columns(300.0, PosterSize::Medium), 1);
    assert_eq!(columns(0.0, PosterSize::Medium), 1);
    assert_eq!(columns(-16.0, PosterSize::Medium), 1);
    assert_eq!(columns(f32::NAN, PosterSize::Medium), 1);
    // A smaller poster band is never a smaller column count.
    assert_eq!(columns(800.0, PosterSize::Small), 6);
    assert_eq!(columns(800.0, PosterSize::Large), 3);
}

#[test]
fn each_poster_size_lays_out_inside_its_own_band() {
    for size in PosterSize::ALL {
        let [min, max] = band(size);
        assert!(min < max);
        for width in [420.0f32, 800.0, 1280.0, 1920.0, 2560.0, 4000.0] {
            let card = card_width(width, size);
            assert!((min..=max).contains(&card), "{size:?} {width} -> {card}");
        }
    }
    // Every band the setting offers is a narrower maximum than the last.
    let bands: Vec<Vec<f32>> = PosterSize::ALL.iter().map(|s| band(*s).to_vec()).collect();
    assert_eq!(
        bands,
        vec![vec![110.0, 150.0], vec![160.0, 200.0], vec![210.0, 280.0]]
    );
}

#[test]
fn cards_stay_in_their_width_band_and_fit_their_row() {
    for size in PosterSize::ALL {
        let [min, max] = band(size);
        for width in [800.0f32, 1280.0, 1920.0, 2560.0, 4000.0] {
            let columns = columns(width, size);
            let card = card_width(width, size);
            assert!((min..=max).contains(&card), "{size:?} {width} -> {card}");
            let row = columns as f32 * card + (columns - 1) as f32 * GAP;
            assert!(row <= width + 0.5, "{width}: {columns} x {card} = {row}");
            // A 2:3 poster over a card title block that never crowds out the poster.
            let poster = poster_size(card);
            assert_eq!(poster, egui::vec2(card, card * POSTER_RATIO));
            assert!(poster.y + CAPTION_HEIGHT > poster.y);
        }
    }
}

#[test]
fn arrows_move_the_selection_by_one_card_and_by_one_row() {
    // 7 cards in 3 columns: 0 1 2 / 3 4 5 / 6.
    let next = |current: Option<usize>, arrow: Arrow| move_focus(3, 7, current, arrow);
    assert_eq!(next(None, Arrow::Right), Some(0));
    assert_eq!(next(None, Arrow::Down), Some(0));
    assert_eq!(next(Some(0), Arrow::Right), Some(1));
    assert_eq!(next(Some(1), Arrow::Left), Some(0));
    assert_eq!(next(Some(0), Arrow::Left), None);
    assert_eq!(next(Some(6), Arrow::Right), None);
    assert_eq!(next(Some(2), Arrow::Down), Some(5));
    assert_eq!(next(Some(5), Arrow::Up), Some(2));
    assert_eq!(next(Some(6), Arrow::Down), None);
    assert_eq!(next(Some(0), Arrow::Down), Some(3));
    // A selection past the last card is stale, so no arrow rescues it.
    assert_eq!(next(Some(9), Arrow::Right), None);
    // Nothing to select.
    assert_eq!(move_focus(3, 0, None, Arrow::Right), None);
}

#[test]
fn show_cards_carry_the_unwatched_count_the_section_row_names() {
    let rows = rows("tv_all.json");
    let counts: Vec<(String, Option<u64>)> = rows
        .iter()
        .map(|row| {
            let card = Card::from_item(&wire_item(row));
            (card.title.clone(), card.unwatched)
        })
        .collect();
    // `leafCount` less `viewedLeafCount`, as tv_all.json names both.
    assert_eq!(
        counts,
        vec![
            ("The 404".to_owned(), Some(100)),
            ("Aphelion: Ember".to_owned(), Some(8)),
            ("All His Folly".to_owned(), Some(8)),
        ]
    );
    for row in &rows {
        let card = Card::from_item(&wire_item(row));
        assert_eq!(card.kind, "show");
        assert!(card.browsable() && !card.playable());
        let count = card
            .unwatched
            .unwrap_or_else(|| panic!("{}: no badge", card.title));
        assert_eq!(marker(&card, false), Marker::Count(count));
    }
}

#[test]
fn season_cards_count_their_own_episodes() {
    for row in rows("show_children_seasons.json") {
        let card = Card::from_item(&wire_item(&row));
        let leaf =
            number(&row, "leafCount").unwrap_or_else(|| panic!("season row with no leafCount"));
        let unwatched = leaf - number(&row, "viewedLeafCount").unwrap_or(0);
        assert_eq!(card.kind, "season");
        assert_eq!(card.unwatched, Some(unwatched));
        assert_eq!(card.subtitle, format!("{leaf} episodes"));
        assert_eq!(marker(&card, false), Marker::Count(unwatched));
    }
}

#[test]
fn movie_rows_get_no_badge_and_no_dot_until_the_server_filters_them() {
    let rows = rows("films_all.json");
    for row in &rows {
        let card = Card::from_item(&wire_item(row));
        assert_eq!(card.kind, "movie");
        assert!(card.playable() && !card.browsable());
        // No captured section answer names leafCount or viewCount for a film.
        assert_eq!(card.unwatched, None);
        assert_eq!(marker(&card, false), Marker::None);
        // Under the server's own unwatched filter the same row is marked.
        assert_eq!(marker(&card, true), Marker::Dot);
        assert_eq!(
            card.subtitle,
            number(row, "year")
                .map(|year| year.to_string())
                .unwrap_or_else(|| panic!("{}: no year", card.title))
        );
    }
}

#[test]
fn an_episode_card_shows_its_season_and_episode_number() {
    for row in rows("season_children_episodes.json") {
        let card = Card::from_item(&wire_item(&row));
        assert_eq!(card.kind, "episode");
        assert!(card.playable());
        assert_eq!(
            card.subtitle,
            format!(
                "S{} E{}",
                number(&row, "parentIndex")
                    .unwrap_or_else(|| panic!("episode row with no parentIndex")),
                number(&row, "index").unwrap_or_else(|| panic!("episode row with no index"))
            )
        );
        assert_eq!(card.unwatched, None);
    }
}

#[test]
fn a_resume_offset_arrives_in_seconds() {
    let mut item = wire_item(&serde_json::json!({"type": "movie", "title": "Heat"}));
    item.view_offset = Some(2_500_000);
    assert_eq!(Card::from_item(&item).start, Some(2500.0));
    assert_eq!(Card::from_item(&item.with_offset(0)).start, None);
}

trait WithOffset {
    fn with_offset(self, offset: u64) -> Self;
}

impl WithOffset for Item {
    fn with_offset(mut self, offset: u64) -> Self {
        self.view_offset = Some(offset);
        self
    }
}

#[test]
fn a_section_starts_on_its_own_title_sort() {
    let sorts = captured_sorts();
    assert_eq!(default_sort(&sorts).as_deref(), Some("titleSort"));
    assert_eq!(sort_label(Some("titleSort"), &sorts), "Sort: Title");
    // Without a titleSort the first named sort is used in its own direction.
    let no_title: Vec<SortOption> = sorts
        .iter()
        .filter(|sort| sort.key != "titleSort")
        .cloned()
        .collect();
    assert_eq!(
        default_sort(&no_title).as_deref(),
        Some("originallyAvailableAt:desc")
    );
    assert_eq!(default_sort(&[]), None);
    assert_eq!(sort_label(None, &sorts), "Sort");
}

#[test]
fn the_sort_chip_alternates_direction_and_sends_a_validated_value() {
    let sorts = captured_sorts();
    let title = sorts
        .iter()
        .find(|sort| sort.key == "titleSort")
        .unwrap_or_else(|| panic!("captured sorts name no titleSort"));
    let release = sorts
        .iter()
        .find(|sort| sort.key == "originallyAvailableAt")
        .unwrap_or_else(|| panic!("captured sorts name no originallyAvailableAt"));

    assert_eq!(
        sort_value(title, Some("titleSort")).as_deref(),
        Some("titleSort:desc")
    );
    assert_eq!(
        sort_value(title, Some("titleSort:desc")).as_deref(),
        Some("titleSort")
    );
    // A sort the section says it is normally shown descending.
    assert_eq!(
        sort_value(release, Some("titleSort")).as_deref(),
        Some("originallyAvailableAt:desc")
    );
    assert_eq!(
        sort_value(release, Some("originallyAvailableAt:desc")).as_deref(),
        Some("originallyAvailableAt")
    );
    assert_eq!(
        sort_value(release, Some("originallyAvailableAt")).as_deref(),
        Some("originallyAvailableAt:desc")
    );

    // Every value the chips produce is one `plex_core` will put in a query.
    for option in &sorts {
        for current in [None, Some(option.key.as_str()), option.desc_key.as_deref()] {
            if let Some(value) = sort_value(option, current) {
                let validated =
                    library::validate_sort(&value).unwrap_or_else(|e| panic!("{value}: {e}"));
                assert_eq!(validated, value);
            }
        }
    }
}

/// The order chip's value: the `:desc` key of a field in effect ascending, the
/// plain key when it is descending, and nothing for a field the section does not
/// list. The Sort chip names the field once, in either direction.
#[test]
fn the_order_chip_flips_to_the_other_direction_of_a_listed_field() {
    let sorts = captured_sorts();
    assert_eq!(
        flip_sort(Some("titleSort"), &sorts).as_deref(),
        Some("titleSort:desc")
    );
    assert_eq!(
        flip_sort(Some("titleSort:desc"), &sorts).as_deref(),
        Some("titleSort")
    );
    assert_eq!(
        flip_sort(Some("originallyAvailableAt:desc"), &sorts).as_deref(),
        Some("originallyAvailableAt")
    );
    // Nothing listed means no field to reverse: no chip, no request.
    assert_eq!(flip_sort(None, &sorts), None);
    assert_eq!(flip_sort(Some("unknownField"), &sorts), None);
    assert_eq!(flip_sort(Some("titleSort"), &[]), None);

    assert_eq!(sort_label(Some("titleSort"), &sorts), "Sort: Title");
    assert_eq!(sort_label(Some("titleSort:desc"), &sorts), "Sort: Title");
    assert_eq!(sort_label(Some("unknownField"), &sorts), "unknownField");
    assert_eq!(sort_direction(Some("titleSort")), "Ascending");
    assert_eq!(sort_direction(Some("titleSort:desc")), "Descending");
    assert_eq!(sort_direction(None), "Ascending");

    // Flipping the field the section named, then flipping it back, lands on the
    // value the server named, and both directions validate against `plex_core`.
    for option in &sorts {
        let Some(desc) = option.desc_key.as_deref() else {
            continue;
        };
        for (from, to) in [(option.key.as_str(), desc), (desc, option.key.as_str())] {
            let flipped = flip_sort(Some(from), &sorts);
            assert_eq!(flipped.as_deref(), Some(to), "{from} -> {flipped:?}");
            let validated = library::validate_sort(to).unwrap_or_else(|e| panic!("{to}: {e}"));
            assert_eq!(validated, to);
            assert_eq!(sort_label(Some(from), &sorts), sort_label(Some(to), &sorts));
        }
    }
}

/// A field a section lists but names no reversed form for is still a field: the
/// Sort chip names it, its menu row is the checked one, and it can be picked
/// from another sort. Only its own order chip is absent.
#[test]
fn a_field_with_no_reversed_form_is_still_named_and_checkable() {
    let index = SortOption {
        key: "indexSort".to_owned(),
        title: Some("Custom Order".to_owned()),
        desc_key: None,
        default_direction: None,
        first_character_path: None,
    };
    let sorts = [index.clone()];
    // The menu's check matches it in the only direction the section gives it.
    assert_eq!(
        sort_field(Some("indexSort"), &sorts).map(|option| &option.key),
        Some(&index.key)
    );
    assert_eq!(sort_label(Some("indexSort"), &sorts), "Sort: Custom Order");
    // It offers no other direction, so flipping it asks for nothing.
    assert_eq!(sort_value(&index, Some("indexSort")), None);
    assert_eq!(flip_sort(Some("indexSort"), &sorts), None);
    // Picking it from the menu is still a sort the grid can ask for.
    assert_eq!(
        sort_value(&index, Some("originallyAvailableAt:desc")).as_deref(),
        Some("indexSort")
    );
    assert_eq!(
        library::validate_sort("indexSort").as_deref().ok(),
        Some("indexSort")
    );
}

/// One answer's size in words: a section holding one row reads as one item.
#[test]
fn one_row_reads_as_one_item() {
    assert_eq!(item_count(1), "1 item");
    assert_eq!(item_count(0), "0 items");
    assert_eq!(item_count(2), "2 items");
    assert_eq!(item_count(147), "147 items");
}

/// Every cell of a row is the same fixed 2:3 box and its artwork is cropped with
/// the same top anchor, so a row of posters shares a top edge and a caption
/// baseline whatever shape each poster's own art is.
#[test]
fn a_row_of_cells_is_one_2_to_3_box_with_one_artwork_top() {
    for available in [420.0, 900.0, 1600.0, 2600.0] {
        let [min, max] = band(PosterSize::Medium);
        let width = card_width(available, PosterSize::Medium);
        let cell = poster_size(width);
        assert!(
            (cell.y / cell.x - POSTER_RATIO).abs() < 1e-6,
            "{available} wide gives {cell}"
        );
        // One `card_width` is shared by the row, so every cell is the same box;
        // what is checkable is that the columns fill the row without overflowing
        // it or leaving a whole card's width unbuilt.
        let columns = columns(available, PosterSize::Medium);
        let row = columns as f32 * width + (columns - 1) as f32 * GAP;
        assert!(
            row <= available + 1e-6,
            "{columns} cards of {width} overflow {available}"
        );
        assert!(
            available - row < min,
            "{available} leaves a whole card's worth unbuilt"
        );
        assert!(
            (min..=max).contains(&width),
            "{width} escapes the {min}..={max} clamp"
        );
        for tex in [
            egui::vec2(1920.0, 1080.0),
            egui::vec2(1000.0, 2000.0),
            egui::vec2(600.0, 900.0),
        ] {
            let crop = cards::cover_uv_top(tex, cell);
            assert_eq!(crop.min.y, 0.0, "{tex} into {cell} is not top-pinned");
            let kept = egui::vec2(crop.width() * tex.x, crop.height() * tex.y);
            assert!(
                kept.x + 1e-3 >= cell.x && kept.y + 1e-3 >= cell.y,
                "{tex} -> {crop}"
            );
            assert!(
                (kept.x / kept.y - cell.x / cell.y).abs() < 1e-2,
                "{tex} -> {crop} does not keep the cell shape"
            );
        }
    }
}

/// The width a page really lays its columns in, and the gutters that stay
/// clear on both sides of it at each poster size and window width.
#[test]
fn a_page_keeps_both_gutters_clear_of_its_columns() {
    for window in [700.0, 900.0, 1280.0, 1600.0, 2400.0] {
        for size in [PosterSize::Small, PosterSize::Medium, PosterSize::Large] {
            let available = content_width(window, rail::WIDTH);
            let columns = columns(available, size);
            let card = card_width(available, size);
            let taken = columns as f32 * card + (columns - 1) as f32 * GAP;
            assert!(columns >= 1);
            assert!(
                taken <= available + 1e-6,
                "{columns} cards of {card} overflow their {available} width"
            );
            // What the window itself keeps clear: the rail plus the gutter on
            // the rail side, the gutter on the window's edge, and anything the
            // columns' clamp leaves over.
            let clear = window - rail::WIDTH - taken;
            assert!(
                clear >= 2.0 * GUTTER,
                "{window} at {size:?} leaves only {clear} of {} clear",
                2.0 * GUTTER
            );
            let wasted = clear - 2.0 * GUTTER;
            assert!(
                wasted < card + GAP,
                "{window} at {size:?} wastes {wasted:.1} unbuilt in a row"
            );
        }
    }
}

/// The height every row reserves, and the gaps it reserves them with: a
/// caption of two title lines and one meta line fits without lifting the row.
#[test]
fn a_row_reserves_the_lines_its_caption_draws() {
    for window in [900.0, 1280.0, 1600.0, 2400.0] {
        for size in [PosterSize::Small, PosterSize::Medium, PosterSize::Large] {
            let available = content_width(window, rail::WIDTH);
            let poster = poster_size(card_width(available, size)).y;
            // The plan's own fit: 10pt from the poster to the title, two title
            // lines, 2pt from the title to its meta line, and the meta line.
            let reserved = poster + 10.0 + 2.0 * TITLE_LINE + TITLE_GAP + 16.0;
            assert_eq!(
                row_height(available, size),
                reserved,
                "{window} at {size:?} reserves {reserved} for its caption"
            );
            let clear = window
                - rail::WIDTH
                - (columns(available, size) as f32 * card_width(available, size)
                    + (columns(available, size) - 1) as f32 * GAP);
            assert!(
                clear >= 2.0 * GUTTER,
                "{window} at {size:?} keeps only {clear} clear of both gutters"
            );
        }
    }
}
