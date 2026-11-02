//! App index: NSWorkspace recency ring + CGWindowList window counts,
//! published as ArcSwap<AppIndex> for the engine to read lock-free.
//! Main thread only: every method is called from the AppKit event loop.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFString};
use objc2_foundation::{NSNotification, NSString};

use mojito_core::bridge::{AppIndex, AppSlot};

type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// `kCGWindowLayer` for normal windows.
const LAYER_NORMAL: i64 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, window_id: u32) -> *const c_void;
    fn CFRelease(value: *const c_void);
    fn CGSMainConnectionID() -> i32;
    fn CGSCopySpacesForWindows(connection: i32, mask: i32, windows: &CFArray) -> *const c_void;
}

const ALL_SPACE_KINDS: i32 = 0b111;

pub struct AppIndexHandle {
    current: Arc<ArcSwap<AppIndex>>,
    /// Recency-ordered regular running apps, most recently touched first.
    /// Only the main thread touches it.
    ring: Arc<Mutex<Vec<AppSlot>>>,
    /// NSWorkspace block observers; alive for the process lifetime.
    observers: Mutex<Vec<Observer>>,
}

#[derive(Clone, Copy)]
enum Kind {
    Activate,
    Launch,
    Terminate,
}

impl Default for AppIndexHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl AppIndexHandle {
    pub fn new() -> Self {
        Self {
            current: Arc::new(ArcSwap::from_pointee(AppIndex::default())),
            ring: Arc::new(Mutex::new(Vec::new())),
            observers: Mutex::new(Vec::new()),
        }
    }

    pub fn handle(&self) -> Arc<ArcSwap<AppIndex>> {
        self.current.clone()
    }

    /// Install NSWorkspace observers (activate/launch/terminate) and refresh
    /// the ring. Runs on the main thread.
    pub fn start(&self) {
        if MainThreadMarker::new().is_none() {
            eprintln!("mojito-macos: the app index requires the main thread");
            return;
        }
        seed_ring(&mut self.ring.lock().unwrap_or_else(|error| error.into_inner()));
        publish(&self.current, &self.ring);

        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: reading the notification-name extern statics.
        let notifications = unsafe {
            [
                (
                    objc2_app_kit::NSWorkspaceDidActivateApplicationNotification,
                    Kind::Activate,
                ),
                (
                    objc2_app_kit::NSWorkspaceDidLaunchApplicationNotification,
                    Kind::Launch,
                ),
                (
                    objc2_app_kit::NSWorkspaceDidTerminateApplicationNotification,
                    Kind::Terminate,
                ),
            ]
        };
        for (name, kind) in notifications {
            let ring = Arc::clone(&self.ring);
            let current = Arc::clone(&self.current);
            let block = block2::RcBlock::new(move |note: NonNull<NSNotification>| {
                // SAFETY: the center hands a valid notification to the block.
                let note = unsafe { note.as_ref() };
                observe(&current, &ring, kind, note);
            });
            // SAFETY: the block runs on the main thread and stays registered
            // (and alive) for the process lifetime via `observers`.
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
            };
            self.observers
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(observer);
        }
    }

    /// CGWindowList layer-0 on-screen window counts per pid, merged into the
    /// published index. No kCGWindowName reads.
    pub fn refresh_window_counts(&self) {
        if MainThreadMarker::new().is_none() {
            eprintln!("mojito-macos: window counts require the main thread");
            return;
        }
        let counts = window_counts();
        eprintln!(
            "mojito: ring {} apps, {} with windows", // TEMP diagnosis
            self.ring.lock().unwrap_or_else(|e| e.into_inner()).len(),
            counts.values().filter(|c| **c > 0).count()
        );
        {
            let mut ring = self.ring.lock().unwrap_or_else(|error| error.into_inner());
            for slot in ring.iter_mut() {
                slot.windows = counts.get(&slot.pid).copied().unwrap_or(0);
            }
        }
        publish(&self.current, &self.ring);
    }
}

/// Snapshot the ring into the published index, flagging the frontmost app.
fn publish(current: &Arc<ArcSwap<AppIndex>>, ring: &Mutex<Vec<AppSlot>>) {
    let frontmost = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier());
    let mut index = AppIndex::default();
    let mut ring = ring.lock().unwrap_or_else(|error| error.into_inner());
    for slot in ring.iter_mut() {
        slot.is_frontmost = frontmost.is_some_and(|pid| pid == slot.pid);
    }
    index.slots.clone_from(&ring);
    drop(ring);
    current.store(Arc::new(index));
}

