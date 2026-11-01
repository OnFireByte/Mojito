//! Config model and boundary parsing. Parse at the boundary, trust inside;
//! this is the only place config errors exist.

use crate::key::PhysicalKey;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Config {
    pub pinned: Vec<PinnedApp>,
    pub tap_max_ms: u16,
    pub capture_enabled: bool,
    pub label_memory: crate::labels::LabelMemory,
    /// Bumped on every accepted save so the engine can rebuild tables lazily.
    pub version: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            tap_max_ms: 250,
            capture_enabled: true,
            label_memory: crate::labels::LabelMemory::default(),
            version: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PinnedApp {
    pub name: String,
    pub bundle_id: String,
    pub key: PhysicalKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning(pub String);

/// Parse settings JSON with hex's recovery rules: unknown keys ignored; an
/// invalid field defaults in memory with a warning; duplicate trigger keys
/// resolve first-wins with a warning; reserved keys rejected with a warning.
/// Never fails hard: a corrupt file yields defaults and warnings. Empty input
/// is a fresh install, not corruption.
pub fn parse(bytes: &[u8]) -> (Config, Vec<Warning>) {
    let mut warnings = Vec::new();
    if bytes.is_empty() {
        return (Config::default(), warnings);
    }
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(Warning(format!(
                "settings: not valid JSON ({e}); loaded defaults"
            )));
            return (Config::default(), warnings);
        }
    };
    let Some(obj) = value.as_object() else {
        warnings.push(Warning(
            "settings: not a JSON object; loaded defaults".into(),
        ));
        return (Config::default(), warnings);
    };

    let mut config = Config::default();
    if let Some(v) = obj.get("tap_max_ms") {
        match v.as_u64() {
            Some(n) if n <= u16::MAX as u64 => config.tap_max_ms = n as u16,
            _ => warnings.push(Warning("tap_max_ms: invalid value; using 250".into())),
        }
    }
    if let Some(v) = obj.get("capture_enabled") {
        match v.as_bool() {
            Some(b) => config.capture_enabled = b,
            None => warnings.push(Warning("capture_enabled: invalid value; using true".into())),
        }
    }
    if let Some(v) = obj.get("version") {
        match v.as_u64() {
            Some(n) => config.version = n,
            None => warnings.push(Warning("version: invalid value; ignored".into())),
        }
    }
    if let Some(v) = obj.get("pinned") {
        match v.as_array() {
            None => warnings.push(Warning("pinned: not a list; ignored".into())),
            Some(entries) => {
                let mut seen = std::collections::HashSet::new();
                for (i, entry) in entries.iter().enumerate() {
                    let Some(entry) = entry.as_object() else {
                        warnings.push(Warning(format!("pinned[{i}]: not an object; skipped")));
                        continue;
                    };
                    let name = entry.get("name").and_then(|v| v.as_str());
                    let bundle = entry.get("bundle_id").and_then(|v| v.as_str());
                    let key = entry.get("key").and_then(key_from_json);
                    let (Some(name), Some(bundle), Some(key)) = (name, bundle, key) else {
                        warnings.push(Warning(format!(
                            "pinned[{i}]: missing or invalid field; skipped"
                        )));
                        continue;
                    };
                    if crate::labels::RESERVED.contains(&key) {
                        warnings.push(Warning(format!(
                            "pinned[{i}]: key {key:?} is reserved; skipped"
                        )));
                        continue;
                    }
                    if !seen.insert(key) {
                        warnings.push(Warning(format!(
                            "pinned[{i}]: duplicate trigger key {key:?}; first wins"
                        )));
                        continue;
                    }
                    config.pinned.push(PinnedApp {
                        name: name.into(),
                        bundle_id: bundle.into(),
                        key,
                    });
                }
            }
        }
    }
    if let Some(v) = obj.get("label_memory") {
        match v.as_object() {
            Some(map) => {
                for (bundle, key) in map {
                    match key_from_json(key) {
                        Some(key) if !crate::labels::RESERVED.contains(&key) => {
                            config.label_memory.0.insert(bundle.clone(), key);
                        }
                        _ => warnings.push(Warning(format!(
                            "label_memory[{bundle}]: invalid or reserved key; skipped"
                        ))),
                    }
                }
            }
            None if !v.is_null() => {
                warnings.push(Warning("label_memory: not an object; ignored".into()))
            }
            None => {}
        }
    }
    (config, warnings)
}

