//! Owner-only scheduled delivery. The queue is relay-owned execution state;
//! dispatched messages still use the ordinary signed-event ingest pipeline.

use crate::{
    api::{api_error, bridge::authenticate_member_post, internal_error},
    state::AppState,
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use buzz_core::TenantContext;
use nostr::Event;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Fixed, NIP-98-authenticated queue route.
pub const PATH: &str = "/scheduled-messages";
const MAX_DAYS: i64 = 120 * 86_400;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    List {},
    Create {
        id: Uuid,
        event: Event,
    },
    Update {
        id: Uuid,
        revision: i64,
        event: Event,
    },
    Cancel {
        id: Uuid,
        revision: i64,
    },
    SendNow {
        id: Uuid,
        revision: i64,
        event: Event,
    },
}

fn validate_message(event: &Event, owner: nostr::PublicKey) -> Result<(Uuid, i64), &'static str> {
    if event.pubkey != owner
        || event.kind.as_u16() != 9
        || event.content.trim().is_empty()
        || event.content.len() > 24 * 1024
    {
        return Err("Invalid scheduled message");
    }
    let at = i64::try_from(event.created_at.as_secs()).map_err(|_| "Invalid send time")?;
    let channels: Vec<_> = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().is_some_and(|s| s == "h"))
        .collect();
    let [channel] = channels.as_slice() else {
        return Err("Choose one conversation");
    };
    let id = channel
        .as_slice()
        .get(1)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or("Invalid conversation")?;
    if serde_json::to_vec(event).map_or(true, |v| v.len() > 32 * 1024) {
        return Err("Message is too large");
    }
    Ok((id, at))
}

fn validate_send_time(at: i64, immediate: bool, now: i64) -> Result<(), &'static str> {
    if (immediate && !(now - 300..=now + 5).contains(&at))
        || (!immediate && (at <= now || at > now + MAX_DAYS))
    {
        return Err("Choose a future time within 120 days");
    }
    Ok(())
}

/// Handle bounded queue reads and compare-and-swap mutations for the proved owner.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config.scheduled_messages_enabled {
        return api_error(StatusCode::NOT_FOUND, "Scheduled delivery is not enabled")
            .into_response();
    }
    if body.len() > 40 * 1024 {
        return api_error(StatusCode::PAYLOAD_TOO_LARGE, "Message is too large").into_response();
    }
    let (tenant, owner) = match authenticate_member_post(&state, &slot, &headers, PATH, &body).await
    {
        Ok(v) => v,
        Err(e) => return e,
    };
    let request: Request = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return api_error(StatusCode::BAD_REQUEST, "Invalid scheduled-message request")
                .into_response()
        }
    };
    let owner_bytes = owner.to_bytes();
    if matches!(request, Request::List {}) {
        return match state
            .db
            .scheduled_list(tenant.community(), &owner_bytes)
            .await
        {
            Ok(messages) => {
                Json(json!({"messages":messages,"sent_history_limit":100})).into_response()
            }
            Err(_) => internal_error("Could not read scheduled messages").into_response(),
        };
    }
    let (id, revision, event, immediate) = match request {
        Request::Create { id, event } => (id, None, Some(event), false),
        Request::Update {
            id,
            revision,
            event,
        } => (id, Some(revision), Some(event), false),
        Request::Cancel { id, revision } => (id, Some(revision), None, false),
        Request::SendNow {
            id,
            revision,
            event,
        } => (id, Some(revision), Some(event), true),
        Request::List {} => {
            return api_error(StatusCode::BAD_REQUEST, "Invalid operation").into_response()
        }
    };
    let value = match event.as_ref().map(serde_json::to_value).transpose() {
        Ok(v) => v,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "Invalid message").into_response(),
    };
    let now = chrono::Utc::now().timestamp();
    let mut at = now;
    if let Some(event) = &event {
        let (channel, send_at) = match validate_message(event, owner) {
            Ok(v) => v,
            Err(reason) => return api_error(StatusCode::BAD_REQUEST, reason).into_response(),
        };
        at = send_at;
        let verify = event.clone();
        if !tokio::task::spawn_blocking(move || {
            buzz_core::verification::verify_event(&verify).is_ok()
        })
        .await
        .unwrap_or(false)
        {
            return api_error(StatusCode::BAD_REQUEST, "Invalid message signature").into_response();
        }
        // An exact, already-committed request is an acknowledgement, not a new
        // admission. Its signed timestamp may no longer be fresh on retry.
        match state
            .db
            .scheduled_replayed(
                tenant.community(),
                &owner_bytes,
                id,
                revision,
                value.as_ref().unwrap_or(&Value::Null),
            )
            .await
        {
            Ok(true) => return Json(json!({"accepted":true})).into_response(),
            Ok(false) => {}
            Err(_) => {
                return internal_error("Could not reconcile scheduled message").into_response()
            }
        }
        if let Err(reason) = validate_send_time(send_at, immediate, now) {
            return api_error(StatusCode::BAD_REQUEST, reason).into_response();
        }
        match state
            .db
            .get_member_role(tenant.community(), channel, &owner_bytes)
            .await
        {
            Ok(Some(role)) if role != "guest" => {}
            Ok(_) => {
                return api_error(
                    StatusCode::FORBIDDEN,
                    "You cannot post in this conversation",
                )
                .into_response()
            }
            Err(_) => return internal_error("Could not check conversation access").into_response(),
        }
        if immediate {
            let items = match state
                .db
                .scheduled_list(tenant.community(), &owner_bytes)
                .await
            {
                Ok(v) => v,
                Err(_) => {
                    return internal_error("Could not read scheduled message").into_response()
                }
            };
            let matches = items.iter().any(|item| {
                item.id == id
                    && item.event["content"] == event.content
                    && item.event["tags"]
                        == serde_json::to_value(&event.tags).unwrap_or(Value::Null)
            });
            if !matches {
                return api_error(
                    StatusCode::CONFLICT,
                    "Message changed; refresh before sending",
                )
                .into_response();
            }
        }
    }
    match state
        .db
        .scheduled_mutate(
            tenant.community(),
            &owner_bytes,
            id,
            revision,
            value.as_ref(),
            at,
        )
        .await
    {
        Ok(()) => Json(json!({"accepted":true})).into_response(),
        Err(buzz_db::DbError::InvalidData(_)) => api_error(
            StatusCode::CONFLICT,
            "Message changed or the queue is full; refresh and try again",
        )
        .into_response(),
        Err(_) => internal_error("Could not update scheduled message").into_response(),
    }
}

