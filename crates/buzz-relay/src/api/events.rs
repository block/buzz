//! Event endpoints — now served via the Nostr HTTP bridge.
//!
//! This module re-exports bridge handlers for backward compatibility with router.rs.

pub use super::bridge::{count_events, query_events, submit_event};
