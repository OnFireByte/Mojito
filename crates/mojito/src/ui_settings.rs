//! The settings window: pinned apps with trigger-key capture, the tap
//! threshold, live permission and remap status. The only config writer;
//! errors land in the footer line and never block the window.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use mojito_core::config::{self, PinnedApp};
use mojito_core::labels::{LABEL_PRIORITY, RESERVED};
use mojito_core::{ConfigStore, PhysicalKey};
use mojito_macos::caps::CapsMap;
use mojito_macos::{layout, permission};
use rune::tok::type_;
use rune::{AppHandle, Cx, El, PhysicalInput, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, IconButton, Kit};

use crate::Hub;
use crate::catalog::{self, CatalogApp};
use crate::keysym;

pub const SIZE: (f32, f32) = (560.0, 520.0);

/// A pinned app under edit; `key` stays None until the user captures one.
struct PinnedDraft {
    name: String,
    bundle_id: String,
    key: Option<PhysicalKey>,
    glyph: Option<char>,
}

enum CatalogState {
    Idle,
    Loading,
    Ready(Vec<CatalogApp>),
}

struct Status {
    text: String,
    error: bool,
}

pub struct SettingsState {
    store: Arc<ConfigStore>,
    hub: AppHandle<Hub>,
    rows: Vec<PinnedDraft>,
    tap_max: u16,
    capturing: Option<usize>,
    catalog_open: bool,
    catalog: CatalogState,
    catalog_rx: Option<mpsc::Receiver<Vec<CatalogApp>>>,
    perm_ready: bool,
    remap_active: bool,
    status: Option<Status>,
    clear_pending: bool,
}

impl SettingsState {
    /// Never runs: `App::init` replaces `AppSpec::init`.
    pub fn placeholder() -> Self {
        unreachable!("App::init replaces AppSpec::init")
    }

    pub fn new(store: Arc<ConfigStore>, hub: AppHandle<Hub>) -> Self {
        let mut s = Self {
            store,
            hub,
            rows: Vec::new(),
            tap_max: 250,
            capturing: None,
            catalog_open: false,
            catalog: CatalogState::Idle,
            catalog_rx: None,
            perm_ready: false,
            remap_active: false,
            status: None,
            clear_pending: false,
        };
        s.reload();
        s
    }

    /// Draft from the live config, plus the live permission and remap state.
    /// Called when the window opens (from the menu pump), not on every
    /// refocus, so in-progress edits survive a click away.
    fn reload(&mut self) {
        let config = self.store.handle().load_full();
        self.rows = config
            .pinned
            .iter()
            .map(|p| PinnedDraft {
                name: p.name.clone(),
                bundle_id: p.bundle_id.clone(),
                key: Some(p.key),
                glyph: layout::glyph(p.key),
            })
            .collect();
        self.tap_max = config.tap_max_ms;
        self.capturing = None;
        self.perm_ready = permission::state() == permission::PermissionState::Ready;
        self.remap_active = CapsMap.is_active();
    }
}

/// Runs when the status item opens the window (right after `open`).
pub fn opened(s: &mut SettingsState, _: &mut Tx) {
    s.reload();
}

/// The window closed: back to accessory.
pub fn closed(s: &mut SettingsState, _: &mut Tx) {
    s.hub.set_dock_icon(false);
    s.capturing = None;
}

/// Settings poll: delivers the catalog scan and clears the save toast.
pub fn poll(s: &mut SettingsState, _: f64, _: &mut Tx) -> bool {
    let mut acted = false;
    if let Some(rx) = &s.catalog_rx
        && let Ok(list) = rx.try_recv()
    {
        s.catalog = CatalogState::Ready(list);
        s.catalog_rx = None;
        acted = true;
    }
    if s.clear_pending {
        s.clear_pending = false;
        s.status = None;
        acted = true;
    }
    acted
}

fn save(s: &mut SettingsState, tx: &mut Tx) {
    let unkeyed = s.rows.iter().filter(|r| r.key.is_none()).count();
    let mut next = (*s.store.handle().load_full()).clone();
    next.pinned = s
        .rows
        .iter()
        .filter_map(|r| {
            r.key.map(|key| PinnedApp {
                name: r.name.clone(),
                bundle_id: r.bundle_id.clone(),
                key,
            })
        })
        .collect();
    next.tap_max_ms = s.tap_max;
    next.version += 1;
    let (next, warnings) = config::validate(next);
    match s.store.save(next) {
        Ok(()) => {
            let mut note = if unkeyed > 0 {
                format!("Saved · {unkeyed} app(s) still need a trigger key")
            } else {
                "Saved".to_string()
            };
            if !warnings.is_empty() {
                let list: Vec<String> = warnings.iter().map(|w| w.0.clone()).collect();
                note.push_str(" · ");
                note.push_str(&list.join("; "));
            }
            s.status = Some(Status {
                text: note,
                error: !warnings.is_empty() && unkeyed == 0,
            });
        }
        Err(error) => {
            s.status = Some(Status {
                text: format!("Save failed: {error}"),
                error: true,
            });
        }
    }
    s.clear_pending = true;
    tx.wake_after(Duration::from_secs(4));
}

