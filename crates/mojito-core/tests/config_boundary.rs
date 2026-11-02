use mojito_core::config::parse;
use mojito_core::tables::{Chord, build_tables};
use mojito_core::{Config, PhysicalKey, PinnedApp};

#[test]
fn out_of_range_pinned_key_is_dropped_without_panicking_table_build() {
    let (config, warnings) = parse(
        br#"{"pinned": [
            {"name": "far", "bundle_id": "b.far", "key": 200},
            {"name": "ok", "bundle_id": "b.ok", "key": 3}
        ]}"#,
    );
    assert_eq!(config.pinned.len(), 1);
    assert_eq!(config.pinned[0].key, PhysicalKey(3));
    assert!(warnings.iter().any(|w| w.0.starts_with("pinned[0]")));
    assert_eq!(build_tables(&config).pinned[3], Some(Chord::TriggerPin(0)));

    let unvalidated = Config {
        pinned: vec![PinnedApp {
            name: "far".into(),
            bundle_id: "b.far".into(),
            key: PhysicalKey(200),
        }],
        ..Default::default()
    };
    assert!(
        build_tables(&unvalidated)
            .pinned
            .iter()
            .all(|chord| !matches!(chord, Some(Chord::TriggerPin(_))))
    );
}
