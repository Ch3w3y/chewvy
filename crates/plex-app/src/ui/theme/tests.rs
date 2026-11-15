//! The theme's rules: what the look must keep readable, that it has no hue
//! but danger, that installing a theme restyles egui, and that no colour is
//! named under `ui` outside this module.

use std::path::{Path, PathBuf};

use egui::{text::LayoutJob, Color32, FontSelection};

use super::*;

/// The backdrop every glass tint is tested over: bright artwork, where a
/// light surface would lose its text.
const WORST_BACKDROP: Color32 = Color32::WHITE;

/// A theme no screen can be read in: cream surfaces under the same light
/// text, pale glass, no dim. It exists to prove [`failures`] catches a bad
/// palette and that [`install`] reaches every stock widget.
fn bright() -> Theme {
    let cream = Color32::from_rgb(0xf4, 0xee, 0xdc);
    Theme {
        palette: Palette {
            window: cream,
            surface: Color32::from_rgb(0xec, 0xe4, 0xce),
            card: Color32::from_rgb(0xe2, 0xd8, 0xbe),
            card_hover: Color32::from_rgb(0xd6, 0xca, 0xac),
            accent: Color32::from_rgb(0xff, 0x7a, 0x1a),
            ..SMOKE.palette
        },
        glass: Glass {
            rail: Color32::from_rgba_premultiplied(120, 110, 90, 128),
            ..SMOKE.glass
        },
        art_shadow: SMOKE.art_shadow,
        art_scrim: SMOKE.art_scrim,
        backdrop: Backdrop {
            browse_dim: 0.0,
            ..SMOKE.backdrop
        },
    }
}

/// Puts [`SMOKE`] back when a test that installed another theme ends, so a
/// single-threaded test run does not carry the theme into the next test.
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        tokens::set_current(SMOKE);
    }
}

/// `tint` over `back`: what a glass surface leaves of a flat backdrop.
fn over(tint: Color32, back: Color32) -> Color32 {
    let [fr, fg, fb, fa] = tint.to_array();
    let keep = 1.0 - f32::from(fa) / 255.0;
    let channel = |front: u8, back: u8| (f32::from(front) + f32::from(back) * keep).round() as u8;
    Color32::from_rgb(
        channel(fr, back.r()),
        channel(fg, back.g()),
        channel(fb, back.b()),
    )
}

/// Every readability rule the look is held to, as the ones `theme` breaks.
/// Each is `ink on ground` at a WCAG ratio: 7 for primary text, 4.5 for body
/// and coloured text, 3 for secondary text on glass over white.
fn failures(theme: &Theme) -> Vec<String> {
    let p = &theme.palette;
    let g = &theme.glass;
    let on_glass = |tint: Color32| over(tint, WORST_BACKDROP);
    let rules: Vec<(&str, Color32, Color32, f32)> = vec![
        ("text on window", p.text, p.window, 7.0),
        ("text on surface", p.text, p.surface, 7.0),
        ("text on card", p.text, p.card, 7.0),
        ("text on card_hover", p.text, p.card_hover, 7.0),
        ("text on accent_tint", p.text, p.accent_tint(), 7.0),
        ("secondary on window", p.secondary, p.window, 4.5),
        ("secondary on surface", p.secondary, p.surface, 4.5),
        ("secondary on card", p.secondary, p.card, 4.5),
        ("muted on window", p.muted, p.window, 4.5),
        ("muted on surface", p.muted, p.surface, 4.5),
        ("accent on surface", p.accent, p.surface, 4.5),
        ("accent on card", p.accent, p.card, 4.5),
        ("accent on accent_tint", p.accent, p.accent_tint(), 4.5),
        ("on_accent on accent", p.on_accent, p.accent, 4.5),
        ("focus on surface", p.focus, p.surface, 3.0),
        ("focus on card_hover", p.focus, p.card_hover, 3.0),
        ("focus on panel glass", p.focus, on_glass(g.panel), 3.0),
        (
            "primary_text on primary_bg",
            p.primary_text,
            p.primary_bg,
            7.0,
        ),
        (
            "primary_text on primary_hover",
            p.primary_text,
            p.primary_hover,
            7.0,
        ),
        (
            "primary_muted on primary_bg",
            p.primary_muted,
            p.primary_bg,
            4.5,
        ),
        ("danger on surface", p.danger, p.surface, 4.5),
        ("danger on card", p.danger, p.card, 4.5),
        ("text on danger_fill", p.text, p.danger_fill, 4.5),
        ("text on floating", p.text, on_glass(p.floating), 4.5),
        ("text on rail glass", p.text, on_glass(g.rail), 4.5),
        ("nav on rail glass", p.nav, on_glass(g.rail), 4.5),
        ("nav_sub on rail glass", p.nav_sub, on_glass(g.rail), 3.0),
        (
            "secondary on rail glass",
            p.secondary,
            on_glass(g.rail),
            3.0,
        ),
        ("text on panel glass", p.text, on_glass(g.panel), 4.5),
        (
            "secondary on panel glass",
            p.secondary,
            on_glass(g.panel),
            3.0,
        ),
        (
            "text on splash_card glass",
            p.text,
            on_glass(g.splash_card),
            4.5,
        ),
        (
            "secondary on splash_card glass",
            p.secondary,
            on_glass(g.splash_card),
            3.0,
        ),
        ("text on section glass", p.text, on_glass(g.section), 4.5),
        (
            "secondary on section glass",
            p.secondary,
            on_glass(g.section),
            3.0,
        ),
        (
            "text on shelf_arrow glass",
            p.text,
            on_glass(g.shelf_arrow),
            4.5,
        ),
        (
            "danger on status_strip glass",
            p.danger,
            on_glass(g.status_strip),
            4.5,
        ),
    ];
    rules
        .into_iter()
        .filter_map(|(rule, ink, ground, bar)| {
            let ratio = contrast(ink, ground);
            (ratio < bar).then(|| format!("{rule} is {ratio:.2}:1, under {bar}:1"))
        })
        .collect()
}

