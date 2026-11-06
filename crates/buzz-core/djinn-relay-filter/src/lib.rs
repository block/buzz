//! Isolated specification, generator-owned solution and frozen full-operation workload.
#![forbid(unsafe_code)]
pub mod adapter;
pub mod bench;
pub mod fixtures;
pub mod historical;
pub mod input;
#[path = "../djinn/solution/mod.rs"]
pub mod solution;
pub mod spec;
pub mod timing;
pub mod event {
    pub use buzz_core::StoredEvent;
}

pub use spec::filters_match as filters_match_projected;

/// Rejected per-call prototype, retained only for comparison of adaptation cost.
pub fn per_call_prototype(filters: &[nostr::Filter], event: &buzz_core::StoredEvent) -> bool {
    filters_match_projected(
        &adapter::prepare_filters(filters),
        &adapter::prepare_event(event),
    )
}

/// Normal Rust view of the same exact generated operation.
pub mod prepared_filter {
    pub use super::adapter::{prepare_event, prepare_filters};
    pub use super::solution::filters_match;
}
