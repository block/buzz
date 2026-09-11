//! Replaceable-event persistence and coordinate locking.

use buzz_core::{CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{Acquire, Postgres, Transaction};
use uuid::Uuid;

use crate::observability::{self, LockType, TransactionOperation};
use crate::{Db, DbError, Result};

/// Result category for a parameterized-replaceable event write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterizedReplaceStatus {
    /// The incoming event was inserted as the coordinate's live head.
    Inserted,
    /// The exact event was already accepted.
    Duplicate,
    /// A newer event, or lower-ID same-second event, already dominates it.
    Superseded,
    /// A requested current revision has no live coordinate head.
    RevisionMissing,
    /// The live coordinate head differs from the requested revision.
    RevisionMismatch,
    /// An exact replay was required, but the event is not the live head.
    ReplayOnlyMiss,
}

/// Structural precondition for a parameterized-replaceable write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterizedReplacePrecondition<'a> {
    /// Apply normal NIP-33 ordering without a revision precondition.
    Unconditional,
    /// Require the live head to match this validated event ID.
    ExpectedRevision(&'a [u8]),
    /// Accept only an exact live-head replay and perform no mutation otherwise.
    ExactReplayOnly,
}

/// Result of a transaction-bound parameterized-replaceable event write.
#[derive(Clone, Debug)]
pub struct ParameterizedReplaceResult {
    /// Stored representation of the submitted event.
    pub event: StoredEvent,
    /// Whether and why the coordinate accepted the event.
    pub status: ParameterizedReplaceStatus,
}

impl ParameterizedReplaceResult {
    fn new(
        event: &nostr::Event,
        received_at: DateTime<Utc>,
        channel_id: Option<Uuid>,
        status: ParameterizedReplaceStatus,
    ) -> Self {
        Self {
            event: StoredEvent::with_received_at(
                event.clone(),
                received_at,
                channel_id,
                status == ParameterizedReplaceStatus::Inserted,
            ),
            status,
        }
    }
}

