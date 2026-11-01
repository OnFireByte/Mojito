//! Input-source cycling via TIS. Main thread only; the list is the enabled,
//! select-capable keyboard input sources in system order. Overlapping TIS
//! calls from worker threads crash, so every entry point refuses to run
//! off-main (hex release 2.1.12 war story).

use std::ffi::c_void;
use std::ptr;

use objc2::MainThreadMarker;
use objc2_core_foundation::CFString;

/// The enabled input sources, ordered as the system orders them (the input
/// menu order). Main thread only.
pub fn enabled() -> Vec<InputSource> {
    if MainThreadMarker::new().is_none() {
        eprintln!("mojito-macos: TIS calls require the main thread");
        return Vec::new();
    }
    // SAFETY: main-thread TIS access; the array retains its source objects.
    unsafe {
        let sources = Owned(TISCreateInputSourceList(ptr::null(), 0));
        if sources.0.is_null() {
            return Vec::new();
        }
        filter_keyboard_sources(sources.0)
    }
}

pub struct InputSource {
    pub id: String,
    pub name: String,
}

/// Select the next source after the current one; wraps around. Main thread
/// only (`Job::CycleInputSource`).
pub fn cycle() -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err("input-source cycling requires the main thread".into());
    }
    // SAFETY: main-thread TIS access; the array retains its source objects
    // for the whole function, which is what TISSelectInputSource needs.
    unsafe {
        let sources = Owned(TISCreateInputSourceList(ptr::null(), 0));
        if sources.0.is_null() {
            return Err("no enabled input sources".into());
        }
        let list = filter_keyboard_source_refs(sources.0);
        if list.len() < 2 {
            // Nothing to cycle; not an error for a single-language machine.
            return Ok(());
        }
        let current = Owned(TISCopyCurrentKeyboardInputSource());
        let current_id = if current.0.is_null() {
            None
        } else {
            string_property(current.0, kTISPropertyInputSourceID)
        };
        let index = current_id
            .and_then(|id| list.iter().position(|entry| entry.id == id))
            .map_or(0, |index| (index + 1) % list.len());
        let status = TISSelectInputSource(list[index].source);
        if status == 0 {
            Ok(())
        } else {
            Err(format!("TISSelectInputSource failed with status {status}"))
        }
    }
}

struct KeyboardSource {
    id: String,
    name: String,
    source: Ref,
}

/// One +1 CF value. Property values remain borrowed until this drops.
struct Owned(Ref);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: only create/copy results are wrapped in Owned.
            unsafe { CFRelease(self.0) };
        }
    }
}

type Ref = *const c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> Ref;
    fn TISCreateInputSourceList(properties: Ref, include_all_installed: u8) -> Ref;
    fn TISSelectInputSource(source: Ref) -> i32;
    fn TISGetInputSourceProperty(source: Ref, key: Ref) -> Ref;
    static kTISPropertyInputSourceID: Ref;
    static kTISPropertyLocalizedName: Ref;
    static kTISPropertyInputSourceCategory: Ref;
    static kTISCategoryKeyboardInputSource: Ref;
    static kTISPropertyInputSourceIsSelectCapable: Ref;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFEqual(left: Ref, right: Ref) -> u8;
    fn CFBooleanGetValue(value: Ref) -> u8;
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
}

/// The enabled, select-capable keyboard sources with their ids.
///
/// # Safety
/// `sources` must be a live CFArray from `TISCreateInputSourceList`.
unsafe fn filter_keyboard_sources(sources: Ref) -> Vec<InputSource> {
    // SAFETY: the refs come from the live array and stay alive until it drops.
    unsafe {
        filter_keyboard_source_refs(sources)
            .into_iter()
            .map(|entry| InputSource {
                id: entry.id,
                name: entry.name,
            })
            .collect()
    }
}

/// # Safety
/// `sources` must be a live CFArray from `TISCreateInputSourceList`; the
/// returned refs are borrowed from it and valid only while it lives.
unsafe fn filter_keyboard_source_refs(sources: Ref) -> Vec<KeyboardSource> {
    // SAFETY: forwarded to the callers above; the array outlives the loop.
    unsafe {
        let mut result = Vec::new();
        for index in 0..CFArrayGetCount(sources) {
            let source = CFArrayGetValueAtIndex(sources, index);
            let category = TISGetInputSourceProperty(source, kTISPropertyInputSourceCategory);
            let selectable =
                TISGetInputSourceProperty(source, kTISPropertyInputSourceIsSelectCapable);
            if category.is_null()
                || CFEqual(category, kTISCategoryKeyboardInputSource) == 0
                || selectable.is_null()
                || CFBooleanGetValue(selectable) == 0
            {
                continue;
            }
            if let Some(id) = string_property(source, kTISPropertyInputSourceID) {
                let name = string_property(source, kTISPropertyLocalizedName)
                    .unwrap_or_else(|| id.clone());
                result.push(KeyboardSource { id, name, source });
            }
        }
        result
    }
}

unsafe fn string_property(source: Ref, key: Ref) -> Option<String> {
    // SAFETY: the live TIS source owns a CFString for either string property.
    unsafe {
        let value = TISGetInputSourceProperty(source, key);
        (!value.is_null()).then(|| (*value.cast::<CFString>()).to_string())
    }
}
