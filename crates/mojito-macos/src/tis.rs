//! Input-source selection via TIS: the Caps-style toggle between the two
//! most recently used sources. Main thread only; the list is the enabled,
//! select-capable keyboard input sources in system order. Overlapping TIS
//! calls from worker threads crash, so every entry point refuses to run
//! off-main (hex release 2.1.12 war story).

use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;

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

/// The current keyboard input source's id. Main thread only.
pub fn current_id() -> Result<String, String> {
    if MainThreadMarker::new().is_none() {
        return Err("input-source queries require the main thread".into());
    }
    // SAFETY: main-thread TIS access; the copy is released by `Owned`.
    let current = Owned(unsafe { TISCopyCurrentKeyboardInputSource() });
    if current.0.is_null() {
        return Err("no current input source".into());
    }
    // SAFETY: the ref is a live TIS input source.
    unsafe { string_property(current.0, kTISPropertyInputSourceID) }
        .ok_or_else(|| "current input source has no id".into())
}

/// The recent-two history behind the toggle. The hardware Caps Lock behavior
/// this replaces toggles between the two most recently used sources; macOS
/// exposes no API for its own history, so this process keeps one. Observations
/// land at presses and after each successful select: a source changed since
/// the previous press (input menu, another switcher) becomes the return
/// target. Process-lifetime memory; a fresh process seeds its first return
/// target from the system's `AppleInputSourceHistory` (see
/// [`system_history_names`]), and only a machine with neither record falls
/// back to menu order.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
struct History {
    previous: Option<String>,
    last_seen: Option<String>,
}

/// History after observing `current` now: a change since the last
/// observation makes the previously seen source the return target.
fn observed(history: &History, current: &str) -> History {
    let previous = if history
        .last_seen
        .as_deref()
        .is_some_and(|seen| seen != current)
    {
        history.last_seen.clone()
    } else {
        history.previous.clone()
    };
    History {
        previous,
        last_seen: Some(current.to_string()),
    }
}

/// Pure: the id to select. The remembered previous source wins when it is
/// not the current one and is still selectable (`selectable` is its id when
/// the enabled list contains it); otherwise `fallback`, the next source in
/// menu order.
fn target(
    previous: Option<&str>,
    current: &str,
    selectable: Option<&str>,
    fallback: &str,
) -> String {
    match previous {
        Some(id) if id != current && selectable == Some(id) => id.to_string(),
        _ => fallback.to_string(),
    }
}

static HISTORY: Mutex<History> = Mutex::new(History {
    previous: None,
    last_seen: None,
});

/// Toggle the input source Caps-Lock style: return to the source that was
/// current before the most recent change, so repeated presses alternate
/// between the two most recent sources. Main thread only
/// (`Job::ToggleInputSource`).
pub fn toggle() -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err("input-source toggling requires the main thread".into());
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
            // Nothing to toggle; not an error for a single-language machine.
            return Ok(());
        }
        let current = Owned(TISCopyCurrentKeyboardInputSource());
        let current_id = if current.0.is_null() {
            String::new()
        } else {
            // SAFETY: the ref is a live TIS input source.
            string_property(current.0, kTISPropertyInputSourceID).unwrap_or_default()
        };
        // The next source in menu order stands in until history exists.
        let fallback = match list.iter().position(|entry| entry.id == current_id) {
            Some(index) => list[(index + 1) % list.len()].id.clone(),
            None => list[0].id.clone(),
        };
        let mut history = HISTORY.lock().unwrap_or_else(|error| error.into_inner());
        let mut next = observed(&history, &current_id);
        // Cold start: seed the return target from the system's own recent
        // list, the record the hardware Caps Lock toggle would have used.
        // Our own observations take over from the first select on.
        if next.previous.is_none() {
            for name in system_history_names() {
                let Some(entry) = list.iter().find(|entry| entry.name == name) else {
                    continue;
                };
                if entry.id != current_id {
                    next.previous = Some(entry.id.clone());
                    break;
                }
            }
        }
        let selectable = list
            .iter()
            .find(|entry| entry.id == next.previous.as_deref().unwrap_or_default())
            .map(|entry| entry.id.as_str());
        let target_id = target(next.previous.as_deref(), &current_id, selectable, &fallback);
        let Some(entry) = list.iter().find(|entry| entry.id == target_id) else {
            return Err(format!("target input source {target_id} is not enabled"));
        };
        let status = TISSelectInputSource(entry.source);
        if status != 0 {
            return Err(format!("TISSelectInputSource failed with status {status}"));
        }
        // Commit only on success: the source we just left is the next
        // return target, and the select is the observation going forward.
        *history = History {
            previous: Some(current_id),
            last_seen: Some(target_id),
        };
        Ok(())
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
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
    fn CFPreferencesCopyAppValue(key: Ref, application_id: Ref) -> Ref;
}

