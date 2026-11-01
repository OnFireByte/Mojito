//! Cursor position; rune has no public cursor API so this is the one
//! CoreGraphics call the UI reveal needs.

use std::ffi::c_void;
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy)]
struct CgPoint {
    x: f64,
    y: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CgPoint;
    fn CFRelease(value: *const c_void);
}

/// Global-space cursor location in points (top-left of primary, y-down).
/// A fresh event carries the current pointer position.
pub fn location() -> (f64, f64) {
    // SAFETY: null source selects the default event source; the owned event
    // is read and released here.
    unsafe {
        let event = CGEventCreate(ptr::null_mut());
        if event.is_null() {
            return (0.0, 0.0);
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        (point.x, point.y)
    }
}
