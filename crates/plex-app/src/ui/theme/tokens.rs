//! The [`Theme`] value, the look the app ships, and the theme the UI thread
//! is drawing with. The data shape is described in the [parent module](super).

use std::cell::Cell;

use egui::Color32;

/// Every colour and material value the UI draws with. The app has one look:
/// `palette`, `glass` and `art_shadow` are the same whatever photograph is
/// behind it, and only `backdrop` follows the photograph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// Opaque colours and overlay inks.
    pub palette: Palette,
    /// The frosted-glass material.
    pub glass: Glass,
    /// The shadows under text drawn over an item's own artwork (the Home
    /// hero, a detail page's head), whose brightness nothing here can know.
    pub art_shadow: [TextShadow; 3],
    /// The strength, of 255, of the soft shade laid behind a block of text
    /// over an item's own artwork (the Home hero), so a bright part of the
    /// art cannot wash the text out.
    pub art_scrim: u8,
    /// What the photograph behind every screen needs to keep the UI readable.
    pub backdrop: Backdrop,
}

/// The app's colours, by the role each one plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Window fill: what shows where nothing else is drawn, the dim laid over
    /// the photograph, and what a page's full-bleed art fades into.
    pub window: Color32,
    /// Panel fill where no glass or backdrop shows through.
    pub surface: Color32,
    /// Card, field and inactive-widget fill.
    pub card: Color32,
    /// Hovered card fill: `card` lifted one step.
    pub card_hover: Color32,
    /// The hairline edge of a control that is not in effect.
    pub outline: Color32,
    /// Translucent fill of the pills, buttons and popovers over video.
    pub floating: Color32,
    /// Translucent plate under a small mark drawn over artwork.
    pub plate: Color32,

    /// Primary text.
    pub text: Color32,
    /// Secondary text (`weak` labels, meta lines).
    pub secondary: Color32,
    /// Small capitals over groups (library and settings captions).
    pub muted: Color32,
    /// A sidebar row's label and icon while the row is not selected.
    pub nav: Color32,
    /// A library row's label under a server in the sidebar.
    pub nav_sub: Color32,
    /// The meta line over full-bleed art.
    pub art_meta: Color32,
    /// The overview paragraph over full-bleed art.
    pub art_body: Color32,
    /// The smallest line over full-bleed art (which server an item is from).
    pub art_caption: Color32,
    /// Ink of the app's own mark.
    pub mark: Color32,

    /// Selection, progress, focus and whatever is in effect. It is neutral:
    /// a state is told by brightness, weight or fill, never by hue.
    pub accent: Color32,
    /// Ink on a solid `accent` fill.
    pub on_accent: Color32,
    /// The ring round the control that holds the keyboard focus: bright and
    /// neutral, so it reads on glass, on cards and over artwork alike.
    pub focus: Color32,

    /// Fill of the one main action on a screen.
    pub primary_bg: Color32,
    /// `primary_bg` while pointed at.
    pub primary_hover: Color32,
    /// Ink on `primary_bg`.
    pub primary_text: Color32,
    /// Quieter ink on `primary_bg` (the time left beside Resume).
    pub primary_muted: Color32,

    /// Failure text: with `danger_fill`, the only hue in the palette.
    pub danger: Color32,
    /// Solid fill of a destructive control under light ink (the close button
    /// while pointed at).
    pub danger_fill: Color32,

    /// Opaque ink of every [`wash`](Self::wash): what lifts a surface.
    pub highlight: Color32,
    /// Opaque ink of every [`scrim`](Self::scrim): what darkens art and video.
    pub shadow: Color32,
}

impl Palette {
    /// Selection fill: `accent` at 18% over `card`, so `text` on it stays
    /// high contrast. Derived, so it cannot drift from the accent it tints.
    #[must_use]
    pub fn accent_tint(&self) -> Color32 {
        mix(self.card, self.accent, 0.18)
    }

    /// `highlight` at `alpha` of 255: hover fills, tracks, rims, hairlines.
    /// The strength is the component's; the ink is the theme's.
    #[must_use]
    pub fn wash(&self, alpha: u8) -> Color32 {
        self.highlight.gamma_multiply_u8(alpha)
    }

    /// `shadow` at `alpha` of 255: plates and gradients over art and video.
    #[must_use]
    pub fn scrim(&self, alpha: u8) -> Color32 {
        self.shadow.gamma_multiply_u8(alpha)
    }

