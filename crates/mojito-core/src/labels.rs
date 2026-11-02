//! Label assignment: deterministic, injective, Colemak-DH reach order.

use crate::bridge::AppSlot;
use crate::key::{ESC, F18, PhysicalKey, RETURN, SWITCHER, TAB};
use std::collections::HashSet;

/// QWERTY-position virtual keycodes, best reach first. The first ten are the
/// Colemak-DH "ten easiest keys": the eight outer home-row positions plus the
/// two bottom-row index-curl positions where the mod places D and H. The home
/// center columns (G/H positions) rank below the strong top-row keys because
/// demoting them is the whole point of the mod. Order within a tier is a
/// product knob; tests pin tier membership and determinism.
pub const LABEL_PRIORITY: [PhysicalKey; 26] = [
    PhysicalKey(0x03), // f  home, left index
    PhysicalKey(0x02), // d  home, left middle
    PhysicalKey(0x01), // s  home, left ring
    PhysicalKey(0x00), // a  home, left pinky
    PhysicalKey(0x26), // j  home, right index
    PhysicalKey(0x28), // k  home, right middle
    PhysicalKey(0x25), // l  home, right ring
    PhysicalKey(0x29), // ;  home, right pinky
    PhysicalKey(0x09), // v  bottom index curl (DH's D position)
    PhysicalKey(0x2E), // m  bottom index curl (DH's H position)
    PhysicalKey(0x11), // t  top, left index
    PhysicalKey(0x10), // y  top, right index
    PhysicalKey(0x20), // u  top, right middle
    PhysicalKey(0x0F), // r  top, left ring
    PhysicalKey(0x22), // i  top, right ring
    PhysicalKey(0x05), // g  home center, demoted by DH
    PhysicalKey(0x04), // h  home center, demoted by DH
    PhysicalKey(0x0B), // b  bottom, left index neighbor
    PhysicalKey(0x2D), // n  bottom, right index neighbor
    PhysicalKey(0x08), // c  bottom, left middle
    PhysicalKey(0x07), // x  bottom, left ring
    PhysicalKey(0x06), // z  bottom, left pinky
    PhysicalKey(0x0C), // q  top, left pinky
    PhysicalKey(0x0D), // w  top, left ring-outer
    PhysicalKey(0x1F), // o  top, right ring-outer
    PhysicalKey(0x23), // p  top, right pinky
];

/// Keys no label or pin may ever take. Enforced in config validation and in
/// label assignment; single source of truth.
pub const RESERVED: [PhysicalKey; 5] = [F18, ESC, RETURN, SWITCHER, TAB];

/// Sticky label memory: an app keeps its last label when that key is free
/// this activation, so labels stay consistent across activations.
#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LabelMemory(pub std::collections::BTreeMap<String, PhysicalKey>);

