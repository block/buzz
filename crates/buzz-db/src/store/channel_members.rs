//! Channel membership and roster persistence.
//!
//! Membership mutations share one advisory-lock namespace. Relay-authored
//! roster snapshots hold that same lock through replacement publication.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::channel::{row_to_channel_record, ChannelRecord};
use crate::error::{DbError, Result};
use crate::Db;
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
async fn acquire_channel_membership_lock(
    tx: &mut Transaction<'_, Postgres>,
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
            .execute(&mut **tx),
    )
    .await?;
    Ok(())
}

/// An active member roster captured while holding the channel's membership
/// serialization lock on one writer connection.
pub struct LockedMemberSnapshot {
    /// Canonical active members captured behind the lock.
    pub members: Vec<MemberRecord>,
    community_id: CommunityId,
    channel_id: Uuid,
    relay_pubkey: Vec<u8>,
    tx: Transaction<'static, Postgres>,
}

impl LockedMemberSnapshot {
    /// Return the newest relay-authored member snapshot timestamp using this
    /// guard's existing connection.
    pub async fn latest_member_event_timestamp(
        &mut self,
        community_id: CommunityId,
        channel_id: Uuid,
        relay_pubkey: &[u8],
    ) -> Result<Option<u64>> {
        let value: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM events WHERE community_id = $1 AND kind = 39002 AND pubkey = $2 AND channel_id = $3 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(relay_pubkey)
        .bind(channel_id)
        .fetch_optional(&mut *self.tx)
        .await?;
        Ok(value.map(|timestamp| timestamp.timestamp() as u64))
    }

    /// Replace the relay-authored member snapshot on this guard's existing
    /// connection. The membership lock therefore spans capture and replacement
    /// without a nested pool checkout.
    pub async fn replace_member_event(
        &mut self,
        community_id: CommunityId,
        channel_id: Uuid,
        event: &nostr::Event,
    ) -> Result<(buzz_core::StoredEvent, bool)> {
        if community_id != self.community_id
            || channel_id != self.channel_id
            || event.pubkey.to_bytes().as_slice() != self.relay_pubkey.as_slice()
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
        .fetch_optional(&mut *self.tx)
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
        sqlx::query("UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND channel_id = $4 AND deleted_at IS NULL")
            .bind(community_id.as_uuid()).bind(kind).bind(pubkey.as_slice()).bind(channel_id)
            .execute(&mut *self.tx).await?;
        let received_at = Utc::now();
        let tags = serde_json::to_value(&event.tags)?;
        let sig = event.sig.serialize();
        let inserted = sqlx::query("INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT DO NOTHING")
            .bind(community_id.as_uuid()).bind(event.id.as_bytes().as_slice())
            .bind(pubkey.as_slice()).bind(created_at).bind(kind).bind(tags)
            .bind(&event.content).bind(sig.as_slice()).bind(received_at).bind(channel_id)
            .bind(crate::event::extract_d_tag(event)).execute(&mut *self.tx).await?;
        if inserted.rows_affected() == 0 {
            return Err(DbError::InvalidData(
                "member snapshot event id already exists".into(),
            ));
        }
        crate::insert_mentions_in_transaction(&mut self.tx, community_id, event, Some(channel_id))
            .await?;
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
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;
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
            .execute(&mut *tx),
    )
    .await?;
    acquire_channel_membership_lock(&mut tx, community_id, channel_id).await?;
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
    .fetch_all(&mut *tx)
    .await?;
    let members = rows
        .into_iter()
        .map(row_to_member_record)
        .collect::<Result<Vec<_>>>()?;
    Ok(LockedMemberSnapshot {
        members,
        community_id,
        channel_id,
        relay_pubkey: relay_pubkey.to_vec(),
        tx,
    })
}

