//! Switch app AND window: activate, then AX raise under 0.25 s messaging
//! timeouts. Ladder: focused window -> main window -> first window -> none.
//! No kCGWindowName reads anywhere (that needs Screen Recording permission).

use std::ffi::{CStr, c_char, c_void};
use std::ptr;

use mojito_core::{AppRef, SwitchRequest};

use crate::permission::{self, PermissionState};

pub struct SwitchError(pub String);

type CfTypeRef = *const c_void;
type CfStringRef = *const c_void;
type AxElementRef = *const c_void;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
/// Every AX element gets this timeout so a hung app cannot stall a gesture.
const MESSAGING_TIMEOUT_SECS: f32 = 0.25;

const KAX_FOCUSED_WINDOW: &CStr = c"AXFocusedWindow";
const KAX_MAIN_WINDOW: &CStr = c"AXMainWindow";
const KAX_WINDOWS: &CStr = c"AXWindows";
const KAX_MINIMIZED: &CStr = c"AXMinimized";
const KAX_RAISE_ACTION: &CStr = c"AXRaise";

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AxElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AxElementRef,
        attribute: CfStringRef,
        value: *mut CfTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AxElementRef,
        attribute: CfStringRef,
        value: CfTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AxElementRef, timeout_in_seconds: f32) -> i32;
    fn AXUIElementPerformAction(element: AxElementRef, action: CfStringRef) -> i32;
    fn AXUIElementIsAttributeSettable(
        element: AxElementRef,
        attribute: CfStringRef,
        settable: *mut u8,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        string: *const c_char,
        encoding: u32,
    ) -> CfStringRef;
    fn CFBooleanGetValue(value: CfTypeRef) -> u8;
    static kCFBooleanFalse: CfTypeRef;
    static kCFBooleanTrue: CfTypeRef;
    fn CFArrayGetCount(array: CfTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CfTypeRef, index: isize) -> CfTypeRef;
    fn CFRetain(value: CfTypeRef);
    fn CFRelease(value: CfTypeRef);
}

/// Resolve, unhide if hidden, activate, then raise the picked window.
/// Degradation: Accessibility denied or no raisable window yields
/// activation-only (still a correct app-level switch). Runs on the main
/// thread as `Job::Switch`.
pub fn switch_to(req: &SwitchRequest, on_error: &mut dyn FnMut(&SwitchError)) {
    let Some(pid) = resolve(&req.app) else {
        on_error(&SwitchError("could not find or launch that app".into()));
        return;
    };
    let Some(app) =
        objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
    else {
        on_error(&SwitchError(
            "the app quit between resolve and activate".into(),
        ));
        return;
    };
    if app.isHidden() {
        app.unhide();
    }
    unminimize(pid);
    // Activation is best-effort: AXRaise runs even when it is refused.
    let _ = app.activateWithOptions(objc2_app_kit::NSApplicationActivationOptions::empty());

    if permission::state() == PermissionState::NeedsGrant {
        on_error(&SwitchError(
            "switched app only: grant Accessibility to raise its window".into(),
        ));
        return;
    }
    if let Some(window) = pick_window(pid)
        && let Err(message) = raise(&window)
    {
        on_error(&SwitchError(message));
    }
    // A windowless app (Finder preferences, background tools) is a normal
    // activation-only switch, not an error.
}

/// AX window pick for one pid: focused -> main -> first -> none.
fn pick_window(pid: i32) -> Option<AxWindow> {
    let app = bounded_element(unsafe { AXUIElementCreateApplication(pid) })?;
    copy_element(&app, KAX_FOCUSED_WINDOW)
        .or_else(|| copy_element(&app, KAX_MAIN_WINDOW))
        .or_else(|| first_window(&app))
}

/// The first entry of the app's AXWindows array, retained out of it.
fn first_window(app: &CfObject) -> Option<AxWindow> {
    let windows = copy_attribute(app, KAX_WINDOWS)?;
    // SAFETY: `windows` is a CFArray of AX elements while owned here; the
    // element is borrowed by the array, so we retain our own reference.
    unsafe {
        if CFArrayGetCount(windows.0) < 1 {
            return None;
        }
        let first = CFArrayGetValueAtIndex(windows.0, 0);
        if first.is_null() {
            return None;
        }
        CFRetain(first);
        bounded_element_raw(first)
    }
}

