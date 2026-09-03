//! Error types for buzz-auth.

/// Prefix of the operator-facing detail inside [`AuthError::Nip98Invalid`] for
/// `u`-tag mismatches. Kept as a shared constant so [`AuthError::client_message`]
/// and the verifier cannot drift apart.
pub const NIP98_URL_MISMATCH_PREFIX: &str = "URL mismatch";

/// All errors that can occur during authentication and authorization.
///
/// Variants are designed to be safe to return to callers without leaking
/// internal implementation details. Do **not** include raw token values,
/// database contents, or stack traces in error messages.
///
/// [`AuthError::Nip98Invalid`] is the exception for **server logs only**: its
/// `Display` may include the signed `u` tag and the relay's expected URL so
/// operators can diagnose Host / proxy mismatches. HTTP handlers must return
/// [`AuthError::client_message`] instead of formatting `{e}` into responses.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The NIP-42 event signature is invalid or the event is structurally malformed.
    #[error("invalid signature or malformed auth event")]
    InvalidSignature,

    /// The `challenge` tag in the AUTH event does not match the relay's issued challenge.
    #[error("challenge mismatch")]
    ChallengeMismatch,

    /// The `relay` tag in the AUTH event does not match this relay's URL.
    #[error("relay url mismatch")]
    RelayUrlMismatch,

    /// The AUTH event's `created_at` timestamp is more than ±60 seconds from now.
    #[error("auth event timestamp outside ±60s window")]
    EventExpired,

    /// NIP-98 HTTP Auth event (kind:27235) failed verification.
    ///
    /// The inner string describes the specific failure (signature, timestamp, URL, etc.)
    /// and is safe to include in **server logs**. Do **not** forward this Display
    /// form to clients — use [`AuthError::client_message`].
    #[error("NIP-98 HTTP Auth verification failed: {0}")]
    Nip98Invalid(String),

    /// A NIP-98 event with the same id has already been observed within the
    /// replay-prevention window. The event itself was structurally valid; the
    /// rejection is on freshness, not validity.
    #[error("NIP-98 replay: event id already seen within window")]
    Nip98Replay,

    /// The pubkey in the auth event does not match the expected identity.
    #[error("pubkey mismatch: event pubkey does not match authenticated identity")]
    PubkeyMismatch,

    /// The authenticated context does not have the required scope for this operation.
    #[error("insufficient scope: required {required}, have {have:?}")]
    InsufficientScope {
        /// The scope that was required.
        required: String,
        /// The scopes the caller actually holds.
        have: Vec<String>,
    },

    /// The authenticated user is not a member of the requested channel.
    #[error("channel access denied")]
    ChannelAccessDenied,

    /// An unexpected internal error occurred (e.g. a `spawn_blocking` panic).
    #[error("internal auth error: {0}")]
    Internal(String),
}

impl AuthError {
    /// Client-facing message that must not disclose internal relay addresses,
    /// signed URL tags, or other verification detail useful to an unauthenticated
    /// caller probing a fronted origin.
    ///
    /// Log the full [`Display`](std::fmt::Display) form server-side; return this
    /// string (or an equivalent opaque phrase) in HTTP 401 bodies.
    #[must_use]
    pub fn client_message(&self) -> &'static str {
        match self {
            Self::Nip98Invalid(detail) if detail.starts_with(NIP98_URL_MISMATCH_PREFIX) => {
                "NIP-98: URL mismatch"
            }
            Self::Nip98Invalid(_) => "NIP-98: authentication failed",
            Self::Nip98Replay => "NIP-98: replay detected",
            Self::InvalidSignature => "invalid signature or malformed auth event",
            Self::ChallengeMismatch => "challenge mismatch",
            Self::RelayUrlMismatch => "relay url mismatch",
            Self::EventExpired => "auth event timestamp outside ±60s window",
            Self::PubkeyMismatch => {
                "pubkey mismatch: event pubkey does not match authenticated identity"
            }
            Self::InsufficientScope { .. } => "insufficient scope",
            Self::ChannelAccessDenied => "channel access denied",
            // Internal detail stays off the wire; callers already map this to 5xx.
            Self::Internal(_) => "internal auth error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nip98_url_mismatch_client_message_omits_urls() {
        let err = AuthError::Nip98Invalid(format!(
            "{NIP98_URL_MISMATCH_PREFIX}: event has `https://public.example.com/query`, expected `http://10.0.0.1:3001/query`"
        ));
        let full = err.to_string();
        assert!(
            full.contains("10.0.0.1"),
            "Display must retain detail for operators; got {full}"
        );
        let client = err.client_message();
        assert_eq!(client, "NIP-98: URL mismatch");
        assert!(
            !client.contains("10.0.0.1") && !client.contains("public.example.com"),
            "client message must not embed either URL; got {client}"
        );
    }

    #[test]
    fn other_nip98_failures_collapse_to_generic_client_message() {
        let err = AuthError::Nip98Invalid("invalid Schnorr signature".into());
        assert_eq!(err.client_message(), "NIP-98: authentication failed");
    }
}
