//! The switcher overlay: one glass card built per examples/glass-panel, a
//! pure display surface (`KeyPolicy::None` — the tap owns every keystroke).
//! `OverlayInput::Regions` plus a hit backdrop gives click-to-cancel without
//! focus.

use std::time::Duration;

use rune::tok::type_;
use rune::{
    Color, Cx, El, KeyPolicy, OverlayHandle, OverlayInput, OverlayOptions, OverlaySpec, Shadow,
    SystemMaterial, TextStyle, col, el, row, text,
};
use rune::{ConfiguredOverlay, DisplayTarget};

/// Shadow room around the card; the Core Animation shadows draw into it.
pub const MARGIN: f32 = 40.0;
const CARD_W: f32 = 380.0;
const CARD_RADIUS: f32 = 22.0;
const HEADER_H: f32 = 34.0;
const ROW_H: f32 = 44.0;
const PAD: f32 = 12.0;
const MAX_ROWS: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Pinned,
    Live,
}

/// Display-ready row: badges are pre-resolved to glyphs on the main thread,
/// so the view stays pure.
pub struct Row {
    pub badge: Option<char>,
    pub name: String,
    pub windows: Option<u8>,
}

#[derive(Clone, Copy)]
struct SwitchTheme {
    text: Color,
    text_secondary: Color,
    hover: Color,
    track: Color,
    frost: Color,
}

#[derive(Default)]
pub struct Switcher {
    mode: Option<Mode>,
    rows: Vec<Row>,
    /// This overlay's own handle, needed to fade out on the backdrop press.
    pub(crate) me: Option<OverlayHandle<Switcher>>,
}

impl Switcher {
    pub fn show(&mut self, mode: Mode, rows: Vec<Row>) {
        self.mode = Some(mode);
        self.rows = rows;
    }
}

/// The overlay window's size for `rows` rows: the card plus shadow margin.
pub fn window_size(rows: usize) -> (f32, f32) {
    (CARD_W + 2.0 * MARGIN, card_height(rows) + 2.0 * MARGIN)
}

fn card_height(rows: usize) -> f32 {
    let shown = rows.min(MAX_ROWS) as f32;
    let mut h = HEADER_H + PAD + shown * ROW_H + PAD;
    if rows > MAX_ROWS {
        h += ROW_H; // the "+N more" footer row
    }
    h.max(HEADER_H + 2.0 * PAD)
}

/// UnderPointer is the spec default; every reveal pins the exact display via
/// `set_display` before showing.
pub fn spec() -> ConfiguredOverlay<Switcher> {
    OverlaySpec::new("Mojito switcher", window_size(0), view)
        .options(OverlayOptions {
            display: DisplayTarget::UnderPointer,
            input: OverlayInput::Regions,
            keys: KeyPolicy::None,
            exclude_from_capture: true,
            prewarm: true,
            ..Default::default()
        })
        .srgb_blend(true)
}

