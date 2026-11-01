//! hidutil Caps->F18 remap: idempotent install, LaunchAgent persistence,
//! self-heal, clean uninstall that preserves foreign UserKeyMapping entries.

use std::process::Command;

use mojito_core::{CAPS_USAGE, F18_USAGE};

pub struct CapsMap;

/// One UserKeyMapping entry as hidutil reports it.
///
/// Wire values are wider than 32 bits: hidutil's canonical encoding for a
/// keyboard-page usage is `(0x07 << 32) | usage` (Caps Lock prints as
/// 30064771129), so these are u64 on purpose.
pub struct MappingEntry {
    pub src: u64,
    pub dst: u64,
}

/// The dictionary keys hidutil uses inside UserKeyMapping entries. Isolated
/// here because the exact encoding needs on-device verification.
const SRC_KEY: &str = "HIDKeyboardModifierMappingSrc";
const DST_KEY: &str = "HIDKeyboardModifierMappingDst";
const AGENT_LABEL: &str = "com.mojito.caps";

impl CapsMap {
    /// Merge our entry into the existing UserKeyMapping and set it. Idempotent.
    /// An absent mapping (a fresh machine prints `(null)`) is an empty slate;
    /// a mapping that exists but fails to parse stays an error so foreign
    /// entries are never clobbered.
    pub fn install(&self) -> Result<(), String> {
        let merged = merged_or_current()?;
        hidutil_set(&merged)?;
        if !contains_pair(&hidutil_get()?, our_pair()) {
            return Err("hidutil did not accept the Caps Lock remap".into());
        }
        Ok(())
    }

    /// Remove only our entry (matched on Src == CAPS_USAGE), bootout the
    /// LaunchAgent, delete the plist. A foreign entry remapping something
    /// else TO F18 survives.
    pub fn uninstall(&self) -> Result<(), String> {
        let current = hidutil_get()?;
        if !absent_mapping(&current) {
            let remaining = without_entry(&current, CAPS_USAGE.0)?;
            hidutil_set(&remaining)?;
        }
        bootout_agent();
        delete_plist();
        Ok(())
    }

    /// True when the current UserKeyMapping contains our caps->F18 pair.
    pub fn is_active(&self) -> bool {
        match hidutil_get() {
            Ok(current) => contains_pair(&current, our_pair()),
            Err(_) => false,
        }
    }

    /// Write and bootstrap the RunAtLoad LaunchAgent so the mapping survives
    /// reboot; Sequoia may require one Login-Items consent click. The agent
    /// carries the merged mapping as of this call, so foreign entries are
    /// replayed too; mappings added after this call are clobbered by the
    /// next login (documented v1 limitation).
    pub fn ensure_agent(&self) -> Result<(), String> {
        let Some(home) = std::env::var_os("HOME") else {
            return Err("no home directory; cannot install the LaunchAgent".into());
        };
        let plist = std::path::Path::new(&home)
            .join("Library/LaunchAgents")
            .join(format!("{AGENT_LABEL}.plist"));
        if let Some(parent) = plist.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create LaunchAgents: {error}"))?;
        }
        let mapping = merged_or_current()?;
        let payload = plist_xml(&mapping);
        std::fs::write(&plist, payload)
            .map_err(|error| format!("could not write {}: {error}", plist.display()))?;
        let uid = current_uid();
        let label_path = format!("gui/{uid}/{AGENT_LABEL}");
        let bootstrapped = Command::new("/bin/launchctl")
            .args(["bootstrap", &format!("gui/{uid}"), &plist.to_string_lossy()])
            .status()
            .map_err(|error| format!("could not run launchctl: {error}"))?
            .success();
        if !bootstrapped {
            // Already bootstrapped: kickstart re-runs the mapping set.
            let kicked = Command::new("/bin/launchctl")
                .args(["kickstart", "-k", &label_path])
                .status()
                .map_err(|error| format!("could not run launchctl: {error}"))?
                .success();
            if !kicked {
                return Err("launchctl could not load the Caps Lock agent".into());
            }
        }
        Ok(())
    }
}

/// True when hidutil reports no UserKeyMapping at all: a fresh machine
/// prints the property's `(null)` placeholder, which is an empty slate,
/// not an error.
fn absent_mapping(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty() || !trimmed.contains('[')
}

/// The merged UserKeyMapping as of now. A genuinely absent mapping is an
/// empty slate; a mapping that fails to parse stays an error so foreign
/// entries are never clobbered.
fn merged_or_current() -> Result<String, String> {
    let current = hidutil_get()?;
    if absent_mapping(&current) {
        return Ok(our_pair_alone());
    }
    merge_mapping(&current, our_pair())
}

fn our_pair_alone() -> String {
    format!("[{{{}}}]", entry_json(our_pair()))
}

fn our_pair() -> (u64, u64) {
    (CAPS_USAGE.0, F18_USAGE.0)
}

