//! Layout tables: keycode -> glyph via UCKeyTranslate. The base table is the
//! current input source, snapshotted on the main thread and refreshed on
//! input-source change. A user-chosen display layout overrides it, so hints
//! can show QWERTY positions under Colemak or Thai. TIS access stays on the
//! main thread behind one mutex.

use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::MainThreadMarker;
use objc2_core_foundation::{CFArray, CFData, CFRetained, CFString, CFType};

/// The keycode -> glyph table of the snapshotted layout; None = no printable
/// glyph (arrows, F-keys, dead keys).
type GlyphTable = [Option<char>; 128];
static TABLE: OnceLock<Mutex<Option<Box<GlyphTable>>>> = OnceLock::new();
/// The user-chosen display layout, when one is set.
static SELECTED: OnceLock<Mutex<Option<Selected>>> = OnceLock::new();
static OBSERVER_INSTALLED: AtomicBool = AtomicBool::new(false);

struct Selected {
    table: Box<GlyphTable>,
}

const KEY_ACTION_DISPLAY: u16 = 3;
const NO_DEAD_KEYS: isize = 0;
const NO_MODIFIER_STATE: u32 = 0;

type InputSourceRef = *const c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> InputSourceRef;
    fn TISCreateInputSourceList(
        properties_filter: *const c_void,
        include_all_installed: u8,
    ) -> InputSourceRef;
    static kTISPropertyUnicodeKeyLayoutData: InputSourceRef;
    static kTISPropertyInputSourceID: InputSourceRef;
    static kTISPropertyLocalizedName: InputSourceRef;
    static kTISPropertyInputSourceCategory: InputSourceRef;
    static kTISCategoryKeyboardInputSource: InputSourceRef;
    fn TISGetInputSourceProperty(
        input_source: InputSourceRef,
        property_key: InputSourceRef,
    ) -> InputSourceRef;
    fn UCKeyTranslate(
        layout: *const u8,
        key_code: u16,
        key_action: u16,
        modifier_state: u32,
        keyboard_type: u32,
        options: isize,
        dead_key_state: *mut u32,
        max_length: isize,
        actual_length: *mut isize,
        output: *mut u16,
    ) -> i32;
    fn LMGetKbdType() -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: InputSourceRef);
    fn CFRetain(value: InputSourceRef);
    fn CFNotificationCenterGetDistributedCenter() -> InputSourceRef;
    fn CFNotificationCenterAddObserver(
        center: InputSourceRef,
        observer: InputSourceRef,
        callback: unsafe extern "C" fn(
            InputSourceRef,
            *mut c_void,
            InputSourceRef,
            InputSourceRef,
            InputSourceRef,
        ),
        name: InputSourceRef,
        object: InputSourceRef,
        suspension_behavior: isize,
    );
    static kTISNotifySelectedKeyboardInputSourceChanged: InputSourceRef;
}

/// Snapshot the current layout's keycode-to-glyph table and refresh it on
/// every later input-source change. Main thread only; call once at startup.
pub fn snapshot() {
    if MainThreadMarker::new().is_none() {
        eprintln!("mojito-macos: the layout snapshot requires the main thread");
        return;
    }
    if let Some(table) = take_snapshot() {
        table_cell()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(table);
    }
    install_observer();
}

/// Display glyph for a physical position. The user-chosen display layout
/// wins; otherwise the current input source's table. Never re-enters TIS.
pub fn glyph(key: mojito_core::PhysicalKey) -> Option<char> {
    if let Some(selected) = selected_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        return selected.table.get(key.0 as usize).copied().flatten();
    }
    let guard = table_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let table = guard.as_ref()?;
    table.get(key.0 as usize).copied().flatten()
}

fn selected_cell() -> &'static Mutex<Option<Selected>> {
    SELECTED.get_or_init(|| Mutex::new(None))
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
    if MainThreadMarker::new().is_none() {
        return Vec::new();
    }
    // SAFETY: TIS enumeration on the main thread; the list is owned and
    // released on drop, and elements are borrowed from it.
    unsafe {
        let list = Owned(TISCreateInputSourceList(ptr::null(), 1));
        if list.0.is_null() {
            return Vec::new();
        }
        let array = &*(list.0 as *const CFArray<CFType>);
        let category = (*kTISCategoryKeyboardInputSource.cast::<CFString>()).to_string();
        array
            .to_vec()
            .iter()
            .filter(|source| {
                string_property(CFRetained::as_ptr(source).as_ptr() as *const c_void, kTISPropertyInputSourceCategory)
                    .is_some_and(|value| value == category)
            })
            .filter(|source| {
                !TISGetInputSourceProperty(
                    CFRetained::as_ptr(source).as_ptr() as *const c_void,
                    kTISPropertyUnicodeKeyLayoutData,
                )
                .is_null()
            })
            .filter_map(|source| {
                let source = CFRetained::as_ptr(source).as_ptr() as *const c_void;
                let id = string_property(source, kTISPropertyInputSourceID)?;
                let name = string_property(source, kTISPropertyLocalizedName)?;
                Some(LayoutInfo { id, name })
            })
            .collect()
    }
}

