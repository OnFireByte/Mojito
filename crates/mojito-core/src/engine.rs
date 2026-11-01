//! The key engine: one pure state machine that turns raw key events into
//! decisions and actions. Runs inline in the tap callback; never does I/O and
//! never blocks.

use crate::bridge::{Job, UiMsg};
use crate::key::{PhysicalKey, ModifierFlags, ARROW_DOWN, ARROW_LEFT, ARROW_RIGHT, ARROW_UP, BACKSPACE, ESC, F18, RETURN};
use arc_swap::ArcSwap;
use std::sync::Arc;

use crate::config::Config;
use crate::labels::LabelMemory;
use crate::tables::Tables;

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
/// triggering event's real modifier flags so meta+shift+h yields shift+Left.
#[derive(Clone, Copy, Debug)]
pub struct SynthKey {
    pub key: PhysicalKey,
    pub down: bool,
    pub flags: ModifierFlags,
}

/// The engine's whole output. Pass/Swallow decide the event's fate; the rest
/// execute. Invariant: `on_event` never does I/O and never blocks.
#[derive(Debug)]
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
    Armed { down_at: Mono, used: bool },
    /// `committed` makes the rest of the hold inert after a switch, so
    /// leftover autorepeats never type into the app we just landed on.
    Meta { overlay: OverlayPhase, committed: bool },
}

#[derive(Debug)]
enum OverlayPhase {
    Hidden,
    Pinned,
    Live(Box<crate::bridge::LiveModel>),
}

/// Per-hold decision memory for each physical key: down decides, up obeys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Undecided,
    Pass,
    Swallow,
}

pub struct Engine {
    phase: Phase,
    config: Arc<ArcSwap<Config>>,
    tables: Tables,
    index: Arc<ArcSwap<crate::bridge::AppIndex>>,
    route: [Route; 256],
    memory: LabelMemory,
    ui: std::sync::mpsc::SyncSender<UiMsg>,
    jobs: std::sync::mpsc::SyncSender<Job>,
}

impl Engine {
    /// Build the engine. `config` and `index` are shared lock-free snapshots;
    /// the senders are the only crossings to the UI and main thread.
    pub fn new(
        config: Arc<ArcSwap<Config>>,
        index: Arc<ArcSwap<crate::bridge::AppIndex>>,
        ui: std::sync::mpsc::SyncSender<UiMsg>,
        jobs: std::sync::mpsc::SyncSender<Job>,
    ) -> Self {
        let _ = (config, index, ui, jobs);
        todo!("Engine::new")
    }

    /// The single decision point. Invariants:
    /// - F18 is always swallowed; real caps behavior can never leak (the
    ///   hidutil remap guarantees macOS never sees a Caps Lock event).
    /// - Any key event during the hold, including passed modifier chords,
    ///   marks the press used, which kills the Esc-on-release.
    /// - Keys with CMD/CTRL/OPT flags pass through untouched; their down
    ///   route is remembered so the up passes too.
    /// - SHIFT-only chords are honored on routed keys.
    /// - Autorepeat of a routed key re-synthesizes.
    /// - Any reset returns to Idle, clears routes, and hides the overlay.
    pub fn on_event(&mut self, ev: RawKey) -> Vec<Action> {
        let _ = (&ev, &self.phase, F18, ESC, RETURN, BACKSPACE, ARROW_LEFT, ARROW_RIGHT, ARROW_DOWN, ARROW_UP);
        todo!("engine state machine")
    }

    /// Health-timer and wake-from-sleep recovery: back to Idle, overlay hidden.
    pub fn reset(&mut self) -> Vec<Action> {
        todo!("reset")
    }

    /// Rebuild binding tables when the config version changes; called by
    /// `on_event` lazily so settings edits apply on the next gesture.
    fn refresh_tables(&mut self) {
        todo!("tables refresh")
    }
}
