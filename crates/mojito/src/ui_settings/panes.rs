//! The three sections' content: pinned apps with trigger capture, behavior
//! (the switcher trigger, the tap threshold and the key-hint layout picker),
//! and system (permissions and the Caps Lock remap).

use rune::{El, PhysicalInput, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{Badge, BadgeVariant, ButtonVariant, TextInput};

use super::controls::{
    Choice, METRICS, Paint, banner, button, icon_button, note, pulsing_dot, section, select,
    setting_row, small_button,
};
use super::{
    CatalogState, PinnedDraft, SettingsState, add_app, add_top_match, capture_input, catalog_key,
    catalog_matches, choose_layout, choose_trigger, is_pinned, remove_row, toggle_remap,
};
use crate::catalog::CatalogApp;
use mojito_core::SwitcherTrigger;
use mojito_macos::keyboard::layout;
use mojito_macos::shell::permission;

/// How tall the catalog list may grow before it scrolls inside its card.
const CATALOG_HEIGHT: f32 = 320.0;

/// The sections column; `view` supplies the page's gutter.
fn sections(key: &str) -> El {
    col().key(key).gap(METRICS.section_gap)
}

pub(super) fn apps(s: &SettingsState, p: Paint) -> El {
    sections("apps")
        .children(s.catalog_open.then(|| catalog_card(s, p)))
        .child(pinned_card(s, p))
}

pub(super) fn behavior(s: &SettingsState, p: Paint, layer: &mut Vec<El>) -> El {
    sections("behavior")
        .child(section(
            p,
            "switcher",
            "Switcher",
            vec![trigger_row(s, p, layer)],
        ))
        .child(section(p, "tapping", "Tapping", vec![tap_row(s, p)]))
        .child(section(
            p,
            "hints",
            "Key hints",
            vec![hint_row(s, p, layer)],
        ))
}

pub(super) fn system(s: &SettingsState, p: Paint) -> El {
    sections("system")
        .child(permission_card(s, p))
        .child(remap_card(s, p))
}

fn pinned_card(s: &SettingsState, p: Paint) -> El {
    let mut rows: Vec<El> = s
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| pinned_row(s, i, r, p))
        .collect();
    if rows.is_empty() {
        rows.push(
            row().key("empty").px(METRICS.row_px).py(16.0).child(
                text("No apps pinned yet. Add one, then press its trigger key.")
                    .key("copy")
                    .font(p.k.ty.ui)
                    .color(p.muted()),
            ),
        );
    }
    rows.push(note(
        "physical-note",
        "Trigger keys are physical positions: they hold under any layout or input language.",
        p.muted(),
    ));
    section(p, "pinned", "Pinned apps", rows)
}

/// One pinned-app row. Pressing anywhere captures the next physical key for
/// its trigger; the row is focusable so `on_physical_input` reaches it.
fn pinned_row(s: &SettingsState, i: usize, r: &PinnedDraft, p: Paint) -> El {
    let capturing = s.capturing == Some(i);
    row()
        .key(format!("pin-{i}"))
        .role("toggle")
        .min_h(METRICS.row_h)
        .px(METRICS.row_px)
        .py(8.0)
        .gaps(10.0, 16.0)
        .center_y()
        .focusable(true)
        .on_press(move |s: &mut SettingsState| s.capturing = Some(i))
        .on_physical_input(
            move |s: &mut SettingsState, ev: &PhysicalInput, _: &mut Tx| capture_input(s, i, ev),
        )
        .child(monogram(p, &r.name))
        .child(label_cell(p, &r.name, &r.bundle_id))
        .child(trigger_badge(p, capturing, r.key.and_then(layout::glyph)))
        .child(icon_button(
            p,
            "remove",
            &rune_theme_opencode::icons::CLOSE,
            "Remove",
            move |s: &mut SettingsState| remove_row(s, i),
        ))
}

/// A monogram tile in place of an app icon (icons are a fast-follow).
fn monogram(p: Paint, name: &str) -> El {
    let initial = name
        .chars()
        .next()
        .map_or('?', |c| c.to_uppercase().next().unwrap_or(c));
    el().key("monogram")
        .size(28.0, 28.0)
        .radius(p.k.radius.md)
        .center()
        .fill(p.k.c.layer_02)
        .child(
            text(initial.to_string())
                .font(p.k.ty.ui_medium)
                .color(p.text()),
        )
}

fn label_cell(p: Paint, title: &str, sub: &str) -> El {
    col()
        .key("label")
        .grow(1.0)
        .min_w(0.0)
        .gap(1.0)
        .child(
            text(title.to_string())
                .font(p.k.ty.ui)
                .color(p.text())
                .truncate(),
        )
        .child(
            text(sub.to_string())
                .key("sub")
                .font(p.k.ty.small)
                .color(p.muted())
                .truncate(),
        )
}

