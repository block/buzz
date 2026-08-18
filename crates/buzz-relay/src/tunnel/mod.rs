//! Relay-side tunnel machinery for inter-relay mesh sessions.
//!
//! The mesh transport moves opaque bytes between runtimes. This module owns the
//! session layer correctness boundary: Redis-fenced ownership, strict generation
//! validation, and profile-specific routing decisions for tunnel consumers.

pub mod directory;
pub mod reliable;
