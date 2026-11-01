//! Pure domain core: key identities, the key engine, binding tables, label
//! assignment, config model, overlay placement math. No macOS, no rune, no
//! I/O except the settings store. Everything here is testable on any OS.

pub mod bridge;
pub mod config;
pub mod engine;
pub mod geom;
pub mod key;
pub mod labels;
pub mod store;
pub mod tables;

pub use bridge::{AppIndex, AppRef, Job, LiveRow, PinnedRow, SwitchRequest, UiMsg};
pub use config::{Config, PinnedApp};
pub use engine::{Action, Engine, Mono, RawKey, SynthKey};
pub use key::{CAPS, CAPS_USAGE, F18, F18_USAGE, HidUsage, ModifierFlags, PhysicalKey};
pub use labels::{LABEL_PRIORITY, LabelMemory, RESERVED, assign_labels};
pub use store::ConfigStore;
