//! Binding tables: the whole domain is two 128-slot arrays per phase, built
//! by one pure function. No runtime branching decides what a key means.

use crate::config::Config;
use crate::engine::SynthKey;

/// What a key does in a phase. Labels live in [`crate::bridge::LiveModel`],
/// not here; pinned trigger keys land in the pinned table.
#[derive(Clone, Copy, Debug)]
pub enum Chord {
    Synth(SynthKey),
    CycleInput,
    OpenPinned,
    ToggleMode,
    TriggerPin(u16),
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub struct Tables {
    pub hidden: [Option<Chord>; 128],
    pub pinned: [Option<Chord>; 128],
}

/// Build both tables from config. The hidden table is the const spec:
/// h/j/k/l arrows, i backspace, enter cycle, m open, everything else unbound.
/// The pinned table: user trigger keys first, then m toggle, then esc cancel.
pub fn build_tables(config: &Config) -> Tables {
    let _ = config;
    todo!("build_tables")
}