/// Derive the transaction-scoped advisory-lock key for an event coordinate.
///
/// Hash collisions only add serialization; the SQL predicates still determine
/// which rows are read or changed.
pub(crate) fn event_replacement_lock_key(
    community_id: CommunityId,
    kind: i32,
    pubkey: &[u8],
    coordinate: Option<&[u8]>,
) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    let kind_bytes = kind.to_le_bytes();
    for bytes in [
        community_id.as_uuid().as_bytes().as_slice(),
        kind_bytes.as_slice(),
        pubkey,
    ] {
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    if let Some(coordinate) = coordinate {
        for byte in coordinate {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash as i64
}

/// Replace a parameterized event in a caller-owned transaction.
///
/// This function acquires a transaction-scoped advisory lock but never commits
/// or rolls back the outer transaction. The typed precondition can require the
/// current live head to have an exact event ID or restrict the operation to an
/// idempotent replay.
async fn replace_parameterized_event_in_transaction_impl(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &nostr::Event,
    d_tag: &str,
    channel_id: Option<Uuid>,
    precondition: ParameterizedReplacePrecondition<'_>,
) -> Result<ParameterizedReplaceResult> {
    let kind_i32 = buzz_core::kind::event_kind_i32(event);
    let pubkey_bytes = event.pubkey.to_bytes();
    let created_at_secs = event.created_at.as_secs() as i64;
    let created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
    let received_at = Utc::now();

    let lock_key = event_replacement_lock_key(
        community_id,
        kind_i32,
        pubkey_bytes.as_slice(),
        Some(d_tag.as_bytes()),
    );
    observability::observe_advisory_lock(
        LockType::Replacement,
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut **tx),
    )
    .await?;

    let d_tag_count = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|part| part == "d"))
        .count();
    let has_exact_d_tag = event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() >= 2 && parts[0] == "d" && parts[1] == d_tag
    });
    let read_state_t_tag_count = event
        .tags
        .iter()
        .filter(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == "t" && parts[1] == "read-state"
        })
        .count();
    let is_nip_rs = kind_i32 == buzz_core::kind::KIND_READ_STATE as i32
        && d_tag_count == 1
        && has_exact_d_tag
        && d_tag.strip_prefix("read-state:").is_some_and(|slot| {
            slot.len() == 32
                && slot
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        && read_state_t_tag_count == 1;
    let is_buzz_mesh_status = kind_i32 == buzz_core::kind::KIND_BOOKMARK_SET as i32
        && d_tag.starts_with("buzz-mesh-member-status:")
        && event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == "k" && parts[1] == "buzz-mesh-status"
        });
    let hard_delete_superseded = is_nip_rs || is_buzz_mesh_status;

    let existing: Option<(DateTime<Utc>, Vec<u8>)> = sqlx::query_as(
        "SELECT created_at, id FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL \
         ORDER BY created_at DESC, id ASC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(kind_i32)
    .bind(pubkey_bytes.as_slice())
    .bind(d_tag)
    .fetch_optional(&mut **tx)
    .await?;
    let watermark: Option<(DateTime<Utc>, Vec<u8>)> = if is_nip_rs {
        sqlx::query_as(
            "SELECT created_at, event_id FROM parameterized_event_watermarks \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4",
        )
        .bind(community_id.as_uuid())
        .bind(kind_i32)
        .bind(pubkey_bytes.as_slice())
        .bind(d_tag)
        .fetch_optional(&mut **tx)
        .await?
    } else {
        None
    };

    let incoming_id = event.id.as_bytes().as_slice();
    if existing
        .as_ref()
        .is_some_and(|(_, existing_id)| existing_id.as_slice() == incoming_id)
        || watermark
            .as_ref()
            .is_some_and(|(_, event_id)| event_id.as_slice() == incoming_id)
    {
        return Ok(ParameterizedReplaceResult::new(
            event,
            received_at,
            channel_id,
            ParameterizedReplaceStatus::Duplicate,
        ));
    }

    if precondition == ParameterizedReplacePrecondition::ExactReplayOnly {
        return Ok(ParameterizedReplaceResult::new(
            event,
            received_at,
            channel_id,
            ParameterizedReplaceStatus::ReplayOnlyMiss,
        ));
    }

    if let ParameterizedReplacePrecondition::ExpectedRevision(expected_revision) = precondition {
        let status = match existing.as_ref() {
            None => Some(ParameterizedReplaceStatus::RevisionMissing),
            Some((_, existing_id)) if existing_id.as_slice() != expected_revision => {
                Some(ParameterizedReplaceStatus::RevisionMismatch)
            }
            Some(_) => None,
        };
        if let Some(status) = status {
            return Ok(ParameterizedReplaceResult::new(
                event,
                received_at,
                channel_id,
                status,
            ));
        }
    }

    let dominated = existing
        .iter()
        .chain(watermark.iter())
        .any(|(accepted_ts, accepted_id)| {
            created_at < *accepted_ts
                || (created_at == *accepted_ts && incoming_id >= accepted_id.as_slice())
        });
    if dominated {
        return Ok(ParameterizedReplaceResult::new(
            event,
            received_at,
            channel_id,
            ParameterizedReplaceStatus::Superseded,
        ));
    }

    let mut savepoint = tx.begin().await?;
    if existing.is_some() {
        let previous_nip_rs_hard_delete: Option<String> = if is_nip_rs {
            sqlx::query_scalar(
                "SELECT NULLIF(current_setting('buzz.nip_rs_hard_delete', true), '')",
            )
            .fetch_one(&mut *savepoint)
            .await?
        } else {
            None
        };
        if is_nip_rs {
            sqlx::query("SELECT set_config('buzz.nip_rs_hard_delete', 'on', true)")
                .execute(&mut *savepoint)
                .await?;
        }
        let statement = if hard_delete_superseded {
            "DELETE FROM events \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL"
        } else {
            "UPDATE events SET deleted_at = NOW() \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL"
        };
        sqlx::query(statement)
            .bind(community_id.as_uuid())
            .bind(kind_i32)
            .bind(pubkey_bytes.as_slice())
            .bind(d_tag)
            .execute(&mut *savepoint)
            .await?;

        if is_nip_rs {
            let previous_value = previous_nip_rs_hard_delete.as_deref().unwrap_or_default();
            sqlx::query("SELECT set_config('buzz.nip_rs_hard_delete', $1, true)")
                .bind(previous_value)
                .execute(&mut *savepoint)
                .await?;
        }

        if hard_delete_superseded {
            if let Some((_, existing_id)) = &existing {
                sqlx::query("DELETE FROM event_mentions WHERE community_id = $1 AND event_id = $2")
                    .bind(community_id.as_uuid())
                    .bind(existing_id)
                    .execute(&mut *savepoint)
                    .await?;
            }
        }
    }

    let sig_bytes = event.sig.serialize();
    let tags_json = serde_json::to_value(&event.tags)?;
    let insert_result = sqlx::query(
        "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag, not_before) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(incoming_id)
    .bind(pubkey_bytes.as_slice())
    .bind(created_at)
    .bind(kind_i32)
    .bind(&tags_json)
    .bind(&event.content)
    .bind(sig_bytes.as_slice())
    .bind(received_at)
    .bind(channel_id)
    .bind(d_tag)
    .bind(crate::event::extract_not_before(event))
    .execute(&mut *savepoint)
    .await?;

    if insert_result.rows_affected() == 0 {
        savepoint.rollback().await?;
        return Ok(ParameterizedReplaceResult::new(
            event,
            received_at,
            channel_id,
            ParameterizedReplaceStatus::Duplicate,
        ));
    }

    if is_nip_rs {
        sqlx::query(
            "INSERT INTO parameterized_event_watermarks \
                 (community_id, kind, pubkey, d_tag, created_at, event_id) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (community_id, kind, pubkey, d_tag) DO UPDATE SET \
                 created_at = EXCLUDED.created_at, event_id = EXCLUDED.event_id",
        )
        .bind(community_id.as_uuid())
        .bind(kind_i32)
        .bind(pubkey_bytes.as_slice())
        .bind(d_tag)
        .bind(created_at)
        .bind(incoming_id)
        .execute(&mut *savepoint)
        .await?;
    }

    crate::insert_mentions_in_transaction(&mut savepoint, community_id, event, channel_id).await?;
    savepoint.commit().await?;

    Ok(ParameterizedReplaceResult::new(
        event,
        received_at,
        channel_id,
        ParameterizedReplaceStatus::Inserted,
    ))
}

impl Db {
    /// Atomically replace a replaceable event: NIP-16 kinds (0, 3, 41, 10000–19999)
    /// and NIP-29 discovery state (39000–39002, called from side_effects.rs).
    ///
    /// Keeps only the event with the highest `created_at` per (kind, pubkey, channel_id).
    /// Same-second ties are broken by lowest event `id` (NIP-16 deterministic ordering).
    /// Returns `(event, false)` for stale writes and duplicate IDs — callers should
    /// skip fan-out/dispatch when `was_inserted` is false.
    #[datastore_span(name = "replace_addressable_event", system = "postgresql")]
    pub async fn replace_addressable_event(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let kind_i32 = buzz_core::kind::event_kind_i32(event);
        let pubkey_bytes = event.pubkey.to_bytes();
        let created_at_secs = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at_secs, 0)
            .ok_or(DbError::InvalidTimestamp(created_at_secs))?;

        // Collisions only cause extra serialization; they cannot change behavior.
        let lock_key = event_replacement_lock_key(
            community_id,
            kind_i32,
            pubkey_bytes.as_slice(),
            channel_id.as_ref().map(|id| id.as_bytes().as_slice()),
        );

        let (mut tx, transaction_timer) = observability::begin_transaction(
            &self.pool,
            observability::TransactionOperation::ReplaceAddressableEvent,
        )
        .await?;

        transaction_timer
            .observe(async {
                // Serialize all writers for the same (kind, pubkey, channel_id) tuple.
                // Advisory lock is transaction-scoped — released on commit/rollback.
                observability::observe_advisory_lock(
                    observability::LockType::Replacement,
                    sqlx::query("SELECT pg_advisory_xact_lock($1)")
                        .bind(lock_key)
                        .execute(&mut *tx),
                )
                .await?;

                // Check for the newest existing event. ORDER BY + LIMIT 1 is defensive against
                // historical data where prior bugs may have left multiple live rows.
                let existing: Option<(chrono::DateTime<chrono::Utc>, Vec<u8>)> =
                    sqlx::query_as(
                        "SELECT created_at, id FROM events \
                         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
                         AND channel_id IS NOT DISTINCT FROM $4 \
                         AND deleted_at IS NULL \
                         ORDER BY created_at DESC, id ASC LIMIT 1",
                    )
                    .bind(community_id.as_uuid())
                    .bind(kind_i32)
                    .bind(pubkey_bytes.as_slice())
                    .bind(channel_id)
                    .fetch_optional(&mut *tx)
                    .await?;

                // Stale-write protection: reject if incoming is not newer.
                // NIP-16: created_at is second-resolution. On same-second tie, lowest
                // event id (lexicographic) wins — deterministic across relays.
                let incoming_id = event.id.as_bytes().as_slice();
                if let Some((existing_ts, existing_id)) = existing {
                    let dominated = created_at < existing_ts
                        || (created_at == existing_ts
                            && incoming_id >= existing_id.as_slice());
                    if dominated {
                        tx.rollback().await?;
                        let received_at = chrono::Utc::now();
                        return Ok((
                            StoredEvent::with_received_at(
                                event.clone(),
                                received_at,
                                channel_id,
                                false,
                            ),
                            false,
                        ));
                    }
                }

                // Soft-delete the old event (if any). IS NOT DISTINCT FROM for NULL safety.
                sqlx::query(
                    "UPDATE events SET deleted_at = NOW() \
                     WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
                     AND channel_id IS NOT DISTINCT FROM $4 \
                     AND deleted_at IS NULL",
                )
                .bind(community_id.as_uuid())
                .bind(kind_i32)
                .bind(pubkey_bytes.as_slice())
                .bind(channel_id)
                .execute(&mut *tx)
                .await?;

                // Insert the new event inside the same transaction.
                let sig_bytes = event.sig.serialize();
                let tags_json = serde_json::to_value(&event.tags)?;
                let received_at = chrono::Utc::now();
                let d_tag = crate::event::extract_d_tag(event);

                let insert_result = sqlx::query(
                    "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                     ON CONFLICT DO NOTHING",
                )
                .bind(community_id.as_uuid())
                .bind(event.id.as_bytes().as_slice())
                .bind(pubkey_bytes.as_slice())
                .bind(created_at)
                .bind(kind_i32)
                .bind(&tags_json)
                .bind(&event.content)
                .bind(sig_bytes.as_slice())
                .bind(received_at)
                .bind(channel_id)
                .bind(d_tag.as_deref())
                .execute(&mut *tx)
                .await?;

                let was_inserted = insert_result.rows_affected() > 0;
                if !was_inserted {
                    // ON CONFLICT fired — the event ID already exists. Rollback the
                    // soft-delete so we don't lose the previous replaceable event.
                    tx.rollback().await?;
                    return Ok((
                        StoredEvent::with_received_at(
                            event.clone(),
                            received_at,
                            channel_id,
                            false,
                        ),
                        false,
                    ));
                }

                // The replaceable event and its denormalized mention index are one
                // authoritative discovery write. An indexing error must roll back the
                // new event and restore the previously-live event.
                crate::insert_mentions_in_transaction(&mut tx, community_id, event, channel_id)
                    .await?;

                tx.commit().await?;

                Ok((
                    StoredEvent::with_received_at(event.clone(), received_at, channel_id, true),
                    true,
                ))
            })
            .await
    }

    /// Replace a NIP-33 event inside a caller-owned transaction.
    ///
    /// The caller owns commit or rollback. Requiring [`Transaction`] here and
    /// in the internal state machine makes the advisory-lock contract explicit.
    pub async fn replace_parameterized_event_in_transaction(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community_id: CommunityId,
        event: &nostr::Event,
        d_tag: &str,
        channel_id: Option<Uuid>,
        precondition: ParameterizedReplacePrecondition<'_>,
    ) -> Result<ParameterizedReplaceResult> {
        replace_parameterized_event_in_transaction_impl(
            tx,
            community_id,
            event,
            d_tag,
            channel_id,
            precondition,
        )
        .await
    }

    /// Atomically replace a NIP-33 parameterized replaceable event.
    ///
    /// Replacement keys on `(kind, pubkey, d_tag)` across channels. The
    /// highest timestamp wins; same-second ties use the lowest event ID.
    #[datastore_span(name = "replace_parameterized_event", system = "postgresql")]
    pub async fn replace_parameterized_event(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        d_tag: &str,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let (mut tx, transaction_timer) = observability::begin_transaction(
            &self.pool,
            TransactionOperation::ReplaceParameterizedEvent,
        )
        .await?;
        transaction_timer
            .observe(async {
                let result = self
                    .replace_parameterized_event_in_transaction(
                        &mut tx,
                        community_id,
                        event,
                        d_tag,
                        channel_id,
                        ParameterizedReplacePrecondition::Unconditional,
                    )
                    .await?;
                let was_inserted = result.status == ParameterizedReplaceStatus::Inserted;
                if was_inserted {
                    tx.commit().await?;
                } else {
                    tx.rollback().await?;
                }
                Ok((result.event, was_inserted))
            })
            .await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::{event, migration, replaceable};
    use sqlx::postgres::PgPoolOptions;
    use sqlx::{Acquire, PgPool};
    use std::time::Duration;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

    async fn setup_db() -> Db {
        let database_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() == Ok("migration") {
            migration::run_migrations(&pool)
                .await
                .expect("apply migration schema");
        }
        Db::from_pool(pool)
    }

    async fn make_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("communities-of-channels-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert community");
        id
    }

    async fn admin_url() -> String {
        std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into())
    }

    /// Create a fresh scratch database on the same server and optionally run migrations.
    async fn create_scratch_db_through(
        admin: &PgPool,
        prefix: &str,
        target: Option<i64>,
    ) -> (PgPool, String) {
        let name = format!("{}_{}", prefix, Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(admin)
            .await
            .expect("create scratch db");
        let base = admin_url().await;
        // Swap the database path segment of the admin URL for the scratch name.
        let scratch_url = {
            let idx = base.rfind('/').expect("db url has a path segment");
            format!("{}/{}", &base[..idx], name)
        };
        let pool = PgPool::connect(&scratch_url)
            .await
            .expect("connect scratch db");
        match target {
            Some(target) => migration::run_migrations_through(&pool, target)
                .await
                .expect("migrate scratch db through target"),
            None => migration::run_migrations(&pool)
                .await
                .expect("migrate scratch db"),
        }
        (pool, name)
    }

    /// Create a fresh scratch database on the same server and run all migrations.
    /// Returns (pool, db_name); callers should `drop_scratch_db` when done.
    async fn create_scratch_db(admin: &PgPool, prefix: &str) -> (PgPool, String) {
        create_scratch_db_through(admin, prefix, None).await
    }

    async fn drop_scratch_db(admin: &PgPool, pool: PgPool, name: &str) {
        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(admin)
        .await;
    }

    /// Insert identical community + channel rows into a database so the same
    /// (community, channel) ids resolve in both writer and replica.
    async fn seed_community_channel(
        pool: &PgPool,
        community: Uuid,
        channel: Uuid,
        author: &nostr::Keys,
    ) {
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("replica-routing-{}.example", community.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        crate::channel::create_channel_with_id(
            pool,
            CommunityId::from_uuid(community),
            channel,
            &format!("replica-routing-{channel}"),
            crate::channel::ChannelType::Stream,
            crate::channel::ChannelVisibility::Open,
            None,
            author.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create channel");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn addressable_replacement_rolls_back_when_mention_indexing_fails() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (pool, scratch_name) = create_scratch_db(&admin, "atomic_addressable").await;
        let db = Db::from_pool(pool.clone());
        let community_uuid = Uuid::new_v4();
        let channel = Uuid::new_v4();
        let keys = Keys::generate();
        let owner_keys = Keys::generate();
        seed_community_channel(&pool, community_uuid, channel, &owner_keys).await;
        let community = CommunityId::from_uuid(community_uuid);
        let member = owner_keys.public_key().to_hex();
        let tags = || {
            vec![
                Tag::parse(["d", channel.to_string().as_str()]).expect("d tag"),
                Tag::parse(["p", member.as_str(), "", "owner"]).expect("p tag"),
            ]
        };
        let base = Timestamp::now().as_secs();
        let old = EventBuilder::new(Kind::Custom(39002), "old")
            .tags(tags())
            .custom_created_at(Timestamp::from(base))
            .sign_with_keys(&keys)
            .expect("sign old");
        db.replace_addressable_event(community, &old, Some(channel))
            .await
            .expect("insert old roster");

        sqlx::query(
            "CREATE FUNCTION reject_test_mention() RETURNS trigger AS $$ \
             BEGIN RAISE EXCEPTION 'injected mention failure'; END; \
             $$ LANGUAGE plpgsql",
        )
        .execute(&pool)
        .await
        .expect("create failure function");
        sqlx::query(
            "CREATE TRIGGER reject_test_mention BEFORE INSERT ON event_mentions \
             FOR EACH ROW EXECUTE FUNCTION reject_test_mention()",
        )
        .execute(&pool)
        .await
        .expect("install failure injection");

        let new = EventBuilder::new(Kind::Custom(39002), "new")
            .tags(tags())
            .custom_created_at(Timestamp::from(base + 1))
            .sign_with_keys(&keys)
            .expect("sign new");
        let error = db
            .replace_addressable_event(community, &new, Some(channel))
            .await
            .expect_err("mention failure must fail replacement");
        assert!(error.to_string().contains("injected mention failure"));

        let live_id: Vec<u8> = sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id=$1 AND channel_id=$2 \
             AND kind=39002 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_one(&pool)
        .await
        .expect("query live roster");
        assert_eq!(live_id, old.id.as_bytes(), "old roster must remain live");
        let new_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
                .bind(community.as_uuid())
                .bind(new.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count rolled-back event");
        assert_eq!(new_rows, 0, "new roster must roll back with its index");

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_legacy_roster_cannot_replace_new_locked_snapshot() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (setup_pool, scratch_name) = create_scratch_db(&admin, "mixed_roster_writer").await;
        let base_url = admin_url().await;
        let slash = base_url.rfind('/').expect("database URL has path segment");
        let scratch_url = format!("{}/{}", &base_url[..slash], scratch_name);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(1))
            .connect(&scratch_url)
            .await
            .expect("connect one-connection scratch pool");
        setup_pool.close().await;
        let db = Db::from_pool(pool.clone());
        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        let channel = Uuid::new_v4();
        let relay_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let owner = owner_keys.public_key().to_bytes();
        seed_community_channel(&pool, community_uuid, channel, &owner_keys).await;

        // This is the old pod's unlocked capture A. It remains in process memory
        // while a role-only canonical mutation advances and the new pod publishes B.
        let base = Timestamp::now().as_secs();
        let roster = |members: &[(&[u8], &str)], timestamp| {
            let tags =
                std::iter::once(Tag::parse(["d", channel.to_string().as_str()]).expect("d tag"))
                    .chain(members.iter().map(|(member, role)| {
                        Tag::parse(["p", hex::encode(member).as_str(), "", *role]).expect("p tag")
                    }))
                    .collect::<Vec<_>>();
            EventBuilder::new(Kind::Custom(39002), "")
                .tags(tags)
                .custom_created_at(Timestamp::from(timestamp))
                .sign_with_keys(&relay_keys)
                .expect("sign roster")
        };

        let newcomer = Keys::generate().public_key().to_bytes();
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'member', $4)",
        )
        .bind(community_uuid)
        .bind(channel)
        .bind(newcomer.as_slice())
        .bind(owner.as_slice())
        .execute(&pool)
        .await
        .expect("seed member before legacy capture");
        let stale_a = roster(
            &[(owner.as_slice(), "owner"), (newcomer.as_slice(), "member")],
            base + 2,
        );

        sqlx::query(
            "UPDATE channel_members SET role = 'admin' \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community_uuid)
        .bind(channel)
        .bind(newcomer.as_slice())
        .execute(&pool)
        .await
        .expect("commit newer canonical role");

        let relay_pubkey = relay_keys.public_key().to_bytes();
        let mut snapshot = db
            .lock_member_snapshot(community, channel, &relay_pubkey)
            .await
            .expect("new writer captures locked roster B");
        let fresh_b = roster(
            &[(owner.as_slice(), "owner"), (newcomer.as_slice(), "admin")],
            base + 1,
        );
        assert!(
            snapshot
                .replace_member_event(community, channel, &fresh_b)
                .await
                .expect("new writer publishes B")
                .1
        );
        snapshot
            .release()
            .await
            .expect("commit B and release locks");

        // The legacy canonical path takes the replacement key, soft-deletes B,
        // then attempts its newer-timestamp stale A. Migration 0032 rejects the
        // INSERT; transaction rollback must restore B. A one-connection pool
        // proves the lock order does not turn this compatibility path into a
        // self-deadlock.
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            db.replace_addressable_event(community, &stale_a, Some(channel)),
        )
        .await
        .expect("legacy replacement must not deadlock")
        .expect_err("stale captured roster A must be rejected");
        assert!(
            matches!(
                error,
                DbError::Sqlx(sqlx::Error::Database(ref db_error))
                    if db_error.code().as_deref() == Some("23514")
            ),
            "expected roster fence check violation, got {error:?}"
        );

        let live_ids: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id=$1 AND channel_id=$2 \
             AND kind=39002 AND pubkey=$3 AND deleted_at IS NULL",
        )
        .bind(community_uuid)
        .bind(channel)
        .bind(relay_pubkey.as_slice())
        .fetch_all(&pool)
        .await
        .expect("load live roster heads");
        assert_eq!(live_ids, vec![fresh_b.id.as_bytes().to_vec()]);
        let stale_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
                .bind(community_uuid)
                .bind(stale_a.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count rejected stale roster");
        assert_eq!(stale_rows, 0, "stale roster insert must roll back");

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn nip_rs_replacement_hard_deletes_payload_and_watermark_rejects_replay() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let d_tag = format!("read-state:{}", "a".repeat(32));
        let tags = vec![
            Tag::parse(["d", d_tag.as_str()]).expect("d tag"),
            Tag::parse(["t", "read-state"]).expect("t tag"),
        ];
        let base = Timestamp::now().as_secs();
        let old = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "old")
            .tags(tags.clone())
            .custom_created_at(Timestamp::from(base))
            .sign_with_keys(&keys)
            .expect("sign old");
        let new = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "new")
            .tags(tags)
            .custom_created_at(Timestamp::from(base + 1))
            .sign_with_keys(&keys)
            .expect("sign new");

        assert!(
            db.replace_parameterized_event(community, &old, &d_tag, None)
                .await
                .expect("insert old")
                .1
        );
        assert!(
            db.replace_parameterized_event(community, &new, &d_tag, None)
                .await
                .expect("replace with new")
                .1
        );

        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("count NIP-RS rows");
        assert_eq!(rows, 1, "superseded payload must be physically deleted");

        sqlx::query(
            "UPDATE events SET deleted_at=NOW() WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .execute(&db.pool)
        .await
        .expect("simulate NIP-09 coordinate deletion");

        assert!(
            !db.replace_parameterized_event(community, &old, &d_tag, None)
                .await
                .expect("replay old")
                .1
        );
        let live: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("count live NIP-RS rows");
        assert_eq!(live, 0, "watermark must block stale resurrection");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_schema_nip_rs_transaction_operation_restores_hard_delete_opt_in() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let base = Timestamp::now().as_secs();
        let replace_d_tag = format!("read-state:{}", "b".repeat(32));
        let victim_d_tag = format!("read-state:{}", "c".repeat(32));
        let event = |d_tag: &str, content: &str, timestamp: u64| {
            EventBuilder::new(
                Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16),
                content,
            )
            .tags(vec![
                Tag::parse(["d", d_tag]).expect("d tag"),
                Tag::parse(["t", "read-state"]).expect("t tag"),
            ])
            .custom_created_at(Timestamp::from(timestamp))
            .sign_with_keys(&keys)
            .expect("sign read state")
        };
        let old = event(&replace_d_tag, "old", base);
        let new = event(&replace_d_tag, "new", base + 1);
        let victim = event(&victim_d_tag, "victim", base);

        assert!(
            db.replace_parameterized_event(community, &old, &replace_d_tag, None)
                .await
                .expect("insert old head")
                .1
        );
        assert!(
            db.replace_parameterized_event(community, &victim, &victim_d_tag, None)
                .await
                .expect("insert victim head")
                .1
        );

        let mut tx = db
            .begin_event_write_transaction()
            .await
            .expect("begin caller transaction");
        let result = db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community,
                &new,
                &replace_d_tag,
                None,
                replaceable::ParameterizedReplacePrecondition::Unconditional,
            )
            .await
            .expect("replace inside caller transaction");
        assert_eq!(
            result.status,
            replaceable::ParameterizedReplaceStatus::Inserted
        );

        let leaked: Option<String> = sqlx::query_scalar(
            "SELECT NULLIF(current_setting('buzz.nip_rs_hard_delete', true), '')",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("read hard-delete opt-in after replacement");
        assert_ne!(leaked.as_deref(), Some("on"));

        let unauthorized = sqlx::query(
            "DELETE FROM events WHERE community_id=$1 AND kind=30078 \
             AND pubkey=$2 AND d_tag=$3 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&victim_d_tag)
        .execute(&mut *tx)
        .await;
        assert!(
            unauthorized.is_err(),
            "replacement opt-in must not authorize later caller SQL"
        );
        tx.rollback().await.expect("roll back caller transaction");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn parameterized_replacement_in_existing_transaction_honors_revision_and_rollback() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let d_tag = format!("transactional-project-{}", Uuid::new_v4().simple());
        let base = Timestamp::now().as_secs();
        let event = |content: &str, timestamp: u64| {
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_PROJECT as u16), content)
                .tags(vec![Tag::parse(["d", d_tag.as_str()]).expect("d tag")])
                .custom_created_at(Timestamp::from(timestamp))
                .sign_with_keys(&keys)
                .expect("sign project")
        };
        let old = event("old", base);
        let new = event("new", base + 1);

        assert!(
            db.replace_parameterized_event(community, &old, &d_tag, None)
                .await
                .expect("insert old head")
                .1
        );

        let mut tx = db
            .begin_event_write_transaction()
            .await
            .expect("begin replacement tx");
        let outcome = db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community,
                &new,
                &d_tag,
                None,
                replaceable::ParameterizedReplacePrecondition::ExpectedRevision(
                    old.id.as_bytes().as_slice(),
                ),
            )
            .await
            .expect("replace inside caller transaction");
        assert_eq!(
            outcome.status,
            replaceable::ParameterizedReplaceStatus::Inserted
        );
        tx.rollback().await.expect("roll back replacement tx");

        let live_id: Vec<u8> = sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
             AND d_tag=$4 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(buzz_core::kind::KIND_PROJECT as i32)
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("load live head after rollback");
        assert_eq!(live_id, old.id.as_bytes().to_vec());

        let mut tx = db
            .begin_event_write_transaction()
            .await
            .expect("begin stale revision tx");
        let mismatch = db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community,
                &new,
                &d_tag,
                None,
                replaceable::ParameterizedReplacePrecondition::ExpectedRevision(
                    [0x42; 32].as_slice(),
                ),
            )
            .await
            .expect("evaluate stale revision");
        assert_eq!(
            mismatch.status,
            replaceable::ParameterizedReplaceStatus::RevisionMismatch
        );
        tx.rollback().await.expect("roll back stale revision tx");

        let missing_d_tag = format!("missing-project-{}", Uuid::new_v4().simple());
        let missing = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_PROJECT as u16),
            "missing",
        )
        .tags(vec![
            Tag::parse(["d", missing_d_tag.as_str()]).expect("missing d tag")
        ])
        .custom_created_at(Timestamp::from(base + 2))
        .sign_with_keys(&keys)
        .expect("sign missing project");
        let mut tx = db
            .begin_event_write_transaction()
            .await
            .expect("begin missing revision tx");
        let missing_result = db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community,
                &missing,
                &missing_d_tag,
                None,
                replaceable::ParameterizedReplacePrecondition::ExpectedRevision(
                    [0x24; 32].as_slice(),
                ),
            )
            .await
            .expect("evaluate missing revision");
        assert_eq!(
            missing_result.status,
            replaceable::ParameterizedReplaceStatus::RevisionMissing
        );
        tx.rollback().await.expect("roll back missing revision tx");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn parameterized_replacement_rolls_back_when_mention_indexing_fails() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (pool, scratch_name) = create_scratch_db(&admin, "atomic_parameterized").await;
        let db = Db::from_pool(pool.clone());
        let community = CommunityId::from_uuid(make_community(&pool).await);
        let keys = Keys::generate();
        let mentioned = Keys::generate().public_key().to_hex();
        let d_tag = format!("mention-project-{}", Uuid::new_v4().simple());
        let tags = || {
            vec![
                Tag::parse(["d", d_tag.as_str()]).expect("d tag"),
                Tag::parse(["p", mentioned.as_str()]).expect("p tag"),
            ]
        };
        let base = Timestamp::now().as_secs();
        let old = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_PROJECT as u16), "old")
            .tags(tags())
            .custom_created_at(Timestamp::from(base))
            .sign_with_keys(&keys)
            .expect("sign old project");
        let new = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_PROJECT as u16), "new")
            .tags(tags())
            .custom_created_at(Timestamp::from(base + 1))
            .sign_with_keys(&keys)
            .expect("sign new project");

        assert!(
            db.replace_parameterized_event(community, &old, &d_tag, None)
                .await
                .expect("insert old project")
                .1
        );
        sqlx::query(
            "CREATE FUNCTION reject_test_mention() RETURNS trigger AS $$ \
             BEGIN RAISE EXCEPTION 'injected mention failure'; END; \
             $$ LANGUAGE plpgsql",
        )
        .execute(&pool)
        .await
        .expect("create failure function");
        sqlx::query(
            "CREATE TRIGGER reject_test_mention BEFORE INSERT ON event_mentions \
             FOR EACH ROW EXECUTE FUNCTION reject_test_mention()",
        )
        .execute(&pool)
        .await
        .expect("install failure injection");

        let mut tx = db
            .begin_event_write_transaction()
            .await
            .expect("begin caller transaction");
        let error = db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community,
                &new,
                &d_tag,
                None,
                replaceable::ParameterizedReplacePrecondition::Unconditional,
            )
            .await
            .expect_err("mention failure must fail replacement");
        assert!(error.to_string().contains("injected mention failure"));

        let probe: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&mut *tx)
            .await
            .expect("inner failure must leave caller transaction usable");
        assert_eq!(probe, 1);

        let live_id: Vec<u8> = sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
             AND d_tag=$4 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(buzz_core::kind::KIND_PROJECT as i32)
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&mut *tx)
        .await
        .expect("load live project after failed indexing");
        assert_eq!(live_id, old.id.as_bytes().to_vec());
        let new_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
                .bind(community.as_uuid())
                .bind(new.id.as_bytes().as_slice())
                .fetch_one(&mut *tx)
                .await
                .expect("count rolled-back project");
        assert_eq!(new_rows, 0);
        tx.commit().await.expect("commit usable caller transaction");

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn parameterized_duplicate_restores_live_head_inside_caller_transaction() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let d_tag = format!("duplicate-project-{}", Uuid::new_v4().simple());
        let base = Timestamp::now().as_secs();
        let event = |content: &str, timestamp: u64| {
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_PROJECT as u16), content)
                .tags(vec![Tag::parse(["d", d_tag.as_str()]).expect("d tag")])
                .custom_created_at(Timestamp::from(timestamp))
                .sign_with_keys(&keys)
                .expect("sign project")
        };
        let old = event("old-live-head", base);
        let duplicate = event("soft-deleted-duplicate", base + 1);

        assert!(
            db.replace_parameterized_event(community, &duplicate, &d_tag, None)
                .await
                .expect("insert future duplicate")
                .1
        );
        sqlx::query("UPDATE events SET deleted_at=NOW() WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(duplicate.id.as_bytes().as_slice())
            .execute(&db.pool)
            .await
            .expect("soft-delete duplicate row");

        let mut seed_tx = db
            .begin_event_write_transaction()
            .await
            .expect("begin seed transaction");
        let (_, was_inserted) =
            event::insert_event_in_transaction(&mut seed_tx, community, &old, None)
                .await
                .expect("insert older live head");
        assert!(was_inserted);
        seed_tx.commit().await.expect("commit older live head");

        let mut tx = db
            .begin_event_write_transaction()
            .await
            .expect("begin caller transaction");
        let result = db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community,
                &duplicate,
                &d_tag,
                None,
                replaceable::ParameterizedReplacePrecondition::Unconditional,
            )
            .await
            .expect("evaluate soft-deleted duplicate");
        assert_eq!(
            result.status,
            replaceable::ParameterizedReplaceStatus::Duplicate
        );

        let live_id: Vec<u8> = sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
             AND d_tag=$4 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(buzz_core::kind::KIND_PROJECT as i32)
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&mut *tx)
        .await
        .expect("caller transaction remains usable after duplicate");
        assert_eq!(live_id, old.id.as_bytes().to_vec());
        tx.rollback().await.expect("roll back caller transaction");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_parameterized_replacement_keeps_deterministic_head() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let d_tag = format!("concurrent-project-{}", Uuid::new_v4().simple());
        let created_at = Timestamp::now().as_secs();
        let event = |content: &str, timestamp: u64| {
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_PROJECT as u16), content)
                .tags(vec![Tag::parse(["d", d_tag.as_str()]).expect("d tag")])
                .custom_created_at(Timestamp::from(timestamp))
                .sign_with_keys(&keys)
                .expect("sign project")
        };
        let first = event("first", created_at);
        let second = event("second", created_at);
        let expected = if first.id.as_bytes() < second.id.as_bytes() {
            &first
        } else {
            &second
        };

        let (first_result, second_result) = tokio::join!(
            db.replace_parameterized_event(community, &first, &d_tag, None),
            db.replace_parameterized_event(community, &second, &d_tag, None),
        );
        let first_inserted = first_result.expect("first concurrent writer").1;
        let second_inserted = second_result.expect("second concurrent writer").1;
        assert!(
            first_inserted || second_inserted,
            "at least one concurrent writer must insert",
        );

        let live_ids: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
             AND d_tag=$4 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(buzz_core::kind::KIND_PROJECT as i32)
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_all(&db.pool)
        .await
        .expect("load concurrent live head");
        assert_eq!(live_ids, vec![expected.id.as_bytes().to_vec()]);

        assert!(
            !db.replace_parameterized_event(community, expected, &d_tag, None)
                .await
                .expect("replay winning event")
                .1,
            "replaying the live event must be idempotent",
        );
        let stale = event("stale", created_at.saturating_sub(1));
        assert!(
            !db.replace_parameterized_event(community, &stale, &d_tag, None)
                .await
                .expect("submit stale event")
                .1,
            "an older event must not replace the live head",
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_schema_mesh_status_replacement_keeps_one_physical_row() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let d_tag = "buzz-mesh-member-status:owner-test";
        let tags = vec![
            Tag::parse(["d", d_tag]).expect("d tag"),
            Tag::parse(["k", "buzz-mesh-status"]).expect("k tag"),
        ];
        let base = Timestamp::now().as_secs();
        for (offset, content) in [(0, "running"), (1, "running-again"), (2, "stopped")] {
            let event = EventBuilder::new(
                Kind::Custom(buzz_core::kind::KIND_BOOKMARK_SET as u16),
                content,
            )
            .tags(tags.clone())
            .custom_created_at(Timestamp::from(base + offset))
            .sign_with_keys(&keys)
            .expect("sign mesh status");
            assert!(
                db.replace_parameterized_event(community, &event, d_tag, None)
                    .await
                    .expect("replace mesh status")
                    .1
            );
        }

        let (rows, live): (i64, i64) = sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE deleted_at IS NULL) FROM events \
             WHERE community_id=$1 AND kind=30003 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("count mesh status rows");
        assert_eq!((rows, live), (1, 1));

        sqlx::query(
            "UPDATE events SET deleted_at=NOW() \
             WHERE community_id=$1 AND kind=30003 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(d_tag)
        .execute(&db.pool)
        .await
        .expect("simulate old relay soft delete");
        let rows_after_legacy_delete: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events \
             WHERE community_id=$1 AND kind=30003 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("count rows after old relay soft delete");
        assert_eq!(
            rows_after_legacy_delete, 0,
            "migration trigger must purge soft-deleted mesh status"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn duplicate_nip_rs_discriminator_tags_keep_legacy_retention() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let base = Timestamp::now().as_secs();

        for (case, tags) in [
            (
                "duplicate-d",
                vec![
                    Tag::parse(["d", &format!("read-state:{}", "c".repeat(32))])
                        .expect("first d tag"),
                    Tag::parse(["d", &format!("read-state:{}", "d".repeat(32))])
                        .expect("second d tag"),
                    Tag::parse(["t", "read-state"]).expect("t tag"),
                ],
            ),
            (
                "duplicate-t",
                vec![
                    Tag::parse(["d", &format!("read-state:{}", "e".repeat(32))]).expect("d tag"),
                    Tag::parse(["t", "read-state"]).expect("first t tag"),
                    Tag::parse(["t", "read-state"]).expect("second t tag"),
                ],
            ),
        ] {
            let d_tag = tags
                .iter()
                .find_map(|tag| {
                    let parts = tag.as_slice();
                    (parts.first().is_some_and(|part| part == "d") && parts.len() >= 2)
                        .then(|| parts[1].clone())
                })
                .expect("first d-tag value");
            let old = EventBuilder::new(
                Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16),
                format!("{case}-old"),
            )
            .tags(tags.clone())
            .custom_created_at(Timestamp::from(base))
            .sign_with_keys(&keys)
            .expect("sign old event");
            let new = EventBuilder::new(
                Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16),
                format!("{case}-new"),
            )
            .tags(tags)
            .custom_created_at(Timestamp::from(base + 1))
            .sign_with_keys(&keys)
            .expect("sign new event");

            assert!(
                db.replace_parameterized_event(community, &old, &d_tag, None)
                    .await
                    .expect("insert old event")
                    .1
            );
            assert!(
                db.replace_parameterized_event(community, &new, &d_tag, None)
                    .await
                    .expect("replace with new event")
                    .1
            );

            let (rows, live): (i64, i64) = sqlx::query_as(
                "SELECT count(*), count(*) FILTER (WHERE deleted_at IS NULL) FROM events \
                 WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
            )
            .bind(community.as_uuid())
            .bind(keys.public_key().to_bytes())
            .bind(&d_tag)
            .fetch_one(&db.pool)
            .await
            .expect("count retained rows");
            assert_eq!((rows, live), (2, 1), "{case} must retain legacy history");

            let watermarks: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM parameterized_event_watermarks \
                 WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
            )
            .bind(community.as_uuid())
            .bind(keys.public_key().to_bytes())
            .bind(&d_tag)
            .fetch_one(&db.pool)
            .await
            .expect("count watermarks");
            assert_eq!(watermarks, 0, "{case} must not create a watermark");
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_schema_nip_rs_hard_delete_fence_fails_closed_and_scopes_opt_in_to_transaction(
    ) {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let base = Timestamp::now().as_secs();
        let conforming_d = format!("read-state:{}", "6".repeat(32));
        let conforming = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16),
            "fenced-conforming",
        )
        .tags(vec![
            Tag::parse(["d", conforming_d.as_str()]).expect("d tag"),
            Tag::parse(["t", "read-state"]).expect("t tag"),
        ])
        .custom_created_at(Timestamp::from(base))
        .sign_with_keys(&keys)
        .expect("sign conforming event");
        assert!(
            db.replace_parameterized_event(community, &conforming, &conforming_d, None)
                .await
                .expect("insert conforming event")
                .1
        );
        sqlx::query(
            "INSERT INTO event_mentions \
             (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
        )
        .bind(community.as_uuid())
        .bind("6".repeat(64))
        .bind(conforming.id.as_bytes().as_slice())
        .bind(conforming.created_at.as_secs() as f64)
        .execute(&db.pool)
        .await
        .expect("insert mention");

        // Model ce10's first destructive statement. RAISE aborts the transaction,
        // so its later mention delete and incoming insert can never commit.
        let mut old_writer = db.pool.begin().await.expect("begin old-writer tx");
        let rejected = sqlx::query(
            "DELETE FROM events WHERE community_id=$1 AND kind=30078 \
             AND pubkey=$2 AND d_tag=$3 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&conforming_d)
        .execute(&mut *old_writer)
        .await;
        assert!(rejected.is_err(), "old-writer hard delete must be rejected");
        old_writer.rollback().await.expect("rollback rejected tx");
        let preserved: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM events WHERE community_id=$1 AND id=$2), \
                    (SELECT count(*) FROM event_mentions WHERE community_id=$1 AND event_id=$2)",
        )
        .bind(community.as_uuid())
        .bind(conforming.id.as_bytes().as_slice())
        .fetch_one(&db.pool)
        .await
        .expect("count preserved payload and mention");
        assert_eq!(preserved, (1, 1));

        let nonconforming_d = format!("read-state:{}", "7".repeat(32));
        let nonconforming = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16),
            "fenced-nonconforming",
        )
        .tags(vec![
            Tag::parse(["d", nonconforming_d.as_str()]).expect("first d tag"),
            Tag::parse(["d", "other"]).expect("second d tag"),
            Tag::parse(["t", "read-state"]).expect("t tag"),
        ])
        .custom_created_at(Timestamp::from(base + 1))
        .sign_with_keys(&keys)
        .expect("sign nonconforming event");
        assert!(
            db.replace_parameterized_event(community, &nonconforming, &nonconforming_d, None,)
                .await
                .expect("insert nonconforming event")
                .1
        );
        let rejected_nonconforming = sqlx::query(
            "DELETE FROM events WHERE community_id=$1 AND id=$2 AND created_at=to_timestamp($3)",
        )
        .bind(community.as_uuid())
        .bind(nonconforming.id.as_bytes().as_slice())
        .bind(nonconforming.created_at.as_secs() as f64)
        .execute(&db.pool)
        .await;
        assert!(
            rejected_nonconforming.is_err(),
            "fence must cover a nonconforming OLD row at a regex coordinate"
        );

        let unrelated_d = format!("read-state:{}", "8".repeat(32));
        let unrelated = EventBuilder::new(Kind::Custom(30023), "unrelated")
            .tags(vec![Tag::parse(["d", unrelated_d.as_str()]).expect("d tag")])
            .custom_created_at(Timestamp::from(base + 2))
            .sign_with_keys(&keys)
            .expect("sign unrelated event");
        assert!(
            db.replace_parameterized_event(community, &unrelated, &unrelated_d, None)
                .await
                .expect("insert unrelated event")
                .1
        );
        let unrelated_delete = sqlx::query(
            "DELETE FROM events WHERE community_id=$1 AND id=$2 AND created_at=to_timestamp($3)",
        )
        .bind(community.as_uuid())
        .bind(unrelated.id.as_bytes().as_slice())
        .bind(unrelated.created_at.as_secs() as f64)
        .execute(&db.pool)
        .await
        .expect("delete unrelated event");
        assert_eq!(unrelated_delete.rows_affected(), 1);

        // Check both transaction exits on one physical session; pool selection
        // cannot accidentally hide a leaked session-local authorization value.
        let mut conn = db.pool.acquire().await.expect("acquire dedicated session");
        for commit in [true, false] {
            let mut tx = conn.begin().await.expect("begin GUC transaction");
            let value: String =
                sqlx::query_scalar("SELECT set_config('buzz.nip_rs_hard_delete', 'on', true)")
                    .fetch_one(&mut *tx)
                    .await
                    .expect("set transaction-local GUC");
            assert_eq!(value, "on");
            if commit {
                tx.commit().await.expect("commit GUC transaction");
            } else {
                tx.rollback().await.expect("rollback GUC transaction");
            }
            let leaked: Option<String> = sqlx::query_scalar(
                "SELECT NULLIF(current_setting('buzz.nip_rs_hard_delete', true), '')",
            )
            .fetch_one(&mut *conn)
            .await
            .expect("read GUC after transaction");
            assert_ne!(leaked.as_deref(), Some("on"));
        }
    }
}
