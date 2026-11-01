//! Layout snapshot: UCKeyTranslate over keycodes 0..128, taken once on the
//! main thread and refreshed on input-source change. Physical key -> glyph
//! for badges only; TIS access stays behind one mutex.

use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::MainThreadMarker;
use objc2_core_foundation::CFData;

/// The keycode -> glyph table of the snapshotted layout; None = no printable
/// glyph (arrows, F-keys, dead keys).
type GlyphTable = [Option<char>; 128];
static TABLE: OnceLock<Mutex<Option<Box<GlyphTable>>>> = OnceLock::new();
static OBSERVER_INSTALLED: AtomicBool = AtomicBool::new(false);

const KEY_ACTION_DISPLAY: u16 = 3;
const NO_DEAD_KEYS: isize = 0;
const NO_MODIFIER_STATE: u32 = 0;

type InputSourceRef = *const c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> InputSourceRef;
    static kTISPropertyUnicodeKeyLayoutData: InputSourceRef;
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

/// Display glyph for a physical position under the snapshotted layout.
/// Never re-enters TIS: a miss is a miss until the next snapshot.
pub fn glyph(key: mojito_core::PhysicalKey) -> Option<char> {
    let guard = table_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let table = guard.as_ref()?;
    table.get(key.0 as usize).copied().flatten()
}

fn table_cell() -> &'static Mutex<Option<Box<[Option<char>; 128]>>> {
    TABLE.get_or_init(|| Mutex::new(None))
}

/// Translate the current layout, keeping the previous snapshot when the
/// layout has no Unicode mapping (some IMEs); a broken snapshot is worse
/// than a stale one.
fn take_snapshot() -> Option<Box<GlyphTable>> {
    // SAFETY: TIS calls on the main thread; the copied source is released
    // below and the layout bytes are copied out before that.
    unsafe {
        let source = TISCopyCurrentKeyboardLayoutInputSource();
        if source.is_null() {
            return None;
        }
        let source = Owned(source);
        let layout_data = TISGetInputSourceProperty(source.0, kTISPropertyUnicodeKeyLayoutData);
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
