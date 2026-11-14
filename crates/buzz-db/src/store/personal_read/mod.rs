//! Private, signer-owned read state, separate from NIP-RS events: a read
//! position, a state and a manual unread flag per conversation and thread.
//!
//! A read position is the relay arrival time of the message a context was
//! read through. Only fixed intents and the person's own posts move it.
//! `crate::store::activity` reads these rows into the activity list.

mod model;
mod writes;

pub use model::*;
pub(crate) use writes::deadlines;