/// The row's trigger key: the glyph the chosen hint layout spells, or the
/// capture prompt while the row listens.
fn trigger_badge(p: Paint, capturing: bool, glyph: Option<char>) -> El {
    let content = if capturing {
        row()
            .key("listening")
            .gap(6.0)
            .center_y()
            .child(pulsing_dot(p.danger()))
            .child(text("Press a key…").font(p.k.ty.small).color(p.muted()))
    } else {
        text(glyph.map_or_else(|| "—".to_string(), |c| c.to_string()))
            .key("glyph")
            .font(p.k.ty.mono)
            .color(p.text())
    };
    row()
        .key("badge")
        .h(24.0)
        .min_w(32.0)
        .px(10.0)
        .center_y()
        .radius(p.k.radius.sm)
        .fill(p.k.c.layer_02)
        .child(content)
}

fn catalog_card(s: &SettingsState, p: Paint) -> El {
    let body = match &s.catalog {
        CatalogState::Idle | CatalogState::Loading => {
            note("scanning", "Scanning installed apps…", p.muted())
        }
        CatalogState::Ready(list) if list.is_empty() => note("none", "No apps found", p.muted()),
        CatalogState::Ready(_) => catalog_list(s, p),
    };
    section(p, "catalog", "Add an app", vec![search_field(s, p), body])
}

fn search_field(s: &SettingsState, p: Paint) -> El {
    row().key("search").px(METRICS.row_px).py(10.0).child(
        TextInput::new(&s.query)
            .placeholder("Search by name or bundle ID")
            .leading_icon(p.k.glyph.search)
            .large()
            .width(None)
            .on_edit(|s: &mut SettingsState| &mut s.query)
            .on_submit(|s: &mut SettingsState, _: &mut Tx| add_top_match(s))
            .on_key(catalog_key)
            .build(p.k)
            .key("query"),
    )
}