    /// Hairlines between rows and along panel edges.
    #[must_use]
    pub fn rule(&self) -> Color32 {
        self.wash(20)
    }

    /// Secondary button fill over whatever is behind it.
    #[must_use]
    pub fn secondary_bg(&self) -> Color32 {
        self.wash(31)
    }

    /// The unfilled part of the progress track on `primary_bg`.
    #[must_use]
    pub fn primary_track(&self) -> Color32 {
        self.primary_text.gamma_multiply_u8(46)
    }
}

/// The frosted-glass material ([`crate::ui::glass`]). A tint is premultiplied
/// and its alpha is its opacity: how much of the blurred backdrop it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    /// The sidebar.
    pub rail: Color32,
    /// Panels and chips over a page's art, and the rooms panel.
    pub panel: Color32,
    /// The sign-in screens' card.
    pub splash_card: Color32,
    /// A section of a settings-like screen (Settings, Sources) and the
    /// notices and prompts drawn over a page.
    pub section: Color32,
    /// A shelf's round pager arrow, over artwork.
    pub shelf_arrow: Color32,
    /// The error strip the sign-in screens draw over their backdrop, under
    /// `danger` text.
    pub status_strip: Color32,
    /// The hairline around every panel.
    pub rim: Color32,
    /// The brighter line along a panel's top edge.
    pub sheen: Color32,
    /// Colour saturation applied to the blurred backdrop (1 is unchanged).
    /// At or under 1 the glass smokes what is behind it instead of tinting it.
    pub saturation: f32,
    /// Opacity added to a tint where the blur cannot run, so the plain fill
    /// drawn instead still carries its text.
    pub solid_boost: u8,
}

impl Glass {
    /// The plain fill drawn in place of a panel tinted `tint` where the blur
    /// cannot run.
    #[must_use]
    pub fn solid(&self, tint: Color32) -> Color32 {
        let [r, g, b, a] = tint.to_array();
        Color32::from_rgba_premultiplied(r, g, b, a.saturating_add(self.solid_boost))
    }
}

/// What one photograph behind the app needs from the UI drawn over it. This
/// is the only part of a [`Theme`] that follows the background: a brighter
/// photograph asks for more dim and a stronger shadow, and
/// [`set_backdrop`] is how the code that changes the photograph says so.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    /// Share of the palette's `window` (0..=1) over the sign-in copy.
    pub sign_in_dim: f32,
    /// Share of the palette's `window` (0..=1) over the browsing copy.
    pub browse_dim: f32,
    /// The shadows under text drawn straight on the photograph (page titles,
    /// captions, notes).
    pub text_shadow: [TextShadow; 3],
}

/// One dark copy of a line of text under the line itself, as CSS's
/// `text-shadow` describes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    /// How far below the text the copy sits, in points.
    pub drop: f32,
    /// How far the copy spreads around the glyphs, in points; 0 is a hard copy.
    pub blur: f32,
    /// Strength of the copy, of 255, in the palette's `shadow` ink; 0 draws
    /// nothing.
    pub alpha: u8,
}

impl TextShadow {
    /// No shadow.
    pub const NONE: Self = Self {
        drop: 0.0,
        blur: 0.0,
        alpha: 0,
    };
}

/// The opaque colour `share` (0..=1) of the way from `from` to `to`.
#[must_use]
pub fn mix(from: Color32, to: Color32, share: f32) -> Color32 {
    let channel = |a: u8, b: u8| (f32::from(a) + share * (f32::from(b) - f32::from(a))).round();
    Color32::from_rgb(
        channel(from.r(), to.r()) as u8,
        channel(from.g(), to.g()) as u8,
        channel(from.b(), to.b()) as u8,
    )
}

