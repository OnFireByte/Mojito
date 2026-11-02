//! The key engine: one pure state machine that turns raw key events into
//! decisions and actions. Runs inline in the tap callback; never does I/O and
//! never blocks.

use crate::bridge::{AppRef, Job, LiveModel, LiveRow, PinnedRow, SwitchRequest, UiMsg};
use crate::config::Config;
use crate::key::{ESC, F18, ModifierFlags, PhysicalKey};
use crate::labels::LabelMemory;
use crate::tables::{Chord, Tables, build_tables};
use arc_swap::ArcSwap;
use std::sync::Arc;

/// Monotonic nanoseconds, read by the backend at callback entry and injected
/// here so the machine stays pure and time-travel-testable.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Mono(pub u64);

impl Mono {
    pub fn ms_since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0) / 1_000_000
    }
}

/// What the tap observed, pre-decision.
#[derive(Clone, Copy, Debug)]
pub struct RawKey {
    pub key: PhysicalKey,
    pub down: bool,
    pub repeat: bool,
    pub flags: ModifierFlags,
    pub t: Mono,
}

/// A synthesized key event. Down and up are explicit; `flags` carries the
/// triggering event's real modifier flags so meta+cmd+shift+h yields
/// cmd+shift+Left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SynthKey {
    pub key: PhysicalKey,
    pub down: bool,
    pub flags: ModifierFlags,
}

/// The engine's whole output. Pass/Swallow decide the event's fate; the rest
/// execute. Invariant: `on_event` never does I/O and never blocks.
#[derive(Debug, PartialEq)]
pub enum Action {
    Pass,
    Swallow,
    Synth(SynthKey),
    Ui(UiMsg),
    Job(Job),
}

/// One F18 press lifecycle. Meta engages eagerly on F18 down; Esc is decided
/// at release; no timers exist anywhere in the engine.
#[derive(Debug)]
enum Phase {
    Idle,
    Armed {
        down_at: Mono,
        used: bool,
    },
    /// Meta layer active, overlay hidden: bound chords route, unbound keys
    /// stay silent. `committed` makes the rest of the hold inert after a
    /// switch so leftover autorepeats never type into the app we landed on.
    Held {
        committed: bool,
    },
    /// The switcher is visible and survives meta release: while it shows,
    /// every bare key is ours. F18 may be up or down under this phase.
    /// `rearm` is a second meta press over the open overlay, holding whether
    /// a key was used under it: a bare release dismisses the overlay, a used
    /// one keeps it.
    Overlay {
        overlay: OverlayPhase,
        committed: bool,
        rearm: Option<bool>,
    },
}

#[derive(Debug)]
enum OverlayPhase {
    Pinned,
    Live(Box<LiveModel>),
}

/// Per-hold decision memory for each physical key: down decides, up obeys.
/// `Synth` records which key a synthesized down opened, so its up (and its
/// autorepeats) close the same key even after release, reset of the overlay,
/// or a mode toggle in between.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Undecided,
    Pass,
    Swallow,
    Synth(PhysicalKey),
}

/// The layer a key decision happens in: which table (or frozen label list)
/// answers "what does this key mean right now".
enum Layer {
    Hidden,
    Pinned,
    Live(Arc<[LiveRow]>),
}

/// Flags that turn an unbound key into a system chord: such keys pass through
/// a hold untouched (cmd+tab survives meta).
const CHORD: ModifierFlags =
    ModifierFlags(ModifierFlags::CMD.0 | ModifierFlags::CTRL.0 | ModifierFlags::OPT.0);

pub struct Engine {
    phase: Phase,
    /// Whether F18 is physically down right now, independent of the phase:
    /// the overlay survives its release, and commit/cancel land differently
    /// depending on whether meta is still held.
    meta_down: bool,
    config: Arc<ArcSwap<Config>>,
    tables: Tables,
    tables_version: u64,
    index: Arc<ArcSwap<crate::bridge::AppIndex>>,
    route: [Route; 256],
    /// The previous event's identity and whether it passed. Three filter
    /// taps (HID head, session, annotated session) each see one hardware
    /// event; a duplicate identity must not act twice.
    last: Option<(PhysicalKey, bool, Mono, bool)>,
    memory: LabelMemory,
}

impl Engine {
    /// Build the engine. `config` and `index` are shared lock-free snapshots;
    /// the engine's output crosses to the UI and main thread only inside the
    /// returned `Vec<Action>`, which the tap callback routes.
    pub fn new(config: Arc<ArcSwap<Config>>, index: Arc<ArcSwap<crate::bridge::AppIndex>>) -> Self {
        let config_snapshot = config.load_full();
        let tables = build_tables(&config_snapshot);
        let tables_version = config_snapshot.version;
        let memory = config_snapshot.label_memory.clone();
        Engine {
            phase: Phase::Idle,
            meta_down: false,
            config,
            tables,
            tables_version,
            index,
            route: [Route::Undecided; 256],
            last: None,
            memory,
        }
    }

