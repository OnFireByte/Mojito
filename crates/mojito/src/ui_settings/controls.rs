//! Settings controls on rune-kit, themed in the OpenCode design language hex
//! uses: cards of rows under muted labels, one row anatomy, buttons, the
//! anchored select, notes, and banners. Colours, type, radii, elevation, and
//! motion come from the kit's tokens; the raw numbers here are layout metrics.

use super::SettingsState;
use rune::{Color, El, LoopSpec, TextStyle, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, Kit, Menu, MenuItem, Placement, Select,
    SelectAppearance, StateColors,
};

/// Row descriptions and secondary copy.
pub(super) const DESCRIPTION: TextStyle = TextStyle::new(12.0, 440.0).lh(16.0);
/// The select field's width, enough for "Follow current layout" and layout names.
const SELECT_WIDTH: f32 = 260.0;
/// A narrow label stops a row from squeezing its description to a word per line.
const COPY_MIN_WIDTH: f32 = 140.0;

/// A settings page's measurements: raised cards of rows under muted labels.
/// The title, section labels, and row text share the gutter; cards hang
/// outward by `row_px` on both sides, so row text sits on the title's edge.
pub(super) struct Metrics {
    /// The page's text edge: the title's, section labels', and rows'.
    pub(super) gutter: f32,
    pub(super) section_gap: f32,
    /// From a section label to its card.
    pub(super) label_gap: f32,
    /// A row's text inset inside its card, and how far cards hang past the gutter.
    pub(super) row_px: f32,
    pub(super) row_h: f32,
    pub(super) row_h_described: f32,
}

pub(super) const METRICS: Metrics = Metrics {
    gutter: 32.0,
    section_gap: 24.0,
    label_gap: 8.0,
    row_px: 16.0,
    row_h: 48.0,
    row_h_described: 60.0,
};

/// Everything a control needs to paint itself.
#[derive(Clone, Copy)]
pub(super) struct Paint {
    pub(super) k: &'static Kit,
    /// One physical pixel, for hairlines that survive Retina.
    pub(super) hair: f32,
}

impl Paint {
    pub(super) fn new(dark: bool, scale: f32) -> Self {
        Self {
            k: rune_theme_opencode::kit(dark),
            hair: 1.0 / scale,
        }
    }

    pub(super) fn text(self) -> Color {
        self.k.c.text_base
    }

    pub(super) fn muted(self) -> Color {
        self.k.c.text_muted
    }

    pub(super) fn danger(self) -> Color {
        self.k.c.danger.fg
    }
}

/// A settings page's column of sections: cards hang `row_px` past the gutter,
/// so their rows' text lands on it.
pub(super) fn page(key: &str) -> El {
    col()
        .key(key)
        .px(METRICS.gutter - METRICS.row_px)
        .gap(METRICS.section_gap)
}

/// A section label: above its card, on the page's text edge.
pub(super) fn section_label(p: Paint, title: &str) -> El {
    text(title.to_string())
        .key("header")
        .font(p.k.ty.ui_medium)
        .color(p.k.c.text_muted)
}

/// A titled card of rows with hairline separators between them.
pub(super) fn section(p: Paint, key: &str, title: &str, rows: Vec<El>) -> El {
    col()
        .key(key)
        .gap(METRICS.label_gap)
        .child(section_label(p, title).mx(METRICS.row_px))
        .child(card(p, format!("{key}-card"), rows))
}

/// A raised card of rows with hairline separators between them. The card
/// shares the panel's fill and lifts off it through its elevation.
pub(super) fn card(p: Paint, key: impl Into<String>, rows: Vec<El>) -> El {
    let mut surface = col().key(key).fill(p.k.c.bg_base).radius(p.k.radius.lg);
    for shadow in &p.k.elevation.raised {
        surface = surface.box_shadow(*shadow);
    }
    surface.children(separated(p, rows))
}

fn separated(p: Paint, rows: Vec<El>) -> Vec<El> {
    let mut children = Vec::with_capacity(rows.len() * 2);
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            children.push(separator(p, format!("sep{}", index - 1)));
        }
        children.push(row);
    }
    children
}

fn separator(p: Paint, key: String) -> El {
    el().key(key)
        .h(p.hair)
        .ml(METRICS.row_px)
        .mr(METRICS.row_px)
        .fill(p.k.c.border_muted)
}

/// A setting: title and optional description on the left, its control on the
/// right. The row never paints a hover fill; only its controls react.
pub(super) fn setting_row(
    p: Paint,
    key: &str,
    title: &str,
    description: Option<&str>,
    control: El,
) -> El {
    let height = if description.is_some() {
        METRICS.row_h_described
    } else {
        METRICS.row_h
    };
    row()
        .key(key)
        .min_h(height)
        .px(METRICS.row_px)
        .py(8.0)
        .gaps(8.0, 24.0)
        .center_y()
        .justify_between()
        .radius(p.k.radius.md)
        .label(title)
        .child(
            col()
                .key("copy")
                .w(COPY_MIN_WIDTH)
                .min_w(COPY_MIN_WIDTH)
                .grow(1.0)
                .gap(2.0)
                .child(
                    text(title.to_string())
                        .key("label")
                        .font(p.k.ty.ui)
                        .color(p.text()),
                )
                .children(description.map(|copy| {
                    text(copy.to_string())
                        .key("description")
                        .font(DESCRIPTION)
                        .color(p.muted())
                        .wrap()
                })),
        )
        .child(control)
}

