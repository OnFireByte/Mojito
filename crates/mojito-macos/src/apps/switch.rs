//! Switch app AND window: activate, then AX raise under 0.25 s messaging
//! timeouts. Ladder: focused window -> main window -> first window -> none.
//! No kCGWindowName reads anywhere (that needs Screen Recording permission).

use mojito_core::AppRef;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
    NSWorkspace, NSWorkspaceLaunchOptions,
};
use objc2_foundation::{NSDictionary, NSString, NSURL};

use super::ax::AxElement;
use crate::shell::permission::{self, PermissionState};

pub enum SwitchNote {
    Step(String),
    Problem(String),
}

const KAX_FOCUSED_WINDOW: &str = "AXFocusedWindow";
const KAX_MAIN_WINDOW: &str = "AXMainWindow";
const KAX_WINDOWS: &str = "AXWindows";
const KAX_MINIMIZED: &str = "AXMinimized";
const KAX_RAISE_ACTION: &str = "AXRaise";

/// Resolve, unhide if hidden, activate, then raise the picked window.
/// Degradation: Accessibility denied or no raisable window yields
/// activation-only (still a correct app-level switch). Runs on the main
/// thread as `Job::Switch`.
pub fn switch_to(app: &AppRef, note: &mut dyn FnMut(SwitchNote)) {
    let Some(pid) = resolve(app) else {
        note(SwitchNote::Problem(
            "could not find or launch that app".into(),
        ));
        return;
    };
    note(SwitchNote::Step(format!("resolve: pid {pid}")));
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        note(SwitchNote::Problem(
            "the app quit between resolve and activate".into(),
        ));
        return;
    };
    if app.isHidden() {
        app.unhide();
        note(SwitchNote::Step("unhide".into()));
    }
    unminimize(pid);
    // Activation is best-effort: AXRaise runs even when it is refused.
    let activated = app.activateWithOptions(NSApplicationActivationOptions::empty());
    note(SwitchNote::Step(format!("activate: {activated}")));

    if permission::state() == PermissionState::NeedsGrant {
        note(SwitchNote::Problem(
            "switched app only: grant Accessibility to raise its window".into(),
        ));
        return;
    }
    // A windowless app (Finder preferences, background tools) is a normal
    // activation-only switch, not an error.
    match pick_window(pid) {
        Ok((via, window)) => {
            note(SwitchNote::Step(format!("pick_window: via {via}")));
            match window.perform(KAX_RAISE_ACTION) {
                Ok(()) => note(SwitchNote::Step("raise: ok".into())),
                Err(message) => note(SwitchNote::Problem(message)),
            }
        }
        Err(message) => note(SwitchNote::Step(format!("pick_window: {message}"))),
    }
}

/// AX window pick for one pid: focused -> main -> first -> none, with the
/// winning attribute named.
fn pick_window(pid: i32) -> Result<(&'static str, AxElement), String> {
    let app = AxElement::application(pid).ok_or("AXUIElementCreateApplication returned null")?;
    if let Some(window) = app.element(KAX_FOCUSED_WINDOW) {
        return Ok(("kAXFocusedWindow", window));
    }
    if let Some(window) = app.element(KAX_MAIN_WINDOW) {
        return Ok(("kAXMainWindow", window));
    }
    if let Some(window) = app.elements(KAX_WINDOWS).into_iter().next() {
        return Ok(("kAXWindows[0]", window));
    }
    Err("no focused, main, or listed window".into())
}

/// Debug trace of one full switch against a bundle id; prints every step so
/// a failing ladder names the failing step. Runs without the GUI.
pub fn debug_switch(bundle: &str) {
    let app = AppRef::Bundle(bundle.to_string());
    switch_to(&app, &mut |note| match note {
        SwitchNote::Step(step) => println!("{step}"),
        SwitchNote::Problem(problem) => println!("FAILED: {problem}"),
    });
}

/// Clear kAXMinimized before raising a minimized target. Best-effort.
fn unminimize(pid: i32) {
    let Some(app) = AxElement::application(pid) else {
        return;
    };
    for window in app.elements(KAX_WINDOWS) {
        if window.bool_attribute(KAX_MINIMIZED) == Some(true) {
            window.set_bool_if_settable(KAX_MINIMIZED, false);
        }
    }
}

/// Resolve an AppRef to a live pid, launching when pinned but not running.
/// The launch path uses the synchronous NSWorkspace APIs so one gesture can
/// still complete resolve -> raise; the modern variants are completion-based
/// and cannot hand a pid back to a synchronous caller.
fn resolve(app: &AppRef) -> Option<i32> {
    let running = match app {
        AppRef::Pid(pid) => NSRunningApplication::runningApplicationWithProcessIdentifier(*pid),
        AppRef::Bundle(bundle) => running_with_bundle(bundle).or_else(|| {
            launch(
                NSWorkspace::sharedWorkspace()
                    .URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle))?,
            )
        }),
    };
    running.map(|app| app.processIdentifier())
}

fn running_with_bundle(bundle: &str) -> Option<Retained<NSRunningApplication>> {
    let apps =
        NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(bundle));
    apps.to_vec().into_iter().find(|app| {
        !app.isTerminated() && app.activationPolicy() == NSApplicationActivationPolicy::Regular
    })
}

#[allow(deprecated)] // synchronous launch so one gesture resolves and raises
fn launch(url: Retained<NSURL>) -> Option<Retained<NSRunningApplication>> {
    // SAFETY: `configuration` is an empty dictionary, which the API accepts
    // as "default options"; the returned NSRunningApplication is retained.
    unsafe {
        NSWorkspace::sharedWorkspace().openURL_options_configuration_error(
            &url,
            NSWorkspaceLaunchOptions::empty(),
            &NSDictionary::new(),
        )
    }
    .ok()
}
