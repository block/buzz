//! Atomic, audited orphaned-channel ownership recovery.
//!
//! Recovery is intentionally separate from generic role mutation. The only
//! supported predicate is durable prior self-consent from every current human
//! owner naming the nominated replacement.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::Event;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::{DbError, Result};

/// Stable identifier recorded in every audit event.
pub const RECOVERY_PREDICATE_ID: &str = "all_current_human_owners_self_archived_for_target_v1";
/// Stable machine-readable reason recorded for this dedicated recovery path.
pub const RECOVERY_REASON_CODE: &str = "orphaned_owner_prior_self_consent";

/// An elevated membership captured before promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorElevatedRole {
    /// Lowercase hex public key.
    pub pubkey: String,
    /// Existing channel role (`owner` or `admin`).
    pub role: String,
}

/// Durable payload used to construct the relay-signed channel audit event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryAuditPayload {
    /// Payload schema version.
    pub schema_version: u8,
    /// System-event type rendered by channel clients.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Community selected from the relay tenant context.
    pub community_id: Uuid,
    /// Channel whose ownership was recovered.
    pub channel_id: Uuid,
    /// Original signed recovery request ID.
    pub request_event_id: String,
    /// Community owner who requested recovery.
    pub actor: String,
    /// Existing member promoted to channel owner.
    pub target: String,
    /// Eligibility predicate applied under lock.
    pub predicate_id: String,
    /// Stable machine-readable recovery reason.
    pub reason_code: String,
    /// Human-supplied reason.
    pub reason: String,
    /// Elevated membership set immediately before promotion.
    pub prior_elevated_roles: Vec<PriorElevatedRole>,
    /// Transaction timestamp.
    pub created_at: DateTime<Utc>,
}

/// Result of an applied or replayed recovery request.
#[derive(Debug, Clone)]
pub struct RecoveryRecord {
    /// Whether this request performed the promotion.
    pub applied: bool,
    /// Durable audit payload.
    pub payload: RecoveryAuditPayload,
    /// Whether the channel audit event has been delivered.
    pub delivered: bool,
}

/// Pending relay delivery loaded from the durable recovery outbox.
#[derive(Debug, Clone)]
pub struct PendingRecoveryDelivery {
    /// Server-resolved community that owns the recovery.
    pub community_id: CommunityId,
    /// Community host used to construct a tenant context for fan-out.
    pub host: String,
    /// Original signed request ID.
    pub request_event_id: Vec<u8>,
    /// Immutable audit payload to publish.
    pub payload: RecoveryAuditPayload,
}

