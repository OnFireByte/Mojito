//! The window frame, hex's settings layout: a deep-background sidebar beside
//! a raised rounded content panel, with the save footer on the panel. Menus
//! and popovers mount above the page through rune-kit's root.

use rune::{Anim, Cx, El, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::ButtonVariant;

use super::controls::{METRICS, Paint, button, page};
use super::panes;
use super::{Pane, SettingsState, save, toggle_catalog};

const SCROLL_FADE: (f32, f32) = (4.0, 24.0);

pub fn view(s: &SettingsState, cx: &mut Cx<SettingsState>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let mut layer = Vec::new();
    let body = match s.pane {
        Pane::Apps => panes::apps(s, p),
        Pane::Behavior => panes::behavior(s, p, &mut layer),
        Pane::System => panes::system(s, p),
    };
    let panel = col()
        .key("panel")
        .grow(1.0)
        .shrink()
        .mt(8.0)
        .mr(8.0)
        .mb(8.0)
        .radius(p.k.radius.lg)
        .fill(p.k.c.bg_base)
        .border(p.hair, p.k.c.border_weak)
        // Children sit inside the border, so their fills never paint over it.
        .padding(p.hair)
        .clip_soft(0.0)
        .child(pane_header(s, p))
        .child(scroll_body("body", page("page").child(body)))
        .child(footer(s, p));
    let window = row()
        .key("window")
        .role("window")
        .w_full()
        .h_full()
        .fill(p.k.c.bg_deep)
        // Hover reacts with no fade anywhere in the window, popovers included.
        .hover_motion_within(Anim::INSTANT, Anim::INSTANT)
        .child(sidebar(s, p))
        .child(panel);
    rune_kit::root(window, layer)
}

/// A vertically scrolling body whose edges fade out, so the first and last
/// card edges are never cut while the content moves under them.
fn scroll_body(key: &str, content: El) -> El {
    let (top, bottom) = SCROLL_FADE;
    col()
        .key(key)
        .grow(1.0)
        .min_h(0.0)
        .clip_fade(0.0, top, 0.0, bottom)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(col().key("inset").pt(top).pb(bottom).child(content))
}

fn pane_header(s: &SettingsState, p: Paint) -> El {
    let action = (s.pane == Pane::Apps).then(|| {
        let label = if s.catalog_open {
            "Cancel"
        } else {
            "Add app…"
        };
        button(p, "add-app", label, ButtonVariant::Neutral, |s, _| {
            toggle_catalog(s)
        })
    });
    row()
        .key("pane-header")
        .h(52.0)
        .px(METRICS.gutter)
        .gap(8.0)
        .center_y()
        .child(
            text(s.pane.title())
                .key("pane-title")
                .font(p.k.ty.body_medium)
                .color(p.text())
                .shrink()
                .truncate(),
        )
        .child(el().key("header-spacer").grow(1.0))
        .children(action)
}

/// Sidebar rows follow OpenCode Desktop's settings navigation: 28 pt rows
/// with a 4 pt gap, so each row's highlight stays its own.
fn sidebar(s: &SettingsState, p: Paint) -> El {
    let nav = col()
        .key("nav")
        .role("list")
        .focus_group()
        .focus_follows()
        .gap(4.0)
        .children(Pane::ALL.map(|pane| nav_item(s, p, pane)));
    col()
        .key("sidebar")
        .role("navigation")
        .w(220.0)
        .h_full()
        // macOS draws the traffic lights over a borderless window's top-left;
        // keep the first nav row below them.
        .pt(30.0)
        .pb(16.0)
        .pl(10.0)
        .pr(6.0)
        .child(nav)
        .child(el().key("spacer").grow(1.0))
}

/// One sidebar section: hover fills like a native sidebar row, and the
/// selected fill and the pane beside it swap in the same frame.
fn nav_item(s: &SettingsState, p: Paint, pane: Pane) -> El {
    let c = &p.k.c;
    let selected = s.pane == pane;
    let mut item = row()
        .key(pane.nav_key())
        .h(28.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(p.k.radius.md)
        .label(pane.title())
        .hover(c.hover)
        .child(icon(pane.icon(), 16.0).key("icon").color(if selected {
            c.icon_base
        } else {
            c.icon_muted
        }))
        .child(
            text(pane.title())
                .key("label")
                .font(p.k.ty.ui)
                .color(if selected { c.text_base } else { c.text_muted }),
        );
    if selected {
        item = item.fill(c.layer_02);
    }
    item.focusable(true)
        .focus_selected(selected)
        .on_drag(
            move |s: &mut SettingsState, d: &mut rune::Drag, tx: &mut Tx| {
                if d.phase == rune::Phase::Down {
                    select_pane(s, tx, pane);
                }
            },
        )
        .on_press_tx(move |s: &mut SettingsState, tx: &mut Tx| select_pane(s, tx, pane))
}

/// Selects as fast as it can: the selected fill and the pane swap in that
/// frame, as a native sidebar's do.
fn select_pane(s: &mut SettingsState, tx: &mut Tx, pane: Pane) {
    if s.pane != pane {
        tx.instant();
        s.pane = pane;
    }
}

fn footer(s: &SettingsState, p: Paint) -> El {
    let status = s.status.as_ref().map(|st| {
        text(st.text.clone())
            .key("status")
            .font(p.k.ty.small)
            .color(if st.error { p.danger() } else { p.muted() })
            .grow(1.0)
            .truncate()
    });
    col()
        .key("footer")
        .w_full()
        .child(
            el().key("footer-rule")
                .h(p.hair)
                .w_full()
                .fill(p.k.c.border_weak),
        )
        .child(
            row()
                .key("footer-row")
                .h(52.0)
                .px(METRICS.gutter)
                .gap(12.0)
                .center_y()
                .children(status)
                .child(el().key("footer-spacer").grow(1.0))
                .child(button(p, "save", "Save", ButtonVariant::Submit, save)),
        )
}