/// Steal the key from any other row: duplicates never survive a save, so
/// claiming here is the honest UX.
fn assign(s: &mut SettingsState, row: usize, key: PhysicalKey) {
    for (i, r) in s.rows.iter_mut().enumerate() {
        if i != row && r.key == Some(key) {
            r.key = None;
            r.glyph = None;
        }
    }
    let draft = &mut s.rows[row];
    draft.key = Some(key);
    draft.glyph = layout::glyph(key);
    s.capturing = None;
}

fn start_capture(s: &mut SettingsState, row: usize) {
    s.capturing = Some(row);
}

fn remove_row(s: &mut SettingsState, row: usize) {
    s.rows.remove(row);
    s.capturing = match s.capturing {
        Some(i) if i == row => None,
        Some(i) if i > row => Some(i - 1),
        other => other,
    };
}

/// First free key in reach order, skipping reserved and already-pinned keys.
fn next_free_key(s: &SettingsState) -> Option<PhysicalKey> {
    let used: Vec<PhysicalKey> = s.rows.iter().filter_map(|r| r.key).collect();
    LABEL_PRIORITY
        .iter()
        .copied()
        .find(|k| !RESERVED.contains(k) && !used.contains(k))
}

fn add_app(s: &mut SettingsState, app: &CatalogApp) {
    if s.rows.iter().any(|r| r.bundle_id == app.bundle_id) {
        s.status = Some(Status {
            text: format!("{} is already pinned", app.name),
            error: false,
        });
        return;
    }
    let key = next_free_key(s);
    s.rows.push(PinnedDraft {
        name: app.name.clone(),
        bundle_id: app.bundle_id.clone(),
        key,
        glyph: key.and_then(layout::glyph),
    });
    s.catalog_open = false;
}

fn open_catalog(s: &mut SettingsState) {
    s.catalog_open = true;
    if matches!(s.catalog, CatalogState::Idle) {
        s.catalog = CatalogState::Loading;
        let (tx, rx) = mpsc::channel();
        s.catalog_rx = Some(rx);
        catalog::scan(tx);
    }
}

pub fn view(s: &SettingsState, cx: &mut Cx<SettingsState>) -> El {
    let k = rune_theme_opencode::kit(cx.system_dark());
    let content = col()
        .key("content")
        .grow(1.0)
        .min_h(0.0)
        .px(24.0)
        .pt(16.0)
        .pb(12.0)
        .gap(20.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(pinned_section(s, k))
        .child(behavior_section(s, k))
        .child(permission_section(s, k))
        .child(remap_section(s, k));
    col()
        .w_full()
        .h_full()
        .fill(k.c.page)
        .child(content)
        .child(footer(s, k))
}

fn section(title: &str, k: &Kit, rows: Vec<El>) -> El {
    col()
        .key(format!("section-{title}"))
        .gap(8.0)
        .child(
            text(title.to_string())
                .font(type_::SECTION)
                .color(k.c.text_muted),
        )
        .child(
            col()
                .key("card")
                .gap(0.0)
                .fill(k.c.surface_raised_base)
                .radius(k.radius.lg)
                .border(1.0, k.c.border_weak)
                .children(separated(rows, k)),
        )
}

fn separated(rows: Vec<El>, k: &Kit) -> Vec<El> {
    let mut out = Vec::new();
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            out.push(
                el().key(format!("sep-{i}"))
                    .h(1.0)
                    .w_full()
                    .fill(k.c.border_weak),
            );
        }
        out.push(r);
    }
    out
}

fn label_cell(k: &Kit, title: &str, sub: Option<String>) -> El {
    let mut cell = col().key("label").grow(1.0).min_w(0.0).gap(1.0).child(
        text(title.to_string())
            .font(k.ty.ui_medium)
            .color(k.c.text_base),
    );
    if let Some(sub) = sub {
        cell = cell.child(text(sub).font(k.ty.small).color(k.c.text_muted).truncate());
    }
    cell
}

