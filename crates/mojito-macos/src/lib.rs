//! All macOS contact lives here. No rune types, no engine logic; the engine
//! consumes RawKey and emits Actions, this crate produces and applies them.

pub mod caps;
pub mod cursor;
pub mod icons;
pub mod index;
pub mod layout;
pub mod permission;
pub mod status_item;
pub mod switch;
pub mod synth;
pub mod taps;
pub mod tis;