/// Keys arrive as virtual keycodes: a JSON number, or a hex string
/// ("0x2E" or "2E") from hand-edited files. Anything else is invalid.
fn key_from_json(value: &serde_json::Value) -> Option<PhysicalKey> {
    if let Some(n) = value.as_u64() {
        return u16::try_from(n).ok().map(PhysicalKey);
    }
    let s = value.as_str()?.trim();
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u16::from_str_radix(digits, 16).ok().map(PhysicalKey)
}

/// Boundary validation for a draft from the settings UI: duplicate keys
/// first-wins, reserved keys rejected, name and bundle required. The returned
/// config is the validated draft (dropped entries removed), warnings explain
/// every drop.
pub fn validate(draft: Config) -> (Config, Vec<Warning>) {
    let mut warnings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut pinned = Vec::with_capacity(draft.pinned.len());
    for (i, app) in draft.pinned.into_iter().enumerate() {
        if app.name.trim().is_empty() || app.bundle_id.trim().is_empty() {
            warnings.push(Warning(format!(
                "pinned[{i}]: name and bundle_id are required; skipped"
            )));
            continue;
        }
        if crate::labels::RESERVED.contains(&app.key) {
            warnings.push(Warning(format!(
                "pinned[{i}]: key {:?} is reserved; skipped",
                app.key
            )));
            continue;
        }
        if !seen.insert(app.key) {
            warnings.push(Warning(format!(
                "pinned[{i}]: duplicate trigger key {:?}; first wins",
                app.key
            )));
            continue;
        }
        pinned.push(app);
    }
    (Config { pinned, ..draft }, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{ESC, F18, M, RETURN};
    use crate::labels::LabelMemory;

    fn parse_str(json: &str) -> (Config, Vec<Warning>) {
        parse(json.as_bytes())
    }

    #[test]
    fn empty_input_is_a_fresh_install() {
        let (config, warnings) = parse(b"");
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn corrupt_json_yields_defaults_with_a_warning() {
        let (config, warnings) = parse(b"{not json");
        assert_eq!(config, Config::default());
        assert!(warnings.iter().any(|w| w.0.contains("not valid JSON")));
    }

    #[test]
    fn non_object_json_yields_defaults_with_a_warning() {
        let (config, warnings) = parse_str("[1,2,3]");
        assert_eq!(config, Config::default());
        assert!(warnings.iter().any(|w| w.0.contains("not a JSON object")));
    }

    #[test]
    fn unknown_keys_are_ignored_silently() {
        let (config, warnings) = parse_str(r#"{"wat": 1, "also_wat": {"x": 2}}"#);
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn invalid_fields_default_with_warnings() {
        let (config, warnings) =
            parse_str(r#"{"tap_max_ms": "fast", "capture_enabled": "yes", "version": "x"}"#);
        assert_eq!(config.tap_max_ms, 250);
        assert!(config.capture_enabled);
        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn valid_scalars_are_accepted() {
        let (config, warnings) =
            parse_str(r#"{"tap_max_ms": 300, "capture_enabled": false, "version": 7}"#);
        assert_eq!(config.tap_max_ms, 300);
        assert!(!config.capture_enabled);
        assert_eq!(config.version, 7);
        assert!(warnings.is_empty());
    }

    #[test]
    fn key_encodings_number_hex_string_bare_hex() {
        let json = r#"{"pinned": [
            {"name": "a", "bundle_id": "b.a", "key": 10},
            {"name": "b", "bundle_id": "b.b", "key": "0x18"},
            {"name": "c", "bundle_id": "b.c", "key": "1E"}
        ]}"#;
        let (config, warnings) = parse_str(json);
        assert!(warnings.is_empty());
        assert_eq!(config.pinned[0].key, PhysicalKey(0x0A));
        assert_eq!(config.pinned[1].key, PhysicalKey(0x18));
        assert_eq!(config.pinned[2].key, PhysicalKey(0x1E));
    }

    #[test]
    fn invalid_pinned_entries_are_skipped_with_warnings() {
        let json = r#"{"pinned": [
            {"name": "ok", "bundle_id": "b.ok", "key": 3},
            {"name": "nokey", "bundle_id": "b.nk"},
            {"name": 5, "bundle_id": "b.bn", "key": 4},
            "junk"
        ]}"#;
        let (config, warnings) = parse_str(json);
        assert_eq!(config.pinned.len(), 1);
        assert_eq!(config.pinned[0].key, PhysicalKey(3));
        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn duplicate_trigger_keys_resolve_first_wins() {
        let json = r#"{"pinned": [
            {"name": "one", "bundle_id": "b.one", "key": 3},
            {"name": "two", "bundle_id": "b.two", "key": 3}
        ]}"#;
        let (config, warnings) = parse_str(json);
        assert_eq!(config.pinned.len(), 1);
        assert_eq!(config.pinned[0].name, "one");
        assert!(warnings.iter().any(|w| w.0.contains("duplicate")));
    }

    #[test]
    fn reserved_keys_are_rejected_with_a_warning() {
        let json = r#"{"pinned": [
            {"name": "m", "bundle_id": "b.m", "key": 46},
            {"name": "esc", "bundle_id": "b.esc", "key": 53},
            {"name": "ok", "bundle_id": "b.ok", "key": 3}
        ]}"#;
        let (config, warnings) = parse_str(json);
        assert_eq!(config.pinned.len(), 1);
        assert_eq!(config.pinned[0].key, PhysicalKey(3));
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn label_memory_parses_and_drops_bad_entries() {
        let json = r#"{"label_memory": {"com.a": 3, "com.b": "1E", "com.c": "zz", "com.d": 46}}"#;
        let (config, warnings) = parse_str(json);
        assert_eq!(config.label_memory.0.get("com.a"), Some(&PhysicalKey(3)));
        assert_eq!(config.label_memory.0.get("com.b"), Some(&PhysicalKey(0x1E)));
        assert_eq!(config.label_memory.0.get("com.c"), None);
        assert_eq!(config.label_memory.0.get("com.d"), None); // m is reserved
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn round_trip_preserves_config() {
        let original = Config {
            pinned: vec![PinnedApp {
                name: "Finder".into(),
                bundle_id: "com.apple.finder".into(),
                key: PhysicalKey(0x18),
            }],
            tap_max_ms: 180,
            capture_enabled: false,
            label_memory: LabelMemory([("com.a".into(), PhysicalKey(3))].into_iter().collect()),
            version: 12,
        };
        let bytes = serde_json::to_string_pretty(&original).unwrap();
        let (parsed, warnings) = parse(bytes.as_bytes());
        assert!(warnings.is_empty());
        assert_eq!(parsed.pinned, original.pinned);
        assert_eq!(parsed.tap_max_ms, 180);
        assert!(!parsed.capture_enabled);
        assert_eq!(parsed.label_memory.0, original.label_memory.0);
        assert_eq!(parsed.version, 12);
    }

    #[test]
    fn validate_enforces_uniqueness_reserved_and_required_fields() {
        let draft = Config {
            pinned: vec![
                PinnedApp {
                    name: "ok".into(),
                    bundle_id: "b.ok".into(),
                    key: PhysicalKey(3),
                },
                PinnedApp {
                    name: "dupe".into(),
                    bundle_id: "b.dupe".into(),
                    key: PhysicalKey(3),
                },
                PinnedApp {
                    name: "res".into(),
                    bundle_id: "b.res".into(),
                    key: M,
                },
                PinnedApp {
                    name: "".into(),
                    bundle_id: "b.empty".into(),
                    key: PhysicalKey(4),
                },
                PinnedApp {
                    name: "nobundle".into(),
                    bundle_id: "".into(),
                    key: PhysicalKey(5),
                },
            ],
            ..Default::default()
        };
        let (config, warnings) = validate(draft);
        assert_eq!(config.pinned.len(), 1);
        assert_eq!(config.pinned[0].key, PhysicalKey(3));
        assert_eq!(warnings.len(), 4);
    }

    #[test]
    fn validate_rejects_every_reserved_key() {
        for key in [F18, ESC, RETURN, M] {
            let draft = Config {
                pinned: vec![PinnedApp {
                    name: "x".into(),
                    bundle_id: "b.x".into(),
                    key,
                }],
                ..Default::default()
            };
            let (config, warnings) = validate(draft);
            assert!(config.pinned.is_empty());
            assert_eq!(warnings.len(), 1);
        }
    }
}
