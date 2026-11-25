//! NIP-WH delivery queue (`webhook_deliveries`) and outcomes
//! (`webhook_records`).
//!
//! A delivery is kept only here, never in `events`, so no read path can serve
//! it. The subscription worker claims queued rows, evaluates the delivery's
//! subscriptions, and records one outcome per subscription. A claim moves
//! `next_attempt_at` to the end of its lease, so one rule finds both queued
//! rows and rows whose worker stopped: due and not finished.

use std::time::Duration;

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::error::Result;
use crate::{Db, DbError};
use buzz_core::StoredEvent;

/// NIP-AR type of a subscription artifact.
const SUBSCRIPTION_TYPE: &str = "buzz.webhook-subscription";
/// Communities returned per discovery page.
const DISCOVERY_PAGE: i64 = 64;

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

/// A delivery claimed by one worker until `claim_id` is finished or retried.
#[derive(Debug)]
pub struct ClaimedDelivery {
    /// The delivery's community.
    pub community: CommunityId,
    /// The signed delivery event, as the gateway sent it.
    pub event: nostr::Event,
    /// The integration the gateway routed it for.
    pub integration: Uuid,
    /// The provider from its `t` tag.
    pub provider: String,
    /// This claim; finishing or retrying a newer claim does nothing.
    pub claim_id: Uuid,
    /// Attempts so far, this one included.
    pub attempts: i32,
}

/// A current subscription to evaluate for one delivery.
#[derive(Debug)]
pub struct SubscriptionHead {
    /// The artifact `d`.
    pub subscription_id: Uuid,
    /// The current revision, which carries the filter and transform.
    pub revision: StoredEvent,
    /// The destination channel, the revision's `h`.
    pub channel_id: Uuid,
    /// Whether the revision's signer is now an owner or admin of the channel.
    pub signer_is_admin: bool,
}

