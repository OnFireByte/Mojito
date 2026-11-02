//! Marked key synthesis: CGEventCreateKeyboardEvent + CGEventSetFlags +
//! EventSourceUserData marker + CGEventPost(kCGHIDEventTap).
//!
//! The marker makes the tap ignore its own output. No chord-breaker posting:
//! F18 is not a modifier, so no system modifier state is ever held.

use mojito_core::{ARROW_DOWN, ARROW_LEFT, ARROW_RIGHT, ARROW_UP, PhysicalKey, SynthKey};
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags, CGEventTapLocation};

use super::modifiers;

/// Reserved control keycodes: unassigned on macOS keyboards (hex's
/// chord-breaker hole and its neighbor). The UI posts unmarked synthetic
/// keyDowns here to reach the engine, and the tap swallows them.
pub(crate) const CONTROL: PhysicalKey = PhysicalKey(0x44);
/// Posts "the settings file changed": the tap turns it into a config reload.
pub(crate) const CONTROL_RELOAD: PhysicalKey = PhysicalKey(0x46);

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
        CGEventFlags(modifiers::to_cg(key.flags).0 | native_flags(key.key)),
    );
    CGEvent::set_integer_value_field(
        Some(&event),
        CGEventField::EventSourceUserData,
        SYNTH_MARKER,
    );
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
}

/// The click-to-cancel control path. Posts an UNMARKED keyDown on the
/// reserved control keycode so our own tap sees it (marked events are
/// ignored there), turns it into a cancel, and swallows it before any app
/// can. Callable from any thread; the tap thread is not required.
pub fn post_control() {
    post_raw(CONTROL.0, true);
}

pub fn post_reload() {
    post_raw(CONTROL_RELOAD.0, true);
}

pub(crate) fn is_marked(event: &CGEvent) -> bool {
    CGEvent::integer_value_field(Some(event), CGEventField::EventSourceUserData) == SYNTH_MARKER
}

pub fn post_raw(code: u16, down: bool) {
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
