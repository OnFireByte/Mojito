//! The three sections' content: pinned apps with trigger capture, behavior
//! (the tap threshold and the key-hint layout picker), and system
//! (permissions and the Caps Lock remap).

use rune::{El, PhysicalInput, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{Badge, BadgeVariant, ButtonVariant};

use super::controls::{
    Choice, METRICS, Paint, banner, button, icon_button, note, pressable_row, pulsing_dot, section,
    select, setting_row, small_button,
};
use super::{
    CatalogState, PinnedDraft, SettingsState, add_app, capture_input, choose_layout, remove_row,
    start_capture, toggle_remap,
};
use crate::catalog::CatalogApp;
use mojito_macos::permission;

/// How tall the catalog list may grow before it scrolls inside its card.
const CATALOG_HEIGHT: f32 = 240.0;

/// The sections column; `view` supplies the page's gutter.
fn sections(key: &str) -> El {
    col().key(key).gap(METRICS.section_gap)
}

pub(super) fn apps(s: &SettingsState, p: Paint, _layer: &mut Vec<El>) -> El {
    let mut list = sections("apps").child(pinned_card(s, p));
    if s.catalog_open {
        list = list.child(catalog_card(s, p));
    }
    list
}

pub(super) fn behavior(s: &SettingsState, p: Paint, layer: &mut Vec<El>) -> El {
    sections("behavior")
        .child(section(p, "tapping", "Tapping", vec![tap_row(s, p)]))
        .child(section(
            p,
            "hints",
            "Key hints",
            vec![hint_row(s, p, layer)],
        ))
}

pub(super) fn system(s: &SettingsState, p: Paint, _layer: &mut Vec<El>) -> El {
    sections("system")
        .child(permission_card(s, p))
        .child(remap_card(s, p))
}

// --- Apps -----------------------------------------------------------------

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
    pressable_row(
        row()
            .key(format!("pin-{i}"))
            .min_h(METRICS.row_h)
            .px(METRICS.row_px)
            .py(8.0)
            .gaps(10.0, 16.0)
            .center_y()
            .focusable(true)
            .on_press(move |s: &mut SettingsState| start_capture(s, i))
            .on_physical_input(
                move |s: &mut SettingsState, ev: &PhysicalInput, _: &mut Tx| {
                    capture_input(s, i, ev)
                },
            )
            .child(monogram(p, &r.name))
            .child(label_cell(p, &r.name, Some(r.bundle_id.clone())))
            .child(trigger_badge(p, capturing, r.glyph))
            .child(icon_button(
                p,
                "remove",
                &rune_theme_opencode::icons::CLOSE,
                "Remove",
                move |s: &mut SettingsState| remove_row(s, i),
            )),
    )
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

fn label_cell(p: Paint, title: &str, sub: Option<String>) -> El {
    let mut cell = col().key("label").grow(1.0).min_w(0.0).gap(1.0).child(
        text(title.to_string())
            .font(p.k.ty.ui)
            .color(p.text())
            .truncate(),
    );
    if let Some(sub) = sub {
        cell = cell.child(
            text(sub)
                .key("sub")
                .font(p.k.ty.small)
                .color(p.muted())
                .truncate(),
        );
    }
    cell
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
        CatalogState::Ready(list) => {
            // Bounded height so the page scrolls instead of stretching.
            col().key("catalog").h(CATALOG_HEIGHT).min_h(0.0).child(
                col()
                    .w_full()
                    .h_full()
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .children(
                        list.iter()
                            .take(40)
                            .enumerate()
                            .map(|(i, app)| catalog_row(p, i, app)),
                    ),
            )
        }
    };
    section(p, "catalog", "Choose an app", vec![body])
}

/// A catalog row is one clickable target: pressing it pins the app.
fn catalog_row(p: Paint, i: usize, app: &CatalogApp) -> El {
    let name = app.name.clone();
    let bundle_id = app.bundle_id.clone();
    let row_app = app.clone();
    row()
        .key(format!("cat-{i}"))
        .h(36.0)
        .px(METRICS.row_px)
        .gap(10.0)
        .center_y()
        .radius(p.k.radius.md)
        .hover(p.k.c.hover)
        .on_press(move |s: &mut SettingsState| add_app(s, &row_app))
        .child(
            text(name)
                .font(p.k.ty.ui)
                .color(p.text())
                .grow(1.0)
                .truncate(),
        )
        .child(
            text(bundle_id)
                .font(p.k.ty.small)
                .color(p.muted())
                .grow(1.0)
                .truncate(),
        )
        .child(
            icon(&rune_theme_opencode::icons::PLUS, 14.0)
                .key("add")
                .color(p.k.c.icon_muted),
        )
}

// --- Behavior -------------------------------------------------------------

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
        Some("How long a hold may last and still count as Esc"),
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
    let label = s.hint_layout.as_ref().map_or_else(
        || "Follow current layout".to_string(),
        |id| {
            s.layouts
                .iter()
                .find(|l| &l.id == id)
                .map_or_else(|| id.clone(), |l| l.name.clone())
        },
    );
    setting_row(
        p,
        "hint-layout",
        "Key hint layout",
        Some("Which installed layout spells the switcher's trigger hints"),
        select(
            p,
            "hint-layout",
            &label,
            s.hint_menu_open,
            choices,
            |s: &mut SettingsState| s.hint_menu_open = !s.hint_menu_open,
            |s: &mut SettingsState| s.hint_menu_open = false,
            layer,
        ),
    )
}

// --- System ---------------------------------------------------------------

fn permission_card(s: &SettingsState, p: Paint) -> El {
    let mut rows = Vec::new();
    if !s.perm_ready {
        // Degraded mode: the engine cannot run, so say so before the rows.
        rows.push(banner(
            p,
            "degraded",
            "Mojito can't watch the keyboard until Accessibility is granted. The switcher stays off.",
            p.k.c.warning,
        ));
    }
    let badge = if s.perm_ready {
        Badge::new("Ready")
            .variant(BadgeVariant::Accent)
            .build(p.k)
            .key("badge")
    } else {
        Badge::new("Needs grant").build(p.k).key("badge")
    };
    let control = row()
        .key("access-control")
        .gap(8.0)
        .center_y()
        .child(badge)
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
        Some("Watch Caps Lock and move windows between apps"),
        control,
    ));
    if !s.perm_ready {
        rows.push(setting_row(
            p,
            "stale",
            "Still stuck after granting?",
            Some("Reset the saved permission and prompt again"),
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
    let badge = if s.remap_active {
        Badge::new("Installed")
            .variant(BadgeVariant::Accent)
            .build(p.k)
            .key("badge")
    } else {
        Badge::new("Not installed").build(p.k).key("badge")
    };
    let action = if s.remap_active {
        button(
            p,
            "remap",
            "Restore Caps Lock",
            ButtonVariant::Neutral,
            |s: &mut SettingsState, _: &mut Tx| toggle_remap(s, false),
        )
    } else {
        button(
            p,
            "remap",
            "Install remap",
            ButtonVariant::Neutral,
            |s: &mut SettingsState, _: &mut Tx| toggle_remap(s, true),
        )
    };
    let control = row()
        .key("remap-control")
        .gap(8.0)
        .center_y()
        .child(badge)
        .child(action);
    section(
        p,
        "remap",
        "Caps Lock remap",
        vec![setting_row(
            p,
            "remap-row",
            "Caps Lock → F18",
            Some("Lets the engine own Caps Lock without touching apps"),
            control,
        )],
    )
}
