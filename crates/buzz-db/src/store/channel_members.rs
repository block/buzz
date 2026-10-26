//! Channel membership and roster persistence.
//!
//! Membership mutations share one advisory-lock namespace. Relay-authored
//! roster snapshots hold that same lock through replacement publication.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::channel::{row_to_channel_record, ChannelRecord};
use crate::error::{DbError, Result};
use crate::{AdmittedTx, Db};
use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;

pub use buzz_core::channel::MemberRole;

/// A channel membership row as returned from the database.
#[derive(Debug, Clone)]
pub struct MemberRecord {
    /// The channel this membership belongs to.
    pub channel_id: Uuid,
    /// Compressed public key bytes of the member.
    pub pubkey: Vec<u8>,
    /// Role string (e.g. `"owner"`, `"member"`, `"bot"`).
    pub role: String,
    /// When the member joined.
    pub joined_at: DateTime<Utc>,
    /// Who invited this member, if applicable.
    pub invited_by: Option<Vec<u8>>,
    /// When the member was removed, if applicable.
    pub removed_at: Option<DateTime<Utc>>,
}

/// Namespace for the per-channel membership advisory lock. Serializes the
/// role-authorization + last-owner-count + write sequences in [`add_member`]
/// and [`remove_member`] against each other.
///
/// Both functions read an owner COUNT and then write a *different* row than the
/// one they counted, so `READ COMMITTED` snapshot isolation alone permits two
/// concurrent demotions (or a demotion racing a removal) to each observe two
/// owners, each pass, and together leave zero — the exact governance loss the
/// guards exist to prevent. An advisory key rather than `SELECT ... FOR UPDATE`
/// on the channel row: membership is its own contention domain and must not
/// serialize against unrelated channel metadata writers (`update_channel`,
/// `set_topic`, the TTL transition). Distinct key domain from
/// `buzz_channel_ttl:`.
const CHANNEL_MEMBERSHIP_LOCK_NAMESPACE: &str = "buzz_channel_membership:";

/// Verify that migration 0032's roster fence is active on the partitioned
/// `events` parent and every attached partition.
///
/// New roster publishers depend on this database-side guard to serialize with
/// legacy publishers during a rolling deployment. If the migration has not
/// been applied, publishing with the new lock protocol would falsely appear
/// safe while an old pod could still overwrite it with stale membership.
pub async fn verify_channel_roster_fence_catalog<'e>(
    executor: impl sqlx::PgExecutor<'e>,
) -> Result<()> {
    // tgtype bits: 1 = ROW, 2 = BEFORE, 4 = INSERT, 16 = UPDATE, 64 = INSTEAD.
    // Required: ROW + BEFORE + INSERT set; UPDATE + INSTEAD clear.
    let missing: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT n.nspname || '.' || c.relname
        FROM (
            SELECT 'public.events'::regclass AS oid
            UNION ALL
            SELECT inhrelid FROM pg_inherits WHERE inhparent = 'public.events'::regclass
        ) rels
        JOIN pg_class c ON c.oid = rels.oid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE NOT EXISTS (
            SELECT 1 FROM pg_trigger t
            WHERE t.tgrelid = rels.oid
              AND t.tgname = 'trg_events_guard_channel_roster_snapshot'
              AND t.tgfoid = to_regprocedure('public.guard_channel_roster_snapshot()')
              AND t.tgenabled IN ('O', 'A')
              AND t.tgtype & 1 = 1      -- row-level
              AND t.tgtype & 2 = 2      -- BEFORE
              AND t.tgtype & 4 = 4      -- fires on INSERT
              AND t.tgtype & 16 = 0     -- not UPDATE
              AND t.tgtype & 64 = 0     -- not INSTEAD OF
        )
        "#,
    )
    .fetch_all(executor)
    .await?;
    if !missing.is_empty() {
        return Err(DbError::InvalidData(format!(
            "channel roster fence trigger missing, disabled, or mis-shaped on: {}",
            missing.join(", ")
        )));
    }
    Ok(())
}

/// Prove migration 0032's roster fence semantics through the live writer pool.
///
/// The catalog check cannot detect a no-op or otherwise corrupted trigger
/// function. This rolled-back probe verifies that a canonical empty roster is
/// accepted while a stale roster member is rejected with `check_violation`.
pub async fn verify_channel_roster_fence_behavior(pool: &sqlx::PgPool) -> Result<()> {
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Bootstrap,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;
    let community_id = Uuid::new_v4();
    let channel_id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community_id)
        .bind(format!(
            "roster-fence-verify-{}.invalid",
            community_id.simple()
        ))
        .execute(&mut *tx)
        .await?;

    let insert = |id: Vec<u8>, tags: serde_json::Value| {
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
             VALUES ($1, $2, $3, NOW(), 39002, $4, '', $5, NOW(), $6, $7)",
        )
        .bind(community_id)
        .bind(id)
        .bind(vec![0u8; 32])
        .bind(tags)
        .bind(vec![0u8; 64])
        .bind(channel_id)
        .bind(channel_id.to_string())
    };

    insert(
        vec![0u8; 32],
        serde_json::json!([["d", channel_id.to_string()]]),
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        DbError::InvalidData(format!(
            "channel roster fence rejected a canonical probe roster: {error}"
        ))
    })?;

    sqlx::query("SAVEPOINT roster_fence_probe")
        .execute(&mut *tx)
        .await?;
    let stale = insert(
        vec![1u8; 32],
        serde_json::json!([
            ["d", channel_id.to_string()],
            ["p", hex::encode([2u8; 32]), "", "member"]
        ]),
    )
    .execute(&mut *tx)
    .await;
    match stale {
        Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("23514") => {}
        Ok(_) => {
            return Err(DbError::InvalidData(
                "channel roster fence is inert: a stale probe roster was accepted".into(),
            ));
        }
        Err(error) => {
            return Err(DbError::InvalidData(format!(
                "channel roster fence probe failed unexpectedly: {error}"
            )));
        }
    }
    sqlx::query("ROLLBACK TO SAVEPOINT roster_fence_probe")
        .execute(&mut *tx)
        .await?;
    tx.rollback().await?;
    Ok(())
}

/// Take the per-channel membership lock. MUST be the first statement in the
/// transaction that then reads roles/owner counts and writes membership, so the
/// whole check-then-write sequence is atomic against a concurrent one.
pub(crate) async fn acquire_channel_membership_lock(
    tx: &mut PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<()> {
    crate::observability::observe_advisory_lock(
        crate::observability::LockType::Membership,
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "{CHANNEL_MEMBERSHIP_LOCK_NAMESPACE}{}:{}",
                community_id.as_uuid(),
                channel_id
            ))
            .execute(&mut *tx),
    )
    .await?;
    Ok(())
}

/// Check the same p-tag shape and canonical active membership as migration 0032.
/// The caller holds this channel's membership advisory lock on the admitted
/// transaction before calling. Every writer that changes the membership of an
/// existing channel takes that key before reading roles or changing rows
/// (channel creation inserts rows that no other transaction can see yet),
/// including both kick paths
/// (`execute_kick_with_marker` and `deploy_kick_member` in
/// relay_admin_actions.rs), so no removal can commit between this check and
/// the roster commit.
/// Legacy replacement takes the replacement key, locks
/// old-head rows in its UPDATE, then takes the membership key at trigger INSERT.
/// The app path takes replacement key -> membership key -> old-head row locks,
/// removing the row-lock/membership-key cycle rather than preserving it.
///
/// On failure, DO raises the trigger's exact PostgreSQL error during mixed-version
/// rollout. Phase 2 can replace this with a typed DbError once legacy pods retire.
pub(crate) async fn validate_channel_roster_snapshot(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
    tags: &serde_json::Value,
) -> Result<()> {
    let verdict: i32 = sqlx::query_scalar(
        r#"
        SELECT CASE WHEN EXISTS (
            SELECT 1 FROM jsonb_array_elements($3::jsonb) AS roster_tag(tag_json)
            WHERE roster_tag.tag_json->>0 = 'p'
              AND (jsonb_array_length(roster_tag.tag_json) <> 4
                   OR COALESCE(roster_tag.tag_json->>1, '') !~ '^[0-9a-fA-F]{64}$'
                   OR roster_tag.tag_json->>2 <> ''
                   OR COALESCE(roster_tag.tag_json->>3, '')
                      NOT IN ('owner', 'admin', 'bot', 'member', 'guest'))
        ) THEN 1
        WHEN
            (SELECT COALESCE(array_agg(encode(cm.pubkey, 'hex') || ':' || cm.role::text
                                        ORDER BY cm.pubkey), ARRAY[]::text[])
             FROM channel_members cm
             WHERE cm.community_id = $1 AND cm.channel_id = $2
               AND cm.removed_at IS NULL)
            IS DISTINCT FROM
            (SELECT COALESCE(array_agg(lower(roster_tag.tag_json->>1) || ':' ||
                                        (roster_tag.tag_json->>3)
                                        ORDER BY decode(roster_tag.tag_json->>1, 'hex')),
                             ARRAY[]::text[])
             FROM jsonb_array_elements($3::jsonb) AS roster_tag(tag_json)
             WHERE roster_tag.tag_json->>0 = 'p')
        THEN 2 ELSE 0 END
        "#,
    )
    .bind(tx.community().as_uuid())
    .bind(channel_id)
    .bind(tags)
    .fetch_one(tx.conn())
    .await?;
    match verdict {
        0 => return Ok(()),
        1 => sqlx::query(
            "DO $$ BEGIN RAISE EXCEPTION 'kind 39002 roster contains an invalid p tag' USING ERRCODE = '23514'; END $$",
        ),
        _ => sqlx::query(
            "DO $$ BEGIN RAISE EXCEPTION 'kind 39002 roster does not match canonical channel membership' USING ERRCODE = '23514'; END $$",
        ),
    }
    .execute(tx.conn())
    .await?;
    Err(DbError::InvalidData(
        "roster check did not reject invalid snapshot".into(),
    ))
}

/// Apply the app-side roster fence to a general event writer. The trigger
/// reacquires this transaction's advisory key until the fleet has upgraded;
/// PostgreSQL advisory locks are reentrant for their current owner.
pub(crate) async fn guard_channel_roster_event(
    tx: &mut AdmittedTx,
    kind: i32,
    channel_id: Option<Uuid>,
    tags: &serde_json::Value,
) -> Result<()> {
    if kind == 39002 {
        if let Some(channel_id) = channel_id {
            let community_id = tx.community();
            acquire_channel_membership_lock(tx.conn(), community_id, channel_id).await?;
            validate_channel_roster_snapshot(tx, channel_id, tags).await?;
        }
    }
    Ok(())
}

// ── Transaction-level membership helpers (for commit_participant_join) ────────

/// Proof that a caller-owned transaction holds the membership locks of a set
/// of channels. Only [`lock_channel_memberships_in_transaction`] makes one,
/// after it takes the locks on the transaction it borrows. The borrow binds
/// the locks to that transaction: the proof cannot be used with another
/// transaction or after its commit. Membership writes that compose with other
/// work on the caller's transaction (a huddle join) are methods on it, so
/// they cannot run without the lock.
pub struct ChannelMembershipLocks<'tx> {
    tx: &'tx mut AdmittedTx,
    channels: Vec<Uuid>,
}

impl ChannelMembershipLocks<'_> {
    /// The locked transaction, for the other reads and writes of the caller's
    /// operation.
    pub fn tx(&mut self) -> &mut AdmittedTx {
        self.tx
    }

    fn check_covers(&self, channel_id: Uuid) -> Result<()> {
        if self.channels.contains(&channel_id) {
            Ok(())
        } else {
            Err(DbError::InvalidData(
                "channel membership write is not covered by its lock".into(),
            ))
        }
    }

    /// Auto-add a member (for ephemeral-channel admission).
    ///
    /// Inserts or reactivates the membership row at `Member` role with the
    /// given `invited_by` (channel creator for huddle auto-add). Refuses a
    /// channel whose membership lock this proof does not hold.
    ///
    /// Used by `commit_participant_join` to atomically add membership and the
    /// `48101` event in a single transaction under a session effect permit.
    pub async fn insert_auto_membership(
        &mut self,
        channel_id: Uuid,
        pubkey: &[u8],
        invited_by: &[u8],
    ) -> Result<()> {
        self.check_covers(channel_id)?;
        let community_id = self.tx.community();
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            VALUES ($1, $2, $3, 'member'::member_role, $4)
            ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
                removed_at = NULL,
                removed_by = NULL,
                role = EXCLUDED.role
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(pubkey)
        .bind(invited_by)
        .execute(self.tx.conn())
        .await?;
        Ok(())
    }
}

/// Take the membership lock of each channel in `channels`, in that order, on
/// a caller-owned transaction, and return the proof that holds them.
///
/// These locks come first in the channel lock order (membership, then TTL,
/// then the channel row; see `channel::lock_channel_for_settings_write`).
/// Take them before any other channel lock on `tx`.
pub async fn lock_channel_memberships_in_transaction<'tx>(
    tx: &'tx mut AdmittedTx,
    channels: &[Uuid],
) -> Result<ChannelMembershipLocks<'tx>> {
    let community_id = tx.community();
    for channel_id in channels {
        acquire_channel_membership_lock(tx.conn(), community_id, *channel_id).await?;
    }
    Ok(ChannelMembershipLocks {
        tx,
        channels: channels.to_vec(),
    })
}