    /// The single decision point. Invariants:
    /// - F18 is always swallowed; real caps behavior can never leak (the
    ///   hidutil remap guarantees macOS never sees a Caps Lock event).
    /// - Any key event during the hold, including passed modifier chords,
    ///   marks the press used, which kills the Esc-on-release.
    /// - Synthesized keys (h/j/k/l, i) carry every modifier, so meta+cmd+h
    ///   is cmd+Left.
    /// - Other keys with CMD/CTRL/OPT flags pass through untouched; their
    ///   down route is remembered so the up passes too.
    /// - Autorepeat of a routed key re-synthesizes.
    /// - Any reset returns to Idle, clears routes, and hides the overlay.
    /// - An event identical to the previous one (same key, direction, and
    ///   timestamp) repeats the previous decision with no side effects: the
    ///   three filter taps each see one hardware event, and only the first
    ///   delivery may act.
    pub fn on_event(&mut self, ev: RawKey) -> Vec<Action> {
        self.refresh_tables();
        if let Some((key, down, t, passed)) = self.last
            && key == ev.key
            && down == ev.down
            && t == ev.t
        {
            // The other tap already decided this exact event.
            return if passed { vec![Action::Pass] } else { vec![Action::Swallow] };
        }
        let actions = self.decide(ev);
        let passed = !matches!(actions.first(), Some(Action::Swallow));
        self.last = Some((ev.key, ev.down, ev.t, passed));
        actions
    }

    /// The fresh-event decision: dispatch on F18 and the routable key space.
    fn decide(&mut self, ev: RawKey) -> Vec<Action> {
        if ev.key == F18 {
            return self.on_f18(ev);
        }
        let ix = ev.key.0 as usize;
        if ix >= self.route.len() {
            return vec![Action::Pass]; // outside the routable key space: stay transparent
        }
        let actions = if ev.down && !ev.repeat {
            match &self.phase {
                Phase::Idle => {
                    self.route[ix] = Route::Pass;
                    vec![Action::Pass]
                }
                _ => self.decide_down(ev, ix),
            }
        } else if ev.down {
            self.obey_repeat(ev, ix)
        } else {
            self.decide_up(ev, ix)
        };
        match &mut self.phase {
            Phase::Armed { used, .. }
            | Phase::Overlay {
                rearm: Some(used), ..
            } => *used = true,
            _ => {}
        }
        actions
    }

    /// Health-timer and wake-from-sleep recovery: back to Idle, overlay hidden.
    pub fn reset(&mut self) -> Vec<Action> {
        self.phase = Phase::Idle;
        self.meta_down = false;
        self.route = [Route::Undecided; 256];
        self.last = None;
        vec![Action::Ui(UiMsg::Hide)]
    }

    /// The switcher's click-to-cancel control path. The UI cannot reach the
    /// engine directly, so it posts the reserved control keycode and the tap
    /// calls this. No-op unless an uncommitted overlay is open.
    pub fn cancel_overlay(&mut self) -> Vec<Action> {
        if matches!(self.phase, Phase::Overlay { committed: false, .. }) {
            self.cancel_state()
        } else {
            vec![]
        }
    }

    /// Rebuild binding tables when the config version changes; called by
    /// `on_event` lazily so settings edits apply on the next gesture.
    fn refresh_tables(&mut self) {
        let config = self.config.load_full();
        if config.version != self.tables_version {
            self.tables = build_tables(&config);
            self.tables_version = config.version;
        }
    }

