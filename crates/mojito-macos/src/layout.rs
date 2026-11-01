//! Layout snapshot: UCKeyTranslate over keycodes 0..128, taken once on the
//! main thread and refreshed on input-source change. Physical key -> glyph
//! for badges only; TIS access stays behind one mutex.

/// Snapshot the current layout's keycode-to-glyph table.
pub fn snapshot() {
    todo!("layout::snapshot")
}

/// Display glyph for a physical position under the snapshotted layout.
pub fn glyph(key: mojito_core::PhysicalKey) -> Option<char> {
    let _ = key;
    todo!("layout::glyph")
}
