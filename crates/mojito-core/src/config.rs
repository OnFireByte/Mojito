//! Config model and boundary parsing. Parse at the boundary, trust inside;
//! this is the only place config errors exist.

use crate::key::PhysicalKey;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Config {
    pub pinned: Vec<PinnedApp>,
    pub tap_max_ms: u16,
    pub capture_enabled: bool,
    pub label_memory: crate::labels::LabelMemory,
    /// Which installed layout renders hint glyphs (InputSourceID); None
    /// follows the current input source.
    pub hint_layout: Option<String>,
    pub switcher_trigger: SwitcherTrigger,
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
            hint_layout: None,
            switcher_trigger: SwitcherTrigger::Both,
            version: 0,
        }
    }
}

/// Which gesture opens the switcher: Caps+E, Cmd+Tab (taking it over from
/// macOS), or either.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitcherTrigger {
    CapsE,
    CmdTab,
    Both,
}

impl SwitcherTrigger {
    pub const ALL: [Self; 3] = [Self::CapsE, Self::CmdTab, Self::Both];

    pub fn caps_e(self) -> bool {
        matches!(self, Self::CapsE | Self::Both)
    }

    pub fn cmd_tab(self) -> bool {
        matches!(self, Self::CmdTab | Self::Both)
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

/// Parse settings JSON with hex's recovery rules: unknown keys ignored, an
/// invalid field keeps its default with a warning, pins go through `admit`.
/// Never fails hard: a corrupt file yields defaults and warnings. Empty input
/// is a fresh install, not corruption.
pub fn parse(bytes: &[u8]) -> (Config, Vec<Warning>) {
    let mut warnings = Vec::new();
    if bytes.is_empty() {
        return (Config::default(), warnings);
    }
    let value: Value = match serde_json::from_slice(bytes) {
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
    warnings.extend(
        [
            read_field(obj, "tap_max_ms", &mut config.tap_max_ms),
            read_field(obj, "capture_enabled", &mut config.capture_enabled),
            read_field(obj, "hint_layout", &mut config.hint_layout),
            read_field(obj, "switcher_trigger", &mut config.switcher_trigger),
            read_field(obj, "version", &mut config.version),
        ]
        .into_iter()
        .flatten(),
    );
    if let Some(v) = obj.get("pinned") {
        match v.as_array() {
            None => warnings.push(Warning("pinned: not a list; ignored".into())),
            Some(entries) => {
                config.pinned = admit(entries.iter().map(pinned_from_json), &mut warnings)
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

fn read_field<T: DeserializeOwned>(
    obj: &Map<String, Value>,
    key: &str,
    slot: &mut T,
) -> Option<Warning> {
    match T::deserialize(obj.get(key)?) {
        Ok(parsed) => {
            *slot = parsed;
            None
        }
        Err(_) => Some(Warning(format!("{key}: invalid value; using the default"))),
    }
}

fn pinned_from_json(entry: &Value) -> Result<PinnedApp, String> {
    let entry = entry.as_object().ok_or("not an object; skipped")?;
    let text = |field| entry.get(field).and_then(Value::as_str);
    let key = entry.get("key").and_then(key_from_json);
    match (text("name"), text("bundle_id"), key) {
        (Some(name), Some(bundle), Some(key)) => Ok(PinnedApp {
            name: name.into(),
            bundle_id: bundle.into(),
            key,
        }),
        _ => Err("missing or invalid field; skipped".into()),
    }
}

/// Keys arrive as virtual keycodes: a JSON number, or a hex string
/// ("0x2E" or "2E") from hand-edited files. Anything else is invalid.
fn key_from_json(value: &Value) -> Option<PhysicalKey> {
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

/// The pinned-app rules shared by file parsing and settings validation:
/// reserved and out-of-range keys rejected, duplicate keys first-wins.
/// Every drop leaves a warning.
fn admit(
    candidates: impl Iterator<Item = Result<PinnedApp, String>>,
    warnings: &mut Vec<Warning>,
) -> Vec<PinnedApp> {
    let mut seen = HashSet::new();
    let mut admitted = Vec::new();
    for (i, candidate) in candidates.enumerate() {
        let verdict = candidate.and_then(|app| {
            if let Some(reason) = unbindable(app.key) {
                Err(format!("key {:?} {reason}; skipped", app.key))
            } else if !seen.insert(app.key) {
                Err(format!("duplicate trigger key {:?}; first wins", app.key))
            } else {
                Ok(app)
            }
        });
        match verdict {
            Ok(app) => admitted.push(app),
            Err(reason) => warnings.push(Warning(format!("pinned[{i}]: {reason}"))),
        }
    }
    admitted
}

fn unbindable(key: PhysicalKey) -> Option<&'static str> {
    if crate::labels::RESERVED.contains(&key) {
        Some("is reserved")
    } else if usize::from(key.0) >= crate::tables::TABLE_SLOTS {
        Some("is outside the bindable key range")
    } else {
        None
    }
}

/// Boundary validation for a draft from the settings UI: `admit`'s rules,
/// plus a name and bundle id on every pin.
pub fn validate(draft: Config) -> (Config, Vec<Warning>) {
    let mut warnings = Vec::new();
    let named = draft.pinned.into_iter().map(|app| {
        if app.name.trim().is_empty() || app.bundle_id.trim().is_empty() {
            Err("name and bundle_id are required; skipped".to_string())
        } else {
            Ok(app)
        }
    });
    let pinned = admit(named, &mut warnings);
    (Config { pinned, ..draft }, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{ESC, META, RETURN, SWITCHER};
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
            {"name": "e", "bundle_id": "b.e", "key": 14},
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
        let json = r#"{"label_memory": {"com.a": 3, "com.b": "1E", "com.c": "zz", "com.d": 14}}"#;
        let (config, warnings) = parse_str(json);
        assert_eq!(config.label_memory.0.get("com.a"), Some(&PhysicalKey(3)));
        assert_eq!(config.label_memory.0.get("com.b"), Some(&PhysicalKey(0x1E)));
        assert_eq!(config.label_memory.0.get("com.c"), None);
        assert_eq!(config.label_memory.0.get("com.d"), None); // e is reserved
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
            hint_layout: None,
            switcher_trigger: SwitcherTrigger::CmdTab,
            version: 12,
        };
        let bytes = serde_json::to_string_pretty(&original).unwrap();
        let (parsed, warnings) = parse(bytes.as_bytes());
        assert!(warnings.is_empty());
        assert_eq!(parsed.pinned, original.pinned);
        assert_eq!(parsed.tap_max_ms, 180);
        assert!(!parsed.capture_enabled);
        assert_eq!(parsed.label_memory.0, original.label_memory.0);
        assert_eq!(parsed.switcher_trigger, SwitcherTrigger::CmdTab);
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
                    key: SWITCHER,
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
        for key in [META, ESC, RETURN, SWITCHER] {
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
