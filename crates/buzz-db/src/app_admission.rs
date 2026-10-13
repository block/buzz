//! Serialized idempotent admission for App callback deliveries.
//!
//! Modeled on [`crate::workflow_admission`]: the advisory lock is the first
//! statement in the transaction so a duplicate waiter cannot observe a vacant
//! key while the first caller is still inserting. Dropping the guard rolls the
//! transaction back and inserts nothing.
//!
//! The delivery UUID is allocated when a vacant guard is created and reused for
//! the final delivered or rejected row. Only final rows are stored.

use chrono::{DateTime, Utc};
use nostr::Event;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use buzz_core::{CommunityId, StoredEvent};

use buzz_datastore_tracing::datastore_span;

use crate::error::{DbError, Result};
use crate::event::insert_event_with_thread_metadata_tx;
use crate::Db;

const SELECT_EXISTING_DELIVERY: &str = r#"
        SELECT community_id, id, app_id, idempotency_key_hash, payload_hash, event_type,
               route_snapshot, status, event_id, failure_code, created_at, completed_at
        FROM app_callback_deliveries
        WHERE community_id = $1 AND app_id = $2 AND idempotency_key_hash = $3
        "#;

const INSERT_DELIVERY: &str = r#"
            INSERT INTO app_callback_deliveries (
                community_id, id, app_id, idempotency_key_hash, payload_hash, event_type,
                route_snapshot, status, event_id, failure_code, created_at, completed_at
            )
            VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW(), NOW()
            )
            RETURNING community_id, id, app_id, idempotency_key_hash, payload_hash, event_type,
                      route_snapshot, status, event_id, failure_code, created_at, completed_at
            "#;

/// Outcome of attempting to admit a callback for an App-scoped idempotency key.
pub enum BeginAppAdmission {
    /// A delivery for this key and payload hash already exists.
    Existing(AppDeliveryRecord),
    /// A delivery for this key exists with a different payload hash.
    PayloadConflict {
        /// Previously admitted delivery for this idempotency key.
        existing: AppDeliveryRecord,
    },
    /// No delivery exists; the caller holds the admission lock until finalize or drop.
    Vacant(AppAdmissionGuard),
}

/// Final delivered row plus the stored root event, returned after a single commit.
#[derive(Debug, Clone)]
pub struct AppDeliveryCommit {
    /// Immutable delivered ledger row.
    pub record: AppDeliveryRecord,
    /// Root event persisted in the same transaction.
    pub stored_event: StoredEvent,
}

/// In-flight admission transaction holding the per-key advisory lock.
///
/// Dropping the guard rolls the transaction back and inserts nothing.
pub struct AppAdmissionGuard {
    tx: Transaction<'static, Postgres>,
    community_id: CommunityId,
    app_id: Uuid,
    delivery_id: Uuid,
    idempotency_key_hash: [u8; 32],
    payload_hash: [u8; 32],
    event_type: String,
}

/// Server-resolved destination captured when a callback is delivered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppRouteSnapshot {
    /// Community that owns the delivery.
    pub community_id: Uuid,
    /// Canonical `30617:<owner>:<d>` repository coordinate. `None` when the
    /// callback named its destination channel directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_coordinate: Option<String>,
    /// Canonical `30621:<owner>:<d>` project coordinate. `None` when the
    /// callback named its destination channel directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_coordinate: Option<String>,
    /// Destination channel selected by the route.
    pub channel_id: Uuid,
}

/// Final App callback delivery row. `completed_at` is required because only
/// final delivered or rejected rows are stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDeliveryRecord {
    /// Host-derived community that owns this delivery.
    pub community_id: CommunityId,
    /// Delivery UUID allocated at vacant admission.
    pub id: Uuid,
    /// Authenticated App UUID.
    pub app_id: Uuid,
    /// SHA-256 of the raw provider idempotency key.
    pub idempotency_key_hash: [u8; 32],
    /// SHA-256 of canonical JSON after removing `idempotency_key`.
    pub payload_hash: [u8; 32],
    /// Sanitized callback event type.
    pub event_type: String,
    /// Route captured on delivered rows; `None` on rejected rows.
    pub route_snapshot: Option<AppRouteSnapshot>,
    /// Final outcome.
    pub status: AppDeliveryStatus,
    /// Root event ID on delivered rows.
    pub event_id: Option<[u8; 32]>,
    /// Stable redacted failure code on rejected rows.
    pub failure_code: Option<String>,
    /// Admission time (stamped when the final row is inserted).
    pub created_at: DateTime<Utc>,
    /// Delivery or rejection time.
    pub completed_at: DateTime<Utc>,
}

/// Final delivery outcome stored on `app_callback_deliveries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppDeliveryStatus {
    /// Callback produced one root event.
    Delivered,
    /// Deterministic rejection; no event was stored.
    Rejected,
}

/// Stable redacted failure recorded on a rejected delivery.
pub struct AppDeliveryFailure<'a> {
    /// Machine-readable failure code (1–64 chars).
    pub code: &'a str,
}

