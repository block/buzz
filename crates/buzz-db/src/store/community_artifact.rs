//! Explicit NIP-AR community audience. Payloads live in the revision ledger,
//! outside the event store, search index, mentions and conversation side effects.
use super::artifact::{artifact_receipt_exists, check_artifact_head, ArtifactOutcome};
use crate::{ArtifactSnapshot, Db, DbError, Result};
use buzz_core::artifact::{
    canonical_uuid, ArtifactEnvelope, ArtifactOp, ArtifactQuery, ArtifactScope,
    ArtifactTypePolicyCommand, ArtifactView, MAX_COMMUNITY_POLICIES,
};
use buzz_core::{CommunityId, StoredEvent};
use nostr::Event;
use serde_json::{json, Value};
use sqlx::{PgConnection, Postgres, QueryBuilder, Row, Transaction};
use uuid::Uuid;

/// Transport-proven principal; never constructed from artifact content.
#[derive(Clone, Debug)]
pub struct CommunityArtifactPrincipal {
    /// Authenticated event signer or reader.
    pub pubkey: Vec<u8>,
    /// Owner proven by the current authenticated transport, if delegated.
    pub verified_owner: Option<Vec<u8>>,
    /// Current transport policy permits owner roster fallback. The verified
    /// owner remains a restriction identity even when this is false.
    pub owner_admission: bool,
    /// Current effective relay admission mode.
    pub require_membership: bool,
    /// A long-lived socket without FI evidence may read only while the tenant
    /// policy permits it. HTTP requests use their own policy admission.
    pub reject_required_nip_fi: bool,
}

fn denied() -> DbError {
    DbError::AccessDenied("community artifact access denied".into())
}

async fn set_admission_timeouts(conn: &mut PgConnection) -> Result<()> {
    sqlx::raw_sql("SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='2s'")
        .execute(conn)
        .await?;
    Ok(())
}

// Match relay_invite::lock_admission and take this advisory lock before any
// row lock, so a restriction writer can commit before admission rechecks it.
async fn lock_restriction_writers(conn: &mut PgConnection, community: CommunityId) -> Result<()> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock_shared(hashtextextended('invite-admission:' || $1::text,0))",
    )
    .bind(community.as_uuid())
    .execute(conn)
    .await?;
    Ok(())
}

async fn lock_member_role(
    conn: &mut PgConnection,
    community: CommunityId,
    pubkey: &[u8],
) -> Result<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT role FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR SHARE",
    )
    .bind(community.as_uuid())
    .bind(hex::encode(pubkey))
    .fetch_optional(conn)
    .await?)
}

/// Call with the restriction and active community locks held. Only the
/// roster row supplying admission needs a member lock.
async fn authorize_members(
    conn: &mut PgConnection,
    community: CommunityId,
    principal: &CommunityArtifactPrincipal,
    allow_owner_fallback: bool,
) -> Result<Authority> {
    let direct = lock_member_role(conn, community, &principal.pubkey).await?;
    let delegated = if principal.require_membership
        && direct.is_none()
        && allow_owner_fallback
        && principal.owner_admission
    {
        match &principal.verified_owner {
            Some(owner) => lock_member_role(conn, community, owner).await?.is_some(),
            None => false,
        }
    } else {
        false
    };
    let row = sqlx::query("SELECT COALESCE(bool_or(banned AND (ban_expires_at IS NULL OR ban_expires_at > clock_timestamp())),false) AS banned, COALESCE(bool_or(muted_until > clock_timestamp()),false) AS timed_out FROM community_bans WHERE community_id=$1 AND pubkey IN (SELECT $2::bytea UNION ALL SELECT $3::bytea UNION ALL SELECT agent_owner_pubkey FROM users WHERE community_id=$1 AND pubkey=$2)")
        .bind(community.as_uuid()).bind(&principal.pubkey).bind(&principal.verified_owner).fetch_one(&mut *conn).await?;
    if row.try_get::<bool, _>("banned")?
        || (principal.require_membership && direct.is_none() && !delegated)
    {
        return Err(denied());
    }
    Ok(Authority {
        admin: matches!(direct.as_deref(), Some("owner" | "admin")),
        timed_out: row.try_get("timed_out")?,
    })
}
struct Authority {
    admin: bool,
    timed_out: bool,
}

