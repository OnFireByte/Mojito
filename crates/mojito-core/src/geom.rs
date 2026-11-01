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
/// Bounds arrive in points from the backend (already scale-resolved), so a
/// scaled display needs no extra math here — only its rect. An empty slice
/// yields 0; callers with no displays never place an overlay.
pub fn display_containing(point: (f64, f64), displays: &[DisplayInfo]) -> usize {
    for (i, d) in displays.iter().enumerate() {
        let b = &d.bounds;
        if point.0 >= b.x && point.0 < b.x + b.w && point.1 >= b.y && point.1 < b.y + b.h {
            return i;
        }
    }
    0
}

/// Center a card of `size` inside `bounds`, clamped so it stays visible.
/// A card larger than the bounds pins to the bounds' origin rather than
/// centering into negative space.
pub fn centered_rect(size: (f64, f64), bounds: Rect) -> Rect {
    let (w, h) = (size.0.max(0.0), size.1.max(0.0));
    let x = if w >= bounds.w {
        bounds.x
    } else {
        bounds.x + (bounds.w - w) * 0.5
    };
    let y = if h >= bounds.h {
        bounds.y
    } else {
        bounds.y + (bounds.h - h) * 0.5
    };
    Rect { x, y, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primary() -> DisplayInfo {
        DisplayInfo {
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 1440.0,
                h: 900.0,
            },
            scale: 1.0,
        }
    }

    fn scaled_negative_left() -> DisplayInfo {
        DisplayInfo {
            bounds: Rect {
                x: -1440.0,
                y: 0.0,
                w: 1440.0,
                h: 900.0,
            },
            scale: 2.0,
        }
    }

    #[test]
    fn containing_finds_display_across_negative_origin() {
        let displays = [primary(), scaled_negative_left()];
        assert_eq!(display_containing((720.0, 450.0), &displays), 0);
        assert_eq!(display_containing((-100.0, 100.0), &displays), 1);
        // Right edges are exclusive; exactly at 0 belongs to the left display.
        assert_eq!(display_containing((0.0, 0.0), &displays), 0);
        assert_eq!(display_containing((-1440.0, 0.0), &displays), 1);
    }

    #[test]
    fn containing_falls_back_to_primary_outside_all() {
        let displays = [primary(), scaled_negative_left()];
        assert_eq!(display_containing((5000.0, 500.0), &displays), 0);
        assert_eq!(display_containing((600.0, 2000.0), &displays), 0);
        assert_eq!(display_containing((600.0, 500.0), &[]), 0);
    }

    #[test]
    fn containing_ignores_scale_bounds_are_points() {
        // A scaled display's bounds are already in points: a point in its
        // right half must match, not be halved by the scale factor.
        let displays = [DisplayInfo {
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 720.0,
                h: 450.0,
            },
            scale: 2.0,
        }];
        assert_eq!(display_containing((700.0, 400.0), &displays), 0);
        assert_eq!(display_containing((730.0, 400.0), &displays), 0); // fallback
    }

    #[test]
    fn centered_rect_centers_in_bounds() {
        let r = centered_rect(
            (400.0, 300.0),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1440.0,
                h: 900.0,
            },
        );
        assert_eq!(
            r,
            Rect {
                x: 520.0,
                y: 300.0,
                w: 400.0,
                h: 300.0
            }
        );
        // Negative-origin display: the card centers in global space.
        let r = centered_rect(
            (400.0, 300.0),
            Rect {
                x: -1440.0,
                y: 0.0,
                w: 1440.0,
                h: 900.0,
            },
        );
        assert_eq!(
            r,
            Rect {
                x: -920.0,
                y: 300.0,
                w: 400.0,
                h: 300.0
            }
        );
    }

    #[test]
    fn centered_rect_clamps_oversized_cards() {
        let r = centered_rect(
            (2000.0, 100.0),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1440.0,
                h: 900.0,
            },
        );
        assert_eq!(r.x, 0.0);
        assert_eq!(r.y, 400.0);
        let r = centered_rect(
            (2000.0, 2000.0),
            Rect {
                x: -100.0,
                y: -100.0,
                w: 500.0,
                h: 500.0,
            },
        );
        assert_eq!(
            r,
            Rect {
                x: -100.0,
                y: -100.0,
                w: 2000.0,
                h: 2000.0
            }
        );
    }
}
