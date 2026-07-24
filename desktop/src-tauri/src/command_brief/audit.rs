//! Encrypted NIP-CB signing, durable spool, owner view, and republish boundary.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use buzz_core_pkg::command_brief::{
    decrypt_command_brief_event, CommandBriefEventPayload, CommandBriefLifecycleState,
    COMMAND_BRIEF_PAYLOAD_VERSION,
};
use chrono::Utc;
use nostr::{Event, JsonUtil, Keys};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

use super::orchestrator::{BriefFuture, BriefPersistence, BriefPersistenceError};
use super::store::{
    insert_spool_event, latest_event_id, list_due_spool_events, mark_publish_failed,
    mark_published, open_command_brief_store, SpoolInsert,
};
use super::types::{CommandBrief, PublicationState, PublishedCommandBrief};

/// Boxed relay-publication future.
pub type AuditPublishFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'a>>;

/// Exact signed-event relay boundary. Implementations must not re-sign.
pub trait BriefAuditPublisher: Send + Sync {
    /// Publish one already-signed event, treating duplicate ID acceptance as success.
    fn publish<'a>(&'a self, event: Event) -> AuditPublishFuture<'a>;
}

/// Production publisher that submits the exact signed event through Buzz's
/// authenticated relay path.
pub struct RelayBriefAuditPublisher {
    app: tauri::AppHandle,
}

impl RelayBriefAuditPublisher {
    /// Bind publication to the running desktop application.
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl BriefAuditPublisher for RelayBriefAuditPublisher {
    fn publish<'a>(&'a self, event: Event) -> AuditPublishFuture<'a> {
        Box::pin(async move {
            let state = self.app.state::<crate::app_state::AppState>();
            crate::relay::submit_signed_event(&event, &state)
                .await
                .map(|_| ())
                .map_err(|_| ())
        })
    }
}

/// Redacted persistence error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BriefAuditError {
    /// Cooperative cancellation won before durable commit.
    Cancelled,
    /// Validation, signing, or durable local persistence failed.
    Failed,
}

/// Owner-bound encrypted audit service.
pub struct EncryptedBriefAudit {
    path: PathBuf,
    owner_keys: Keys,
    publisher: Arc<dyn BriefAuditPublisher>,
}

impl std::fmt::Debug for EncryptedBriefAudit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EncryptedBriefAudit")
            .finish_non_exhaustive()
    }
}

impl EncryptedBriefAudit {
    /// Construct an audit service bound to the currently unlocked owner.
    pub fn new(path: PathBuf, owner_keys: Keys, publisher: Arc<dyn BriefAuditPublisher>) -> Self {
        Self {
            path,
            owner_keys,
            publisher,
        }
    }

