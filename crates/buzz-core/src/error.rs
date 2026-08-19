/// Errors that can occur during Nostr event verification.
#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    /// The event ID does not match the canonical hash of the event fields.
    #[error("invalid event id: computed {computed}, got {got}")]
    InvalidId {
        /// The ID we computed from the event fields.
        computed: String,
        /// The ID present in the event.
        got: String,
    },

    /// The Schnorr signature over the event ID is invalid.
    #[error("invalid schnorr signature")]
    InvalidSignature,

    /// Low-level secp256k1 cryptographic error.
    #[error("secp256k1 error: {0}")]
    Secp(#[from] nostr::secp256k1::Error),
}
