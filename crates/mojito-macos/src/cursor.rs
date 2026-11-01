//! Cursor position; rune has no public cursor API so this is the one
//! CoreGraphics call the UI reveal needs.

/// Global-space cursor location in points (top-left of primary, y-down).
pub fn location() -> (f64, f64) {
    todo!("cursor::location")
}