/// Seed the recency ring from the current running applications, frontmost
/// first (recency is unknown at launch; activation notifications take over).
fn seed_ring(ring: &mut Vec<AppSlot>) {
    let workspace = NSWorkspace::sharedWorkspace();
    let frontmost = workspace
        .frontmostApplication()
        .map(|app| app.processIdentifier());
    ring.clear();
    let mut others = Vec::new();
    for app in workspace.runningApplications().to_vec() {
        if app.isTerminated() || app.activationPolicy() != NSApplicationActivationPolicy::Regular {
            continue;
        }
        let slot = slot_for(&app);
        if frontmost.is_some_and(|pid| pid == slot.pid) {
            ring.push(slot);
        } else {
            others.push(slot);
        }
    }
    ring.extend(others);
}

fn slot_for(app: &NSRunningApplication) -> AppSlot {
    AppSlot {
        pid: app.processIdentifier(),
        bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
        name: app
            .localizedName()
            .map(|name| name.to_string())
            .or_else(|| app.bundleIdentifier().map(|id| id.to_string()))
            .unwrap_or_else(|| "Unknown app".into()),
        windows: 0,
        is_frontmost: false,
    }
}

/// Move the observed app to the front of the ring (or drop it on terminate)
/// and republish. NSWorkspace notifications arrive on the main thread.
fn observe(
    current: &Arc<ArcSwap<AppIndex>>,
    ring: &Mutex<Vec<AppSlot>>,
    kind: Kind,
    note: &NSNotification,
) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let Some(app) = observed_app(note) else {
        return;
    };
    {
        let mut ring = ring.lock().unwrap_or_else(|error| error.into_inner());
        match kind {
            Kind::Terminate => ring.retain(|slot| slot.pid != app.processIdentifier()),
            Kind::Activate | Kind::Launch => {
                // Only regular apps belong in the switcher ring.
                if app.activationPolicy() == NSApplicationActivationPolicy::Regular {
                    let slot = slot_for(&app);
                    ring.retain(|existing| existing.pid != slot.pid);
                    ring.insert(0, slot);
                }
            }
        }
    }
    publish(current, ring);
}

/// `NSWorkspaceApplicationKey` -> the NSRunningApplication the notification
/// is about.
fn observed_app(note: &NSNotification) -> Option<Retained<NSRunningApplication>> {
    let info = note.userInfo()?;
    let object = info.objectForKey(&*NSString::from_str("NSWorkspaceApplicationKey"))?;
    // SAFETY: NSWorkspace puts an NSRunningApplication under this key.
    Some(unsafe { Retained::cast_unchecked::<NSRunningApplication>(object) })
}

/// `kCGWindowListOptionAll` (0) plus `kCGWindowListExcludeDesktopElements`
/// (1 << 4): windows on every Space, desktop and wallpaper excluded. An
/// on-screen-only count saw one app when the user was in a fullscreen
/// Space, which emptied the live list.
const ALL_SPACES: u32 = 0 | (1 << 4);

/// Layer-0 window counts keyed by owning pid. Minimized windows still
/// count (their app is switchable); windows on other Spaces count too.
fn window_counts() -> HashMap<i32, u8> {
    let mut counts = HashMap::new();
    let connection = unsafe { CGSMainConnectionID() };
    // SAFETY: CoreGraphics returns a +1 CFArray of CFDictionary window info.
    let list = unsafe { CGWindowListCopyWindowInfo(ALL_SPACES, 0) };
    if list.is_null() {
        return counts;
    }
    // SAFETY: the array lives until the release below and holds dictionaries.
    let array = unsafe { &*(list as *const CFArray<CFDictionary>) };
    for index in 0..array.len() as isize {
        // SAFETY: the index is in bounds and the array is not mutated.
        let dict = unsafe { array.get_unchecked(index) };
        if dict_number(dict, "kCGWindowLayer") != Some(LAYER_NORMAL) {
            continue;
        }
        if dict_cf_number(dict, "kCGWindowAlpha").and_then(CFNumber::as_f64) == Some(0.0) {
            continue;
        }
        let Some(pid) = dict_number(dict, "kCGWindowOwnerPID") else {
            continue;
        };
        let Some(id) = dict_number(dict, "kCGWindowNumber") else {
            continue;
        };
        if !is_on_a_space(connection, id) {
            continue;
        }
        let count = counts.entry(pid as i32).or_insert(0_u8);
        *count = (*count).saturating_add(1);
    }
    // SAFETY: the list was retained by the copy call above.
    unsafe { CFRelease(list) };
    counts
}

fn is_on_a_space(connection: i32, window_id: i64) -> bool {
    let windows = CFArray::from_retained_objects(&[CFNumber::new_i64(window_id)]);
    let spaces =
        unsafe { CGSCopySpacesForWindows(connection, ALL_SPACE_KINDS, windows.as_opaque()) };
    if spaces.is_null() {
        return false;
    }
    let count = unsafe { &*(spaces as *const CFArray) }.len();
    unsafe { CFRelease(spaces) };
    count > 0
}

