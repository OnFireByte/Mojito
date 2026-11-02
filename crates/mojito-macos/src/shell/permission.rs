//! Accessibility permission flow: preflight + live check, prompt, poll,
//! stale-grant recovery. Input Monitoring is never requested.

use objc2_application_services::{
    AXIsProcessTrusted, AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt,
};
use objc2_core_foundation::{CFBoolean, CFDictionary};
use objc2_core_graphics::CGPreflightPostEventAccess;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PermissionState {
    Ready,
    NeedsGrant,
}

/// Preflight (launch-frozen) combined with the live check.
pub fn state() -> PermissionState {
    // SAFETY: AXIsProcessTrusted takes no arguments and only reads this
    // process's Accessibility grant.
    let trusted = CGPreflightPostEventAccess() && unsafe { AXIsProcessTrusted() };
    if trusted {
        PermissionState::Ready
    } else {
        PermissionState::NeedsGrant
    }
}

pub fn prompt() {
    // SAFETY: kAXTrustedCheckOptionPrompt is an immutable CFString constant
    // exported by ApplicationServices.
    let key = unsafe { kAXTrustedCheckOptionPrompt };
    let options = CFDictionary::from_slices(&[key], &[CFBoolean::new(true)]);
    // SAFETY: the dictionary maps CFString keys to CFBoolean values, the
    // generic shape AXIsProcessTrustedWithOptions documents.
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) };
}

/// Offer a tccutil reset and re-prompt after a stale grant (~15 s): performs
/// the reset for this bundle id and prompts again. The caller (settings UI)
/// asks the user first; without a bundle id there is nothing to reset.
pub fn offer_stale_reset() {
    let bundle = objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string());
    let Some(bundle) = bundle else {
        eprintln!("mojito-macos: cannot reset Accessibility without a bundle id (bundle the app)");
        return;
    };
    let _ = std::process::Command::new("/usr/bin/tccutil")
        .args(["reset", "Accessibility", &bundle])
        .status();
    prompt();
}
