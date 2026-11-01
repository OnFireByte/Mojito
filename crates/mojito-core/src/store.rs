//! Settings persistence: one JSON file, durable writes, ArcSwap hot reload.
//! The settings window is the only writer; the engine reads per event.

use crate::config::Config;
use arc_swap::ArcSwap;
use std::path::PathBuf;
use std::sync::Arc;

pub struct ConfigStore {
    path: PathBuf,
    current: Arc<ArcSwap<Config>>,
}

impl ConfigStore {
    /// Load from the default location; a missing or unreadable file loads
    /// defaults and leaves the original untouched. An unreadable file is
    /// preserved as `settings.json.unreadable-<sha256>` before any save
    /// overwrites it.
    pub fn open(path: PathBuf) -> Self {
        let _ = path;
        todo!("ConfigStore::open")
    }

    pub fn handle(&self) -> Arc<ArcSwap<Config>> {
        self.current.clone()
    }

    /// Durable write (fsync file and directory, one mutex), then swap the
    /// Arc so the engine sees the new config on its next event.
    pub fn save(&self, next: Config) -> Result<(), std::io::Error> {
        let _ = next;
        todo!("ConfigStore::save")
    }

    /// Debounced merge of sticky label memory after a Live activation.
    pub fn merge_labels(&self, patch: crate::labels::LabelMemory) {
        let _ = patch;
        todo!("ConfigStore::merge_labels")
    }
}
