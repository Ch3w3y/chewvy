//! The carousel's own rules as pure functions: which item comes next, when a
//! frame advances, and where its dots and arrows land.

use super::*;

#[test]
fn the_index_wraps_at_both_ends() {
    assert_eq!(next_index(0, 5), 1);
    assert_eq!(next_index(3, 5), 4);
    assert_eq!(next_index(4, 5), 0);
}

#[test]
fn a_carousel_of_one_has_nowhere_to_go() {
    assert_eq!(next_index(0, 1), 0);
    assert_eq!(next_index(0, 0), 0);
}

#[test]
fn a_pool_that_shrank_still_holds_the_item_on_screen() {
    assert_eq!(clamp_index(4, 5), 4);
    assert_eq!(clamp_index(4, 2), 1);
    assert_eq!(clamp_index(0, 0), 0);
}

#[test]
fn one_item_up_stays_up_its_whole_interval_then_resets() {
    let (elapsed, advanced) = advance(0.0, 3.0, 5, false, AUTO_ADVANCE_SECS);
    assert_eq!((elapsed, advanced), (3.0, false));
    let (elapsed, advanced) = advance(elapsed, 3.0, 5, false, AUTO_ADVANCE_SECS);
    assert_eq!((elapsed, advanced), (6.0, false));
    let (elapsed, advanced) = advance(elapsed, 3.0, 5, false, AUTO_ADVANCE_SECS);
    assert_eq!((elapsed, advanced), (0.0, true));
}

#[test]
fn hovering_the_hero_freezes_the_countdown() {
    let (elapsed, advanced) = advance(5.0, 8.0, 5, true, AUTO_ADVANCE_SECS);
    assert_eq!((elapsed, advanced), (5.0, false));
    // Released, it continues from where it stopped rather than from zero.
    let (elapsed, advanced) = advance(elapsed, 3.0, 5, false, AUTO_ADVANCE_SECS);
    assert_eq!((elapsed, advanced), (0.0, true));
}

#[test]
fn a_carousel_of_one_or_no_interval_counts_nothing() {
    assert_eq!(advance(4.0, 8.0, 1, false, AUTO_ADVANCE_SECS), (4.0, false));
    assert_eq!(advance(4.0, 8.0, 5, false, 0.0), (4.0, false));
}

#[test]
fn a_negative_frame_counts_as_no_time() {
    assert_eq!(
        advance(2.0, -9.0, 5, false, AUTO_ADVANCE_SECS),
        (2.0, false)
    );
}

#[test]
fn the_next_repaint_is_promised_for_the_rest_of_the_interval() {
    assert_eq!(
        repaint_after(3.0, 5, false, AUTO_ADVANCE_SECS),
        Some(5.0),
        "the rest of one interval"
    );
    assert!(
        repaint_after(7.9, 5, false, AUTO_ADVANCE_SECS)
            .is_some_and(|rest| (rest - 0.1).abs() < 1e-3),
        "a nearly-expired item repaints almost at once"
    );
    assert_eq!(repaint_after(3.0, 5, true, AUTO_ADVANCE_SECS), None);
    assert_eq!(repaint_after(3.0, 1, false, AUTO_ADVANCE_SECS), None);
}

#[test]
fn dots_line_up_left_to_right_inside_the_hero() {
    let hero = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(900.0, height(900.0)));
    let rects = dot_rects(hero, 3);
    assert_eq!(rects.len(), 3);
    for rect in &rects {
        assert!(hero.contains(rect.center()));
        assert_eq!(rect.width(), DOT_BOX);
    }
    assert!(rects
        .iter()
        .zip(rects.iter().skip(1))
        .all(|(earlier, later)| earlier.center().x < later.center().x));
    assert_eq!(
        rects.last().map(|rect| rect.center().x),
        Some(hero.max.x - DOT_INSET)
    );
}

/// `Color32` alpha compositing, so the claim below is about the colour the hero
/// actually paints rather than about the alpha it asks for.
fn composite(over: Color32, under: Color32) -> Color32 {
    let a = f32::from(over.a()) / 255.0;
    let mix = |over: u8, under: u8| (f32::from(over) * a + f32::from(under) * (1.0 - a)).round();
    Color32::from_rgb(
        mix(over.r(), under.r()) as u8,
        mix(over.g(), under.g()) as u8,
        mix(over.b(), under.b()) as u8,
    )
}

/// WCAG 2.2 relative luminance.
fn luminance(color: Color32) -> f32 {
    let channel = |value: u8| {
        let s = f32::from(value) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let (hi, lo) = (
        luminance(a).max(luminance(b)),
        luminance(a).min(luminance(b)),
    );
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn hero_text_holds_over_white_art_at_the_scrim_s_darkest_stop() {
    let alpha = (bleed::Mask::Hero.scrim().first().map_or(0.0, |s| s.1) * 255.0).round() as u8;
    let ground = composite(
        Color32::from_rgba_unmultiplied(
            theme::WINDOW.r(),
            theme::WINDOW.g(),
            theme::WINDOW.b(),
            alpha,
        ),
        Color32::WHITE,
    );
    assert!(
        contrast(theme::TEXT, ground) >= 4.5,
        "{}",
        contrast(theme::TEXT, ground)
    );
    assert!(
        contrast(theme::SECONDARY, ground) >= 3.0,
        "{}",
        contrast(theme::SECONDARY, ground)
    );
}

#[test]
fn the_hero_takes_its_share_of_the_window_within_bounds() {
    assert_eq!(height(900.0), 540.0);
    assert_eq!(height(500.0), 440.0);
    assert_eq!(height(2160.0), 560.0);
}

#[test]
fn the_mask_keeps_the_top_right_and_fades_the_left_and_bottom() {
    assert_eq!(bleed::Mask::Hero.alpha(0.82, 0.22), 1.0);
    assert_eq!(bleed::Mask::Hero.alpha(0.0, 1.0), 0.0);
    assert!(bleed::Mask::Hero.alpha(0.3, 0.5) < bleed::Mask::Hero.alpha(0.7, 0.3));
}

#[test]
fn time_left_reads_hours_and_minutes() {
    assert_eq!(home::time_left(22), "22 min left");
    assert_eq!(home::time_left(70), "1h 10m left");
}

#[test]
fn wide_windows_crop_the_art_from_the_bottom_so_heads_stay() {
    // 16:9 art across a maximised 2000 x 780 hero keeps the full top edge.
    let uv = bleed::art_uv(egui::vec2(3840.0, 2160.0), egui::vec2(2000.0, 780.0));
    assert_eq!(uv.min.y, 0.0);
    assert!(uv.max.y < 1.0);
    assert!((uv.width() - 1.0).abs() < 1e-6);
}
