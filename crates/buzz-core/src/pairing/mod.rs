//! NIP-AB device pairing — crypto primitives, message types, and error types.
//!
//! NIP-AB enables two Nostr devices to securely exchange a secret (e.g., an
//! `nsec` or a NIP-46 bunker connection string) over an untrusted relay, using:
//!
//! 1. **HKDF-SHA256** for all key derivation (session ID, SAS code, transcript hash).
//! 2. **ECDH** (via [`nostr::util::generate_shared_key`]) for the shared secret.
//! 3. **NIP-44 v2** for encrypting the message payloads.
//! 4. **Short Authentication String (SAS)** for out-of-band confirmation.
//!
//! # Module layout
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`crypto`] | Pure HKDF derivation functions |
//! | [`types`]  | Serde-serializable pairing message types |
//!
//! # Error handling
//!
//! All fallible operations in the pairing flow return [`PairingError`].

pub mod crypto;
pub mod qr;
pub mod session;
pub mod types;

pub use qr::QrPayload;
pub use session::{PairingSession, Role, SessionState};
pub use types::{AbortReason, PairingMessage, PayloadType};

use thiserror::Error;

/// Errors that can occur during a NIP-AB pairing session.
#[derive(Debug, Error)]
pub enum PairingError {
    /// The scanned QR URI was not a valid NIP-AB pairing URI.
    #[error("invalid QR URI: {0}")]
    InvalidQr(String),

    /// The session ID extracted from a message was not a valid 32-byte hex string.
    #[error("invalid session ID")]
    InvalidSessionId,

    /// The SAS code shown on both devices did not match — session must be aborted.
    #[error("SAS mismatch")]
    SasMismatch,

    /// The transcript hash received from the peer did not match the locally computed value.
    #[error("transcript hash mismatch")]
    TranscriptMismatch,

    /// A message arrived out of sequence or with the wrong type for the current state.
    #[error("unexpected message type: expected {expected}, got {got}")]
    UnexpectedMessage {
        /// The message type that was expected at this point in the protocol.
        expected: String,
        /// The message type that was actually received.
        got: String,
    },

    /// The pairing session exceeded its time limit without completing.
    #[error("session expired")]
    SessionExpired,

    /// NIP-44 encryption or decryption failed.
    #[error("NIP-44 error: {0}")]
    Nip44(#[from] nostr::nips::nip44::Error),

    /// JSON serialization or deserialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// A public key string could not be parsed.
    #[error("invalid pubkey: {0}")]
    InvalidPubkey(String),

    /// Event signing or construction failed.
    #[error("event signing failed: {0}")]
    SigningError(String),
}