/// Lock the live channel row on a caller-owned transaction and return its
/// `archived_at` (`None` when the channel is missing, deleted, or not archived).
///
/// The `FOR NO KEY UPDATE` row lock makes `archive_channel`'s
/// `UPDATE channels SET archived_at = NOW()` wait until the caller commits or
/// rolls back, so a join cannot commit into a channel archived concurrently.
/// `FOR UPDATE` would invert the lock order against `add_member`, whose
/// membership INSERT needs `KEY SHARE` on `channels` for its foreign key
/// after taking the advisory membership lock; `FOR NO KEY UPDATE` still blocks
/// archive (a non-key update) but is compatible with `KEY SHARE`. Re-reading
/// under the lock is safe: the caller already holds it.
pub async fn lock_channel_archived_at_in_transaction(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
) -> Result<Option<DateTime<Utc>>> {
    let community_id = tx.community();
    let archived_at: Option<Option<DateTime<Utc>>> = sqlx::query_scalar(
        "SELECT archived_at FROM channels \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL \
         FOR NO KEY UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(tx.conn())
    .await?;
    Ok(archived_at.flatten())
}

/// Take the shared TTL lock of `channel_id` on a caller-owned transaction.
///
/// An event in an ephemeral channel refreshes the channel's TTL at commit,
/// which takes this lock and then updates the channel row. A transaction that
/// locks that row earlier takes this lock first, so the order stays
/// membership lock, TTL lock, row against a TTL change in `update_channel`.
/// The commit-time refresh takes the same lock again, which is a no-op.
pub async fn lock_channel_ttl_shared_in_transaction(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
) -> Result<()> {
    let community_id = tx.community();
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(crate::channel::channel_ttl_lock_key(
            community_id,
            channel_id,
        ))
        .execute(tx.conn())
        .await?;
    Ok(())
}

/// The live channel's `(visibility, join_role)` on a caller-owned transaction,
/// or `None` when the channel is missing or deleted. Both settings change only
/// under the channel's membership lock, so a caller holding that lock reads
/// the value that stays in force until it commits.
pub async fn channel_write_settings_in_transaction(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
) -> Result<Option<(String, String)>> {
    let community_id = tx.community();
    let settings: Option<(String, String)> = sqlx::query_as(
        "SELECT visibility::text, join_role FROM channels \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(tx.conn())
    .await?;
    Ok(settings)
}

/// Check whether a pubkey is an active channel member on a caller-owned transaction.
///
/// Runs the same query as `is_member` but within the caller's transaction so
/// the read is serialized with any concurrent membership writes on the same lock.
pub async fn is_member_in_transaction(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<bool> {
    let community_id = tx.community();
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM channel_members cm \
         JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL \
         WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.pubkey = $3 AND cm.removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_one(tx.conn())
    .await?;
    let cnt: i64 = row.try_get("cnt")?;
    Ok(cnt > 0)
}

/// Read a pubkey's active channel role on a caller-owned transaction, or
/// `None` when it is not an active member.
pub async fn active_role_in_transaction(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<Option<String>> {
    let community_id = tx.community();
    get_active_role_tx(tx.conn(), community_id, channel_id, pubkey).await
}

// ── End transaction-level helpers ─────────────────────────────────────────────

/// The immutable facts that every relay-signed group-state event
/// (kinds 39000–39002) publishes about its channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelIdentity {
    /// Channel type string (e.g. `"stream"`, `"forum"`, `"dm"`).
    pub channel_type: String,
    /// Labels, published as `t` tags after the channel type.
    pub labels: Vec<String>,
    /// Public key bytes of the channel creator (the kind:9007 signer).
    pub created_by: Vec<u8>,
}

/// An active member roster captured while holding the channel's membership
/// serialization lock on one writer connection.
pub struct LockedMemberSnapshot {
    /// Canonical active members captured behind the lock.
    pub members: Vec<MemberRecord>,
    /// The channel's type, labels and creator, read on the same connection, so the
    /// roster carries the same identity tags as the channel's other
    /// group-state events. `None` when no channel row exists.
    pub identity: Option<ChannelIdentity>,
    channel_id: Uuid,
    relay_pubkey: Vec<u8>,
    tx: AdmittedTx,
}

impl LockedMemberSnapshot {
    /// The channel whose membership lock this guard holds.
    pub fn channel_id(&self) -> Uuid {
        self.channel_id
    }

    /// Return the newest relay-authored member snapshot timestamp for the
    /// locked coordinate, using this guard's existing connection.
    pub async fn latest_member_event_timestamp(&mut self) -> Result<Option<u64>> {
        let value: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM events WHERE community_id = $1 AND kind = 39002 AND pubkey = $2 AND channel_id = $3 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(self.tx.community().as_uuid())
        .bind(self.relay_pubkey.as_slice())
        .bind(self.channel_id)
        .fetch_optional(self.tx.conn())
        .await?;
        Ok(value.map(|timestamp| timestamp.timestamp() as u64))
    }

    /// Replace the relay-authored member snapshot on this guard's existing
    /// connection. The membership lock therefore spans capture and replacement
    /// without a nested pool checkout.
    pub async fn replace_member_event(
        &mut self,
        event: &nostr::Event,
    ) -> Result<(buzz_core::StoredEvent, bool)> {
        let community_id = self.tx.community();
        let channel_id = self.channel_id;
        if event.pubkey.to_bytes().as_slice() != self.relay_pubkey.as_slice()
            || crate::event::extract_d_tag(event) != Some(channel_id.to_string())
        {
            return Err(DbError::InvalidData(
                "member snapshot replacement does not match its locked coordinate".into(),
            ));
        }
        let kind = buzz_core::kind::event_kind_i32(event);
        if kind != 39002 {
            return Err(DbError::InvalidData(
                "member snapshot replacement requires kind 39002".into(),
            ));
        }
        let pubkey = event.pubkey.to_bytes();
        let created_at_secs = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at_secs, 0)
            .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
        let existing: Option<(chrono::DateTime<Utc>, Vec<u8>)> = sqlx::query_as(
            "SELECT created_at, id FROM events WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND channel_id = $4 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(kind)
        .bind(pubkey.as_slice())
        .bind(channel_id)
        .fetch_optional(self.tx.conn())
        .await?;
        let incoming_id = event.id.as_bytes().as_slice();
        if let Some((existing_ts, existing_id)) = existing {
            if created_at < existing_ts
                || (created_at == existing_ts && incoming_id >= existing_id.as_slice())
            {
                return Ok((
                    buzz_core::StoredEvent::with_received_at(
                        event.clone(),
                        Utc::now(),
                        Some(channel_id),
                        false,
                    ),
                    false,
                ));
            }
        }
        let tags = serde_json::to_value(&event.tags)?;
        validate_channel_roster_snapshot(&mut self.tx, channel_id, &tags).await?;
        sqlx::query("UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND channel_id = $4 AND deleted_at IS NULL")
            .bind(community_id.as_uuid()).bind(kind).bind(pubkey.as_slice()).bind(channel_id)
            .execute(self.tx.conn()).await?;
        let received_at = Utc::now();
        let sig = event.sig.serialize();
        let inserted = sqlx::query("INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT DO NOTHING")
            .bind(community_id.as_uuid()).bind(event.id.as_bytes().as_slice())
            .bind(pubkey.as_slice()).bind(created_at).bind(kind).bind(tags)
            .bind(&event.content).bind(sig.as_slice()).bind(received_at).bind(channel_id)
            .bind(crate::event::extract_d_tag(event)).execute(self.tx.conn()).await?;
        if inserted.rows_affected() == 0 {
            return Err(DbError::InvalidData(
                "member snapshot event id already exists".into(),
            ));
        }
        crate::store::event_follow_up::after_admitted_insert(
            &mut self.tx,
            event.id.as_bytes().as_slice(),
            kind,
            Some(channel_id),
        )
        .await?;
        crate::insert_mentions_in_transaction(&mut self.tx, event, Some(channel_id)).await?;
        Ok((
            buzz_core::StoredEvent::with_received_at(
                event.clone(),
                received_at,
                Some(channel_id),
                true,
            ),
            true,
        ))
    }

    /// Commit the replacement and release the membership lock.
    pub async fn release(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
}

/// Capture all active members while holding the same per-channel lock used by
/// membership writers.
///
/// The returned guard must remain alive through publication. This prevents a
/// rolling relay from publishing an older roster after a concurrent add or
/// remove has committed and published newer membership state.
pub async fn lock_member_snapshot(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    relay_pubkey: &[u8],
) -> Result<LockedMemberSnapshot> {
    let mut tx = crate::begin_community_event_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    // Match the canonical replacement writer's lock order. Old binaries take
    // this key before INSERT; migration 0032 then takes the membership key in
    // the INSERT trigger. Taking both in that order avoids mixed-version
    // duplicate heads without introducing a lock-order inversion.
    let replacement_lock = crate::replaceable::event_replacement_lock_key(
        community_id,
        39002,
        relay_pubkey,
        Some(channel_id.as_bytes()),
    );
    crate::observability::observe_advisory_lock(
        crate::observability::LockType::Replacement,
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(replacement_lock)
            .execute(tx.conn()),
    )
    .await?;
    acquire_channel_membership_lock(tx.conn(), community_id, channel_id).await?;
    let rows = sqlx::query(
        r#"
        SELECT cm.channel_id, cm.pubkey, cm.role::text AS role, cm.joined_at, cm.invited_by, cm.removed_at
        FROM channel_members cm
        JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL
        WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.removed_at IS NULL
        ORDER BY cm.joined_at ASC
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_all(tx.conn())
    .await?;
    let members = rows
        .into_iter()
        .map(row_to_member_record)
        .collect::<Result<Vec<_>>>()?;
    let identity = sqlx::query_as::<_, (String, Vec<String>, Vec<u8>)>(
        "SELECT channel_type::text, labels, created_by FROM channels WHERE community_id = $1 AND id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(tx.conn())
    .await?
    .map(|(channel_type, labels, created_by)| ChannelIdentity {
        channel_type,
        labels,
        created_by,
    });
    Ok(LockedMemberSnapshot {
        members,
        identity,
        channel_id,
        relay_pubkey: relay_pubkey.to_vec(),
        tx,
    })
}

/// Add a member to a channel.
///
/// The caller picks the role explicitly. See [`put_member`].
pub async fn add_member(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
    role: MemberRole,
    invited_by: Option<&[u8]>,
) -> Result<MemberRecord> {
    put_member(
        pool,
        community_id,
        channel_id,
        pubkey,
        Some(role),
        invited_by,
    )
    .await
}

/// Add a member to a channel, or change an active member's role (kind:9000,
/// and the kind:9021 self-join).
///
/// `invited_by` is the actor; `None` means the target is adding themselves.
/// `requested` is `None` when the event has no `role` tag: an active member
/// then keeps their role, and a new member gets the channel's `join_role`.
///
/// This is the authority for every admission rule
/// ([`buzz_core::channel::decide_member_put`]), and for the last-owner rule.
/// The relay validator applies the same rules earlier only to give the client
/// a real error. Everything the decision reads — the channel's visibility and
/// `join_role`, and the actor's and target's active roles — is read in one
/// transaction after the channel's membership lock. A change to visibility or
/// `join_role` takes the same lock ([`crate::channel::update_channel`]), so
/// neither a membership write nor a setting change can slip between the
/// decision and the write.
///
/// There is no creator exception: channel creation installs the creator as
/// owner in the same transaction, so a creator without an active row was
/// removed and is admitted like anyone else.
pub async fn put_member(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
    requested: Option<MemberRole>,
    invited_by: Option<&[u8]>,
) -> Result<MemberRecord> {
    let target = MemberPut { pubkey, requested };
    let mut results = put_members(pool, community_id, channel_id, &[target], invited_by).await?;
    results
        .pop()
        .expect("put_members returns one result per target")
        .map(|put| put.member)
}

/// One person a [`put_members`] call adds or changes.
#[derive(Debug, Clone, Copy)]
pub struct MemberPut<'a> {
    /// The person's 32-byte pubkey.
    pub pubkey: &'a [u8],
    /// The role asked for; `None` asks for no role change (see [`put_member`]).
    pub requested: Option<MemberRole>,
}

/// One [`put_members`] write, and the role the member held just before it.
#[derive(Debug, Clone)]
pub struct PutMemberResult {
    /// The membership row after the write.
    pub member: MemberRecord,
    /// The active role before the write; `None` when they were not an active
    /// member. Read under the same lock as the write.
    pub prior_role: Option<String>,
}

/// [`put_member`] for several people at once (kind:9000 naming several `p`
/// tags), in one transaction under one membership lock.
///
/// Each person is decided exactly as [`put_member`] would decide them, in
/// order, as though each had been its own write: a decision sees the roles
/// the earlier people in the call were given, and the last-owner rule counts
/// the owners they leave. The roles involved and the owner count are read once
/// and the writes go out as one upsert, so the round trips do not grow with
/// the number of people.
///
/// Returns one result per target, in order. A refused person
/// ([`DbError::AccessDenied`], or [`DbError::InvalidData`] for a malformed
/// pubkey) is not written and does not stop the others. The outer error means
/// nothing was written. A pubkey may appear only once.
pub async fn put_members(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    targets: &[MemberPut<'_>],
    invited_by: Option<&[u8]>,
) -> Result<Vec<Result<PutMemberResult>>> {
    let mut seen = std::collections::HashSet::with_capacity(targets.len());
    if let Some(dup) = targets.iter().find(|t| !seen.insert(t.pubkey)) {
        return Err(DbError::InvalidData(format!(
            "pubkey {} appears more than once",
            hex::encode(dup.pubkey)
        )));
    }

    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;

    // First statement: serialize the whole decision / owner-count / upsert
    // sequence against concurrent membership writes on this channel.
    acquire_channel_membership_lock(tx.conn(), community_id, channel_id).await?;

    let channel = get_channel_tx(tx.conn(), community_id, channel_id).await?;
    let private = channel.visibility == "private";
    let join_role = buzz_core::channel::ChannelJoinRole::parse_fail_closed(&channel.join_role);

    // Active roles of everyone the decisions read: the targets and the actor.
    let mut involved: Vec<Vec<u8>> = targets.iter().map(|t| t.pubkey.to_vec()).collect();
    if let Some(actor) = invited_by {
        involved.push(actor.to_vec());
    }
    let mut roles: std::collections::HashMap<Vec<u8>, String> = sqlx::query(
        "SELECT pubkey, role::text AS role FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = ANY($3) AND removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(&involved)
    .fetch_all(tx.conn())
    .await?
    .into_iter()
    .map(|row| Ok((row.try_get("pubkey")?, row.try_get("role")?)))
    .collect::<Result<_>>()?;

    // Owners gained (+) or lost (-) by the decisions so far, against the count
    // before this call, which is read only if a demotion needs it.
    let mut owner_delta: i64 = 0;
    let mut owners_before: Option<i64> = None;

    let mut decided: Vec<Result<(Option<String>, MemberRole)>> = Vec::with_capacity(targets.len());
    for target in targets {
        if target.pubkey.len() != 32 {
            decided.push(Err(DbError::InvalidData(format!(
                "pubkey must be 32 bytes, got {}",
                target.pubkey.len()
            ))));
            continue;
        }
        let actor = invited_by.unwrap_or(target.pubkey);
        let current_role = roles.get(target.pubkey).cloned();
        let actor_role: Option<MemberRole> = roles
            .get(actor)
            .map(|r| {
                r.parse()
                    .map_err(|_| DbError::InvalidData(format!("invalid role in database: {r}")))
            })
            .transpose()?;

        let effective_role =
            match buzz_core::channel::decide_member_put(buzz_core::channel::MemberPut {
                private,
                join_role,
                actor_role,
                target_is_actor: actor == target.pubkey,
                target_role: current_role.as_deref(),
                requested: target.requested,
            }) {
                Ok(role) => role,
                Err(denial) => {
                    decided.push(Err(DbError::AccessDenied(
                        member_put_denial_message(denial).to_string(),
                    )));
                    continue;
                }
            };

        let was_owner = current_role.as_deref() == Some("owner");
        let is_owner = effective_role == MemberRole::Owner;
        // Defense-in-depth, mirroring `remove_member`: a demotion must not
        // strip the channel of its last owner, which would leave nobody able
        // to moderate, edit metadata, or re-grant ownership.
        if was_owner && !is_owner {
            let before = match owners_before {
                Some(count) => count,
                None => {
                    let row = sqlx::query(
                        "SELECT COUNT(*) as cnt FROM channel_members \
                         WHERE community_id = $1 AND channel_id = $2 AND role = 'owner' AND removed_at IS NULL",
                    )
                    .bind(community_id.as_uuid())
                    .bind(channel_id)
                    .fetch_one(tx.conn())
                    .await?;
                    let count: i64 = row.try_get("cnt")?;
                    *owners_before.insert(count)
                }
            };
            if before + owner_delta <= 1 {
                decided.push(Err(DbError::AccessDenied(
                    "cannot demote the last owner — transfer ownership first".to_string(),
                )));
                continue;
            }
            owner_delta -= 1;
        } else if is_owner && !was_owner {
            owner_delta += 1;
        }

        roles.insert(target.pubkey.to_vec(), effective_role.as_str().to_string());
        decided.push(Ok((current_role, effective_role)));
    }

    let (write_keys, write_roles): (Vec<Vec<u8>>, Vec<&str>) = targets
        .iter()
        .zip(&decided)
        .filter_map(|(target, decision)| {
            let (_, role) = decision.as_ref().ok()?;
            Some((target.pubkey.to_vec(), role.as_str()))
        })
        .unzip();
    let mut written: std::collections::HashMap<Vec<u8>, MemberRecord> =
        std::collections::HashMap::with_capacity(write_keys.len());
    if !write_keys.is_empty() {
        let rows = sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            SELECT $1, $2, t.pubkey, t.role::member_role, $5
            FROM UNNEST($3::bytea[], $4::text[]) AS t(pubkey, role)
            ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
                removed_at = NULL,
                removed_by = NULL,
                role = EXCLUDED.role
            RETURNING channel_id, pubkey, role::text AS role, joined_at, invited_by, removed_at
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(&write_keys)
        .bind(&write_roles)
        .bind(invited_by)
        .fetch_all(tx.conn())
        .await?;
        for row in rows {
            let member = row_to_member_record(row)?;
            written.insert(member.pubkey.clone(), member);
        }
    }
    tx.commit().await?;

    Ok(targets
        .iter()
        .zip(decided)
        .map(|(target, decision)| {
            let (prior_role, _) = decision?;
            let member = written.remove(target.pubkey).ok_or_else(|| {
                DbError::InvalidData("membership upsert returned no row".to_string())
            })?;
            Ok(PutMemberResult { member, prior_role })
        })
        .collect())
}

/// The client-visible reason for a [`buzz_core::channel::MemberPutDenial`].
fn member_put_denial_message(denial: buzz_core::channel::MemberPutDenial) -> &'static str {
    use buzz_core::channel::MemberPutDenial as D;
    match denial {
        D::NotAMember => "private channel requires an invite from an active member",
        D::ElevatedRoleGrant => "only owners/admins may grant elevated roles",
        D::GuestCannotAdd => "only members can add people in this channel",
        D::SelfJoinAboveJoinRole => {
            "joining this channel gives the guest role; ask a member to make you a member"
        }
        D::RoleChange => "only owners/admins may change an active member's role",
    }
}

/// Remove a member from a channel (soft delete).
///
/// `actor_pubkey` must be an active owner/admin, the agent's owner, or the member
/// removing themselves.
///
/// Returns `Err(DbError::MemberNotFound)` if the target is not an active member.
///
/// The per-channel membership lock is the transaction's first statement, so the
/// actor's role check, the last-owner count, and the UPDATE are all serialized
/// against concurrent membership writes — otherwise a concurrent demotion of the
/// actor could commit after their role was read and this removal would proceed on
/// a stale elevated role.
///
/// The `is_agent_owner` lookup deliberately runs *before* the transaction opens:
/// it borrows a second connection from `pool`, and issuing it while holding the
/// lock could deadlock against ourselves on a small pool. That is safe because
/// `agent_owner_pubkey` is immutable — [`crate::user::set_agent_owner`] only
/// updates it when it `IS NULL` (first-mint-wins), so its value cannot change
/// under us and needs no serialization.
pub async fn remove_member(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
    actor_pubkey: &[u8],
) -> Result<()> {
    let is_self_remove = pubkey == actor_pubkey;

    // Immutable, and must not be queried while holding the lock (second pool
    // connection). Resolved up front so every *mutable* authorization read can
    // sit behind the serialization point below.
    let actor_is_agent_owner = if is_self_remove {
        false
    } else {
        crate::user::is_agent_owner(pool, community_id, pubkey, actor_pubkey).await?
    };

    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;

    // First statement: serialize the actor-role check, the last-owner count and
    // the UPDATE against concurrent membership writes on this channel (same key
    // as `add_member`).
    acquire_channel_membership_lock(tx.conn(), community_id, channel_id).await?;

    if !is_self_remove {
        let actor_role_str = get_active_role_tx(tx.conn(), community_id, channel_id, actor_pubkey)
            .await?
            .ok_or_else(|| DbError::AccessDenied("actor is not an active member".to_string()))?;
        let actor_role: MemberRole = actor_role_str.parse().map_err(|_| {
            DbError::InvalidData(format!("invalid role in database: {actor_role_str}"))
        })?;
        if !actor_role.is_elevated() && !actor_is_agent_owner {
            return Err(DbError::AccessDenied(
                "only owners/admins or the agent's owner may remove other members".to_string(),
            ));
        }
    }

    // Defense-in-depth: prevent removing the last owner regardless of caller.
    // Callers (REST handlers, NIP-29 handlers) also check this, but the DB
    // layer enforces it as the final safety net.
    let target_role = get_active_role_tx(tx.conn(), community_id, channel_id, pubkey).await?;
    if target_role.as_deref() == Some("owner") {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND role = 'owner' AND removed_at IS NULL",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_one(tx.conn())
        .await?;
        let owner_count: i64 = row.try_get("cnt")?;
        if owner_count <= 1 {
            return Err(DbError::AccessDenied(
                "cannot remove the last owner — transfer ownership first".to_string(),
            ));
        }
    }

    let result = sqlx::query(
        r#"
        UPDATE channel_members
        SET removed_at = NOW(), removed_by = $1
        WHERE community_id = $2 AND channel_id = $3 AND pubkey = $4 AND removed_at IS NULL
        "#,
    )
    .bind(actor_pubkey)
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .execute(tx.conn())
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::MemberNotFound(channel_id));
    }

    tx.commit().await?;
    Ok(())
}

/// Returns `true` if the given pubkey is an active member of the channel.
pub async fn is_member(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM channel_members cm \
         JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL \
         WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.pubkey = $3 AND cm.removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_one(&mut *connection)
    .await?;
    let cnt: i64 = row.try_get("cnt")?;
    Ok(cnt > 0)
}

/// Verify that a member is still removed under the channel-membership lock,
/// serializing with any concurrent [`add_member`] call on the same channel.
///
/// Returns `true` when `removed_at IS NOT NULL` for `(community_id, channel_id,
/// pubkey)` — i.e., the removal that crash-recovery is re-applying is still the
/// current state and no re-add has reversed it.
///
/// Uses the same `pg_advisory_xact_lock` key as [`add_member`] so that a
/// concurrent re-add either sees this check complete (and then sets `removed_at
/// = NULL` after recovery finishes) or serializes before it (in which case this
/// check observes the re-add and returns `false`, preventing stale eviction).
///
/// Designed for use before firing subscription eviction and workflow
/// disablement: cache invalidation is always safe (it only drops a stale
/// positive), but eviction and workflow-disable must not target a member who
/// was legitimately re-added after the kick committed.
///
/// **Use [`membership_removal_fence`] instead of this function** when the caller
/// needs to hold the advisory lock through the destructive effects themselves.
/// This function releases the lock immediately after the read; concurrent
/// `add_member` calls can commit between the return and the caller's effects.
pub async fn verify_member_still_removed(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<bool> {
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;
    acquire_channel_membership_lock(&mut tx, community_id, channel_id).await?;

    let row = sqlx::query(
        "SELECT removed_at FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_optional(&mut *tx)
    .await?;

    // Row absent → never joined or hard-deleted; treat as removed (safe to skip
    // eviction for someone who isn't and wasn't a member).
    let still_removed = match row {
        None => true,
        Some(r) => {
            let removed_at: Option<chrono::DateTime<Utc>> = r.try_get("removed_at")?;
            removed_at.is_some()
        }
    };
    tx.rollback().await?;
    Ok(still_removed)
}

/// A guard that holds the per-channel membership advisory lock for the
/// duration of kick live side effects.
///
/// Acquired via [`membership_removal_fence`]. The lock is released when the
/// guard is dropped (the inner transaction rolls back). Callers must NOT drop
/// the guard until after subscription eviction and workflow-disable complete,
/// so that no concurrent [`add_member`] can commit between the `still_removed`
/// observation and the destructive effects.
///
/// After firing all live side effects, call
/// [`commit_disabling_workflows`][Self::commit_disabling_workflows] to run
/// the durable workflow-disable UPDATE on this guard's own connection and
/// commit the transaction (releasing the lock). If the guard is dropped
/// without committing, the inner transaction rolls back — the advisory lock
/// is released, but no domain writes persist.
pub struct MembershipRemovalFence {
    /// `true` when the member row still has `removed_at IS NOT NULL` — i.e., no
    /// re-add has reversed the kick since it committed. When `false`, the caller
    /// must NOT fire eviction or workflow-disable.
    pub still_removed: bool,
    // Holds the advisory transaction lock. The caller commits via
    // `commit_disabling_workflows`; on drop without commit the tx rolls back.
    tx: AdmittedTx,
}

impl MembershipRemovalFence {
    /// Run the workflow-disable UPDATE on this guard's own connection, then
    /// commit the transaction (releasing the advisory lock).
    ///
    /// By running the UPDATE inside the same connection that holds the lock,
    /// no additional pool connection is needed — preventing the self-deadlock
    /// that would arise from a pool-size-N scenario where every kick holds one
    /// connection while trying to acquire a second for the disable write.
    ///
    /// The commit makes the disable durable before the lock is released, so
    /// there is no window between "workflows disabled" and "lock released."
    ///
    /// On failure the transaction is rolled back (the advisory lock is still
    /// released), and the error is returned to the caller to handle (log/skip).
    pub async fn commit_disabling_workflows(
        mut self,
        community_id: CommunityId,
        channel_id: Uuid,
        owner_pubkey: &[u8],
    ) -> crate::Result<u64> {
        let affected = crate::workflow::disable_workflows_for_owner_in_channel_on_conn(
            &mut self.tx,
            community_id,
            channel_id,
            owner_pubkey,
        )
        .await?;
        self.tx.commit().await?;
        Ok(affected)
    }
}

/// Acquire the per-channel membership advisory lock and check whether the
/// kicked member is still removed, returning a [`MembershipRemovalFence`] that
/// keeps the lock alive until the guard is dropped.
///
/// By holding the lock from the moment of the `removed_at` check until after
/// the caller's subscription eviction and workflow-disable complete, this
/// prevents the window where a concurrent [`add_member`] could commit between
/// the check and the effects (the race that [`verify_member_still_removed`]
/// leaves open, since it releases the lock before returning).
///
/// The guard is read-only before [`Self::commit_disabling_workflows`] is called.
/// Once the caller has finished its effects, dropping the guard releases the
/// lock (the transaction is implicitly rolled back if not committed). To
/// durably write the workflow-disable and release the lock in one step, call
/// [`Self::commit_disabling_workflows`].
///
/// Write admission is taken up front, before the membership lock, so the
/// lock order matches [`add_member`]. Taking it lazily in
/// [`Self::commit_disabling_workflows`] would invert that order. As a result,
/// a community that leaves `active` between the kick commit and this call
/// refuses the fence, and the caller skips its live side effects. That is
/// intended: the membership cache is already invalidated and fan-out
/// re-checks membership before delivery.
pub async fn membership_removal_fence(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<MembershipRemovalFence> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    acquire_channel_membership_lock(tx.conn(), community_id, channel_id).await?;

    let row = sqlx::query(
        "SELECT removed_at FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_optional(tx.conn())
    .await?;

    // Row absent → never joined or hard-deleted; treat as removed (safe to
    // skip eviction for someone who isn't and wasn't a member).
    let still_removed = match row {
        None => true,
        Some(r) => {
            let removed_at: Option<chrono::DateTime<Utc>> = r.try_get("removed_at")?;
            removed_at.is_some()
        }
    };

    Ok(MembershipRemovalFence { still_removed, tx })
}

/// Return which of the given (channel, pubkey) combinations are active
/// memberships, restricted to non-deleted channels — one statement for any
/// batch size (T2b). Semantics per pair match [`is_member`].
pub async fn membership_pairs(
    pool: &PgPool,
    community_id: CommunityId,
    channel_ids: &[Uuid],
    pubkeys: &[Vec<u8>],
) -> Result<Vec<(Uuid, Vec<u8>)>> {
    if channel_ids.is_empty() || pubkeys.is_empty() {
        return Ok(Vec::new());
    }
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let rows = sqlx::query(
        "SELECT cm.channel_id, cm.pubkey FROM channel_members cm \
         JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL \
         WHERE cm.community_id = $1 AND cm.channel_id = ANY($2) AND cm.pubkey = ANY($3) AND cm.removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_ids)
    .bind(pubkeys)
    .fetch_all(&mut *connection)
    .await?;
    rows.into_iter()
        .map(|row| Ok((row.try_get("channel_id")?, row.try_get("pubkey")?)))
        .collect()
}

/// Returns all active members of the given channel, ordered by `joined_at`.
///
/// The roster is returned in full and is never truncated: callers use it to
/// build the kind 39002 (NIP-29 group members) snapshot and to resolve actor
/// roles for admin-event authorization, so a partial list silently hides late
/// joiners from channel discovery and makes them read as non-members.
///
/// Returns an empty list if the channel has been soft-deleted.
pub async fn get_members(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<Vec<MemberRecord>> {
    get_members_with_operation(
        pool,
        community_id,
        channel_id,
        crate::observability::WriterOperation::Authorization,
    )
    .await
}

async fn get_members_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    operation: crate::observability::WriterOperation,
) -> Result<Vec<MemberRecord>> {
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;
    let rows = sqlx::query(
        r#"
        SELECT cm.channel_id, cm.pubkey, cm.role::text AS role, cm.joined_at, cm.invited_by, cm.removed_at
        FROM channel_members cm
        JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL
        WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.removed_at IS NULL
        ORDER BY cm.joined_at ASC
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_all(&mut *connection)
    .await?;
    rows.into_iter().map(row_to_member_record).collect()
}

/// Returns active members for multiple channels in a single query.
///
/// Designed for small-batch use (e.g. DM participant resolution where each
/// channel has 2-9 members). For large channel sets, consider pagination.
/// Returns a flat `Vec<MemberRecord>` ordered by `joined_at`; callers should
/// group by `channel_id` if per-channel access is needed.
/// Returns an empty vec immediately when `channel_ids` is empty.
pub async fn get_members_bulk(
    pool: &PgPool,
    community_id: CommunityId,
    channel_ids: &[Uuid],
) -> Result<Vec<MemberRecord>> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let rows = sqlx::query(
        r#"
        SELECT cm.channel_id, cm.pubkey, cm.role::text AS role, cm.joined_at, cm.invited_by, cm.removed_at
        FROM channel_members cm
        JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL
        WHERE cm.community_id = $1 AND cm.channel_id = ANY($2) AND cm.removed_at IS NULL
        ORDER BY cm.joined_at ASC
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_ids)
    .fetch_all(&mut *connection)
    .await?;
    rows.into_iter().map(row_to_member_record).collect()
}

/// Get all channel IDs accessible to a pubkey.
///
/// Includes channels where the pubkey is an active member AND all listed open
/// channels. Open channels must be included in REQ filter resolution.
///
/// Only legacy channel types are in either half
/// ([`buzz_core::channel::LEGACY_CHANNEL_TYPES`]): this is the implicit scope
/// of queries that do not name a channel, so system channels and any type
/// added later stay out. Unlisted open channels stay out of the open half for
/// the same reason; a member still has them through the member half. A query
/// that names such a channel, or opts in to system channels, adds them from
/// [`get_readable_named_only_channels`].
pub async fn get_accessible_channel_ids(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Vec<Uuid>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let sql = format!(
        r#"
        SELECT cm.channel_id
        FROM channel_members cm
        JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL
        WHERE cm.community_id = $1 AND cm.pubkey = $2 AND cm.removed_at IS NULL
          AND {member_types}
        UNION
        SELECT id AS channel_id
        FROM channels
        WHERE community_id = $1 AND visibility = 'open' AND NOT unlisted AND deleted_at IS NULL
          AND {open_types}
        "#,
        member_types = crate::store::channel::legacy_channel_type_sql("c.channel_type"),
        open_types = crate::store::channel::legacy_channel_type_sql("channel_type"),
    );
    // Built only from the constant type list.
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(community_id.as_uuid())
        .bind(pubkey)
        .fetch_all(&mut *connection)
        .await?;

    rows.into_iter()
        .map(|r| {
            let id: Uuid = r.try_get("channel_id")?;
            Ok(id)
        })
        .collect()
}

/// A channel that [`get_readable_named_only_channels`] found readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadableNamedOnlyChannel {
    /// The channel id.
    pub id: Uuid,
    /// Its channel type, as text.
    pub channel_type: String,
    /// Whether the system-channel opt-in (`#t:["system"]` or `#P`) may add
    /// it. False for an unlisted channel the reader is not a member of: only
    /// a filter that names it reads it.
    pub opt_in: bool,
}

/// The channels that a pubkey can read but that [`get_accessible_channel_ids`]
/// leaves out:
/// - channels of non-legacy types (system channels, and any type added later)
///   where it is an active member, and every open one that is listed or among
///   `named`, the channels the request names;
/// - each unlisted open channel of a legacy type among `named`. A member of
///   one also has it through [`get_accessible_channel_ids`]; listing it here
///   as well is harmless, since a filter's scope is deduplicated.
///
/// Uncached, so a caller can trust it for a channel the request names. A
/// read reaches one only by naming it, or, for system channels, by opting in
/// to them. The opt-in needs every listed system channel, so those are all
/// loaded; an unlisted channel is loaded only when named, since a community
/// can have many, and it is marked so the opt-in never adds it for a
/// non-member ([`ReadableNamedOnlyChannel::opt_in`]).
pub async fn get_readable_named_only_channels(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
    named: &[Uuid],
) -> Result<Vec<ReadableNamedOnlyChannel>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let sql = format!(
        r#"
        SELECT c.id AS channel_id, c.channel_type::text AS channel_type,
               (NOT c.unlisted OR {is_member}) AS opt_in
        FROM channels c
        WHERE c.community_id = $1 AND c.deleted_at IS NULL
          AND (
            (
              NOT {legacy_types}
              AND (
                (c.visibility = 'open' AND (NOT c.unlisted OR c.id = ANY($3)))
                OR {is_member}
              )
            )
            OR ({legacy_types} AND c.visibility = 'open' AND c.unlisted AND c.id = ANY($3))
          )
        "#,
        legacy_types = crate::store::channel::legacy_channel_type_sql("c.channel_type"),
        is_member = "EXISTS (
                  SELECT 1 FROM channel_members cm
                  WHERE cm.community_id = c.community_id AND cm.channel_id = c.id
                    AND cm.pubkey = $2 AND cm.removed_at IS NULL
                )",
    );
    // Built only from constants.
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(community_id.as_uuid())
        .bind(pubkey)
        .bind(named)
        .fetch_all(&mut *connection)
        .await?;

    rows.into_iter()
        .map(|r| {
            Ok(ReadableNamedOnlyChannel {
                id: r.try_get("channel_id")?,
                channel_type: r.try_get("channel_type")?,
                opt_in: r.try_get("opt_in")?,
            })
        })
        .collect()
}

/// The type of channel `channel_id`, deleted or not, or `None` when no such
/// channel exists. Membership notices carry it as their `t` tag (NIP-CL).
pub async fn channel_type_of(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<Option<String>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    Ok(sqlx::query_scalar(
        "SELECT channel_type::text FROM channels WHERE community_id = $1 AND id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut *connection)
    .await?)
}

/// The channels in `channel_ids` whose type is not a legacy type
/// ([`buzz_core::channel::LEGACY_CHANNEL_TYPES`]), deleted or not, each with
/// its type: system channels and any type added later. A filter reads them
/// only by naming them.
pub async fn named_only_channels_among(
    pool: &PgPool,
    community_id: CommunityId,
    channel_ids: &[Uuid],
) -> Result<Vec<(Uuid, String)>> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let sql = format!(
        "SELECT id, channel_type::text FROM channels \
         WHERE community_id = $1 AND id = ANY($2) AND NOT {}",
        crate::store::channel::legacy_channel_type_sql("channel_type")
    );
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(community_id.as_uuid())
        .bind(channel_ids)
        .fetch_all(&mut *connection)
        .await?)
}

/// A large channel whose canonical active-member count may need its legacy
/// discovery snapshot repaired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LargeChannelRoster {
    /// Community that owns the channel.
    pub community_id: CommunityId,
    /// Canonical host for the owning community.
    pub host: String,
    /// Channel whose roster snapshot differs from canonical membership.
    pub channel_id: Uuid,
    /// Canonical active-member count.
    pub member_count: i64,
}

/// Returns active channels whose canonical roster exceeds `minimum_members`.
///
/// Only active communities are scanned: archived communities must not have
/// their discovery state rewritten, and deleting or tombstoned communities
/// would only produce fenced-write failures.
///
/// This is an internal cross-community maintenance read. Callers must preserve
/// the returned community id when reading or rewriting discovery state.
pub async fn list_large_channel_rosters_needing_reconciliation(
    pool: &PgPool,
    minimum_members: i64,
    relay_pubkey: &[u8],
) -> Result<Vec<LargeChannelRoster>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Maintenance,
    )
    .await?;
    let rows = sqlx::query(
        r#"
        WITH large_rosters AS (
            SELECT cm.community_id, cm.channel_id, COUNT(*) AS member_count
            FROM channel_members cm
            JOIN channels ch
              ON ch.community_id = cm.community_id
             AND ch.id = cm.channel_id
             AND ch.deleted_at IS NULL
            WHERE cm.removed_at IS NULL
            GROUP BY cm.community_id, cm.channel_id
            HAVING COUNT(*) > $1
        )
        SELECT lr.community_id, community.host, lr.channel_id, lr.member_count
        FROM large_rosters lr
        JOIN communities community
          ON community.id = lr.community_id
         AND community.archived_at IS NULL
         AND community.deleted_at IS NULL
         AND community.deletion_state = 'active'
        JOIN LATERAL (
            SELECT roster.tags
            FROM events roster
            WHERE roster.community_id = lr.community_id
              AND roster.channel_id = lr.channel_id
              AND roster.kind = 39002
              AND roster.pubkey = $2
              AND roster.deleted_at IS NULL
            ORDER BY roster.created_at DESC, roster.id ASC
            LIMIT 1
        ) live_roster ON true
        WHERE lr.member_count <> (
            SELECT COUNT(*)
            FROM jsonb_array_elements(live_roster.tags) tag
            WHERE tag->>0 = 'p'
        )
        ORDER BY lr.community_id, lr.channel_id
        "#,
    )
    .bind(minimum_members)
    .bind(relay_pubkey)
    .fetch_all(&mut *connection)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(LargeChannelRoster {
                community_id: CommunityId::from_uuid(row.try_get("community_id")?),
                host: row.try_get("host")?,
                channel_id: row.try_get("channel_id")?,
                member_count: row.try_get("member_count")?,
            })
        })
        .collect()
}