fn validate_reason(reason: &str) -> Result<&str> {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return Err(DbError::InvalidData(
            "recovery reason must not be empty".into(),
        ));
    }
    if trimmed.len() > 500 {
        return Err(DbError::InvalidData(
            "recovery reason must not exceed 500 bytes".into(),
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(DbError::InvalidData(
            "recovery reason must not contain control characters".into(),
        ));
    }
    Ok(trimmed)
}

fn payload_from_row(row: &sqlx::postgres::PgRow) -> Result<RecoveryAuditPayload> {
    let community_id: Uuid = row.try_get("community_id")?;
    let request_event_id: Vec<u8> = row.try_get("request_event_id")?;
    let actor_pubkey: Vec<u8> = row.try_get("actor_pubkey")?;
    let target_pubkey: Vec<u8> = row.try_get("target_pubkey")?;
    let prior_elevated_roles: serde_json::Value = row.try_get("prior_elevated_roles")?;
    Ok(RecoveryAuditPayload {
        schema_version: 1,
        event_type: "channel_owner_recovered".into(),
        community_id,
        channel_id: row.try_get("channel_id")?,
        request_event_id: hex::encode(request_event_id),
        actor: hex::encode(actor_pubkey),
        target: hex::encode(target_pubkey),
        predicate_id: row.try_get("predicate_id")?,
        reason_code: row.try_get("reason_code")?,
        reason: row.try_get("reason")?,
        prior_elevated_roles: serde_json::from_value(prior_elevated_roles)?,
        created_at: row.try_get("created_at")?,
    })
}

/// Apply or idempotently replay a protected channel-owner recovery request.
pub async fn recover_channel_owner(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    target_pubkey: &[u8],
    reason: &str,
    request: &Event,
) -> Result<RecoveryRecord> {
    if target_pubkey.len() != 32 {
        return Err(DbError::InvalidData(
            "target pubkey must be 32 bytes".into(),
        ));
    }
    let reason = validate_reason(reason)?;
    let actor_pubkey = request.pubkey.to_bytes();
    let actor_hex = request.pubkey.to_hex();
    let target_hex = hex::encode(target_pubkey);
    let request_id = request.id.as_bytes();
    let request_id_hex = request.id.to_hex();

    let mut tx = pool.begin().await?;
    crate::channel::lock_channel_membership(&mut tx, community_id, channel_id).await?;

    if let Some(row) = sqlx::query(
        "SELECT a.community_id, a.request_event_id, a.channel_id, \
                a.actor_pubkey, a.target_pubkey, a.predicate_id, \
                a.reason_code, a.reason, a.prior_elevated_roles, a.created_at, \
                o.delivered_at \
         FROM channel_owner_recovery_audit a \
         JOIN channel_owner_recovery_outbox o \
           ON o.community_id = a.community_id \
          AND o.request_event_id = a.request_event_id \
         WHERE a.community_id = $1 AND a.request_event_id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(request_id.as_slice())
    .fetch_optional(&mut *tx)
    .await?
    {
        let payload = payload_from_row(&row)?;
        if payload.community_id != *community_id.as_uuid()
            || payload.channel_id != channel_id
            || payload.request_event_id != request_id_hex
            || payload.actor != actor_hex
            || payload.target != target_hex
            || payload.reason != reason
        {
            return Err(DbError::InvalidData(
                "recovery replay does not match the committed audit".into(),
            ));
        }
        let delivered_at: Option<DateTime<Utc>> = row.try_get("delivered_at")?;
        tx.commit().await?;
        return Ok(RecoveryRecord {
            applied: false,
            payload,
            delivered: delivered_at.is_some(),
        });
    }

    // Freshness applies only to a new state transition. An exact cryptographic
    // replay of an already-committed request remains eligible to drain the
    // durable outbox and refresh discovery after the original 120-second
    // window has elapsed.
    let request_ts = request.created_at.as_secs() as i64;
    let now = Utc::now().timestamp();
    if (request_ts - now).abs() > 120 {
        return Err(DbError::AccessDenied(format!(
            "recovery request timestamp out of range (delta={}s, max ±120s)",
            request_ts - now
        )));
    }

    let channel = sqlx::query(
        "SELECT channel_type::text AS channel_type, archived_at, deleted_at \
         FROM channels WHERE community_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::ChannelNotFound(channel_id))?;
    let channel_type: String = channel.try_get("channel_type")?;
    let archived_at: Option<DateTime<Utc>> = channel.try_get("archived_at")?;
    let deleted_at: Option<DateTime<Utc>> = channel.try_get("deleted_at")?;
    if archived_at.is_some() || deleted_at.is_some() {
        return Err(DbError::AccessDenied(
            "channel must be active for owner recovery".into(),
        ));
    }
    if channel_type == "dm" {
        return Err(DbError::AccessDenied(
            "direct-message channels do not support owner recovery".into(),
        ));
    }

    let actor_role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM relay_members \
         WHERE community_id = $1 AND pubkey = $2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(&actor_hex)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DbError::AccessDenied("actor is not a community member".into()))?;
    if actor_role != "owner" {
        return Err(DbError::AccessDenied(
            "recovery requires an active, known human community owner".into(),
        ));
    }

    let target_role = sqlx::query_scalar::<_, String>(
        "SELECT role::text FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 \
           AND pubkey = $3 AND removed_at IS NULL FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(target_pubkey)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::MemberNotFound(channel_id))?;
    if !matches!(target_role.as_str(), "member" | "guest") {
        return Err(DbError::AccessDenied(
            "target must be an active, known human, non-elevated channel member".into(),
        ));
    }

    let elevated_rows = sqlx::query(
        "SELECT pubkey, role::text AS role FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 \
           AND removed_at IS NULL AND role IN ('owner', 'admin') \
         ORDER BY pubkey FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_all(&mut *tx)
    .await?;
    if elevated_rows.is_empty() {
        return Err(DbError::AccessDenied(
            "channel has no current owner record to authorize recovery".into(),
        ));
    }

    let mut identities = vec![actor_hex.clone(), target_hex.clone()];
    for row in &elevated_rows {
        let pubkey: Vec<u8> = row.try_get("pubkey")?;
        identities.push(hex::encode(pubkey));
    }
    identities.sort();
    identities.dedup();
    for identity in &identities {
        crate::archived_identities::lock_identity(&mut tx, community_id, identity).await?;
    }

    let actor = sqlx::query(
        "SELECT agent_owner_pubkey, deactivated_at FROM users \
         WHERE community_id = $1 AND pubkey = $2 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(actor_pubkey.as_slice())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        DbError::AccessDenied("recovery requires an active, known human community owner".into())
    })?;
    let actor_agent_owner: Option<Vec<u8>> = actor.try_get("agent_owner_pubkey")?;
    let actor_deactivated: Option<DateTime<Utc>> = actor.try_get("deactivated_at")?;
    let actor_archived = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM archived_identities \
         WHERE community_id = $1 AND pubkey = $2)",
    )
    .bind(community_id.as_uuid())
    .bind(&actor_hex)
    .fetch_one(&mut *tx)
    .await?;
    if actor_agent_owner.is_some() || actor_deactivated.is_some() || actor_archived {
        return Err(DbError::AccessDenied(
            "recovery requires an active, known human community owner".into(),
        ));
    }

    let target = sqlx::query(
        "SELECT agent_owner_pubkey, deactivated_at FROM users \
         WHERE community_id = $1 AND pubkey = $2 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(target_pubkey)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        DbError::AccessDenied(
            "target must be an active, known human, non-elevated channel member".into(),
        )
    })?;
    let target_agent_owner: Option<Vec<u8>> = target.try_get("agent_owner_pubkey")?;
    let target_deactivated: Option<DateTime<Utc>> = target.try_get("deactivated_at")?;
    if target_agent_owner.is_some() || target_deactivated.is_some() {
        return Err(DbError::AccessDenied(
            "target must be an active, known human, non-elevated channel member".into(),
        ));
    }

    let target_archived = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM archived_identities \
         WHERE community_id = $1 AND pubkey = $2)",
    )
    .bind(community_id.as_uuid())
    .bind(&target_hex)
    .fetch_one(&mut *tx)
    .await?;
    if target_archived {
        return Err(DbError::AccessDenied("target identity is archived".into()));
    }

    let mut prior_elevated_roles = Vec::with_capacity(elevated_rows.len());
    for row in elevated_rows {
        let pubkey: Vec<u8> = row.try_get("pubkey")?;
        let pubkey_hex = hex::encode(&pubkey);
        let role: String = row.try_get("role")?;
        let user = sqlx::query(
            "SELECT agent_owner_pubkey, deactivated_at FROM users \
             WHERE community_id = $1 AND pubkey = $2 FOR SHARE",
        )
        .bind(community_id.as_uuid())
        .bind(&pubkey)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            DbError::AccessDenied("owner recovery rejects unknown elevated identities".into())
        })?;
        let agent_owner: Option<Vec<u8>> = user.try_get("agent_owner_pubkey")?;
        let deactivated_at: Option<DateTime<Utc>> = user.try_get("deactivated_at")?;
        if agent_owner.is_some() {
            return Err(DbError::AccessDenied(
                "owner recovery is disabled while an owner/admin agent exists".into(),
            ));
        }
        if deactivated_at.is_some() {
            return Err(DbError::AccessDenied(
                "owner recovery rejects unknown or deactivated elevated identities".into(),
            ));
        }
        if role == "admin" {
            // Identity archival is a UI hint and does not revoke channel
            // authority. Any active admin membership can still use the normal
            // role path, regardless of archive state.
            return Err(DbError::AccessDenied(
                "use the ordinary role path while an active human channel admin exists".into(),
            ));
        } else {
            let eligible = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM archived_identities \
                 WHERE community_id = $1 AND pubkey = $2 \
                   AND consent_path = 'self' AND actor = $2 AND replaced_by = $3)",
            )
            .bind(community_id.as_uuid())
            .bind(&pubkey_hex)
            .bind(&target_hex)
            .fetch_one(&mut *tx)
            .await?;
            if !eligible {
                return Err(DbError::AccessDenied(format!(
                    "owner {pubkey_hex} lacks prior durable self-consent naming the target"
                )));
            }
        }
        prior_elevated_roles.push(PriorElevatedRole {
            pubkey: pubkey_hex,
            role,
        });
    }

    let promoted = sqlx::query(
        "UPDATE channel_members SET role = 'owner' \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
           AND removed_at IS NULL AND role IN ('member', 'guest')",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(target_pubkey)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if promoted != 1 {
        return Err(DbError::InvalidData(
            "target membership changed before owner promotion".into(),
        ));
    }

    let created_at_secs = request.created_at.as_secs() as i64;
    let request_created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
    let tags = serde_json::to_value(&request.tags)?;
    let inserted = sqlx::query(
        "INSERT INTO events \
         (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,NOW(),$9) ON CONFLICT DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(request_id.as_slice())
    .bind(actor_pubkey.as_slice())
    .bind(request_created_at)
    .bind(i32::from(request.kind.as_u16()))
    .bind(tags)
    .bind(&request.content)
    .bind(request.sig.serialize().as_slice())
    .bind(channel_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted != 1 {
        return Err(DbError::InvalidData(
            "request event already exists without a matching recovery audit".into(),
        ));
    }

    let created_at = Utc::now();
    let payload = RecoveryAuditPayload {
        schema_version: 1,
        event_type: "channel_owner_recovered".into(),
        community_id: *community_id.as_uuid(),
        channel_id,
        request_event_id: request_id_hex,
        actor: actor_hex,
        target: target_hex,
        predicate_id: RECOVERY_PREDICATE_ID.into(),
        reason_code: RECOVERY_REASON_CODE.into(),
        reason: reason.to_string(),
        prior_elevated_roles,
        created_at,
    };
    let prior_roles_json = serde_json::to_value(&payload.prior_elevated_roles)?;
    sqlx::query(
        "INSERT INTO channel_owner_recovery_audit \
         (community_id,request_event_id,channel_id,actor_pubkey,target_pubkey,predicate_id,reason_code,reason,prior_elevated_roles,created_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(community_id.as_uuid())
    .bind(request_id.as_slice())
    .bind(channel_id)
    .bind(actor_pubkey.as_slice())
    .bind(target_pubkey)
    .bind(RECOVERY_PREDICATE_ID)
    .bind(RECOVERY_REASON_CODE)
    .bind(reason)
    .bind(prior_roles_json)
    .bind(created_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO channel_owner_recovery_outbox \
         (community_id,request_event_id,channel_id) VALUES ($1,$2,$3)",
    )
    .bind(community_id.as_uuid())
    .bind(request_id.as_slice())
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(RecoveryRecord {
        applied: true,
        payload,
        delivered: false,
    })
}

/// Mark the channel audit event as durably delivered.
pub async fn mark_recovery_delivered(
    pool: &PgPool,
    community_id: CommunityId,
    request_event_id: &[u8],
) -> Result<()> {
    sqlx::query(
        "UPDATE channel_owner_recovery_outbox SET delivered_at = NOW(), \
         attempts = attempts + 1, last_error = NULL \
         WHERE community_id = $1 AND request_event_id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(request_event_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a retryable channel audit delivery failure.
pub async fn record_recovery_delivery_failure(
    pool: &PgPool,
    community_id: CommunityId,
    request_event_id: &[u8],
    error: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE channel_owner_recovery_outbox SET attempts = attempts + 1, last_error = $3 \
         WHERE community_id = $1 AND request_event_id = $2 AND delivered_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(request_event_id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// Load a bounded batch of undelivered channel recovery audits.
pub async fn pending_recovery_deliveries(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<PendingRecoveryDelivery>> {
    let rows = sqlx::query(
        "SELECT a.community_id, c.host, a.request_event_id, a.channel_id, \
                a.actor_pubkey, a.target_pubkey, a.predicate_id, \
                a.reason_code, a.reason, a.prior_elevated_roles, a.created_at \
         FROM channel_owner_recovery_outbox o \
         JOIN channel_owner_recovery_audit a \
           ON a.community_id = o.community_id \
          AND a.request_event_id = o.request_event_id \
         JOIN communities c ON c.id = o.community_id \
         WHERE o.delivered_at IS NULL \
         ORDER BY o.created_at, o.community_id, o.request_event_id \
         LIMIT $1",
    )
    .bind(limit.clamp(1, 1_000))
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let community_id: Uuid = row.try_get("community_id")?;
            Ok(PendingRecoveryDelivery {
                community_id: CommunityId::from_uuid(community_id),
                host: row.try_get("host")?,
                request_event_id: row.try_get("request_event_id")?,
                payload: payload_from_row(&row)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};
    use buzz_core::kind::KIND_CHANNEL_OWNER_RECOVERY;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    struct Fixture {
        community: CommunityId,
        channel: Uuid,
        actor: Keys,
        owner: Keys,
        target: Keys,
    }

    struct ContinuityRecords {
        root_event_id: Vec<u8>,
        reply_event_id: Vec<u8>,
        workflow_id: Uuid,
        agent: Keys,
    }

    async fn setup_pool() -> PgPool {
        PgPool::connect(TEST_DB_URL)
            .await
            .expect("connect to test DB")
    }

    async fn insert_human(pool: &PgPool, community: CommunityId, keys: &Keys) {
        sqlx::query("INSERT INTO users (community_id,pubkey) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(keys.public_key().to_bytes().as_slice())
            .execute(pool)
            .await
            .expect("insert human");
    }

    async fn setup_fixture(pool: &PgPool) -> Fixture {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(id)
            .bind(format!("owner-recovery-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        let community = CommunityId::from_uuid(id);
        let actor = Keys::generate();
        let owner = Keys::generate();
        let target = Keys::generate();
        for keys in [&actor, &owner, &target] {
            insert_human(pool, community, keys).await;
        }
        sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'owner')")
            .bind(community.as_uuid())
            .bind(actor.public_key().to_hex())
            .execute(pool)
            .await
            .expect("insert community owner");

        let channel = crate::channel::create_channel(
            pool,
            community,
            "orphaned",
            ChannelType::Stream,
            ChannelVisibility::Open,
            Some("continuity fixture"),
            owner.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create channel")
        .id;
        crate::channel::add_member(
            pool,
            community,
            channel,
            target.public_key().to_bytes().as_slice(),
            MemberRole::Member,
            Some(owner.public_key().to_bytes().as_slice()),
        )
        .await
        .expect("add target");

        Fixture {
            community,
            channel,
            actor,
            owner,
            target,
        }
    }

    fn request(fixture: &Fixture, reason: &str) -> Event {
        request_as(&fixture.actor, fixture, reason)
    }

    fn request_as(actor: &Keys, fixture: &Fixture, reason: &str) -> Event {
        request_as_at(actor, fixture, reason, nostr::Timestamp::now())
    }

    fn request_as_at(
        actor: &Keys,
        fixture: &Fixture,
        reason: &str,
        created_at: nostr::Timestamp,
    ) -> Event {
        EventBuilder::new(Kind::Custom(KIND_CHANNEL_OWNER_RECOVERY as u16), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["h", &fixture.channel.to_string()]).unwrap(),
                Tag::parse(["p", &fixture.target.public_key().to_hex()]).unwrap(),
                Tag::parse(["reason", reason]).unwrap(),
            ])
            .custom_created_at(created_at)
            .sign_with_keys(actor)
            .expect("sign request")
    }

    async fn record_owner_consent(pool: &PgPool, fixture: &Fixture) {
        crate::archived_identities::archive(
            pool,
            fixture.community,
            &fixture.owner.public_key().to_hex(),
            "self",
            &fixture.owner.public_key().to_hex(),
            Some("retired"),
            Some(&fixture.target.public_key().to_hex()),
            &"aa".repeat(32),
        )
        .await
        .expect("archive owner");
    }

    async fn seed_continuity_records(pool: &PgPool, fixture: &Fixture) -> ContinuityRecords {
        crate::channel::set_canvas(
            pool,
            fixture.community,
            fixture.channel,
            Some("# Recovery continuity\n\nKeep this canvas."),
        )
        .await
        .expect("seed canvas");

        let channel = fixture.channel.to_string();
        let root = EventBuilder::new(Kind::Custom(9), "continuity root")
            .tags([Tag::parse(["h", &channel]).unwrap()])
            .sign_with_keys(&fixture.owner)
            .expect("sign root");
        let reply = EventBuilder::new(Kind::Custom(9), "continuity reply")
            .tags([
                Tag::parse(["h", &channel]).unwrap(),
                Tag::parse(["e", &root.id.to_hex(), "", "root"]).unwrap(),
                Tag::parse(["e", &root.id.to_hex(), "", "reply"]).unwrap(),
            ])
            .custom_created_at(nostr::Timestamp::from(root.created_at.as_secs() + 1))
            .sign_with_keys(&fixture.target)
            .expect("sign reply");
        for event in [&root, &reply] {
            crate::event::insert_event(pool, fixture.community, event, Some(fixture.channel))
                .await
                .expect("seed message");
        }

        let root_created_at =
            DateTime::from_timestamp(root.created_at.as_secs() as i64, 0).expect("root timestamp");
        let reply_created_at = DateTime::from_timestamp(reply.created_at.as_secs() as i64, 0)
            .expect("reply timestamp");
        sqlx::query(
            "INSERT INTO thread_metadata \
             (community_id,event_created_at,event_id,channel_id,depth,reply_count,descendant_count,last_reply_at) \
             VALUES ($1,$2,$3,$4,0,1,1,$5)",
        )
        .bind(fixture.community.as_uuid())
        .bind(root_created_at)
        .bind(root.id.as_bytes().as_slice())
        .bind(fixture.channel)
        .bind(reply_created_at)
        .execute(pool)
        .await
        .expect("seed root metadata");
        sqlx::query(
            "INSERT INTO thread_metadata \
             (community_id,event_created_at,event_id,channel_id,parent_event_id,parent_event_created_at,root_event_id,root_event_created_at,depth) \
             VALUES ($1,$2,$3,$4,$5,$6,$5,$6,1)",
        )
        .bind(fixture.community.as_uuid())
        .bind(reply_created_at)
        .bind(reply.id.as_bytes().as_slice())
        .bind(fixture.channel)
        .bind(root.id.as_bytes().as_slice())
        .bind(root_created_at)
        .execute(pool)
        .await
        .expect("seed reply metadata");

        let workflow_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO workflows \
             (community_id,id,name,owner_pubkey,channel_id,definition,definition_hash) \
             VALUES ($1,$2,'continuity workflow',$3,$4,$5,$6)",
        )
        .bind(fixture.community.as_uuid())
        .bind(workflow_id)
        .bind(fixture.actor.public_key().to_bytes().as_slice())
        .bind(fixture.channel)
        .bind(serde_json::json!({"name":"continuity workflow","steps":[]}))
        .bind(vec![7_u8; 32])
        .execute(pool)
        .await
        .expect("seed workflow");

        let agent = Keys::generate();
        insert_human(pool, fixture.community, &agent).await;
        sqlx::query(
            "UPDATE users SET agent_owner_pubkey=$3 \
             WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(agent.public_key().to_bytes().as_slice())
        .bind(fixture.actor.public_key().to_bytes().as_slice())
        .execute(pool)
        .await
        .expect("classify continuity agent");
        crate::channel::add_member(
            pool,
            fixture.community,
            fixture.channel,
            agent.public_key().to_bytes().as_slice(),
            MemberRole::Bot,
            Some(fixture.owner.public_key().to_bytes().as_slice()),
        )
        .await
        .expect("seed agent membership");

        ContinuityRecords {
            root_event_id: root.id.as_bytes().to_vec(),
            reply_event_id: reply.id.as_bytes().to_vec(),
            workflow_id,
            agent,
        }
    }

    async fn assert_continuity_preserved(
        pool: &PgPool,
        fixture: &Fixture,
        records: &ContinuityRecords,
    ) {
        let canvas: Option<String> =
            sqlx::query_scalar("SELECT canvas FROM channels WHERE community_id=$1 AND id=$2")
                .bind(fixture.community.as_uuid())
                .bind(fixture.channel)
                .fetch_one(pool)
                .await
                .expect("read canvas");
        assert_eq!(
            canvas.as_deref(),
            Some("# Recovery continuity\n\nKeep this canvas.")
        );

        let message_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events \
             WHERE community_id=$1 AND channel_id=$2 AND (id=$3 OR id=$4)",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .bind(&records.root_event_id)
        .bind(&records.reply_event_id)
        .fetch_one(pool)
        .await
        .expect("read messages");
        assert_eq!(message_count, 2);

        let thread_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM thread_metadata \
             WHERE community_id=$1 AND channel_id=$2 \
               AND (event_id=$3 OR event_id=$4)",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .bind(&records.root_event_id)
        .bind(&records.reply_event_id)
        .fetch_one(pool)
        .await
        .expect("read thread metadata");
        assert_eq!(thread_rows, 2);

        let workflow_channel: Uuid =
            sqlx::query_scalar("SELECT channel_id FROM workflows WHERE community_id=$1 AND id=$2")
                .bind(fixture.community.as_uuid())
                .bind(records.workflow_id)
                .fetch_one(pool)
                .await
                .expect("read workflow");
        assert_eq!(workflow_channel, fixture.channel);

        let agent_role: String = sqlx::query_scalar(
            "SELECT role::text FROM channel_members \
             WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3 AND removed_at IS NULL",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .bind(records.agent.public_key().to_bytes().as_slice())
        .fetch_one(pool)
        .await
        .expect("read agent membership");
        assert_eq!(agent_role, "bot");
    }

    async fn assert_no_recovery_side_effects(pool: &PgPool, fixture: &Fixture) {
        let target_role: String = sqlx::query_scalar(
            "SELECT role::text FROM channel_members \
             WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .bind(fixture.target.public_key().to_bytes().as_slice())
        .fetch_one(pool)
        .await
        .expect("target role");
        assert_eq!(target_role, "member");
        let audit_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM channel_owner_recovery_audit \
             WHERE community_id=$1 AND channel_id=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .fetch_one(pool)
        .await
        .expect("audit count");
        assert_eq!(audit_count, 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn eligible_recovery_is_promotion_only_atomic_audited_and_idempotent() {
        let pool = setup_pool().await;
        let fixture = setup_fixture(&pool).await;
        let continuity = seed_continuity_records(&pool, &fixture).await;
        record_owner_consent(&pool, &fixture).await;
        let request = request(&fixture, "approved continuity recovery");

        let first = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "approved continuity recovery",
            &request,
        )
        .await
        .expect("eligible recovery");
        assert!(first.applied);
        assert!(!first.delivered);
        assert_eq!(first.payload.predicate_id, RECOVERY_PREDICATE_ID);
        assert_eq!(first.payload.reason_code, RECOVERY_REASON_CODE);
        assert_eq!(first.payload.event_type, "channel_owner_recovered");
        assert_continuity_preserved(&pool, &fixture, &continuity).await;
        let pending = pending_recovery_deliveries(&pool, 1_000)
            .await
            .expect("load pending recovery audit");
        let pending = pending
            .iter()
            .find(|item| item.request_event_id.as_slice() == request.id.as_bytes().as_slice())
            .expect("recovery audit remains available for worker delivery");
        assert_eq!(pending.community_id, fixture.community);
        assert_eq!(pending.payload, first.payload);

        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT role::text FROM channel_members \
             WHERE community_id=$1 AND channel_id=$2 AND removed_at IS NULL \
             ORDER BY role::text,pubkey",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .fetch_all(&pool)
        .await
        .expect("read roles");
        assert_eq!(roles, vec!["bot", "owner", "owner"]);

        let channel_name: String = sqlx::query_scalar(
            "SELECT name FROM channels WHERE community_id=$1 AND id=$2 \
             AND archived_at IS NULL AND deleted_at IS NULL",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .fetch_one(&pool)
        .await
        .expect("channel preserved");
        assert_eq!(channel_name, "orphaned");

        let mismatched_replay = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "different reason",
            &request,
        )
        .await;
        assert!(matches!(
            mismatched_replay,
            Err(DbError::InvalidData(message))
                if message == "recovery replay does not match the committed audit"
        ));

        let replay = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "approved continuity recovery",
            &request,
        )
        .await
        .expect("idempotent replay");
        assert!(!replay.applied);
        let audit_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM channel_owner_recovery_audit \
             WHERE community_id=$1 AND request_event_id=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(request.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count audit");
        assert_eq!(audit_count, 1);

        let immutable = sqlx::query(
            "UPDATE channel_owner_recovery_audit SET reason='changed' \
             WHERE community_id=$1 AND request_event_id=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(request.id.as_bytes().as_slice())
        .execute(&pool)
        .await;
        assert!(immutable.is_err(), "audit row must be immutable");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_new_request_is_denied_but_committed_replay_remains_retryable() {
        let pool = setup_pool().await;
        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;

        let stale = request_as_at(
            &fixture.actor,
            &fixture,
            "stale new request",
            nostr::Timestamp::from(nostr::Timestamp::now().as_secs().saturating_sub(121)),
        );
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "stale new request",
            &stale,
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let retryable = request_as_at(
            &fixture.actor,
            &fixture,
            "retryable committed request",
            nostr::Timestamp::from(nostr::Timestamp::now().as_secs().saturating_sub(119)),
        );
        recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "retryable committed request",
            &retryable,
        )
        .await
        .expect("fresh original request");
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let replay = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "retryable committed request",
            &retryable,
        )
        .await
        .expect("stale committed replay");
        assert!(!replay.applied);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn every_current_owner_must_self_consent_to_the_same_target() {
        let pool = setup_pool().await;
        let fixture = setup_fixture(&pool).await;
        let second_owner = Keys::generate();
        insert_human(&pool, fixture.community, &second_owner).await;
        crate::channel::add_member(
            &pool,
            fixture.community,
            fixture.channel,
            second_owner.public_key().to_bytes().as_slice(),
            MemberRole::Owner,
            Some(fixture.owner.public_key().to_bytes().as_slice()),
        )
        .await
        .expect("add second owner");
        record_owner_consent(&pool, &fixture).await;

        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "second owner has not consented",
            &request(&fixture, "second owner has not consented"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        crate::archived_identities::archive(
            &pool,
            fixture.community,
            &second_owner.public_key().to_hex(),
            "self",
            &second_owner.public_key().to_hex(),
            Some("second owner retired"),
            Some(&fixture.target.public_key().to_hex()),
            &"12".repeat(32),
        )
        .await
        .expect("archive second owner");
        let accepted = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "all owners consented",
            &request(&fixture, "all owners consented"),
        )
        .await
        .expect("all owners consented");
        assert!(accepted.applied);
        assert_eq!(accepted.payload.prior_elevated_roles.len(), 2);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn missing_self_consent_and_archived_target_fail_without_side_effects() {
        let pool = setup_pool().await;
        let fixture = setup_fixture(&pool).await;
        let denied_request = request(&fixture, "must be denied");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "must be denied",
            &denied_request,
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));

        crate::archived_identities::archive(
            &pool,
            fixture.community,
            &fixture.target.public_key().to_hex(),
            "self",
            &fixture.target.public_key().to_hex(),
            Some("target retired"),
            None,
            &"bb".repeat(32),
        )
        .await
        .expect("archive target");
        record_owner_consent(&pool, &fixture).await;
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "target archived",
            &request(&fixture, "target archived"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));

        assert_no_recovery_side_effects(&pool, &fixture).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unknown_actor_and_unknown_target_fail_closed() {
        let pool = setup_pool().await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        let unknown_actor = Keys::generate();
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "unknown actor",
            &request_as(&unknown_actor, &fixture, "unknown actor"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        sqlx::query("DELETE FROM users WHERE community_id=$1 AND pubkey=$2")
            .bind(fixture.community.as_uuid())
            .bind(fixture.target.public_key().to_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("remove target user classification");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "unknown target",
            &request(&fixture, "unknown target"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn community_admin_member_and_agent_actor_are_denied() {
        let pool = setup_pool().await;

        for role in ["admin", "member"] {
            let fixture = setup_fixture(&pool).await;
            record_owner_consent(&pool, &fixture).await;
            sqlx::query(
                "UPDATE relay_members SET role=$3 \
                 WHERE community_id=$1 AND pubkey=$2",
            )
            .bind(fixture.community.as_uuid())
            .bind(fixture.actor.public_key().to_hex())
            .bind(role)
            .execute(&pool)
            .await
            .expect("change community role");
            let denied = recover_channel_owner(
                &pool,
                fixture.community,
                fixture.channel,
                fixture.target.public_key().to_bytes().as_slice(),
                "unauthorized community role",
                &request(&fixture, "unauthorized community role"),
            )
            .await;
            assert!(
                matches!(denied, Err(DbError::AccessDenied(_))),
                "community {role} must be denied"
            );
            assert_no_recovery_side_effects(&pool, &fixture).await;
        }

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        sqlx::query(
            "UPDATE users SET agent_owner_pubkey=$3 \
             WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.actor.public_key().to_bytes().as_slice())
        .bind(fixture.owner.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("classify actor as agent");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "agent actor",
            &request(&fixture, "agent actor"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deactivated_actor_target_and_owner_are_denied() {
        let pool = setup_pool().await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        sqlx::query(
            "UPDATE users SET deactivated_at=NOW() \
             WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.actor.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("deactivate actor");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "deactivated actor",
            &request(&fixture, "deactivated actor"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        sqlx::query(
            "UPDATE users SET deactivated_at=NOW() \
             WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.target.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("deactivate target");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "deactivated target",
            &request(&fixture, "deactivated target"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        sqlx::query(
            "UPDATE users SET deactivated_at=NOW() \
             WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.owner.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("deactivate current owner");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "deactivated current owner",
            &request(&fixture, "deactivated current owner"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn removed_bot_and_agent_targets_are_denied() {
        let pool = setup_pool().await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        crate::channel::remove_member(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            fixture.owner.public_key().to_bytes().as_slice(),
        )
        .await
        .expect("remove target");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "removed target",
            &request(&fixture, "removed target"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::MemberNotFound(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        sqlx::query(
            "UPDATE channel_members SET role='bot' \
             WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .bind(fixture.target.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("classify target membership as bot");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "bot target",
            &request(&fixture, "bot target"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        sqlx::query(
            "UPDATE users SET agent_owner_pubkey=$3 \
             WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.target.public_key().to_bytes().as_slice())
        .bind(fixture.actor.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("classify target as agent");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "agent target",
            &request(&fixture, "agent target"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn administrative_archive_and_wrong_replacement_do_not_qualify_owner() {
        let pool = setup_pool().await;

        let fixture = setup_fixture(&pool).await;
        crate::archived_identities::archive(
            &pool,
            fixture.community,
            &fixture.owner.public_key().to_hex(),
            "admin",
            &fixture.actor.public_key().to_hex(),
            Some("administrative archive"),
            Some(&fixture.target.public_key().to_hex()),
            &"dd".repeat(32),
        )
        .await
        .expect("archive through administrative path");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "administrative archive cannot qualify",
            &request(&fixture, "administrative archive cannot qualify"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let fixture = setup_fixture(&pool).await;
        crate::archived_identities::archive(
            &pool,
            fixture.community,
            &fixture.owner.public_key().to_hex(),
            "self",
            &fixture.owner.public_key().to_hex(),
            Some("named another replacement"),
            Some(&Keys::generate().public_key().to_hex()),
            &"ee".repeat(32),
        )
        .await
        .expect("archive with another replacement");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "wrong replacement",
            &request(&fixture, "wrong replacement"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn active_admin_and_elevated_agent_each_block_recovery() {
        let pool = setup_pool().await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        let admin = Keys::generate();
        insert_human(&pool, fixture.community, &admin).await;
        crate::channel::add_member(
            &pool,
            fixture.community,
            fixture.channel,
            admin.public_key().to_bytes().as_slice(),
            MemberRole::Admin,
            Some(fixture.owner.public_key().to_bytes().as_slice()),
        )
        .await
        .expect("add active human admin");
        crate::archived_identities::archive(
            &pool,
            fixture.community,
            &admin.public_key().to_hex(),
            "self",
            &admin.public_key().to_hex(),
            Some("admin retired but membership still active"),
            Some(&fixture.target.public_key().to_hex()),
            &"ff".repeat(32),
        )
        .await
        .expect("archive active admin");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "admin can use ordinary path",
            &request(&fixture, "admin can use ordinary path"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        let agent = Keys::generate();
        insert_human(&pool, fixture.community, &agent).await;
        sqlx::query(
            "UPDATE users SET agent_owner_pubkey=$3 \
             WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(agent.public_key().to_bytes().as_slice())
        .bind(fixture.actor.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("classify elevated identity as agent");
        crate::channel::add_member(
            &pool,
            fixture.community,
            fixture.channel,
            agent.public_key().to_bytes().as_slice(),
            MemberRole::Admin,
            Some(fixture.owner.public_key().to_bytes().as_slice()),
        )
        .await
        .expect("add elevated agent");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "agent must block",
            &request(&fixture, "agent must block"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn archived_deleted_and_cross_tenant_channels_are_rejected() {
        let pool = setup_pool().await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        sqlx::query(
            "UPDATE channels SET archived_at=NOW() \
             WHERE community_id=$1 AND id=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .execute(&pool)
        .await
        .expect("archive channel");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "archived channel",
            &request(&fixture, "archived channel"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));

        sqlx::query(
            "UPDATE channels SET archived_at=NULL, deleted_at=NOW() \
             WHERE community_id=$1 AND id=$2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .execute(&pool)
        .await
        .expect("delete channel");
        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "deleted channel",
            &request(&fixture, "deleted channel"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let other_id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(other_id)
            .bind(format!("cross-tenant-{}.example", other_id.simple()))
            .execute(&pool)
            .await
            .expect("insert other community");
        let denied = recover_channel_owner(
            &pool,
            CommunityId::from_uuid(other_id),
            fixture.channel,
            fixture.target.public_key().to_bytes().as_slice(),
            "cross tenant",
            &request(&fixture, "cross tenant"),
        )
        .await;
        assert!(matches!(denied, Err(DbError::ChannelNotFound(_))));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cross_community_target_is_not_a_channel_member() {
        let pool = setup_pool().await;
        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;

        let other_id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(other_id)
            .bind(format!("cross-target-{}.example", other_id.simple()))
            .execute(&pool)
            .await
            .expect("insert target community");
        let other_community = CommunityId::from_uuid(other_id);
        let other_target = Keys::generate();
        insert_human(&pool, other_community, &other_target).await;
        let cross_target_request =
            EventBuilder::new(Kind::Custom(KIND_CHANNEL_OWNER_RECOVERY as u16), "")
                .tags([
                    Tag::parse(["-"]).unwrap(),
                    Tag::parse(["h", &fixture.channel.to_string()]).unwrap(),
                    Tag::parse(["p", &other_target.public_key().to_hex()]).unwrap(),
                    Tag::parse(["reason", "cross-community target"]).unwrap(),
                ])
                .sign_with_keys(&fixture.actor)
                .expect("sign cross-community target request");

        let denied = recover_channel_owner(
            &pool,
            fixture.community,
            fixture.channel,
            other_target.public_key().to_bytes().as_slice(),
            "cross-community target",
            &cross_target_request,
        )
        .await;
        assert!(matches!(denied, Err(DbError::MemberNotFound(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_requests_allow_exactly_one_promotion_and_audit() {
        let pool = setup_pool().await;
        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        let request_a = request(&fixture, "concurrent A");
        let request_b = request(&fixture, "concurrent B");
        let target = fixture.target.public_key().to_bytes();
        let community = fixture.community;
        let channel = fixture.channel;
        let pool_a = pool.clone();
        let pool_b = pool.clone();

        let (result_a, result_b) = tokio::join!(
            recover_channel_owner(
                &pool_a,
                community,
                channel,
                target.as_slice(),
                "concurrent A",
                &request_a,
            ),
            recover_channel_owner(
                &pool_b,
                community,
                channel,
                target.as_slice(),
                "concurrent B",
                &request_b,
            )
        );
        assert_eq!(
            [result_a.is_ok(), result_b.is_ok()]
                .into_iter()
                .filter(|ok| *ok)
                .count(),
            1
        );
        let audit_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM channel_owner_recovery_audit \
             WHERE community_id=$1 AND channel_id=$2",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_one(&pool)
        .await
        .expect("count audits");
        assert_eq!(audit_count, 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn target_archive_and_removal_races_cannot_partially_recover() {
        let pool = setup_pool().await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        let mut archive_tx = pool.begin().await.expect("begin archive race");
        crate::archived_identities::lock_identity(
            &mut archive_tx,
            fixture.community,
            &fixture.target.public_key().to_hex(),
        )
        .await
        .expect("hold target archive lock");
        let recovery_pool = pool.clone();
        let community = fixture.community;
        let channel = fixture.channel;
        let target = fixture.target.public_key().to_bytes();
        let recovery_request = request(&fixture, "archive race");
        let recovery = tokio::spawn(async move {
            recover_channel_owner(
                &recovery_pool,
                community,
                channel,
                target.as_slice(),
                "archive race",
                &recovery_request,
            )
            .await
        });
        tokio::task::yield_now().await;
        sqlx::query(
            "INSERT INTO archived_identities \
             (community_id,pubkey,consent_path,actor,reason,request_event_id) \
             VALUES ($1,$2,'self',$2,'archive won race',$3)",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.target.public_key().to_hex())
        .bind("cc".repeat(32))
        .execute(&mut *archive_tx)
        .await
        .expect("archive target while recovery waits");
        archive_tx.commit().await.expect("commit archive");
        let denied = recovery.await.expect("join archive race");
        assert!(matches!(denied, Err(DbError::AccessDenied(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;

        let fixture = setup_fixture(&pool).await;
        record_owner_consent(&pool, &fixture).await;
        let mut removal_tx = pool.begin().await.expect("begin removal race");
        crate::channel::lock_channel_membership(
            &mut removal_tx,
            fixture.community,
            fixture.channel,
        )
        .await
        .expect("hold membership lock");
        let recovery_pool = pool.clone();
        let community = fixture.community;
        let channel = fixture.channel;
        let target = fixture.target.public_key().to_bytes();
        let recovery_request = request(&fixture, "removal race");
        let recovery = tokio::spawn(async move {
            recover_channel_owner(
                &recovery_pool,
                community,
                channel,
                target.as_slice(),
                "removal race",
                &recovery_request,
            )
            .await
        });
        tokio::task::yield_now().await;
        sqlx::query(
            "UPDATE channel_members SET removed_at=NOW() \
             WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.channel)
        .bind(fixture.target.public_key().to_bytes().as_slice())
        .execute(&mut *removal_tx)
        .await
        .expect("remove target while recovery waits");
        removal_tx.commit().await.expect("commit removal");
        let denied = recovery.await.expect("join removal race");
        assert!(matches!(denied, Err(DbError::MemberNotFound(_))));
        assert_no_recovery_side_effects(&pool, &fixture).await;
    }
}