/// A settings row that acts when pressed anywhere. The row itself never
/// paints a hover or press fill; the controls inside it show the state.
pub(super) fn pressable_row(row: El) -> El {
    row.role("toggle")
}

/// A short note inside a card, between rows.
pub(super) fn note(key: &str, copy: &str, color: Color) -> El {
    row().key(key).px(METRICS.row_px).py(8.0).child(
        text(copy.to_string())
            .key("copy")
            .font(DESCRIPTION)
            .color(color)
            .wrap()
            .grow(1.0),
    )
}

/// A tinted strip above a card's rows: a state the user must act on, such as
/// a permission the engine is missing.
pub(super) fn banner(p: Paint, key: &str, copy: &str, tone: StateColors) -> El {
    row()
        .key(key)
        .mt(8.0)
        .mx(METRICS.row_px)
        .py(10.0)
        .pl(10.0)
        .pr(12.0)
        .gap(10.0)
        .center_y()
        .radius(p.k.radius.md)
        .fill(tone.bg)
        .child(
            el().key("icon-slot")
                .size(16.0, 16.0)
                .center()
                .child(rune::icon(&rune_theme_opencode::icons::WARNING, 14.0).color(tone.icon)),
        )
        .child(
            text(copy.to_string())
                .key("copy")
                .font(DESCRIPTION)
                .color(tone.fg)
                .wrap()
                .grow(1.0),
        )
}

/// A dot for something live, such as a capture in progress.
pub(super) fn pulsing_dot(color: Color) -> El {
    status_dot(color).looping(PULSE)
}

fn status_dot(color: Color) -> El {
    el().key("dot").size(8.0, 8.0).radius(4.0).fill(color)
}

/// Breathes between full and this opacity every two seconds.
const PULSE: LoopSpec = LoopSpec::opacity(2.0, 0.0, 1.0, 0.45);

/// Main, other, and destructive actions at hex's sizes: 28 pt for a pane's
/// standing buttons, 24 pt for actions inside rows.
pub(super) fn button(
    p: Paint,
    key: &str,
    label: &str,
    variant: ButtonVariant,
    press: impl Fn(&mut SettingsState, &mut Tx) + 'static,
) -> El {
    Button::new(label.to_string())
        .variant(variant)
        .on_press_tx(press)
        .control_size(ControlSize::Large)
        .build(p.k)
        .px(11.0)
        .radius(p.k.radius.md)
        .key(key)
}

/// A smaller button for inside a row.
pub(super) fn small_button(
    p: Paint,
    key: &str,
    label: &str,
    variant: ButtonVariant,
    press: impl Fn(&mut SettingsState) + 'static,
) -> El {
    Button::new(label.to_string())
        .variant(variant)
        .on_press(press)
        .control_size(ControlSize::Regular)
        .build(p.k)
        .px(9.0)
        .radius(p.k.radius.sm)
        .key(key)
}

/// An icon-only button for actions inside a row.
pub(super) fn icon_button(
    p: Paint,
    key: &str,
    icon: &'static rune::Icon,
    label: &str,
    press: impl Fn(&mut SettingsState) + 'static,
) -> El {
    IconButton::new(icon)
        .label(label.to_string())
        .control_size(ControlSize::Small)
        .on_press(press)
        .build(p.k)
        .key(key)
}

/// Runs with the transaction, so a choice can save and close at once.
pub(super) type Choose = Box<dyn Fn(&mut SettingsState, &mut Tx)>;

/// One choice of a [`select`].
pub(super) struct Choice {
    pub(super) label: String,
    pub(super) selected: bool,
    pub(super) choose: Choose,
}

/// A select field whose choices open in an anchored menu above the page: the
/// kit's field trigger carrying a `Menu` of checked items.
#[allow(clippy::too_many_arguments)]
pub(super) fn select(
    p: Paint,
    key: &str,
    label: &str,
    open: bool,
    choices: Vec<Choice>,
    toggle: impl Fn(&mut SettingsState) + 'static,
    close: impl Fn(&mut SettingsState) + 'static,
    layer: &mut Vec<El>,
) -> El {
    let trigger = Select::new::<&str>(format!("{key}-field"), &[])
        .appearance(SelectAppearance::Field)
        .width(SELECT_WIDTH)
        .current(Some(label))
        .open(open)
        .on_toggle(toggle)
        .trigger(p.k);
    let mut menu = Menu::new(format!("{key}-menu"))
        .trigger(trigger)
        .placement(Placement::BottomEnd)
        .gutter(4.0)
        .min_width(SELECT_WIDTH)
        .open(open)
        .on_close(close);
    for choice in choices {
        menu = menu.item(
            MenuItem::new(choice.label)
                .checked(choice.selected)
                .on_select_tx(move |s: &mut SettingsState, tx: &mut Tx| (choice.choose)(s, tx)),
        );
    }
    menu.build(p.k, layer)
}
