use thiserror::Error;

/// Redacted outcomes of an assertion-bearing WebSocket handshake.
///
/// Neither HTTP response bodies nor NIP-42 refusal text are retained: an
/// untrusted relay can echo the assertion in either. These errors never cause
/// an automatic retry without the assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NipFiConnectError {
    /// The destination, bearer header, or handshake budget is invalid.
    #[error("invalid federated identity connection configuration")]
    InvalidConfiguration,
    /// HTTP 401 or a rejected NIP-42 proof; refresh or authenticate again.
    #[error("relay authentication rejected")]
    AuthenticationRejected,
    /// HTTP 403; authentication alone does not grant access.
    #[error("relay permission denied")]
    PermissionDenied,
    /// The caller's connection budget or a NIP-42 deadline elapsed.
    #[error("relay authentication timed out")]
    Timeout,
    /// DNS, socket, TLS, or connection closure prevented authentication.
    #[error("relay network failure")]
    Network,
    /// Other HTTP statuses, redirects, malformed messages, or protocol errors.
    #[error("relay authentication unavailable")]
    Unavailable,
}

impl From<WsClientError> for NipFiConnectError {
    fn from(error: WsClientError) -> Self {
        use tokio_tungstenite::tungstenite::Error as WsError;
        match error {
            WsClientError::WebSocket(WsError::Http(response)) => match response.status().as_u16() {
                401 => Self::AuthenticationRejected,
                403 => Self::PermissionDenied,
                _ => Self::Unavailable,
            },
            WsClientError::WebSocket(WsError::Io(_) | WsError::Tls(_))
            | WsClientError::ConnectionClosed => Self::Network,
            WsClientError::Timeout | WsClientError::NoAuthChallenge => Self::Timeout,
            WsClientError::AuthFailed(_) => Self::AuthenticationRejected,
            _ => Self::Unavailable,
        }
    }
}

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
