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

    /// The relay closed the connection with a policy close (1008), as a
    /// NIP-FI denial does; the string is the close reason.
    #[error("Connection refused by relay policy: {0}")]
    PolicyClose(String),

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

impl WsClientError {
    /// The HTTP status and body when the relay refused the WebSocket upgrade
    /// with an HTTP response (e.g. a NIP-FI `401`/`403` decided at upgrade time).
    pub fn upgrade_rejection(&self) -> Option<(u16, String)> {
        match self {
            WsClientError::WebSocket(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                let body = response.body().as_deref().unwrap_or_default();
                Some((
                    response.status().as_u16(),
                    String::from_utf8_lossy(body).into_owned(),
                ))
            }
            _ => None,
        }
    }
}