#[test]
fn the_shipped_look_keeps_its_text_readable() {
    let broken = failures(&SMOKE);
    assert!(broken.is_empty(), "{}", broken.join("\n"));
}

#[test]
fn a_bright_theme_is_caught_by_the_same_rules() {
    let broken = failures(&bright());
    let caught = |rule: &str| broken.iter().any(|line| line.contains(rule));
    for rule in [
        "text on window",
        "text on card",
        "secondary on surface",
        "muted on window",
        "accent on card",
        "text on rail glass",
    ] {
        assert!(
            caught(rule),
            "{rule} not reported in:\n{}",
            broken.join("\n")
        );
    }
    // Rules the bright theme did not touch still pass, so a report names
    // what is wrong and nothing else.
    assert!(!caught("primary_text on primary_bg"));
    assert!(!caught("text on panel glass"));
}

#[test]
fn glass_over_white_is_the_tint_plus_what_it_lets_through() {
    assert_eq!(over(Color32::TRANSPARENT, WORST_BACKDROP), WORST_BACKDROP);
    assert_eq!(over(Color32::BLACK, WORST_BACKDROP), Color32::BLACK);
    let [r, g, b, _] = over(SMOKE.glass.shelf_arrow, WORST_BACKDROP).to_array();
    assert!(r < 90 && g < 90 && b < 90, "{r},{g},{b} over white");
}

#[test]
fn contrast_is_the_wcag_ratio_in_both_orders() {
    assert!(contrast(Color32::WHITE, Color32::BLACK) > 20.9);
    assert_eq!(
        contrast(Color32::WHITE, Color32::BLACK),
        contrast(Color32::BLACK, Color32::WHITE)
    );
}

/// How far apart a colour's channels may be and still count as having no hue.
const NEUTRAL: u8 = 2;

fn has_no_hue(colour: Color32) -> bool {
    let [r, g, b, _] = colour.to_array();
    r.max(g).max(b) - r.min(g).min(b) <= NEUTRAL
}

