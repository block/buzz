//! Private, signer-owned accessory read progress, separate from NIP-RS events.
//!
//! A frontier is the relay arrival time of the message a context was read
//! through. Read intents and the author's own posts advance frontiers. A
//! thread frontier exists only for a followed thread, and the sidebar counts
//! forward from each frontier. An implicit follow goes dormant after the
//! relay's window without the actor's involvement; an explicit one never does.
//! Agent accounts keep no read state.

mod coverage;
mod follows;
mod model;
mod projection;
mod writes;

pub use coverage::{Coverage, CoverageAnswer, MAX_COVERAGE_IDS};
pub use follows::set_follow_window_seconds;
pub(crate) use follows::{place, record_message};
pub use model::*;

#[cfg(test)]
mod postgres_tests;

#[cfg(test)]
mod follows_postgres_tests;

#[cfg(test)]
mod projection_postgres_tests;

#[cfg(test)]
mod threads_postgres_tests;

#[cfg(test)]
mod arrival_postgres_tests;
