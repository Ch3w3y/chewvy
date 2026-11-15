//! The rule that keeps egui's stock widgets off the product screens, and the
//! controls' own behaviour.

use std::path::{Path, PathBuf};

/// What names a stock egui widget in code. Each has an app control in
/// [`super`]: a button kind, [`super::Field`], [`super::slider`],
/// [`super::option_row`] or [`super::segmented`], [`super::rule`],
/// [`super::spinner`], [`super::dialog`].
const STOCK: [&str; 24] = [
    "ui.button(",
    ".small_button(",
    "egui::Button",
    "Slider::new",
    "egui::Slider",
    "DragValue",
    "selectable_label(",
    "selectable_value(",
    ".separator(",
    "TextEdit::singleline",
    "TextEdit::multiline",
    ".text_edit_singleline(",
    ".text_edit_multiline(",
    "toggle_value(",
    ".checkbox(",
    "egui::Checkbox",
    ".radio(",
    ".radio_value(",
    "ComboBox",
    ".spinner(",
    "egui::Spinner",
    ".hyperlink(",
    ".link(",
    "egui::Modal",
];

/// The lines of `source` that name a stock widget, 1-based. Comments and an
/// inline `mod tests` block are skipped.
fn stock_named_in(source: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut in_tests = false;
    for (index, line) in source.lines().enumerate() {
        if line.starts_with("mod tests {") {
            in_tests = true;
        } else if in_tests && line == "}" {
            in_tests = false;
        }
        let code = line.split("//").next().unwrap_or_default();
        if !in_tests && STOCK.iter().any(|stock| code.contains(stock)) {
            found.push(index + 1);
        }
    }
    found
}

/// Every `.rs` file under `dir` that is not a test file.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            sources(&path, out);
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_stock_widget_check_catches_each_one() {
    for bad in [
        "if ui.button(\"Check\").clicked() {",
        "ui.add(egui::Slider::new(&mut zoom, 50.0..=200.0))",
        "if ui.selectable_label(on, label).clicked() {",
        "    ui.separator();",
        "ui.add(TextEdit::singleline(&mut flow.address))",
        "egui::TextEdit::singleline(field.query)",
        "ui.toggle_value(&mut flow.show_password, \"show\");",
        "            ui.spinner();",
        "ui.add(egui::Spinner::new().size(16.0))",
        "egui::Modal::new(Id::new(\"hdr\")).show(ctx, |ui| {",
        "let press = ui.add(egui::Button::new(label).frame(false));",
    ] {
        assert_eq!(stock_named_in(bad), vec![1], "{bad}");
    }
    for fine in [
        "if controls::button(ui, Kind::Secondary, \"Check\").clicked() {",
        "controls::spinner(ui);",
        "controls::rule(ui);",
        "if ui.style().interaction.selectable_labels {",
        "/// was `ui.separator()` before",
        "let pick = pick.or(link(ui, rect, id, page, on));",
    ] {
        assert!(stock_named_in(fine).is_empty(), "{fine}");
    }
    let with_tests =
        "fn f() {}\n#[cfg(test)]\nmod tests {\n    ui.button(\"x\");\n}\nui.button(\"y\");\n";
    assert_eq!(stock_named_in(with_tests), vec![6]);
}

/// A product screen builds its controls from [`super`], so every button,
/// field, slider, choice and dialog looks like the app's and shows the
/// keyboard focus. Only the controls module itself wraps egui's widgets.
#[test]
fn no_stock_egui_widget_is_used_under_ui_outside_the_controls() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
    let mut files = Vec::new();
    sources(&ui, &mut files);
    assert!(files.len() > 30, "found only {} files", files.len());
    let mut named = Vec::new();
    for path in files {
        let inside = path.strip_prefix(&ui).unwrap_or(&path);
        if inside.starts_with("controls") || inside == Path::new("controls.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(!source.is_empty(), "{path:?} could not be read");
        for line in stock_named_in(&source) {
            named.push(format!("{}:{line}", inside.display()));
        }
    }
    assert!(
        named.is_empty(),
        "stock egui widgets under ui (use ui::controls instead):\n{}",
        named.join("\n")
    );
}

