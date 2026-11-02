//! Wiring only. Decisions live in mojito-core, macOS contact in
//! mojito-macos, the settings pixels in ui_settings. Two threads: the tap
//! thread owns the engine; this process's main thread runs AppKit and drains
//! the bridges.

mod app_spec;
mod catalog;
mod keysym;
mod ui_settings;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use arc_swap::ArcSwap;
use dispatch2::DispatchQueue;
use mojito_core::{AppIndex, Config, ConfigStore, Engine, Job, UiMsg};
use mojito_macos::caps::CapsMap;
use mojito_macos::index::AppIndexHandle;
use mojito_macos::switcher::{Mode, Row, Switcher};
use mojito_macos::{layout, native_switcher, permission, status_item, switch, taps, tis};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;

/// Generous bounds so the engine's `try_send` never drops in practice while
/// the tap callback still cannot block.
const UI_CHANNEL_BOUND: usize = 64;
const JOB_CHANNEL_BOUND: usize = 256;
const ICON_PIXELS: usize = 48;

fn main() {
    // Debug verbs run headless in this same bundle so they carry the app's
    // TCC grants; everything else starts the GUI.
    let mut args = std::env::args().skip(1);
    if let Some(verb) = args.next() {
        run_verb(&verb, args);
        return;
    }
    warn_unbundled();
    if status_item::activate_existing() {
        eprintln!("mojito: already running; activated the existing instance");
        return;
    }
    let mtm = MainThreadMarker::new().expect("main runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let store = Arc::new(ConfigStore::open(settings_path()));
    let index = AppIndexHandle::new();
    let (ui_tx, ui_rx) = mpsc::sync_channel::<UiMsg>(UI_CHANNEL_BOUND);
    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<Job>(JOB_CHANNEL_BOUND);

    index.start();
    layout::snapshot();
    // Hint glyphs follow the saved display layout from the first reveal.
    layout::set_display_layout(store.handle().load().hint_layout.clone());
    sync_caps_mapping(&store);
    if let Err(error) = status_item::install(on_menu) {
        eprintln!("mojito: status item: {error}");
    }
    if permission::state() == permission::PermissionState::NeedsGrant {
        permission::prompt();
    }

    let wiring = TapWiring {
        config: store.handle(),
        index: index.handle(),
        ui_tx,
        jobs_tx,
    };
    HUB.set(Some(Hub {
        ui_rx,
        jobs_rx,
        switcher: Switcher::new(mtm),
        store: store.clone(),
        index,
        icons: HashMap::new(),
    }));
    native_switcher::restore_on_exit();
    match start_taps(wiring.clone()) {
        Ok(()) => sync_native_switcher(&store),
        Err(error) => {
            eprintln!("mojito: taps: {error}");
            native_switcher::set_enabled(true);
            // The grant can land while we run; go live the moment it does.
            arm_permission_wait(wiring);
        }
    }
    app.run();
}

fn run_verb(verb: &str, mut args: impl Iterator<Item = String>) {
    match verb {
        "switch" => match args.next() {
            Some(bundle) => mojito_macos::switch::debug_switch(&bundle),
            None => eprintln!("usage: mojito switch <bundle-id>"),
        },
        "key" => {
            let (Some(code), Some(dir)) = (args.next(), args.next()) else {
                eprintln!("usage: mojito key <hex-code> <down|up>");
                return;
            };
            let code = u16::from_str_radix(code.trim_start_matches("0x"), 16).unwrap_or(0);
            mojito_macos::synth::post_raw(code, dir == "down");
            // The window server routes a posted event through the HID taps
            // asynchronously; exiting this process immediately races the
            // delivery and can drop the event.
            std::thread::sleep(Duration::from_millis(60));
        }
        "settings" => ui_settings::run_child(),
        "windows" => mojito_macos::diagnostics::windows(),
        "sources" => {
            for source in tis::enabled() {
                println!("{}\t{}", source.id, source.name);
            }
            match tis::current_id() {
                Ok(id) => println!("current\t{id}"),
                Err(error) => eprintln!("mojito: {error}"),
            }
            for name in tis::system_history_names() {
                println!("history\t{name}");
            }
        }
        "taps" => mojito_macos::diagnostics::taps(),
        other => {
            eprintln!("unknown verb {other}; verbs: switch, key, settings, windows, sources, taps")
        }
    }
}

/// What the main-thread pump reaches: the bridge receivers and everything
/// the jobs and the switcher touch. Lives in `HUB` on the main thread.
struct Hub {
    ui_rx: mpsc::Receiver<UiMsg>,
    jobs_rx: mpsc::Receiver<Job>,
    switcher: Switcher,
    store: Arc<ConfigStore>,
    index: AppIndexHandle,
    /// App icons keyed by bundle id, rendered once per app; `None` caches a
    /// miss so the row keeps its monogram without retrying.
    icons: HashMap<String, Option<CFRetained<CGImage>>>,
}

thread_local! {
    static HUB: RefCell<Option<Hub>> = const { RefCell::new(None) };
}

impl Hub {
    fn icon_for(&mut self, bundle: &str) -> Option<CFRetained<CGImage>> {
        self.icons
            .entry(bundle.to_string())
            .or_insert_with(|| mojito_macos::icons::app_icon(bundle, ICON_PIXELS))
            .clone()
    }

    fn pump(&mut self) {
        while let Ok(msg) = self.ui_rx.try_recv() {
            match msg {
                UiMsg::ShowPinned { rows } => {
                    let rows: Vec<Row> = rows
                        .iter()
                        .map(|r| Row {
                            key: Some(r.key),
                            badge: layout::glyph(r.key).or(Some(r.glyph)),
                            name: r.name.clone(),
                            windows: None,
                            icon: self.icon_for(&r.bundle_id),
                        })
                        .collect();
                    self.switcher.show(Mode::Pinned, &rows);
                }
                UiMsg::ShowLive { rows } => {
                    let rows: Vec<Row> = rows
                        .iter()
                        .map(|r| Row {
                            key: r.label,
                            badge: r.label.and_then(layout::glyph),
                            name: r.name.clone(),
                            windows: Some(r.windows),
                            icon: r.bundle.as_deref().and_then(|b| self.icon_for(b)),
                        })
                        .collect();
                    self.switcher.show(Mode::Live, &rows);
                }
                UiMsg::Hide => self.switcher.hide(),
                UiMsg::HideFade => self.switcher.fade_out(Duration::from_millis(120)),
            }
        }
        while let Ok(job) = self.jobs_rx.try_recv() {
            match job {
                Job::Switch(req) => {
                    switch::switch_to(&req, &mut |error| eprintln!("mojito: switch: {}", error.0))
                }
                Job::ToggleInputSource => {
                    if let Err(error) = tis::toggle() {
                        eprintln!("mojito: input sources: {error}");
                    }
                }
                Job::RefreshWindowCounts => self.index.refresh_window_counts(),
                Job::MergeLabels(memory) => self.store.merge_labels(memory),
                Job::ReloadConfig => {
                    if let Err(error) = self.store.reload() {
                        eprintln!("mojito: config reload: {error}");
                    }
                    layout::set_display_layout(self.store.handle().load().hint_layout.clone());
                    sync_native_switcher(&self.store);
                }
            }
        }
    }
}

/// Wakes the main thread to drain both bridge channels. Callable from any
/// thread; the tap thread's wakers are this.
fn wake_main() {
    DispatchQueue::main().exec_async(|| {
        HUB.with_borrow_mut(|hub| {
            if let Some(hub) = hub {
                hub.pump();
            }
        })
    });
}

/// macOS's Cmd+Tab stays off only while the engine runs and the trigger
/// setting gives Cmd+Tab to Mojito. Main thread, after the taps are live.
fn sync_native_switcher(store: &ConfigStore) {
    let ours = store.handle().load().switcher_trigger.cmd_tab();
    native_switcher::set_enabled(!ours);
}

/// Status menu actions, run on the main thread by the menu item's target.
fn on_menu(action: status_item::MenuAction) {
    match action {
        status_item::MenuAction::OpenSettings => {
            // The settings window runs in its own short-lived
            // process, so its memory returns fully on close.
            if let Err(spawn_error) = std::process::Command::new(
                std::env::current_exe().unwrap_or_else(|_| "mojito".into()),
            )
            .arg("settings")
            .spawn()
            {
                eprintln!("mojito: settings process: {spawn_error}");
            }
        }
        status_item::MenuAction::ToggleLogin => mojito_macos::login::toggle(),
        status_item::MenuAction::RestoreCaps => {
            if let Err(error) = CapsMap.uninstall() {
                eprintln!("mojito: caps restore: {error}");
            }
            quit();
        }
        status_item::MenuAction::Quit => quit(),
    }
}

fn quit() {
    taps::stop();
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
}

#[derive(Clone)]
struct TapWiring {
    config: Arc<ArcSwap<Config>>,
    index: Arc<ArcSwap<AppIndex>>,
    ui_tx: mpsc::SyncSender<UiMsg>,
    jobs_tx: mpsc::SyncSender<Job>,
}

/// Build a fresh engine and spawn the tap thread. Called at startup and
/// again by the permission waiter; each attempt owns its own engine, so a
/// failed start leaves nothing behind.
fn start_taps(wiring: TapWiring) -> Result<(), String> {
    let TapWiring {
        config,
        index,
        ui_tx,
        jobs_tx,
    } = wiring;
    taps::spawn(taps::TapDeps {
        engine: Engine::new(config, index),
        ui: ui_tx,
        jobs: jobs_tx,
        ui_wake: Box::new(wake_main),
        job_wake: Box::new(wake_main),
    })
}

/// Poll for the Accessibility grant and bring the engine live the moment it
/// lands. The thread exits after one successful spawn, so it costs nothing
/// once granted.
fn arm_permission_wait(wiring: TapWiring) {
    std::thread::Builder::new()
        .name("mojito-permwait".into())
        .spawn(move || {
            let mut polls = 0u32;
            while permission::state() == permission::PermissionState::NeedsGrant {
                std::thread::sleep(Duration::from_millis(250));
                polls += 1;
                if polls == 60 {
                    // Still untrusted 15 s after prompting: the grant is
                    // likely stale from a previous ad-hoc rebuild, and macOS
                    // will not prompt again on its own. Surface it.
                    DispatchQueue::main().exec_async(|| status_item::set_needs_attention(true));
                }
            }
            match start_taps(wiring) {
                Ok(()) => DispatchQueue::main().exec_async(|| {
                    HUB.with_borrow(|hub| {
                        if let Some(hub) = hub {
                            sync_native_switcher(&hub.store);
                        }
                    })
                }),
                Err(error) => eprintln!("mojito: taps after grant: {error}"),
            }
        })
        .expect("spawn permission waiter");
}

/// Make the hidutil mapping match the capture setting and keep the
/// LaunchAgent behind it alive. Runs on the main thread at startup; the
/// settings pane and the status menu cover it afterwards.
fn sync_caps_mapping(store: &ConfigStore) {
    let capture_on = store.handle().load().capture_enabled;
    let result = if capture_on {
        let install = if CapsMap.is_active() {
            Ok(())
        } else {
            CapsMap.install()
        };
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
pub(crate) fn settings_path() -> PathBuf {
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