/// Transaction-aware variant of [`get_active_role_tx`].
async fn get_active_role_tx(
    tx: &mut PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<Option<String>> {
    let row = sqlx::query(
        "SELECT role::text AS role FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 AND removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_optional(&mut *tx)
    .await?;
    Ok(row.map(|r| r.try_get("role")).transpose()?)
}

/// Transaction-aware variant of [`get_channel`].
async fn get_channel_tx(
    tx: &mut PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<ChannelRecord> {
    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at,
               ttl_seconds, ttl_deadline, labels, join_role, unlisted
        FROM channels WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::ChannelNotFound(channel_id))?;
    row_to_channel_record(row)
}

/// A channel entry returned as part of a bot member record.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BotChannelEntry {
    /// Channel display name.
    pub name: String,
    /// Channel UUID (as string from the DB).
    pub id: String,
}

/// Bot member record — a user with role=bot, with their channel memberships aggregated.
#[derive(Debug, Clone)]
pub struct BotMemberRecord {
    /// Compressed public key bytes of the bot user.
    pub pubkey: Vec<u8>,
    /// Optional display name for the bot.
    pub display_name: Option<String>,
    /// Optional agent type identifier.
    pub agent_type: Option<String>,
    /// Optional JSON capabilities descriptor.
    pub capabilities: Option<serde_json::Value>,
    /// Channel entries with both name and UUID, from json_agg.
    pub channels: Vec<BotChannelEntry>,
}

/// User record for bulk lookup.
#[derive(Debug, Clone)]
pub struct UserRecord {
    /// Compressed public key bytes of the user.
    pub pubkey: Vec<u8>,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Optional avatar image URL.
    pub avatar_url: Option<String>,
    /// Optional NIP-05 identifier (e.g. `user@example.com`).
    pub nip05_handle: Option<String>,
}

/// A channel record paired with whether the querying user is an active member.
#[derive(Debug, Clone)]
pub struct AccessibleChannel {
    /// The channel record.
    pub channel: ChannelRecord,
    /// Whether the querying user is an active member of this channel.
    pub is_member: bool,
}

/// Returns full channel records for all channels a user can access:
/// listed open channels (visible to everyone) plus channels where the user is
/// an active member. An unlisted open channel is returned only to its members.
///
/// Uses a LEFT JOIN on channel_members (PK: channel_id + pubkey) which produces at
/// most one row per channel. Results are ordered stream -> forum -> dm, then by name.
///
/// If `visibility_filter` is `Some("open")` or `Some("private")`, only channels with
/// that visibility value are returned. `None` returns all accessible channels.
pub async fn get_accessible_channels(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
    visibility_filter: Option<&str>,
    member_only: Option<bool>,
) -> Result<Vec<AccessibleChannel>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    // When `member_only` is `Some(true)`, restrict to channels where the user
    // has an active membership (cm.channel_id IS NOT NULL). This is a strict
    // subset of the default result set and is pushed into SQL so the LIMIT 1000
    // applies to the filtered set, not the pre-filter set.
    let membership_clause = if member_only == Some(true) {
        "AND cm.channel_id IS NOT NULL"
    } else {
        // Unlisted open channels are listed to their members only.
        "AND ((c.visibility = 'open' AND NOT c.unlisted) OR cm.channel_id IS NOT NULL)"
    };

    let base = format!(
        r#"
        SELECT c.id, c.name, c.channel_type::text AS channel_type,
               c.visibility::text AS visibility, c.description, c.canvas,
               c.created_by, c.created_at, c.updated_at, c.archived_at, c.deleted_at,
               c.nip29_group_id, c.topic_required, c.max_members,
               c.topic, c.topic_set_by, c.topic_set_at,
               c.purpose, c.purpose_set_by, c.purpose_set_at,
               c.ttl_seconds, c.ttl_deadline, c.labels, c.join_role, c.unlisted,
               (cm.channel_id IS NOT NULL) AS is_member
        FROM channels c
        LEFT JOIN channel_members cm
            ON c.community_id = cm.community_id AND c.id = cm.channel_id AND cm.pubkey = $2 AND cm.removed_at IS NULL
        WHERE c.community_id = $1 AND c.deleted_at IS NULL
          {membership_clause}
          AND (c.channel_type != 'dm' OR cm.hidden_at IS NULL)
          AND {legacy_types}
    "#,
        legacy_types = crate::store::channel::legacy_channel_type_sql("c.channel_type"),
    );

    let sql = if visibility_filter.is_some() {
        format!("{base}  AND c.visibility::text = $3\n        ORDER BY array_position(ARRAY['stream','forum','dm']::text[], c.channel_type::text), c.name\n        LIMIT 1000")
    } else {
        format!("{base}        ORDER BY array_position(ARRAY['stream','forum','dm']::text[], c.channel_type::text), c.name\n        LIMIT 1000")
    };

    let query = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(community_id.as_uuid())
        .bind(pubkey);
    let query = if let Some(vis) = visibility_filter {
        query.bind(vis)
    } else {
        query
    };

    let rows = query.fetch_all(&mut *connection).await?;
    rows.into_iter()
        .map(|row| {
            let is_member: bool = row.try_get("is_member").unwrap_or(false);
            let channel = row_to_channel_record(row)?;
            Ok(AccessibleChannel { channel, is_member })
        })
        .collect()
}

