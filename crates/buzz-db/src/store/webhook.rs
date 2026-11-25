//! NIP-WH delivery queue (`webhook_deliveries`).
//!
//! A delivery is kept only here, never in `events`, so no read path can serve
//! it. The subscription worker claims queued rows and writes
//! `webhook_records`.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use uuid::Uuid;

use crate::error::Result;
use crate::Db;

/// Whether [`Db::enqueue_webhook_delivery`] queued the event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Enqueued {
    /// The event is newly queued.
    New,
    /// An event with the same ID is already queued or was processed.
    Duplicate,
}

impl Db {
    /// Queue a validated delivery event for its community's subscriptions.
    ///
    /// Keyed by the signed event ID, so the gateway resending the same event
    /// is a [`Enqueued::Duplicate`] with no effect. Refused once the
    /// community's deletion has started.
    #[datastore_span(name = "enqueue_webhook_delivery", system = "postgresql")]
    pub async fn enqueue_webhook_delivery(
        &self,
        community: CommunityId,
        delivery: &nostr::Event,
        integration: Uuid,
        provider: &str,
    ) -> Result<Enqueued> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let inserted = sqlx::query(
            "INSERT INTO webhook_deliveries \
                 (community_id, delivery_event_id, integration_id, provider, event) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (community_id, delivery_event_id) DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(delivery.id.as_bytes().as_slice())
        .bind(integration)
        .bind(provider)
        .bind(sqlx::types::Json(delivery))
        .execute(tx.conn())
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(if inserted == 1 {
            Enqueued::New
        } else {
            Enqueued::Duplicate
        })
    }
}
