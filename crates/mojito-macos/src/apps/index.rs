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
use objc2_core_foundation::{CFArray, CFNumber, CFRetained, CFString};
use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption};
use objc2_foundation::{NSNotification, NSString};

use mojito_core::bridge::{AppIndex, AppSlot};

use crate::cf::{cf_dicts, cf_get};
use crate::{lock, main_thread};

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

#[derive(Default)]
pub struct AppIndexHandle {
    current: Arc<ArcSwap<AppIndex>>,
    /// Recency-ordered regular running apps, most recently touched first.
    /// Only the main thread touches it.
    ring: Arc<Mutex<RecencyRing>>,
    /// NSWorkspace block observers; alive for the process lifetime.
    observers: Mutex<Vec<Observer>>,
}

#[derive(Clone, Copy)]
enum Kind {
    Activate,
    Launch,
    Terminate,
}

impl AppIndexHandle {
    pub fn handle(&self) -> Arc<ArcSwap<AppIndex>> {
        self.current.clone()
    }

    /// Install NSWorkspace observers (activate/launch/terminate) and refresh
    /// the ring. Runs on the main thread.
    pub fn start(&self) {
        if main_thread("the app index").is_none() {
            return;
        }
        lock(&self.ring).seed(running_regular_apps(), frontmost_pid());
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
            lock(&self.observers).push(observer);
        }
    }

    /// CGWindowList layer-0 on-screen window counts per pid, merged into the
    /// published index. No kCGWindowName reads.
    pub fn refresh_window_counts(&self) {
        if main_thread("the window count refresh").is_none() {
            return;
        }
        lock(&self.ring).set_window_counts(&window_counts());
        publish(&self.current, &self.ring);
    }
}

#[derive(Default)]
struct RecencyRing {
    slots: Vec<AppSlot>,
}

enum RingEvent {
    Touched(AppSlot),
    Gone(i32),
}

impl RecencyRing {
    fn seed(&mut self, apps: Vec<AppSlot>, frontmost: Option<i32>) {
        let (front, others): (Vec<AppSlot>, Vec<AppSlot>) = apps
            .into_iter()
            .partition(|slot| Some(slot.pid) == frontmost);
        self.slots = front.into_iter().chain(others).collect();
    }

    fn apply(&mut self, event: RingEvent) {
        match event {
            RingEvent::Gone(pid) => self.slots.retain(|slot| slot.pid != pid),
            RingEvent::Touched(slot) => {
                self.slots.retain(|existing| existing.pid != slot.pid);
                self.slots.insert(0, slot);
            }
        }
    }

    fn set_window_counts(&mut self, counts: &HashMap<i32, u8>) {
        for slot in &mut self.slots {
            slot.windows = counts.get(&slot.pid).copied().unwrap_or(0);
        }
    }

    fn snapshot(&self, frontmost: Option<i32>) -> AppIndex {
        let slots = self
            .slots
            .iter()
            .map(|slot| AppSlot {
                is_frontmost: Some(slot.pid) == frontmost,
                ..slot.clone()
            })
            .collect();
        AppIndex { slots }
    }
}

fn publish(current: &Arc<ArcSwap<AppIndex>>, ring: &Mutex<RecencyRing>) {
    current.store(Arc::new(lock(ring).snapshot(frontmost_pid())));
}

fn frontmost_pid() -> Option<i32> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}

fn running_regular_apps() -> Vec<AppSlot> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .to_vec()
        .iter()
        .filter(|app| !app.isTerminated())
        .filter_map(|app| regular_slot(app))
        .collect()
}

fn regular_slot(app: &NSRunningApplication) -> Option<AppSlot> {
    (app.activationPolicy() == NSApplicationActivationPolicy::Regular).then(|| AppSlot {
        pid: app.processIdentifier(),
        bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
        name: app
            .localizedName()
            .map(|name| name.to_string())
            .or_else(|| app.bundleIdentifier().map(|id| id.to_string()))
            .unwrap_or_else(|| "Unknown app".into()),
        windows: 0,
        is_frontmost: false,
    })
}