// Take the community row before member rows, matching ownership transfer.
// SHARE conflicts with archive/auth-policy writers' NO KEY UPDATE lock while
// allowing readers together. Policy commands take NO KEY UPDATE up front.
async fn lock_active_community(conn: &mut PgConnection, community: CommunityId) -> Result<bool> {
    sqlx::query_scalar("SELECT requires_nip_fi FROM communities WHERE id=$1 AND archived_at IS NULL AND deleted_at IS NULL AND deletion_state='active' FOR SHARE")
        .bind(community.as_uuid())
        .fetch_optional(conn)
        .await?
        .ok_or_else(denied)
}

/// Policy lookup result. Explicit `everyone` entries keep their revision and slot.
#[derive(Debug, serde::Serialize, PartialEq, Eq)]
pub struct ArtifactTypePolicy {
    /// Effective write policy.
    pub policy: String,
    /// Last accepted command ID, absent only when never configured.
    pub revision: Option<String>,
}
fn policy_value(state: &Value, artifact_type: &str) -> Result<ArtifactTypePolicy> {
    match state.get(artifact_type) {
        None => Ok(ArtifactTypePolicy {
            policy: "everyone".into(),
            revision: None,
        }),
        Some(value) => Ok(ArtifactTypePolicy {
            policy: value
                .get("policy")
                .and_then(Value::as_str)
                .filter(|p| matches!(*p, "everyone" | "admins"))
                .ok_or_else(|| DbError::InvalidData("invalid stored artifact policy".into()))?
                .into(),
            revision: Some(
                value
                    .get("revision")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        DbError::InvalidData("invalid stored artifact policy revision".into())
                    })?
                    .into(),
            ),
        }),
    }
}

