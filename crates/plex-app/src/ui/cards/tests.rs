use egui::pos2 as p;

use super::*;

#[test]
fn an_unflipped_arrow_points_right_and_a_flipped_one_left() {
    let center = p(100.0, 50.0);
    let [top, tip, bottom] = arrow_points(center, false);
    assert!(tip.x > center.x, "tip right of centre");
    assert!(top.x < tip.x && bottom.x < tip.x, "arms trail to the left");
    assert_eq!(top.y - center.y, -ARROW[1]);
    assert_eq!(bottom.y - center.y, ARROW[1]);
    let [top, tip, bottom] = arrow_points(center, true);
    assert!(tip.x < center.x, "tip left of centre");
    assert!(top.x > tip.x && bottom.x > tip.x, "arms trail to the right");
}

#[test]
fn an_arrows_clickable_box_is_two_radii_wide() {
    let rect = arrow_rect(p(40.0, 40.0));
    assert_eq!(rect.width(), 2.0 * ARROW_RADIUS);
    assert_eq!(rect.height(), 2.0 * ARROW_RADIUS);
    assert_eq!(rect.center(), p(40.0, 40.0));
}

/// A plain vertical wheel belongs to the page, never to a row: only a delta
/// that came sideways (trackpad flick, horizontal wheel) or one held with Shift
/// pages a row.
#[test]
fn only_a_sideways_or_shifted_wheel_pages_a_row() {
    assert_eq!(row_wheel_delta(false, egui::vec2(0.0, -30.0)), 0.0);
    assert_eq!(row_wheel_delta(false, egui::vec2(0.0, 30.0)), 0.0);
    assert_eq!(row_wheel_delta(false, egui::vec2(18.0, 0.0)), 18.0);
    assert_eq!(row_wheel_delta(true, egui::vec2(0.0, -30.0)), -30.0);
    assert_eq!(row_wheel_delta(true, egui::vec2(0.0, 30.0)), 30.0);
    assert_eq!(row_wheel_delta(true, egui::vec2(-18.0, 0.0)), -18.0);
    assert_eq!(row_wheel_delta(false, egui::Vec2::ZERO), 0.0);
}

/// A row takes the wheel only while it can still page, so a wheel at the end of
/// a row reaches the page behind it instead of being swallowed.
#[test]
fn a_row_at_its_end_leaves_the_wheel_to_the_page() {
    // A negative delta is content moving left, i.e. paging on to later cards.
    assert_eq!(row_advance(0.0, 500.0, -30.0), Some(30.0));
    assert_eq!(row_advance(500.0, 500.0, -30.0), None);
    assert_eq!(row_advance(500.0, 500.0, 30.0), Some(-30.0));
    assert_eq!(row_advance(0.0, 500.0, 30.0), None);
    // Nothing to move: a row that fits its band, or a wheel that moved nothing.
    assert_eq!(row_advance(0.0, 0.0, -30.0), None);
    assert_eq!(row_advance(120.0, 500.0, 0.0), None);
}

/// A cover crop must cover the target and keep the texture's shape: the uv it
/// names is the same aspect as the box it is drawn into, never a letterbox or a
/// stretch, for art wider or taller than that box.
#[test]
fn a_cover_crop_covers_the_box_without_distorting_the_art() {
    let box_ = egui::vec2(300.0, 450.0);
    for tex in [
        egui::vec2(1920.0, 1080.0),
        egui::vec2(1000.0, 2000.0),
        egui::vec2(600.0, 900.0),
    ] {
        for uv in [cover_uv(tex, box_), cover_uv_top(tex, box_)] {
            let w = uv.width() * tex.x;
            let h = uv.height() * tex.y;
            assert!(w + 1e-3 >= box_.x && h + 1e-3 >= box_.y, "{tex} -> {uv}");
            assert!(
                (w / h - box_.x / box_.y).abs() < 1e-2,
                "{tex} -> {uv} does not keep the box shape"
            );
            assert!(uv.min.x >= 0.0 && uv.min.y >= 0.0);
            assert!(uv.max.x <= 1.0 && uv.max.y <= 1.0);
        }
    }
    // Art with no size at all keeps the whole texture rather than a broken quad.
    assert_eq!(cover_uv_top(egui::vec2(0.0, 450.0), box_), FULL_UV);
}

/// Posters in one row share a top edge and therefore a caption baseline: the
/// crop is pinned to the art's top edge whenever the cover has vertical room
/// left over, while a backdrop keeps the centred rule.
#[test]
fn a_poster_crop_is_pinned_to_the_art_top_and_a_backdrop_is_centred() {
    let cell = egui::vec2(300.0, 450.0);
    // Art taller than the cell: only the vertical has room left over.
    let tall = egui::vec2(1000.0, 2000.0);
    let poster = cover_uv_top(tall, cell);
    let backdrop = cover_uv(tall, cell);
    assert_eq!(poster.min.y, 0.0, "a poster keeps the top row of its art");
    assert!(backdrop.min.y > poster.min.y, "{backdrop} is not centred");
    assert_eq!(poster.size(), backdrop.size(), "only the anchor differs");
    // Art wider than the cell: the cover leaves room only sideways, so both
    // rules keep the whole height and centre the horizontal crop.
    let wide = egui::vec2(1920.0, 1080.0);
    let wide_top = cover_uv_top(wide, cell);
    assert_eq!(wide_top.min.y, 0.0);
    assert!((wide_top.max.y - 1.0).abs() < 1e-3);
    assert_eq!(wide_top.min.x, cover_uv(wide, cell).min.x);
}
