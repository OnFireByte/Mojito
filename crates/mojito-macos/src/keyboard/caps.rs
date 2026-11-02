//! hidutil Caps->F18 remap: idempotent install, LaunchAgent persistence,
//! self-heal, clean uninstall that preserves foreign UserKeyMapping entries.

use std::process::Command;

use mojito_core::lifecycle::Remap;

pub struct CapsMap;

/// HID usage id in the keyboard page (the hidutil numbering space).
/// Deliberately a distinct type from `PhysicalKey`: Caps Lock is usage 0x39
/// and virtual keycode 0x39 by coincidence, but F18 is usage 0x6D and virtual
/// keycode 0x4F, and conflating the spaces breaks remaps silently.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HidUsage(pub u64);

/// HID usages for the hidutil remap. Apple TN2450 encodes a full usage as
/// (usage_page << 32) | usage, so keyboard-page values are 36-bit: caps lock
/// is 0x700000039 and F18 is 0x70000006D. u64 on purpose.
pub const CAPS_USAGE: HidUsage = HidUsage(0x7_0000_0039);
pub const F18_USAGE: HidUsage = HidUsage(0x7_0000_006D);

/// One UserKeyMapping entry as hidutil reports it.
///
/// Wire values are wider than 32 bits: hidutil's canonical encoding for a
/// keyboard-page usage is `(0x07 << 32) | usage` (Caps Lock prints as
/// 30064771129), so these are u64 on purpose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MappingEntry {
    pub src: u64,
    pub dst: u64,
}

/// The dictionary keys hidutil uses inside UserKeyMapping entries. Isolated
/// here because the exact encoding needs on-device verification.
const SRC_KEY: &str = "HIDKeyboardModifierMappingSrc";
const DST_KEY: &str = "HIDKeyboardModifierMappingDst";
const AGENT_LABEL: &str = "com.mojito.caps";

impl Remap for CapsMap {
    /// Merge our entry into the existing UserKeyMapping and set it. Idempotent.
    /// An absent mapping (a fresh machine prints `(null)`) is an empty slate;
    /// a mapping that exists but fails to parse stays an error so foreign
    /// entries are never clobbered.
    fn install(&self) -> Result<(), String> {
        set_mapping(&merged_or_current()?)?;
        if !contains_pair(&current_mapping()?, OUR_PAIR) {
            return Err("hidutil did not accept the Caps Lock remap".into());
        }
        Ok(())
    }

    /// Remove only our entry (matched on Src == CAPS_USAGE), bootout the
    /// LaunchAgent, delete the plist. A foreign entry remapping something
    /// else TO F18 survives.
    fn uninstall(&self) -> Result<(), String> {
        set_mapping(&without_entry(&current_mapping()?, CAPS_USAGE.0)?)?;
        bootout_agent();
        delete_plist();
        Ok(())
    }

    fn is_active(&self) -> bool {
        current_mapping().is_ok_and(|current| contains_pair(&current, OUR_PAIR))
    }

    /// Write and bootstrap the RunAtLoad LaunchAgent so the mapping survives
    /// reboot; Sequoia may require one Login-Items consent click. The agent
    /// carries the merged mapping as of this call, so foreign entries are
    /// replayed too; mappings added after this call are clobbered by the
    /// next login (documented v1 limitation).
    fn ensure_agent(&self) -> Result<(), String> {
        let plist = agent_plist().ok_or("no home directory; cannot install the LaunchAgent")?;
        if let Some(parent) = plist.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create LaunchAgents: {error}"))?;
        }
        std::fs::write(&plist, plist_xml(&merged_or_current()?))
            .map_err(|error| format!("could not write {}: {error}", plist.display()))?;
        let uid = getuid();
        let bootstrapped =
            launchctl(&["bootstrap", &format!("gui/{uid}"), &plist.to_string_lossy()])?;
        // Already bootstrapped: kickstart re-runs the mapping set.
        if !bootstrapped && !launchctl(&["kickstart", "-k", &format!("gui/{uid}/{AGENT_LABEL}")])? {
            return Err("launchctl could not load the Caps Lock agent".into());
        }
        Ok(())
    }
}

const OUR_PAIR: (u64, u64) = (CAPS_USAGE.0, F18_USAGE.0);

fn merged_or_current() -> Result<String, String> {
    merge_mapping(&current_mapping()?, OUR_PAIR)
}

fn current_mapping() -> Result<String, String> {
    hidutil_property("--get", "UserKeyMapping")
}

