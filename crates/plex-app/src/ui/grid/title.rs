//! How a poster card's title breaks over its two caption lines.

use egui::{Color32, FontId};

use crate::ui::rail;

/// Words that lead into the word after them. A line never ends on one when
/// the title can break elsewhere, so "The Lord of the Rings: The" does not
/// leave "The" hanging at the end of a line.
const LEADING_WORDS: [&str; 16] = [
    "the", "a", "an", "of", "and", "&", "in", "on", "at", "to", "for", "with", "from", "by", "or",
    "vs.",
];

/// A mark that joins the parts of a title (`Show · S1E3`, `Title: Subtitle`
/// is already joined to its word); a line never starts with one when the
/// title can break elsewhere.
pub(super) fn separates(word: &str) -> bool {
    matches!(word, "·" | "-" | "–" | "—" | "|" | "/")
}

pub(super) fn leads(word: &str) -> bool {
    LEADING_WORDS
        .iter()
        .any(|leading| word.eq_ignore_ascii_case(leading))
}

/// `text` laid out as at most [`super::CAPTION_TITLE_ROWS`] lines of `width` each,
/// and whether it had to be cut short.
///
/// A title that needs two lines is broken where the two come out closest in
/// length, never after a leading word ([`LEADING_WORDS`]) when another break
/// fits. A title too long for two lines keeps whole words on the first and
/// ends the second with `…`. Measured, so what a card shows and what
/// [`super::row_height`] reserved for it are the same lines.
pub(super) fn title_lines(
    painter: &egui::Painter,
    text: &str,
    width: f32,
    font: &FontId,
) -> (Vec<String>, bool) {
    if width <= 0.0 {
        return (Vec::new(), !text.is_empty());
    }
    let measure = |line: &str| {
        painter
            .layout_no_wrap(line.to_owned(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
    };
    let text = text.trim();
    if measure(text) <= width {
        return (vec![text.to_owned()], false);
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let join = |range: &[&str]| range.join(" ");
    let mut best: Option<(f32, usize)> = None;
    for at in 1..words.len() {
        let (first, second) = words.split_at(at);
        let (a, b) = (measure(&join(first)), measure(&join(second)));
        if a > width || b > width {
            continue;
        }
        let dangling = first.last().is_some_and(|word| leads(word));
        let orphaned = second.first().is_some_and(|word| separates(word));
        let cost = a.max(b) + if dangling || orphaned { width } else { 0.0 };
        if best.is_none_or(|(least, _)| cost < least) {
            best = Some((cost, at));
        }
    }
    if let Some((_, at)) = best {
        let (first, second) = words.split_at(at);
        return (vec![join(first), join(second)], false);
    }
    // Too long for two lines: whole words on the first, the rest cut short.
    let mut at = (1..=words.len())
        .take_while(|at| words.get(..*at).is_some_and(|w| measure(&join(w)) <= width))
        .last()
        .unwrap_or(0);
    if at > 1 && words.get(at - 1).is_some_and(|word| leads(word)) {
        at -= 1;
    }
    let (first, rest) = words.split_at(at);
    if first.is_empty() {
        return (vec![rail::elide(painter, text, width, font)], true);
    }
    (
        vec![join(first), rail::elide(painter, &join(rest), width, font)],
        true,
    )
}
