//! Host-side Buzz operations for ACP/MCP integrations.
//!
//! These helpers intentionally do not print to stdout: MCP servers use stdout
//! for their protocol transport. Authentication stays in the host-side process
//! environment and is never handed to the model or its tool container.

use nostr::{EventId, Keys};
use serde::Serialize;
use uuid::Uuid;

use crate::client::{normalize_relay_url, BuzzClient};

/// Delivery proof returned by [`reply_from_env`].
#[derive(Debug, Clone, Serialize)]
pub struct NativeReplyReceipt {
    /// Signed Nostr event ID accepted for delivery.
    pub event_id: String,
    /// Channel that received the reply.
    pub channel_id: String,
}

/// Publish a stream-channel reply using host-side Buzz credentials.
///
/// Credentials are read from `BUZZ_RELAY_URL`, `BUZZ_PRIVATE_KEY`, and the
/// optional `BUZZ_AUTH_TAG`. `reply_to` is the immediate parent; `thread_root`
/// may be supplied for a nested reply and otherwise defaults to `reply_to`.
pub async fn reply_from_env(
    channel_id: &str,
    content: &str,
    reply_to: &str,
    thread_root: Option<&str>,
    mention_pubkeys: &[String],
) -> Result<NativeReplyReceipt, String> {
    let channel_id =
        Uuid::parse_str(channel_id).map_err(|error| format!("invalid channel UUID: {error}"))?;
    if content.trim().is_empty() {
        return Err("message content must not be empty".into());
    }
    let parent_event_id =
        EventId::from_hex(reply_to).map_err(|error| format!("invalid reply event ID: {error}"))?;
    let root_event_id = match thread_root {
        Some(root) => {
            EventId::from_hex(root).map_err(|error| format!("invalid thread root ID: {error}"))?
        }
        None => parent_event_id,
    };

    let relay_url = std::env::var("BUZZ_RELAY_URL")
        .map_err(|_| "BUZZ_RELAY_URL is not configured for the native Buzz tool".to_string())?;
    let private_key = std::env::var("BUZZ_PRIVATE_KEY")
        .map_err(|_| "BUZZ_PRIVATE_KEY is not configured for the native Buzz tool".to_string())?;
    let keys =
        Keys::parse(&private_key).map_err(|error| format!("invalid BUZZ_PRIVATE_KEY: {error}"))?;

    let auth_tag_json = std::env::var("BUZZ_AUTH_TAG")
        .ok()
        .filter(|value| !value.is_empty());
    let auth_tag = match auth_tag_json.as_deref() {
        Some(raw) => {
            let tag = buzz_sdk::nip_oa::parse_auth_tag(raw)
                .map_err(|error| format!("BUZZ_AUTH_TAG is malformed: {error}"))?;
            buzz_sdk::nip_oa::verify_auth_tag(raw, &keys.public_key())
                .map_err(|error| format!("BUZZ_AUTH_TAG verification failed: {error}"))?;
            Some(tag)
        }
        None => None,
    };

    let client = BuzzClient::new(
        normalize_relay_url(&relay_url),
        keys,
        auth_tag,
        auth_tag_json,
    )
    .map_err(|error| error.to_string())?;
    let thread_ref = buzz_sdk::ThreadRef {
        root_event_id,
        parent_event_id,
    };
    let mentions: Vec<&str> = mention_pubkeys.iter().map(String::as_str).collect();
    let builder = buzz_sdk::build_message(
        channel_id,
        content,
        Some(&thread_ref),
        &mentions,
        false,
        &[],
    )
    .map_err(|error| format!("failed to build Buzz reply: {error}"))?;
    let event = client
        .sign_event(builder)
        .map_err(|error| error.to_string())?;
    let event_id = event.id.to_hex();
    client
        .submit_event(event)
        .await
        .map_err(|error| error.to_string())?;

    Ok(NativeReplyReceipt {
        event_id,
        channel_id: channel_id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_invalid_routing_before_reading_credentials() {
        let error = reply_from_env("not-a-uuid", "hello", &"a".repeat(64), None, &[])
            .await
            .expect_err("invalid channel must fail");
        assert!(error.contains("invalid channel UUID"));

        let error = reply_from_env(
            "5508fd33-ed7e-46c0-9950-8a3a76a51779",
            "hello",
            "not-an-event",
            None,
            &[],
        )
        .await
        .expect_err("invalid reply target must fail");
        assert!(error.contains("invalid reply event ID"));
    }
}
