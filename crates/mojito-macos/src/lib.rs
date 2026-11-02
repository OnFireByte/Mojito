//! All macOS contact lives here. No rune types, no engine logic; the engine
//! consumes RawKey and emits Actions, this crate produces and applies them.

pub mod apps;
mod cf;
pub mod keyboard;
pub mod shell;

use std::sync::{Mutex, MutexGuard};

use objc2::MainThreadMarker;

fn abort_on_panic<R>(callback: impl FnOnce() -> R) -> R {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback))
        .unwrap_or_else(|_| std::process::abort())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn main_thread(what: &str) -> Option<MainThreadMarker> {
    let mtm = MainThreadMarker::new();
    if mtm.is_none() {
        eprintln!("mojito-macos: {what} requires the main thread");
    }
    mtm
}
