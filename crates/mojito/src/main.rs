//! Wiring only. Decisions live in mojito-core, macOS contact in
//! mojito-macos, pixels in the ui modules. Two threads: the tap thread owns
//! the engine; this process's main thread runs rune and drains the bridges.

mod app_spec;
mod catalog;
mod keysym;
mod login;
mod ui_settings;
mod ui_switcher;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use mojito_core::geom::{self, DisplayInfo, Rect};
use mojito_core::{ConfigStore, Engine, Job, UiMsg};
use mojito_macos::caps::CapsMap;
use mojito_macos::index::AppIndexHandle;
use mojito_macos::{cursor, layout, permission, status_item, switch, taps, tis};
use rune::{
    App, AppHandle, DisplayId, DisplayTarget, OverlayHandle, OverlayPlacement, RunOptions, Tx,
    WindowSpec,
};
use ui_settings::SettingsState;
use ui_switcher::Switcher;

/// Generous bounds so the engine's `try_send` never drops in practice while
/// the tap callback still cannot block.
const UI_CHANNEL_BOUND: usize = 64;
const JOB_CHANNEL_BOUND: usize = 256;

fn main() {
    warn_unbundled();
    rune::prewarm_gpu();

    let store = Arc::new(ConfigStore::open(settings_path()));
    let index = AppIndexHandle::new();
    let (ui_tx, ui_rx) = mpsc::sync_channel::<UiMsg>(UI_CHANNEL_BOUND);
    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<Job>(JOB_CHANNEL_BOUND);
    let engine = Engine::new(
        store.handle(),
        index.handle(),
        ui_tx.clone(),
        jobs_tx.clone(),
    );
    let (menu_tx, menu_rx) = mpsc::sync_channel::<status_item::MenuAction>(8);

    rune::run_windows_with(RunOptions { accessory: true }, move |displays| {
        let displays: Vec<Display> = displays
            .iter()
            .map(|d| {
                let b = d.native_bounds();
                Display {
                    id: d.id.clone(),
                    info: DisplayInfo {
                        bounds: Rect {
                            x: b[0],
                            y: b[1],
                            w: b[2],
                            h: b[3],
                        },
                        scale: d.scale,
                    },
                }
            })
            .collect();

        let mut app = App::new(app_spec::hub_spec()).windowless().keep_running();
        let hub = app.handle();
        let switcher = app.overlay(ui_switcher::spec(), Switcher::default());
        // Click-to-cancel needs the overlay's own handle inside its state; the
        // view closures cannot capture it any other way.
        switcher.update({
            let handle = switcher.clone();
            move |s, _| s.me = Some(handle)
        });

        let settings_app = App::new(app_spec::settings_spec())
            // Deferred creation: the window opens only from the status item.
            .windowless()
            .standard_menus()
            .on_close(ui_settings::closed)
            .init({
                let store = store.clone();
                let hub = hub.clone();
                move || SettingsState::new(store, hub)
            });
        let (settings, _settings_window) = app.window(
            WindowSpec::from_app("settings", settings_app)
                .configured_options(app_spec::settings_window_options()),
        );

        index.start();
        layout::snapshot();
        sync_caps_mapping(&store);
        if let Err(error) = status_item::install(menu_tx) {
            eprintln!("mojito: status item: {error}");
        }
        if permission::state() == permission::PermissionState::NeedsGrant {
            permission::prompt();
        }
        pump_menu(menu_rx, hub.clone(), settings.clone());

        let ui_wake: Box<dyn Fn() + Send> = {
            let hub = hub.clone();
            Box::new(move || hub.wake())
        };
        let job_wake: Box<dyn Fn() + Send> = {
            let hub = hub.clone();
            Box::new(move || hub.wake())
        };
        if let Err(error) = taps::spawn(taps::TapDeps {
            engine,
            ui: ui_tx,
            jobs: jobs_tx,
            ui_wake,
            job_wake,
        }) {
            eprintln!("mojito: taps: {error}");
        }

        let app = app.init(move || Hub {
            ui_rx,
            jobs_rx,
            overlay: Some(switcher),
            store: Some(store),
            index: Some(index),
            displays,
        });
        vec![WindowSpec::from_app("hub", app)]
    });
}

