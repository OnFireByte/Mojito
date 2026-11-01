//! Config model and boundary parsing. Parse at the boundary, trust inside;
//! this is the only place config errors exist.

use crate::key::PhysicalKey;

#[derive(Clone, Debug)]
pub struct Config {
    pub pinned: Vec<PinnedApp>,
    pub tap_max_ms: u16,
    pub capture_enabled: bool,
    pub label_memory: crate::labels::LabelMemory,
    /// Bumped on every accepted save so the engine can rebuild tables lazily.
    pub version: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            tap_max_ms: 250,
            capture_enabled: true,
            label_memory: crate::labels::LabelMemory::default(),
            version: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PinnedApp {
    pub name: String,
    pub bundle_id: String,
    pub key: PhysicalKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning(pub String);

/// Parse settings JSON with hex's recovery rules: unknown keys ignored; an
/// invalid field defaults in memory with a warning; duplicate trigger keys
/// resolve first-wins with a warning; reserved keys rejected with a warning.
/// Never fails hard: a corrupt file yields defaults and warnings.
pub fn parse(bytes: &[u8]) -> (Config, Vec<Warning>) {
    let _ = bytes;
    todo!("config::parse")
}

/// Boundary validation for a draft from the settings UI: duplicate keys
/// first-wins, reserved keys rejected, name and bundle required.
pub fn validate(draft: Config) -> (Config, Vec<Warning>) {
    let _ = draft;
    todo!("config::validate")
}