    fn on_f18(&mut self, ev: RawKey) -> Vec<Action> {
        if ev.down {
            match &mut self.phase {
                // Re-arm on a second down: tolerates duplicate evidence from
                // the taps and gestures whose up a hotkey consumed.
                Phase::Idle | Phase::Armed { .. } => {
                    self.phase = Phase::Armed {
                        down_at: ev.t,
                        used: false,
                    };
                }
                // A down during an active hold is duplicate evidence only.
                Phase::Held { .. } => {}
                // A second meta press over the open overlay arms the
                // bare-release dismissal.
                Phase::Overlay { rearm, .. } => {
                    rearm.get_or_insert(false);
                }
            }
            self.meta_down = true;
            return vec![Action::Swallow];
        }
        self.meta_down = false;
        let mut actions = vec![Action::Swallow];
        let mut dismiss = false;
        match &self.phase {
            Phase::Idle => {}
            Phase::Armed { down_at, used } => {
                let tap_max = self.config.load_full().tap_max_ms as u64;
                if !*used && ev.t.ms_since(*down_at) <= tap_max {
                    actions.push(Action::Synth(SynthKey {
                        key: ESC,
                        down: true,
                        flags: ModifierFlags::NONE,
                    }));
                    actions.push(Action::Synth(SynthKey {
                        key: ESC,
                        down: false,
                        flags: ModifierFlags::NONE,
                    }));
                }
            }
            Phase::Held { .. } => {}
            // A bare release of the second meta press dismisses the sticky
            // overlay; a release after using keys keeps it open.
            Phase::Overlay { rearm: Some(used), .. } => dismiss = !*used,
            Phase::Overlay { rearm: None, .. } => {}
        }
        if dismiss {
            actions.extend(self.cancel_state()); // HideFade; meta is up -> Idle
        } else if !matches!(self.phase, Phase::Overlay { .. }) {
            self.phase = Phase::Idle;
        }
        actions
    }

    /// Which layer answers key decisions right now.
    fn layer(&self) -> Layer {
        match &self.phase {
            Phase::Armed { .. } | Phase::Held { .. } | Phase::Idle => Layer::Hidden,
            Phase::Overlay { overlay, .. } => match overlay {
                OverlayPhase::Pinned => Layer::Pinned,
                OverlayPhase::Live(model) => Layer::Live(model.rows.clone()),
            },
        }
    }

    fn synthesizes(&self, ix: usize) -> bool {
        matches!(
            self.phase,
            Phase::Armed { .. } | Phase::Held { committed: false }
        ) && matches!(self.tables.hidden.get(ix), Some(Some(Chord::Synth(_))))
    }

    /// A committed hold is inert until F18 releases.
    fn committed(&self) -> bool {
        match &self.phase {
            Phase::Held { committed } | Phase::Overlay { committed, .. } => *committed,
            _ => false,
        }
    }

    /// A fresh (non-repeat) down during a hold. Chords pass and mark used;
    /// everything else routes through the layer's table, and a live label
    /// outranks the pinned table.
    fn decide_down(&mut self, ev: RawKey, ix: usize) -> Vec<Action> {
        if ev.flags.intersects(CHORD) && !self.synthesizes(ix) {
            self.route[ix] = Route::Pass;
            return vec![Action::Pass];
        }
        self.route[ix] = Route::Swallow;
        if self.committed() {
            return vec![Action::Swallow];
        }
        let layer = self.layer();
        if let Layer::Live(rows) = &layer
            && let Some(row) = rows.iter().find(|row| row.label == Some(ev.key))
        {
            return self.commit(AppRef::Pid(row.pid));
        }
        let table = match layer {
            Layer::Hidden => &self.tables.hidden,
            Layer::Pinned | Layer::Live(_) => &self.tables.pinned,
        };
        match (table.get(ix).copied().flatten(), layer) {
            (Some(Chord::Synth(key)), _) => {
                self.route[ix] = Route::Synth(key);
                self.phase = Phase::Held { committed: false };
                vec![
                    Action::Swallow,
                    Action::Synth(SynthKey {
                        key,
                        down: true,
                        flags: ev.flags.keep_modifiers(),
                    }),
                ]
            }
            (Some(Chord::ToggleInput), _) => {
                self.phase = Phase::Held { committed: false };
                vec![Action::Swallow, Action::Job(Job::ToggleInputSource)]
            }
            (Some(Chord::OpenPinned), _) | (Some(Chord::ToggleMode), Layer::Live(_)) => {
                self.enter_pinned()
            }
            (Some(Chord::ToggleMode), Layer::Pinned) => self.enter_live(),
            (Some(Chord::TriggerPin(pin)), Layer::Pinned) => {
                let config = self.config.load_full();
                match config.pinned.get(pin as usize) {
                    Some(app) => self.commit(AppRef::Bundle(app.bundle_id.clone())),
                    None => vec![Action::Swallow],
                }
            }
            (Some(Chord::Cancel), _) => {
                let mut actions = self.cancel_state();
                actions.insert(0, Action::Swallow);
                actions
            }
            _ => vec![Action::Swallow],
        }
    }

    /// An autorepeat follows its down's decision, whatever the phase now is.
    fn obey_repeat(&mut self, ev: RawKey, ix: usize) -> Vec<Action> {
        match self.route[ix] {
            Route::Pass => vec![Action::Pass],
            Route::Synth(key) if !self.committed() => vec![
                Action::Swallow,
                Action::Synth(SynthKey {
                    key,
                    down: true,
                    flags: ev.flags.keep_modifiers(),
                }),
            ],
            Route::Swallow | Route::Synth(_) => vec![Action::Swallow],
            Route::Undecided => match &self.phase {
                Phase::Idle => {
                    self.route[ix] = Route::Pass;
                    vec![Action::Pass]
                }
                _ => {
                    self.route[ix] = Route::Swallow;
                    vec![Action::Swallow]
                }
            },
        }
    }

