//! All macOS contact lives here. No rune types, no engine logic; the engine
//! consumes RawKey and emits Actions, this crate produces and applies them.

mod ax;
pub mod caps;
pub mod cursor;
pub mod diagnostics;
pub mod icons;
pub mod index;
mod input_source;
pub mod layout;
pub mod login;
pub mod permission;
pub mod status_item;
pub mod switch;
pub mod synth;
pub mod taps;
pub mod tis;

fn abort_on_panic<R>(callback: impl FnOnce() -> R) -> R {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback))
        .unwrap_or_else(|_| std::process::abort())
}
