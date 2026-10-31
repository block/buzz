//! Bounded OAuth response decoding shared by legacy and strict callers.
//!
//! Deliberate compatibility change: oversized discovery/token documents are
//! infrastructure failures, never grant rejections. Raw provider diagnostics
//! are not returned or logged (they can echo codes, verifiers and tokens).

use reqwest::Response;
use serde_json::Value;

pub(crate) const MAX_OAUTH_RESPONSE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_OAUTH_ERROR_BYTES: usize = 16 * 1024;

pub(crate) async fn read_oauth_json(mut response: Response) -> Result<Value, ()> {
    let limit = if response.status().is_success() {
        MAX_OAUTH_RESPONSE_BYTES
    } else {
        MAX_OAUTH_ERROR_BYTES
    };
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(());
    }
    let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if chunk.len() > limit - bytes.len() {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}