/// One pixel of a photograph as a colour, so it can be measured against the
/// palette: read from the picture, not chosen for the interface.
#[must_use]
pub fn pixel([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// WCAG 2.2 relative luminance of a colour: what its brightness is worth to a
/// reader, the half of the contrast ratio a surface is measured against.
#[must_use]
pub fn luminance(c: Color32) -> f32 {
    fn channel(v: u8) -> f32 {
        let s = f32::from(v) / 255.0;
        if s <= 0.040_45 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    let [r, g, b, _] = c.to_array();
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// The WCAG 2.2 contrast ratio between two colours, in either order: the
/// `4.5:1` body-text and `3:1` secondary-text bars are what a page is held to.
#[must_use]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = (la.max(lb), la.min(lb));
    (hi + 0.05) / (lo + 0.05)
}

/// The vertex colour that leaves a texture's own pixels as they are. Not a
/// theme colour: artwork is never tinted.
pub const UNTINTED: Color32 = Color32::WHITE;
/// What surrounds video and stands in for it while the next item resolves.
pub const LETTERBOX: Color32 = Color32::BLACK;

/// Smoky glass: neutral greys and blacks over whatever photograph is behind
/// the app. Nothing here has a hue but the two danger colours.
pub const SMOKE: Theme = Theme {
    palette: Palette {
        window: Color32::from_gray(0x08),
        surface: Color32::from_gray(0x0e),
        card: Color32::from_gray(0x1a),
        card_hover: Color32::from_gray(0x27),
        outline: Color32::from_gray(0x42),
        floating: Color32::from_rgba_premultiplied(14, 14, 14, 215),
        plate: Color32::from_rgba_premultiplied(8, 8, 8, 140),
        text: Color32::from_gray(0xf2),
        secondary: Color32::from_gray(0xad),
        muted: Color32::from_gray(0x93),
        nav: Color32::from_gray(0xcb),
        nav_sub: Color32::from_gray(0xb2),
        art_meta: Color32::from_gray(0xea),
        art_body: Color32::from_gray(0xe0),
        art_caption: Color32::from_gray(0xc9),
        mark: Color32::from_gray(0xf4),
        accent: Color32::from_gray(0xf4),
        on_accent: Color32::from_gray(0x0c),
        focus: Color32::from_gray(0xf4),
        primary_bg: Color32::from_gray(0xf4),
        primary_hover: Color32::WHITE,
        primary_text: Color32::from_gray(0x0c),
        primary_muted: Color32::from_gray(0x48),
        danger: Color32::from_rgb(0xff, 0x8a, 0x80),
        danger_fill: Color32::from_rgb(0xc4, 0x2b, 0x1c),
        highlight: Color32::WHITE,
        shadow: Color32::BLACK,
    },
    glass: Glass {
        rail: Color32::from_rgba_premultiplied(9, 9, 9, 184),
        panel: Color32::from_rgba_premultiplied(10, 10, 10, 190),
        splash_card: Color32::from_rgba_premultiplied(10, 10, 10, 178),
        section: Color32::from_rgba_premultiplied(12, 12, 12, 180),
        shelf_arrow: Color32::from_rgba_premultiplied(9, 9, 9, 190),
        status_strip: Color32::from_rgba_premultiplied(12, 12, 12, 212),
        rim: Color32::from_rgba_premultiplied(30, 30, 30, 30),
        sheen: Color32::from_rgba_premultiplied(46, 46, 46, 46),
        saturation: 0.85,
        solid_boost: 70,
    },
    art_shadow: [
        TextShadow {
            drop: 1.0,
            blur: 0.0,
            alpha: 150,
        },
        TextShadow {
            drop: 1.0,
            blur: 1.5,
            alpha: 150,
        },
        TextShadow {
            drop: 1.0,
            blur: 3.0,
            alpha: 90,
        },
    ],
    art_scrim: 120,
    backdrop: Backdrop {
        sign_in_dim: 0.0,
        browse_dim: 0.25,
        text_shadow: [
            TextShadow {
                drop: 1.0,
                blur: 0.0,
                alpha: 110,
            },
            TextShadow::NONE,
            TextShadow::NONE,
        ],
    },
};

thread_local! {
    /// The theme the UI thread draws with. Per thread because egui runs on
    /// one, and so a test that installs a theme cannot restyle another test.
    static CURRENT: Cell<Theme> = const { Cell::new(SMOKE) };
}

/// The theme in effect: a copy, cheap enough to take wherever a colour is
/// needed.
#[must_use]
pub fn current() -> Theme {
    CURRENT.get()
}

/// The current theme's palette.
#[must_use]
pub fn palette() -> Palette {
    current().palette
}

/// Make `theme` the one [`current`] answers with. [`super::install`] also
/// restyles egui from it.
pub(super) fn set_current(theme: Theme) {
    CURRENT.set(theme);
}

/// Say what the photograph now behind the app needs: its dim and the shadow
/// under text drawn straight on it. This is the one thing a change of
/// background changes about the look; no egui style depends on it, so the
/// next frame simply draws with it.
pub fn set_backdrop(backdrop: Backdrop) {
    CURRENT.set(Theme {
        backdrop,
        ..CURRENT.get()
    });
}
