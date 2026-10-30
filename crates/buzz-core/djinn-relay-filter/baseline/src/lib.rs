//! Focused fallback when workspace registry resolution is blocked.
//! Includes the actual complete production owner source and its existing tests.
//! These aliases use the real core types/helpers, not replacements.
#![forbid(unsafe_code)]
pub use buzz_core::StoredEvent;
#[cfg(not(feature = "first-generated"))]
#[path = "../../../src/prepared_filter.rs"]
pub mod prepared_filter;
pub mod event {
    pub use buzz_core::event::*;
}
pub mod kind {
    pub use buzz_core::kind::*;
}
pub mod test_helpers {
    pub use buzz_core::test_helpers::*;
}
#[path = "../../../src/filter.rs"]
pub mod owner;

/// Frozen original registry for benchmark comparisons only.
pub mod historical_subscription;

pub mod fanout;
#[path = "../../src/fixtures.rs"]
pub mod fixtures;

/// Comparison-only view of the immutable first verified generated source.
#[cfg(feature = "first-generated")]
pub mod first_prepared_filter;
#[cfg(feature = "first-generated")]
pub use first_prepared_filter as prepared_filter;

#[cfg(feature = "first-generated")]
pub mod current_subscription {
    include!(concat!(env!("OUT_DIR"), "/first_subscription.rs"));
}

/// Current production registry source, including its full owner tests.
#[cfg(not(feature = "first-generated"))]
#[path = "../../../../buzz-relay/src/subscription.rs"]
pub mod current_subscription;
