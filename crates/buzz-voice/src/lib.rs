//! Reusable local voice primitives for Buzz.
//!
//! Product code owns capture, playback, huddle membership, and posting. This
//! crate owns reusable STT/TTS model and pipeline logic that can be shared by
//! desktop, mobile, and standalone hosts.

pub mod models;
pub mod pocket;