fn pinned_section(s: &SettingsState, k: &Kit) -> El {
    let mut rows: Vec<El> = s
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| pinned_row(s, i, r, k))
        .collect();
    rows.push(row().key("add").h(40.0).px(12.0).center_y()
        .child(
            Button::new(if s.catalog_open { "Hide list" } else { "Add app…" })
                .variant(ButtonVariant::Ghost)
                .on_press(open_catalog)
                .build(k),
        )
        .child(el().grow(1.0))
        .child(text("Trigger keys are physical positions: they hold under any layout or input language.")
            .font(k.ty.small).color(k.c.text_muted).w(220.0).wrap()));
    if s.catalog_open {
        rows.push(catalog_list(s, k));
    }
    section("Pinned apps", k, rows)
}

/// One pinned-app row. Clicking anywhere captures the next physical key for
/// its trigger; the row is focusable so `on_physical_input` reaches it.
fn pinned_row(s: &SettingsState, i: usize, r: &PinnedDraft, k: &Kit) -> El {
    let capturing = s.capturing == Some(i);
    let badge = if capturing {
        "Press a key…".to_string()
    } else {
        r.glyph.map_or_else(|| "—".to_string(), |c| c.to_string())
    };
    row()
        .key(format!("pin-{i}"))
        .h(40.0)
        .px(12.0)
        .gap(10.0)
        .center_y()
        .focusable(true)
        .on_press(move |s: &mut SettingsState| start_capture(s, i))
        .on_physical_input(
            move |s: &mut SettingsState, ev: &PhysicalInput, _: &mut Tx| capture_input(s, i, ev),
        )
        .child(
            el().key("mono")
                .size(24.0, 24.0)
                .radius(k.radius.md)
                .center()
                .fill(k.c.layer_03)
                .child(
                    text(
                        r.name
                            .chars()
                            .next()
                            .map_or('?', |c| c.to_uppercase().next().unwrap_or(c))
                            .to_string(),
                    )
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base),
                ),
        )
        .child(label_cell(k, &r.name, Some(r.bundle_id.clone())))
        .child(
            el().key("badge")
                .h(24.0)
                .px(10.0)
                .center_y()
                .radius(k.radius.md)
                .fill(if capturing {
                    k.c.bg_accent
                } else {
                    k.c.layer_02
                })
                .child(text(badge).font(k.ty.small_medium).color(if capturing {
                    k.c.text_contrast
                } else {
                    k.c.text_muted
                })),
        )
        .child(
            IconButton::new(&rune_theme_opencode::icons::CLOSE)
                .label("Remove")
                .control_size(rune_kit::ControlSize::Small)
                .on_press(move |s: &mut SettingsState| remove_row(s, i))
                .build(k),
        )
}

/// The focused capture row's physical keys. Only while that row is capturing:
/// Esc cancels, reserved keys are rejected, anything mappable assigns.
/// Modifiers and unknown keys are rejected silently.
fn capture_input(s: &mut SettingsState, row: usize, ev: &PhysicalInput) -> bool {
    match ev {
        PhysicalInput::Key {
            key,
            pressed: true,
            repeat: false,
            mods,
        } => {
            if s.capturing != Some(row) {
                return false;
            }
            if !mods_are_bare(mods) {
                return true;
            }
            if *key == rune::PhysicalKeyCode::Escape {
                s.capturing = None;
                return true;
            }
            match keysym::physical(*key) {
                Some(pk) if RESERVED.contains(&pk) => {
                    s.status = Some(Status {
                        text: "That key is reserved (meta, esc, return, m)".to_string(),
                        error: true,
                    });
                }
                Some(pk) => assign(s, row, pk),
                None => {
                    s.status = Some(Status {
                        text: "That key can't be a trigger".to_string(),
                        error: true,
                    });
                }
            }
            true
        }
        PhysicalInput::FocusLost => {
            s.capturing = None;
            true
        }
        _ => false,
    }
}

fn mods_are_bare(mods: &rune::Mods) -> bool {
    !(mods.shift || mods.cmd || mods.alt || mods.ctrl || mods.logo)
}

fn catalog_list(s: &SettingsState, k: &Kit) -> El {
    let body = match &s.catalog {
        CatalogState::Idle | CatalogState::Loading => col().child(
            text("Scanning installed apps…")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        ),
        CatalogState::Ready(list) if list.is_empty() => {
            col().child(text("No apps found").font(k.ty.ui).color(k.c.text_muted))
        }
        CatalogState::Ready(list) => {
            col()
                .key("catalog")
                .gap(2.0)
                .children(list.iter().take(40).enumerate().map(|(i, app)| {
                    row()
                        .key(format!("cat-{i}"))
                        .h(32.0)
                        .px(10.0)
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(app.name.clone())
                                .font(k.ty.ui)
                                .color(k.c.text_base)
                                .grow(1.0)
                                .truncate(),
                        )
                        .child(
                            text(app.bundle_id.clone())
                                .font(k.ty.small)
                                .color(k.c.text_muted)
                                .grow(1.0)
                                .truncate(),
                        )
                        .child({
                            let app = app.clone();
                            Button::new("Add")
                                .variant(ButtonVariant::Ghost)
                                .on_press(move |s: &mut SettingsState| add_app(s, &app))
                                .build(k)
                        })
                }))
        }
    };
    // Bounded height so the section list scrolls instead of stretching.
    col()
        .key("catalog-wrap")
        .h(180.0)
        .min_h(0.0)
        .px(12.0)
        .pb(8.0)
        .child(
            col()
                .w_full()
                .h_full()
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
}

