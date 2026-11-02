//! Layout tables: keycode -> glyph via UCKeyTranslate. The base table is the
//! current input source, snapshotted on the main thread and refreshed on
//! input-source change. A user-chosen display layout overrides it, so hints
//! can show QWERTY positions under Colemak or Thai. TIS access stays on the
//! main thread behind one mutex.

use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::MainThreadMarker;
use objc2_core_foundation::{
    CFDictionary, CFNotificationCenter, CFNotificationName, CFNotificationSuspensionBehavior,
};

use super::input_source::{self, InputSource, Scope, kTISNotifySelectedKeyboardInputSourceChanged};
use crate::{abort_on_panic, lock, main_thread};

/// The keycode -> glyph table of the snapshotted layout; None = no printable
/// glyph (arrows, F-keys, dead keys).
type GlyphTable = [Option<char>; 128];
static TABLE: Mutex<Option<Box<GlyphTable>>> = Mutex::new(None);
static SELECTED: Mutex<Option<Box<GlyphTable>>> = Mutex::new(None);
static OBSERVER_INSTALLED: AtomicBool = AtomicBool::new(false);

const KEY_ACTION_DISPLAY: u16 = 3;
const NO_DEAD_KEYS: u32 = 0;
const NO_MODIFIER_STATE: u32 = 0;

// SAFETY: signatures match Carbon's UnicodeUtilities.h and TextInputSources.h.
// `LMGetKbdType` takes no arguments and only reads the current keyboard type.
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn UCKeyTranslate(
        layout: *const u8,
        key_code: u16,
        key_action: u16,
        modifier_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        output: *mut u16,
    ) -> i32;
    safe fn LMGetKbdType() -> u8;
}

/// Snapshot the current layout's keycode-to-glyph table and refresh it on
/// every later input-source change. Main thread only; call once at startup.
pub fn snapshot() {
    let Some(mtm) = main_thread("the layout snapshot") else {
        return;
    };
    refresh_snapshot(mtm);
    install_observer();
}

/// Display glyph for a physical position. The user-chosen display layout
/// wins; otherwise the current input source's table. Never re-enters TIS.
pub fn glyph(key: mojito_core::PhysicalKey) -> Option<char> {
    let selected = lock(&SELECTED);
    let current = lock(&TABLE);
    let table = selected.as_ref().or(current.as_ref())?;
    table.get(key.0 as usize).copied().flatten()
}

/// One installed keyboard layout the hint picker can offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutInfo {
    /// `kTISPropertyInputSourceID`, e.g. "com.apple.keylayout.US".
    pub id: String,
    /// Localized display name, e.g. "U.S.".
    pub name: String,
}

/// Every installed keyboard layout that can render glyphs, in system order.
/// Includes layouts that are installed but not currently enabled. Main
/// thread only (TIS).
pub fn installed() -> Vec<LayoutInfo> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    glyph_layouts(mtm)
        .filter_map(|source| {
            Some(LayoutInfo {
                id: source.id()?,
                name: source.localized_name()?,
            })
        })
        .collect()
}

/// Pick which installed layout renders hint glyphs; None follows the
/// current input source. Main thread only (TIS).
pub fn set_display_layout(id: Option<String>) {
    let Some(mtm) = main_thread("set_display_layout") else {
        return;
    };
    let table = id.and_then(|id| {
        glyph_layouts(mtm)
            .find(|source| source.id().as_deref() == Some(id.as_str()))
            .and_then(|source| translate_table_from(&source))
    });
    *lock(&SELECTED) = table;
}

fn glyph_layouts(mtm: MainThreadMarker) -> impl Iterator<Item = InputSource> {
    input_source::list(mtm, Scope::Installed)
        .into_iter()
        .filter(|source| source.is_keyboard() && source.key_layout_data().is_some())
}

/// Translate the current layout, keeping the previous snapshot when the
/// layout has no Unicode mapping (some IMEs); a broken snapshot is worse
/// than a stale one.
fn refresh_snapshot(mtm: MainThreadMarker) {
    let table = input_source::current_layout(mtm).and_then(|source| translate_table_from(&source));
    if let Some(table) = table {
        lock(&TABLE).replace(table);
    }
}

fn translate_table_from(source: &InputSource) -> Option<Box<GlyphTable>> {
    let layout = source.key_layout_data()?.to_vec();
    let keyboard_type = u32::from(LMGetKbdType());
    let mut table: Box<GlyphTable> = Box::new([None; 128]);
    for (key_code, slot) in (0_u16..).zip(table.iter_mut()) {
        let mut dead_key_state = 0_u32;
        let mut output = [0_u16; 4];
        let mut length = 0_usize;
        // SAFETY: `layout` is the source's `uchr` keyboard-layout data, alive
        // for the call; every out-pointer is a live local and `output.len()`
        // bounds the write.
        let status = unsafe {
            UCKeyTranslate(
                layout.as_ptr(),
                key_code,
                KEY_ACTION_DISPLAY,
                NO_MODIFIER_STATE,
                keyboard_type,
                NO_DEAD_KEYS,
                &mut dead_key_state,
                output.len(),
                &mut length,
                output.as_mut_ptr(),
            )
        };
        if status != 0 || length < 1 {
            continue;
        }
        *slot = String::from_utf16(&output[..length.min(output.len())])
            .ok()
            .and_then(|text| text.chars().next());
    }
    Some(table)
}

/// Distributed notifications deliver on the registering thread's run loop;
/// the observer is registered from the main thread, so the refresh runs
/// there, where TIS is safe.
unsafe extern "C-unwind" fn input_source_changed(
    _center: *mut CFNotificationCenter,
    _observer: *mut c_void,
    _name: *const CFNotificationName,
    _object: *const c_void,
    _user_info: *const CFDictionary,
) {
    abort_on_panic(|| {
        if let Some(mtm) = MainThreadMarker::new() {
            refresh_snapshot(mtm);
        }
    });
}

/// A stable dummy address identifies this observer; the registration lives
/// for the process, so it is never removed.
static OBSERVER_ID: u8 = 0;

fn install_observer() {
    if OBSERVER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(center) = CFNotificationCenter::distributed_center() else {
        return;
    };
    // SAFETY: the observer id is a static, so it outlives the registration;
    // the callback matches CFNotificationCallback, ignores its pointers,
    // aborts instead of unwinding, and only runs on the main thread it was
    // registered from. The name is an immutable HIToolbox constant.
    unsafe {
        center.add_observer(
            ptr::from_ref(&OBSERVER_ID).cast(),
            Some(input_source_changed),
            Some(kTISNotifySelectedKeyboardInputSourceChanged),
            ptr::null(),
            CFNotificationSuspensionBehavior::DeliverImmediately,
        );
    }
}