fn dict_number(dict: &CFDictionary, key: &str) -> Option<i64> {
    dict_cf_number(dict, key).and_then(CFNumber::as_i64)
}

fn dict_cf_number<'a>(dict: &'a CFDictionary, key: &str) -> Option<&'a CFNumber> {
    // SAFETY: the dictionary is alive and its values are CF objects.
    let key = CFString::from_str(key);
    let value = unsafe { dict.value(&*key as *const CFString as *const c_void) };
    if value.is_null() {
        return None;
    }
    // SAFETY: the window info values read here are CFNumbers.
    Some(unsafe { &*(value as *const CFNumber) })
}

/// Debug: every event tap installed on the system, with its pid, tap point,
/// and mask, so a tap that eats keys before ours can be named.
/// `CGGetEventTapList`.
pub fn debug_taps() {
    const MAX: usize = 64;
    /// Mirrors `CGEventTapInformation` (CoreGraphics CGEventTypes.h); the
    /// field order is ABI.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct TapInfo {
        event_tap_id: u32,
        tap_point: u32,
        options: u32,
        event_mask: u64,
        tapping_process: i32,
        process_being_tapped: i32,
        enabled: u8,
        min_latency: f32,
        avg_latency: f32,
        max_latency: f32,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGGetEventTapList(max_taps: u32, taps: *mut TapInfo, count: *mut u32) -> i32;
    }
    let points = ["HID", "session", "annotated session"];
    // SAFETY: the array is filled by CoreGraphics up to `count`.
    unsafe {
        let mut taps = [std::mem::zeroed::<TapInfo>(); MAX];
        let mut count: u32 = 0;
        if CGGetEventTapList(MAX as u32, taps.as_mut_ptr(), &mut count) != 0 {
            println!("CGGetEventTapList failed");
            return;
        }
        for tap in &taps[..count.min(MAX as u32) as usize] {
            let owner = if tap.tapping_process == 0 {
                "kernel".to_string()
            } else {
                process_name(tap.tapping_process)
            };
            let point = points.get(tap.tap_point as usize).copied().unwrap_or("?");
            let kind = if tap.options == 0 { "filter" } else { "listen" };
            println!(
                "pid {} ({}) {} {} mask {:#x} enabled {}",
                tap.tapping_process, owner, point, kind, tap.event_mask, tap.enabled
            );
        }
    }
}

/// The process name for a pid via libproc; "(unknown)" when it is gone.
fn process_name(pid: i32) -> String {
    // libproc_name lives in libSystem, which is always auto-linked.
    unsafe extern "C" {
        fn proc_name(pid: i32, buffer: *mut c_void, size: u32) -> isize;
    }
    let mut buffer = [0_u8; 128];
    // SAFETY: the buffer outlives the call.
    let length = unsafe { proc_name(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if length <= 0 {
        return "(unknown)".into();
    }
    String::from_utf8_lossy(&buffer[..length as usize]).into_owned()
}
pub fn debug_windows() {
    // SAFETY: CoreGraphics returns a +1 CFArray of CFDictionary window info.
    let list = unsafe { CGWindowListCopyWindowInfo(0, 0) };
    if list.is_null() {
        println!("no window list");
        return;
    }
    // SAFETY: the array lives until the release below and holds dictionaries.
    let array = unsafe { &*(list as *const CFArray<CFDictionary>) };
    let mut printed = 0;
    for index in 0..array.len() as isize {
        // SAFETY: the index is in bounds and the array is not mutated.
        let dict = unsafe { array.get_unchecked(index) };
        let Some(owner) = dict_string(dict, "kCGWindowOwnerName") else {
            continue;
        };
        let layer = dict_number(dict, "kCGWindowLayer").unwrap_or(0);
        // Status-level windows (the overlay) or our own process only.
        if owner.to_lowercase().contains("moji") == false && layer < 20 {
            continue;
        }
        let number = dict_number(dict, "kCGWindowNumber").unwrap_or(0);
        let alpha = dict_number(dict, "kCGWindowAlpha").unwrap_or(-1);
        println!("owner {owner:?} window {number} layer {layer} alpha {alpha}");
        printed += 1;
    }
    if printed == 0 {
        println!("no mojito windows on screen");
    }
    // SAFETY: the list was retained by the copy call above.
    unsafe { CFRelease(list) };
}

fn dict_string(dict: &CFDictionary, key: &str) -> Option<String> {
    // SAFETY: the dictionary is alive and its values are CF objects.
    let key = CFString::from_str(key);
    let value = unsafe { dict.value(&*key as *const CFString as *const c_void) };
    if value.is_null() {
        return None;
    }
    // SAFETY: kCGWindowOwnerName values are CFStrings.
    Some(unsafe { &*(value as *const CFString) }.to_string())
}
