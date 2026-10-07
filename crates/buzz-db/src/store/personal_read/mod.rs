//! Private, signer-owned accessory read progress, separate from NIP-RS events.
//!
//! A frontier is the relay arrival time of the message a context was read
//! through, and unread counts forward from it. Fixed intents advance and
//! follow; ingest advances the author's own frontiers (posting marks read)
//! and creates follow rows for replies and mentions.

mod classification;
mod context;
mod membership;
mod model;
mod projection;
mod writes;

pub(crate) use membership::{record_message, Place};
pub use model::*;

#[cfg(test)]
mod postgres_tests;

#[cfg(test)]
mod projection_postgres_tests;

#[cfg(test)]
mod threads_postgres_tests;

#[cfg(test)]
mod arrival_postgres_tests;

#[cfg(test)]
mod bench_postgres_tests;