fn catalog_list(s: &SettingsState, p: Paint) -> El {
    let matches = catalog_matches(s);
    if matches.is_empty() {
        return note(
            "no-match",
            &format!("No apps match “{}”", s.query.text.trim()),
            p.muted(),
        );
    }
    let highlight_top = !s.query.text.trim().is_empty();
    col().key("catalog").h(CATALOG_HEIGHT).min_h(0.0).child(
        col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .px(METRICS.row_px - 8.0)
            .pb(8.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .children(matches.into_iter().enumerate().map(|(i, app)| {
                catalog_row(
                    p,
                    app,
                    is_pinned(s, &app.bundle_id),
                    highlight_top && i == 0,
                )
            })),
    )
}

/// A catalog row is one clickable target: pressing it pins the app.
fn catalog_row(p: Paint, app: &CatalogApp, pinned: bool, top: bool) -> El {
    let row_app = app.clone();
    let mut item = row()
        .key(format!("cat-{}", app.bundle_id))
        .h(36.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(p.k.radius.md)
        .hover(p.k.c.hover)
        .on_press(move |s: &mut SettingsState| add_app(s, &row_app))
        .child(monogram(p, &app.name))
        .child(
            text(app.name.clone())
                .font(p.k.ty.ui)
                .color(if pinned { p.muted() } else { p.text() })
                .grow(1.0)
                .truncate(),
        )
        .child(
            text(app.bundle_id.clone())
                .font(p.k.ty.small)
                .color(p.muted())
                .grow(1.0)
                .truncate(),
        );
    let status = if pinned {
        row()
            .key("pinned")
            .gap(4.0)
            .center_y()
            .child(icon(&rune_theme_opencode::icons::CHECK_SMALL, 14.0).color(p.k.c.icon_muted))
            .child(text("Pinned").font(p.k.ty.small).color(p.muted()))
    } else {
        icon(&rune_theme_opencode::icons::PLUS, 14.0)
            .key("add")
            .color(p.k.c.icon_muted)
    };
    item = item.child(
        row()
            .key("status")
            .w(64.0)
            .center_y()
            .justify_end()
            .child(status),
    );
    if top {
        item = item.fill(p.k.c.layer_02);
    }
    item
}

fn trigger_name(trigger: SwitcherTrigger) -> &'static str {
    match trigger {
        SwitcherTrigger::CapsE => "Caps Lock + E",
        SwitcherTrigger::CmdTab => "⌘ Tab",
        SwitcherTrigger::Both => "Either",
    }
}

fn trigger_row(s: &SettingsState, p: Paint, layer: &mut Vec<El>) -> El {
    let choices = SwitcherTrigger::ALL
        .into_iter()
        .map(|trigger| Choice {
            label: trigger_name(trigger).to_string(),
            selected: s.trigger == trigger,
            choose: Box::new(move |s: &mut SettingsState, tx: &mut Tx| {
                choose_trigger(s, trigger, tx)
            }),
        })
        .collect();
    setting_row(
        p,
        "trigger",
        "Open the switcher with",
        "Hold ⌘ and tap Caps Lock, or hold Caps Lock and press E. Tab flips between pinned and open apps.",
        select(
            p,
            "trigger",
            trigger_name(s.trigger),
            choices,
            s.trigger_menu_open,
            |s| &mut s.trigger_menu_open,
            layer,
        ),
    )
}

fn tap_row(s: &SettingsState, p: Paint) -> El {
    let stepper = row()
        .key("stepper")
        .gap(4.0)
        .center_y()
        .child(small_button(
            p,
            "less",
            "−",
            ButtonVariant::Ghost,
            |s: &mut SettingsState| s.tap_max = s.tap_max.saturating_sub(20).max(80),
        ))
        .child(
            text(format!("{} ms", s.tap_max))
                .key("value")
                .font(p.k.ty.ui)
                .color(p.text())
                .w(64.0)
                .align(rune::Align::Center),
        )
        .child(small_button(
            p,
            "more",
            "+",
            ButtonVariant::Ghost,
            |s: &mut SettingsState| s.tap_max = s.tap_max.saturating_add(20).min(600),
        ));
    setting_row(
        p,
        "tap",
        "Tap threshold",
        "How long a hold may last and still count as Esc",
        stepper,
    )
}

/// The key-hint layout picker: "Follow current layout" plus every installed
/// keyboard layout. Choosing applies the layout at once and saves it.
fn hint_row(s: &SettingsState, p: Paint, layer: &mut Vec<El>) -> El {
    let mut choices = vec![Choice {
        label: "Follow current layout".to_string(),
        selected: s.hint_layout.is_none(),
        choose: Box::new(|s: &mut SettingsState, tx: &mut Tx| choose_layout(s, None, tx)),
    }];
    choices.extend(s.layouts.iter().map(|l| {
        let id = Some(l.id.clone());
        Choice {
            label: l.name.clone(),
            selected: s.hint_layout.as_deref() == Some(l.id.as_str()),
            choose: Box::new(move |s: &mut SettingsState, tx: &mut Tx| {
                choose_layout(s, id.clone(), tx)
            }),
        }
    }));
    let label = choices.iter().find(|c| c.selected).map_or_else(
        || s.hint_layout.clone().unwrap_or_default(),
        |c| c.label.clone(),
    );
    setting_row(
        p,
        "hint-layout",
        "Key hint layout",
        "Which installed layout spells the switcher's trigger hints",
        select(
            p,
            "hint-layout",
            &label,
            choices,
            s.hint_menu_open,
            |s| &mut s.hint_menu_open,
            layer,
        ),
    )
}

fn permission_card(s: &SettingsState, p: Paint) -> El {
    let mut rows = Vec::new();
    if !s.perm_ready {
        rows.push(banner(
            p,
            "degraded",
            "Mojito can't watch the keyboard until Accessibility is granted. The switcher stays off.",
            p.k.c.warning,
        ));
    }
    let control = row()
        .key("access-control")
        .gap(8.0)
        .center_y()
        .child(state_badge(p, s.perm_ready, "Ready", "Needs grant"))
        .children((!s.perm_ready).then(|| {
            button(
                p,
                "open-settings",
                "Open System Settings",
                ButtonVariant::Neutral,
                |_: &mut SettingsState, _: &mut Tx| permission::prompt(),
            )
        }));
    rows.push(setting_row(
        p,
        "accessibility",
        "Accessibility",
        "Watch Caps Lock and move windows between apps",
        control,
    ));
    if !s.perm_ready {
        rows.push(setting_row(
            p,
            "stale",
            "Still stuck after granting?",
            "Reset the saved permission and prompt again",
            small_button(
                p,
                "reset",
                "Reset and re-prompt",
                ButtonVariant::Neutral,
                |_: &mut SettingsState| permission::offer_stale_reset(),
            ),
        ));
    }
    section(p, "permissions", "Permissions", rows)
}

fn remap_card(s: &SettingsState, p: Paint) -> El {
    let installed = s.remap_active;
    let label = if installed {
        "Restore Caps Lock"
    } else {
        "Install remap"
    };
    let control = row()
        .key("remap-control")
        .gap(8.0)
        .center_y()
        .child(state_badge(p, installed, "Installed", "Not installed"))
        .child(button(
            p,
            "remap",
            label,
            ButtonVariant::Neutral,
            move |s, _| toggle_remap(s, !installed),
        ));
    section(
        p,
        "remap",
        "Caps Lock remap",
        vec![setting_row(
            p,
            "remap-row",
            "Caps Lock → F18",
            "Lets the engine own Caps Lock without touching apps",
            control,
        )],
    )
}

fn state_badge(p: Paint, ok: bool, on: &str, off: &str) -> El {
    let (label, variant) = if ok {
        (on, BadgeVariant::Accent)
    } else {
        (off, BadgeVariant::Neutral)
    };
    Badge::new(label).variant(variant).build(p.k).key("badge")
}
