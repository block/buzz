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
use nostr::{EventBuilder, Tag};

const GIT_ORIGIN_CHANNEL_ENV: &str = "BUZZ_GIT_ORIGIN_CHANNEL_ID";
const GIT_ORIGIN_AGENT_ENV: &str = "BUZZ_GIT_ORIGIN_AGENT_NAME";

/// Add trusted, session-scoped provenance supplied by the ACP harness.
///
/// Public channels use the standard NIP-29 `h` tag. Private conversations
/// intentionally omit their channel coordinate and retain only the agent's
/// display name.
pub(crate) fn with_git_provenance(builder: EventBuilder) -> Result<EventBuilder, CliError> {
    apply_git_provenance(
        builder,
        std::env::var(GIT_ORIGIN_CHANNEL_ENV).ok().as_deref(),
        std::env::var(GIT_ORIGIN_AGENT_ENV).ok().as_deref(),
    )
}

fn apply_git_provenance(
    builder: EventBuilder,
    channel_id: Option<&str>,
    agent_name: Option<&str>,
) -> Result<EventBuilder, CliError> {
    if let Some(channel_id) = channel_id {
        let channel_id = channel_id.trim();
        uuid::Uuid::parse_str(channel_id)
            .map_err(|_| CliError::Other("invalid git origin channel ID".into()))?;
        let origin_tag = Tag::parse(["h", channel_id])
            .map_err(|error| CliError::Other(format!("invalid git origin tag: {error}")))?;
        return Ok(builder.tag(origin_tag));
    }

    if let Some(agent_name) = agent_name {
        let agent_name = agent_name.trim();
        if agent_name.is_empty()
            || agent_name.len() > 256
            || agent_name.chars().any(char::is_control)
        {
            return Err(CliError::Other(
                "invalid private-conversation agent name".into(),
            ));
        }
        let origin_tag = Tag::parse(["buzz-origin-agent", agent_name])
            .map_err(|error| CliError::Other(format!("invalid git origin tag: {error}")))?;
        return Ok(builder.tag(origin_tag));
    }

    Ok(builder)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, Kind};

    fn event_with_origin(channel_id: Option<&str>, agent_name: Option<&str>) -> nostr::Event {
        apply_git_provenance(
            EventBuilder::new(Kind::Custom(1621), "issue"),
            channel_id,
            agent_name,
        )
        .expect("apply provenance")
        .sign_with_keys(&Keys::generate())
        .expect("sign event")
    }

    #[test]
    fn public_channel_origin_uses_h_tag_and_suppresses_agent_name() {
        let channel_id = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
        let event = event_with_origin(Some(channel_id), Some("Builder"));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["h", channel_id]));
        assert!(!event
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("buzz-origin-agent")));
    }

    #[test]
    fn private_origin_exposes_only_agent_name() {
        let event = event_with_origin(None, Some("Builder"));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["buzz-origin-agent", "Builder"]));
        assert!(!event
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("h")));
    }
}