/// The system's own recently-used input sources, most recent first:
/// `com.apple.HIToolbox`'s `AppleInputSourceHistory`. Entries carry layout
/// names (not TIS ids), so callers resolve them against the enabled list by
/// localized name. Empty when the key is absent or shaped unexpectedly;
/// this is a best-effort seed, never a hard dependency. Main thread only.
pub fn system_history_names() -> Vec<String> {
    if MainThreadMarker::new().is_none() {
        eprintln!("mojito-macos: TIS calls require the main thread");
        return Vec::new();
    }
    // SAFETY: main-thread CFPreferences read; the returned property list is
    // owned here and released by `Owned`.
    unsafe {
        let key = CFString::from_static_str("AppleInputSourceHistory");
        let app = CFString::from_static_str("com.apple.HIToolbox");
        let raw = CFPreferencesCopyAppValue(
            std::ptr::from_ref(&*key).cast(),
            std::ptr::from_ref(&*app).cast(),
        );
        if raw.is_null() {
            return Vec::new();
        }
        let history = Owned(raw);
        if CFGetTypeID(history.0) != CFArrayGetTypeID() {
            return Vec::new();
        }
        let name_key = CFString::from_static_str("KeyboardLayout Name");
        let mut names = Vec::new();
        for index in 0..CFArrayGetCount(history.0) {
            let entry = CFArrayGetValueAtIndex(history.0, index);
            if CFGetTypeID(entry) != CFDictionaryGetTypeID() {
                continue;
            }
            let value = CFDictionaryGetValue(entry, std::ptr::from_ref(&*name_key).cast());
            if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
                continue;
            }
            names.push((*value.cast::<CFString>()).to_string());
        }
        names
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "com.apple.keylayout.ABC";
    const B: &str = "com.apple.keylayout.Thai";

    fn history(previous: Option<&str>, last_seen: Option<&str>) -> History {
        History {
            previous: previous.map(str::to_string),
            last_seen: last_seen.map(str::to_string),
        }
    }

    /// The core toggle: after switching A -> B, the next press returns to A
    /// and the press after that returns to B, forever.
    #[test]
    fn toggle_oscillates_between_the_two_most_recent() {
        let mut state = history(None, None);
        // First press: no history yet; the fallback stands in.
        let next = observed(&state, A);
        assert_eq!(target(next.previous.as_deref(), A, None, B), B);
        state = History {
            previous: Some(A.into()),
            last_seen: Some(B.into()),
        };
        // Second press: back to A.
        let next = observed(&state, B);
        assert_eq!(target(next.previous.as_deref(), B, Some(A), A), A);
        // Third press: back to B again.
        let next = observed(
            &History {
                previous: Some(A.into()),
                last_seen: Some(A.into()),
            },
            A,
        );
        assert_eq!(next.previous.as_deref(), Some(A));
        assert_eq!(target(next.previous.as_deref(), A, Some(B), B), B);
    }

    /// A change made outside the toggle (input menu, another switcher) is
    /// noticed at the next press and becomes the return target, so the
    /// toggle tracks the two *recently used* sources, not its own writes.
    #[test]
    fn external_switch_becomes_the_return_target() {
        // Last the toggle knew, B was current; the user has since moved to C.
        let state = history(Some(A), Some(B));
        let next = observed(&state, "com.apple.inputmethod.TYIM.Cangjie");
        assert_eq!(next.previous.as_deref(), Some(B));
    }

    /// A remembered source that is no longer enabled cannot be selected;
    /// the menu-order fallback stands in instead.
    #[test]
    fn unselectable_previous_falls_back() {
        assert_eq!(target(Some(A), B, None, B), B);
        assert_eq!(target(Some(A), B, Some("other"), B), B);
    }

    /// Toggling to the current source would be a no-op; the fallback steps
    /// somewhere else instead.
    #[test]
    fn previous_equal_to_current_falls_back() {
        assert_eq!(target(Some(B), B, Some(B), A), A);
    }

    /// The observation collapses to the unchanged source: seeing the same
    /// source twice keeps the previous target untouched.
    #[test]
    fn unchanged_observation_keeps_previous() {
        let state = history(Some(A), Some(B));
        let next = observed(&state, B);
        assert_eq!(next, history(Some(A), Some(B)));
    }
}