/// The outcome of evaluating one subscription for one delivery.
#[derive(Debug)]
pub enum Outcome<'a> {
    /// The filter matched: publish this relay-signed record to the channel.
    Matched(&'a nostr::Event),
    /// The subscription's content or evaluation failed.
    Error(&'a str),
    /// The revision's signer is not a current owner or admin.
    SkippedSigner,
}

/// What [`Db::record_webhook_outcome`] stored.
#[derive(Debug)]
pub enum Recorded {
    /// The outcome, with the stored record when it matched.
    New(Option<Box<StoredEvent>>),
    /// The channel is missing, deleted or archived, so the match was
    /// recorded as `skipped_channel` and nothing was published.
    SkippedChannel,
    /// An earlier attempt already recorded this subscription.
    Already,
}

impl Db {
    /// Communities with a due delivery, and their hosts, after `after`, a
    /// page at a time.
    #[datastore_span(name = "due_webhook_delivery_communities", system = "postgresql")]
    pub async fn due_webhook_delivery_communities(
        &self,
        after: Option<CommunityId>,
    ) -> Result<Vec<(CommunityId, String)>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT c.id, c.host FROM communities c \
             WHERE ($1::uuid IS NULL OR c.id > $1) \
               AND c.archived_at IS NULL AND c.deleted_at IS NULL \
               AND c.deletion_state = 'active' \
               AND EXISTS ( \
                   SELECT 1 FROM webhook_deliveries d \
                   WHERE d.community_id = c.id \
                     AND d.state IN ('pending', 'evaluating') \
                     AND d.next_attempt_at <= now()) \
             ORDER BY c.id LIMIT $2",
        )
        .bind(after.map(|community| *community.as_uuid()))
        .bind(DISCOVERY_PAGE)
        .fetch_all(&mut *connection)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, host)| (CommunityId::from_uuid(id), host))
            .collect())
    }

    /// Claim up to `limit` due deliveries of `community` for `lease`.
    #[datastore_span(name = "claim_webhook_deliveries", system = "postgresql")]
    pub async fn claim_webhook_deliveries(
        &self,
        community: CommunityId,
        limit: i64,
        lease: Duration,
    ) -> Result<Vec<ClaimedDelivery>> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        let rows = sqlx::query(
            "WITH due AS ( \
                 SELECT delivery_event_id FROM webhook_deliveries \
                 WHERE community_id = $1 AND state IN ('pending', 'evaluating') \
                   AND next_attempt_at <= now() \
                 ORDER BY next_attempt_at \
                 FOR UPDATE SKIP LOCKED LIMIT $2) \
             UPDATE webhook_deliveries d \
             SET state = 'evaluating', claim_id = gen_random_uuid(), \
                 attempts = d.attempts + 1, next_attempt_at = now() + $3 \
             FROM due \
             WHERE d.community_id = $1 AND d.delivery_event_id = due.delivery_event_id \
             RETURNING d.event, d.integration_id, d.provider, d.claim_id, d.attempts",
        )
        .bind(community.as_uuid())
        .bind(limit)
        .bind(lease)
        .fetch_all(tx.conn())
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let sqlx::types::Json(event) = row.try_get("event")?;
                Ok(ClaimedDelivery {
                    community,
                    event,
                    integration: row.try_get("integration_id")?,
                    provider: row.try_get("provider")?,
                    claim_id: row.try_get("claim_id")?,
                    attempts: row.try_get("attempts")?,
                })
            })
            .collect()
    }

    /// The current subscriptions to `integration` in `community` that have no
    /// outcome for `delivery` yet, with whether each signer is still an admin.
    ///
    /// Starts from the community's artifact heads, so its cost does not grow
    /// with the records an integration has published, which carry the same
    /// `integration` tag.
    #[datastore_span(name = "webhook_subscriptions_to_evaluate", system = "postgresql")]
    pub async fn webhook_subscriptions_to_evaluate(
        &self,
        community: CommunityId,
        integration: Uuid,
        delivery: &nostr::EventId,
    ) -> Result<Vec<SubscriptionHead>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        let rows = sqlx::query(
            "SELECT e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, \
                    e.received_at, e.channel_id, h.artifact_id, h.channel_id AS home, \
                    EXISTS ( \
                        SELECT 1 FROM channel_members m \
                        JOIN channels c ON c.community_id = m.community_id \
                            AND c.id = m.channel_id AND c.deleted_at IS NULL \
                        WHERE m.community_id = h.community_id AND m.channel_id = h.channel_id \
                          AND m.pubkey = e.pubkey AND m.removed_at IS NULL \
                          AND m.role IN ('owner', 'admin')) AS signer_is_admin \
             FROM artifact_heads h \
             JOIN events e ON e.community_id = h.community_id AND e.id = h.event_id \
             WHERE h.community_id = $1 AND h.artifact_type = $2 AND NOT h.deleted \
               AND e.kind = 45010 AND e.deleted_at IS NULL AND e.tags @> $3 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM webhook_records r \
                   WHERE r.community_id = h.community_id AND r.delivery_event_id = $4 \
                     AND r.subscription_id = h.artifact_id) \
             ORDER BY h.artifact_id",
        )
        .bind(community.as_uuid())
        .bind(SUBSCRIPTION_TYPE)
        .bind(serde_json::json!([[
            "integration",
            integration.to_string()
        ]]))
        .bind(delivery.as_bytes().as_slice())
        .fetch_all(&mut *connection)
        .await?;
        let mut heads = Vec::with_capacity(rows.len());
        for row in rows {
            let subscription_id = row.try_get("artifact_id")?;
            let channel_id = row.try_get("home")?;
            let signer_is_admin = row.try_get("signer_is_admin")?;
            if let Some(revision) = crate::event::row_to_stored_event(row)? {
                heads.push(SubscriptionHead {
                    subscription_id,
                    revision,
                    channel_id,
                    signer_is_admin,
                });
            }
        }
        Ok(heads)
    }

    /// Record one subscription's outcome for `delivery`, and store its record
    /// when it matched, in one transaction: a retry never publishes a second
    /// record, and a record is never stored without its outcome.
    ///
    /// A match for a channel that is missing, deleted or archived is recorded
    /// as `skipped_channel` instead. That check is not locked: like any
    /// channel message, a record can land as the channel is archived.
    #[datastore_span(name = "record_webhook_outcome", system = "postgresql")]
    pub async fn record_webhook_outcome(
        &self,
        community: CommunityId,
        delivery: &nostr::EventId,
        head: &SubscriptionHead,
        outcome: Outcome<'_>,
    ) -> Result<Recorded> {
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        tx.set_push_enqueue(self.push_enqueue.clone());
        let (outcome, record, last_error) = match outcome {
            Outcome::Matched(record) => {
                let live: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM channels WHERE community_id = $1 AND id = $2 \
                         AND deleted_at IS NULL AND archived_at IS NULL)",
                )
                .bind(community.as_uuid())
                .bind(head.channel_id)
                .fetch_one(tx.conn())
                .await?;
                if live {
                    ("matched", Some(record), None)
                } else {
                    ("skipped_channel", None, None)
                }
            }
            Outcome::Error(error) => ("error", None, Some(error)),
            Outcome::SkippedSigner => ("skipped_signer", None, None),
        };
        let inserted = sqlx::query(
            "INSERT INTO webhook_records (community_id, delivery_event_id, subscription_id, \
                 subscription_revision, outcome, record_event_id, last_error) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (community_id, delivery_event_id, subscription_id) DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(delivery.as_bytes().as_slice())
        .bind(head.subscription_id)
        .bind(head.revision.event.id.as_bytes().as_slice())
        .bind(outcome)
        .bind(record.map(|r| r.id.as_bytes().to_vec()))
        .bind(last_error)
        .execute(tx.conn())
        .await?
        .rows_affected();
        if inserted == 0 {
            return Ok(Recorded::Already);
        }
        let stored = match record {
            Some(record) => {
                let (stored, _) = crate::event::insert_event_in_transaction(
                    &mut tx,
                    record,
                    Some(head.channel_id),
                )
                .await?;
                Some(Box::new(stored))
            }
            None => None,
        };
        tx.commit().await?;
        Ok(match (outcome, stored) {
            ("skipped_channel", _) => Recorded::SkippedChannel,
            (_, stored) => Recorded::New(stored),
        })
    }

    /// Finish a claimed delivery. False when a newer claim owns it.
    #[datastore_span(name = "finish_webhook_delivery", system = "postgresql")]
    pub async fn finish_webhook_delivery(&self, delivery: &ClaimedDelivery) -> Result<bool> {
        self.settle_webhook_delivery(delivery, "done", Duration::ZERO, None)
            .await
    }

    /// Return a claimed delivery to the queue after `delay`, or fail it for
    /// good when `give_up`. False when a newer claim owns it.
    #[datastore_span(name = "retry_webhook_delivery", system = "postgresql")]
    pub async fn retry_webhook_delivery(
        &self,
        delivery: &ClaimedDelivery,
        delay: Duration,
        give_up: bool,
        error: &str,
    ) -> Result<bool> {
        let state = if give_up { "failed" } else { "pending" };
        self.settle_webhook_delivery(delivery, state, delay, Some(error))
            .await
    }

    async fn settle_webhook_delivery(
        &self,
        delivery: &ClaimedDelivery,
        state: &str,
        delay: Duration,
        error: Option<&str>,
    ) -> Result<bool> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            delivery.community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        let settled = sqlx::query(
            "UPDATE webhook_deliveries \
             SET state = $4, claim_id = NULL, next_attempt_at = now() + $5, \
                 last_error = COALESCE($6, last_error) \
             WHERE community_id = $1 AND delivery_event_id = $2 AND claim_id = $3 \
               AND state = 'evaluating'",
        )
        .bind(delivery.community.as_uuid())
        .bind(delivery.event.id.as_bytes().as_slice())
        .bind(delivery.claim_id)
        .bind(state)
        .bind(delay)
        .bind(error)
        .execute(tx.conn())
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(settled == 1)
    }

    /// Delete up to `limit` finished deliveries of `community` received
    /// before `before`, with their outcomes. Returns the deliveries deleted.
    ///
    /// Records stay: they are channel events.
    #[datastore_span(name = "reap_webhook_deliveries", system = "postgresql")]
    pub async fn reap_webhook_deliveries(
        &self,
        community: CommunityId,
        before: DateTime<Utc>,
        limit: i64,
    ) -> Result<u64> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        let reaped: i64 = sqlx::query_scalar(
            "WITH gone AS ( \
                 DELETE FROM webhook_deliveries d \
                 WHERE d.community_id = $1 AND d.delivery_event_id IN ( \
                     SELECT delivery_event_id FROM webhook_deliveries \
                     WHERE community_id = $1 AND state IN ('done', 'failed') \
                       AND received_at < $2 \
                     ORDER BY received_at LIMIT $3) \
                 RETURNING d.delivery_event_id), \
             outcomes AS ( \
                 DELETE FROM webhook_records r USING gone \
                 WHERE r.community_id = $1 AND r.delivery_event_id = gone.delivery_event_id) \
             SELECT count(*) FROM gone",
        )
        .bind(community.as_uuid())
        .bind(before)
        .bind(limit)
        .fetch_one(tx.conn())
        .await?;
        tx.commit().await?;
        u64::try_from(reaped).map_err(|_| DbError::InvalidData("negative count".into()))
    }

    /// Communities with finished deliveries received before `before`, after
    /// `after`, a page at a time.
    #[datastore_span(name = "reapable_webhook_communities", system = "postgresql")]
    pub async fn reapable_webhook_communities(
        &self,
        before: DateTime<Utc>,
        after: Option<CommunityId>,
    ) -> Result<Vec<CommunityId>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM communities c \
             WHERE ($2::uuid IS NULL OR c.id > $2) \
               AND c.deletion_state = 'active' \
               AND EXISTS ( \
                   SELECT 1 FROM webhook_deliveries d \
                   WHERE d.community_id = c.id AND d.state IN ('done', 'failed') \
                     AND d.received_at < $1) \
             ORDER BY c.id LIMIT $3",
        )
        .bind(before)
        .bind(after.map(|community| *community.as_uuid()))
        .bind(DISCOVERY_PAGE)
        .fetch_all(&mut *connection)
        .await?;
        Ok(ids.into_iter().map(CommunityId::from_uuid).collect())
    }
}
