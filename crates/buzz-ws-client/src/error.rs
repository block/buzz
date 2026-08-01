use thiserror::Error;

/// Errors returned by [`crate::NostrWsConnection`] and related operations.
#[derive(Debug, Error)]
pub enum WsClientError {
    /// A WebSocket transport error occurred.
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    /// A JSON serialization or deserialization error occurred.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// An EVENT frame had a syntactically valid raw payload that did not decode
    /// as a Nostr event. The raw object is retained so callers can classify the
    /// malformed signed fields without losing their lexical representation.
    #[error("invalid EVENT payload: {message}")]
    InvalidEvent {
        /// Exact JSON bytes of the event object from the relay frame.
        raw_event_json: Box<str>,
        /// Deserialization failure without the raw payload.
        message: String,
    },

    /// Failed to build a Nostr event.
    #[error("Nostr event builder error: {0}")]
    EventBuilder(String),

    /// Failed to parse a URL.
    #[error("URL parse error: {0}")]
    Url(String),

    /// The relay did not respond within the expected time.
    #[error("Timeout waiting for relay message")]
    Timeout,

    /// The WebSocket connection was closed before the operation completed.
    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    /// The relay sent a message that was not expected at this point.
    #[error("Unexpected relay message: {0}")]
    UnexpectedMessage(String),

    /// NIP-42 authentication was rejected by the relay.
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    /// The relay rejected the submitted event.
    #[error("Event rejected by relay: {0}")]
    EventRejected(String),

    /// No NIP-42 AUTH challenge was received from the relay.
    #[error("No AUTH challenge received from relay")]
    NoAuthChallenge,

    /// A relay supplied an AUTH challenge larger than the client will sign.
    #[error("AUTH challenge is {size} bytes; maximum is {max} bytes")]
    AuthChallengeTooLarge {
        /// Challenge length in UTF-8 bytes.
        size: usize,
        /// Maximum accepted challenge length in UTF-8 bytes.
        max: usize,
    },

    /// More than one AUTH challenge was received before authentication.
    #[error("relay sent multiple AUTH challenges before authentication")]
    AmbiguousAuthChallenge,

    /// The serialized AUTH frame exceeded the private transport boundary's cap.
    #[error("serialized AUTH frame is {size} bytes; maximum is {max} bytes")]
    AuthFrameTooLarge {
        /// Serialized JSON payload length in bytes.
        size: u64,
        /// Maximum accepted private frame payload length in bytes.
        max: u64,
    },

    /// A private AUTH frame write did not complete, so the WebSocket cannot be
    /// safely reused without appending bytes to a potentially partial frame.
    #[error("WebSocket connection is unusable after an incomplete AUTH frame write")]
    AuthTransportPoisoned,

    /// The relay reflected bytes from the signed private authentication event.
    #[error("relay reflected private authentication material")]
    ReflectedAuthMaterial,
}

impl From<nostr::event::builder::Error> for WsClientError {
    fn from(e: nostr::event::builder::Error) -> Self {
        WsClientError::EventBuilder(e.to_string())
    }
}
