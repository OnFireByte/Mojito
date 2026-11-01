//! The only engine-to-UI vocabulary. Full small snapshots, not diffs; the
//! overlay replaces state wholesale.

use crate::key::PhysicalKey;
use std::sync::Arc;

#[derive(Debug, PartialEq)]
pub struct PinnedRow {
    pub key: PhysicalKey,
    pub name: String,
    pub glyph: char,
}

/// Frozen when Live mode is entered; never mutated during the hold, so
/// labels cannot reshuffle under the user's fingers.
#[derive(Debug)]
pub struct LiveModel {
    pub rows: Arc<[LiveRow]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveRow {
    pub pid: i32,
    pub bundle: Option<String>,
    pub name: String,
    pub windows: u8,
    pub label: Option<PhysicalKey>,
}

/// Recency-ordered running apps, published by the main thread. Window counts
/// refresh when Pinned mode opens so Live mode is instant.
#[derive(Clone, Default, Debug)]
pub struct AppIndex {
    pub slots: Vec<AppSlot>,
}

#[derive(Clone, Debug)]
pub struct AppSlot {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub name: String,
    pub windows: u8,
    pub is_frontmost: bool,
}

#[derive(Debug, PartialEq)]
pub enum UiMsg {
    ShowPinned {
        rows: Arc<[PinnedRow]>,
    },
    ShowLive {
        rows: Arc<[LiveRow]>,
    },
    /// Instant hide on commit.
    Hide,
    /// Short fade on cancel.
    HideFade,
}

#[derive(Debug, PartialEq)]
pub enum AppRef {
    Pid(i32),
    Bundle(String),
    Launch(std::path::PathBuf, String),
}

#[derive(Debug, PartialEq)]
pub struct SwitchRequest {
    pub app: AppRef,
}

#[derive(Debug, PartialEq)]
pub enum Job {
    Switch(SwitchRequest),
    CycleInputSource,
    RefreshWindowCounts,
    MergeLabels(crate::labels::LabelMemory),
}
