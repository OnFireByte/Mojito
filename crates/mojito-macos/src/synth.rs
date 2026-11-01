//! Marked key synthesis: CGEventCreateKeyboardEvent + CGEventSetFlags +
//! EventSourceUserData marker + CGEventPost(kCGHIDEventTap).

use mojito_core::{PhysicalKey, SynthKey};

/// Marker carried on every synthesized event so the tap never intercepts
/// its own output ("MOJ1T").
pub const SYNTH_MARKER: i64 = 0x004D_4F4A_3154;
pub const SOURCE_USER_DATA_FIELD: u32 = 42;

/// Post one marked key event at the HID tap. Cheap enough to call inline
/// from the tap callback.
pub fn post(key: &SynthKey) {
    let _ = key;
    todo!("synth::post")
}

/// Marker check for the tap callback.
pub fn is_marked(event: *mut std::ffi::c_void) -> bool {
    let _ = event;
    todo!("is_marked")
}

/// Virtual keycode for a physical key on the current layout snapshot; only
/// needed for keys whose CG keycode differs from position (none today).
pub fn keycode_of(key: PhysicalKey) -> u16 {
    key.0
}
