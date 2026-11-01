//! Accessibility permission flow: preflight + live check, prompt, poll,
//! stale-grant recovery. Input Monitoring is never requested.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PermissionState {
    Ready,
    NeedsGrant,
}

/// Preflight (launch-frozen) combined with the live check.
pub fn state() -> PermissionState {
    todo!("permission::state")
}

/// Prompt with kAXTrustedCheckOptionPrompt; opens System Settings.
pub fn prompt() {
    todo!("permission::prompt")
}

/// True when the live check flipped to trusted since launch.
pub fn became_ready_since_launch() -> bool {
    todo!("permission::became_ready_since_launch")
}

/// Offer a tccutil reset and re-prompt after a stale grant (~15 s).
pub fn offer_stale_reset() {
    todo!("permission::offer_stale_reset")
}