/// Returns all bot-role members with their channel memberships in one community.
///
/// Channels are returned as a JSON array of `{name, id}` objects via `json_agg`,
/// preserving the 1:1 name↔UUID pairing. No separate string_agg ordering issues.
/// Members with no active channel memberships are excluded (INNER JOIN on channels).
pub async fn get_bot_members(
    pool: &PgPool,
    community_id: CommunityId,
) -> Result<Vec<BotMemberRecord>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let rows = sqlx::query(
        r#"
        SELECT cm.pubkey, u.display_name, u.agent_type, u.capabilities,
               COALESCE(json_agg(DISTINCT jsonb_build_object('name', c.name, 'id', c.id::text)), '[]') AS channels_json
        FROM channel_members cm
        LEFT JOIN users u ON cm.community_id = u.community_id AND cm.pubkey = u.pubkey
        JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL
        WHERE cm.community_id = $1 AND cm.role = 'bot' AND cm.removed_at IS NULL
        GROUP BY cm.pubkey, u.display_name, u.agent_type, u.capabilities
        LIMIT 1000
        "#,
    )
    .bind(community_id.as_uuid())
    .fetch_all(&mut *connection)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let capabilities: Option<serde_json::Value> = row.try_get("capabilities")?;
        let channels_json: serde_json::Value = row
            .try_get::<serde_json::Value, _>("channels_json")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let channels: Vec<BotChannelEntry> =
            serde_json::from_value(channels_json).unwrap_or_default();
        out.push(BotMemberRecord {
            pubkey: row.try_get("pubkey")?,
            display_name: row.try_get("display_name")?,
            agent_type: row.try_get("agent_type")?,
            capabilities,
            channels,
        });
    }
    Ok(out)
}

/// Bulk-fetch user records by pubkey inside one community.
///
/// Returns only users that exist in the `users` table. Ordering matches input order
/// is NOT guaranteed — callers should index by pubkey if order matters.
/// Returns an empty vec immediately when `pubkeys` is empty (no query issued).
pub async fn get_users_bulk(
    pool: &PgPool,
    community_id: CommunityId,
    pubkeys: &[Vec<u8>],
) -> Result<Vec<UserRecord>> {
    get_users_bulk_with_operation(
        pool,
        community_id,
        pubkeys,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await
}

async fn get_users_bulk_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    pubkeys: &[Vec<u8>],
    operation: crate::observability::WriterOperation,
) -> Result<Vec<UserRecord>> {
    if pubkeys.is_empty() {
        return Ok(Vec::new());
    }
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;

    // Build a parameterised IN clause: ($2, $3, ...); $1 is community_id.
    let placeholders = (2..(pubkeys.len() + 2))
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT pubkey, display_name, avatar_url, nip05_handle \
         FROM users WHERE community_id = $1 AND pubkey IN ({placeholders})"
    );

    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(community_id.as_uuid());
    for pk in pubkeys {
        q = q.bind(pk);
    }

    let rows = q.fetch_all(&mut *connection).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(UserRecord {
            pubkey: row.try_get("pubkey")?,
            display_name: row.try_get("display_name")?,
            avatar_url: row.try_get("avatar_url")?,
            nip05_handle: row.try_get("nip05_handle")?,
        });
    }
    Ok(out)
}

fn row_to_member_record(row: sqlx::postgres::PgRow) -> Result<MemberRecord> {
    let channel_id: Uuid = row.try_get("channel_id")?;

    Ok(MemberRecord {
        channel_id,
        pubkey: row.try_get("pubkey")?,
        role: row.try_get("role")?,
        joined_at: row.try_get("joined_at")?,
        invited_by: row.try_get("invited_by")?,
        removed_at: row.try_get("removed_at")?,
    })
}

/// Returns the count of active (non-removed) members in a channel.
pub async fn get_member_count(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<i64> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM channel_members WHERE community_id = $1 AND channel_id = $2 AND removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_one(&mut *connection)
    .await?;
    Ok(row.try_get("cnt")?)
}

/// Bulk-fetch member counts for a set of channel IDs.
///
/// Returns a map of `channel_id -> count`. Channels with zero members are omitted.
/// Single query regardless of input size.
pub async fn get_member_counts_bulk(
    pool: &PgPool,
    community_id: CommunityId,
    channel_ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, i64>> {
    if channel_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;

    let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
        "SELECT channel_id, COUNT(*) as cnt FROM channel_members \
         WHERE community_id = ",
    );
    qb.push_bind(community_id.as_uuid());
    qb.push(" AND removed_at IS NULL AND channel_id IN (");
    let mut sep = qb.separated(", ");
    for id in channel_ids {
        sep.push_bind(*id);
    }
    qb.push(") GROUP BY channel_id");

    let rows = qb.build().fetch_all(&mut *connection).await?;

    let mut map = std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        let id: Uuid = row.try_get("channel_id")?;
        let cnt: i64 = row.try_get("cnt")?;
        map.insert(id, cnt);
    }
    Ok(map)
}

