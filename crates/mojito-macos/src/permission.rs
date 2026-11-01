//! Accessibility permission flow: preflight + live check, prompt, poll,
//! stale-grant recovery. Input Monitoring is never requested.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PermissionState {
    Ready,
    NeedsGrant,
}

/// The answer at first `state()` call, so `became_ready_since_launch` can
/// tell a fresh grant from an app that simply launched trusted.
static INITIAL: AtomicU8 = AtomicU8::new(UNKNOWN);
const UNKNOWN: u8 = 0;
const NEEDS_GRANT: u8 = 1;
const READY: u8 = 2;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const KAX_TRUSTED_CHECK_OPTION_PROMPT: &std::ffi::CStr = c"AXTrustedCheckOptionPrompt";

type CfTypeRef = *const c_void;
type CfStringRef = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// Launch-frozen: true when the process had the grant at launch. Reads
    /// stay live through `AXIsProcessTrusted`.
    safe fn CGPreflightPostEventAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    safe fn AXIsProcessTrusted() -> bool;
    safe fn AXIsProcessTrustedWithOptions(options: CfTypeRef) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        string: *const std::ffi::c_char,
        encoding: u32,
    ) -> CfStringRef;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const c_void,
        values: *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CfTypeRef;
    static kCFBooleanTrue: CfTypeRef;
    static kCFTypeDictionaryKeyCallBacks: CfTypeRef;
    static kCFTypeDictionaryValueCallBacks: CfTypeRef;
    fn CFRelease(value: CfTypeRef);
}

/// Preflight (launch-frozen) combined with the live check.
pub fn state() -> PermissionState {
    // SAFETY: both functions are documented thread-safe read-only checks.
    let trusted = CGPreflightPostEventAccess() && AXIsProcessTrusted();
    record_initial(trusted);
    if trusted {
        PermissionState::Ready
    } else {
        PermissionState::NeedsGrant
    }
}

/// Prompt with kAXTrustedCheckOptionPrompt; opens System Settings.
pub fn prompt() {
    let Some(key) = cf_string(KAX_TRUSTED_CHECK_OPTION_PROMPT) else {
        return;
    };
    let key = CfObject(key);
    // SAFETY: the globals are valid CF constants and the dictionary is
    // released below; AXIsProcessTrustedWithOptions copies what it needs.
    let options = unsafe {
        CFDictionaryCreate(
            ptr::null(),
            [key.0].as_ptr().cast(),
            [kCFBooleanTrue].as_ptr().cast(),
            1,
            // The callbacks are struct constants; their declarations are
            // pointer-typed because we only ever pass their addresses.
            ptr::addr_of!(kCFTypeDictionaryKeyCallBacks).cast::<c_void>(),
            ptr::addr_of!(kCFTypeDictionaryValueCallBacks).cast::<c_void>(),
        )
    };
    if options.is_null() {
        return;
    }
    // SAFETY: a valid dictionary built above; the function is declared safe
    // because it is documented as callable from any thread.
    let _trusted = AXIsProcessTrustedWithOptions(options);
    // SAFETY: the dictionary was created by us.
    unsafe { CFRelease(options) };
}

/// True when the live check flipped to trusted since launch.
pub fn became_ready_since_launch() -> bool {
    INITIAL.load(Ordering::Acquire) == NEEDS_GRANT && state() == PermissionState::Ready
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

fn record_initial(trusted: bool) {
    let _ = INITIAL.compare_exchange(
        UNKNOWN,
        if trusted { READY } else { NEEDS_GRANT },
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

/// A retained Core Foundation object, released on drop.
struct CfObject(CfStringRef);

impl Drop for CfObject {
    fn drop(&mut self) {
        // SAFETY: only non-null retained objects are wrapped.
        unsafe { CFRelease(self.0) };
    }
}

fn cf_string(name: &std::ffi::CStr) -> Option<CfStringRef> {
    // SAFETY: CoreFoundation copies the bytes; the result is owned.
    let string =
        unsafe { CFStringCreateWithCString(ptr::null(), name.as_ptr(), CF_STRING_ENCODING_UTF8) };
    (!string.is_null()).then_some(string)
}
