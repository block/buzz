//! Huddle audio primitives shared by desktop clients.
//!
//! The host retains identity, room admission, device routing, UI, and speech models.
//! This crate owns the v2 handshake, wire format, Opus encoding and receive jitter
//! buffering. It neither loads private keys nor starts background tasks.
#![deny(unsafe_code)]

pub mod connection;
pub mod encoder;
pub mod jitter;
pub mod wire;
