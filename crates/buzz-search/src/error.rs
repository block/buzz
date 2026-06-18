use thiserror::Error;

/// Errors produced by the search service.
#[derive(Debug, Error)]
pub enum SearchError {
    /// An HTTP transport error from reqwest.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// Typesense returned a non-success HTTP status.
    #[error("Typesense API error (status {status}): {body}")]
    Api {
        /// HTTP status code returned by Typesense.
        status: u16,
        /// Response body from Typesense.
        body: String,
    },

    /// JSON serialization or deserialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// A batch import partially failed.
    #[error("Batch import partial failure: {succeeded} succeeded, {failed} failed")]
    BatchPartial {
        /// Number of documents successfully imported.
        succeeded: usize,
        /// Number of documents that failed to import.
        failed: usize,
    },

    /// A Nostr event could not be converted to a Typesense document.
    #[error("Event conversion error: {0}")]
    Conversion(String),

    /// The provided event ID is not valid hex.
    #[error("Invalid event_id: {0}")]
    InvalidEventId(String),
}