/// Get the active role of a pubkey in a channel.
///
/// Returns `None` if the pubkey is not an active member.
pub async fn get_member_role(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<Option<String>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let row = sqlx::query(
        "SELECT cm.role::text AS role FROM channel_members cm \
         JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL \
         WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.pubkey = $3 AND cm.removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.map(|r| r.try_get("role")).transpose()?)
}

impl Db {
    /// Verify the mixed-version channel-roster database fence end to end.
    #[datastore_span(name = "verify_channel_roster_fence", system = "postgresql")]
    pub async fn verify_channel_roster_fence(&self) -> Result<()> {
        {
            let mut connection = crate::observability::acquire_writer(
                &self.pool,
                crate::observability::WriterOperation::Bootstrap,
            )
            .await?;
            verify_channel_roster_fence_catalog(&mut *connection).await?;
        }
        verify_channel_roster_fence_behavior(&self.pool).await
    }

    /// Capture the active roster while holding the membership-writer lock.
    #[datastore_span(name = "lock_member_snapshot", system = "postgresql")]
    pub async fn lock_member_snapshot(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        relay_pubkey: &[u8],
    ) -> Result<LockedMemberSnapshot> {
        lock_member_snapshot(&self.pool, community_id, channel_id, relay_pubkey).await
    }

    /// Adds a member to a channel.
    #[datastore_span(name = "add_member", system = "postgresql")]
    pub async fn add_member(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
        role: MemberRole,
        invited_by: Option<&[u8]>,
    ) -> Result<MemberRecord> {
        add_member(
            &self.pool,
            community_id,
            channel_id,
            pubkey,
            role,
            invited_by,
        )
        .await
    }

    /// Adds a member or changes a role, deciding the role from the request
    /// and current channel state (see [`put_member`]).
    #[datastore_span(name = "put_member", system = "postgresql")]
    pub async fn put_member(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
        requested: Option<MemberRole>,
        invited_by: Option<&[u8]>,
    ) -> Result<MemberRecord> {
        put_member(
            &self.pool,
            community_id,
            channel_id,
            pubkey,
            requested,
            invited_by,
        )
        .await
    }

    /// [`Db::put_member`] for several people in one transaction (see
    /// [`put_members`]).
    #[datastore_span(name = "put_members", system = "postgresql")]
    pub async fn put_members(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        targets: &[MemberPut<'_>],
        invited_by: Option<&[u8]>,
    ) -> Result<Vec<Result<PutMemberResult>>> {
        put_members(&self.pool, community_id, channel_id, targets, invited_by).await
    }

    /// Removes a member from a channel.
    #[datastore_span(name = "remove_member", system = "postgresql")]
    pub async fn remove_member(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<()> {
        remove_member(&self.pool, community_id, channel_id, pubkey, actor_pubkey).await
    }

    /// Returns `true` if the pubkey is an active member.
    #[datastore_span(name = "is_member", system = "postgresql")]
    pub async fn is_member(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<bool> {
        is_member(&self.pool, community_id, channel_id, pubkey).await
    }

    /// Returns `true` when the member row still has `removed_at IS NOT NULL`,
    /// holding the channel-membership advisory lock so the check serializes
    /// with concurrent [`add_member`] calls.
    ///
    /// **Note:** this function releases the lock before returning. Use
    /// [`Db::membership_removal_fence`] when the advisory lock must remain
    /// held through subscription eviction and workflow-disable.
    #[datastore_span(name = "verify_member_still_removed", system = "postgresql")]
    pub async fn verify_member_still_removed(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<bool> {
        verify_member_still_removed(&self.pool, community_id, channel_id, pubkey).await
    }

    /// Acquire the per-channel membership advisory lock and return a
    /// [`MembershipRemovalFence`] that keeps the lock alive until it is
    /// dropped or committed via [`MembershipRemovalFence::commit_disabling_workflows`].
    ///
    /// The guard's `still_removed` field indicates whether the kick is still
    /// the current state (i.e., no re-add has reversed it). When `true`, the
    /// caller fires eviction and then calls
    /// [`MembershipRemovalFence::commit_disabling_workflows`] to durably
    /// write the workflow-disable and commit the transaction (releasing the
    /// lock). When `false`, the member was re-added and effects must be skipped.
    /// Dropping the guard without committing releases the advisory lock (the
    /// transaction is implicitly rolled back).
    #[datastore_span(name = "membership_removal_fence", system = "postgresql")]
    pub async fn membership_removal_fence(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<MembershipRemovalFence> {
        membership_removal_fence(&self.pool, community_id, channel_id, pubkey).await
    }

    /// Return the active (channel, pubkey) membership pairs among the given
    /// sets, in one statement.
    #[datastore_span(name = "membership_pairs", system = "postgresql")]
    pub async fn membership_pairs(
        &self,
        community_id: CommunityId,
        channel_ids: &[Uuid],
        pubkeys: &[Vec<u8>],
    ) -> Result<Vec<(Uuid, Vec<u8>)>> {
        membership_pairs(&self.pool, community_id, channel_ids, pubkeys).await
    }

    /// Returns all active members of a channel.
    #[datastore_span(name = "get_members", system = "postgresql")]
    pub async fn get_members(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Vec<MemberRecord>> {
        get_members(&self.pool, community_id, channel_id).await
    }

    /// Return a channel roster used to build or validate an event mutation.
    #[datastore_span(name = "get_members_for_event_write", system = "postgresql")]
    pub async fn get_members_for_event_write(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Vec<MemberRecord>> {
        get_members_with_operation(
            &self.pool,
            community_id,
            channel_id,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
    }

    /// Returns active members for multiple channels in a single query.
    #[datastore_span(name = "get_members_bulk", system = "postgresql")]
    pub async fn get_members_bulk(
        &self,
        community_id: CommunityId,
        channel_ids: &[Uuid],
    ) -> Result<Vec<MemberRecord>> {
        get_members_bulk(&self.pool, community_id, channel_ids).await
    }

    /// Get all channel IDs accessible to a pubkey.
    #[datastore_span(name = "get_accessible_channel_ids", system = "postgresql")]
    pub async fn get_accessible_channel_ids(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Vec<Uuid>> {
        get_accessible_channel_ids(&self.pool, community_id, pubkey).await
    }

    /// The channels of non-legacy types a pubkey can read (member, or open
    /// and listed or among `named`), and the unlisted open channels among
    /// `named`. See [`get_readable_named_only_channels`].
    #[datastore_span(name = "get_readable_named_only_channels", system = "postgresql")]
    pub async fn get_readable_named_only_channels(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        named: &[Uuid],
    ) -> Result<Vec<ReadableNamedOnlyChannel>> {
        get_readable_named_only_channels(&self.pool, community_id, pubkey, named).await
    }

    /// The type of channel `channel_id`, deleted or not, or `None` when no
    /// such channel exists.
    #[datastore_span(name = "channel_type_of", system = "postgresql")]
    pub async fn channel_type_of(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Option<String>> {
        channel_type_of(&self.pool, community_id, channel_id).await
    }

    /// The channels in `channel_ids` whose type is not a legacy type, deleted
    /// or not, each with its type.
    #[datastore_span(name = "named_only_channels_among", system = "postgresql")]
    pub async fn named_only_channels_among(
        &self,
        community_id: CommunityId,
        channel_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, String)>> {
        named_only_channels_among(&self.pool, community_id, channel_ids).await
    }

    /// Returns large active-channel rosters whose relay-authored snapshots differ.
    #[datastore_span(
        name = "list_large_channel_rosters_needing_reconciliation",
        system = "postgresql"
    )]
    pub async fn list_large_channel_rosters_needing_reconciliation(
        &self,
        minimum_members: i64,
        relay_pubkey: &[u8],
    ) -> Result<Vec<LargeChannelRoster>> {
        list_large_channel_rosters_needing_reconciliation(&self.pool, minimum_members, relay_pubkey)
            .await
    }

    /// Returns full channel records for all channels a user can access.
    #[datastore_span(name = "get_accessible_channels", system = "postgresql")]
    pub async fn get_accessible_channels(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        visibility_filter: Option<&str>,
        member_only: Option<bool>,
    ) -> Result<Vec<AccessibleChannel>> {
        get_accessible_channels(
            &self.pool,
            community_id,
            pubkey,
            visibility_filter,
            member_only,
        )
        .await
    }

    /// Returns all bot-role members with their aggregated channel names in one community.
    #[datastore_span(name = "get_bot_members", system = "postgresql")]
    pub async fn get_bot_members(&self, community_id: CommunityId) -> Result<Vec<BotMemberRecord>> {
        get_bot_members(&self.pool, community_id).await
    }

    /// Bulk-fetch user records by pubkey.
    #[datastore_span(name = "get_users_bulk", system = "postgresql")]
    pub async fn get_users_bulk(
        &self,
        community_id: CommunityId,
        pubkeys: &[Vec<u8>],
    ) -> Result<Vec<UserRecord>> {
        get_users_bulk(&self.pool, community_id, pubkeys).await
    }

    /// Bulk-fetch user names while constructing an event and its mention tags.
    #[datastore_span(name = "get_users_bulk_for_event_write", system = "postgresql")]
    pub async fn get_users_bulk_for_event_write(
        &self,
        community_id: CommunityId,
        pubkeys: &[Vec<u8>],
    ) -> Result<Vec<UserRecord>> {
        get_users_bulk_with_operation(
            &self.pool,
            community_id,
            pubkeys,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
    }

    /// Returns the count of active members in a channel.
    #[datastore_span(name = "get_member_count", system = "postgresql")]
    pub async fn get_member_count(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<i64> {
        get_member_count(&self.pool, community_id, channel_id).await
    }

    /// Bulk-fetch member counts for a set of channel IDs.
    #[datastore_span(name = "get_member_counts_bulk", system = "postgresql")]
    pub async fn get_member_counts_bulk(
        &self,
        community_id: CommunityId,
        channel_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, i64>> {
        get_member_counts_bulk(&self.pool, community_id, channel_ids).await
    }

    /// Get the active role of a pubkey in a channel.
    #[datastore_span(name = "get_member_role", system = "postgresql")]
    pub async fn get_member_role(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<Option<String>> {
        get_member_role(&self.pool, community_id, channel_id, pubkey).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::channel::{ChannelType, ChannelVisibility};
    use crate::migration;
    use crate::user::{ensure_user, set_agent_owner};
    use nostr::Keys;
    use sqlx::postgres::PgPoolOptions;

    async fn setup_pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB")
    }

    fn random_pubkey() -> Vec<u8> {
        Keys::generate().public_key().to_bytes().to_vec()
    }

    async fn make_test_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("channel-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        id
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_test_channel(
        pool: &PgPool,
        community_id: Uuid,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> Result<ChannelRecord> {
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline)
            VALUES
                ($1, $2, $3, $4::channel_type, $5::channel_visibility, $6, $7, $8,
                 CASE WHEN $8 IS NOT NULL THEN NOW() + ($8 || ' seconds')::interval ELSE NULL END)
            "#,
        )
        .bind(id)
        .bind(community_id)
        .bind(name)
        .bind(channel_type.as_str())
        .bind(visibility.as_str())
        .bind(description)
        .bind(created_by)
        .bind(ttl_seconds)
        .execute(pool)
        .await
        .expect("insert test channel");

        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            VALUES ($1, $2, $3, 'owner', $4)
            "#,
        )
        .bind(community_id)
        .bind(id)
        .bind(created_by)
        .bind(created_by)
        .execute(pool)
        .await
        .expect("insert owner membership");

        crate::channel::get_channel(pool, CommunityId::from_uuid(community_id), id).await
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_users_bulk_is_scoped_when_pubkey_exists_in_multiple_communities() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let community_a = CommunityId::from_uuid(community_a);
        let community_b = CommunityId::from_uuid(community_b);
        let pubkey = random_pubkey();

        sqlx::query(
            "INSERT INTO users (community_id, pubkey, display_name) VALUES ($1, $2, $3), ($4, $5, $6)",
        )
        .bind(community_a.as_uuid())
        .bind(&pubkey)
        .bind("community-a-profile")
        .bind(community_b.as_uuid())
        .bind(&pubkey)
        .bind("community-b-profile")
        .execute(&pool)
        .await
        .expect("insert same pubkey in two communities");

        let users = get_users_bulk(&pool, community_a, std::slice::from_ref(&pubkey))
            .await
            .expect("bulk fetch users");

        assert_eq!(users.len(), 1);
        assert_eq!(
            users[0].display_name.as_deref(),
            Some("community-a-profile")
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_agent_owner_can_remove_bot() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner_pk = random_pubkey();
        let agent_pk = random_pubkey();

        // Create users and set agent ownership
        ensure_user(&pool, community, &owner_pk)
            .await
            .expect("ensure owner");
        ensure_user(&pool, community, &agent_pk)
            .await
            .expect("ensure agent");
        set_agent_owner(&pool, community, &agent_pk, &owner_pk)
            .await
            .expect("set agent owner");

        // Create a channel owned by someone else entirely
        let channel_owner_pk = random_pubkey();
        ensure_user(&pool, community, &channel_owner_pk)
            .await
            .expect("ensure channel owner");
        let channel = create_test_channel(
            &pool,
            community_id,
            "test-bot-remove",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &channel_owner_pk,
            None,
        )
        .await
        .expect("create channel");

        // Add owner and agent as regular members
        add_member(
            &pool,
            community,
            channel.id,
            &owner_pk,
            MemberRole::Member,
            None,
        )
        .await
        .expect("add owner as member");
        add_member(
            &pool,
            community,
            channel.id,
            &agent_pk,
            MemberRole::Member,
            None,
        )
        .await
        .expect("add agent as member");

        // Owner should be able to remove their agent
        remove_member(&pool, community, channel.id, &agent_pk, &owner_pk)
            .await
            .expect("agent owner should be able to remove their bot");

        // Verify the agent is no longer a member
        assert!(
            !is_member(&pool, community, channel.id, &agent_pk)
                .await
                .expect("is_member check"),
            "agent should no longer be a member"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn accessible_channel_ids_are_not_truncated_at_one_thousand() {
        let database_url = crate::test_support::database_url();
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let viewer = random_pubkey();
        let channel_count = 1_001;

        sqlx::query(
            r#"
            INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by)
            SELECT gen_random_uuid(), $1, 'high-volume-' || n, 'stream', 'open', $2
            FROM generate_series(1, $3) n
            "#,
        )
        .bind(community_id)
        .bind(&viewer)
        .bind(channel_count)
        .execute(&pool)
        .await
        .expect("insert high-volume open channels");

        let channel_ids = get_accessible_channel_ids(&pool, community, &viewer)
            .await
            .expect("load accessible channel ids");
        assert_eq!(channel_ids.len(), channel_count as usize);
    }

    /// `get_members` must return the complete roster, not a truncated prefix.
    ///
    /// The relay builds the kind 39002 (NIP-29 group members) snapshot and every
    /// admin role lookup from this list, so a cap silently hides late joiners:
    /// their clients never discover the channel, and an owner past the cutoff
    /// reads as a non-member.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_members_returns_full_roster_beyond_1000() {
        let database_url = crate::test_support::database_url();
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let creator = random_pubkey();

        // create_test_channel also inserts the creator as the first (owner) member.
        let channel = create_test_channel(
            &pool,
            community_id,
            "high-volume-roster",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &creator,
            None,
        )
        .await
        .expect("create test channel");

        // Bulk-insert additional members with strictly increasing `joined_at`, so
        // member N lands at roster position N (the creator holds position 0).
        // The final member is an owner joining well past the old 1000-row cutoff.
        let extra_members = 1_500;
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at)
            SELECT
                $1,
                $2,
                decode(lpad(to_hex(n), 64, '0'), 'hex'),
                (CASE WHEN n = $3 THEN 'owner' ELSE 'member' END)::member_role,
                NOW() + (n || ' seconds')::interval
            FROM generate_series(1, $3) n
            "#,
        )
        .bind(community_id)
        .bind(channel.id)
        .bind(extra_members)
        .execute(&pool)
        .await
        .expect("insert high-volume channel members");

        let members = get_members(&pool, community, channel.id)
            .await
            .expect("load channel members");

        assert_eq!(
            members.len(),
            extra_members as usize + 1,
            "get_members truncated the roster"
        );

        // The last joiner sits at the final roster position — past any
        // 1000-row cap — which also pins the documented `joined_at` ordering.
        let late_owner = hex::decode(format!("{:064x}", extra_members)).expect("hex pubkey");
        let late = members.last().expect("roster is non-empty");
        assert_eq!(
            late.pubkey, late_owner,
            "member who joined after the 1000th must be present and ordered last"
        );
        assert_eq!(
            late.role, "owner",
            "role of a late-joining owner must resolve correctly"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn large_roster_reconciliation_candidates_respect_snapshot_count_and_signer() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let creator = random_pubkey();
        let relay_pubkey = random_pubkey();
        let other_relay_pubkey = random_pubkey();
        let channel = create_test_channel(
            &pool,
            community_id,
            "stale-large-roster",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &creator,
            None,
        )
        .await
        .expect("create test channel");

        let extra_members = 1_500;
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at)
            SELECT $1, $2, decode(lpad(to_hex(n), 64, '0'), 'hex'), 'member',
                   NOW() + (n || ' seconds')::interval
            FROM generate_series(1, $3) n
            "#,
        )
        .bind(community_id)
        .bind(channel.id)
        .bind(extra_members)
        .execute(&pool)
        .await
        .expect("insert large roster");

        // Migration 0032's roster guard requires canonical four-field p tags
        // whose roles exactly match channel_members, including the creator's
        // owner row created by create_test_channel.
        let creator_hex = hex::encode(&creator);
        let stale_tags: Vec<serde_json::Value> =
            std::iter::once(serde_json::json!(["d", channel.id.to_string()]))
                .chain(std::iter::once(serde_json::json!([
                    "p",
                    creator_hex,
                    "",
                    "owner"
                ])))
                .chain(
                    (1..1_000).map(|n| serde_json::json!(["p", format!("{n:064x}"), "", "member"])),
                )
                .collect();
        let complete_tags: Vec<serde_json::Value> =
            std::iter::once(serde_json::json!(["d", channel.id.to_string()]))
                .chain(std::iter::once(serde_json::json!([
                    "p",
                    creator_hex,
                    "",
                    "owner"
                ])))
                .chain(
                    (1..=1_500)
                        .map(|n| serde_json::json!(["p", format!("{n:064x}"), "", "member"])),
                )
                .collect();
        let other_complete_tags: Vec<serde_json::Value> =
            std::iter::once(serde_json::json!(["d", channel.id.to_string()]))
                .chain(std::iter::once(serde_json::json!([
                    "p",
                    hex::encode(&creator),
                    "",
                    "owner"
                ])))
                .chain(
                    (1..=extra_members)
                        .map(|n| serde_json::json!(["p", format!("{n:064x}"), "", "member"])),
                )
                .collect();

        // Insert canonical-looking history first, then corrupt the newest row
        // with UPDATE to model a stale snapshot that predates migration 0032's
        // INSERT fence. New stale snapshots cannot be inserted once that fence
        // is deployed.
        sqlx::query(
            r#"
            INSERT INTO events
                (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id, d_tag)
            VALUES
                ($1, $2, $3, NOW() - INTERVAL '1 minute', 39002, $4, '', $5, $6, $7),
                ($1, $8, $3, NOW(), 39002, $4, '', $5, $6, $7)
            "#,
        )
        .bind(community_id)
        .bind(random_pubkey())
        .bind(&relay_pubkey)
        .bind(serde_json::Value::Array(complete_tags.clone()))
        .bind(vec![0u8; 64])
        .bind(channel.id)
        .bind(channel.id.to_string())
        .bind(random_pubkey())
        .execute(&pool)
        .await
        .expect("insert historical duplicate snapshots");
        sqlx::query(
            "UPDATE events SET tags = $1 WHERE community_id = $2 AND channel_id = $3 \
             AND kind = 39002 AND pubkey = $4 AND created_at = (SELECT MAX(created_at) \
             FROM events WHERE community_id = $2 AND channel_id = $3 AND kind = 39002 AND pubkey = $4)",
        )
        .bind(serde_json::Value::Array(stale_tags))
        .bind(community_id)
        .bind(channel.id)
        .bind(&relay_pubkey)
        .execute(&pool)
        .await
        .expect("simulate pre-fence stale live snapshot");

        // The same channel UUID in another tenant is deliberately valid. A
        // complete snapshot there must not mask this tenant's stale head.
        let other_community_id = make_test_community(&pool).await;
        // Insert directly because create_test_channel generates a fresh UUID,
        // while this test needs the same channel ID in both tenants. Direct
        // insertion skips the helper's creator membership, so add the owner
        // row explicitly below.
        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, created_by)
            VALUES ($1, $2, 'same-id-complete-roster', 'stream', 'open', $3)
            "#,
        )
        .bind(channel.id)
        .bind(other_community_id)
        .bind(&creator)
        .execute(&pool)
        .await
        .expect("insert same channel id in other tenant");
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at)
            VALUES ($1, $2, $3, 'owner', NOW())
            "#,
        )
        .bind(other_community_id)
        .bind(channel.id)
        .bind(&creator)
        .execute(&pool)
        .await
        .expect("insert other-tenant owner");
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at)
            SELECT $1, $2, decode(lpad(to_hex(n), 64, '0'), 'hex'), 'member',
                   NOW() + (n || ' seconds')::interval
            FROM generate_series(1, $3) n
            "#,
        )
        .bind(other_community_id)
        .bind(channel.id)
        .bind(extra_members)
        .execute(&pool)
        .await
        .expect("insert complete other-tenant roster");
        sqlx::query(
            r#"
            INSERT INTO events
                (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id, d_tag)
            VALUES ($1, $2, $3, NOW(), 39002, $4, '', $5, $6, $7)
            "#,
        )
        .bind(other_community_id)
        .bind(random_pubkey())
        .bind(&relay_pubkey)
        .bind(serde_json::Value::Array(other_complete_tags))
        .bind(vec![0u8; 64])
        .bind(channel.id)
        .bind(channel.id.to_string())
        .execute(&pool)
        .await
        .expect("insert complete other-tenant snapshot");

        // Put the stale channel behind the 1,000 newest channels that the old
        // list_channels-based sweep could see. This set-based scan has no such
        // pagination ceiling.
        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, created_by, created_at)
            SELECT gen_random_uuid(), $1, 'newer-decoy-' || n, 'stream', 'open', $2,
                   NOW() + (n || ' seconds')::interval
            FROM generate_series(1, 1000) n
            "#,
        )
        .bind(community_id)
        .bind(&creator)
        .execute(&pool)
        .await
        .expect("insert channels beyond old list ceiling");

        let candidates =
            list_large_channel_rosters_needing_reconciliation(&pool, 1_000, &relay_pubkey)
                .await
                .expect("find stale snapshot");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].community_id, community);
        assert_eq!(candidates[0].channel_id, channel.id);
        assert_eq!(candidates[0].member_count, 1_501);

        let other_signer_candidates =
            list_large_channel_rosters_needing_reconciliation(&pool, 1_000, &other_relay_pubkey)
                .await
                .expect("other signer is isolated from relay-authored snapshot");
        assert!(other_signer_candidates.is_empty());

        sqlx::query(
            "UPDATE events SET tags = $1, created_at = NOW() + INTERVAL '1 minute' WHERE community_id = $2 AND channel_id = $3 AND kind = 39002 AND pubkey = $4 AND created_at = (SELECT MAX(created_at) FROM events WHERE community_id = $2 AND channel_id = $3 AND kind = 39002 AND pubkey = $4 AND deleted_at IS NULL)",
        )
        .bind(serde_json::Value::Array(complete_tags))
        .bind(community_id)
        .bind(channel.id)
        .bind(&relay_pubkey)
        .execute(&pool)
        .await
        .expect("complete snapshot");

        let converged =
            list_large_channel_rosters_needing_reconciliation(&pool, 1_000, &relay_pubkey)
                .await
                .expect("check converged snapshot");
        assert!(converged.is_empty());
    }

    /// Seed a channel whose roster (owner + one member) has diverged from its
    /// live relay-authored kind-39002 snapshot (owner only).
    async fn seed_divergent_roster(pool: &PgPool, community_id: Uuid, relay_pubkey: &[u8]) -> Uuid {
        let creator = random_pubkey();
        let channel = create_test_channel(
            pool,
            community_id,
            "divergent-roster",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &creator,
            None,
        )
        .await
        .expect("create test channel");
        let tags = serde_json::json!([
            ["d", channel.id.to_string()],
            ["p", hex::encode(&creator), "", "owner"]
        ]);
        sqlx::query(
            r#"
            INSERT INTO events
                (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id, d_tag)
            VALUES ($1, $2, $3, NOW(), 39002, $4, '', $5, $6, $7)
            "#,
        )
        .bind(community_id)
        .bind(random_pubkey())
        .bind(relay_pubkey)
        .bind(tags)
        .bind(vec![0u8; 64])
        .bind(channel.id)
        .bind(channel.id.to_string())
        .execute(pool)
        .await
        .expect("insert roster snapshot");
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at) \
             VALUES ($1, $2, $3, 'member', NOW())",
        )
        .bind(community_id)
        .bind(channel.id)
        .bind(random_pubkey())
        .execute(pool)
        .await
        .expect("diverge roster");
        channel.id
    }

    /// Startup roster reconciliation must not rewrite discovery state for
    /// archived communities, nor attempt fenced writes for deleting or
    /// tombstoned ones. Same active-community predicate as #7558.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn large_roster_reconciliation_skips_archived_and_deleted_communities() {
        let pool = setup_pool().await;
        let relay_pubkey = random_pubkey();
        let mut fixtures = Vec::new();
        for label in ["active", "archived", "quiescing", "fenced", "tombstone"] {
            let community_id = make_test_community(&pool).await;
            let channel_id = seed_divergent_roster(&pool, community_id, &relay_pubkey).await;
            fixtures.push((label, community_id, channel_id));
        }
        for (label, community_id, _) in &fixtures {
            match *label {
                "active" => {}
                "archived" => {
                    sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
                        .bind(community_id)
                        .execute(&pool)
                        .await
                        .expect("archive fixture");
                }
                state => crate::test_support::set_deletion_state(&pool, *community_id, state).await,
            }
        }

        let candidates = list_large_channel_rosters_needing_reconciliation(&pool, 1, &relay_pubkey)
            .await
            .expect("list candidates");
        let found: Vec<(CommunityId, Uuid)> = candidates
            .iter()
            .map(|c| (c.community_id, c.channel_id))
            .collect();
        let (_, active_community, active_channel) = fixtures[0];
        assert_eq!(
            found,
            vec![(CommunityId::from_uuid(active_community), active_channel)],
            "only the active community's divergent roster may be reconciled"
        );
    }

    /// A random non-admin, non-owner user cannot remove someone else's bot.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_random_user_cannot_remove_bot() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner_pk = random_pubkey();
        let agent_pk = random_pubkey();
        let random_pk = random_pubkey();

        // Create users and set agent ownership
        ensure_user(&pool, community, &owner_pk)
            .await
            .expect("ensure owner");
        ensure_user(&pool, community, &agent_pk)
            .await
            .expect("ensure agent");
        ensure_user(&pool, community, &random_pk)
            .await
            .expect("ensure random");
        set_agent_owner(&pool, community, &agent_pk, &owner_pk)
            .await
            .expect("set agent owner");

        // Create a channel
        let channel_owner_pk = random_pubkey();
        ensure_user(&pool, community, &channel_owner_pk)
            .await
            .expect("ensure channel owner");
        let channel = create_test_channel(
            &pool,
            community_id,
            "test-bot-no-remove",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &channel_owner_pk,
            None,
        )
        .await
        .expect("create channel");

        // Add random user and agent as regular members
        add_member(
            &pool,
            community,
            channel.id,
            &random_pk,
            MemberRole::Member,
            None,
        )
        .await
        .expect("add random as member");
        add_member(
            &pool,
            community,
            channel.id,
            &agent_pk,
            MemberRole::Member,
            None,
        )
        .await
        .expect("add agent as member");

        // Random user should NOT be able to remove the agent
        let result = remove_member(&pool, community, channel.id, &agent_pk, &random_pk).await;
        assert!(
            result.is_err(),
            "random user should not be able to remove someone else's bot"
        );
    }

    /// SECURITY REPRO (Dawn, kind:9000 demotion report): an unprivileged plain
    /// member calls add_member with role=Member against the channel OWNER.
    /// If this succeeds, add_member has no demotion authorization and no
    /// last-owner guard.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn repro_unprivileged_member_can_demote_owner() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let victim_owner = random_pubkey();
        let attacker = random_pubkey();

        for pk in [&victim_owner, &attacker] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }

        let channel = create_test_channel(
            &pool,
            community_id,
            "repro-demote-owner",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &victim_owner,
            None,
        )
        .await
        .expect("create channel");

        // create_test_channel already seeds the creator as 'owner', mirroring
        // create_channel's own INSERT (channel.rs:131-145).
        let role_of = |members: Vec<MemberRecord>, pk: Vec<u8>| -> Option<String> {
            members.into_iter().find(|m| m.pubkey == pk).map(|m| m.role)
        };
        let before = role_of(
            get_members(&pool, community, channel.id)
                .await
                .expect("members"),
            victim_owner.clone(),
        );
        assert_eq!(
            before.as_deref(),
            Some("owner"),
            "victim must start as owner"
        );

        // Attacker: plain member, not owner/admin.
        add_member(
            &pool,
            community,
            channel.id,
            &attacker,
            MemberRole::Member,
            None,
        )
        .await
        .expect("attacker self-joins open channel");

        // The attack: attacker is `invited_by` and demotes the owner.
        let res = add_member(
            &pool,
            community,
            channel.id,
            &victim_owner,
            MemberRole::Member,
            Some(&attacker),
        )
        .await;

        let after = role_of(
            get_members(&pool, community, channel.id)
                .await
                .expect("members"),
            victim_owner.clone(),
        );
        let owners = get_members(&pool, community, channel.id)
            .await
            .expect("members")
            .into_iter()
            .filter(|m| m.role == "owner")
            .count();

        assert!(
            res.is_err(),
            "unprivileged member must not be able to demote the owner"
        );
        assert_eq!(after.as_deref(), Some("owner"), "owner role must survive");
        assert_eq!(owners, 1, "channel must still have its owner");
    }

    /// SECURITY REPRO (Dawn): same demotion on a PRIVATE channel, where the
    /// attacker is a plain member. The report claims any member suffices here.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn repro_private_channel_member_can_demote_owner() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let victim_owner = random_pubkey();
        let attacker = random_pubkey();

        for pk in [&victim_owner, &attacker] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }

        let channel = create_test_channel(
            &pool,
            community_id,
            "repro-demote-owner-private",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            &victim_owner,
            None,
        )
        .await
        .expect("create private channel");

        // Owner invites the attacker as a plain member (legitimate).
        add_member(
            &pool,
            community,
            channel.id,
            &attacker,
            MemberRole::Member,
            Some(&victim_owner),
        )
        .await
        .expect("owner invites attacker");

        // Attack: plain member demotes the owner.
        let res = add_member(
            &pool,
            community,
            channel.id,
            &victim_owner,
            MemberRole::Member,
            Some(&attacker),
        )
        .await;

        let members = get_members(&pool, community, channel.id)
            .await
            .expect("members");
        let victim_role = members
            .iter()
            .find(|m| m.pubkey == victim_owner)
            .map(|m| m.role.clone());
        let owners = members.iter().filter(|m| m.role == "owner").count();

        assert!(
            res.is_err(),
            "plain member must not be able to demote the owner on a private channel"
        );
        assert_eq!(
            victim_role.as_deref(),
            Some("owner"),
            "owner role must survive"
        );
        assert_eq!(owners, 1, "channel must still have its owner");
    }

    /// A member may promote a guest to member, as they may add a new member.
    /// They still cannot grant an elevated role, and a guest or non-member
    /// cannot promote anyone.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn member_may_promote_a_guest_to_member_only() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let member = random_pubkey();
        let guest = random_pubkey();
        let other_guest = random_pubkey();
        let outsider = random_pubkey();
        for pk in [&owner, &member, &guest, &other_guest, &outsider] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }
        let channel = create_test_channel(
            &pool,
            community_id,
            "member-promotes-guest",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");
        add_member(
            &pool,
            community,
            channel.id,
            &member,
            MemberRole::Member,
            Some(&owner),
        )
        .await
        .expect("add member");
        for pk in [&guest, &other_guest] {
            add_member(&pool, community, channel.id, pk, MemberRole::Guest, None)
                .await
                .expect("join as guest");
        }

        for (actor, role) in [
            (&other_guest, MemberRole::Member),
            (&outsider, MemberRole::Member),
            (&member, MemberRole::Admin),
        ] {
            let err = add_member(&pool, community, channel.id, &guest, role, Some(actor))
                .await
                .expect_err("promotion must be refused");
            assert!(matches!(err, DbError::AccessDenied(_)), "{err:?}");
        }

        add_member(
            &pool,
            community,
            channel.id,
            &guest,
            MemberRole::Member,
            Some(&member),
        )
        .await
        .expect("a member may promote a guest");
        let role = get_member_role(&pool, community, channel.id, &guest)
            .await
            .expect("read role");
        assert_eq!(role.as_deref(), Some("member"));

        // Promotion is one way: the member cannot demote them again.
        let err = add_member(
            &pool,
            community,
            channel.id,
            &guest,
            MemberRole::Guest,
            Some(&member),
        )
        .await
        .expect_err("a member cannot demote a member");
        assert!(matches!(err, DbError::AccessDenied(_)), "{err:?}");

        // Roles mean the same for agents: any member may raise a guest
        // agent, not only its owner.
        let agent = random_pubkey();
        let agent_owner = random_pubkey();
        for pk in [&agent, &agent_owner] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }
        set_agent_owner(&pool, community, &agent, &agent_owner)
            .await
            .expect("set agent owner");
        add_member(
            &pool,
            community,
            channel.id,
            &agent,
            MemberRole::Guest,
            None,
        )
        .await
        .expect("agent joins as guest");
        add_member(
            &pool,
            community,
            channel.id,
            &agent,
            MemberRole::Member,
            Some(&member),
        )
        .await
        .expect("any member may raise a guest agent");
        let role = get_member_role(&pool, community, channel.id, &agent)
            .await
            .expect("read role");
        assert_eq!(role.as_deref(), Some("member"));
    }

    /// `put_members` decides each person as though each were its own write:
    /// later decisions see the roles earlier people were given, including the
    /// actor's own, and the last-owner rule counts owners the call gains and
    /// loses. A refused person is not written and does not stop the others.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn put_members_decides_each_person_in_order() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let heir = random_pubkey();
        let late = random_pubkey();
        let member = random_pubkey();
        for pk in [&owner, &heir, &late, &member] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }
        let channel = create_test_channel(
            &pool,
            community_id,
            "put-members-order",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");
        put_member(&pool, community, channel.id, &member, None, Some(&owner))
            .await
            .expect("seed member");
        let put = |pubkey, requested| MemberPut { pubkey, requested };

        // The sole owner cannot step down before an heir is named, but the
        // heir is still added.
        let results = put_members(
            &pool,
            community,
            channel.id,
            &[
                put(&owner, Some(MemberRole::Member)),
                put(&heir, Some(MemberRole::Owner)),
            ],
            Some(&owner),
        )
        .await
        .expect("first batch");
        assert!(
            matches!(&results[0], Err(DbError::AccessDenied(m)) if m.contains("last owner")),
            "{:?}",
            results[0]
        );
        let heir_put = results[1].as_ref().expect("heir added");
        assert_eq!(heir_put.member.role, "owner");
        assert_eq!(heir_put.prior_role, None);

        // With the heir counted, the owner may step down in the same call. The
        // owner is then a plain member, so the admin grant after it is
        // refused, and the member re-added at their role reports that role.
        let results = put_members(
            &pool,
            community,
            channel.id,
            &[
                put(&owner, Some(MemberRole::Member)),
                put(&late, Some(MemberRole::Admin)),
                put(&member, None),
            ],
            Some(&owner),
        )
        .await
        .expect("second batch");
        let stepped_down = results[0].as_ref().expect("owner steps down");
        assert_eq!(stepped_down.member.role, "member");
        assert_eq!(stepped_down.prior_role.as_deref(), Some("owner"));
        assert!(
            matches!(&results[1], Err(DbError::AccessDenied(_))),
            "{:?}",
            results[1]
        );
        let readded = results[2].as_ref().expect("member re-added");
        assert_eq!(readded.prior_role.as_deref(), Some("member"));
        assert_eq!(readded.member.role, "member");
        assert_eq!(
            get_member_role(&pool, community, channel.id, &late)
                .await
                .expect("read role"),
            None
        );

        // A repeated pubkey refuses the call.
        let err = put_members(
            &pool,
            community,
            channel.id,
            &[put(&late, None), put(&late, None)],
            Some(&heir),
        )
        .await
        .expect_err("duplicate pubkey");
        assert!(matches!(err, DbError::InvalidData(_)), "{err:?}");
    }

    /// `put_member` enforces admission itself, so a caller that skips the
    /// relay validator still cannot join an announce channel above
    /// `join_role`, and a guest still cannot add anyone. No role tag keeps an
    /// active member's role and gives a new member `join_role`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn put_member_enforces_announce_admission() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let joiner = random_pubkey();
        let invitee = random_pubkey();
        let outsider = random_pubkey();
        for pk in [&owner, &joiner, &invitee, &outsider] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }
        let channel = create_test_channel(
            &pool,
            community_id,
            "put-member-announce",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");
        crate::channel::update_channel(
            &pool,
            community,
            channel.id,
            crate::channel::ChannelUpdate {
                join_role: Some("guest".into()),
                ..Default::default()
            },
        )
        .await
        .expect("make announce channel");

        // A joiner cannot ask for more than `join_role`.
        let err = put_member(
            &pool,
            community,
            channel.id,
            &joiner,
            Some(MemberRole::Member),
            None,
        )
        .await
        .expect_err("self-join above join_role");
        assert!(matches!(err, DbError::AccessDenied(_)), "{err:?}");
        // No role tag: a new member gets `join_role`.
        let joined = put_member(&pool, community, channel.id, &joiner, None, None)
            .await
            .expect("self-join");
        assert_eq!(joined.role, "guest");

        // A guest cannot add anyone, and neither can a non-member where
        // joining gives `guest`.
        for actor in [&joiner, &outsider] {
            let err = put_member(
                &pool,
                community,
                channel.id,
                &invitee,
                Some(MemberRole::Guest),
                Some(actor),
            )
            .await
            .expect_err("guest or outsider add");
            assert!(matches!(err, DbError::AccessDenied(_)), "{err:?}");
        }
        assert_eq!(
            get_member_role(&pool, community, channel.id, &invitee)
                .await
                .expect("read role"),
            None
        );

        // A member adds with no role tag: the new member gets `join_role`;
        // then no role tag keeps the role the owner raised them to.
        let added = put_member(&pool, community, channel.id, &invitee, None, Some(&owner))
            .await
            .expect("owner adds");
        assert_eq!(added.role, "guest");
        put_member(
            &pool,
            community,
            channel.id,
            &invitee,
            Some(MemberRole::Member),
            Some(&owner),
        )
        .await
        .expect("owner raises");
        let kept = put_member(&pool, community, channel.id, &invitee, None, Some(&owner))
            .await
            .expect("re-add without role tag");
        assert_eq!(kept.role, "member");
    }

    /// A `join_role` change and an admission are ordered by the membership
    /// lock: a join that waits on an uncommitted change to `guest` is decided
    /// on the new setting once it commits, not on the old one.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn put_member_waits_for_a_concurrent_join_role_change() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let joiner = random_pubkey();
        for pk in [&owner, &joiner] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }
        let channel = create_test_channel(
            &pool,
            community_id,
            "put-member-join-role-race",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");

        // The same lock order as `update_channel`: the lock, then the row update.
        let mut tx = pool.begin().await.expect("begin");
        acquire_channel_membership_lock(&mut tx, community, channel.id)
            .await
            .expect("lock");
        sqlx::query("UPDATE channels SET join_role = 'guest' WHERE community_id = $1 AND id = $2")
            .bind(community_id)
            .bind(channel.id)
            .execute(&mut *tx)
            .await
            .expect("change join_role");

        let join = tokio::spawn({
            let pool = pool.clone();
            let channel_id = channel.id;
            let joiner = joiner.clone();
            async move {
                put_member(
                    &pool,
                    community,
                    channel_id,
                    &joiner,
                    Some(MemberRole::Member),
                    None,
                )
                .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            !join.is_finished(),
            "the join must wait for the setting change"
        );
        tx.commit().await.expect("commit setting change");

        let err = join
            .await
            .expect("join task")
            .expect_err("the join is decided on join_role = guest");
        assert!(matches!(err, DbError::AccessDenied(_)), "{err:?}");
        assert_eq!(
            get_member_role(&pool, community, channel.id, &joiner)
                .await
                .expect("read role"),
            None
        );

        // The other order: `update_channel` waits for an admission that holds
        // the lock.
        let mut admission = pool.begin().await.expect("begin admission");
        acquire_channel_membership_lock(&mut admission, community, channel.id)
            .await
            .expect("lock");
        let update = tokio::spawn({
            let pool = pool.clone();
            let channel_id = channel.id;
            async move {
                crate::channel::update_channel(
                    &pool,
                    community,
                    channel_id,
                    crate::channel::ChannelUpdate {
                        join_role: Some("member".into()),
                        ..Default::default()
                    },
                )
                .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            !update.is_finished(),
            "a join_role change must wait for an admission in progress"
        );
        admission.commit().await.expect("commit admission");
        update
            .await
            .expect("update task")
            .expect("join_role change");
    }

    /// A `join_role` change on an ephemeral channel and a roster publication
    /// take the channel's locks in one order. The roster holds the membership
    /// lock and refreshes the channel's TTL (TTL lock, then row) at commit. A
    /// settings change that locked the row first and then waited for the
    /// membership lock would deadlock with it, or make the refresh time out
    /// and be dropped.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn join_role_change_waits_for_a_roster_publication_on_an_ephemeral_channel() {
        join_role_change_waits_for_roster_publication(SettingsWriter::UpdateChannel).await;
    }

    /// The kind:9002 path stores the event and applies its changes in its own
    /// transaction. It must take the same locks in the same order as
    /// `update_channel`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn kind_9002_join_role_change_waits_for_a_roster_publication_on_an_ephemeral_channel() {
        join_role_change_waits_for_roster_publication(SettingsWriter::Kind9002).await;
    }

    #[derive(Clone, Copy)]
    enum SettingsWriter {
        UpdateChannel,
        Kind9002,
    }

    async fn join_role_change_waits_for_roster_publication(writer: SettingsWriter) {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let channel = create_test_channel(
            &pool,
            community_id,
            "roster-join-role-lock-order",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            Some(3600),
        )
        .await
        .expect("create ephemeral channel");
        let deadline_before = channel.ttl_deadline.expect("ephemeral deadline");

        let snapshot_pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect one-connection pool");
        let relay_keys = Keys::generate();
        let mut snapshot = lock_member_snapshot(
            &snapshot_pool,
            community,
            channel.id,
            &relay_keys.public_key().to_bytes(),
        )
        .await
        .expect("capture locked roster");

        let update = tokio::spawn({
            let pool = pool.clone();
            let channel_id = channel.id;
            async move {
                let updates = crate::channel::ChannelUpdate {
                    join_role: Some("guest".into()),
                    ..Default::default()
                };
                match writer {
                    SettingsWriter::UpdateChannel => {
                        crate::channel::update_channel(&pool, community, channel_id, updates).await
                    }
                    SettingsWriter::Kind9002 => {
                        let edit_event = nostr::EventBuilder::new(nostr::Kind::Custom(9002), "")
                            .tags(vec![
                                nostr::Tag::parse(["h", &channel_id.to_string()]).expect("h tag"),
                                nostr::Tag::parse(["join_role", "guest"]).expect("join_role tag"),
                            ])
                            .sign_with_keys(&Keys::generate())
                            .expect("sign kind:9002");
                        let write = crate::channel::apply_channel_metadata_event(
                            &pool,
                            None,
                            community,
                            channel_id,
                            &edit_event,
                            crate::channel::ChannelMetadataEdit {
                                columns: updates,
                                ..Default::default()
                            },
                            &[],
                        )
                        .await?;
                        assert!(write.inserted, "the kind:9002 event must be stored");
                        crate::channel::get_channel(&pool, community, channel_id).await
                    }
                }
            }
        });
        // The kind:9002 writer also stores an event before it would reach
        // the row, so give an unblocked writer time to finish. A blocked
        // writer waits until the roster commits, so this cannot pass early.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert!(
            !update.is_finished(),
            "the join_role change must wait for the roster publication"
        );

        let event = nostr::EventBuilder::new(nostr::Kind::Custom(39002), "")
            .tags(vec![
                nostr::Tag::parse(["d", &channel.id.to_string()]).expect("d tag"),
                nostr::Tag::parse(["p", &hex::encode(&owner), "", "owner"]).expect("p tag"),
            ])
            .sign_with_keys(&relay_keys)
            .expect("sign roster");
        let (_, inserted) = snapshot
            .replace_member_event(&event)
            .await
            .expect("replace roster");
        assert!(inserted);
        snapshot.release().await.expect("publish roster");

        let updated = update
            .await
            .expect("update task")
            .expect("join_role change after the roster");
        assert_eq!(updated.join_role, "guest");
        let deadline_after = updated.ttl_deadline.expect("ephemeral deadline");
        assert!(
            deadline_after > deadline_before,
            "the roster's TTL refresh must not be dropped: {deadline_before} -> {deadline_after}"
        );
    }

    /// The fix must not break legitimate role management: an owner demoting a
    /// co-owner (while another owner remains) must still succeed, and promotion
    /// by an owner must still succeed.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_can_still_manage_roles_after_demotion_guard() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let other = random_pubkey();

        for pk in [&owner, &other] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }

        let channel = create_test_channel(
            &pool,
            community_id,
            "roles-still-manageable",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");

        // Owner promotes `other` to owner — allowed (actor is elevated).
        add_member(
            &pool,
            community,
            channel.id,
            &other,
            MemberRole::Owner,
            Some(&owner),
        )
        .await
        .expect("owner may promote to owner");

        // Owner demotes the co-owner back to member — allowed: actor is elevated
        // and another owner remains, so the last-owner guard does not trip.
        add_member(
            &pool,
            community,
            channel.id,
            &other,
            MemberRole::Member,
            Some(&owner),
        )
        .await
        .expect("owner may demote a co-owner while another owner remains");

        let members = get_members(&pool, community, channel.id)
            .await
            .expect("members");
        let role_of = |pk: &Vec<u8>| {
            members
                .iter()
                .find(|m| &m.pubkey == pk)
                .map(|m| m.role.clone())
        };
        assert_eq!(role_of(&other).as_deref(), Some("member"));
        assert_eq!(role_of(&owner).as_deref(), Some("owner"));

        // Idempotent re-add at the SAME role must stay unguarded even from a
        // non-elevated actor — the huddle bot-add path depends on this.
        let bot = random_pubkey();
        ensure_user(&pool, community, &bot)
            .await
            .expect("ensure bot");
        add_member(
            &pool,
            community,
            channel.id,
            &bot,
            MemberRole::Bot,
            Some(&owner),
        )
        .await
        .expect("add bot");
        add_member(
            &pool,
            community,
            channel.id,
            &bot,
            MemberRole::Bot,
            Some(&other),
        )
        .await
        .expect("re-adding at the same role must remain idempotent");

        // But the last owner cannot be demoted, even by themselves.
        let err = add_member(
            &pool,
            community,
            channel.id,
            &owner,
            MemberRole::Member,
            Some(&owner),
        )
        .await
        .expect_err("last owner must not be demotable");
        println!("last-owner demotion rejected: {err}");
    }

    /// Isolates the actor-authorization guard from the last-owner guard.
    ///
    /// `repro_unprivileged_member_can_demote_owner` demotes the *sole* owner, so
    /// the last-owner guard alone is enough to reject it: stubbing out the actor
    /// check leaves that test green and the authorization hole invisible. Here a
    /// second owner remains, so the last-owner guard cannot fire and only the
    /// actor check stands between an unprivileged member and a co-owner's role.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unprivileged_member_cannot_demote_a_co_owner() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let co_owner = random_pubkey();
        let attacker = random_pubkey();

        for pk in [&owner, &co_owner, &attacker] {
            ensure_user(&pool, community, pk)
                .await
                .expect("ensure user");
        }

        let channel = create_test_channel(
            &pool,
            community_id,
            "co-owner-demotion-authz",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");

        add_member(
            &pool,
            community,
            channel.id,
            &co_owner,
            MemberRole::Owner,
            Some(&owner),
        )
        .await
        .expect("owner may promote a co-owner");

        add_member(
            &pool,
            community,
            channel.id,
            &attacker,
            MemberRole::Member,
            None,
        )
        .await
        .expect("attacker self-joins the open channel");

        // Two owners remain, so the last-owner guard cannot reject this. Only
        // the actor-authorization check can.
        let err = add_member(
            &pool,
            community,
            channel.id,
            &co_owner,
            MemberRole::Member,
            Some(&attacker),
        )
        .await
        .expect_err("an unprivileged member must not demote a co-owner");
        println!("co-owner demotion by unprivileged actor rejected: {err}");

        let members = get_members(&pool, community, channel.id)
            .await
            .expect("members");
        let role_of = |pk: &Vec<u8>| {
            members
                .iter()
                .find(|m| &m.pubkey == pk)
                .map(|m| m.role.clone())
        };
        assert_eq!(
            role_of(&co_owner).as_deref(),
            Some("owner"),
            "co-owner must keep their role"
        );
        assert_eq!(
            members.iter().filter(|m| m.role == "owner").count(),
            2,
            "both owners must survive"
        );
    }

    /// Sets up an open channel with exactly two owners, returning
    /// `(community, channel_id, owner_a, owner_b)`.
    async fn channel_with_two_owners(
        pool: &PgPool,
        name: &str,
    ) -> (CommunityId, Uuid, Vec<u8>, Vec<u8>) {
        let community_id = make_test_community(pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner_a = random_pubkey();
        let owner_b = random_pubkey();
        for pk in [&owner_a, &owner_b] {
            ensure_user(pool, community, pk).await.expect("ensure user");
        }

        let channel = create_test_channel(
            pool,
            community_id,
            name,
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner_a,
            None,
        )
        .await
        .expect("create channel");

        add_member(
            pool,
            community,
            channel.id,
            &owner_b,
            MemberRole::Owner,
            Some(&owner_a),
        )
        .await
        .expect("promote second owner");

        (community, channel.id, owner_a, owner_b)
    }

    /// Commits `DISABLE TRIGGER` on the test database. Safe only under the
    /// per-test database wrapper; do not run these against a shared database,
    /// or later trigger-dependent tests there will see no trigger.
    async fn disable_roster_trigger(pool: &PgPool) {
        let partitions: Vec<String> = sqlx::query_scalar(
            "SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) \
             FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE i.inhparent = 'public.events'::regclass",
        )
        .fetch_all(pool)
        .await
        .expect("list event partitions");
        for partition in partitions {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "ALTER TABLE {partition} DISABLE TRIGGER trg_events_guard_channel_roster_snapshot"
            )))
            .execute(pool)
            .await
            .expect("disable only the roster trigger on partition");
        }
        sqlx::query(
            "ALTER TABLE ONLY events DISABLE TRIGGER trg_events_guard_channel_roster_snapshot",
        )
        .execute(pool)
        .await
        .expect("disable only the roster trigger on parent");
        assert!(
            verify_channel_roster_fence_catalog(pool).await.is_err(),
            "app-only tests must run without the catalog-verified roster fence"
        );
    }

    fn signed_roster(
        keys: &Keys,
        channel_id: Uuid,
        members: &[(&[u8], &str, &str)],
    ) -> nostr::Event {
        use nostr::{EventBuilder, Kind, Tag};

        let mut tags = vec![Tag::parse(["d", &channel_id.to_string()]).expect("d tag")];
        for (pubkey, relay, role) in members {
            tags.push(Tag::parse(["p", &hex::encode(pubkey), relay, role]).expect("p tag"));
        }
        EventBuilder::new(Kind::Custom(39002), "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign roster")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn app_roster_guard_locked_writer_rejects_stale_and_accepts_current_without_trigger() {
        let pool = setup_pool().await;
        disable_roster_trigger(&pool).await;
        let community_uuid = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_uuid);
        let owner = random_pubkey();
        let channel = create_test_channel(
            &pool,
            community_uuid,
            "app-roster-locked",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");
        let keys = Keys::generate();
        let stale = signed_roster(&keys, channel.id, &[]);
        let mut snapshot =
            lock_member_snapshot(&pool, community, channel.id, &keys.public_key().to_bytes())
                .await
                .expect("lock snapshot");
        let error = snapshot
            .replace_member_event(&stale)
            .await
            .expect_err("app must reject stale roster without trigger");
        assert!(matches!(error, DbError::Sqlx(sqlx::Error::Database(ref db))
            if db.code().as_deref() == Some("23514")
            && db.message() == "kind 39002 roster does not match canonical channel membership"));
        drop(snapshot);

        let current = signed_roster(&keys, channel.id, &[(&owner, "", "owner")]);
        let mut snapshot =
            lock_member_snapshot(&pool, community, channel.id, &keys.public_key().to_bytes())
                .await
                .expect("lock current snapshot");
        assert!(
            snapshot
                .replace_member_event(&current)
                .await
                .expect("current roster")
                .1
        );
        snapshot.release().await.expect("commit current roster");
        let live_id: Vec<u8> = sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id=$1 AND channel_id=$2 \
             AND kind=39002 AND deleted_at IS NULL",
        )
        .bind(community_uuid)
        .bind(channel.id)
        .fetch_one(&pool)
        .await
        .expect("load current roster");
        assert_eq!(live_id, current.id.as_bytes().to_vec());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn app_roster_guard_matches_trigger_for_roster_input_table() {
        use nostr::{EventBuilder, Kind, Tag};
        use serde_json::json;

        let pool = setup_pool().await;
        verify_channel_roster_fence_catalog(&pool)
            .await
            .expect("trigger side must start with the real fence enabled");
        let owner = random_pubkey();
        let owner_hex = hex::encode(&owner);
        let extra_hex = hex::encode(random_pubkey());
        let invalid = "kind 39002 roster contains an invalid p tag";
        let mismatch = "kind 39002 roster does not match canonical channel membership";
        let mut cases = vec![
            (
                "three-field p",
                json!([["p", owner_hex, ""]]),
                Some("owner"),
                Some(invalid),
            ),
            (
                "five-field p",
                json!([["p", owner_hex, "", "owner", "extra"]]),
                Some("owner"),
                Some(invalid),
            ),
            (
                "63-char pubkey",
                json!([["p", &owner_hex[..63], "", "owner"]]),
                Some("owner"),
                Some(invalid),
            ),
            (
                "non-hex pubkey",
                json!([["p", "z".repeat(64), "", "owner"]]),
                Some("owner"),
                Some(invalid),
            ),
            (
                "nonempty relay",
                json!([["p", owner_hex, "wss://relay.invalid", "owner"]]),
                Some("owner"),
                Some(invalid),
            ),
            (
                "unknown role",
                json!([["p", owner_hex, "", "moderator"]]),
                Some("owner"),
                Some(invalid),
            ),
            (
                "duplicate member",
                json!([["p", owner_hex, "", "owner"], ["p", owner_hex, "", "owner"]]),
                Some("owner"),
                Some(mismatch),
            ),
            (
                "role mismatch",
                json!([["p", owner_hex, "", "admin"]]),
                Some("owner"),
                Some(mismatch),
            ),
            ("missing member", json!([]), Some("owner"), Some(mismatch)),
            (
                "extra member",
                json!([
                    ["p", owner_hex, "", "owner"],
                    ["p", extra_hex, "", "member"]
                ]),
                Some("owner"),
                Some(mismatch),
            ),
            (
                "uppercase real member",
                json!([["p", owner_hex.to_uppercase(), "", "owner"]]),
                Some("owner"),
                None,
            ),
            (
                "non-p tags ignored",
                json!([
                    ["p", owner_hex, "", "owner"],
                    ["name", "ignored"],
                    ["x"],
                    ["P", "not a p tag"]
                ]),
                Some("owner"),
                None,
            ),
            ("empty roster and membership", json!([]), None, None),
            (
                "nonempty roster and empty membership",
                json!([["p", owner_hex, "", "owner"]]),
                None,
                Some(mismatch),
            ),
        ];
        for role in ["owner", "admin", "bot", "member", "guest"] {
            cases.push((role, json!([["p", owner_hex, "", role]]), Some(role), None));
        }
        assert_ne!(
            owner_hex,
            owner_hex.to_uppercase(),
            "uppercase case must change the input"
        );

        let database_error = |error: sqlx::Error| match error {
            sqlx::Error::Database(error) => (
                error.code().expect("SQLSTATE").into_owned(),
                error.message().to_owned(),
            ),
            error => panic!("expected a database verdict, got {error:?}"),
        };
        let keys = Keys::generate();
        let mut measured = Vec::new();
        // Measure all trigger verdicts first with raw inserts, bypassing every
        // app guard. Each savepoint and transaction is rolled back so the app
        // later sees exactly the same canonical membership and no event head.
        for (name, tags, role, expected_error) in cases {
            let community_uuid = make_test_community(&pool).await;
            let community = CommunityId::from_uuid(community_uuid);
            let channel = create_test_channel(
                &pool,
                community_uuid,
                name,
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &owner,
                None,
            )
            .await
            .expect("create case channel");
            if let Some(role) = role {
                sqlx::query("UPDATE channel_members SET role=$3::member_role WHERE community_id=$1 AND channel_id=$2")
                    .bind(community_uuid)
                    .bind(channel.id)
                    .bind(role)
                    .execute(&pool)
                    .await
                    .expect("set canonical case role");
            } else {
                sqlx::query("UPDATE channel_members SET removed_at=NOW() WHERE community_id=$1 AND channel_id=$2")
                    .bind(community_uuid)
                    .bind(channel.id)
                    .execute(&pool)
                    .await
                    .expect("empty active membership");
            }
            let mut tags: Vec<Vec<String>> = serde_json::from_value(tags).expect("case tag arrays");
            tags.insert(0, vec!["d".into(), channel.id.to_string()]);
            let event = EventBuilder::new(Kind::Custom(39002), "")
                .tags(tags.iter().map(|tag| Tag::parse(tag).expect("case tag")))
                .sign_with_keys(&keys)
                .expect("sign case");
            let mut tx = pool.begin().await.expect("trigger transaction");
            sqlx::query("SAVEPOINT roster_case")
                .execute(&mut *tx)
                .await
                .expect("case savepoint");
            let trigger = sqlx::query(
                "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
                 VALUES ($1, $2, $3, $4, 39002, $5, '', $6, NOW(), $7, $8)",
            )
            .bind(community_uuid)
            .bind(event.id.as_bytes().as_slice())
            .bind(event.pubkey.to_bytes().as_slice())
            .bind(DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).expect("timestamp"))
            .bind(serde_json::to_value(&event.tags).expect("tags"))
            .bind(event.sig.serialize().as_slice())
            .bind(channel.id)
            .bind(channel.id.to_string())
            .execute(&mut *tx)
            .await
            .map(|result| result.rows_affected() == 1)
            .map_err(database_error);
            sqlx::query("ROLLBACK TO SAVEPOINT roster_case")
                .execute(&mut *tx)
                .await
                .expect("undo trigger case");
            tx.rollback().await.expect("undo trigger transaction");
            let expected =
                expected_error.map_or(Ok(true), |message| Err(("23514".into(), message.into())));
            assert_eq!(trigger, expected, "trigger contract: {name}");
            measured.push((name, community, channel.id, event, trigger));
        }

        // Only the app side runs with the trigger disabled; this helper also
        // asserts that the retained catalog check now rejects the database.
        disable_roster_trigger(&pool).await;
        let db = Db::from_pool(pool.clone());
        for (name, community, channel, event, trigger) in measured {
            let app = db
                .replace_addressable_event(community, &event, Some(channel))
                .await
                .map(|(_, inserted)| inserted)
                .map_err(|error| match error {
                    DbError::Sqlx(error) => database_error(error),
                    error => panic!("expected app database verdict for {name}, got {error:?}"),
                });
            assert_eq!(app, trigger, "app/trigger parity: {name}");
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn app_roster_guard_covers_generic_and_parameterized_writers_without_trigger() {
        let pool = setup_pool().await;
        disable_roster_trigger(&pool).await;
        let community_uuid = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_uuid);
        let owner = random_pubkey();
        let channel = create_test_channel(
            &pool,
            community_uuid,
            "app-roster-generic",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");
        let stale = signed_roster(&Keys::generate(), channel.id, &[]);
        let db = Db::from_pool(pool.clone());

        for result in [
            db.insert_event(community, &stale, Some(channel.id)).await,
            db.insert_event_with_thread_metadata(community, &stale, Some(channel.id), None)
                .await,
            db.replace_parameterized_event(
                community,
                &stale,
                &channel.id.to_string(),
                Some(channel.id),
            )
            .await,
        ] {
            let error = result.expect_err("every event writer must reject stale roster");
            assert!(matches!(error, DbError::Sqlx(sqlx::Error::Database(ref db))
                if db.code().as_deref() == Some("23514")
                && db.message() == "kind 39002 roster does not match canonical channel membership"));
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn app_roster_guard_rechecks_after_concurrent_membership_commit_without_trigger() {
        let pool = setup_pool().await;
        disable_roster_trigger(&pool).await;
        let community_uuid = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_uuid);
        let owner = random_pubkey();
        let newcomer = random_pubkey();
        let channel = create_test_channel(
            &pool,
            community_uuid,
            "app-roster-race",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");
        let old = signed_roster(&Keys::generate(), channel.id, &[(&owner, "", "owner")]);

        let mut holder = pool.begin().await.expect("hold membership transaction");
        acquire_channel_membership_lock(&mut holder, community, channel.id)
            .await
            .expect("hold membership key");
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'member', $4)",
        )
        .bind(community_uuid)
        .bind(channel.id)
        .bind(&newcomer)
        .bind(&owner)
        .execute(&mut *holder)
        .await
        .expect("stage newer canonical roster");

        let writer_pool = pool.clone();
        let old_id = old.id.as_bytes().to_vec();
        let writer = tokio::spawn(async move {
            Db::from_pool(writer_pool)
                .replace_addressable_event(community, &old, Some(channel.id))
                .await
        });
        // A waiting advisory lock proves the old publication has reached the
        // app fence before the newer canonical membership commits.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' \
                     AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database()))",
                )
                .fetch_one(&pool)
                .await
                .expect("observe waiting roster writer");
                if waiting { break; }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("old roster writer must wait for membership key");
        holder
            .commit()
            .await
            .expect("commit newer canonical roster");

        let error = writer
            .await
            .expect("join old writer")
            .expect_err("old in-flight roster must be rejected");
        assert!(matches!(error, DbError::Sqlx(sqlx::Error::Database(ref db))
            if db.code().as_deref() == Some("23514")
            && db.message() == "kind 39002 roster does not match canonical channel membership"));
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE id=$1")
            .bind(old_id)
            .fetch_one(&pool)
            .await
            .expect("count rejected old roster");
        assert_eq!(count, 0);
    }

    /// A captured roster holds the same lock as membership writers until the
    /// publisher explicitly releases it. This is the freshness fence used by
    /// rolling-deploy reconciliation.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn locked_member_snapshot_blocks_post_capture_membership_mutation() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let newcomer = random_pubkey();
        let channel = create_test_channel(
            &pool,
            community_id,
            "snapshot-freshness-fence",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");

        let snapshot_pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect one-connection pool");
        let relay_keys = Keys::generate();
        let mut snapshot = lock_member_snapshot(
            &snapshot_pool,
            community,
            channel.id,
            &relay_keys.public_key().to_bytes(),
        )
        .await
        .expect("capture locked roster");
        assert_eq!(snapshot.members.len(), 1);
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(39002), "")
            .tags(vec![
                nostr::Tag::parse(["d", &channel.id.to_string()]).expect("d tag"),
                nostr::Tag::parse(["p", &hex::encode(&owner), "", "owner"]).expect("p tag"),
            ])
            .sign_with_keys(&relay_keys)
            .expect("sign roster");
        let other_channel = nostr::EventBuilder::new(nostr::Kind::Custom(39002), "")
            .tags(vec![
                nostr::Tag::parse(["d", &Uuid::new_v4().to_string()]).expect("d tag"),
                nostr::Tag::parse(["p", &hex::encode(&owner), "", "owner"]).expect("p tag"),
            ])
            .sign_with_keys(&relay_keys)
            .expect("sign roster for another channel");
        assert!(
            matches!(
                snapshot.replace_member_event(&other_channel).await,
                Err(DbError::InvalidData(_))
            ),
            "a roster whose d tag names another channel must not replace the locked one"
        );
        let (_, inserted) = snapshot
            .replace_member_event(&event)
            .await
            .expect("replace roster on held connection");
        assert!(inserted);

        let mut contender = pool.begin().await.expect("begin membership writer");
        let acquired: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!(
                    "{CHANNEL_MEMBERSHIP_LOCK_NAMESPACE}{}:{}",
                    community.as_uuid(),
                    channel.id
                ))
                .fetch_one(&mut *contender)
                .await
                .expect("try membership writer lock");
        assert!(
            !acquired,
            "membership mutation must wait until the captured roster is published"
        );
        contender.rollback().await.expect("rollback contender");

        snapshot.release().await.expect("release snapshot fence");
        add_member(
            &pool,
            community,
            channel.id,
            &newcomer,
            MemberRole::Member,
            None,
        )
        .await
        .expect("membership mutation after publication");
        assert_eq!(
            get_members(&pool, community, channel.id)
                .await
                .expect("fresh roster")
                .len(),
            2
        );
    }

    /// Captured roster publication must hold the same shared community-deletion
    /// lock as serving event writes so deletion fencing can serialize against it.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn locked_member_snapshot_holds_shared_community_fence_lock() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let channel = create_test_channel(
            &pool,
            community_id,
            "snapshot-community-fence-lock",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");

        let snapshot_pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect one-connection pool");
        let relay_keys = Keys::generate();
        let snapshot = lock_member_snapshot(
            &snapshot_pool,
            community,
            channel.id,
            &relay_keys.public_key().to_bytes(),
        )
        .await
        .expect("capture locked roster");

        let mut contender = pool.begin().await.expect("begin deletion contender");
        let exclusive_taken: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(community_deletion_lock_key($1))")
                .bind(community.as_uuid())
                .fetch_one(&mut *contender)
                .await
                .expect("try exclusive community deletion lock");
        assert!(
            !exclusive_taken,
            "snapshot publication must hold a shared community deletion lock"
        );
        contender.rollback().await.expect("rollback contender");

        snapshot.release().await.expect("release snapshot fence");
    }

    /// The lock must be shared with `remove_member`: a demotion racing an owner
    /// removal goes through a separate count/update path, so both must serialize
    /// on the same key or they can jointly empty the owner set.
    ///
    /// Deterministic rather than timing-based: an outer transaction takes the
    /// per-channel membership key first, then each membership writer must block
    /// until it is released. Verified by mutation — dropping the lock from either
    /// function makes that call return immediately and fails this test.
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn membership_writes_serialize_on_the_shared_channel_lock() {
        let pool = setup_pool().await;
        let (community, channel_id, owner_a, owner_b) =
            channel_with_two_owners(&pool, "membership-lock-shared").await;

        for label in ["add_member", "remove_member"] {
            // Hold the same advisory key an in-tree membership write would take.
            let mut holder = pool.begin().await.expect("begin lock holder");
            acquire_channel_membership_lock(&mut holder, community, channel_id)
                .await
                .expect("holder acquires membership key");

            let pool2 = pool.clone();
            let (target, actor) = (owner_a.clone(), owner_b.clone());
            let mut writer = tokio::spawn(async move {
                match label {
                    "add_member" => add_member(
                        &pool2,
                        community,
                        channel_id,
                        &target,
                        MemberRole::Member,
                        Some(&actor),
                    )
                    .await
                    .map(|_| ()),
                    _ => remove_member(&pool2, community, channel_id, &target, &actor).await,
                }
            });

            // While the key is held, the writer must make no progress.
            let blocked =
                tokio::time::timeout(std::time::Duration::from_millis(750), &mut writer).await;
            assert!(
                blocked.is_err(),
                "{label} completed while the channel membership key was held — \
                 it is not serializing on the shared lock"
            );
            println!("{label} blocked on the held membership key, as required");

            // Releasing the key lets it proceed.
            holder.rollback().await.expect("release membership key");
            tokio::time::timeout(std::time::Duration::from_secs(10), writer)
                .await
                .expect("writer must proceed once the key is released")
                .expect("writer task panicked")
                .expect("writer must succeed after the key is released");

            // Restore two owners for the next iteration.
            add_member(
                &pool,
                community,
                channel_id,
                &owner_a,
                MemberRole::Owner,
                Some(&owner_b),
            )
            .await
            .expect("restore second owner");
        }
    }

    /// Every *mutable* authorization read must sit behind the membership lock.
    /// A remover that reads its elevated role before acquiring the lock can be
    /// demoted by a concurrent writer and still proceed on the stale role.
    ///
    /// Deterministic: the holder takes the key, `remove_member` blocks on it, the
    /// holder then demotes the remover and commits. Once the key is released the
    /// remover must re-read its (now unprivileged) role and be rejected.
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn remove_member_rejects_an_actor_demoted_while_it_waited() {
        let pool = setup_pool().await;
        let (community, channel_id, owner_a, owner_b) =
            channel_with_two_owners(&pool, "stale-actor-role").await;
        // owner_b removes a plain member, so the last-owner guard is not what
        // rejects this — only the actor's own role can.
        let victim = random_pubkey();
        ensure_user(&pool, community, &victim)
            .await
            .expect("ensure victim");
        add_member(
            &pool,
            community,
            channel_id,
            &victim,
            MemberRole::Member,
            Some(&owner_a),
        )
        .await
        .expect("add victim");

        let mut holder = pool.begin().await.expect("begin lock holder");
        acquire_channel_membership_lock(&mut holder, community, channel_id)
            .await
            .expect("holder acquires membership key");

        let pool2 = pool.clone();
        let (actor, target) = (owner_b.clone(), victim.clone());
        let mut remover = tokio::spawn(async move {
            remove_member(&pool2, community, channel_id, &target, &actor).await
        });

        // Must be waiting on the key, not already authorized past it.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(750), &mut remover)
                .await
                .is_err(),
            "remove_member must block on the membership key before authorizing"
        );

        // Demote the waiting actor to a plain member and release the key.
        sqlx::query(
            "UPDATE channel_members SET role = 'member' \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(&owner_b)
        .execute(&mut *holder)
        .await
        .expect("demote the waiting actor");
        holder.commit().await.expect("commit demotion");

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), remover)
            .await
            .expect("remover must proceed once the key is released")
            .expect("remover task panicked");

        let err = result.expect_err("a demoted actor must not remove another member");
        println!("stale-role removal rejected: {err}");

        // The victim must still be an active member.
        let members = get_members(&pool, community, channel_id)
            .await
            .expect("members");
        assert!(
            members.iter().any(|m| m.pubkey == victim),
            "victim must not have been removed by a demoted actor"
        );
    }

    /// A soft-removed row keeps its stored `role`, but that role is history,
    /// not live authority — `removed_at` says it is no longer in force. So
    /// reactivation must land at the baseline the caller was authorized for,
    /// never at the role the row happens to remember.
    ///
    /// Regression for the sharper vulnerability the alternative would create:
    /// an owner kicked by another owner self-rejoins through the kind:9021
    /// path (`Member`, no inviter) and must come back as a plain member. If
    /// `add_member` inferred authority from the removed row, soft-deleted
    /// ownership would be a resurrection token.
    ///
    /// Two owners on purpose, so the last-owner guard can never be what
    /// decides the outcome — only role resolution can.
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn kicked_owner_rejoins_as_member_not_owner() {
        let pool = setup_pool().await;
        let (community, channel_id, owner_a, owner_b) =
            channel_with_two_owners(&pool, "kicked-owner-rejoin").await;

        // owner_a kicks owner_b (allowed: owner_a remains as the last owner).
        remove_member(&pool, community, channel_id, &owner_b, &owner_a)
            .await
            .expect("an owner may remove another owner");

        let stored: String = sqlx::query_scalar(
            "SELECT role::text FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(&owner_b)
        .fetch_one(&pool)
        .await
        .expect("stored role survives soft removal");
        assert_eq!(
            stored, "owner",
            "the removed row still remembers `owner` — which is exactly why \
             authorization must not read it"
        );

        // The kind:9021 self-rejoin path: `Member`, no inviter.
        add_member(
            &pool,
            community,
            channel_id,
            &owner_b,
            MemberRole::Member,
            None,
        )
        .await
        .expect("a removed member may rejoin an open channel");

        let rejoined = get_member_role(&pool, community, channel_id, &owner_b)
            .await
            .expect("read role after rejoin");
        assert_eq!(
            rejoined.as_deref(),
            Some("member"),
            "a kicked owner must rejoin at baseline privilege, not regain ownership"
        );
    }

    /// A removed creator is not re-admitted to a private channel by being its
    /// creator. A kind:9021 self-join can pass the relay's open-channel check
    /// and reach `put_member` after the channel became private; it must be
    /// refused there, not turned into a creator bootstrap that restores
    /// ownership. The same holds for a self-add that names the creator as
    /// inviter and for one that asks for `owner`.
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn removed_creator_cannot_rejoin_a_private_channel() {
        let pool = setup_pool().await;
        let (community, channel_id, creator, other_owner) =
            channel_with_two_owners(&pool, "removed-creator-private").await;

        remove_member(&pool, community, channel_id, &creator, &other_owner)
            .await
            .expect("an owner may remove the creator");
        crate::channel::update_channel(
            &pool,
            community,
            channel_id,
            crate::channel::ChannelUpdate {
                visibility: Some("private".into()),
                ..Default::default()
            },
        )
        .await
        .expect("make the channel private");

        for (requested, invited_by) in [
            (None, None),
            (None, Some(creator.as_slice())),
            (Some(MemberRole::Owner), Some(creator.as_slice())),
        ] {
            let result = put_member(
                &pool, community, channel_id, &creator, requested, invited_by,
            )
            .await;
            assert!(
                matches!(result, Err(DbError::AccessDenied(_))),
                "a removed creator must not rejoin a private channel \
                 (requested {requested:?}, inviter set {}): {result:?}",
                invited_by.is_some()
            );
        }
        let role = get_member_role(&pool, community, channel_id, &creator)
            .await
            .expect("read creator role");
        assert_eq!(role, None, "the removed creator must stay removed");
    }

    /// The other side of the same boundary: reactivation may reach an elevated
    /// role, but only because a *currently* elevated granter asked for it.
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn removed_owner_is_restored_only_by_a_current_owner() {
        let pool = setup_pool().await;
        let (community, channel_id, owner_a, owner_b) =
            channel_with_two_owners(&pool, "removed-owner-restore").await;

        remove_member(&pool, community, channel_id, &owner_b, &owner_a)
            .await
            .expect("an owner may remove another owner");

        // An unprivileged member cannot re-add them at `owner`.
        let rando = random_pubkey();
        ensure_user(&pool, community, &rando)
            .await
            .expect("ensure rando");
        add_member(
            &pool,
            community,
            channel_id,
            &rando,
            MemberRole::Member,
            None,
        )
        .await
        .expect("rando self-joins open channel");
        let denied = add_member(
            &pool,
            community,
            channel_id,
            &owner_b,
            MemberRole::Owner,
            Some(&rando),
        )
        .await;
        assert!(
            matches!(denied, Err(DbError::AccessDenied(_))),
            "an unprivileged actor must not re-add anyone at `owner`, got {denied:?}"
        );

        // The remaining owner can.
        add_member(
            &pool,
            community,
            channel_id,
            &owner_b,
            MemberRole::Owner,
            Some(&owner_a),
        )
        .await
        .expect("a current owner may restore ownership");
        let restored = get_member_role(&pool, community, channel_id, &owner_b)
            .await
            .expect("read role after restore");
        assert_eq!(restored.as_deref(), Some("owner"));
    }

    async fn admin_url() -> String {
        crate::test_support::database_url()
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
            &[],
        )
        .await
        .expect("create channel");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unmigrated_roster_fence_blocks_startup_until_0032_is_applied() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (pool, scratch_name) =
            create_scratch_db_through(&admin, "roster_fence_unmigrated", Some(31)).await;
        let db = Db::from_pool(pool.clone());

        let error = db
            .verify_channel_roster_fence()
            .await
            .expect_err("pre-0032 schema must block roster publishers");
        assert!(
            error.to_string().contains("channel roster fence trigger"),
            "startup gate must report the missing schema fence: {error}"
        );
        let rows_before: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE kind = 39002")
            .fetch_one(&pool)
            .await
            .expect("count pre-migration rosters");
        assert_eq!(
            rows_before, 0,
            "failed startup gate must not publish a roster"
        );

        migration::run_migrations(&pool)
            .await
            .expect("apply migration 0032");
        db.verify_channel_roster_fence()
            .await
            .expect("0032 must open the startup gate");

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn channel_roster_fence_behavior_verification_detects_inert_function() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (pool, scratch_name) = create_scratch_db(&admin, "roster_fence_inert").await;
        let db = Db::from_pool(pool.clone());

        sqlx::raw_sql(
            "CREATE OR REPLACE FUNCTION guard_channel_roster_snapshot() \
             RETURNS TRIGGER AS $$ BEGIN RETURN NEW; END; $$ LANGUAGE plpgsql;",
        )
        .execute(&pool)
        .await
        .expect("replace roster fence with inert body");
        let error = db
            .verify_channel_roster_fence()
            .await
            .expect_err("inert roster fence must fail closed");
        assert!(
            error
                .to_string()
                .contains("stale probe roster was accepted"),
            "behavior probe must identify inert semantics: {error}"
        );

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn channel_roster_fence_catalog_verification_fails_closed() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (pool, scratch_name) = create_scratch_db(&admin, "roster_fence_catalog").await;
        let db = Db::from_pool(pool.clone());

        db.verify_channel_roster_fence()
            .await
            .expect("migrated roster fence must verify");

        let child: String = sqlx::query_scalar(
            "SELECT n.nspname || '.' || c.relname \
             FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE i.inhparent = 'public.events'::regclass ORDER BY i.inhrelid LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load event partition");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {child} DISABLE TRIGGER trg_events_guard_channel_roster_snapshot"
        )))
        .execute(&pool)
        .await
        .expect("disable partition roster trigger");
        let error = db
            .verify_channel_roster_fence()
            .await
            .expect_err("disabled partition roster fence must fail closed");
        assert!(
            error.to_string().contains(&child),
            "verification must identify the unfenced partition: {error}"
        );

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn channel_roster_fence_verification_supports_size_one_pool() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (seed_pool, scratch_name) = create_scratch_db(&admin, "roster_fence_size_one").await;
        seed_pool.close().await;

        let base_url = admin_url().await;
        let path = base_url.rfind('/').expect("database URL path");
        let scratch_url = format!("{}/{}", &base_url[..path], scratch_name);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect(&scratch_url)
            .await
            .expect("connect size-one writer pool");
        let db = Db::from_pool(pool.clone());

        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            db.verify_channel_roster_fence(),
        )
        .await
        .expect("roster verification must not self-deadlock on its second checkout")
        .expect("migrated roster fence verifies on a size-one pool");

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn desired_schema_rejects_stale_legacy_roster_role() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let scratch_name = format!("schema_roster_role_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {scratch_name}"
        )))
        .execute(&admin)
        .await
        .expect("create desired-schema scratch db");
        let base_url = admin_url().await;
        let slash = base_url.rfind('/').expect("database URL has path segment");
        let scratch_url = format!("{}/{}", &base_url[..slash], scratch_name);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&scratch_url)
            .await
            .expect("connect desired-schema scratch db");
        sqlx::raw_sql(sqlx::AssertSqlSafe(
            crate::test_support::desired_state_schema_sql(),
        ))
        .execute(&pool)
        .await
        .expect("apply desired-state schema");

        let db = Db::from_pool(pool.clone());
        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        let channel = Uuid::new_v4();
        let relay_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let owner = owner_keys.public_key().to_bytes();
        seed_community_channel(&pool, community_uuid, channel, &owner_keys).await;
        let member = Keys::generate().public_key().to_bytes();
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'admin', $4)",
        )
        .bind(community_uuid)
        .bind(channel)
        .bind(member.as_slice())
        .bind(owner.as_slice())
        .execute(&pool)
        .await
        .expect("seed canonical admin");

        let roster = |role: &str, timestamp| {
            EventBuilder::new(Kind::Custom(39002), "")
                .tags(vec![
                    Tag::parse(["d", channel.to_string().as_str()]).expect("d tag"),
                    Tag::parse(["p", hex::encode(owner).as_str(), "", "owner"])
                        .expect("owner p tag"),
                    Tag::parse(["p", hex::encode(member).as_str(), "", role])
                        .expect("member p tag"),
                ])
                .custom_created_at(Timestamp::from(timestamp))
                .sign_with_keys(&relay_keys)
                .expect("sign roster")
        };
        let base = Timestamp::now().as_secs();
        let fresh = roster("admin", base);
        assert!(
            db.replace_addressable_event(community, &fresh, Some(channel))
                .await
                .expect("publish canonical role")
                .1
        );
        let stale = roster("member", base + 1);
        // Reproduce the old binary on one connection: replacement key ->
        // old-head UPDATE -> raw INSERT. Do not use the app API, whose guard
        // rejects before the retained trigger can exercise this legacy fence.
        let error = async {
            let mut tx = pool.begin().await.expect("begin legacy replacement");
            let relay_pubkey = relay_keys.public_key().to_bytes();
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(crate::store::replaceable::event_replacement_lock_key(
                    community,
                    39002,
                    &relay_pubkey,
                    Some(channel.as_bytes().as_slice()),
                ))
                .execute(&mut *tx)
                .await
                .expect("legacy replacement key");
            let deleted = sqlx::query(
                "UPDATE events SET deleted_at = NOW() \
                 WHERE community_id=$1 AND kind=39002 AND pubkey=$2 \
                 AND channel_id=$3 AND deleted_at IS NULL",
            )
            .bind(community_uuid)
            .bind(relay_pubkey.as_slice())
            .bind(channel)
            .execute(&mut *tx)
            .await
            .expect("legacy soft-delete of current head");
            assert_eq!(deleted.rows_affected(), 1, "legacy path must delete the live head");
            let result = sqlx::query(
                "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
                 VALUES ($1, $2, $3, $4, 39002, $5, $6, $7, NOW(), $8, $9)",
            )
            .bind(community_uuid)
            .bind(stale.id.as_bytes().as_slice())
            .bind(relay_pubkey.as_slice())
            .bind(chrono::DateTime::from_timestamp(stale.created_at.as_secs() as i64, 0).expect("timestamp"))
            .bind(serde_json::to_value(&stale.tags).expect("tags"))
            .bind(&stale.content)
            .bind(stale.sig.serialize().as_slice())
            .bind(channel)
            .bind(channel.to_string())
            .execute(&mut *tx)
            .await;
            tx.rollback().await.expect("restore live head after legacy attempt");
            result
        }
        .await
        .expect_err("desired-state fence must reject stale role");
        assert!(
            matches!(error, sqlx::Error::Database(ref db_error)
                if db_error.code().as_deref() == Some("23514")
                && db_error.message() == "kind 39002 roster does not match canonical channel membership"),
            "expected trigger roster fence check violation, got {error:?}"
        );

        let live_id: Vec<u8> = sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id=$1 AND channel_id=$2 \
             AND kind=39002 AND deleted_at IS NULL",
        )
        .bind(community_uuid)
        .bind(channel)
        .fetch_one(&pool)
        .await
        .expect("load desired-state live roster");
        assert_eq!(live_id, fresh.id.as_bytes().to_vec());
        let stale_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
                .bind(community_uuid)
                .bind(stale.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count rejected stale roster");
        assert_eq!(stale_rows, 0, "legacy stale insert must roll back");

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }
}