/// Assign labels to rows in the given (recency) order. Walk LABEL_PRIORITY
/// skipping pinned trigger keys; grant a sticky remembered label first when
/// free; rows beyond the pool get no label. Returns one entry per input row,
/// in order.
pub fn assign_labels(
    rows: &[AppSlot],
    memory: &LabelMemory,
    pinned: &[PhysicalKey],
) -> Vec<Option<PhysicalKey>> {
    let mut used: HashSet<PhysicalKey> = HashSet::new();
    let mut cursor = 0usize;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let mut granted = None;
        if let Some(bundle) = &row.bundle_id
            && let Some(&key) = memory.0.get(bundle)
            && !RESERVED.contains(&key)
            && !pinned.contains(&key)
            && used.insert(key)
        {
            granted = Some(key);
        }
        if granted.is_none() {
            while cursor < LABEL_PRIORITY.len() {
                let key = LABEL_PRIORITY[cursor];
                cursor += 1;
                if pinned.contains(&key) || used.contains(&key) {
                    continue;
                }
                used.insert(key);
                granted = Some(key);
                break;
            }
        }
        result.push(granted);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(pid: i32, bundle: Option<&str>) -> AppSlot {
        AppSlot {
            pid,
            bundle_id: bundle.map(String::from),
            name: format!("app{pid}"),
            windows: 1,
            is_frontmost: false,
        }
    }

    #[test]
    fn assignment_is_deterministic_and_injective() {
        let rows: Vec<AppSlot> = (0..30).map(|i| slot(i, Some(&format!("b{i}")))).collect();
        let a = assign_labels(&rows, &LabelMemory::default(), &[]);
        let b = assign_labels(&rows, &LabelMemory::default(), &[]);
        assert_eq!(a, b);
        let labeled: Vec<PhysicalKey> = a.iter().copied().flatten().collect();
        assert_eq!(
            labeled.len(),
            HashSet::<PhysicalKey>::from_iter(labeled.iter().copied()).len()
        );
    }

    #[test]
    fn priority_walk_pins_tier_membership() {
        // Rows in recency order take the pool in reach order: the DH home
        // outer eight and bottom curl first, demoted G/H after the strong
        // top-row keys, pinky corners last.
        let rows: Vec<AppSlot> = (0..26).map(|i| slot(i, Some(&format!("b{i}")))).collect();
        let labels = assign_labels(&rows, &LabelMemory::default(), &[]);
        let got: Vec<PhysicalKey> = labels.iter().copied().flatten().collect();
        assert_eq!(got, LABEL_PRIORITY);
        // The reserved keys never appear even with demand for every key.
        for reserved in [F18, ESC, RETURN, SWITCHER] {
            assert!(!got.contains(&reserved));
        }
    }

    #[test]
    fn overflow_rows_go_unlabeled() {
        let rows: Vec<AppSlot> = (0..30).map(|i| slot(i, Some(&format!("b{i}")))).collect();
        let labels = assign_labels(&rows, &LabelMemory::default(), &[]);
        assert_eq!(labels.iter().flatten().count(), 26);
        assert!(labels[26..].iter().all(|l| l.is_none()));
    }

    #[test]
    fn pinned_trigger_keys_are_skipped() {
        let rows = [slot(1, Some("a")), slot(2, Some("b"))];
        let f = LABEL_PRIORITY[0];
        let labels = assign_labels(&rows, &LabelMemory::default(), &[f]);
        assert_eq!(labels[0], Some(LABEL_PRIORITY[1])); // d, f taken by the pin
    }

    #[test]
    fn sticky_memory_is_honored_when_free() {
        let rows = [slot(1, Some("b1")), slot(2, Some("b2"))];
        let j = PhysicalKey(0x26);
        let memory = LabelMemory([("b2".into(), j)].into_iter().collect());
        let labels = assign_labels(&rows, &memory, &[]);
        assert_eq!(labels[1], Some(j));
        assert_eq!(labels[0], Some(LABEL_PRIORITY[0])); // b1 still opens the pool
    }

    #[test]
    fn sticky_memory_is_skipped_when_taken() {
        let rows = [slot(1, Some("b1")), slot(2, Some("b2"))];
        let j = PhysicalKey(0x26);
        let memory = LabelMemory([("b1".into(), j), ("b2".into(), j)].into_iter().collect());
        let labels = assign_labels(&rows, &memory, &[]);
        assert_eq!(labels[0], Some(j)); // earlier row wins the remembered key
        assert_eq!(labels[1], Some(LABEL_PRIORITY[0])); // b2 falls back to the pool
    }

    #[test]
    fn memory_pointing_at_reserved_or_pinned_keys_is_ignored() {
        let rows = [slot(1, Some("b1")), slot(2, Some("b2"))];
        let f = LABEL_PRIORITY[0];
        let memory = LabelMemory(
            [("b1".into(), SWITCHER), ("b2".into(), f)]
                .into_iter()
                .collect(),
        );
        let labels = assign_labels(&rows, &memory, &[f]);
        assert_eq!(labels[0], Some(LABEL_PRIORITY[1])); // e reserved, f pinned -> d
        assert_eq!(labels[1], Some(LABEL_PRIORITY[2])); // f pinned again -> s
    }

    #[test]
    fn rows_without_bundle_take_the_pool_in_order() {
        let rows = [slot(1, None), slot(2, Some("b"))];
        let labels = assign_labels(&rows, &LabelMemory::default(), &[]);
        assert_eq!(labels[0], Some(LABEL_PRIORITY[0]));
        assert_eq!(labels[1], Some(LABEL_PRIORITY[1]));
    }
}
