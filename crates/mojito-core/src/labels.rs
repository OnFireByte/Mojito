//! Label assignment: deterministic, injective, Colemak-DH reach order.

use crate::bridge::AppSlot;
use crate::key::PhysicalKey;

/// QWERTY-position virtual keycodes, best reach first. The first ten are the
/// Colemak-DH "ten easiest keys": the eight outer home-row positions plus the
/// two bottom-row index-curl positions where the mod places D and H. The home
/// center columns (G/H positions) rank below the strong top-row keys because
/// demoting them is the whole point of the mod. Order within a tier is a
/// product knob; tests pin tier membership and determinism.
pub const LABEL_PRIORITY: [PhysicalKey; 27] = [
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
    PhysicalKey(0x0E), // e  top, left middle
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
pub const RESERVED: [PhysicalKey; 4] = [
    PhysicalKey(0x4F), // F18: the meta key itself
    PhysicalKey(0x35), // esc: cancel
    PhysicalKey(0x24), // return: reserved for a future confirm
    PhysicalKey(0x2E), // m: the mode toggle
];

/// Sticky label memory: an app keeps its last label when that key is free
/// this activation, so labels stay consistent across activations.
#[derive(Default, Debug, Clone)]
pub struct LabelMemory(pub std::collections::BTreeMap<String, PhysicalKey>);

/// Assign labels to rows in the given (recency) order. Walk LABEL_PRIORITY
/// skipping RESERVED and pinned trigger keys; grant a sticky remembered label
/// first when free; rows beyond the pool get no label.
pub fn assign_labels(
    rows: &[AppSlot],
    memory: &LabelMemory,
    pinned: &[PhysicalKey],
) -> Vec<Option<PhysicalKey>> {
    let _ = (rows, memory, pinned);
    todo!("assign_labels")
}