/// Debug trace of one full switch against a bundle id; prints every step so
/// a failing ladder names the failing step. Runs without the GUI.
pub fn debug_switch(bundle: &str) {
    let app = mojito_core::AppRef::Bundle(bundle.to_string());
    let pid = match resolve(&app) {
        Some(pid) => {
            println!("resolve: pid {pid}");
            pid
        }
        None => {
            println!("resolve: FAILED for {bundle}");
            return;
        }
    };
    let running = objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
    if let Some(running) = running {
        println!("hidden: {}", running.isHidden());
        let activated = running.activateWithOptions(objc2_app_kit::NSApplicationActivationOptions::empty());
        println!("activate: {activated}");
    }
    match pick_window_verbose(pid) {
        Ok((via, window)) => {
            println!("pick_window: via {via}");
            match raise(&window) {
                Ok(()) => println!("raise: ok"),
                Err(message) => println!("raise: {message}"),
            }
        }
        Err(message) => println!("pick_window: {message}"),
    }
}

/// The pick ladder with the winning attribute named.
fn pick_window_verbose(pid: i32) -> Result<(String, AxWindow), String> {
    let app = bounded_element(unsafe { AXUIElementCreateApplication(pid) })
        .ok_or("AXUIElementCreateApplication returned null")?;
    if let Some(window) = copy_element(&app, KAX_FOCUSED_WINDOW) {
        return Ok(("kAXFocusedWindow".into(), window));
    }
    if let Some(window) = copy_element(&app, KAX_MAIN_WINDOW) {
        return Ok(("kAXMainWindow".into(), window));
    }
    if let Some(window) = first_window(&app) {
        return Ok(("kAXWindows[0]".into(), window));
    }
    Err("no focused, main, or listed window".into())
}

/// Clear kAXMinimized before raising a minimized target. Best-effort.
fn unminimize(pid: i32) {
    let Some(app) = bounded_element(unsafe { AXUIElementCreateApplication(pid) }) else {
        return;
    };
    let Some(windows) = copy_attribute(&app, KAX_WINDOWS) else {
        return;
    };
    // SAFETY: `windows` is a CFArray of AX elements while owned here.
    unsafe {
        for index in 0..CFArrayGetCount(windows.0) {
            let window = match retained_element_at(&windows, index) {
                Some(window) => window,
                None => continue,
            };
            if attribute_bool(&window, KAX_MINIMIZED) == Some(true) {
                set_bool(&window, KAX_MINIMIZED, false);
            }
        }
    }
}

/// Resolve an AppRef to a live pid, launching when pinned but not running.
/// The launch path uses the synchronous NSWorkspace APIs so one gesture can
/// still complete resolve -> raise; the modern variants are completion-based
/// and cannot hand a pid back to a synchronous caller.
fn resolve(app: &AppRef) -> Option<i32> {
    match app {
        mojito_core::AppRef::Pid(pid) => {
            objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(*pid)
                .map(|running| running.processIdentifier())
        }
        mojito_core::AppRef::Bundle(bundle) => running_with_bundle(bundle)
            .map(|running| running.processIdentifier())
            .or_else(|| {
                launch_url(
                    objc2_app_kit::NSWorkspace::sharedWorkspace()
                        .URLForApplicationWithBundleIdentifier(
                            &objc2_foundation::NSString::from_str(bundle),
                        )?,
                )
            }),
        mojito_core::AppRef::Launch(path, bundle) => running_with_bundle(bundle)
            .map(|running| running.processIdentifier())
            .or_else(|| {
                launch_url(objc2_foundation::NSURL::fileURLWithPath(
                    &objc2_foundation::NSString::from_str(&path.to_string_lossy()),
                ))
            }),
    }
}

fn running_with_bundle(
    bundle: &str,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSRunningApplication>> {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication};
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &objc2_foundation::NSString::from_str(bundle),
    );
    apps.to_vec().into_iter().find(|app| {
        !app.isTerminated() && app.activationPolicy() == NSApplicationActivationPolicy::Regular
    })
}

#[allow(deprecated)] // synchronous launch so one gesture resolves and raises
fn launch_url(url: objc2::rc::Retained<objc2_foundation::NSURL>) -> Option<i32> {
    // SAFETY: `configuration` is an empty dictionary, which the API accepts
    // as "default options"; the returned NSRunningApplication is retained.
    let app = unsafe {
        objc2_app_kit::NSWorkspace::sharedWorkspace().openURL_options_configuration_error(
            &url,
            objc2_app_kit::NSWorkspaceLaunchOptions::empty(),
            &objc2_foundation::NSDictionary::new(),
        )
    }
    .ok()?;
    Some(app.processIdentifier())
}

