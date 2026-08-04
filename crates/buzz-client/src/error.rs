use nostr::EventId;

use crate::EndpointError;

/// Typed failure categories returned by reusable Buzz client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Community endpoint parsing or authority validation failed.
    #[error(transparent)]
    Endpoint(#[from] EndpointError),
    /// Explicit client configuration is invalid.
    #[error("invalid client configuration: {0}")]
    Configuration(String),
    /// Signer-bound authentication is malformed or invalid.
    #[error("authentication failed: {0}")]
    Authentication(String),
    /// The configured signer refused or failed an operation.
    #[error("signer failed: {0}")]
    Signer(#[source] nostr::signer::SignerError),
    /// The HTTP client could not be constructed.
    #[error("HTTP client initialization failed: {0}")]
    HttpClient(#[source] reqwest::Error),
    /// An authenticated network request failed.
    #[error("network request failed: {0}")]
    Network(#[source] reqwest::Error),
    /// The relay definitively rejected or could not fulfill the operation.
    #[error("relay returned HTTP {status}: {reason}")]
    Relay {
        /// HTTP response status.
        status: u16,
        /// Relay-provided reason.
        reason: String,
        /// Whether repeating the same operation is classified as safe.
        retry_safe: bool,
    },
    /// Serialization or response parsing failed.
    #[error("serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    /// A domain request is invalid.
    #[error("invalid request: {0}")]
    InvalidInput(String),
}

impl ClientError {
    /// Whether retrying the same operation is safe under Buzz policy.
    pub fn is_retry_safe(&self) -> bool {
        matches!(
            self,
            Self::Relay {
                retry_safe: true,
                ..
            }
        )
    }
}

/// Semantic result of submitting one signed stored event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    /// The relay confirmed acceptance.
    Accepted(AcceptedDelivery),
    /// The relay definitively rejected the event.
    Rejected(RejectedDelivery),
    /// The final failure cannot establish whether the relay stored the event.
    Unknown(DeliveryUnknown),
}

/// Confirmed stored-event acceptance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedDelivery {
    /// Identifier of the signed event accepted by the relay.
    pub event_id: EventId,
    /// Relay-provided acceptance message.
    pub relay_message: String,
}

/// Definitive stored-event rejection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedDelivery {
    /// Identifier of the signed event rejected by the relay.
    pub event_id: EventId,
    /// HTTP response status, when rejection used the HTTP bridge.
    pub status: u16,
    /// Relay-provided rejection reason.
    pub reason: String,
    /// Whether resubmitting the same signed event is safe.
    pub retry_safe: bool,
}

/// Ambiguous stored-event delivery after bounded internal attempts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryUnknown {
    /// Original signed event identifier, unchanged across every attempt.
    pub event_id: EventId,
    /// Description of the terminal ambiguous failure.
    pub reason: String,
}
