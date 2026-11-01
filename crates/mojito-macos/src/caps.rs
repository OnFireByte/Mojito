//! hidutil Caps->F18 remap: idempotent install, LaunchAgent persistence,
//! self-heal, clean uninstall that preserves foreign UserKeyMapping entries.

use mojito_core::{CAPS_USAGE, F18_USAGE};

pub struct CapsMap;

/// One UserKeyMapping entry as hidutil reports it.
pub struct MappingEntry {
    pub src: u32,
    pub dst: u32,
}

impl CapsMap {
    /// Merge our entry into the existing UserKeyMapping and set it. Idempotent.
    pub fn install(&self) -> Result<(), String> {
        todo!("caps::install ({} -> {})", CAPS_USAGE.0, F18_USAGE.0)
    }

    /// Remove only our entry (matched on Src == CAPS_USAGE), bootout the
    /// LaunchAgent, delete the plist.
    pub fn uninstall(&self) -> Result<(), String> {
        todo!("caps::uninstall")
    }

    /// True when the current UserKeyMapping contains our caps->F18 pair.
    pub fn is_active(&self) -> bool {
        todo!("caps::is_active")
    }

    /// Write and bootstrap the RunAtLoad LaunchAgent so the mapping survives
    /// reboot; Sequoia may require one Login-Items consent click.
    pub fn ensure_agent(&self) -> Result<(), String> {
        todo!("caps::ensure_agent")
    }
}
