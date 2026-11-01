//! App index: NSWorkspace recency ring + CGWindowList window counts,
//! published as ArcSwap<AppIndex> for the engine to read lock-free.

use mojito_core::AppIndex;
use std::sync::Arc;
use arc_swap::ArcSwap;

pub struct AppIndexHandle {
    current: Arc<ArcSwap<AppIndex>>,
}

impl AppIndexHandle {
    pub fn new() -> Self {
        Self { current: Arc::new(ArcSwap::from_pointee(AppIndex::default())) }
    }

    pub fn handle(&self) -> Arc<ArcSwap<AppIndex>> {
        self.current.clone()
    }

    /// Install NSWorkspace observers (activate/launch/terminate) and refresh
    /// the ring. Runs on the main thread.
    pub fn start(&self) {
        todo!("index::start")
    }

    /// CGWindowList layer-0 on-screen window counts per pid, merged into the
    /// published index. No kCGWindowName reads.
    pub fn refresh_window_counts(&self) {
        todo!("refresh_window_counts")
    }
}
