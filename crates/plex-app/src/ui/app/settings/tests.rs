//! Settings' own tests: the geometry a setting row and its switch are drawn in.

use super::*;

fn row() -> Rect {
    Rect::from_min_size(egui::pos2(20.0, 40.0), egui::vec2(520.0, ROW_HEIGHT))
}

/// A row's text is laid out for what is left of its switch, and the switch
/// stays inside the row it belongs to.
#[test]
fn a_setting_row_keeps_its_switch_at_the_right_edge() {
    let row = row();
    let switch = switch_rect(row);
    assert_eq!(switch.width(), SWITCH.x);
    assert_eq!(switch.height(), SWITCH.y);
    assert_eq!(switch.center().y, row.center().y);
    assert!(switch.max.x <= row.max.x);
    assert!(switch.min.x > row.min.x);
    let text = row.width() - SWITCH.x - SWITCH_GAP;
    assert!(
        text > SWITCH.x,
        "the row keeps more for its text than for its switch"
    );
    assert!(
        row.min.x + text <= switch.min.x,
        "the text ends before the switch"
    );
}

/// The knob is drawn, not an egui widget, so both ends of its slide have to be
/// inside the track it is painted on.
#[test]
fn a_switch_knob_stays_inside_its_track_at_both_ends() {
    let track = switch_rect(row());
    let knob = (track.height() - 2.0 * SWITCH_PAD) * 0.5;
    let slide = track.width() * 0.5 - knob - SWITCH_PAD;
    assert!(slide > 0.0, "on and off are two positions, not one");
    for on in [false, true] {
        let center = track.center() + egui::vec2(if on { slide } else { -slide }, 0.0);
        assert!(center.x - knob >= track.min.x && center.x + knob <= track.max.x);
        assert!(center.y - knob >= track.min.y && center.y + knob <= track.max.y);
    }
}
