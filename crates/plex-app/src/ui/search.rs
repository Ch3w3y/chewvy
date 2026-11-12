//! Search results as the server groups them: one section per search hub that
//! answered with rows.
//!
//! [`group`] turns the [`Hub`] rows of `GET /hubs/search` into the sections the
//! Search screen draws, [`caption`] is the one line an episode row is read as,
//! and [`initials`] labels a people circle, whose artwork the server names only
//! as an absolute URL the `/photo/:/transcode` request builder rejects.

use plex_core::library::{Hub, Item};

use crate::ui::grid::{self, Arrow};

/// The hub `type` values the captured search answer names; only `artist` holds
/// people rather than titled works, so it is the kind drawn as circles.
const PEOPLE_KINDS: [&str; 1] = ["artist"];

/// One search hub's matches, under the label the hub itself named.
pub struct Group {
    /// The hub's `title` (`Movies`, `Shows`, `Artists`).
    pub title: String,
    /// The hub's `Metadata` rows as poster cards, in the server's order.
    pub cards: Vec<grid::Card>,
    /// Draw [`Self::cards`] as initials circles instead of posters: the rows of
    /// a people hub are credited names, not works with a poster.
    pub people: bool,
}

/// The searchable groups of a search answer: a hub the server found nothing for
/// (its answer names no `Metadata` array, so [`Hub::items`] is empty) draws no
/// section.
#[must_use]
pub fn group(hubs: &[Hub]) -> Vec<Group> {
    hubs.iter()
        .filter(|hub| !hub.items.is_empty())
        .map(|hub| {
            let kind = hub.kind.clone().unwrap_or_default();
            Group {
                title: hub
                    .title
                    .clone()
                    .unwrap_or_else(|| hub.hub_id.clone().unwrap_or_default()),
                cards: hub
                    .items
                    .iter()
                    .map(|item| {
                        let mut card = grid::Card::from_item(item);
                        if card.kind == "episode" {
                            card.title = caption(item);
                            card.subtitle.clear();
                        }
                        card
                    })
                    .collect(),
                people: PEOPLE_KINDS.contains(&kind.as_str()),
            }
        })
        .collect()
}

/// The caption for a search result: an episode reads
/// `grandparentTitle · S{parentIndex}E{index} · title` — the show it belongs to,
/// which the season row's own `parentTitle` ("Season 1") does not name. Anything
/// else reads as its own title. Parts an answer omits are left out rather than
/// invented.
#[must_use]
pub fn caption(item: &Item) -> String {
    if item.kind.as_deref() != Some("episode") {
        return item
            .title
            .clone()
            .unwrap_or_else(|| "(untitled)".to_owned());
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(show) = &item.grandparent_title {
        parts.push(show.clone());
    }
    if let (Some(season), Some(number)) = (item.parent_index, item.index) {
        parts.push(format!("S{season}E{number}"));
    }
    if let Some(title) = &item.title {
        parts.push(title.clone());
    }
    parts.join(" · ")
}

/// The label for a people circle: the first letter of the first word and of the
/// last, uppercased. A one-word name gives its own first letter.
#[must_use]
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let letter = |word: &str| {
        word.chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
    };
    let first = words.first().and_then(|w| letter(w));
    let last = words
        .get(1..)
        .and_then(|rest| rest.last())
        .and_then(|w| letter(w));
    match (first, last) {
        (Some(a), Some(b)) => format!("{a}{b}"),
        (Some(a), None) => a,
        (None, _) => String::new(),
    }
}

/// Where the keyboard selection stands in a grouped search answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Which section, a [`Group`] index.
    pub group: usize,
    /// Which card of that section.
    pub index: usize,
}

/// The columns each section lays its cards out in at `width` points and
/// `size`: one row per section, the same rule
/// [`crate::ui::grid::columns`] gives the library grid.
#[must_use]
pub fn columns(groups: &[Group], width: f32, size: crate::ui::settings::PosterSize) -> Vec<usize> {
    groups.iter().map(|_| grid::columns(width, size)).collect()
}

/// The next selection, where `counts[group]` is how many cards a section holds
/// and `columns[group]` how many sit in one row.
///
/// Left and Right step within a section and continue at the far end of the next
/// one, wrapping at the ends of the answer. Up and Down step a row within a
/// section and otherwise jump to the section above or below at the same card
/// index, clamped to what that section holds; they do not wrap. With no
/// selection yet the first card of the first section is selected, and an arrow
/// with nowhere to go leaves the selection where it was.
#[must_use]
pub fn move_selection(
    counts: &[usize],
    columns: &[usize],
    current: Option<Selection>,
    arrow: Arrow,
) -> Option<Selection> {
    let current = current.filter(|selected| {
        counts
            .get(selected.group)
            .is_some_and(|count| selected.index < *count)
    })?;
    let count = counts.get(current.group).copied().unwrap_or_default();
    let cols = columns.get(current.group).copied().unwrap_or(1).max(1);
    if let Some(index) = grid::move_focus(cols, count, Some(current.index), arrow) {
        return Some(Selection {
            group: current.group,
            index,
        });
    }
    let forward = matches!(arrow, Arrow::Right | Arrow::Down);
    let Some(group) = next_group(counts, current.group, forward) else {
        return Some(current);
    };
    let last = counts.get(group).copied().unwrap_or(0).saturating_sub(1);
    let index = match arrow {
        Arrow::Right => 0,
        Arrow::Left => last,
        _ => current.index.min(last),
    };
    Some(Selection { group, index })
}

/// The next section of the answer that has cards in it, wrapping at its ends.
fn next_group(counts: &[usize], from: usize, forward: bool) -> Option<usize> {
    let last = counts.len();
    (1..last).find_map(|step| {
        let group = if forward {
            (from + step) % last
        } else {
            (from + last - step) % last
        };
        counts.get(group).filter(|count| **count > 0).map(|_| group)
    })
}

#[cfg(test)]
#[path = "search/tests.rs"]
mod tests;
