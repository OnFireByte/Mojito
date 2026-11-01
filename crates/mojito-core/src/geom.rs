//! Overlay placement math, pure and testable. Points are in the global
//! point space (top-left of the primary display, y-down), matching rune's
//! `OverlayPlacement::Rect`.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug)]
pub struct DisplayInfo {
    /// Global-space bounds of the display.
    pub bounds: Rect,
    pub scale: f64,
}

/// The display containing the point, or the primary when none contains it.
pub fn display_containing(point: (f64, f64), displays: &[DisplayInfo]) -> usize {
    let _ = (point, displays);
    todo!("display_containing")
}

/// Center a card of `size` inside `bounds`, clamped so it stays visible.
pub fn centered_rect(size: (f64, f64), bounds: Rect) -> Rect {
    let _ = (size, bounds);
    todo!("centered_rect")
}
