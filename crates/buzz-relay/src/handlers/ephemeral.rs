//! Shared non-persistent submission path for WebSocket and HTTP events.

use std::sync::Arc;

use buzz_auth::Scope;
use buzz_core::{
    kind::{
        event_kind_u32, is_ephemeral, KIND_AGENT_OBSERVER_FRAME, KIND_AUTH, KIND_PRESENCE_UPDATE,
    },
    tenant::TenantContext,
    verification::verify_event,
    StoredEvent,
};
use buzz_pubsub::EventTopic;
use nostr::Event;
use tracing::warn;
use uuid::Uuid;

use super::ingest::{IngestAuth, IngestError};
use crate::state::AppState;

/// Validate auth and routing before any side effects. Special-purpose ephemeral
/// protocols retain their own transport and authorization handlers.
fn validate_route(event: &Event, auth: &IngestAuth) -> Result<Option<Uuid>, IngestError> {
    let kind = event_kind_u32(event);
    if !is_ephemeral(kind) || kind == KIND_AUTH || kind == KIND_AGENT_OBSERVER_FRAME {
        return Err(IngestError::Rejected(
            "invalid: kind requires a dedicated protocol handler".into(),
        ));
    }
    if auth.is_http() && kind == KIND_PRESENCE_UPDATE {
        return Err(IngestError::Rejected(
            "invalid: presence is only accepted via WebSocket".into(),
        ));
    }
    if event.pubkey != *auth.pubkey() {
        return Err(IngestError::AuthFailed(
            "invalid: event pubkey does not match authenticated identity".into(),
        ));
    }
    if !auth.scopes().is_empty() && !auth.scopes().contains(&Scope::MessagesWrite) {
        return Err(IngestError::AuthFailed(
            "restricted: insufficient scope for ephemeral events".into(),
        ));
    }
    let mut channel_id = None;
    for tag in event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "h")
    {
        if channel_id.is_some() {
            return Err(IngestError::Rejected(
                "invalid: multiple channel tags".into(),
            ));
        }
        let id = tag
            .content()
            .and_then(|value| value.parse::<Uuid>().ok())
            .filter(|id| !id.is_nil())
            .ok_or_else(|| IngestError::Rejected("invalid: malformed channel tag".into()))?;
        channel_id = Some(id);
    }
    if let Some(allowed) = auth.channel_ids() {
        if !channel_id.is_some_and(|id| allowed.contains(&id)) {
            return Err(IngestError::AuthFailed(
                "restricted: token does not have access to this channel".into(),
            ));
        }
    }
    Ok(channel_id)
}

