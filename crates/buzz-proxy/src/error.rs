use thiserror::Error;

/// Errors returned by the proxy layer.
#[derive(Debug, Error)]
pub enum ProxyError {
    /// The invite token was not found in the store.
    #[error("invite token not found")]
    InviteNotFound,

    /// The invite token has passed its expiry time.
    #[error("invite token expired")]
    InviteExpired,

    /// The invite token has reached its maximum use count.
    #[error("invite token exhausted")]
    InviteExhausted,

    /// The supplied external public key is not a valid 32-byte hex string.
    #[error("invalid external pubkey: {0}")]
    InvalidPubkey(String),

    /// Shadow key derivation failed.
    #[error("shadow key derivation failed: {0}")]
    KeyDerivation(String),

    /// An I/O or network error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The upstream relay connection failed.
    #[error("upstream error: {0}")]
    Upstream(String),

    /// Authentication failed.
    #[error("auth error: {0}")]
    Auth(String),

    /// JSON serialization/deserialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The client does not have permission for this operation.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// The requested channel was not found.
    #[error("channel not found: {0}")]
    ChannelNotFound(String),
}
