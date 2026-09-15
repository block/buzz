//! One experimental gate shared by HTTP and WebSocket ingest.

use std::{sync::Arc, time::Duration};

use buzz_core::{kind::*, tenant::TenantContext, StoredEvent};
use nostr::Event;

use super::ingest::{IngestError, IngestResult};
use crate::state::AppState;

pub(crate) fn check_enabled(enabled: bool, kind: u32) -> Result<(), IngestError> {
    if !enabled
        && matches!(
            kind,
            KIND_INTERACTION_PROMPT
                | KIND_INTERACTION_RESPONSE
                | KIND_INTERACTION_CLOSE
                | KIND_INTERACTION_STATE
        )
    {
        return Err(IngestError::Rejected(
            "restricted: experimental interactions are disabled".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn try_ingest(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    meta: Option<buzz_db::event::ThreadMetadataParams<'_>>,
) -> Result<Option<IngestResult>, IngestError> {
    let kind = event_kind_u32(event);
    if !matches!(
        kind,
        KIND_INTERACTION_PROMPT
            | KIND_INTERACTION_RESPONSE
            | KIND_INTERACTION_CLOSE
            | KIND_STREAM_MESSAGE
            | KIND_STREAM_MESSAGE_V2
            | KIND_FORUM_COMMENT
    ) {
        return Ok(None);
    }
    if !matches!(
        kind,
        KIND_INTERACTION_PROMPT | KIND_INTERACTION_RESPONSE | KIND_INTERACTION_CLOSE
    ) && buzz_core::nip10::parse_thread_markers(&event.tags)
        .resolve()
        .is_none()
    {
        return Ok(None);
    }
    let result = state
        .db
        .accept_interaction(tenant.community(), event, &state.relay_keypair, meta)
        .await
        .map_err(|e| match e {
            buzz_db::DbError::InvalidData(message) => {
                IngestError::Rejected(format!("invalid: {message}"))
            }
            buzz_db::DbError::AccessDenied(message) => {
                IngestError::Rejected(format!("restricted: {message}"))
            }
            other => IngestError::Internal(format!("error: accepting interaction: {other}")),
        })?;
    let Some(events) = result else {
        return Ok(None);
    };
    // A matched text answer remains an ordinary message, including its existing
    // workflow triggers. Run once on insertion, never on outbox retries/replays.
    for stored in &events {
        if matches!(
            event_kind_u32(&stored.event),
            KIND_STREAM_MESSAGE | KIND_STREAM_MESSAGE_V2 | KIND_FORUM_COMMENT
        ) {
            super::event::trigger_event_workflows(
                tenant,
                state,
                stored,
                event_kind_u32(&stored.event),
            );
        }
    }
    // No fire-and-forget dependency: accepted events are in the durable outbox.
    // The worker publishes them with tenant labels and the ordinary access gates.
    Ok(Some(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: if events.is_empty() {
            "duplicate".into()
        } else {
            String::new()
        },
    }))
}

/// Run bounded expiry and at-least-once Redis delivery. All failures retain
/// durable work; restart and multiple pods are safe (event IDs deduplicate).
pub async fn run_worker(state: Arc<AppState>) {
    let mut ticks = 0u64;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        ticks += 1;
        if ticks.is_multiple_of(5) {
            if let Err(error) = state.db.expire_interactions(&state.relay_keypair).await {
                tracing::error!(%error, "interaction expiry failed; will retry");
            }
        }
        if let Err(error) = deliver_pending(&state).await {
            tracing::error!(%error, "interaction delivery failed; durable outbox retained");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}

async fn deliver_pending(state: &Arc<AppState>) -> anyhow::Result<()> {
    for row in state.db.pending_interaction_events().await? {
        let tenant = TenantContext::resolved(row.community, row.host);
        state
            .pubsub
            .publish_event(
                &tenant,
                buzz_pubsub::EventTopic::Channel(row.channel),
                &row.event,
            )
            .await?;
        let stored = StoredEvent::new(row.event.clone(), Some(row.channel));
        super::event::enqueue_event_created_audit(
            &tenant,
            state,
            &stored,
            event_kind_u32(&row.event),
            &row.event.pubkey.to_hex(),
            &row.event.id.to_hex(),
        )
        .await;
        state
            .db
            .acknowledge_interaction_event(row.community, &row.event)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn experimental_gate_is_closed_for_every_new_write_kind_only() {
        for k in [
            KIND_INTERACTION_PROMPT,
            KIND_INTERACTION_RESPONSE,
            KIND_INTERACTION_CLOSE,
            KIND_INTERACTION_STATE,
        ] {
            assert!(check_enabled(false, k).is_err());
            assert!(check_enabled(true, k).is_ok());
        }
        for k in [KIND_STREAM_MESSAGE, KIND_REACTION, KIND_APPROVAL_GRANT] {
            assert!(check_enabled(false, k).is_ok());
        }
        assert!(is_relay_only_kind(KIND_INTERACTION_STATE));
    }
}