/// The app's only colouring is smoky glass. Everything the look draws with is
/// a grey, a black or a white, bar the two danger colours, and the glass does
/// not boost what shows through it.
#[test]
fn the_look_has_no_hue_but_danger() {
    let p = SMOKE.palette;
    let g = SMOKE.glass;
    let neutral = [
        ("window", p.window),
        ("surface", p.surface),
        ("card", p.card),
        ("card_hover", p.card_hover),
        ("outline", p.outline),
        ("floating", p.floating),
        ("plate", p.plate),
        ("text", p.text),
        ("secondary", p.secondary),
        ("muted", p.muted),
        ("nav", p.nav),
        ("nav_sub", p.nav_sub),
        ("art_meta", p.art_meta),
        ("art_body", p.art_body),
        ("art_caption", p.art_caption),
        ("mark", p.mark),
        ("accent", p.accent),
        ("accent_tint", p.accent_tint()),
        ("on_accent", p.on_accent),
        ("focus", p.focus),
        ("primary_bg", p.primary_bg),
        ("primary_hover", p.primary_hover),
        ("primary_text", p.primary_text),
        ("primary_muted", p.primary_muted),
        ("primary_track", p.primary_track()),
        ("highlight", p.highlight),
        ("shadow", p.shadow),
        ("glass rail", g.rail),
        ("glass panel", g.panel),
        ("glass splash_card", g.splash_card),
        ("glass section", g.section),
        ("glass shelf_arrow", g.shelf_arrow),
        ("glass status_strip", g.status_strip),
        ("glass rim", g.rim),
        ("glass sheen", g.sheen),
    ];
    for (name, colour) in neutral {
        assert!(has_no_hue(colour), "{name} is {colour:?}");
    }
    assert!(g.saturation <= 1.0, "glass saturation {}", g.saturation);
    assert!(!has_no_hue(p.danger) && !has_no_hue(p.danger_fill));
    // The bright test theme's orange accent is what this check exists to
    // refuse.
    assert!(!has_no_hue(bright().palette.accent));
}

/// Stock egui widgets are styled from the same palette, so they carry no hue
/// either: egui's own defaults (a blue text cursor, a blue selection) do not
/// come through.
#[test]
fn egui_is_styled_without_a_hue_but_the_error_colour() {
    let v = style(&SMOKE).visuals;
    let w = &v.widgets;
    let states = [w.noninteractive, w.inactive, w.hovered, w.active, w.open];
    let mut colours = vec![
        ("window_fill", v.window_fill),
        ("panel_fill", v.panel_fill),
        ("extreme_bg_color", v.extreme_bg_color),
        ("faint_bg_color", v.faint_bg_color),
        ("code_bg_color", v.code_bg_color),
        ("window_stroke", v.window_stroke.color),
        ("hyperlink_color", v.hyperlink_color),
        ("warn_fg_color", v.warn_fg_color),
        ("selection fill", v.selection.bg_fill),
        ("selection stroke", v.selection.stroke.color),
        ("text cursor", v.text_cursor.stroke.color),
        (
            "ime underline",
            v.ime_composition.active_underline_stroke.color,
        ),
        (
            "ime underline, inactive",
            v.ime_composition.inactive_underline_stroke.color,
        ),
        ("window shadow", v.window_shadow.color),
        ("popup shadow", v.popup_shadow.color),
    ];
    for state in states {
        colours.push(("widget fill", state.bg_fill));
        colours.push(("widget weak fill", state.weak_bg_fill));
        colours.push(("widget edge", state.bg_stroke.color));
        colours.push(("widget ink", state.fg_stroke.color));
    }
    colours.extend(v.override_text_color.map(|c| ("text", c)));
    colours.extend(v.weak_text_color.map(|c| ("weak text", c)));
    colours.extend(v.text_edit_bg_color.map(|c| ("text edit fill", c)));
    for (name, colour) in colours {
        assert!(has_no_hue(colour), "{name} is {colour:?}");
    }
    assert_eq!(v.error_fg_color, SMOKE.palette.danger);
}

/// The overlay inks are plain white and black at the strength asked for, and
/// what is derived follows what it is derived from.
#[test]
fn derived_colours_follow_their_sources() {
    let p = SMOKE.palette;
    for alpha in 0..=u8::MAX {
        assert_eq!(p.wash(alpha), Color32::from_white_alpha(alpha));
        assert_eq!(p.scrim(alpha), Color32::from_black_alpha(alpha));
    }
    assert_eq!(p.rule(), p.wash(20));
    assert_eq!(p.secondary_bg(), p.wash(31));
    assert_eq!(p.accent_tint(), mix(p.card, p.accent, 0.18));
    let lifted = Palette {
        accent: p.card_hover,
        ..p
    };
    assert_ne!(lifted.accent_tint(), p.accent_tint());
    // An accent is a fill under dark ink or ink on a dark fill, never a fill
    // under light text.
    assert!(contrast(p.text, p.accent) < 1.5);
    assert!(contrast(p.on_accent, p.accent) > 7.0);
}