/// Live destination channel visible inside an admission transaction.
pub struct AppAdmissionChannel {
    /// Channel identifier.
    pub id: Uuid,
}

/// Active destination-channel member with an optional display name.
pub struct AppAdmissionNamedMember {
    /// Member pubkey bytes.
    pub pubkey: Vec<u8>,
    /// Profile display name, if set.
    pub display_name: Option<String>,
}

impl AppAdmissionGuard {
    /// Delivery UUID allocated when this vacant guard was created.
    pub fn delivery_id(&self) -> Uuid {
        self.delivery_id
    }

    /// List latest live global parameterized heads of `kind` on this transaction.
    pub async fn list_latest_parameterized_heads(&mut self, kind: i32) -> Result<Vec<StoredEvent>> {
        crate::project_heads::list_latest_parameterized_heads_on(
            &mut self.tx,
            self.community_id,
            kind,
        )
        .await
    }

    /// Load a live (not archived, not deleted) destination channel in this community.
    ///
    /// Soft-deleted and archived rows are excluded. A channel that exists only
    /// in another community is not visible.
    pub async fn load_live_destination_channel(
        &mut self,
        channel_id: Uuid,
    ) -> Result<Option<AppAdmissionChannel>> {
        let row = sqlx::query(
            "SELECT id FROM channels \
             WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL AND archived_at IS NULL",
        )
        .bind(self.community_id.as_uuid())
        .bind(channel_id)
        .fetch_optional(&mut *self.tx)
        .await?;
        row.map(|row| {
            Ok(AppAdmissionChannel {
                id: row.try_get("id")?,
            })
        })
        .transpose()
    }

    /// Live stream channels in this community whose name equals `name`.
    ///
    /// Comparison is exact and case-sensitive. Returns at most two IDs so the
    /// caller can tell a unique match from an ambiguous one. DM, forum, and
    /// workflow channels are never returned.
    pub async fn find_live_stream_channels_by_name(&mut self, name: &str) -> Result<Vec<Uuid>> {
        let rows = sqlx::query(
            "SELECT id FROM channels \
             WHERE community_id = $1 AND name = $2 AND channel_type = 'stream' \
               AND deleted_at IS NULL AND archived_at IS NULL \
             LIMIT 2",
        )
        .bind(self.community_id.as_uuid())
        .bind(name)
        .fetch_all(&mut *self.tx)
        .await?;
        rows.into_iter().map(|row| Ok(row.try_get("id")?)).collect()
    }

    /// Active destination-channel members and display names in this community.
    ///
    /// Does not consult App membership. Members of a deleted, archived, or
    /// foreign-community channel are not returned.
    pub async fn list_named_destination_members(
        &mut self,
        channel_id: Uuid,
    ) -> Result<Vec<AppAdmissionNamedMember>> {
        let rows = sqlx::query(
            "SELECT cm.pubkey, u.display_name \
             FROM channel_members cm \
             JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id \
               AND c.deleted_at IS NULL AND c.archived_at IS NULL \
             LEFT JOIN users u ON cm.community_id = u.community_id AND cm.pubkey = u.pubkey \
             WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.removed_at IS NULL \
             ORDER BY cm.joined_at ASC",
        )
        .bind(self.community_id.as_uuid())
        .bind(channel_id)
        .fetch_all(&mut *self.tx)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AppAdmissionNamedMember {
                    pubkey: row.try_get("pubkey")?,
                    display_name: row.try_get("display_name")?,
                })
            })
            .collect()
    }

    /// Persist one final immutable rejected row and commit.
    pub async fn reject(self, failure: AppDeliveryFailure<'_>) -> Result<AppDeliveryRecord> {
        self.finalize(None, None, Some(failure.code)).await
    }

    /// Persist the root event, mentions, and delivered row atomically, then commit.
    ///
    /// Returns the final record and stored event for post-commit dispatch. The
    /// delivery UUID is the one allocated when this vacant guard was created.
    pub async fn deliver(
        self,
        event: &Event,
        route: &AppRouteSnapshot,
    ) -> Result<AppDeliveryCommit> {
        let community_id = self.community_id;
        let mut tx = self.tx;
        let (stored_event, was_inserted) = insert_event_with_thread_metadata_tx(
            &mut tx,
            community_id,
            event,
            Some(route.channel_id),
            None,
        )
        .await?;
        if !was_inserted {
            return Err(DbError::InvalidData(
                "app callback event id already exists".into(),
            ));
        }
        crate::insert_mentions_in_transaction(&mut tx, community_id, event, Some(route.channel_id))
            .await?;
        let snapshot_json = serde_json::to_value(route)?;
        let row = sqlx::query(INSERT_DELIVERY)
            .bind(community_id.as_uuid())
            .bind(self.delivery_id)
            .bind(self.app_id)
            .bind(self.idempotency_key_hash.as_slice())
            .bind(self.payload_hash.as_slice())
            .bind(&self.event_type)
            .bind(snapshot_json)
            .bind("delivered")
            .bind(event.id.as_bytes().as_slice())
            .bind(Option::<&str>::None)
            .fetch_one(&mut *tx)
            .await?;
        let record = row_to_delivery(row)?;
        tx.commit().await?;
        Ok(AppDeliveryCommit {
            record,
            stored_event,
        })
    }

    async fn finalize(
        self,
        route_snapshot: Option<serde_json::Value>,
        event_id: Option<&[u8]>,
        failure_code: Option<&str>,
    ) -> Result<AppDeliveryRecord> {
        let mut tx = self.tx;
        let status = if failure_code.is_some() {
            "rejected"
        } else {
            "delivered"
        };
        let row = sqlx::query(INSERT_DELIVERY)
            .bind(self.community_id.as_uuid())
            .bind(self.delivery_id)
            .bind(self.app_id)
            .bind(self.idempotency_key_hash.as_slice())
            .bind(self.payload_hash.as_slice())
            .bind(&self.event_type)
            .bind(route_snapshot)
            .bind(status)
            .bind(event_id)
            .bind(failure_code)
            .fetch_one(&mut *tx)
            .await?;
        let record = row_to_delivery(row)?;
        tx.commit().await?;
        Ok(record)
    }
}

