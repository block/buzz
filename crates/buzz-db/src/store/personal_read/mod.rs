//! Private, signer-owned accessory read progress, separate from NIP-RS events.
//!
//! A frontier is the relay arrival time of the message a context was read
//! through, and unread counts forward from it. Only fixed context intents
//! advance frontiers; ingest only creates thread membership rows.

mod classification;
mod context;
mod membership;
mod model;
mod projection;
mod writes;

pub(crate) use membership::{commit, record_reply, reserve};
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
