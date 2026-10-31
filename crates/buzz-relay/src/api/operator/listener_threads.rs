//! Explicit target-consented subscriptions to a single community thread.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use super::{
    api_error, authorize_operator_listener_request, internal_error, verify_registration_proof,
};
use crate::state::AppState;
use buzz_core::CommunityId;

const PATH: &str = "/operator/listener/threads";
const MAX_REGISTRATIONS: usize = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadBinding {
    community_id: Uuid,
    channel_id: Uuid,
    root_event_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    registrations: Vec<nostr::Event>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Removal {
    community_id: Uuid,
    channel_id: Uuid,
    root_event_id: String,
    pubkey: String,
}

fn event_id(value: &str) -> Result<Vec<u8>, (StatusCode, Json<Value>)> {
    let bytes = hex::decode(value).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "expected lowercase 64-character hex key or event ID",
        )
    })?;
    if bytes.len() != 32 || value != value.to_lowercase() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "expected lowercase 64-character hex key or event ID",
        ));
    }
    Ok(bytes)
}

type ParsedRegistration = (ThreadBinding, Vec<u8>, Vec<u8>);

fn parse_registration(
    proof: &nostr::Event,
    listener: &nostr::PublicKey,
    url: &str,
    now: u64,
) -> Result<ParsedRegistration, (StatusCode, Json<Value>)> {
    let target = verify_registration_proof(proof, listener, url, now)
        .map_err(|reason| api_error(StatusCode::BAD_REQUEST, reason))?;
    if proof.content.len() > 1024 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "thread registration content exceeds limit",
        ));
    }
    let binding: ThreadBinding = serde_json::from_str(&proof.content).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid thread registration content",
        )
    })?;
    if binding.community_id.is_nil() || binding.channel_id.is_nil() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "thread scope UUIDs must not be nil",
        ));
    }
    let root = event_id(&binding.root_event_id)?;
    Ok((binding, target, root))
}

/// Register/renew target-signed watches. Every proof binds one exact thread;
/// server expiry is 30 days, with at most 128 watches per target/community.
pub async fn register_listener_threads(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let listener =
        authorize_operator_listener_request(&state, &headers, "POST", PATH, &body).await?;
    let request: Registration = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid thread registration JSON"))?;
    if request.registrations.is_empty() || request.registrations.len() > MAX_REGISTRATIONS {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "registrations must contain exactly one proof",
        ));
    }
    let origin = state
        .config
        .relay_operator_api_origin
        .as_deref()
        .ok_or_else(|| internal_error("operator API origin is not configured"))?;
    let parsed = request
        .registrations
        .iter()
        .map(|proof| {
            parse_registration(
                proof,
                &listener,
                &format!("{origin}{PATH}"),
                nostr::Timestamp::now().as_secs(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    // One binding per request makes registration atomic. Retry an unknown
    // result with a fresh proof for the same tuple.
    for (binding, target, root) in parsed {
        let accepted = state
            .db
            .register_operator_listener_thread(
                CommunityId::from_uuid(binding.community_id),
                listener.as_bytes(),
                &target,
                binding.channel_id,
                &root,
            )
            .await
            .map_err(|_| internal_error("register operator-listener thread failed"))?;
        if !accepted {
            return Err(api_error(StatusCode::FORBIDDEN, "thread registration unavailable: verify target registration, membership, root and capacity"));
        }
    }
    Ok(Json(
        serde_json::json!({"expires_after_seconds":buzz_db::operator_listener_threads::THREAD_WATCH_RETENTION.num_seconds()}),
    ))
}

/// Delete the exact authenticated listener's watch. No new target proof is needed
/// to relinquish delivery; this cannot create or broaden a subscription.
pub async fn remove_listener_threads(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let listener =
        authorize_operator_listener_request(&state, &headers, "DELETE", PATH, &body).await?;
    let request: Removal = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid thread removal JSON"))?;
    state
        .db
        .remove_operator_listener_thread(
            CommunityId::from_uuid(request.community_id),
            listener.as_bytes(),
            &event_id(&request.pubkey)?,
            request.channel_id,
            &event_id(&request.root_event_id)?,
        )
        .await
        .map_err(|_| internal_error("remove operator-listener thread failed"))?;
    Ok(Json(serde_json::json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    #[test]
    fn thread_proof_binds_listener_endpoint_scope_and_fresh_target_consent() {
        let target = Keys::generate();
        let listener = Keys::generate();
        let url = "https://operator.example/operator/listener/threads";
        let now = Timestamp::now().as_secs();
        let content = serde_json::json!({"community_id":Uuid::new_v4(),"channel_id":Uuid::new_v4(),"root_event_id":"ab".repeat(32)}).to_string();
        let proof = |listener: &Keys, url: &str, content: &str, created: u64| {
            EventBuilder::new(
                Kind::Custom(buzz_core::kind::KIND_OPERATOR_LISTENER_REGISTRATION as u16),
                content,
            )
            .tags([
                Tag::public_key(listener.public_key()),
                Tag::custom(nostr::TagKind::custom("u"), [url]),
            ])
            .custom_created_at(Timestamp::from(created))
            .sign_with_keys(&target)
            .unwrap()
        };
        let valid = proof(&listener, url, &content, now);
        let (binding, signed_target, root) =
            parse_registration(&valid, &listener.public_key(), url, now).unwrap();
        assert_eq!(signed_target, target.public_key().to_bytes());
        assert_eq!(root, vec![0xab; 32]);
        assert_eq!(binding.root_event_id, "ab".repeat(32));
        for rejected in [
            proof(&Keys::generate(), url, &content, now),
            proof(
                &listener,
                "https://other.example/operator/listener/threads",
                &content,
                now,
            ),
            proof(&listener, url, &content, now - 61),
            proof(
                &listener,
                url,
                &content.replace(&"ab".repeat(32), &"AB".repeat(32)),
                now,
            ),
            proof(&listener, url, "{}", now),
        ] {
            assert!(parse_registration(&rejected, &listener.public_key(), url, now).is_err());
        }
        let mut tampered = valid;
        tampered.content = content.replace("ab", "cd");
        assert!(parse_registration(&tampered, &listener.public_key(), url, now).is_err());
    }
}