/// What the main thread's pump needs to reach. Everything arrives from
/// `App::init`; `AppSpec::init`'s placeholder never runs.
struct Hub {
    ui_rx: mpsc::Receiver<UiMsg>,
    jobs_rx: mpsc::Receiver<Job>,
    overlay: Option<OverlayHandle<Switcher>>,
    store: Option<Arc<ConfigStore>>,
    index: Option<AppIndexHandle>,
    displays: Vec<Display>,
}

struct Display {
    id: Option<DisplayId>,
    info: DisplayInfo,
}

impl Default for Hub {
    fn default() -> Self {
        // Placeholder only: `App::init` replaces it. Dropped senders make
        // these receivers read disconnected, which `poll` treats as idle.
        let (_, ui_rx) = mpsc::sync_channel(1);
        let (_, jobs_rx) = mpsc::sync_channel(1);
        Self {
            ui_rx,
            jobs_rx,
            overlay: None,
            store: None,
            index: None,
            displays: Vec::new(),
        }
    }
}

/// The main-thread pump: drains both bridge channels. Runs on every wake the
/// tap thread's wakers ask for; returning false keeps an idle loop at zero
/// draws.
fn hub_poll(s: &mut Hub, _: f64, _: &mut Tx) -> bool {
    let Some(overlay) = s.overlay.as_ref() else {
        return false;
    };
    let mut acted = false;
    while let Ok(msg) = s.ui_rx.try_recv() {
        acted = true;
        match msg {
            UiMsg::ShowPinned { rows } => {
                let rows = rows
                    .iter()
                    .map(|r| ui_switcher::Row {
                        badge: Some(r.glyph),
                        name: r.name.clone(),
                        windows: None,
                    })
                    .collect();
                reveal(overlay, s, ui_switcher::Mode::Pinned, rows);
            }
            UiMsg::ShowLive { rows } => {
                let rows = rows
                    .iter()
                    .map(|r| ui_switcher::Row {
                        badge: r.label.and_then(layout::glyph),
                        name: r.name.clone(),
                        windows: Some(r.windows),
                    })
                    .collect();
                reveal(overlay, s, ui_switcher::Mode::Live, rows);
            }
            UiMsg::Hide => overlay.hide(),
            UiMsg::HideFade => overlay.fade_out(Duration::from_millis(120)),
        }
    }
    let (Some(store), Some(index)) = (s.store.as_ref(), s.index.as_ref()) else {
        return acted;
    };
    while let Ok(job) = s.jobs_rx.try_recv() {
        acted = true;
        match job {
            Job::Switch(req) => {
                switch::switch_to(&req, &mut |error| eprintln!("mojito: switch: {}", error.0))
            }
            Job::CycleInputSource => {
                if let Err(error) = tis::cycle() {
                    eprintln!("mojito: input sources: {error}");
                }
            }
            Job::RefreshWindowCounts => index.refresh_window_counts(),
            Job::MergeLabels(memory) => store.merge_labels(memory),
        }
    }
    acted
}

/// Place, populate and reveal the switcher in one batch so the first visible
/// frame already holds the rows (`tx.instant()` inside the update).
fn reveal(
    overlay: &OverlayHandle<Switcher>,
    s: &Hub,
    mode: ui_switcher::Mode,
    rows: Vec<ui_switcher::Row>,
) {
    let size = ui_switcher::window_size(rows.len());
    let (target, rect) = placement(s, size);
    if let Some(target) = target {
        overlay.set_display(target);
    }
    overlay.set_placement(Some(OverlayPlacement::Rect {
        x: rect.x as f32,
        y: rect.y as f32,
        width: size.0,
        height: size.1,
    }));
    overlay.update(move |card, tx| {
        card.show(mode, rows);
        tx.instant();
    });
    overlay.show();
}