/// Result of a policy CAS. Its caller commits the audit on the same transaction.
#[derive(Debug, PartialEq, Eq)]
pub enum PolicyOutcome {
    /// A new accepted command; append its audit entry before committing.
    Applied,
    /// Exact current command retry, with current direct authority.
    Duplicate,
    /// Stale `prev` or the bounded policy map is full.
    Conflict(&'static str),
}

/// Validate and apply a policy CAS inside the caller's audit transaction.
/// Community deletion admission and current direct authority are rechecked here.
/// The caller must have verified transport, signature, tenant and `admin:users`.
pub async fn set_policy_in_transaction(
    tx: &mut Transaction<'static, Postgres>,
    community: CommunityId,
    principal: &CommunityArtifactPrincipal,
    event: &Event,
    command: &ArtifactTypePolicyCommand,
) -> Result<PolicyOutcome> {
    if command.community != *community.as_uuid()
        || event.pubkey.as_bytes().as_slice() != principal.pubkey
    {
        return Err(denied());
    }
    set_admission_timeouts(tx).await?;
    crate::deletion::guard_community_write(tx, community).await?;
    lock_restriction_writers(tx, community).await?;
    // The row lock fences artifact writers' SHARE locks and permits foreign-key
    // checks by membership writers. It also serializes policy map updates.
    let state: Value =
        sqlx::query_scalar("SELECT artifact_type_policies FROM communities WHERE id=$1 AND archived_at IS NULL AND deleted_at IS NULL AND deletion_state='active' FOR NO KEY UPDATE")
            .bind(community.as_uuid())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(denied)?;
    let authority = authorize_members(tx, community, principal, false).await?;
    // Recheck the shared relay-admin write restriction under admission locks
    // before accepting either a new policy or an exact retry.
    if !authority.admin || authority.timed_out {
        return Err(denied());
    }
    let current = policy_value(&state, &command.artifact_type)?;
    // An authorized retry of the current command bypasses the freshness check.
    if current.revision.as_deref() == Some(event.id.to_hex().as_str()) {
        return Ok(PolicyOutcome::Duplicate);
    }
    let now = chrono::Utc::now().timestamp();
    if event.created_at.as_secs().abs_diff(now as u64) > 120 {
        return Err(DbError::InvalidData(
            "policy command timestamp out of range".into(),
        ));
    }
    if command.prev != current.revision {
        return Ok(PolicyOutcome::Conflict("artifact policy changed"));
    }
    let mut entries = state
        .as_object()
        .cloned()
        .ok_or_else(|| DbError::InvalidData("invalid artifact policy state".into()))?;
    if !entries.contains_key(&command.artifact_type) && entries.len() >= MAX_COMMUNITY_POLICIES {
        return Ok(PolicyOutcome::Conflict("artifact policy limit exceeded"));
    }
    entries.insert(
        command.artifact_type.clone(),
        json!({"policy":command.policy,"revision":event.id.to_hex()}),
    );
    sqlx::query("UPDATE communities SET artifact_type_policies=$2 WHERE id=$1 AND deletion_state = 'active' AND deleted_at IS NULL")
        .bind(community.as_uuid())
        .bind(Value::Object(entries))
        .execute(&mut **tx)
        .await?;
    Ok(PolicyOutcome::Applied)
}

impl Db {
    /// Contract readiness, checked before activation. The old unscoped key must
    /// be gone and the scoped primary key valid; the expand alone never activates.
    pub async fn community_artifact_schema_ready(&self) -> Result<bool> {
        Ok(sqlx::query_scalar(r#"
            SELECT EXISTS (
                SELECT 1 FROM pg_index i
                WHERE i.indrelid='artifact_heads'::regclass AND i.indisprimary
                  AND i.indisvalid AND i.indisready AND i.indpred IS NULL AND i.indexprs IS NULL
                  AND (SELECT array_agg(a.attname::text ORDER BY k.ord)
                       FROM unnest(i.indkey) WITH ORDINALITY k(num,ord)
                       JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.num)
                      = ARRAY['community_id','scope','artifact_id']
            ) AND NOT EXISTS (
                SELECT 1 FROM pg_index i WHERE i.indrelid='artifact_heads'::regclass AND i.indisunique
                  AND (SELECT array_agg(a.attname::text ORDER BY k.ord)
                       FROM unnest(i.indkey) WITH ORDINALITY k(num,ord)
                       JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.num)
                      = ARRAY['community_id','artifact_id']
            ) AND (
                SELECT count(*)=7 FROM (VALUES
                    ('artifact_heads','scope','text',true),
                    ('artifact_heads','channel_id','uuid',false),
                    ('artifact_revisions','scope','text',true),
                    ('artifact_revisions','community_event','jsonb',false),
                    ('artifact_revisions','redacted','boolean',true),
                    ('communities','artifact_type_policies','jsonb',true),
                    ('artifact_revisions','event_id','bytea',true)
                ) expected(rel,col,typ,required)
                JOIN pg_attribute a ON a.attrelid=to_regclass(expected.rel)
                  AND a.attname=expected.col AND NOT a.attisdropped
                  AND a.atttypid=to_regtype(expected.typ) AND a.attnotnull=expected.required
            ) AND (
                SELECT count(*)=2 FROM pg_attribute a JOIN pg_attrdef d
                  ON d.adrelid=a.attrelid AND d.adnum=a.attnum
                WHERE a.attrelid IN ('artifact_heads'::regclass,'artifact_revisions'::regclass)
                  AND a.attname='scope' AND pg_get_expr(d.adbin,d.adrelid) = $$'channel'::text$$
            )
        "#).fetch_one(&self.pool).await?)
    }
    // Admission and content use the same repeatable-read cut on the writer.
    async fn begin_community_artifact_read(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
    ) -> Result<Transaction<'static, Postgres>> {
        let connection = crate::observability::acquire_writer_until(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
            tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        )
        .await?;
        let mut tx = super::artifact_query::begin_snapshot(connection).await?;
        let admitted: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM communities c
                WHERE c.id=$1 AND c.archived_at IS NULL AND c.deleted_at IS NULL
                  AND c.deletion_state='active' AND (NOT $6 OR NOT c.requires_nip_fi)
                  AND (NOT $4 OR EXISTS (
                      SELECT 1 FROM relay_members m WHERE m.community_id=c.id
                      AND (m.pubkey=encode($2,'hex') OR ($5 AND m.pubkey=encode($3,'hex')))
                  ))
                  AND NOT EXISTS (
                      SELECT 1 FROM community_bans b WHERE b.community_id=c.id AND b.banned
                      AND (b.ban_expires_at IS NULL OR b.ban_expires_at>clock_timestamp())
                      AND b.pubkey IN (
                          SELECT $2::bytea UNION ALL SELECT $3::bytea
                          UNION ALL SELECT u.agent_owner_pubkey FROM users u
                          WHERE u.community_id=c.id AND u.pubkey=$2
                      )
                  )
             )",
        )
        .bind(community.as_uuid())
        .bind(&principal.pubkey)
        .bind(&principal.verified_owner)
        .bind(principal.require_membership)
        .bind(principal.owner_admission)
        .bind(principal.reject_required_nip_fi)
        .fetch_one(&mut *tx)
        .await?;
        if !admitted {
            return Err(denied());
        }
        Ok(tx)
    }
    /// Read policy with current admission. Unset types have no revision.
    pub async fn community_artifact_policy(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        artifact_type: &str,
    ) -> Result<ArtifactTypePolicy> {
        let mut tx = self
            .begin_community_artifact_read(community, principal)
            .await?;
        let state: Value =
            sqlx::query_scalar("SELECT artifact_type_policies FROM communities WHERE id=$1")
                .bind(community.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
        let policy = policy_value(&state, artifact_type)?;
        tx.commit().await?;
        Ok(policy)
    }
    /// Exact accepted replay needs admission and tenant binding, never edit authority.
    pub async fn community_artifact_accepted(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        event: &Event,
    ) -> Result<bool> {
        let env = buzz_core::artifact::validate(event).map_err(|_| denied())?;
        check_revision_binding(community, principal, event, &env)?;
        let mut tx = self
            .begin_community_artifact_read(community, principal)
            .await?;
        let accepted =
            artifact_receipt_exists(&mut tx, community, event.id.as_bytes(), Some("community"))
                .await?;
        tx.commit().await?;
        Ok(accepted)
    }
    /// Community CAS: admission, policy scope, identity, head check, ledger and
    /// head persist. No event-store insert or conversation side effects.
    pub async fn accept_community_artifact(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        event: &Event,
        env: &ArtifactEnvelope,
    ) -> Result<ArtifactOutcome> {
        check_revision_binding(community, principal, event, env)?;
        let mut tx = self.begin_event_write_transaction(community).await?;
        set_admission_timeouts(tx.conn()).await?;
        lock_restriction_writers(tx.conn(), community).await?;
        lock_active_community(tx.conn(), community).await?;
        let authority = authorize_members(tx.conn(), community, principal, true).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("artifact:{community}:community:{}", env.id))
            .execute(tx.conn())
            .await?;
        if artifact_receipt_exists(tx.conn(), community, event.id.as_bytes(), Some("community"))
            .await?
        {
            return Ok(ArtifactOutcome::Duplicate);
        }
        if authority.timed_out {
            return Err(denied());
        }
        let state: Value =
            sqlx::query_scalar("SELECT artifact_type_policies FROM communities WHERE id=$1")
                .bind(community.as_uuid())
                .fetch_one(tx.conn())
                .await?;
        if policy_value(&state, &env.artifact_type)?.policy == "admins" && !authority.admin {
            return Err(denied());
        }
        let head=sqlx::query("SELECT event_id,artifact_type,deleted FROM artifact_heads WHERE community_id=$1 AND scope='community' AND artifact_id=$2 FOR UPDATE")
            .bind(community.as_uuid()).bind(env.id).fetch_optional(tx.conn()).await?;
        if let Some(outcome) = check_artifact_head(env, head.as_ref())? {
            return Ok(outcome);
        }
        sqlx::query("INSERT INTO artifact_revisions(community_id,scope,event_id,artifact_id,community_event,modified_at) VALUES($1,'community',$2,$3,$4,clock_timestamp())")
            .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice()).bind(env.id).bind(serde_json::to_value(event).map_err(|e|DbError::InvalidData(e.to_string()))?).execute(tx.conn()).await?;
        sqlx::query("INSERT INTO artifact_heads(community_id,scope,artifact_id,event_id,channel_id,artifact_type,deleted,modified_at) VALUES($1,'community',$2,$3,NULL,$4,$5,clock_timestamp()) ON CONFLICT(community_id,scope,artifact_id) DO UPDATE SET event_id=EXCLUDED.event_id,deleted=EXCLUDED.deleted,modified_at=clock_timestamp()")
            .bind(community.as_uuid()).bind(env.id).bind(event.id.as_bytes().as_slice()).bind(&env.artifact_type).bind(env.op==ArtifactOp::Delete).execute(tx.conn()).await?;
        tx.commit().await?;
        Ok(ArtifactOutcome::Accepted(vec![StoredEvent::new(
            event.clone(),
            None,
        )]))
    }
}

