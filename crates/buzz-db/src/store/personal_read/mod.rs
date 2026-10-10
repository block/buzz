//! Private, signer-owned accessory read progress, separate from NIP-RS events.
//!
//! A frontier is the relay arrival time of the message a context was read
//! through; the unread horizon alone uses author time. Read intents and the
//! author's own posts advance frontiers. A thread frontier exists only for a
//! followed thread, and the sidebar counts forward from each frontier.

mod follows;
mod model;
mod projection;
mod writes;

pub(crate) use follows::{record_message, Place};
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
