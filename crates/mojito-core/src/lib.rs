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

pub use bridge::{AppIndex, AppRef, Job, SwitchRequest, UiMsg};
pub use config::{Config, PinnedApp};
pub use engine::{Action, Engine, Mono, RawKey, SynthKey};
pub use key::{
    ARROW_DOWN, ARROW_LEFT, ARROW_RIGHT, ARROW_UP, CAPS_USAGE, CONTROL, CONTROL_RELOAD, F18_USAGE,
    ModifierFlags, PhysicalKey,
};
pub use store::ConfigStore;