#[test]
fn the_style_and_the_text_helpers_take_their_colours_from_the_theme() {
    for theme in [SMOKE, bright()] {
        let p = theme.palette;
        let s = style(&theme);
        assert_eq!(s.visuals.window_fill, p.window);
        assert_eq!(s.visuals.panel_fill, p.surface);
        assert_eq!(s.visuals.selection.bg_fill, p.accent_tint());
        assert_eq!(s.visuals.widgets.inactive.bg_fill, p.card);
        assert_eq!(s.visuals.widgets.hovered.bg_fill, p.card_hover);
        assert_eq!(s.visuals.widgets.active.bg_fill, p.accent_tint());
        assert_eq!(s.visuals.widgets.active.fg_stroke.color, p.text);
        assert_eq!(s.visuals.widgets.open.bg_fill, p.accent_tint());
        assert_eq!(s.visuals.hyperlink_color, p.accent);
        assert_eq!(s.visuals.error_fg_color, p.danger);
    }
}

#[test]
fn installing_a_theme_restyles_stock_widgets_and_what_the_app_paints() {
    let _restore = Restore;
    let ctx = egui::Context::default();
    apply(&ctx);
    assert_eq!(current(), SMOKE);
    for egui_theme in [egui::Theme::Dark, egui::Theme::Light] {
        let s = ctx.style_of(egui_theme);
        assert_eq!(s.visuals.window_fill, SMOKE.palette.window);
        assert_eq!(s.text_styles.get(&TextStyle::Body), Some(&body()));
    }

    let bright = bright();
    install(&ctx, bright);
    assert_eq!(current(), bright);
    assert_eq!(palette(), bright.palette);
    for egui_theme in [egui::Theme::Dark, egui::Theme::Light] {
        let s = ctx.style_of(egui_theme);
        assert_eq!(s.visuals.window_fill, bright.palette.window);
        assert_eq!(s.visuals.panel_fill, bright.palette.surface);
        assert_eq!(s.visuals.widgets.inactive.bg_fill, bright.palette.card);
        assert_eq!(s.visuals.hyperlink_color, bright.palette.accent);
        assert_eq!(s.visuals.selection.bg_fill, bright.palette.accent_tint());
        assert_eq!(s.text_styles.get(&TextStyle::Body), Some(&body()));
    }
    // What the app paints itself reads the theme when it paints.
    let drawn = |text: RichText| {
        let mut job = LayoutJob::default();
        text.append_to(
            &mut job,
            &ctx.style_of(egui::Theme::Dark),
            FontSelection::Default,
            egui::Align::Center,
        );
        job.sections.last().map(|section| section.format.color)
    };
    assert_eq!(drawn(title("Heat")), Some(bright.palette.text));
    assert_eq!(
        drawn(meta_text("movie · 1h41m")),
        Some(bright.palette.secondary)
    );
    assert_eq!(drawn(strong("Heat")), Some(bright.palette.text));
    assert_eq!(
        crate::ui::chips::chip_colors(true, false),
        (
            bright.palette.accent_tint(),
            bright.palette.accent,
            bright.palette.accent
        )
    );

    install(&ctx, SMOKE);
    assert_eq!(
        ctx.style_of(egui::Theme::Dark).visuals,
        style(&SMOKE).visuals
    );
}

#[test]
fn a_change_of_background_changes_the_backdrop_and_nothing_else() {
    let _restore = Restore;
    let brighter = Backdrop {
        browse_dim: 0.5,
        ..SMOKE.backdrop
    };
    set_backdrop(brighter);
    assert_eq!(
        current(),
        Theme {
            backdrop: brighter,
            ..SMOKE
        }
    );
}

#[test]
fn type_scale_sizes() {
    assert_eq!(display(), FontId::new(32.0, family(BOLD)));
    assert_eq!(h1(), FontId::new(24.0, family(BOLD)));
    assert_eq!(h2(), FontId::new(19.0, family(SEMIBOLD)));
    assert_eq!(body(), FontId::new(15.0, egui::FontFamily::Proportional));
    assert_eq!(meta(), FontId::new(13.0, egui::FontFamily::Proportional));
    assert_eq!(caption(), FontId::new(12.0, egui::FontFamily::Proportional));
}

#[test]
fn every_named_style_is_in_the_scale() {
    let styles = style(&SMOKE).text_styles;
    for name in [&*DISPLAY, &*H1, &*H2, &*META, &*CAPTION] {
        assert!(styles.contains_key(name), "{name:?} missing");
    }
    assert_eq!(styles.get(&TextStyle::Body), Some(&body()));
    assert_eq!(styles.get(&TextStyle::Heading), Some(&h1()));
}