/// Begin serialized admission for `(community, app, idempotency_key_hash)`.
///
/// The advisory lock is the first statement in the transaction so a duplicate
/// waiter cannot observe a vacant key while the first caller is still inserting.
pub async fn begin_app_admission(
    pool: &PgPool,
    community_id: CommunityId,
    app_id: Uuid,
    idempotency_key_hash: &[u8; 32],
    payload_hash: &[u8; 32],
    event_type: &str,
) -> Result<BeginAppAdmission> {
    let mut tx = pool.begin().await?;
    let lock_key = format!(
        "buzz_app_admission:{}:{}:{}",
        community_id.as_uuid(),
        app_id,
        hex::encode(idempotency_key_hash)
    );
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *tx)
        .await?;

    let row = sqlx::query(SELECT_EXISTING_DELIVERY)
        .bind(community_id.as_uuid())
        .bind(app_id)
        .bind(idempotency_key_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await?;

    match row {
        Some(row) => {
            let existing = row_to_delivery(row)?;
            let payload_matches = existing.payload_hash == *payload_hash;
            tx.commit().await?;
            if payload_matches {
                Ok(BeginAppAdmission::Existing(existing))
            } else {
                Ok(BeginAppAdmission::PayloadConflict { existing })
            }
        }
        None => Ok(BeginAppAdmission::Vacant(AppAdmissionGuard {
            tx,
            community_id,
            app_id,
            delivery_id: Uuid::new_v4(),
            idempotency_key_hash: *idempotency_key_hash,
            payload_hash: *payload_hash,
            event_type: event_type.to_owned(),
        })),
    }
}

impl Db {
    /// Begin serialized idempotent admission for an App callback delivery.
    #[datastore_span(name = "begin_app_admission", system = "postgresql")]
    pub async fn begin_app_admission(
        &self,
        community_id: CommunityId,
        app_id: Uuid,
        idempotency_key_hash: &[u8; 32],
        payload_hash: &[u8; 32],
        event_type: &str,
    ) -> Result<BeginAppAdmission> {
        begin_app_admission(
            &self.pool,
            community_id,
            app_id,
            idempotency_key_hash,
            payload_hash,
            event_type,
        )
        .await
    }
}

fn row_to_delivery(row: sqlx::postgres::PgRow) -> Result<AppDeliveryRecord> {
    let status: String = row.try_get("status")?;
    let status = match status.as_str() {
        "delivered" => AppDeliveryStatus::Delivered,
        "rejected" => AppDeliveryStatus::Rejected,
        other => {
            return Err(DbError::InvalidData(format!(
                "unknown app delivery status: {other}"
            )))
        }
    };
    let route_snapshot =
        match row.try_get::<Option<serde_json::Value>, _>("route_snapshot")? {
            Some(value) => Some(serde_json::from_value(value).map_err(|err| {
                DbError::InvalidData(format!("malformed app route snapshot: {err}"))
            })?),
            None => None,
        };
    Ok(AppDeliveryRecord {
        community_id: CommunityId::from_uuid(row.try_get("community_id")?),
        id: row.try_get("id")?,
        app_id: row.try_get("app_id")?,
        idempotency_key_hash: hash32(row.try_get("idempotency_key_hash")?, "idempotency_key_hash")?,
        payload_hash: hash32(row.try_get("payload_hash")?, "payload_hash")?,
        event_type: row.try_get("event_type")?,
        route_snapshot,
        status,
        event_id: optional_hash32(row.try_get("event_id")?, "event_id")?,
        failure_code: row.try_get("failure_code")?,
        created_at: row.try_get("created_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

fn hash32(bytes: Vec<u8>, field: &str) -> Result<[u8; 32]> {
    <[u8; 32]>::try_from(bytes).map_err(|bytes| {
        DbError::InvalidData(format!("{field} must be 32 bytes, got {}", bytes.len()))
    })
}

fn optional_hash32(bytes: Option<Vec<u8>>, field: &str) -> Result<Option<[u8; 32]>> {
    bytes.map(|bytes| hash32(bytes, field)).transpose()
}