pub fn view(s: &Switcher, cx: &mut Cx<Switcher>) -> El {
    let dark = cx.system_dark();
    // Tokens from the pinned theme crate; one accent per surface (brand).
    let th = rune_theme_opencode::theme(dark);
    let t = SwitchTheme {
        text: th.color("v2-text-text-base"),
        text_secondary: th.color("v2-text-text-muted"),
        hover: th.color("v2-overlay-simple-overlay-hover"),
        track: th.color("v2-background-bg-layer-02"),
        frost: th.color("v2-background-bg-layer-01").alpha(0.62),
    };
    let accent = rune_theme_opencode::brand(dark);
    let ink = t.text;
    let card_h = card_height(s.rows.len());

    // The card: glass, two Core Animation shadows, frost tint, hairline, per
    // examples/glass-panel. A no-op press stops rune's ancestor walk at the
    // frost, so card clicks stay inert instead of reaching the backdrop.
    let mut frost = col()
        .key("frost")
        .w_full()
        .h(card_h)
        .padding(PAD)
        .radius(CARD_RADIUS)
        .continuous_corners()
        .clip_rounded(CARD_RADIUS)
        .fill(t.frost)
        .border(1.0, ink.alpha(0.08))
        .on_press(|_: &mut Switcher| {})
        .child(header(s, t, accent));
    if s.rows.is_empty() {
        frost = frost.child(
            row().key("empty").h(ROW_H).px(10.0).center_y().child(
                text("No apps pinned yet")
                    .font(type_::BODY)
                    .color(t.text_secondary),
            ),
        );
    } else {
        let rows = col().key("rows").w_full().gap(2.0).children(
            s.rows
                .iter()
                .take(MAX_ROWS)
                .enumerate()
                .map(|(i, r)| row_view(i, r, t, accent)),
        );
        frost = frost.child(rows);
        if s.rows.len() > MAX_ROWS {
            let more = s.rows.len() - MAX_ROWS;
            frost = frost.child(
                row().key("more").h(ROW_H).px(10.0).center_y().child(
                    text(format!("+{more} more"))
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                ),
            );
        }
    }

    let card = col()
        .key("card")
        .w(CARD_W)
        .h(card_h)
        .radius(CARD_RADIUS)
        .continuous_corners()
        .box_shadow(Shadow::core_animation(
            0.0,
            0.5,
            1.5,
            Color::BLACK.alpha(0.3),
        ))
        .box_shadow(Shadow::core_animation(
            0.0,
            8.0,
            18.0,
            Color::BLACK.alpha(0.32),
        ))
        .system_material(SystemMaterial::Glass)
        .child(frost);

    // The backdrop: the whole window. A press here cancels the overlay the
    // same way the engine's Esc does visually (fade), though the engine only
    // learns of it when F18 releases — see the report on click-to-cancel.
    col()
        .key("backdrop")
        .w_full()
        .h_full()
        .padding(MARGIN)
        .hit()
        .on_press(dismiss)
        .child(card)
}

fn header(s: &Switcher, t: SwitchTheme, accent: rune::Color) -> El {
    let title = match s.mode {
        Some(Mode::Pinned) => "Pinned apps",
        Some(Mode::Live) => "Open apps",
        None => "Mojito",
    };
    row()
        .key("header")
        .h(HEADER_H)
        .px(4.0)
        .center_y()
        .justify_between()
        .child(text(title).font(type_::SECTION).color(t.text_secondary))
        .child(el().key("dot").size(8.0, 8.0).radius(4.0).fill(accent))
}

fn row_view(i: usize, r: &Row, t: SwitchTheme, accent: rune::Color) -> El {
    row()
        .key(format!("row-{i}"))
        .h(ROW_H)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(9.0)
        .continuous_corners()
        .hover(t.hover)
        // Makes the row a hover target; presses stop here and stay inert —
        // keys are the whole selection model.
        .on_hover(|_: &mut Switcher| {})
        .child(badge(r, t, accent))
        .child(monogram(r, accent))
        .child(
            el().key("name").grow(1.0).min_w(0.0).child(
                text(r.name.clone())
                    .font(type_::BODY)
                    .color(t.text)
                    .truncate(),
            ),
        )
        .children(r.windows.map(|n| {
            text(n.to_string())
                .key("windows")
                .font(type_::MONO_CAPTION)
                .color(t.text_secondary)
        }))
}

/// Key badge: the glyph of the trigger or label key.
fn badge(r: &Row, t: SwitchTheme, accent: rune::Color) -> El {
    el().key("badge")
        .size(26.0, 26.0)
        .radius(6.0)
        .continuous_corners()
        .center()
        .fill(t.track)
        .child(
            text(r.badge.map_or_else(|| "·".to_string(), |c| c.to_string()))
                .font(type_::MONO_CAPTION.mono())
                .color(accent),
        )
}

/// Monogram tile in place of an app icon (icons are a fast-follow).
fn monogram(r: &Row, accent: rune::Color) -> El {
    let initial = r
        .name
        .chars()
        .next()
        .map_or('?', |c| c.to_uppercase().next().unwrap_or(c));
    el().key("monogram")
        .size(28.0, 28.0)
        .radius(8.0)
        .continuous_corners()
        .center()
        .fill(accent.alpha(0.18))
        .child(
            text(initial.to_string())
                .font(TextStyle::new(13.0, 600.0))
                .color(accent),
        )
}

/// Click-to-cancel. The engine never learns of this — `UiMsg` flows one way —
/// so the hold stays engaged until F18 releases; the engine's
/// release-cancels-anyway invariant hides the overlay for good then.
fn dismiss(s: &mut Switcher) {
    if let Some(me) = &s.me {
        me.fade_out(Duration::from_millis(120));
    }
}