#[test]
fn radii_and_spacing_do_not_come_from_the_theme() {
    for theme in [SMOKE, bright()] {
        let s = style(&theme);
        assert_eq!(s.visuals.window_corner_radius, CornerRadius::same(8));
        assert_eq!(
            s.visuals.widgets.noninteractive.corner_radius,
            CornerRadius::same(8)
        );
        assert_eq!(
            s.visuals.widgets.inactive.corner_radius,
            CornerRadius::same(8)
        );
        assert_eq!(s.spacing.item_spacing, egui::Vec2::new(16.0, 16.0));
        assert_eq!(s.spacing.window_margin, Margin::same(24));
        assert_eq!(s.spacing.indent, 16.0);
    }
}

/// The ways a colour can be named in code. Everything on `Color32` but the
/// two values that are not colours, and the other colour types egui exports.
const COLOUR_ITEMS: [&str; 4] = ["Color32::", "Rgba::", "Hsva", "hex_color!"];
/// What may follow `Color32::` anywhere: no colour at all, and the marker a
/// layout uses for "whatever the painter is given".
const NOT_COLOURS: [&str; 2] = ["TRANSPARENT", "PLACEHOLDER"];

/// The lines of `source` that name a colour, 1-based. Comments and an inline
/// `mod tests` block are skipped: a test may build any colour to test with.
fn colours_named_in(source: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut in_tests = false;
    for (index, line) in source.lines().enumerate() {
        if line.starts_with("mod tests {") {
            in_tests = true;
        } else if in_tests && line == "}" {
            in_tests = false;
        }
        if in_tests {
            continue;
        }
        let code = line.split("//").next().unwrap_or_default();
        let names_one = COLOUR_ITEMS.iter().any(|item| {
            code.match_indices(item).any(|(at, _)| {
                let rest = code.get(at + item.len()..).unwrap_or_default();
                *item != "Color32::" || !NOT_COLOURS.iter().any(|ok| rest.starts_with(ok))
            })
        });
        if names_one {
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
fn the_colour_check_catches_each_way_of_naming_one() {
    for bad in [
        "let c = Color32::from_rgb(1, 2, 3);",
        "    egui::Color32::WHITE,",
        "painter.rect_filled(r, 0.0, Color32::from_black_alpha(90));",
        "let c = egui::Rgba::from_rgb(0.1, 0.2, 0.3);",
        "let c = Hsva::new(0.5, 1.0, 1.0, 1.0);",
        "let c = egui::hex_color!(\"#ff0000\");",
        "(Color32::TRANSPARENT, Color32::BLACK)",
    ] {
        assert_eq!(colours_named_in(bad), vec![1], "{bad}");
    }
    for fine in [
        "(Color32::TRANSPARENT, theme::palette().outline)",
        ".layout_no_wrap(text, font, Color32::PLACEHOLDER)",
        "fn chip_colors(active: bool) -> (Color32, Color32, Color32) {",
        "/// was `Color32::WHITE` before themes",
        "let ink = theme::palette().wash(28);",
    ] {
        assert!(colours_named_in(fine).is_empty(), "{fine}");
    }
    let with_tests = "fn f() {}\n#[cfg(test)]\nmod tests {\n    let c = Color32::RED;\n}\nconst C: Color32 = Color32::RED;\n";
    assert_eq!(colours_named_in(with_tests), vec![6]);
}

/// A colour named outside this module cannot follow a theme switch. Everything
/// under `ui` takes its colours from [`current`], or from the fixed constants
/// this module documents.
#[test]
fn no_colour_is_named_under_ui_outside_the_theme() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
    let mut files = Vec::new();
    sources(&ui, &mut files);
    assert!(
        files.len() > 30,
        "found only {} files under {ui:?}",
        files.len()
    );
    let mut named = Vec::new();
    for path in files {
        let inside = path.strip_prefix(&ui).unwrap_or(&path);
        if inside.starts_with("theme") || inside == Path::new("theme.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(!source.is_empty(), "{path:?} could not be read");
        for line in colours_named_in(&source) {
            named.push(format!("{}:{line}", inside.display()));
        }
    }
    assert!(
        named.is_empty(),
        "colours named outside ui/theme (take them from theme::current()):\n{}",
        named.join("\n")
    );
}
