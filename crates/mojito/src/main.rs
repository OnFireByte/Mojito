//! Wiring only. Decisions live in mojito-core, macOS contact in
//! mojito-macos, the settings pixels in ui_settings. Two threads: the tap
//! thread owns the engine; this process's main thread runs AppKit and drains
//! the bridges.

mod app_spec;
mod catalog;
mod keysym;
mod ui_settings;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use arc_swap::ArcSwap;
use mojito_core::lifecycle::{self, Lifecycle, Next, Remap};
use mojito_core::{AppIndex, Config, ConfigStore, Engine, Job, UiMsg};
use mojito_macos::apps::icons::IconCache;
use mojito_macos::apps::index::AppIndexHandle;
use mojito_macos::apps::switch;
use mojito_macos::keyboard::caps::CapsMap;
use mojito_macos::keyboard::{layout, taps, tis};
use mojito_macos::shell::app;
use mojito_macos::shell::switcher::{Mode, Row, Switcher};
use mojito_macos::shell::{native_switcher, permission, status_item};

/// Generous bounds so the engine's `try_send` never drops in practice while
/// the tap callback still cannot block.
const UI_CHANNEL_BOUND: usize = 64;
const JOB_CHANNEL_BOUND: usize = 256;

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
    app::become_accessory();

    let store = Arc::new(ConfigStore::open(settings_path()));
    let index = AppIndexHandle::default();
    let (ui_tx, ui_rx) = mpsc::sync_channel::<UiMsg>(UI_CHANNEL_BOUND);
    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<Job>(JOB_CHANNEL_BOUND);

    index.start();
    layout::snapshot();
    // Hint glyphs follow the saved display layout from the first reveal.
    layout::set_display_layout(store.handle().load().hint_layout.clone());
    if let Err(error) = status_item::install(on_menu) {
        eprintln!("mojito: status item: {error}");
    }

    let host = MacHost {
        config: store.handle(),
        index: index.handle(),
        ui_tx,
        jobs_tx,
    };
    HUB.set(Some(Hub {
        ui_rx,
        jobs_rx,
        switcher: Switcher::create(),
        store: store.clone(),
        index,
        icons: IconCache::default(),
        lifecycle: Lifecycle::default(),
        host,
    }));
    native_switcher::restore_on_exit();
    with_lifecycle(|lifecycle, host, config| lifecycle.start(host, config));
    app::run();
}

fn run_verb(verb: &str, mut args: impl Iterator<Item = String>) {
    match verb {
        "switch" => match args.next() {
            Some(bundle) => mojito_macos::apps::switch::debug_switch(&bundle),
            None => eprintln!("usage: mojito switch <bundle-id>"),
        },
        "key" => {
            let (Some(code), Some(dir)) = (args.next(), args.next()) else {
                eprintln!("usage: mojito key <hex-code> <down|up>");
                return;
            };
            let code = u16::from_str_radix(code.trim_start_matches("0x"), 16).unwrap_or(0);
            mojito_macos::keyboard::synth::post_raw(code, dir == "down");
            // The window server routes a posted event through the HID taps
            // asynchronously; exiting this process immediately races the
            // delivery and can drop the event.
            std::thread::sleep(Duration::from_millis(60));
        }
        "settings" => ui_settings::run_child(),
        "windows" => mojito_macos::shell::diagnostics::windows(),
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
        "taps" => mojito_macos::shell::diagnostics::taps(),
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
    icons: IconCache,
    lifecycle: Lifecycle,
    host: MacHost,
}

thread_local! {
    static HUB: RefCell<Option<Hub>> = const { RefCell::new(None) };
}

impl Hub {
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
                            icon: self.icons.get(&r.bundle_id),
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
                            icon: r.bundle.as_deref().and_then(|b| self.icons.get(b)),
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
                Job::Switch(req) => switch::switch_to(&req, &mut |note| {
                    if let switch::SwitchNote::Problem(problem) = note {
                        eprintln!("mojito: switch: {problem}");
                    }
                }),
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
                    let config = self.store.handle().load_full();
                    layout::set_display_layout(config.hint_layout.clone());
                    self.lifecycle.config_reloaded(&mut self.host, &config);
                }
            }
        }
    }
}

/// Wakes the main thread to drain both bridge channels. Callable from any
/// thread; the tap thread's waker is this.
fn wake_main() {
    app::on_main(|| {
        HUB.with_borrow_mut(|hub| {
            if let Some(hub) = hub {
                hub.pump();
            }
        })
    });
}

fn with_lifecycle(step: fn(&mut Lifecycle, &mut MacHost, &Config) -> Next) {
    let next = HUB.with_borrow_mut(|hub| {
        let hub = hub.as_mut()?;
        let config = hub.store.handle().load_full();
        Some(step(&mut hub.lifecycle, &mut hub.host, &config))
    });
    if let Some(Next::PollGrantAfter(delay)) = next {
        app::on_main_after(delay, || {
            with_lifecycle(|lifecycle, host, config| lifecycle.grant_poll(host, config))
        });
    }
}

/// Status menu actions, run on the main thread by the menu item's target.
fn on_menu(action: status_item::MenuAction) {
    match action {
        status_item::MenuAction::OpenSettings => {
            if let Err(spawn_error) = std::process::Command::new(
                std::env::current_exe().unwrap_or_else(|_| "mojito".into()),
            )
            .arg("settings")
            .spawn()
            {
                eprintln!("mojito: settings process: {spawn_error}");
            }
        }
        status_item::MenuAction::ToggleLogin => mojito_macos::shell::login::toggle(),
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
    app::terminate();
}

struct MacHost {
    config: Arc<ArcSwap<Config>>,
    index: Arc<ArcSwap<AppIndex>>,
    ui_tx: mpsc::SyncSender<UiMsg>,
    jobs_tx: mpsc::SyncSender<Job>,
}

impl lifecycle::Host for MacHost {
    fn remap(&self) -> &dyn Remap {
        &CapsMap
    }

    fn grant_ready(&self) -> bool {
        permission::state() == permission::PermissionState::Ready
    }

    fn prompt_for_grant(&mut self) {
        permission::prompt();
    }

    /// Each attempt owns a fresh engine, so a failed start leaves nothing behind.
    fn start_taps(&mut self) -> Result<(), String> {
        taps::spawn(taps::TapDeps {
            engine: Engine::new(self.config.clone(), self.index.clone()),
            ui: self.ui_tx.clone(),
            jobs: self.jobs_tx.clone(),
            wake: Box::new(wake_main),
        })
    }

    fn set_native_cmd_tab(&mut self, enabled: bool) {
        native_switcher::set_enabled(enabled);
    }

    fn set_needs_attention(&mut self, flag: bool) {
        status_item::set_needs_attention(flag);
    }

    fn report(&mut self, context: &str, error: &str) {
        eprintln!("mojito: {context}: {error}");
    }
}

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
