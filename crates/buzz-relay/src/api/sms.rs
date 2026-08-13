//! Twilio inbound SMS webhook — `POST /hooks/sms/inbound`.
//!
//! Trust chain for an inbound SMS, in order, each step fatal on failure:
//! 1. `X-Twilio-Signature` validates the request actually came from Twilio
//!    ([`crate::twilio_auth::validate_signature`]).
//! 2. The sender's phone number must be `allowed = true` in `sms_identities`
//!    (closes the anonymous-spam / oracle vector — an unlisted number gets
//!    the exact same rejection as a signature failure, so a prober can't
//!    distinguish "bad signature" from "signature fine, number not allowed").
//!
//! On success, synthesizes a `KIND_STREAM_MESSAGE` (kind 9) event into the
//! configured SMS-inbox channel — the same kind [`crate::workflow_sink`] uses
//! to post a system-authored message, not `KIND_STREAM_MESSAGE_V2`: buzz-acp's
//! default (Mentions-mode) subscribe filter only wakes agents on kind 9 (see
//! `buzz-acp/src/config.rs`'s `resolve_channel_filters`), so a V2-only event
//! would never trigger the sms-operator persona that's meant to read it.
//! The sms-operator persona itself (reading this channel and dispatching) is
//! a follow-up slice — this only gets the message onto the relay correctly.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use chrono::Utc;
use nostr::{EventBuilder, Kind, Tag};

use buzz_core::kind::KIND_STREAM_MESSAGE;
use buzz_db::event::ThreadMetadataParams;
use buzz_db::sms::SmsIdentity;

use crate::state::AppState;
use crate::twilio_auth::validate_signature;

use super::{api_error, internal_error};

/// Generic rejection for both "bad signature" and "number not allowed" —
/// deliberately identical so the endpoint isn't an oracle for either fact.
fn rejected() -> (StatusCode, Json<serde_json::Value>) {
    api_error(StatusCode::FORBIDDEN, "request not accepted")
}

/// Build the tag set for a synthesized inbound-SMS event: `h` scopes it to
/// the SMS-inbox channel, `p` attributes it to the sender's linked pubkey
/// when known, `sms_from`/`sms_sid` carry the Twilio identifiers, and
/// `project` carries the sender's routing default when set (read by the
/// sms-operator persona's fast path — see module docs).
fn build_tags(
    channel_id: uuid::Uuid,
    identity: &SmsIdentity,
    message_sid: &str,
) -> Result<Vec<Tag>, String> {
    let mut tags = vec![
        Tag::parse(["h", &channel_id.to_string()]).map_err(|e| format!("h tag: {e}"))?,
        Tag::parse(["sms_from", &identity.phone_number])
            .map_err(|e| format!("sms_from tag: {e}"))?,
        Tag::parse(["sms_sid", message_sid]).map_err(|e| format!("sms_sid tag: {e}"))?,
    ];
    if let Some(pubkey_bytes) = &identity.linked_pubkey {
        if let Ok(pk) = nostr::PublicKey::from_slice(pubkey_bytes) {
            tags.push(Tag::parse(["p", &pk.to_hex()]).map_err(|e| format!("p tag: {e}"))?);
        }
    }
    if let Some(project) = &identity.default_project {
        tags.push(Tag::parse(["project", project]).map_err(|e| format!("project tag: {e}"))?);
    }
    Ok(tags)
}

