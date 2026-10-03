//! Private, signer-owned accessory read progress, separate from NIP-RS events.
//!
//! Timestamp prefixes and the unread horizon both use author time. Only fixed
//! context intents advance prefixes, never a query scan cap.

mod classification;
mod context;
mod model;
mod participation;
mod projection;
mod writes;

pub use model::*;

#[cfg(test)]
mod postgres_tests;

#[cfg(test)]
mod participation_postgres_tests;

#[cfg(test)]
mod projection_postgres_tests;

#[cfg(test)]
mod threads_postgres_tests;
