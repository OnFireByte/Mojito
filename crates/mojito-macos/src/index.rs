//! App index: NSWorkspace recency ring + CGWindowList window counts,
//! published as ArcSwap<AppIndex> for the engine to read lock-free.
//! Main thread only: every method is called from the AppKit event loop.

use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, ConcreteType,
};
use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption};
use objc2_foundation::{NSNotification, NSString};

use mojito_core::bridge::{AppIndex, AppSlot};

type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// `kCGWindowLayer` for normal windows.
const LAYER_NORMAL: i64 = 0;
const ALL_SPACE_KINDS: i32 = 0b111;

// SAFETY: private SkyLight SPI, signatures as used by every window manager
// that calls it: the connection id is a plain integer, and
// CGSCopySpacesForWindows returns a +1 CFArray (or null).
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    safe fn CGSMainConnectionID() -> i32;
    fn CGSCopySpacesForWindows(
        connection: i32,
        mask: i32,
        windows: &CFArray,
    ) -> Option<NonNull<CFArray>>;
}

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
        // SAFETY: reading immutable NSString constants exported by AppKit.
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
                // SAFETY: the center hands the block a notification that
                // stays alive for the duration of the call.
                let note = unsafe { note.as_ref() };
                observe(&current, &ring, kind, note);
            });
            // SAFETY: a nil queue delivers on the posting (main) thread, and
            // the observer token is kept in `observers` for the process
            // lifetime, so the block is never called after it is freed.
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
    object.downcast::<NSRunningApplication>().ok()
}

pub(crate) struct WindowInfo {
    pub(crate) pid: Option<i64>,
    pub(crate) number: Option<i64>,
    pub(crate) layer: Option<i64>,
    pub(crate) alpha: Option<f64>,
    pub(crate) owner_name: Option<String>,
}

pub(crate) fn window_infos(option: CGWindowListOption) -> Option<Vec<WindowInfo>> {
    let list = CGWindowListCopyWindowInfo(option, 0)?;
    // SAFETY: every element of a CFArray is a CFType; each one is
    // type-checked with `downcast` before use.
    let list = unsafe { CFRetained::cast_unchecked::<CFArray<CFType>>(list) };
    let infos = list
        .iter()
        .filter_map(|entry| entry.downcast::<CFDictionary>().ok())
        .map(|dict| {
            // SAFETY: window info dictionaries are keyed by CFStrings; lookups
            // only hash and compare keys, and every value is type-checked.
            let dict =
                unsafe { CFRetained::cast_unchecked::<CFDictionary<CFString, CFType>>(dict) };
            let number = |key| dict_value::<CFNumber>(&dict, key);
            WindowInfo {
                pid: number("kCGWindowOwnerPID").and_then(|n| n.as_i64()),
                number: number("kCGWindowNumber").and_then(|n| n.as_i64()),
                layer: number("kCGWindowLayer").and_then(|n| n.as_i64()),
                alpha: number("kCGWindowAlpha").and_then(|n| n.as_f64()),
                owner_name: dict_value::<CFString>(&dict, "kCGWindowOwnerName")
                    .map(|name| name.to_string()),
            }
        })
        .collect();
    Some(infos)
}

fn dict_value<T: ConcreteType>(
    dict: &CFDictionary<CFString, CFType>,
    key: &'static str,
) -> Option<CFRetained<T>> {
    dict.get(&CFString::from_static_str(key))?
        .downcast::<T>()
        .ok()
}

/// Layer-0 window counts keyed by owning pid. Minimized windows still
/// count (their app is switchable); windows on other Spaces count too.
/// Listing every Space, desktop and wallpaper excluded: an on-screen-only
/// count saw one app when the user was in a fullscreen Space, which
/// emptied the live list.
fn window_counts() -> HashMap<i32, u8> {
    let mut counts = HashMap::new();
    let option = CGWindowListOption::OptionAll | CGWindowListOption::ExcludeDesktopElements;
    let connection = CGSMainConnectionID();
    for window in window_infos(option).unwrap_or_default() {
        if window.layer != Some(LAYER_NORMAL) || window.alpha == Some(0.0) {
            continue;
        }
        let (Some(pid), Some(id)) = (window.pid, window.number) else {
            continue;
        };
        if !is_on_a_space(connection, id) {
            continue;
        }
        let count = counts.entry(pid as i32).or_insert(0_u8);
        *count = count.saturating_add(1);
    }
    counts
}

fn is_on_a_space(connection: i32, window_id: i64) -> bool {
    let windows = CFArray::from_retained_objects(&[CFNumber::new_i64(window_id)]);
    // SAFETY: `windows` is a live CFArray of CFNumber window ids, and the
    // returned +1 array is adopted exactly once.
    let spaces = unsafe {
        CGSCopySpacesForWindows(connection, ALL_SPACE_KINDS, windows.as_opaque())
            .map(|raw| CFRetained::from_raw(raw))
    };
    spaces.is_some_and(|spaces| !spaces.is_empty())
}
