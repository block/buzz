//! Private, signer-owned accessory read progress, separate from NIP-RS events.
//!
//! A frontier is the relay arrival time of the message a context was read
//! through; the unread horizon alone uses author time. Only fixed context
//! intents advance frontiers, never a query scan cap.

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

#[cfg(test)]
mod arrival_postgres_tests;