/// One bounded sweep, shared by the daemon and live integration tests.
pub async fn dispatch_due(state: &Arc<AppState>) -> buzz_db::Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let due = state.db.scheduled_due(now).await?;
    let mut sent = 0;
    for (community, host, owner, id) in due {
        let Some(mut claim) = state.db.scheduled_claim(community, &owner, id, now).await? else {
            continue;
        };
        let event: Event = match serde_json::from_value(claim.message.event.clone()) {
            Ok(event) => event,
            Err(_) => {
                claim
                    .finish(
                        "failed",
                        Some("Stored message is invalid. Cancel and recreate it."),
                        now,
                    )
                    .await?;
                continue;
            }
        };
        let tenant = TenantContext::resolved(community, host);
        let stored = state
            .db
            .get_event_by_id_including_deleted(community, &event.id.to_bytes())
            .await?;
        if stored.is_some() {
            claim.finish("sent", None, now).await?;
            sent += 1;
            continue;
        }
        let member = crate::api::relay_members::enforce_relay_membership(
            state,
            &tenant,
            &owner,
            None,
            Some(now as u64),
            None,
        )
        .await;
        if let Err(refusal) = member {
            if refusal.status().is_server_error() {
                claim
                    .finish(
                        "scheduled",
                        Some("Access check delayed; the relay will retry."),
                        now,
                    )
                    .await?;
                continue;
            }
            claim
                .finish(
                    "failed",
                    Some("Community access could not be confirmed. Review before retrying."),
                    now,
                )
                .await?;
            continue;
        }
        let outcome = crate::handlers::ingest::ingest_scheduled(
            state,
            &tenant,
            &mut claim,
            crate::handlers::ingest::IngestAuth::Http {
                pubkey: event.pubkey,
                scopes: buzz_auth::Scope::all_known(),
                auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
            },
        )
        .await;
        match outcome {
            Ok(result) if result.accepted => {
                claim.finish("sent", None, now).await?;
                sent += 1;
            }
            Err(crate::handlers::ingest::IngestError::Internal(_))
            | Err(crate::handlers::ingest::IngestError::Ban {
                refusal: crate::handlers::ingest::BanRefusal::Unavailable,
                ..
            }) => {
                claim
                    .finish(
                        "scheduled",
                        Some("Delivery delayed; the relay will retry."),
                        now,
                    )
                    .await?;
            }
            _ => {
                claim.finish("failed",Some("Delivery was refused. Review conversation access and message before retrying."),now).await?;
            }
        }
    }
    Ok(sent)
}

/// Run even when every client is closed. Row locks coordinate relay replicas.
pub async fn run(state: Arc<AppState>) {
    tokio::time::sleep(std::time::Duration::from_millis(
        (Uuid::new_v4().as_u128() % 1000) as u64,
    ))
    .await;
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(1));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticks.tick().await;
        if dispatch_due(&state).await.is_err() {
            tracing::warn!("Scheduled delivery sweep failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    #[test]
    fn validates_owner_time_and_single_conversation() {
        let keys = Keys::generate();
        let other = Keys::generate();
        let channel = Uuid::new_v4();
        let now = 1_800_000_000;
        let build = |kind, at, content: &str, tags: Vec<Tag>| {
            EventBuilder::new(Kind::Custom(kind), content)
                .tags(tags)
                .custom_created_at(Timestamp::from(at as u64))
                .sign_with_keys(&keys)
                .unwrap()
        };
        let tags = || vec![Tag::parse(["h".to_string(), channel.to_string()]).unwrap()];
        let validate = |event: &Event, owner, immediate, now| {
            validate_message(event, owner).and_then(|(channel, at)| {
                validate_send_time(at, immediate, now).map(|()| (channel, at))
            })
        };
        let good = build(9, now + 3600, "Tomorrow", tags());
        assert_eq!(
            validate(&good, keys.public_key(), false, now),
            Ok((channel, now + 3600))
        );
        assert!(validate(&good, other.public_key(), false, now).is_err());
        for bad in [
            build(1, now + 3600, "Wrong kind", tags()),
            build(9, now, "Past", tags()),
            build(9, now + MAX_DAYS + 1, "Too far", tags()),
            build(9, now + 3600, " ", tags()),
            build(9, now + 3600, "Missing channel", vec![]),
            build(9, now + 3600, "Two channels", [tags(), tags()].concat()),
            build(9, now + 3600, &"x".repeat(24 * 1024 + 1), tags()),
        ] {
            assert!(validate(&bad, keys.public_key(), false, now).is_err());
        }
        for (at, valid) in [
            (now - 301, false),
            (now - 300, true),
            (now + 5, true),
            (now + 6, false),
        ] {
            assert_eq!(
                validate(&build(9, at, "Now", tags()), keys.public_key(), true, now).is_ok(),
                valid
            );
        }
        assert!(serde_json::from_value::<Request>(
            json!({"action":"list","owner":other.public_key().to_hex()})
        )
        .is_err());
    }
}
