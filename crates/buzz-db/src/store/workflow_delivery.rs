//! Atomic workflow output persistence and captured-revision reads.
use crate::{event, insert_mentions_in_transaction, Db, Result};
use buzz_core::{tenant::CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use uuid::Uuid;

impl Db {
    /// Atomically persist a visible event, its thread metadata/mentions, and all
    /// required notifications. No caller may publish any row until this commits.
    /// Cancellation or any insert failure rolls the entire bundle back.
    #[datastore_span(name = "insert_event_with_notifications", system = "postgresql")]
    pub async fn insert_event_with_notifications(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        channel_id: Uuid,
        thread_meta: Option<event::ThreadMetadataParams<'_>>,
        notifications: &[nostr::Event],
    ) -> Result<Vec<(StoredEvent, bool)>> {
        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community_id)
            .await?;
        let mut stored = Vec::with_capacity(1 + notifications.len());
        let message = event::insert_event_with_thread_metadata_tx(
            &mut tx,
            community_id,
            event,
            Some(channel_id),
            thread_meta,
        )
        .await?;
        insert_mentions_in_transaction(&mut tx, community_id, event, Some(channel_id)).await?;
        stored.push(message);
        for notification in notifications {
            let row = event::insert_event_in_transaction(
                &mut tx,
                community_id,
                notification,
                Some(channel_id),
            )
            .await?;
            insert_mentions_in_transaction(&mut tx, community_id, notification, Some(channel_id))
                .await?;
            stored.push(row);
        }
        tx.commit().await?;
        Ok(stored)
    }

    /// Read a captured workflow definition without reviving explicitly deleted revisions.
    #[datastore_span(name = "get_workflow_revision", system = "postgresql")]
    pub async fn get_workflow_revision(
        &self,
        community_id: CommunityId,
        id_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        event::get_workflow_revision(&self.pool, community_id, id_bytes).await
    }
}
