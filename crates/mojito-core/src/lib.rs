//! Pure domain core: key identities, the key engine, binding tables, label
//! assignment, config model, overlay placement math. No macOS, no rune, no
//! I/O except the settings store. Everything here is testable on any OS.

pub mod bridge;
pub mod config;
pub mod engine;
pub mod geom;
pub mod key;
pub mod labels;
pub mod lifecycle;
pub mod store;
pub mod tables;

pub use bridge::{AppIndex, AppRef, Job, UiMsg};
pub use config::{Config, PinnedApp, SwitcherTrigger};
pub use engine::{Action, Engine, Mono, RawKey, SynthKey};
pub use key::{ARROW_DOWN, ARROW_LEFT, ARROW_RIGHT, ARROW_UP, ModifierFlags, PhysicalKey};
pub use store::ConfigStore;