/// Handle an inbound Twilio SMS webhook POST. Validates the request
/// signature, requires the sender's phone number be allow-listed, then
/// publishes the message into the SMS-inbox channel.
pub async fn twilio_inbound(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(params): Form<BTreeMap<String, String>>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let (Some(auth_token), Some(webhook_url)) = (
        state.config.twilio_auth_token.as_deref(),
        state.config.twilio_webhook_url.as_deref(),
    ) else {
        // Not configured — fail closed rather than skip validation.
        return Err(rejected());
    };

    let signature = headers
        .get("X-Twilio-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !validate_signature(webhook_url, &params, auth_token, signature) {
        return Err(rejected());
    }

    let from = params.get("From").map(String::as_str).unwrap_or("");
    if from.is_empty() {
        return Err(rejected());
    }

    let identity = state
        .db
        .get_sms_identity(from)
        .await
        .map_err(|_| internal_error("sms identity lookup failed"))?;

    let Some(identity) = identity.filter(|i| i.allowed) else {
        return Err(rejected());
    };

    let Some(channel_id) = state.config.twilio_sms_inbox_channel else {
        // No inbox channel configured — reject rather than silently drop.
        return Err(internal_error("sms inbox channel not configured"));
    };

    let body = params.get("Body").cloned().unwrap_or_default();
    let message_sid = params.get("MessageSid").map(String::as_str).unwrap_or("");

    let tags = build_tags(channel_id, &identity, message_sid)
        .map_err(|e| internal_error(&format!("sms event tag build failed: {e}")))?;

    let event = EventBuilder::new(Kind::from(KIND_STREAM_MESSAGE as u16), &body)
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| internal_error(&format!("sms event signing failed: {e}")))?;

    let event_created_at = {
        let ts = event.created_at.as_secs() as i64;
        chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now)
    };
    let event_id_bytes = event.id.as_bytes().to_vec();
    let thread_meta = Some(ThreadMetadataParams {
        event_id: &event_id_bytes,
        event_created_at,
        channel_id,
        parent_event_id: None,
        parent_event_created_at: None,
        root_event_id: None,
        root_event_created_at: None,
        depth: 0,
        broadcast: false,
    });

    state
        .db
        .insert_event_with_thread_metadata(
            identity.community_id,
            &event,
            Some(channel_id),
            thread_meta,
        )
        .await
        .map_err(|e| internal_error(&format!("sms event insert failed: {e}")))?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "status": "accepted" })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_is_forbidden_and_generic() {
        let (status, Json(body)) = rejected();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "request not accepted");
    }

    fn test_identity() -> SmsIdentity {
        SmsIdentity {
            phone_number: "+15551234567".to_string(),
            community_id: buzz_core::CommunityId::from_uuid(uuid::Uuid::new_v4()),
            allowed: true,
            linked_pubkey: None,
            default_project: None,
        }
    }

    fn tag_value<'a>(tags: &'a [Tag], name: &str) -> Option<&'a str> {
        tags.iter()
            .find(|t| t.as_slice().first().map(String::as_str) == Some(name))
            .and_then(|t| t.as_slice().get(1))
            .map(String::as_str)
    }

    #[test]
    fn build_tags_includes_h_from_and_sid() {
        let channel_id = uuid::Uuid::new_v4();
        let identity = test_identity();
        let tags = build_tags(channel_id, &identity, "SM123").unwrap();

        assert_eq!(
            tag_value(&tags, "h"),
            Some(channel_id.to_string()).as_deref()
        );
        assert_eq!(tag_value(&tags, "sms_from"), Some("+15551234567"));
        assert_eq!(tag_value(&tags, "sms_sid"), Some("SM123"));
    }

    #[test]
    fn build_tags_omits_p_and_project_when_unset() {
        let tags = build_tags(uuid::Uuid::new_v4(), &test_identity(), "SM123").unwrap();
        assert_eq!(tag_value(&tags, "p"), None);
        assert_eq!(tag_value(&tags, "project"), None);
    }

    #[test]
    fn build_tags_includes_project_when_default_project_set() {
        let mut identity = test_identity();
        identity.default_project = Some("bidcraft".to_string());
        let tags = build_tags(uuid::Uuid::new_v4(), &identity, "SM123").unwrap();
        assert_eq!(tag_value(&tags, "project"), Some("bidcraft"));
    }

    #[test]
    fn build_tags_includes_p_when_linked_pubkey_valid() {
        let mut identity = test_identity();
        let keys = nostr::Keys::generate();
        identity.linked_pubkey = Some(keys.public_key().to_bytes().to_vec());
        let tags = build_tags(uuid::Uuid::new_v4(), &identity, "SM123").unwrap();
        assert_eq!(
            tag_value(&tags, "p"),
            Some(keys.public_key().to_hex()).as_deref()
        );
    }

    #[test]
    fn build_tags_omits_p_when_linked_pubkey_malformed() {
        let mut identity = test_identity();
        identity.linked_pubkey = Some(vec![1, 2, 3]); // not a valid 32-byte pubkey
        let tags = build_tags(uuid::Uuid::new_v4(), &identity, "SM123").unwrap();
        assert_eq!(tag_value(&tags, "p"), None);
    }
}
