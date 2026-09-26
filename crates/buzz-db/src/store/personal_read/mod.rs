//! Private, signer-owned accessory read progress, separate from NIP-RS events.
//!
//! Timestamp prefixes use author time; retention uses persisted relay receipt
//! time. Only fixed context intents advance prefixes, never a query scan cap.

mod context;
mod model;
mod projection;
mod writes;

pub use model::*;

#[cfg(test)]
mod postgres_tests;
