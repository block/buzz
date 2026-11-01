use thiserror::Error;

/// Errors that can end a bridge turn or the bridge itself.
///
/// Production paths return these with `?`; they are never masked as success.
/// A driver error during `session/prompt` surfaces to buzz-acp as a JSON-RPC
/// error on that request, so the harness records a failed turn instead of a
/// silent empty one.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// The Chromium binary could not be found or launched.
    #[error("could not launch Chromium: {0}")]
    ChromeLaunch(String),

    /// The CDP connection or a CDP command failed.
    #[error("browser protocol error: {0}")]
    Protocol(String),

    /// The Muse web app did not present an expected element. The message
    /// names what was looked for so the DOM model in `selectors` can be
    /// corrected without guessing.
    #[error("Muse web UI changed or is unreachable: {0}")]
    WebDom(String),

    /// The user is not signed in to Muse in the bridge's browser profile,
    /// and the sign-in wait expired.
    #[error("not signed in to Muse: {0}")]
    NotSignedIn(String),

    /// The turn was cancelled via `session/cancel`.
    #[error("turn cancelled")]
    Cancelled,

    /// Underlying I/O failure (stdio, websocket, process spawn).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON value could not be parsed or had an unexpected shape.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
