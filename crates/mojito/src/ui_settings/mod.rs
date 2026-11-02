//! The settings window: a Hex-style sidebar (Apps, Behavior, System) beside a
//! raised content panel. State and handlers live here, the frame in `view`,
//! the sections in `panes`. The only config writer; errors land in the footer
//! line and never block the window.

mod controls;
mod panes;
mod view;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use mojito_core::config::{self, PinnedApp};
use mojito_core::labels::{LABEL_PRIORITY, RESERVED};
use mojito_core::{ConfigStore, PhysicalKey};
use mojito_macos::caps::CapsMap;
use mojito_macos::layout::LayoutInfo;
use mojito_macos::{layout, permission};
use rune::{App, AppHandle, Key, KeyEvent, PhysicalInput, RunOptions, TextEdit, Tx, WindowSpec};

use crate::Hub;
use crate::catalog::{self, CatalogApp};
use crate::keysym;

pub use view::view;

/// Fixed size, so tiling window managers float the settings window instead of
/// tiling it.
pub const SIZE: (f32, f32) = (980.0, 760.0);

/// How often the open window re-reads the permission state, as hex's pane
/// does. Remap state is read on open and after its own actions only: it costs
/// a `hidutil` subprocess.
const REFRESH_AFTER: Duration = Duration::from_secs(1);

/// A sidebar section.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Apps,
    Behavior,
    System,
}

impl Pane {
    const ALL: [Pane; 3] = [Self::Apps, Self::Behavior, Self::System];

    fn title(self) -> &'static str {
        match self {
            Self::Apps => "Apps",
            Self::Behavior => "Behavior",
            Self::System => "System",
        }
    }

    fn nav_key(self) -> &'static str {
        match self {
            Self::Apps => "nav-apps",
            Self::Behavior => "nav-behavior",
            Self::System => "nav-system",
        }
    }

    fn icon(self) -> &'static rune::Icon {
        match self {
            Self::Apps => &rune_theme_opencode::icons::GRID_PLUS,
            Self::Behavior => &rune_theme_opencode::icons::OUTLINE_DOTS,
            Self::System => &rune_theme_opencode::icons::STATUS,
        }
    }
}

/// A pinned app under edit; `key` stays None until the user captures one.
pub(super) struct PinnedDraft {
    name: String,
    bundle_id: String,
    key: Option<PhysicalKey>,
    glyph: Option<char>,
}

pub(super) enum CatalogState {
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
    pane: Pane,
    rows: Vec<PinnedDraft>,
    capturing: Option<usize>,
    catalog_open: bool,
    catalog: CatalogState,
    catalog_rx: Option<mpsc::Receiver<Vec<CatalogApp>>>,
    query: TextEdit,
    tap_max: u16,
    /// The chosen hint layout (InputSourceID); None follows the current one.
    hint_layout: Option<String>,
    /// Installed layouts for the picker, listed when the window opens (the
    /// TIS call is main-thread only).
    layouts: Vec<LayoutInfo>,
    hint_menu_open: bool,
    perm_ready: bool,
    remap_active: bool,
    status: Option<Status>,
    clear_pending: bool,
    /// The permission tick runs while the window shows.
    live_refresh: bool,
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
            pane: Pane::Apps,
            rows: Vec::new(),
            capturing: None,
            catalog_open: false,
            catalog: CatalogState::Idle,
            catalog_rx: None,
            query: TextEdit::new(""),
            tap_max: 250,
            hint_layout: None,
            layouts: Vec::new(),
            hint_menu_open: false,
            perm_ready: false,
            remap_active: false,
            status: None,
            clear_pending: false,
            live_refresh: false,
        };
        s.reload();
        // A fresh install lands on the pane that unblocks the engine.
        if !s.perm_ready {
            s.pane = Pane::System;
        }
        s
    }

    /// Draft from the live config, plus the live permission, remap, and
    /// layout state. Called when the window opens (from the menu pump), not
    /// on every refocus, so in-progress edits survive a click away.
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
        self.hint_layout = config.hint_layout.clone();
        self.layouts = layout::installed();
        self.capturing = None;
        self.perm_ready = permission::state() == permission::PermissionState::Ready;
        self.remap_active = CapsMap.is_active();
    }
}

/// Runs when the status item opens the window (right after `open`).
pub fn opened(s: &mut SettingsState, tx: &mut Tx) {
    eprintln!("mojito: settings window live"); // TEMP diagnosis
    s.reload();
    s.live_refresh = true;
    tx.wake_after(REFRESH_AFTER);
}

