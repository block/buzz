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
}

impl From<nostr::event::builder::Error> for WsClientError {
    fn from(e: nostr::event::builder::Error) -> Self {
        WsClientError::EventBuilder(e.to_string())
    }
}
