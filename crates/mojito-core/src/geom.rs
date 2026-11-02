//! Overlay placement math, pure and testable. Points and display bounds
//! share one global point space; the macOS switcher passes AppKit's
//! (bottom-left of the primary display, y-up).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The index of the display whose global-space bounds contain the point, or
/// the primary when none contains it. Bounds arrive in points from the
/// backend (already scale-resolved), so a scaled display needs no extra math
/// here. An empty slice yields 0; callers with no displays never place an
/// overlay.
pub fn display_containing(point: (f64, f64), displays: &[Rect]) -> usize {
    displays
        .iter()
        .position(|b| {
            point.0 >= b.x && point.0 < b.x + b.w && point.1 >= b.y && point.1 < b.y + b.h
        })
        .unwrap_or(0)
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

    const PRIMARY: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1440.0,
        h: 900.0,
    };

    const NEGATIVE_LEFT: Rect = Rect {
        x: -1440.0,
        y: 0.0,
        w: 1440.0,
        h: 900.0,
    };

    #[test]
    fn containing_finds_display_across_negative_origin() {
        let displays = [PRIMARY, NEGATIVE_LEFT];
        assert_eq!(display_containing((720.0, 450.0), &displays), 0);
        assert_eq!(display_containing((-100.0, 100.0), &displays), 1);
        // Right edges are exclusive; exactly at 0 belongs to the left display.
        assert_eq!(display_containing((0.0, 0.0), &displays), 0);
        assert_eq!(display_containing((-1440.0, 0.0), &displays), 1);
    }

    #[test]
    fn containing_falls_back_to_primary_outside_all() {
        let displays = [PRIMARY, NEGATIVE_LEFT];
        assert_eq!(display_containing((5000.0, 500.0), &displays), 0);
        assert_eq!(display_containing((600.0, 2000.0), &displays), 0);
        assert_eq!(display_containing((600.0, 500.0), &[]), 0);
    }

    #[test]
    fn containing_matches_points_inside_a_smaller_secondary() {
        let secondary = Rect {
            x: 1440.0,
            y: 0.0,
            w: 720.0,
            h: 450.0,
        };
        let displays = [PRIMARY, secondary];
        assert_eq!(display_containing((2140.0, 400.0), &displays), 1);
        assert_eq!(display_containing((2170.0, 400.0), &displays), 0);
        assert_eq!(display_containing((2140.0, 460.0), &displays), 0);
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