/// Add a member to a channel.
///
/// Role enforcement:
/// - Open channels: `invited_by` is optional; role is forced to `Member` regardless of
///   what the caller passes — callers cannot self-assign elevated roles.
/// - Private channels: requires an `invited_by` who is an active member, or the channel
///   creator bootstrapping their own first membership. Any active member may add an
///   ordinary member, guest, or bot; only owners/admins may grant elevated roles.
/// - Elevated roles (`Owner`, `Admin`) may only be granted by an existing owner/admin,
///   even on open channels.
///
/// The entire check-then-insert sequence runs inside a transaction to prevent TOCTOU
/// races (e.g. the inviter being removed between the role check and the INSERT).
pub async fn add_member(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
    role: MemberRole,
    invited_by: Option<&[u8]>,
) -> Result<MemberRecord> {
    if pubkey.len() != 32 {
        return Err(DbError::InvalidData(format!(
            "pubkey must be 32 bytes, got {}",
            pubkey.len()
        )));
    }

    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    // First statement: serialize the whole role-check / owner-count / upsert
    // sequence against concurrent membership writes on this channel.
    acquire_channel_membership_lock(&mut tx, community_id, channel_id).await?;

    let channel = get_channel_tx(&mut tx, community_id, channel_id).await?;

    let effective_role = if channel.visibility == "private" {
        let inviter = invited_by.ok_or_else(|| {
            DbError::AccessDenied("private channel requires an invite".to_string())
        })?;

        // Bootstrap: channel creator may add themselves as the first member.
        let is_creator_bootstrap = inviter == pubkey && inviter == channel.created_by.as_slice();

        if !is_creator_bootstrap {
            let inviter_role_str = get_active_role_tx(&mut tx, community_id, channel_id, inviter)
                .await?
                .ok_or_else(|| {
                    DbError::AccessDenied("inviter is not an active member".to_string())
                })?;

            let inviter_role: MemberRole = inviter_role_str.parse().map_err(|_| {
                DbError::InvalidData(format!("invalid role in database: {inviter_role_str}"))
            })?;

            // Any active member may extend private-channel access with an
            // ordinary role. Granting owner/admin remains reserved for an
            // existing owner/admin.
            if role.is_elevated() && !inviter_role.is_elevated() {
                return Err(DbError::AccessDenied(
                    "only owners/admins may grant elevated roles".to_string(),
                ));
            }
        }

        role
    } else {
        // Open channel: anyone may join, but only existing owners/admins may grant
        // elevated roles. Self-join always gets Member.
        if role.is_elevated() {
            let granter_role = match invited_by {
                Some(inv) => get_active_role_tx(&mut tx, community_id, channel_id, inv).await?,
                None => None,
            };
            match granter_role.as_deref() {
                Some("owner") | Some("admin") => role,
                _ => {
                    return Err(DbError::AccessDenied(
                        "only owners/admins may grant elevated roles".to_string(),
                    ))
                }
            }
        } else {
            role
        }
    };

    // Changing an *active* member's role is privileged in BOTH directions.
    // Demotion is as consequential as promotion: only owners/admins may grant
    // elevated roles, so a demoted owner cannot restore themselves. Guarding
    // only `role.is_elevated()` above therefore left owner→member demotion
    // unauthorized-by-anyone. Re-adding an active member with the role they
    // already hold stays idempotent and unguarded — the huddle bot-add and
    // kind:9021 join paths rely on that.
    //
    // Deliberately keyed on the *active* role. A soft-removed row's stored role
    // is history, not live authority: `removed_at` says it is no longer in
    // force. Reactivation therefore lands at whatever `effective_role` the
    // checks above already authorized — `Member` for any unprivileged caller,
    // elevated only when a currently-elevated granter asked for it. Inferring
    // current authority from a removed row would make soft-deleted ownership a
    // resurrection token: an owner removed by another owner could self-rejoin
    // via kind:9021 (`Member, None`) and silently regain ownership.
    let current_role = get_active_role_tx(&mut tx, community_id, channel_id, pubkey).await?;
    if let Some(current_role) = current_role.filter(|r| r != effective_role.as_str()) {
        let actor_role = match invited_by {
            Some(inviter) => get_active_role_tx(&mut tx, community_id, channel_id, inviter).await?,
            None => None,
        };
        let actor_role: Option<MemberRole> = actor_role.and_then(|r| r.parse().ok());
        if !actor_role.is_some_and(|r| r.is_elevated()) {
            return Err(DbError::AccessDenied(
                "only owners/admins may change an active member's role".to_string(),
            ));
        }

        // Defense-in-depth, mirroring `remove_member`: a demotion must not
        // strip the channel of its last owner, which would leave nobody able
        // to moderate, edit metadata, or re-grant ownership.
        if current_role == "owner" && effective_role != MemberRole::Owner {
            let row = sqlx::query(
                "SELECT COUNT(*) as cnt FROM channel_members \
                 WHERE community_id = $1 AND channel_id = $2 AND role = 'owner' AND removed_at IS NULL",
            )
            .bind(community_id.as_uuid())
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await?;
            let owner_count: i64 = row.try_get("cnt")?;
            if owner_count <= 1 {
                return Err(DbError::AccessDenied(
                    "cannot demote the last owner — transfer ownership first".to_string(),
                ));
            }
        }
    }

    sqlx::query(
        r#"
        INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
        VALUES ($1, $2, $3, $4::member_role, $5)
        ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
            removed_at = NULL,
            removed_by = NULL,
            role = EXCLUDED.role
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .bind(effective_role.as_str())
    .bind(invited_by)
    .execute(&mut *tx)
    .await?;

    let row = sqlx::query(
        r#"
        SELECT channel_id, pubkey, role::text AS role, joined_at, invited_by, removed_at
        FROM channel_members WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_one(&mut *tx)
    .await?;

    let record = row_to_member_record(row)?;
    tx.commit().await?;
    Ok(record)
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

    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    // First statement: serialize the actor-role check, the last-owner count and
    // the UPDATE against concurrent membership writes on this channel (same key
    // as `add_member`).
    acquire_channel_membership_lock(&mut tx, community_id, channel_id).await?;

    if !is_self_remove {
        let actor_role_str = get_active_role_tx(&mut tx, community_id, channel_id, actor_pubkey)
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
    let target_role = get_active_role_tx(&mut tx, community_id, channel_id, pubkey).await?;
    if target_role.as_deref() == Some("owner") {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND role = 'owner' AND removed_at IS NULL",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_one(&mut *tx)
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
    .execute(&mut *tx)
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
/// Includes channels where the pubkey is an active member AND all open channels.
/// Open channels must be included in REQ filter resolution.
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
    let rows = sqlx::query(
        r#"
        SELECT cm.channel_id
        FROM channel_members cm
        JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL
        WHERE cm.community_id = $1 AND cm.pubkey = $2 AND cm.removed_at IS NULL
        UNION
        SELECT id AS channel_id
        FROM channels
        WHERE community_id = $1 AND visibility = 'open' AND deleted_at IS NULL
        "#,
    )
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
        JOIN communities community ON community.id = lr.community_id
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
    tx: &mut Transaction<'_, Postgres>,
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
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|r| r.try_get("role")).transpose()?)
}

/// Transaction-aware variant of [`get_channel`].
async fn get_channel_tx(
    tx: &mut Transaction<'_, Postgres>,
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
               ttl_seconds, ttl_deadline
        FROM channels WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut **tx)
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
/// open channels (visible to everyone) plus channels where the user is an active member.
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
        "AND (c.visibility = 'open' OR cm.channel_id IS NOT NULL)"
    };

    let base = format!(
        r#"
        SELECT c.id, c.name, c.channel_type::text AS channel_type,
               c.visibility::text AS visibility, c.description, c.canvas,
               c.created_by, c.created_at, c.updated_at, c.archived_at, c.deleted_at,
               c.nip29_group_id, c.topic_required, c.max_members,
               c.topic, c.topic_set_by, c.topic_set_at,
               c.purpose, c.purpose_set_by, c.purpose_set_at,
               c.ttl_seconds, c.ttl_deadline,
               (cm.channel_id IS NOT NULL) AS is_member
        FROM channels c
        LEFT JOIN channel_members cm
            ON c.community_id = cm.community_id AND c.id = cm.channel_id AND cm.pubkey = $2 AND cm.removed_at IS NULL
        WHERE c.community_id = $1 AND c.deleted_at IS NULL
          {membership_clause}
          AND (c.channel_type != 'dm' OR cm.hidden_at IS NULL)
    "#
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
        let (_, inserted) = snapshot
            .replace_member_event(community, channel.id, &event)
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
        sqlx::raw_sql(include_str!("../../../../schema/schema.sql"))
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
        let error = db
            .replace_addressable_event(community, &stale, Some(channel))
            .await
            .expect_err("desired-state fence must reject stale role");
        assert!(matches!(
            error,
            DbError::Sqlx(sqlx::Error::Database(ref db_error))
                if db_error.code().as_deref() == Some("23514")
        ));
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

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }
}