fn check_revision_binding(
    community: CommunityId,
    principal: &CommunityArtifactPrincipal,
    event: &Event,
    env: &ArtifactEnvelope,
) -> Result<()> {
    if env.scope != ArtifactScope::Community
        || env.community != Some(*community.as_uuid())
        || event.pubkey.as_bytes().as_slice() != principal.pubkey
    {
        return Err(denied());
    }
    Ok(())
}

pub(super) fn query_builder(
    community: CommunityId,
    view: ArtifactView,
    tags: &[(String, Vec<String>)],
) -> QueryBuilder<Postgres> {
    let mut qb = QueryBuilder::new("SELECT r.artifact_id,r.community_event,r.modified_at ");
    if view == ArtifactView::Head {
        qb.push(",a.deleted ");
    }
    if view == ArtifactView::History {
        qb.push("FROM artifact_revisions r WHERE r.community_id=")
            .push_bind(*community.as_uuid())
            .push(" AND r.scope='community' AND NOT r.redacted AND r.community_event IS NOT NULL");
    } else {
        // Drive current reads from typed heads, not every historical revision.
        // LIMIT 1 keeps the PK lookup lateral so the planner cannot reverse
        // the join and scan the tenant ledger before the bounded head page.
        qb.push("FROM artifact_heads a JOIN LATERAL (SELECT r.artifact_id,r.community_event,r.modified_at,r.event_id,r.scope FROM artifact_revisions r WHERE r.community_id=a.community_id AND r.event_id=a.event_id AND NOT r.redacted AND r.community_event IS NOT NULL LIMIT 1) r ON r.scope=a.scope AND r.artifact_id=a.artifact_id WHERE a.community_id=")
            .push_bind(*community.as_uuid())
            .push(" AND a.scope='community'");
    }
    if view == ArtifactView::Current {
        qb.push(" AND NOT a.deleted");
    }
    for (name, values) in tags {
        if view != ArtifactView::History && name == "type" {
            if let [value] = values.as_slice() {
                qb.push(" AND a.artifact_type=").push_bind(value);
            } else {
                qb.push(" AND a.artifact_type=ANY(")
                    .push_bind(values)
                    .push(")");
            }
            continue;
        }
        if name == "d" {
            let (predicate, ids) = if view == ArtifactView::History {
                // History matched the signed tag exactly. Only the canonical
                // UUID spelling can match an accepted revision.
                (
                    " AND r.artifact_id=ANY(",
                    values
                        .iter()
                        .filter_map(|v| canonical_uuid(v).ok())
                        .collect::<Vec<_>>(),
                )
            } else {
                (
                    " AND a.artifact_id=ANY(",
                    values
                        .iter()
                        .filter_map(|v| v.parse().ok())
                        .collect::<Vec<_>>(),
                )
            };
            qb.push(predicate).push_bind(ids).push(")");
            continue;
        }
        qb.push(" AND EXISTS(SELECT 1 FROM jsonb_array_elements(r.community_event->'tags') tag WHERE tag->>0=").push_bind(name).push(" AND tag->>1=ANY(").push_bind(values).push("))");
    }
    qb
}
pub(super) fn rows_to_events(rows: Vec<sqlx::postgres::PgRow>) -> Result<Vec<StoredEvent>> {
    rows.iter().map(row_to_event).collect()
}
fn row_to_event(row: &sqlx::postgres::PgRow) -> Result<StoredEvent> {
    let event: Event = serde_json::from_value(row.try_get("community_event")?)
        .map_err(|e| DbError::InvalidData(e.to_string()))?;
    Ok(StoredEvent::with_received_at(
        event,
        row.try_get("modified_at")?,
        None,
        true,
    ))
}
impl Db {
    /// Explicit bounded community read; admission precedes filtering and paging.
    pub async fn query_community_artifacts(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        query: &ArtifactQuery,
    ) -> Result<Vec<StoredEvent>> {
        if query.scope != ArtifactScope::Community || query.view == ArtifactView::Policy {
            return Err(denied());
        }
        let mut tx = self
            .begin_community_artifact_read(community, principal)
            .await?;
        let mut qb = query_builder(community, query.view, &query.tags);
        qb.push(" ORDER BY r.modified_at DESC,r.event_id LIMIT ")
            .push_bind(query.limit)
            .push(" OFFSET ")
            .push_bind(query.offset);
        let events = rows_to_events(qb.build().fetch_all(&mut *tx).await?)?;
        tx.commit().await?;
        Ok(events)
    }
    /// Open a community complete-set snapshot on the writer, after live interest.
    pub async fn open_community_artifact_snapshot(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
    ) -> Result<ArtifactSnapshot> {
        // Reserve capacity for other writer-pool operations.
        if self.pool.options().get_max_connections() < 2 {
            return Err(DbError::ArtifactSyncLimit);
        }
        let permit = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            self.community_artifact_snapshots.clone().acquire_owned(),
        )
        .await
        .map_err(|_| DbError::ArtifactSyncLimit)?
        .map_err(|_| DbError::ArtifactSyncLimit)?;
        let tx = self
            .begin_community_artifact_read(community, principal)
            .await?;
        Ok(ArtifactSnapshot::for_community(tx, community, permit))
    }
    /// Reload the current authorized head for a community change signal.
    pub async fn community_artifact_head(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        id: Uuid,
    ) -> Result<Option<(StoredEvent, bool)>> {
        let mut tx = self
            .begin_community_artifact_read(community, principal)
            .await?;
        let mut query = query_builder(community, ArtifactView::Head, &[]);
        query.push(" AND a.artifact_id=").push_bind(id);
        let row = query.build().fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        row.as_ref()
            .map(|r| Ok((row_to_event(r)?, r.try_get("deleted")?)))
            .transpose()
    }
    /// Redact only a community revision in this tenant. Keep its acceptance and
    /// head ID. The same transaction writes the existing moderation audit row.
    pub async fn redact_community_artifact(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        target: &[u8],
    ) -> Result<Option<(Uuid, String)>> {
        let mut tx = self.begin_event_write_transaction(community).await?;
        set_admission_timeouts(tx.conn()).await?;
        lock_restriction_writers(tx.conn(), community).await?;
        lock_active_community(tx.conn(), community).await?;
        let authority = authorize_members(tx.conn(), community, principal, false).await?;
        if !authority.admin || authority.timed_out {
            return Err(denied());
        }
        let changed=sqlx::query("UPDATE artifact_revisions SET redacted=true WHERE community_id=$1 AND scope='community' AND event_id=$2 AND community_event IS NOT NULL AND NOT redacted")
            .bind(community.as_uuid()).bind(target).execute(tx.conn()).await?.rows_affected()>0;
        if changed {
            sqlx::query("INSERT INTO moderation_actions(community_id,actor_pubkey,action,target_event_id,actor_authority) VALUES($1,$2,'delete_message',$3,'community')")
                .bind(community.as_uuid()).bind(&principal.pubkey).bind(target).execute(tx.conn()).await?;
        } else {
            let exists: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND scope='community' AND event_id=$2 AND community_event IS NOT NULL)")
                .bind(community.as_uuid()).bind(target).fetch_one(tx.conn()).await?;
            if !exists {
                return Err(DbError::NotFound("community artifact unavailable".into()));
            }
        }
        // Lock the current head through commit so a historical redaction does
        // not masquerade as retirement of a later revision. Concurrent CAS
        // writers also lock this row before moving the head.
        let retired = if changed {
            sqlx::query_as::<_, (Uuid, String)>("SELECT artifact_id,artifact_type FROM artifact_heads WHERE community_id=$1 AND scope='community' AND event_id=$2 FOR UPDATE")
                .bind(community.as_uuid()).bind(target).fetch_optional(tx.conn()).await?
        } else {
            None
        };
        tx.commit().await?;
        Ok(retired)
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use sqlx::Execute;

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn community_artifact_history_can_seek_by_identity() {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            crate::migration::run_migrations(&pool).await.unwrap();
        }
        let mut tx = pool.begin().await.unwrap();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("history-plan-{community}.test"))
            .execute(&mut *tx)
            .await
            .unwrap();
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        // Give the planner enough revisions in one community that locating a
        // few identities through the index is cheaper than parsing every tag.
        sqlx::query(
            "INSERT INTO artifact_revisions(community_id,scope,event_id,artifact_id,community_event) \
             SELECT $1,'community',sha256(n::text::bytea),d, \
                    jsonb_build_object('tags',jsonb_build_array(jsonb_build_array('d',d))) \
             FROM generate_series(1,2048) n \
             CROSS JOIN LATERAL (SELECT coalesce(($2::uuid[])[n],md5(n::text)::uuid) AS d) identity",
        )
        .bind(community.as_uuid())
        .bind(ids.as_slice())
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("ANALYZE artifact_revisions")
            .execute(&mut *tx)
            .await
            .unwrap();
        let tags = [("d".into(), ids.iter().map(Uuid::to_string).collect())];
        let mut builder = query_builder(community, ArtifactView::History, &tags);
        builder.push(" ORDER BY r.modified_at DESC,r.event_id LIMIT 50");
        let mut statement = builder.build();
        let arguments = statement.take_arguments().unwrap().unwrap();
        let explain = format!("EXPLAIN (FORMAT JSON) {}", statement.sql().as_str());
        let plan: Value = sqlx::query_scalar_with(sqlx::AssertSqlSafe(explain), arguments)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        fn seeks_artifact(node: &Value) -> bool {
            node["Index Cond"]
                .as_str()
                .is_some_and(|condition| condition.contains("artifact_id"))
                || node["Plans"]
                    .as_array()
                    .is_some_and(|plans| plans.iter().any(seeks_artifact))
        }
        assert!(seeks_artifact(&plan[0]["Plan"]), "{plan:#}");
    }
}
