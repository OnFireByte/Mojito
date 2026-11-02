//! Marked key synthesis: CGEventCreateKeyboardEvent + CGEventSetFlags +
//! EventSourceUserData marker + CGEventPost(kCGHIDEventTap).
//!
//! The marker makes the tap ignore its own output. No chord-breaker posting:
//! F18 is not a modifier, so no system modifier state is ever held.

use mojito_core::{ARROW_DOWN, ARROW_LEFT, ARROW_RIGHT, ARROW_UP, PhysicalKey, SynthKey};
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags, CGEventTapLocation};

/// Marker carried on every synthesized event so the tap never intercepts
/// its own output ("MOJ1T").
const SYNTH_MARKER: i64 = 0x004D_4F4A_3154;
const NUMERIC_PAD_FLAG: u64 = 1 << 21;
const SECONDARY_FN_FLAG: u64 = 1 << 23;

/// Post one marked key event at the HID tap. Cheap enough to call inline
/// from the tap callback.
pub fn post(key: &SynthKey) {
    let Some(event) = CGEvent::new_keyboard_event(None, key.key.0, key.down) else {
        return;
    };
    CGEvent::set_flags(
        Some(&event),
        CGEventFlags(key.flags.0 | native_flags(key.key)),
    );
    CGEvent::set_integer_value_field(
        Some(&event),
        CGEventField::EventSourceUserData,
        SYNTH_MARKER,
    );
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
}

/// Post one key event without the marker. Debug only: the running tap will
/// interpret it like a physical key.
pub fn post_raw(code: u16, down: bool) {
    post_unmarked(code, down);
}

/// The click-to-cancel control path. Posts an UNMARKED keyDown on the
/// reserved control keycode so our own tap sees it (marked events are
/// ignored there), turns it into a cancel, and swallows it before any app
/// can. Callable from any thread; the tap thread is not required.
pub fn post_control() {
    post_unmarked(mojito_core::CONTROL.0, true);
}

pub(crate) fn is_marked(event: &CGEvent) -> bool {
    CGEvent::integer_value_field(Some(event), CGEventField::EventSourceUserData) == SYNTH_MARKER
}

fn post_unmarked(code: u16, down: bool) {
    if let Some(event) = CGEvent::new_keyboard_event(None, code, down) {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    }
}

fn native_flags(key: PhysicalKey) -> u64 {
    match key {
        ARROW_LEFT | ARROW_RIGHT | ARROW_DOWN | ARROW_UP => NUMERIC_PAD_FLAG | SECONDARY_FN_FLAG,
        _ => 0,
    }
}
