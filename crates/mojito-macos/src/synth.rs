//! Marked key synthesis: CGEventCreateKeyboardEvent + CGEventSetFlags +
//! EventSourceUserData marker + CGEventPost(kCGHIDEventTap).
//!
//! The marker makes the tap ignore its own output. No chord-breaker posting:
//! F18 is not a modifier, so no system modifier state is ever held.

use std::ffi::c_void;
use std::ptr;

use mojito_core::{PhysicalKey, SynthKey};

/// Marker carried on every synthesized event so the tap never intercepts
/// its own output ("MOJ1T").
pub const SYNTH_MARKER: i64 = 0x004D_4F4A_3154;
/// `kCGKeyboardEventIntegerFieldEventSourceUserData`.
pub const SOURCE_USER_DATA_FIELD: u32 = 42;
/// `kCGHIDEventTap`: events posted here cross every downstream tap and app.
const HID_EVENT_TAP: u32 = 0;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
    fn CFRelease(value: *const c_void);
}

/// Post one marked key event at the HID tap. Cheap enough to call inline
/// from the tap callback.
pub fn post(key: &SynthKey) {
    // SAFETY: Null selects the default event source; the owned event is
    // released below and is valid for every call in between.
    unsafe {
        let event = CGEventCreateKeyboardEvent(ptr::null_mut(), keycode_of(key.key), key.down);
        if event.is_null() {
            return;
        }
        CGEventSetFlags(event, key.flags.0);
        CGEventSetIntegerValueField(event, SOURCE_USER_DATA_FIELD, SYNTH_MARKER);
        CGEventPost(HID_EVENT_TAP, event);
        CFRelease(event);
    }
}

/// Post one key event without the marker. Debug only: the running tap will
/// interpret it like a physical key.
pub fn post_raw(code: u16, down: bool) {
    // SAFETY: as `post`; unmarked so the tap processes it as raw input.
    unsafe {
        let event = CGEventCreateKeyboardEvent(ptr::null_mut(), code, down);
        if event.is_null() {
            return;
        }
        CGEventPost(HID_EVENT_TAP, event);
        CFRelease(event);
    }
}

/// The click-to-cancel control path. Posts an UNMARKED keyDown on the
/// reserved control keycode so our own tap sees it (marked events are
/// ignored there), turns it into a cancel, and swallows it before any app
/// can. Callable from any thread; the tap thread is not required.
pub fn post_control() {
    // SAFETY: as `post`; this event deliberately carries no marker.
    unsafe {
        let event =
            CGEventCreateKeyboardEvent(ptr::null_mut(), mojito_core::CONTROL.0, true);
        if event.is_null() {
            return;
        }
        CGEventPost(HID_EVENT_TAP, event);
        CFRelease(event);
    }
}

/// Marker check for the tap callback.
///
/// # Safety
/// `event` must be a live CGEvent the caller owns a reference to (as every
/// tap callback does).
pub unsafe fn is_marked(event: *mut c_void) -> bool {
    if event.is_null() {
        return false;
    }
    // SAFETY: CoreGraphics supplied a valid event to the callback.
    unsafe { CGEventGetIntegerValueField(event, SOURCE_USER_DATA_FIELD) == SYNTH_MARKER }
}

/// Virtual keycode for a physical key on the current layout snapshot; only
/// needed for keys whose CG keycode differs from position (none today).
pub fn keycode_of(key: PhysicalKey) -> u16 {
    key.0
}