    /// Encrypt, sign, durably spool, and best-effort publish a terminal brief.
    ///
    /// Relay failure returns a valid queued envelope: local completion remains
    /// durable and reconnect republishes the exact same signed event ID.
    pub async fn persist_terminal(
        &self,
        brief: &CommandBrief,
        cancellation: CancellationToken,
    ) -> Result<PublishedCommandBrief, BriefAuditError> {
        if cancellation.is_cancelled() {
            return Err(BriefAuditError::Cancelled);
        }
        let owner = self.owner_keys.public_key().to_hex();
        let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
        let previous =
            latest_event_id(&conn, &owner, brief.run_id()).map_err(|_| BriefAuditError::Failed)?;
        let lifecycle_state = if brief.is_degraded() {
            CommandBriefLifecycleState::Degraded
        } else {
            CommandBriefLifecycleState::Completed
        };
        let payload = CommandBriefEventPayload {
            version: COMMAND_BRIEF_PAYLOAD_VERSION,
            classification: "OFFICIAL".into(),
            run_id: brief.run_id().to_string(),
            schedule_id: brief.schedule_id().to_string(),
            lifecycle_state,
            occurred_at: brief.generated_at().to_string(),
            frozen_snapshot_id: brief.snapshot_id().to_string(),
            final_brief: Some(serde_json::to_value(brief).map_err(|_| BriefAuditError::Failed)?),
            failure: None,
            previous_lifecycle_event_id: previous.clone(),
        };
        let event = crate::events::build_command_brief_lifecycle_event(&self.owner_keys, &payload)
            .map_err(|_| BriefAuditError::Failed)?;
        if cancellation.is_cancelled() {
            return Err(BriefAuditError::Cancelled);
        }
        let event_id = event.id.to_hex();
        let status = match lifecycle_state {
            CommandBriefLifecycleState::Completed => "completed",
            CommandBriefLifecycleState::Degraded => "degraded",
            CommandBriefLifecycleState::Cancelled => "cancelled",
            CommandBriefLifecycleState::Failed => "failed",
        };
        insert_spool_event(
            &conn,
            SpoolInsert {
                owner_pubkey: owner.clone(),
                run_id: brief.run_id().to_string(),
                event_id: event_id.clone(),
                status: status.to_string(),
                previous_event_id: previous,
                encrypted_payload: event.content.clone(),
                raw_event: event.as_json(),
                created_at: event.created_at.as_secs() as i64,
            },
        )
        .map_err(|_| BriefAuditError::Failed)?;
        drop(conn);

        let publication_state = if self.publisher.publish(event).await.is_ok() {
            let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
            mark_published(&conn, &owner, &event_id, Utc::now().timestamp())
                .map_err(|_| BriefAuditError::Failed)?;
            PublicationState::Published
        } else {
            let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
            mark_publish_failed(&conn, &owner, &event_id, Utc::now().timestamp())
                .map_err(|_| BriefAuditError::Failed)?;
            PublicationState::Queued
        };
        Ok(PublishedCommandBrief::new(
            brief.clone(),
            event_id,
            publication_state,
        ))
    }

    /// Republish a bounded due batch for only this unlocked owner.
    pub async fn republish_due(&self, now: i64) -> Result<usize, BriefAuditError> {
        let owner = self.owner_keys.public_key().to_hex();
        let rows = {
            let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
            list_due_spool_events(&conn, &owner, now, 64).map_err(|_| BriefAuditError::Failed)?
        };
        let mut published = 0;
        for row in rows {
            let event = Event::from_json(&row.raw_event).map_err(|_| BriefAuditError::Failed)?;
            if event.id.to_hex() != row.event_id
                || event.pubkey.to_hex() != owner
                || !event.verify_id()
                || !event.verify_signature()
            {
                return Err(BriefAuditError::Failed);
            }
            let accepted = self.publisher.publish(event).await.is_ok();
            let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
            if accepted {
                mark_published(&conn, &owner, &row.event_id, Utc::now().timestamp())
                    .map_err(|_| BriefAuditError::Failed)?;
                published += 1;
            } else {
                mark_publish_failed(&conn, &owner, &row.event_id, now)
                    .map_err(|_| BriefAuditError::Failed)?;
            }
        }
        Ok(published)
    }

    /// Decrypt and return only the authoritative validated brief view model.
    pub fn decrypt_for_current_owner(
        &self,
        event: &Event,
    ) -> Result<CommandBrief, BriefAuditError> {
        let payload = decrypt_command_brief_event(&self.owner_keys, event)
            .map_err(|_| BriefAuditError::Failed)?;
        let value = payload.final_brief.ok_or(BriefAuditError::Failed)?;
        CommandBrief::try_from(value).map_err(|_| BriefAuditError::Failed)
    }
}

impl BriefPersistence for EncryptedBriefAudit {
    fn persist<'a>(
        &'a self,
        brief: &'a CommandBrief,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<PublishedCommandBrief, BriefPersistenceError>> {
        Box::pin(async move {
            self.persist_terminal(brief, cancellation)
                .await
                .map_err(|error| match error {
                    BriefAuditError::Cancelled => BriefPersistenceError::Cancelled,
                    BriefAuditError::Failed => BriefPersistenceError::Failed,
                })
        })
    }
}