/// `hidutil property --get UserKeyMapping`.
fn hidutil_get() -> Result<String, String> {
    let output = Command::new("/usr/bin/hidutil")
        .args(["property", "--get", "UserKeyMapping"])
        .output()
        .map_err(|error| format!("could not run hidutil: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "hidutil property --get failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `hidutil property --set {"UserKeyMapping": <mapping>}`.
fn hidutil_set(mapping: &str) -> Result<(), String> {
    let payload = format!(r#"{{"UserKeyMapping":{mapping}}}"#);
    let output = Command::new("/usr/bin/hidutil")
        .args(["property", "--set", &payload])
        .output()
        .map_err(|error| format!("could not run hidutil: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "hidutil property --set failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Extract the UserKeyMapping JSON array from hidutil output. The output is
/// either bare JSON or prefixed with `("UserKeyMapping") = `, so scan for the
/// array bounds instead of parsing the whole document.
fn extract_array(text: &str) -> Result<String, String> {
    let start = text
        .find('[')
        .ok_or_else(|| "no UserKeyMapping array in hidutil output".to_string())?;
    let end = text
        .rfind(']')
        .ok_or_else(|| "unterminated UserKeyMapping array".to_string())?;
    if end < start {
        return Err("malformed hidutil output".into());
    }
    Ok(text[start..=end].to_string())
}

/// Numeric JSON value reader that also accepts hex strings, because hidutil
/// documentation examples use both forms.
fn json_number(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().map(|v| v as u64)),
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if let Some(hex) = trimmed.strip_prefix("0x") {
                u64::from_str_radix(hex, 16).ok()
            } else {
                trimmed.parse().ok()
            }
        }
        _ => None,
    }
}

fn entry_src(entry: &serde_json::Map<String, serde_json::Value>) -> Option<u64> {
    json_number(entry.get(SRC_KEY)?)
}

fn entry_dst(entry: &serde_json::Map<String, serde_json::Value>) -> Option<u64> {
    json_number(entry.get(DST_KEY)?)
}

fn entries_from(text: &str) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, String> {
    let array = extract_array(text)?;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&array)
        .map_err(|error| format!("malformed UserKeyMapping: {error}"))?;
    Ok(parsed
        .into_iter()
        .filter_map(|value| match value {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .collect())
}

fn serialize(entries: &[serde_json::Map<String, serde_json::Value>]) -> String {
    let items: Vec<String> = entries
        .iter()
        .map(|entry| {
            let pairs: Vec<String> = entry
                .iter()
                .map(|(key, value)| format!("{key:?}:{value}"))
                .collect();
            format!("{{{}}}", pairs.join(","))
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// One of our entries as a JSON object, dropping any extra keys.
fn entry_json(pair: (u64, u64)) -> String {
    format!(r#""{SRC_KEY}":{},"{DST_KEY}":{}"#, pair.0, pair.1)
}

/// Merge our caps->F18 entry into the parsed UserKeyMapping: our previous
/// entries (matched on Src) are replaced, foreign entries survive untouched,
/// and the result serializes back to hidutil's array form. Pure; unit-tested.
fn merge_mapping(existing: &str, ours: (u64, u64)) -> Result<String, String> {
    let mut entries = entries_from(existing)?;
    entries.retain(|entry| entry_src(entry) != Some(ours.0));
    let mut merged = serde_json::Map::new();
    merged.insert(SRC_KEY.into(), serde_json::Value::from(ours.0));
    merged.insert(DST_KEY.into(), serde_json::Value::from(ours.1));
    entries.push(merged);
    Ok(serialize(&entries))
}

/// Every entry except the one whose Src matches `src`. Pure; unit-tested.
fn without_entry(existing: &str, src: u64) -> Result<String, String> {
    let mut entries = entries_from(existing)?;
    entries.retain(|entry| entry_src(entry) != Some(src));
    Ok(serialize(&entries))
}

/// True when some entry maps exactly `src -> dst`. Pure; unit-tested.
fn contains_pair(existing: &str, ours: (u64, u64)) -> bool {
    entries_from(existing).is_ok_and(|entries| {
        entries
            .iter()
            .any(|entry| entry_src(entry) == Some(ours.0) && entry_dst(entry) == Some(ours.1))
    })
}

/// The current user id, for `launchctl bootstrap gui/<uid>`.
fn current_uid() -> u32 {
    // SAFETY: `getuid` has no failure mode and no preconditions.
    unsafe { getuid() }
}

#[allow(non_snake_case)]
unsafe extern "C" {
    fn getuid() -> u32;
}

/// The LaunchAgent body: run the same `hidutil property --set` at login.
fn plist_xml(mapping: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/hidutil</string>
        <string>property</string>
        <string>--set</string>
        <string>{{"UserKeyMapping":{mapping}}}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
    )
}

fn bootout_agent() {
    let uid = current_uid();
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &format!("gui/{uid}/{AGENT_LABEL}")])
        .status();
}

fn delete_plist() {
    if let Some(home) = std::env::var_os("HOME") {
        let plist = std::path::Path::new(&home)
            .join("Library/LaunchAgents")
            .join(format!("{AGENT_LABEL}.plist"));
        match std::fs::remove_file(&plist) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "mojito-macos: could not delete {}: {error}",
                plist.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The real TN2450 encoding: (usage_page << 32) | usage.
    const OURS: (u64, u64) = (0x7_0000_0039, 0x7_0000_006D);

    /// The on-device discovery that forced this fix: a fresh machine prints
    /// `(null)`, and treating that as an error made the first install a
    /// silent no-op.
    #[test]
    fn absent_hidutil_output_is_an_empty_slate() {
        assert!(absent_mapping("(null)\n"));
        assert!(absent_mapping("  "));
        assert!(!absent_mapping(
            "(\"UserKeyMapping\") = [{\"HIDKeyboardModifierMappingSrc\":30064771129}]"
        ));
    }

    #[test]
    fn merge_installs_ours_into_an_empty_mapping() {
        let merged = merge_mapping("[]", OURS).unwrap();
        let entries = entries_from(&merged).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entry_src(&entries[0]), Some(OURS.0));
        assert_eq!(entry_dst(&entries[0]), Some(OURS.1));
        assert!(contains_pair(&merged, OURS));
    }

    #[test]
    fn merge_preserves_foreign_entries() {
        let existing =
            r#"[{"HIDKeyboardModifierMappingSrc":123,"HIDKeyboardModifierMappingDst":456}]"#;
        let merged = merge_mapping(existing, OURS).unwrap();
        assert!(contains_pair(&merged, OURS));
        assert!(contains_pair(&merged, (123, 456)));
    }

    #[test]
    fn merge_replaces_our_stale_entry_but_keeps_foreign_remaps_to_f18() {
        // Our old entry pointed somewhere else, and someone else remaps
        // another key TO F18: only the matching-Src entry is ours.
        let existing = format!(
            "[{{\"{SRC_KEY}\":{},\"{DST_KEY}\":777}},{{\"{SRC_KEY}\":42,\"{DST_KEY}\":{}}}]",
            OURS.0, OURS.1
        );
        let merged = merge_mapping(&existing, OURS).unwrap();
        let entries = entries_from(&merged).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(contains_pair(&merged, OURS));
        assert!(contains_pair(&merged, (42, OURS.1)));
    }

    #[test]
    fn merge_tolerates_the_hidutil_output_prefix_and_decimal_values() {
        let existing = "(\"UserKeyMapping\") = [\n  {\n    \"HIDKeyboardModifierMappingSrc\" : 30064771129,\n    \"HIDKeyboardModifierMappingDst\" : 30064771113\n  }\n]";
        let merged = merge_mapping(existing, OURS).unwrap();
        assert!(contains_pair(&merged, (30064771129, 30064771113)));
        assert!(contains_pair(&merged, OURS));
    }

    #[test]
    fn merge_accepts_hex_string_values() {
        let existing = r#"[{"HIDKeyboardModifierMappingSrc":"0x700000039","HIDKeyboardModifierMappingDst":"0x700000029"}]"#;
        let merged = merge_mapping(existing, OURS).unwrap();
        assert!(contains_pair(&merged, (0x700000039, 0x700000029)));
    }

    #[test]
    fn merge_rejects_malformed_input() {
        assert!(merge_mapping("not json at all", OURS).is_err());
        assert!(merge_mapping("[{\"HIDKeyboardModifierMappingSrc\":}]", OURS).is_err());
        assert!(merge_mapping("[", OURS).is_err());
    }

    #[test]
    fn without_entry_removes_only_the_matching_src() {
        let existing = format!(
            "[{{\"{SRC_KEY}\":{},\"{DST_KEY}\":{}}},{{\"{SRC_KEY}\":42,\"{DST_KEY}\":{}}}]",
            OURS.0, OURS.1, OURS.1
        );
        let remaining = without_entry(&existing, OURS.0).unwrap();
        assert!(!contains_pair(&remaining, OURS));
        assert!(contains_pair(&remaining, (42, OURS.1)));
    }

    #[test]
    fn contains_pair_requires_both_sides_to_match() {
        let existing = format!("[{{{}}}]", entry_json(OURS));
        assert!(contains_pair(&existing, OURS));
        assert!(!contains_pair(&existing, (OURS.0, 1)));
        assert!(!contains_pair(&existing, (1, OURS.1)));
        assert!(!contains_pair("[]", OURS));
    }

    #[test]
    fn entry_json_round_trips_through_the_parser() {
        let text = format!("[{{{}}}]", entry_json(OURS));
        let entries = entries_from(&text).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entry_src(&entries[0]), Some(OURS.0));
        assert_eq!(entry_dst(&entries[0]), Some(OURS.1));
    }

    #[test]
    fn plist_carries_the_mapping_and_runs_at_load() {
        let xml = plist_xml(&serialize(&[]));
        assert!(xml.contains("<key>RunAtLoad</key>"));
        assert!(xml.contains("/usr/bin/hidutil"));
        assert!(xml.contains("UserKeyMapping"));
        assert!(xml.contains(AGENT_LABEL));
    }
}