/// A CFString property of an input source as a Rust string.
///
/// # Safety
/// `source` must be a live TISInputSource; `key` a live CFString global.
unsafe fn string_property(source: InputSourceRef, key: InputSourceRef) -> Option<String> {
    // SAFETY: both arguments are live CF objects per the caller's contract.
    let value = unsafe { TISGetInputSourceProperty(source, key) };
    if value.is_null() {
        return None;
    }
    // SAFETY: the property is a CFString for the keys queried here.
    Some(unsafe { (*value.cast::<CFString>()).to_string() })
}

/// Pick which installed layout renders hint glyphs; None follows the
/// current input source. Main thread only (TIS).
pub fn set_display_layout(id: Option<String>) {
    if MainThreadMarker::new().is_none() {
        eprintln!("mojito-macos: set_display_layout requires the main thread");
        return;
    }
    let table = id
        .as_deref()
        .and_then(|id| unsafe { installed_source(id) })
        .and_then(|source| {
            let owned = Owned(source);
            translate_table_from(owned.0)
        });
    let mut selected = selected_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    match (id, table) {
        (Some(_id), Some(table)) => *selected = Some(Selected { table }),
        _ => *selected = None,
    }
}

/// The installed input source with this id, when it carries a key layout.
/// The returned source is retained; the caller owns the release.
///
/// # Safety
/// Main thread only; TIS is not thread-safe.
unsafe fn installed_source(id: &str) -> Option<InputSourceRef> {
    // SAFETY: main thread; TIS is not thread-safe, and the list outlives
    // every borrowed element below.
    return unsafe {
    let list = Owned(TISCreateInputSourceList(ptr::null(), 1));
    if list.0.is_null() {
        return None;
    }
    let array = &*(list.0 as *const CFArray<CFType>);
    array.to_vec().iter().find_map(|source| {
        let source = CFRetained::as_ptr(source).as_ptr() as *const c_void;
        let matches = string_property(source, kTISPropertyInputSourceID).is_some_and(|s| s == id);
        let usable =
            !TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData).is_null();
        if matches && usable {
            CFRetain(source);
            Some(source)
        } else {
            None
        }
    })
    }
}

fn table_cell() -> &'static Mutex<Option<Box<[Option<char>; 128]>>> {
    TABLE.get_or_init(|| Mutex::new(None))
}

/// Translate the current layout, keeping the previous snapshot when the
/// layout has no Unicode mapping (some IMEs); a broken snapshot is worse
/// than a stale one.
fn take_snapshot() -> Option<Box<GlyphTable>> {
    // SAFETY: TIS calls on the main thread; the copied source is released
    // on drop.
    unsafe {
        let source = Owned(TISCopyCurrentKeyboardLayoutInputSource());
        if source.0.is_null() {
            return None;
        }
        translate_table_from(source.0)
    }
}

/// Translate one input source's key layout into a glyph table. The caller
/// owns `source`, which must outlive the call.
fn translate_table_from(source: InputSourceRef) -> Option<Box<GlyphTable>> {
    // SAFETY: TIS reads on the main thread; the layout bytes are copied out
    // before any release.
    unsafe {
        let layout_data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        if layout_data.is_null() {
            return None;
        }
        let bytes = (*layout_data.cast::<CFData>()).to_vec();
        let layout = bytes.as_ptr();
        let keyboard_type = LMGetKbdType().into();
        let mut table: Box<[Option<char>; 128]> = Box::new(std::array::from_fn(|_| None));
        for key_code in 0_u16..128 {
            let mut dead_key_state: u32 = 0;
            let mut output = [0_u16; 4];
            let mut length: isize = 0;
            let status = UCKeyTranslate(
                layout,
                key_code,
                KEY_ACTION_DISPLAY,
                NO_MODIFIER_STATE,
                keyboard_type,
                NO_DEAD_KEYS,
                &mut dead_key_state,
                output.len() as isize,
                &mut length,
                output.as_mut_ptr(),
            );
            if status != 0 || length < 1 {
                continue;
            }
            if let Ok(text) = String::from_utf16(&output[..length as usize]) {
                table[key_code as usize] = text.chars().next();
            }
        }
        Some(table)
    }
}

/// One +1 TIS input source, released on drop.
struct Owned(InputSourceRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the copy call retained it for us.
            unsafe { CFRelease(self.0) };
        }
    }
}

/// Distributed notifications deliver on the registering thread's run loop;
/// the observer is registered from the main thread, so the refresh runs
/// there, where TIS is safe.
unsafe extern "C" fn input_source_changed(
    _center: InputSourceRef,
    _observer: *mut c_void,
    _name: InputSourceRef,
    _object: InputSourceRef,
    _user_info: InputSourceRef,
) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    if let Some(table) = take_snapshot() {
        table_cell()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(table);
    }
}

/// A stable dummy address identifies this observer; the registration lives
/// for the process, so it is never removed.
static OBSERVER_ID: u8 = 0;

fn install_observer() {
    if OBSERVER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    // SAFETY: the observer id outlives the registration; the callback only
    // runs on the main thread it was registered from. 4 = DeliverImmediately.
    unsafe {
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetDistributedCenter(),
            ptr::from_ref(&OBSERVER_ID).cast(),
            input_source_changed,
            kTISNotifySelectedKeyboardInputSourceChanged,
            ptr::null(),
            4,
        );
    }
}