/// Cursor display -> containing display (core math) -> centered window rect,
/// converted to that display's local points, which is the space
/// `OverlayPlacement::Rect` positions in.
fn placement(s: &Hub, size: (f32, f32)) -> (Option<DisplayTarget>, Rect) {
    if s.displays.is_empty() {
        return (
            None,
            Rect {
                x: 0.0,
                y: 0.0,
                w: f64::from(size.0),
                h: f64::from(size.1),
            },
        );
    }
    let infos: Vec<DisplayInfo> = s.displays.iter().map(|d| d.info.clone()).collect();
    let at = geom::display_containing(cursor::location(), &infos);
    let bounds = infos[at].bounds;
    let global = geom::centered_rect((f64::from(size.0), f64::from(size.1)), bounds);
    let local = Rect {
        x: global.x - bounds.x,
        y: global.y - bounds.y,
        w: global.w,
        h: global.h,
    };
    (s.displays[at].id.clone().map(DisplayTarget::Id), local)
}

/// The status item's menu actions arrive over a channel whose sender cannot
/// wake the rune loop, so a thread parked on `recv` pumps each action into an
/// `AppHandle::update`, which does. Idle cost: one parked thread.
fn pump_menu(
    rx: mpsc::Receiver<status_item::MenuAction>,
    hub: AppHandle<Hub>,
    settings: AppHandle<SettingsState>,
) {
    std::thread::Builder::new()
        .name("mojito-menu".into())
        .spawn(move || {
            while let Ok(action) = rx.recv() {
                let inner = hub.clone();
                let settings = settings.clone();
                hub.update(move |_, tx| match action {
                    status_item::MenuAction::OpenSettings => {
                        inner.set_dock_icon(true);
                        settings.open();
                        settings.update(ui_settings::opened);
                    }
                    status_item::MenuAction::ToggleLogin => login::toggle(),
                    status_item::MenuAction::RestoreCaps => {
                        if let Err(error) = CapsMap.uninstall() {
                            eprintln!("mojito: caps restore: {error}");
                        }
                        taps::stop();
                        tx.quit();
                    }
                    status_item::MenuAction::Quit => {
                        taps::stop();
                        tx.quit();
                    }
                });
            }
        })
        .expect("spawn menu pump");
}

/// Make the hidutil mapping match the capture setting and keep the
/// LaunchAgent behind it alive. Runs on the main thread at startup; the
/// settings pane and the status menu cover it afterwards.
fn sync_caps_mapping(store: &ConfigStore) {
    let capture_on = store.handle().load().capture_enabled;
    let result = if capture_on {
        let install = if CapsMap.is_active() { Ok(()) } else { CapsMap.install() };
        install.and_then(|()| CapsMap.ensure_agent())
    } else if CapsMap.is_active() {
        CapsMap.uninstall()
    } else {
        Ok(())
    };
    if let Err(error) = result {
        eprintln!("mojito: caps mapping: {error}");
    }
    if capture_on && !CapsMap.is_active() {
        status_item::set_needs_attention(true);
    }
}

/// `~/Library/Application Support/Mojito/settings.json`, the architecture's
/// one settings file.
fn settings_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Mojito/settings.json"))
        .unwrap_or_else(|| PathBuf::from("settings.json"))
}

/// A bare cargo-run binary has no bundle id, so the TCC grant attaches to
/// nothing and silently vanishes on rebuild. The .app from
/// scripts/build-app.sh is the real shape; warn once.
fn warn_unbundled() {
    let bundled =
        std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/"));
    if !bundled {
        eprintln!(
            "mojito: running outside Mojito.app; macOS permissions attach to no bundle id. \
             Build with scripts/build-app.sh first."
        );
    }
}