/// The settings process: one rune loop and one window. Closing it exits the
/// process and returns every byte it held to the system; saves post the
/// reserved reload keycode so the engine applies them live.
pub fn run_child() {
    let store = Arc::new(ConfigStore::open(crate::settings_path()));
    // Regular, not accessory: an accessory app's window cannot become key
    // when it opens, so the settings window would start unfocused.
    rune::run_windows_with_args(RunOptions { accessory: false }, Vec::new(), move |_| {
        // Glyph lookups need this process's own snapshot: the hub builds
        // one at startup, but TABLE lives per process, and without it every
        // trigger badge renders the dash instead of its key's character.
        layout::snapshot();
        // The hint picker's saved choice decides which layout spells the
        // badges, exactly as the hub applies it before the first reveal.
        layout::set_display_layout(store.handle().load().hint_layout.clone());
        // The state's handle wants a Hub; a detached windowless hub app
        // satisfies it without adding a surface. This process's lifetime
        // is the settings window's, so no keep_running.
        let hub = App::new(crate::app_spec::hub_spec()).windowless().handle();
        let app = App::new(crate::app_spec::settings_spec())
            .standard_menus()
            .on_close(closed)
            .init({
                let store = store.clone();
                move || SettingsState::new(store, hub)
            });
        let handle = app.handle();
        let app = app.on_ready(move |_| {
            handle.update(opened);
            // A process spawned from another app gets no activation
            // ceremony; take focus once the window exists.
            mojito_macos::status_item::activate_self();
        });
        vec![
            WindowSpec::from_app("settings", app)
                .configured_options(crate::app_spec::settings_window_options()),
        ]
    });
}

/// The window closed: back to accessory, and the periodic reads stop.
pub fn closed(s: &mut SettingsState, _: &mut Tx) {
    s.hub.set_dock_icon(false);
    s.capturing = None;
    s.hint_menu_open = false;
    close_catalog(s);
    s.live_refresh = false;
    let usage = procwatch::usage();
    let mb = |bytes: Option<u64>| {
        bytes.map_or_else(|| "?".into(), |b| format!("{:.1}MB", b as f64 / 1e6))
    };
    eprintln!(
        "mojito: settings closed; footprint {} peak {}",
        mb(usage.footprint),
        mb(usage.footprint_peak)
    ); // TEMP diagnosis
}

/// Settings poll: delivers the catalog scan, clears the save toast, and
/// re-reads the permission state once a second while the window shows.
pub fn poll(s: &mut SettingsState, _: f64, tx: &mut Tx) -> bool {
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
    if s.live_refresh {
        tx.wake_after(REFRESH_AFTER);
        let ready = permission::state() == permission::PermissionState::Ready;
        if ready != s.perm_ready {
            s.perm_ready = ready;
            acted = true;
        }
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
    next.hint_layout = s.hint_layout.clone();
    next.version += 1;
    let (next, warnings) = config::validate(next);
    // The parent engine owns label memory; this process's snapshot must not
    // roll it back. Saving then posts the reserved reload keycode so the
    // engine applies the new config live.
    let next = s.store.with_disk_labels(next);
    match s.store.save(next) {
        Ok(()) => {
            mojito_macos::synth::post_raw(mojito_core::CONTROL_RELOAD.0, true);
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
    if is_pinned(s, &app.bundle_id) {
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
    close_catalog(s);
}

fn catalog_matches(s: &SettingsState) -> Vec<&CatalogApp> {
    match &s.catalog {
        CatalogState::Ready(list) => catalog::search(list, &s.query.text),
        CatalogState::Idle | CatalogState::Loading => Vec::new(),
    }
}

fn add_top_match(s: &mut SettingsState) {
    if let Some(app) = catalog_matches(s).first().map(|app| (*app).clone()) {
        add_app(s, &app);
    }
}

fn is_pinned(s: &SettingsState, bundle_id: &str) -> bool {
    s.rows.iter().any(|r| r.bundle_id == bundle_id)
}

pub(super) fn toggle_catalog(s: &mut SettingsState) {
    if s.catalog_open {
        close_catalog(s);
    } else {
        open_catalog(s);
    }
}

fn open_catalog(s: &mut SettingsState) {
    s.catalog_open = true;
    s.capturing = None;
    s.query = TextEdit::new("");
    s.query.focused = true;
    if matches!(s.catalog, CatalogState::Idle) {
        s.catalog = CatalogState::Loading;
        let (tx, rx) = mpsc::channel();
        s.catalog_rx = Some(rx);
        catalog::scan(tx);
    }
}

fn close_catalog(s: &mut SettingsState) {
    s.catalog_open = false;
    s.query = TextEdit::new("");
}

fn catalog_key(s: &mut SettingsState, ev: &KeyEvent, _: &mut Tx) -> bool {
    if ev.key == Key::Escape {
        close_catalog(s);
        return true;
    }
    false
}

/// Apply the picked hint layout now — a main-thread TIS call — and save it
/// through the normal path, so the next overlay reveal spells hints in it.
fn choose_layout(s: &mut SettingsState, id: Option<String>, tx: &mut Tx) {
    s.hint_menu_open = false;
    s.hint_layout = id.clone();
    // Apply locally so this window's own badges render in the chosen
    // layout; the overlay process learns of the pick through the saved
    // config and the reload signal.
    layout::set_display_layout(id);
    save(s, tx);
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
                        text: "That key is reserved (meta, esc, return, e)".to_string(),
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