/// Move the observed app to the front of the ring (or drop it on terminate)
/// and republish. NSWorkspace notifications arrive on the main thread.
fn observe(
    current: &Arc<ArcSwap<AppIndex>>,
    ring: &Mutex<RecencyRing>,
    kind: Kind,
    note: &NSNotification,
) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let Some(app) = observed_app(note) else {
        return;
    };
    let event = match kind {
        Kind::Terminate => Some(RingEvent::Gone(app.processIdentifier())),
        Kind::Activate | Kind::Launch => regular_slot(&app).map(RingEvent::Touched),
    };
    if let Some(event) = event {
        lock(ring).apply(event);
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
    let infos = cf_dicts(list)
        .iter()
        .map(|dict| {
            let number = |key| cf_get::<CFNumber>(dict, key);
            WindowInfo {
                pid: number("kCGWindowOwnerPID").and_then(|n| n.as_i64()),
                number: number("kCGWindowNumber").and_then(|n| n.as_i64()),
                layer: number("kCGWindowLayer").and_then(|n| n.as_i64()),
                alpha: number("kCGWindowAlpha").and_then(|n| n.as_f64()),
                owner_name: cf_get::<CFString>(dict, "kCGWindowOwnerName")
                    .map(|name| name.to_string()),
            }
        })
        .collect();
    Some(infos)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(pid: i32) -> AppSlot {
        AppSlot {
            pid,
            bundle_id: Some(format!("com.test.{pid}")),
            name: format!("app{pid}"),
            windows: 0,
            is_frontmost: false,
        }
    }

    fn order(ring: &RecencyRing) -> Vec<i32> {
        ring.snapshot(None)
            .slots
            .iter()
            .map(|slot| slot.pid)
            .collect()
    }

    #[test]
    fn seed_puts_the_frontmost_app_first_and_keeps_the_rest_in_order() {
        let mut ring = RecencyRing::default();
        ring.seed(vec![slot(1), slot(2), slot(3)], Some(2));
        assert_eq!(order(&ring), vec![2, 1, 3]);
    }

    #[test]
    fn a_touched_app_moves_to_the_front_without_a_duplicate() {
        let mut ring = RecencyRing::default();
        ring.seed(vec![slot(1), slot(2), slot(3)], None);
        ring.apply(RingEvent::Touched(slot(3)));
        ring.apply(RingEvent::Touched(slot(4)));
        assert_eq!(order(&ring), vec![4, 3, 1, 2]);
    }

    #[test]
    fn a_gone_app_leaves_the_ring() {
        let mut ring = RecencyRing::default();
        ring.seed(vec![slot(1), slot(2)], None);
        ring.apply(RingEvent::Gone(1));
        ring.apply(RingEvent::Gone(9));
        assert_eq!(order(&ring), vec![2]);
    }

    #[test]
    fn window_counts_land_on_their_pid_and_absent_pids_read_zero() {
        let mut ring = RecencyRing::default();
        ring.seed(vec![slot(1), slot(2)], None);
        ring.set_window_counts(&HashMap::from([(1, 3)]));
        ring.set_window_counts(&HashMap::from([(2, 2)]));
        let windows: Vec<u8> = ring
            .snapshot(None)
            .slots
            .iter()
            .map(|s| s.windows)
            .collect();
        assert_eq!(windows, vec![0, 2]);
    }

    #[test]
    fn the_snapshot_flags_only_the_frontmost_app() {
        let mut ring = RecencyRing::default();
        ring.seed(vec![slot(1), slot(2)], None);
        let flags: Vec<bool> = ring
            .snapshot(Some(2))
            .slots
            .iter()
            .map(|s| s.is_frontmost)
            .collect();
        assert_eq!(flags, vec![false, true]);
    }
}
