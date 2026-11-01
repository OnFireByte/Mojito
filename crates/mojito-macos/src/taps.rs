//! Three event taps on one CFRunLoop thread, plus the health timer.
//! Session tap: filtering, keyDown|keyUp only, flagsChanged never filtered.
//! HID tap: listen-only keyDown, evidence when a Carbon hotkey consumes F18.
//! Callbacks never block: pure engine inline, synth posted inline, Ui/Job
//! over channels with a waker poke.

use mojito_core::{Action, Engine, RawKey};

pub struct TapDeps {
    pub engine: Engine,
    pub ui_wake: Box<dyn Fn() + Send>,
    pub job_wake: Box<dyn Fn() + Send>,
}

/// Spawn the tap thread; readiness is signaled once the run loop is live.
/// Returns an error string on failure so the permission flow can explain it.
pub fn spawn(deps: TapDeps) -> Result<(), String> {
    let _ = deps;
    todo!("taps::spawn")
}

/// The filtering callback. Marker check first, then RawKey, then the engine.
unsafe extern "C" fn on_key_event(
    proxy: *mut std::ffi::c_void,
    event_type: u32,
    event: *mut std::ffi::c_void,
    ctx: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    let _ = (proxy, event_type, event, ctx);
    todo!("on_key_event")
}

/// Backend conversion: a CGEvent into the engine's raw input.
fn raw_from(event: *mut std::ffi::c_void, event_type: u32) -> RawKey {
    let _ = (event, event_type);
    todo!("raw_from")
}

/// Execute one action on the tap thread (synth inline, Ui/Job over channels).
fn apply(action: &Action) {
    let _ = action;
    todo!("apply")
}