/// Verify and fan out an ephemeral event without persistent ingest or effects.
pub(crate) async fn submit(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: Event,
    auth: &IngestAuth,
) -> Result<(), IngestError> {
    let channel_id = validate_route(&event, auth)?;
    super::ingest::map_serving_fence_state(
        buzz_deletion::store(&state.db)
            .is_serving_active(tenant.community())
            .await,
    )?;
    let event_clone = event.clone();
    let event_id = event.id.to_hex();
    let verify_result = tokio::task::spawn_blocking(move || verify_event(&event_clone)).await;

    match verify_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(IngestError::Rejected(format!("invalid: {e}"))),
        Err(_) => return Err(IngestError::Internal("error: internal error".to_string())),
    }

    // Special handling for presence events (kind:20001).
    if event_kind_u32(&event) == KIND_PRESENCE_UPDATE {
        // Accept both bare strings ("online") and legacy JSON ({"status":"online"}).
        let raw = event.content.to_string();
        let status = if raw.starts_with('{') {
            serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| v.get("status")?.as_str().map(String::from))
                .unwrap_or(raw)
        } else if raw.len() > 128 {
            let mut end = 128;
            while !raw.is_char_boundary(end) {
                end -= 1;
            }
            raw[..end].to_string()
        } else {
            raw
        };

        // Presence mutation is the inclusion contract for the live fan-out
        // below: a client that observes the fanned-out event treats a later
        // snapshot as reflecting it (see `synthesize_presence` in
        // `api/bridge.rs`, which reads Redis). If the mutation failed we
        // published nothing — so we must also fan out nothing and reject the
        // ACK, or a snapshot later "confirms" stale storage over a live event
        // the sender believes was delivered.
        if status == "offline" {
            if let Err(e) = state.pubsub.clear_presence(tenant, auth.pubkey()).await {
                warn!(
                    conn_id = ?auth.conn_id(),
                    event_id = %event_id,
                    "Presence clear failed, refusing publish and fan-out: {e}"
                );
                // Internal, not Rejected: a storage outage is a server
                // failure, so the dispatcher must count it under the
                // `error` reason, not as client-invalid input.
                return Err(IngestError::Internal(
                    "error: presence storage unavailable".to_string(),
                ));
            }
        } else if let Err(e) = state
            .pubsub
            .set_presence(tenant, auth.pubkey(), &status)
            .await
        {
            warn!(
                conn_id = ?auth.conn_id(),
                event_id = %event_id,
                "Presence set failed, refusing publish and fan-out: {e}"
            );
            // Internal for the same reason as the clear arm above.
            return Err(IngestError::Internal(
                "error: presence storage unavailable".to_string(),
            ));
        }

        // Presence is a channel-less ephemeral event. After updating Redis
        // presence state, let it fall through to the shared global ephemeral
        // publish/fan-out path below so other relay nodes receive the live delta.
    }

    // Check channel membership before publishing other ephemeral events.
    if let Some(ch_id) = channel_id {
        // Membership refusals are client-input rejections, and the shared
        // gate's message text is surfaced verbatim exactly as before this
        // typed classification; no behavior change on this path.
        super::ingest::check_channel_membership(
            tenant,
            state,
            ch_id,
            &auth.principal_pubkey_bytes(),
            None,
        )
        .await
        .map_err(IngestError::Rejected)?;

        // Mark as local before Redis publish to prevent double-delivery when
        // the event comes back through the Redis subscriber loop.
        state.mark_local_event(tenant.community(), &event.id);

        if let Err(e) = state
            .pubsub
            .publish_event(tenant, EventTopic::Channel(ch_id), &event)
            .await
        {
            state
                .local_event_ids
                .invalidate(&(tenant.community(), event.id.to_bytes()));
            warn!(event_id = %event_id, "Ephemeral publish failed: {e}");
            return Err(IngestError::Internal(
                "error: ephemeral delivery unavailable".into(),
            ));
        }

        // Direct fan-out to local WS subscribers, through the guarded send path
        // so a stale subscription on a removed/non-member connection cannot
        // receive this private-channel ephemeral event.
        // Pass the channel_id so fan_out() uses the channel-kind index.
        let stored_event = StoredEvent::new(event.clone(), Some(ch_id));
        super::event::fan_out_event_to_local_subscribers(state, tenant.community(), &stored_event)
            .await;
    } else {
        // Channel-less ephemeral events (e.g., NIP-AB pairing kind:24134).
        //
        // Sentinel pattern: we use `Uuid::nil()` (all-zeros UUID) as a
        // "global channel" routing key in Redis pub/sub. This lets other relay
        // nodes receive and fan out these events without any real channel_id.
        // The nil UUID is ONLY a Redis routing key — it never reaches the DB.
        // On the receiving end (main.rs subscriber loop), `is_nil()` is checked
        // and converted back to `None` so `fan_out()` uses the global index.
        state.mark_local_event(tenant.community(), &event.id);

        if let Err(e) = state
            .pubsub
            .publish_event(tenant, EventTopic::Global, &event)
            .await
        {
            state
                .local_event_ids
                .invalidate(&(tenant.community(), event.id.to_bytes()));
            warn!(event_id = %event_id, "Ephemeral global publish failed: {e}");
            return Err(IngestError::Internal(
                "error: ephemeral delivery unavailable".into(),
            ));
        }

        // Direct fan-out to local WS subscribers through the guarded send path.
        // Pass channel_id=None so fan_out() uses the global subscriber index;
        // filter_fanout_by_access no-ops for channel-less events except the
        // author-only-kind gate.
        let stored_event = StoredEvent::new(event.clone(), None);
        super::event::fan_out_event_to_local_subscribers(state, tenant.community(), &stored_event)
            .await;
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn auth(keys: &Keys, channels: Option<Vec<Uuid>>, scopes: Vec<Scope>) -> IngestAuth {
        IngestAuth::Nip42 {
            pubkey: keys.public_key(),
            scopes,
            channel_ids: channels,
            conn_id: Uuid::new_v4(),
        }
    }

    fn event(keys: &Keys, tags: &[Vec<String>]) -> Event {
        EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_AGENT_ACTIVITY_SNAPSHOT as u16),
            "{}",
        )
        .tags(tags.iter().map(|tag| Tag::parse(tag).unwrap()))
        .sign_with_keys(keys)
        .unwrap()
    }

    #[test]
    fn malformed_channel_never_falls_back_to_global_delivery() {
        let keys = Keys::generate();
        let unrestricted = auth(&keys, None, vec![]);
        let id = Uuid::new_v4().to_string();
        for tags in [
            vec![vec!["h".into()]],
            vec![vec!["h".into(), "not-a-channel".into()]],
            vec![vec!["h".into(), Uuid::nil().to_string()]],
            vec![vec!["h".into(), id.clone()], vec!["h".into(), id.clone()]],
        ] {
            assert!(validate_route(&event(&keys, &tags), &unrestricted).is_err());
        }
        assert_eq!(
            validate_route(&event(&keys, &[]), &unrestricted).unwrap(),
            None
        );
    }

    #[test]
    fn channel_scope_and_authenticated_author_bound_ephemeral_publish() {
        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        let scoped = auth(&keys, Some(vec![channel]), vec![Scope::MessagesWrite]);
        let valid = event(&keys, &[vec!["h".into(), channel.to_string()]]);
        assert_eq!(validate_route(&valid, &scoped).unwrap(), Some(channel));
        assert!(validate_route(&event(&keys, &[]), &scoped).is_err());
        let wrong = event(&keys, &[vec!["h".into(), Uuid::new_v4().to_string()]]);
        assert!(validate_route(&wrong, &scoped).is_err());
        assert!(validate_route(&valid, &auth(&Keys::generate(), None, vec![])).is_err());
        assert!(validate_route(&valid, &auth(&keys, None, vec![Scope::MessagesRead])).is_err());
    }
}