    /// An up obeys its down's decision: passed keys pass, synthesized keys
    /// close with a matching up, swallowed keys stay silent.
    fn decide_up(&mut self, ev: RawKey, ix: usize) -> Vec<Action> {
        let route = self.route[ix];
        self.route[ix] = Route::Undecided;
        match route {
            Route::Pass => vec![Action::Pass],
            Route::Synth(key) if !self.committed() => vec![
                Action::Swallow,
                Action::Synth(SynthKey {
                    key,
                    down: false,
                    flags: ev.flags.keep_modifiers(),
                }),
            ],
            Route::Swallow | Route::Synth(_) => vec![Action::Swallow],
            Route::Undecided => match &self.phase {
                Phase::Idle => vec![Action::Pass],
                _ => vec![Action::Swallow],
            },
        }
    }

    /// Where commit and cancel land depends on whether meta is still held:
    /// a held F18 keeps the chord layer alive (inert when committed), a
    /// released F18 returns the machine to transparent Idle.
    fn land(&mut self, committed: bool) {
        self.phase = if self.meta_down {
            Phase::Held { committed }
        } else {
            Phase::Idle
        };
    }

    fn cancel_state(&mut self) -> Vec<Action> {
        self.land(false);
        vec![Action::Ui(UiMsg::HideFade)]
    }

    /// Commit = hide instantly, switch on the main thread, and make the rest
    /// of the hold inert so leftover autorepeats never type into the app we
    /// just landed on.
    fn commit(&mut self, app: AppRef) -> Vec<Action> {
        self.land(true);
        vec![
            Action::Swallow,
            Action::Ui(UiMsg::Hide),
            Action::Job(Job::Switch(SwitchRequest { app })),
        ]
    }

    /// Open the switcher on user-pinned apps; an empty pin list boots
    /// straight to the live list (an empty overlay is a dead end).
    fn enter_pinned(&mut self) -> Vec<Action> {
        let config = self.config.load_full();
        if config.pinned.is_empty() {
            let mut actions = self.enter_live();
            actions.push(Action::Job(Job::RefreshWindowCounts));
            return actions;
        }
        let rows: Vec<PinnedRow> = config
            .pinned
            .iter()
            .map(|app| PinnedRow {
                key: app.key,
                name: app.name.clone(),
                glyph: crate::key::glyph(app.key),
                bundle_id: app.bundle_id.clone(),
            })
            .collect();
        self.phase = Phase::Overlay {
            overlay: OverlayPhase::Pinned,
            committed: false,
            rearm: None,
        };
        vec![
            Action::Swallow,
            Action::Ui(UiMsg::ShowPinned { rows: rows.into() }),
            Action::Job(Job::RefreshWindowCounts),
        ]
    }

    /// Freeze a live model from the current app index and show it. Labels
    /// (and the sticky memory they update) are computed once here and never
    /// mutate during the hold.
    fn enter_live(&mut self) -> Vec<Action> {
        let index = self.index.load_full();
        let slots: Vec<crate::bridge::AppSlot> = index
            .slots
            .iter()
            .filter(|slot| !slot.is_frontmost && slot.windows > 0)
            .cloned()
            .collect();
        let pinned: Vec<PhysicalKey> = self
            .config
            .load_full()
            .pinned
            .iter()
            .map(|app| app.key)
            .collect();
        let labels = crate::labels::assign_labels(&slots, &self.memory, &pinned);
        let mut rows = Vec::with_capacity(slots.len());
        for (slot, label) in slots.into_iter().zip(labels) {
            if let (Some(bundle), Some(key)) = (&slot.bundle_id, label) {
                self.memory.0.insert(bundle.clone(), key);
            }
            rows.push(LiveRow {
                pid: slot.pid,
                bundle: slot.bundle_id,
                name: slot.name,
                windows: slot.windows,
                label,
            });
        }
        let rows: Arc<[LiveRow]> = Arc::from(rows);
        self.phase = Phase::Overlay {
            overlay: OverlayPhase::Live(Box::new(LiveModel { rows: rows.clone() })),
            committed: false,
            rearm: None,
        };
        vec![
            Action::Swallow,
            Action::Ui(UiMsg::ShowLive { rows }),
            Action::Job(Job::MergeLabels(self.memory.clone())),
        ]
    }
}