fn set_mapping(mapping: &str) -> Result<(), String> {
    hidutil_property("--set", &format!(r#"{{"UserKeyMapping":{mapping}}}"#)).map(drop)
}

fn hidutil_property(flag: &str, value: &str) -> Result<String, String> {
    let output = Command::new("/usr/bin/hidutil")
        .args(["property", flag, value])
        .output()
        .map_err(|error| format!("could not run hidutil: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "hidutil property {flag} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn launchctl(args: &[&str]) -> Result<bool, String> {
    Command::new("/bin/launchctl")
        .args(args)
        .status()
        .map(|status| status.success())
        .map_err(|error| format!("could not run launchctl: {error}"))
}

/// Parse the UserKeyMapping entries out of `hidutil property --get` output.
/// On current macOS the property prints as an old-style plist fragment,
/// `( { HIDKeyboardModifierMappingDst = 30064771181; ... } )`; some sources
/// show JSON with the same keys. An absent mapping prints `(null)` or an
/// empty string and is an empty slate, not an error. Anything else that
/// yields no readable entries stays an error so foreign remaps are never
/// clobbered.
fn entries_from(text: &str) -> Result<Vec<MappingEntry>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "(null)" {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    let mut rest = trimmed;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            return Err("unterminated UserKeyMapping entry".into());
        };
        let block = &rest[start + 1..start + end];
        rest = &rest[start + end + 1..];
        if let (Some(src), Some(dst)) = (block_value(block, SRC_KEY), block_value(block, DST_KEY)) {
            entries.push(MappingEntry { src, dst });
        }
    }
    if entries.is_empty() {
        if trimmed.contains('{') {
            return Err("could not read any UserKeyMapping entry from hidutil output".into());
        }
        // `( )`, `[]`, `(null)`, and empty all mean no custom mapping.
        return Ok(Vec::new());
    }
    Ok(entries)
}

/// The numeric value assigned to `key` inside one `{...}` entry, decimal or
/// 0x-hex. The scan is anchored on the key, so entry ordering does not
/// matter and both `=` (plist) and `:` (JSON) assignments are accepted.
fn block_value(block: &str, key: &str) -> Option<u64> {
    let tail = &block[block.find(key)? + key.len()..];
    let assign = tail.find(['=', ':'])?;
    if tail[..assign].contains([';', ',']) {
        return None;
    }
    // Values are separated from '=' by whitespace in the plist form, so
    // trim before the split or the first item is the empty prefix.
    let value = tail[assign + 1..].trim_start();
    let value = value
        .split(|c: char| c == ';' || c == ',' || c == '}' || c.is_whitespace())
        .next()?;
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

/// Our entries plus the parsed foreign entries as hidutil's `--set` input,
/// which is JSON on every version we support.
fn serialize(entries: &[MappingEntry]) -> String {
    let items: Vec<String> = entries
        .iter()
        .map(|entry| format!("{{{}}}", entry_json((entry.src, entry.dst))))
        .collect();
    format!("[{}]", items.join(","))
}

fn entry_json(pair: (u64, u64)) -> String {
    format!(r#""{SRC_KEY}":{},"{DST_KEY}":{}"#, pair.0, pair.1)
}

/// Merge our caps->F18 entry into the parsed UserKeyMapping: any previous
/// entry for the same source is replaced, foreign entries survive untouched.
fn merge_mapping(existing: &str, ours: (u64, u64)) -> Result<String, String> {
    let mut entries = entries_from(existing)?;
    entries.retain(|entry| entry.src != ours.0);
    entries.push(MappingEntry {
        src: ours.0,
        dst: ours.1,
    });
    Ok(serialize(&entries))
}

fn without_entry(existing: &str, src: u64) -> Result<String, String> {
    let mut entries = entries_from(existing)?;
    entries.retain(|entry| entry.src != src);
    Ok(serialize(&entries))
}

fn contains_pair(existing: &str, ours: (u64, u64)) -> bool {
    entries_from(existing).is_ok_and(|entries| {
        entries
            .iter()
            .any(|entry| entry.src == ours.0 && entry.dst == ours.1)
    })
}

// SAFETY: `getuid` takes no arguments, cannot fail, and has no preconditions.
unsafe extern "C" {
    safe fn getuid() -> u32;
}

fn agent_plist() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::Path::new(&home)
            .join("Library/LaunchAgents")
            .join(format!("{AGENT_LABEL}.plist")),
    )
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
    let _ = launchctl(&["bootout", &format!("gui/{}/{AGENT_LABEL}", getuid())]);
}

fn delete_plist() {
    let Some(plist) = agent_plist() else {
        return;
    };
    match std::fs::remove_file(&plist) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!(
            "mojito-macos: could not delete {}: {error}",
            plist.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The real TN2450 encoding: (usage_page << 32) | usage.
    const OURS: (u64, u64) = (0x7_0000_0039, 0x7_0000_006D);

    /// The trap the architecture doc warns about: keycode 0x4F and HID usage
    /// 0x70000006D both name F18, in different spaces. The TN2450 encoding is
    /// (usage_page << 32) | usage, so the usage constants are 36-bit.
    #[test]
    fn keycode_spaces_are_distinct_and_golden() {
        assert_eq!(mojito_core::key::META, mojito_core::PhysicalKey(0x4F));
        assert_eq!(F18_USAGE, HidUsage(0x7_0000_006D));
        assert_eq!(CAPS_USAGE, HidUsage(0x7_0000_0039));
        assert_eq!(F18_USAGE.0, 30064771181);
        assert_eq!(CAPS_USAGE.0, 30064771129);
        assert_ne!(mojito_core::key::META.0 as u64, F18_USAGE.0);
    }

    /// The on-device discovery that forced these tests: a fresh machine
    /// prints `(null)`, and the first parser treated that as an error, so
    /// the first install was a silent no-op.
    #[test]
    fn absent_output_is_an_empty_slate() {
        assert_eq!(entries_from("(null)\n"), Ok(vec![]));
        assert_eq!(entries_from("  "), Ok(vec![]));
        assert_eq!(entries_from("( \n)"), Ok(vec![]));
        assert_eq!(entries_from("[]"), Ok(vec![]));
    }

    /// The exact output captured on this machine (macOS 15, old-style plist
    /// fragment with the destination listed first).
    #[test]
    fn parses_the_on_device_plist_format() {
        let text = "(\n        {\n        HIDKeyboardModifierMappingDst = 30064771181;\n        HIDKeyboardModifierMappingSrc = 30064771129;\n    }\n)";
        let entries = entries_from(text).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].src, 30064771129);
        assert_eq!(entries[0].dst, 30064771181);
        assert!(contains_pair(text, OURS));
    }

    #[test]
    fn parses_multi_entry_plist_with_hex_values() {
        let text = "( { HIDKeyboardModifierMappingDst = 0x70000006D; HIDKeyboardModifierMappingSrc = 0x70000002A; }, { HIDKeyboardModifierMappingDst = 5; HIDKeyboardModifierMappingSrc = 6; } )";
        let entries = entries_from(text).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0],
            MappingEntry {
                src: 0x70000002A,
                dst: 0x70000006D
            }
        );
        assert_eq!(entries[1], MappingEntry { src: 6, dst: 5 });
    }

    #[test]
    fn parses_the_json_form_with_colon_assignment() {
        let text = r#"[{"HIDKeyboardModifierMappingSrc":30064771129,"HIDKeyboardModifierMappingDst":30064771181}]"#;
        assert!(contains_pair(text, OURS));
    }

    #[test]
    fn merge_installs_ours_into_an_empty_mapping() {
        let merged = merge_mapping("[]", OURS).unwrap();
        let entries = entries_from(&merged).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            MappingEntry {
                src: OURS.0,
                dst: OURS.1
            }
        );
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
    fn merge_replaces_a_foreign_caps_lock_remap() {
        // A pre-existing remap OF caps lock is replaced by ours; that is the
        // product's premise. The plist form is what this machine prints.
        let existing = "( {\n    HIDKeyboardModifierMappingDst = 30064771113;\n    HIDKeyboardModifierMappingSrc = 30064771129;\n} )";
        let merged = merge_mapping(existing, OURS).unwrap();
        assert!(contains_pair(&merged, OURS));
        assert!(!contains_pair(&merged, (OURS.0, 30064771113)));
    }

    #[test]
    fn merge_rejects_malformed_input() {
        assert!(merge_mapping("not a mapping at all {", OURS).is_err());
        assert!(merge_mapping("{ HIDKeyboardModifierMappingSrc = }", OURS).is_err());
        assert!(merge_mapping("{ unterminated", OURS).is_err());
    }

    #[test]
    fn without_entry_removes_only_the_matching_src() {
        let existing = "( { HIDKeyboardModifierMappingDst = 30064771181; HIDKeyboardModifierMappingSrc = 30064771129; }, { HIDKeyboardModifierMappingDst = 9; HIDKeyboardModifierMappingSrc = 42; } )";
        let remaining = without_entry(existing, OURS.0).unwrap();
        assert!(!contains_pair(&remaining, OURS));
        assert!(contains_pair(&remaining, (42, 9)));
    }

    #[test]
    fn contains_pair_requires_both_sides_to_match() {
        let existing = format!("[{{{}}}]", entry_json(OURS));
        assert!(contains_pair(&existing, OURS));
        assert!(!contains_pair(&existing, (OURS.0, 1)));
        assert!(!contains_pair(&existing, (1, OURS.1)));
        assert!(!contains_pair("( )", OURS));
    }

    #[test]
    fn serialize_round_trips_through_the_parser() {
        let entries = vec![
            MappingEntry {
                src: OURS.0,
                dst: OURS.1,
            },
            MappingEntry { src: 42, dst: 43 },
        ];
        let parsed = entries_from(&serialize(&entries)).unwrap();
        assert_eq!(parsed, entries);
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