/// Run `frames` frames of `add` in a fresh themed context, feeding each frame
/// the events `input` gives for its index, and answer what the last frame's
/// `add` returned.
fn frames<R: Default>(
    frames: usize,
    input: impl Fn(usize) -> Vec<egui::Event>,
    mut add: impl FnMut(&mut egui::Ui) -> R,
) -> R {
    let ctx = egui::Context::default();
    crate::ui::theme::apply(&ctx);
    let mut out = R::default();
    for frame in 0..frames {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events: input(frame),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| out = add(ui));
        output.textures_delta.clear();
    }
    out
}

/// A press and release of the primary button at `at`.
fn click(at: egui::Pos2) -> Vec<egui::Event> {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![egui::Event::PointerMoved(at), button(true), button(false)]
}

/// A press and release of `key`.
fn key(key: egui::Key) -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        })
        .to_vec()
}

/// Where a field sits and whether it holds the keyboard.
#[derive(Default)]
struct Seen {
    rect: Option<egui::Rect>,
    focused: bool,
}

/// Lay out one field, with a label over it the way the add flow does.
fn field(ui: &mut egui::Ui) -> Seen {
    egui::ScrollArea::vertical()
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                let mut text = String::new();
                ui.label("Username");
                let answer = super::Field::new(&mut text, "someone", "name").show(ui);
                Seen {
                    rect: Some(answer.response.rect),
                    focused: answer.response.has_focus(),
                }
            })
            .inner
        })
        .inner
}

#[test]
fn clicking_a_field_puts_the_caret_in_it() {
    let rect = frames(1, |_| Vec::new(), field)
        .rect
        .unwrap_or(egui::Rect::NOTHING);
    let at = rect.left_center() + egui::vec2(20.0, 0.0);
    let seen = frames(
        4,
        |frame| if frame == 1 { click(at) } else { Vec::new() },
        field,
    );
    assert!(seen.focused);
}

#[test]
fn a_field_is_one_stop_for_tab() {
    let seen = frames(
        3,
        |frame| {
            if frame == 1 {
                key(egui::Key::Tab)
            } else {
                Vec::new()
            }
        },
        field,
    );
    assert!(
        seen.focused,
        "the first Tab stopped somewhere other than the text"
    );
}

#[test]
fn arrow_keys_move_a_focused_slider_by_its_step() {
    let mut value = 1.0;
    frames(
        5,
        |frame| match frame {
            1 => key(egui::Key::Tab),
            3 => key(egui::Key::ArrowRight),
            _ => Vec::new(),
        },
        |ui| {
            super::slider(ui, "Zoom", &mut value, 0.5..=2.0, 0.05, |v| format!("{v}"));
        },
    );
    assert!((value - 1.05).abs() < 1e-9, "{value}");
}

/// Two segmented rows on one panel (the player's sleep timer and aspect
/// ratio) are separate controls: a click on one is not a click on the other.
#[test]
fn a_click_on_one_segmented_row_is_not_a_click_on_another() {
    let lower = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut picked = Vec::new();
    frames(
        4,
        |frame| {
            if frame == 2 {
                click(lower.get().left_center() + egui::vec2(10.0, 0.0))
            } else {
                Vec::new()
            }
        },
        |ui| {
            let first = super::segmented(ui, &["Off", "15m"], Some(0));
            let top = ui.min_rect().bottom();
            let second = super::segmented(ui, &["Auto", "16:9"], Some(0));
            lower.set(egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), top),
                ui.min_rect().max,
            ));
            if first.is_some() || second.is_some() {
                picked.push((first, second));
            }
        },
    );
    assert_eq!(picked, vec![(None, Some(0))]);
}