fn behavior_section(s: &SettingsState, k: &Kit) -> El {
    section(
        "Behavior",
        k,
        vec![
            row()
                .key("tap")
                .h(40.0)
                .px(12.0)
                .gap(10.0)
                .center_y()
                .child(label_cell(
                    k,
                    "Tap threshold",
                    Some("How long a hold may last and still count as Esc".into()),
                ))
                .child(
                    Button::new("−")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|s: &mut SettingsState| {
                            s.tap_max = s.tap_max.saturating_sub(20).max(80)
                        })
                        .build(k),
                )
                .child(
                    text(format!("{} ms", s.tap_max))
                        .font(k.ty.text_small_medium)
                        .color(k.c.text_base)
                        .w(64.0)
                        .align(rune::Align::Center),
                )
                .child(
                    Button::new("+")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|s: &mut SettingsState| {
                            s.tap_max = s.tap_max.saturating_add(20).min(600)
                        })
                        .build(k),
                ),
        ],
    )
}

fn permission_section(s: &SettingsState, k: &Kit) -> El {
    let badge = if s.perm_ready {
        Badge::new("Ready").variant(BadgeVariant::Accent)
    } else {
        Badge::new("Needs grant")
    };
    section(
        "Permissions",
        k,
        vec![
            row()
                .key("accessibility")
                .h(40.0)
                .px(12.0)
                .gap(10.0)
                .center_y()
                .child(label_cell(
                    k,
                    "Accessibility",
                    Some("Watch Caps Lock and move windows between apps".into()),
                ))
                .child(badge.build(k))
                .child(
                    Button::new("Open System Settings")
                        .on_press(|_: &mut SettingsState| permission::prompt())
                        .build(k),
                ),
        ],
    )
}

fn remap_section(s: &SettingsState, k: &Kit) -> El {
    let badge = if s.remap_active {
        Badge::new("Installed").variant(BadgeVariant::Accent)
    } else {
        Badge::new("Not installed")
    };
    let action: El = if s.remap_active {
        Button::new("Restore Caps Lock")
            .on_press(|s: &mut SettingsState| toggle_remap(s, false))
            .build(k)
    } else {
        Button::new("Install remap")
            .on_press(|s: &mut SettingsState| toggle_remap(s, true))
            .build(k)
    };
    section(
        "Caps Lock remap",
        k,
        vec![
            row()
                .key("remap")
                .h(40.0)
                .px(12.0)
                .gap(10.0)
                .center_y()
                .child(label_cell(
                    k,
                    "Caps Lock → F18",
                    Some("Lets the engine own Caps Lock without touching apps".into()),
                ))
                .child(badge.build(k))
                .child(action),
        ],
    )
}

fn toggle_remap(s: &mut SettingsState, install: bool) {
    let caps = CapsMap;
    let result = if install {
        caps.install()
    } else {
        caps.uninstall()
    };
    match result {
        Ok(()) => {
            s.remap_active = caps.is_active();
            s.status = Some(Status {
                text: if s.remap_active {
                    "Remap installed".into()
                } else {
                    "Caps Lock restored".into()
                },
                error: false,
            });
        }
        Err(error) => {
            s.status = Some(Status {
                text: error,
                error: true,
            })
        }
    }
}

fn footer(s: &SettingsState, k: &Kit) -> El {
    let status = s.status.as_ref().map(|st| {
        text(st.text.clone())
            .key("status")
            .font(k.ty.small)
            .color(if st.error {
                k.c.danger.fg
            } else {
                k.c.text_muted
            })
            .grow(1.0)
            .truncate()
    });
    col()
        .key("footer-area")
        .w_full()
        .child(
            el().key("footer-rule")
                .h(1.0)
                .w_full()
                .fill(k.c.border_weak),
        )
        .child(
            row()
                .key("footer")
                .h(52.0)
                .px(24.0)
                .gap(12.0)
                .center_y()
                .fill(k.c.surface_base)
                .children(status)
                .child(el().grow(1.0))
                .child(
                    Button::new("Save")
                        .variant(ButtonVariant::Submit)
                        .on_press_tx(save)
                        .build(k),
                ),
        )
}
