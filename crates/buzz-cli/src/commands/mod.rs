pub mod agents;
pub mod channel_templates;
pub mod channels;
pub mod dms;
pub mod emoji;
pub mod feed;
pub mod issues;
pub mod mem;
pub mod messages;
pub mod moderation;
pub mod notes;
pub mod pack;
pub mod patches;
pub mod pr;
pub mod projects;
pub mod reactions;
pub mod repos;
pub mod social;
pub mod upload;
pub mod users;
pub mod workflows;

use crate::{client::normalize_write_response, error::CliError};

/// Parse a relay write-response JSON blob, mapping a duplicate (dominated)
/// write to [`CliError::Conflict`] with the caller-supplied message.
///
/// Used by every command that publishes an NIP-33 addressable event and
/// needs to tell accepted from duplicate/dominated.
pub fn parse_write_response(raw: &str, conflict_msg: &str) -> Result<String, CliError> {
    let response: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e} ({raw})")))?;
    let accepted = response
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let message = response
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !accepted {
        return Err(CliError::Other(format!("relay rejected event: {message}")));
    }
    if message == "duplicate" || message.starts_with("duplicate:") {
        return Err(CliError::Conflict(conflict_msg.to_string()));
    }
    Ok(normalize_write_response(raw))
}
