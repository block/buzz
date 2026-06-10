#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `sprout-proxy` — Guest relay proxy for Nostr client compatibility.
//!
//! Translates standard Nostr kinds ↔ Sprout custom kinds, derives deterministic
//! shadow keypairs for external users, and authenticates guests via invite tokens.

/// Bidirectional UUID ↔ NIP-28 kind:40 event ID mapping.
pub mod channel_map;
/// Error types for the proxy layer.
pub mod error;
/// Pubkey-based guest registry for persistent access control.
pub mod guest_store;
/// Invite token management for guest authentication.
pub mod invite;
/// Thread-safe invite token registry.
pub mod invite_store;
/// Kind translation between standard Nostr and Sprout-internal kinds.
pub mod kind_translator;
/// External-facing NIP-01 WebSocket server for standard Nostr clients.
pub mod server;
/// Deterministic shadow keypair derivation and caching.
pub mod shadow_keys;
/// Event translation between Sprout internal format and NIP-28 standard format.
pub mod translate;
/// Upstream relay WebSocket client with NIP-42 auth and reconnect.
pub mod upstream;

pub use error::ProxyError;
pub use guest_store::GuestStore;
pub use invite::InviteToken;
pub use invite_store::InviteStore;
pub use kind_translator::KindTranslator;
pub use shadow_keys::ShadowKeyManager;
