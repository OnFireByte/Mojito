//! Binding tables: the whole domain is two 128-slot arrays per phase, built
//! by one pure function. No runtime branching decides what a key means.

use crate::config::Config;
use crate::key::{
    ARROW_DOWN, ARROW_LEFT, ARROW_RIGHT, ARROW_UP, BACKSPACE, ESC, PhysicalKey, RETURN, SWITCHER,
};

pub const TABLE_SLOTS: usize = 128;

/// What a key does in a phase. Labels live in [`crate::bridge::LiveModel`],
/// not here; pinned trigger keys land in the pinned table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chord {
    /// Arrow/backspace positions; the engine fills `flags` from the
    /// triggering event's shift/ctrl/opt/cmd.
    Synth(PhysicalKey),
    ToggleInput,
    OpenPinned,
    ToggleMode,
    TriggerPin(u16),
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub struct Tables {
    pub hidden: [Option<Chord>; TABLE_SLOTS],
    pub pinned: [Option<Chord>; TABLE_SLOTS],
}

/// Build both tables from config. The hidden table is the const spec:
/// h/j/k/l arrows, i backspace, enter toggles the input source, e open.
/// The pinned table: user trigger keys first, then e toggle, then esc cancel.
/// Duplicate trigger keys resolve first-wins; reserved keys never take a
/// chord even if a malformed config carries them.
pub fn build_tables(config: &Config) -> Tables {
    let mut tables = Tables {
        hidden: [None; TABLE_SLOTS],
        pinned: [None; TABLE_SLOTS],
    };
    let put = |slot: &mut [Option<Chord>; TABLE_SLOTS], key: PhysicalKey, chord: Chord| {
        if (key.0 as usize) < slot.len() {
            slot[key.0 as usize] = Some(chord);
        }
    };
    put(&mut tables.hidden, PhysicalKey(0x04), Chord::Synth(ARROW_LEFT)); // h
    put(&mut tables.hidden, PhysicalKey(0x26), Chord::Synth(ARROW_DOWN)); // j
    put(&mut tables.hidden, PhysicalKey(0x28), Chord::Synth(ARROW_UP)); // k
    put(&mut tables.hidden, PhysicalKey(0x25), Chord::Synth(ARROW_RIGHT)); // l
    put(&mut tables.hidden, PhysicalKey(0x22), Chord::Synth(BACKSPACE)); // i
    put(&mut tables.hidden, RETURN, Chord::ToggleInput);
    put(&mut tables.hidden, SWITCHER, Chord::OpenPinned);

    for (i, app) in config.pinned.iter().enumerate() {
        if crate::labels::RESERVED.contains(&app.key) {
            continue;
        }
        if matches!(tables.pinned.get(app.key.0 as usize), Some(Some(_))) {
            continue; // first-wins on duplicate trigger keys
        }
        put(&mut tables.pinned, app.key, Chord::TriggerPin(i as u16));
    }
    put(&mut tables.pinned, SWITCHER, Chord::ToggleMode);
    put(&mut tables.pinned, ESC, Chord::Cancel);
    tables
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PinnedApp;

    fn pin(key: u16, name: &str) -> PinnedApp {
        PinnedApp {
            name: name.into(),
            bundle_id: format!("com.test.{name}"),
            key: PhysicalKey(key),
        }
    }

    #[test]
    fn hidden_table_matches_the_const_spec() {
        let t = build_tables(&Config::default());
        assert_eq!(t.hidden[0x04], Some(Chord::Synth(ARROW_LEFT)));
        assert_eq!(t.hidden[0x26], Some(Chord::Synth(ARROW_DOWN)));
        assert_eq!(t.hidden[0x28], Some(Chord::Synth(ARROW_UP)));
        assert_eq!(t.hidden[0x25], Some(Chord::Synth(ARROW_RIGHT)));
        assert_eq!(t.hidden[0x22], Some(Chord::Synth(BACKSPACE)));
        assert_eq!(t.hidden[RETURN.0 as usize], Some(Chord::ToggleInput));
        assert_eq!(t.hidden[SWITCHER.0 as usize], Some(Chord::OpenPinned));
        // Everything else is unbound; the engine swallows those bare keys.
        let bound = [
            0x04,
            0x26,
            0x28,
            0x25,
            0x22,
            RETURN.0 as usize,
            SWITCHER.0 as usize,
        ];
        for (i, slot) in t.hidden.iter().enumerate() {
            if !bound.contains(&i) {
                assert!(slot.is_none(), "unexpected chord at {i:#x}");
            }
        }
    }

    #[test]
    fn pinned_table_maps_triggers_toggle_and_cancel() {
        let config = Config {
            pinned: vec![pin(0x03, "one"), pin(0x01, "two")],
            ..Default::default()
        };
        let t = build_tables(&config);
        assert_eq!(t.pinned[0x03], Some(Chord::TriggerPin(0)));
        assert_eq!(t.pinned[0x01], Some(Chord::TriggerPin(1)));
        assert_eq!(t.pinned[SWITCHER.0 as usize], Some(Chord::ToggleMode));
        assert_eq!(t.pinned[ESC.0 as usize], Some(Chord::Cancel));
        assert_eq!(t.pinned[0x04], None); // unbound bare key
    }

    #[test]
    fn duplicate_trigger_keys_resolve_first_wins() {
        let config = Config {
            pinned: vec![pin(0x03, "one"), pin(0x03, "two")],
            ..Default::default()
        };
        let t = build_tables(&config);
        assert_eq!(t.pinned[0x03], Some(Chord::TriggerPin(0)));
        // The key never routes to the second pin: one key, one pin.
        assert_ne!(t.pinned[0x03], Some(Chord::TriggerPin(1)));
    }

    #[test]
    fn reserved_keys_never_yield_chords() {
        let config = Config {
            pinned: vec![
                pin(0x4F, "f18"),
                pin(SWITCHER.0, "e"),
                pin(ESC.0, "esc"),
                pin(RETURN.0, "ret"),
                pin(0x03, "ok"),
            ],
            ..Default::default()
        };
        let t = build_tables(&config);
        assert_eq!(t.pinned[0x4F], None);
        assert_eq!(t.pinned[SWITCHER.0 as usize], Some(Chord::ToggleMode));
        assert_eq!(t.pinned[ESC.0 as usize], Some(Chord::Cancel));
        assert_eq!(t.pinned[RETURN.0 as usize], None);
        // The valid pin keeps its config index even after rejections.
        assert_eq!(t.pinned[0x03], Some(Chord::TriggerPin(4)));
    }
}
