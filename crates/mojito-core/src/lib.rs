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
pub use engine::{Action, Engine, RawKey, SynthKey};
pub use key::{HidUsage, ModifierFlags, PhysicalKey, F18, F18_USAGE, CAPS, CAPS_USAGE};
pub use labels::{assign_labels, LabelMemory, LABEL_PRIORITY, RESERVED};
pub use store::ConfigStore;
