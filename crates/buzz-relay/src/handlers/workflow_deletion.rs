//! Canonical workflow deletes commit their signed event and effects together.

use std::sync::Arc;

use buzz_core::{kind::KIND_DELETION, kind::KIND_WORKFLOW_DEF, tenant::TenantContext, StoredEvent};
use nostr::{Event, TagKind};
use uuid::Uuid;

use super::ingest::IngestError;
use crate::state::AppState;

/// Persist an already-authorized canonical workflow deletion, if applicable.
/// Legacy name coordinates retain their existing handler.
pub(super) async fn persist(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    channel_id: Option<Uuid>,
) -> Result<Option<(StoredEvent, bool)>, IngestError> {
    if event.kind.as_u16() as u32 != KIND_DELETION {
        return Ok(None);
    }
    let Some(address) = event.tags.iter().find_map(|tag| {
        (tag.kind() == TagKind::a())
            .then(|| tag.content())
            .flatten()
    }) else {
        return Ok(None);
    };
    let mut parts = address.splitn(3, ':');
    if parts.next().and_then(|kind| kind.parse::<u32>().ok()) != Some(KIND_WORKFLOW_DEF) {
        return Ok(None);
    }
    let owner_hex = parts.next().unwrap_or_default();
    let coordinate = parts.next().unwrap_or_default();
    let Ok(workflow_id) = Uuid::parse_str(coordinate) else {
        return Ok(None);
    };
    let owner = hex::decode(owner_hex)
        .map_err(|_| IngestError::Rejected("invalid: workflow coordinate owner".into()))?;

    let mut tx = state
        .db
        .begin_event_write_transaction()
        .await
        .map_err(database_error)?;
    buzz_deletion::store(&state.db)
        .guard_transaction(&mut tx, tenant.community())
        .await
        .map_err(|error| {
            IngestError::Rejected(format!("restricted: community writes are fenced: {error}"))
        })?;
    let channels = buzz_db::workflow_deletion::delete_in_transaction(
        &mut tx,
        tenant.community(),
        &owner,
        workflow_id,
        coordinate,
        event.created_at.as_secs() as i64,
    )
    .await
    .map_err(database_error)?;
    // Do not skip effects for an exact replay: older relays may have accepted
    // the deletion while leaving a live definition behind.
    let stored =
        buzz_db::event::insert_event_in_transaction(&mut tx, tenant.community(), event, channel_id)
            .await
            .map_err(database_error)?;
    tx.commit().await.map_err(|error| {
        IngestError::Internal(format!("error: committing workflow deletion: {error}"))
    })?;
    for channel in channels {
        state
            .workflow_engine
            .invalidate_channel_workflows(tenant.community(), channel);
    }
    Ok(Some(stored))
}

fn database_error(error: buzz_db::DbError) -> IngestError {
    IngestError::Internal(format!("error: persisting workflow deletion: {error}"))
}