fn raise(window: &AxWindow) -> Result<(), String> {
    let Some(action) = cf_string(KAX_RAISE_ACTION) else {
        return Err("could not create the AXRaise action name".into());
    };
    let action = CfObject(action);
    // SAFETY: `window` is a valid AX element owned by the caller.
    let status = unsafe { AXUIElementPerformAction(window.0, action.0) };
    if status == 0 {
        Ok(())
    } else {
        Err(format!("AXRaise failed (status {status})"))
    }
}

/// A retained Core Foundation object, released on drop.
struct CfObject(CfTypeRef);

impl Drop for CfObject {
    fn drop(&mut self) {
        // SAFETY: only non-null retained objects are wrapped.
        unsafe { CFRelease(self.0) };
    }
}

/// An owned AXUIElement window.
struct AxWindow(CfTypeRef);

impl Drop for AxWindow {
    fn drop(&mut self) {
        // SAFETY: retained by `bounded_element_raw` before wrapping.
        unsafe { CFRelease(self.0) };
    }
}

fn cf_string(name: &CStr) -> Option<CfStringRef> {
    // SAFETY: CoreFoundation copies the bytes; the result is owned.
    let string =
        unsafe { CFStringCreateWithCString(ptr::null(), name.as_ptr(), CF_STRING_ENCODING_UTF8) };
    (!string.is_null()).then_some(string)
}

/// Take ownership of a fresh AX element and bound its messaging timeout.
fn bounded_element(element: AxElementRef) -> Option<CfObject> {
    if element.is_null() {
        return None;
    }
    let element = CfObject(element);
    // SAFETY: `element` is a valid AX element.
    (unsafe { AXUIElementSetMessagingTimeout(element.0, MESSAGING_TIMEOUT_SECS) } == 0)
        .then_some(element)
}

/// Same bound for an element retained by the caller.
fn bounded_element_raw(element: CfTypeRef) -> Option<AxWindow> {
    // SAFETY: `element` is a valid, retained AX element.
    (unsafe { AXUIElementSetMessagingTimeout(element, MESSAGING_TIMEOUT_SECS) } == 0)
        .then_some(AxWindow(element))
}

fn copy_attribute(element: &CfObject, name: &CStr) -> Option<CfObject> {
    let attribute = CfObject(cf_string(name)?);
    let mut value = ptr::null();
    // SAFETY: `element` is a valid AX element; `value` receives a +1 object.
    let status = unsafe { AXUIElementCopyAttributeValue(element.0, attribute.0, &mut value) };
    // `then`, not `then_some`: dropping a CfObject(null) would CFRelease(NULL).
    (status == 0 && !value.is_null()).then(|| CfObject(value))
}

fn copy_element(element: &CfObject, name: &CStr) -> Option<AxWindow> {
    let child = copy_attribute(element, name)?;
    let window = bounded_element_raw(child.0);
    // Ownership moved from the CfObject into the AxWindow: exactly one owner.
    std::mem::forget(child);
    window
}

/// Borrow (Get rule) the element at `index` of a CFArray of AX elements.
fn retained_element_at(array: &CfObject, index: isize) -> Option<CfObject> {
    // SAFETY: the array is alive and owns its elements for this duration.
    let element = unsafe { CFArrayGetValueAtIndex(array.0, index) };
    if element.is_null() {
        return None;
    }
    // SAFETY: promoting the array's reference to our own.
    unsafe { CFRetain(element) };
    bounded_element(element)
}

fn attribute_bool(element: &CfObject, name: &CStr) -> Option<bool> {
    let value = copy_attribute(element, name)?;
    // SAFETY: the attribute value is a CFBoolean when the attribute exists.
    Some(unsafe { CFBooleanGetValue(value.0) } != 0)
}

fn set_bool(element: &CfObject, name: &CStr, value: bool) {
    let Some(attribute) = cf_string(name) else {
        return;
    };
    let attribute = CfObject(attribute);
    // SAFETY: both are valid global CFBoolean constants.
    let cf_value = if value {
        unsafe { kCFBooleanTrue }
    } else {
        unsafe { kCFBooleanFalse }
    };
    let mut settable: u8 = 0;
    // SAFETY: `element` is a valid AX element.
    let status = unsafe { AXUIElementIsAttributeSettable(element.0, attribute.0, &mut settable) };
    if status != 0 || settable == 0 {
        return;
    }
    // SAFETY: `element` and `cf_value` are valid CF/AX objects.
    unsafe { AXUIElementSetAttributeValue(element.0, attribute.0, cf_value) };
}
