//! Input-source selection via TIS: the Caps-style toggle between the two
//! most recently used sources. Main thread only; the list is the enabled,
//! select-capable keyboard input sources in system order. Overlapping TIS
//! calls from worker threads crash, so every entry point refuses to run
//! off-main (hex release 2.1.12 war story).

use std::sync::Mutex;

use objc2::MainThreadMarker;
use objc2_core_foundation::{CFArray, CFPreferencesCopyAppValue, CFString};

use super::input_source::{self, Scope};
use crate::cf::{cf_dicts, cf_get};
use crate::{lock, main_thread};

/// The enabled input sources, ordered as the system orders them (the input
/// menu order). Main thread only.
pub fn enabled() -> Vec<KeyboardSource> {
    main_thread("TIS access").map_or_else(Vec::new, selectable_keyboards)
}

/// The current keyboard input source's id. Main thread only.
pub fn current_id() -> Result<String, String> {
    let mtm = MainThreadMarker::new().ok_or("input-source queries require the main thread")?;
    input_source::current_keyboard(mtm)
        .ok_or("no current input source")?
        .id()
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
    let mtm = MainThreadMarker::new().ok_or("input-source toggling requires the main thread")?;
    let list = selectable_keyboards(mtm);
    if list.len() < 2 {
        // Nothing to toggle; not an error for a single-language machine.
        return Ok(());
    }
    let current_id = input_source::current_keyboard(mtm)
        .and_then(|source| source.id())
        .unwrap_or_default();
    // The next source in menu order stands in until history exists.
    let fallback = match list.iter().position(|entry| entry.id == current_id) {
        Some(index) => list[(index + 1) % list.len()].id.clone(),
        None => list[0].id.clone(),
    };
    let mut history = lock(&HISTORY);
    let mut next = observed(&history, &current_id);
    if next.previous.is_none() {
        next.previous = system_return_target(&list, &current_id);
    }
    let selectable = list
        .iter()
        .find(|entry| entry.id == next.previous.as_deref().unwrap_or_default())
        .map(|entry| entry.id.as_str());
    let target_id = target(next.previous.as_deref(), &current_id, selectable, &fallback);
    let Some(entry) = list.iter().find(|entry| entry.id == target_id) else {
        return Err(format!("target input source {target_id} is not enabled"));
    };
    entry.source.select()?;
    // Commit only on success: the source we just left is the next
    // return target, and the select is the observation going forward.
    *history = History {
        previous: Some(current_id),
        last_seen: Some(target_id),
    };
    Ok(())
}

/// Cold start: seed the return target from the system's own recent
/// list, the record the hardware Caps Lock toggle would have used.
/// Our own observations take over from the first select on.
fn system_return_target(list: &[KeyboardSource], current_id: &str) -> Option<String> {
    system_history_names().into_iter().find_map(|name| {
        let entry = list.iter().find(|entry| entry.name == name)?;
        (entry.id != current_id).then(|| entry.id.clone())
    })
}

pub struct KeyboardSource {
    pub id: String,
    pub name: String,
    source: input_source::InputSource,
}

fn selectable_keyboards(mtm: MainThreadMarker) -> Vec<KeyboardSource> {
    input_source::list(mtm, Scope::Enabled)
        .into_iter()
        .filter(|source| source.is_keyboard() && source.is_select_capable())
        .filter_map(|source| {
            let id = source.id()?;
            let name = source.localized_name().unwrap_or_else(|| id.clone());
            Some(KeyboardSource { id, name, source })
        })
        .collect()
}

/// The system's own recently-used input sources, most recent first:
/// `com.apple.HIToolbox`'s `AppleInputSourceHistory`. Entries carry layout
/// names (not TIS ids), so callers resolve them against the enabled list by
/// localized name. Empty when the key is absent or shaped unexpectedly;
/// this is a best-effort seed, never a hard dependency. Main thread only.
pub fn system_history_names() -> Vec<String> {
    if main_thread("TIS access").is_none() {
        return Vec::new();
    }
    let key = CFString::from_static_str("AppleInputSourceHistory");
    let app = CFString::from_static_str("com.apple.HIToolbox");
    let Some(history) =
        CFPreferencesCopyAppValue(&key, &app).and_then(|value| value.downcast::<CFArray>().ok())
    else {
        return Vec::new();
    };
    cf_dicts(history)
        .iter()
        .filter_map(|entry| cf_get::<CFString>(entry, "KeyboardLayout Name"))
        .map(|name| name.to_string())
        .collect()
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
