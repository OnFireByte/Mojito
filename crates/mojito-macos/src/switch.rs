//! Switch app AND window: activate, then AX raise under 0.25 s messaging
//! timeouts. Ladder: focused window -> main window -> first window -> none.
//! No kCGWindowName reads anywhere (that needs Screen Recording permission).

use mojito_core::{AppRef, SwitchRequest};

pub struct SwitchError(pub String);

/// Resolve, unhide if hidden, activate, then raise the picked window.
/// Degradation: Accessibility denied or no raisable window yields
/// activation-only (still a correct app-level switch).
pub fn switch_to(req: &SwitchRequest, on_error: &mut dyn FnMut(&SwitchError)) {
    let _ = (req, on_error);
    todo!("switch::switch_to")
}

/// AX window pick for one pid: focused -> main -> first -> none.
fn pick_window(pid: i32) -> Option<ax_window> {
    let _ = pid;
    todo!("pick_window")
}

enum ax_window {}

/// Clear kAXMinimized before raising a minimized target.
fn unminimize(pid: i32) {
    let _ = pid;
    todo!("unminimize")
}

/// Resolve an AppRef to a live pid, launching when pinned but not running.
fn resolve(app: &AppRef) -> Option<i32> {
    let _ = app;
    todo!("resolve")
}
