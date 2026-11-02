//! Cursor position; rune has no public cursor API so this is the one
//! CoreGraphics call the UI reveal needs.

use objc2_core_graphics::CGEvent;

/// Global-space cursor location in points (top-left of primary, y-down).
/// A fresh event carries the current pointer position.
pub fn location() -> (f64, f64) {
    let Some(event) = CGEvent::new(None) else {
        return (0.0, 0.0);
    };
    let point = CGEvent::location(Some(&event));
    (point.x, point.y)
}
