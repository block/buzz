//! Explicit NIP-AR community audience. Payloads live in the revision ledger,
//! outside the event store, search index, mentions and conversation side effects.
use super::artifact::{ArtifactOutcome, HEAD_UNAVAILABLE};
use crate::{Db, DbError, Result};
use buzz_core::artifact::{
    ArtifactEnvelope, ArtifactOp, ArtifactQuery, ArtifactScope, ArtifactTypePolicyCommand,
    ArtifactView, MAX_COMMUNITY_POLICIES,
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
}

fn denied() -> DbError {
    DbError::AccessDenied("community artifact access denied".into())
}

/// Serialize with existing ban, timeout and ownership writers. Membership rows
/// are also share-locked, so a removal cannot commit between this check and a
/// guarded enqueue. Shared reads do not serialize readers with each other.
async fn authorize(
    conn: &mut PgConnection,
    community: CommunityId,
    principal: &CommunityArtifactPrincipal,
) -> Result<Authority> {
    sqlx::query("SET LOCAL statement_timeout='2s'")
        .execute(&mut *conn)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='2s'")
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "SELECT pg_advisory_xact_lock_shared(hashtextextended('invite-admission:' || $1::text,0))",
    )
    .bind(community.as_uuid())
    .execute(&mut *conn)
    .await?;
    let direct: Option<String> = sqlx::query_scalar(
        "SELECT role FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR SHARE",
    )
    .bind(community.as_uuid())
    .bind(hex::encode(&principal.pubkey))
    .fetch_optional(&mut *conn)
    .await?;
    let delegated = if let Some(owner) = principal
        .verified_owner
        .as_ref()
        .filter(|_| principal.owner_admission)
    {
        sqlx::query_scalar::<_, String>(
            "SELECT role FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR SHARE",
        )
        .bind(community.as_uuid())
        .bind(hex::encode(owner))
        .fetch_optional(&mut *conn)
        .await?
        .is_some()
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

/// A fresh authorization transaction held through a response's queue point.
/// Its locks serialize that queue point with committed admission revocations.
pub struct CommunityArtifactReadGuard {
    tx: Transaction<'static, Postgres>,
    requires_nip_fi: bool,
}
impl CommunityArtifactReadGuard {
    /// Current stored authentication requirement, protected through enqueue.
    pub fn requires_nip_fi(&self) -> bool {
        self.requires_nip_fi
    }

    /// Release read-admission locks after the protected enqueue.
    pub async fn finish(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
}

// SHARE conflicts with archive/auth-policy writers' NO KEY UPDATE lock, while
// allowing readers together. Writers take their type lock before this row;
// policy commands use UPDATE directly, avoiding shared-lock upgrade deadlocks.
async fn lock_active_community(conn: &mut PgConnection, community: CommunityId) -> Result<bool> {
    sqlx::query_scalar("SELECT requires_nip_fi FROM communities WHERE id=$1 AND archived_at IS NULL AND deleted_at IS NULL AND deletion_state='active' FOR SHARE")
        .bind(community.as_uuid())
        .fetch_optional(conn)
        .await?
        .ok_or_else(denied)
}

async fn policy_lock(
    conn: &mut PgConnection,
    community: CommunityId,
    artifact_type: &str,
) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("artifact-policy:{community}:{artifact_type}"))
        .execute(conn)
        .await?;
    Ok(())
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
    crate::deletion::guard_community_write(tx, community).await?;
    let authority = authorize(tx, community, principal).await?;
    // Relay-admin commands retain administrative authority during a content
    // timeout. authorize still refuses the principal's and applicable owner's
    // bans; new artifact revisions separately enforce their content timeout.
    if !authority.admin {
        return Err(denied());
    }
    policy_lock(tx, community, &command.artifact_type).await?;
    let state: Value =
        sqlx::query_scalar("SELECT artifact_type_policies FROM communities WHERE id=$1 AND archived_at IS NULL AND deleted_at IS NULL AND deletion_state='active' FOR UPDATE")
            .bind(community.as_uuid())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(denied)?;
    let current = policy_value(&state, &command.artifact_type)?;
    // Bans and current direct authority precede command receipts, matching
    // the shared relay-admin gate (content timeouts do not block commands).
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
    /// Fresh writer-backed audience admission held through the caller's enqueue.
    pub async fn community_artifact_read_guard(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
    ) -> Result<CommunityArtifactReadGuard> {
        let connection = crate::observability::acquire_writer_until(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
            tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        )
        .await?;
        let mut tx = Transaction::begin(connection, None).await?;
        sqlx::raw_sql("SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        crate::deletion::guard_community_write(&mut tx, community).await?;
        authorize(&mut tx, community, principal).await?;
        let requires_nip_fi = lock_active_community(&mut tx, community).await?;
        Ok(CommunityArtifactReadGuard {
            tx,
            requires_nip_fi,
        })
    }
    /// Read policy with current admission. Unset types have no revision.
    pub async fn community_artifact_policy(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        artifact_type: &str,
    ) -> Result<ArtifactTypePolicy> {
        let mut guard = self
            .community_artifact_read_guard(community, principal)
            .await?;
        let state: Value =
            sqlx::query_scalar("SELECT artifact_type_policies FROM communities WHERE id=$1")
                .bind(community.as_uuid())
                .fetch_one(&mut *guard.tx)
                .await?;
        let policy = policy_value(&state, artifact_type)?;
        guard.finish().await?;
        Ok(policy)
    }
    /// Exact accepted replay needs admission and tenant binding, never new edit
    /// authority. The durable ledger survives redaction and deletion payloads.
    pub async fn community_artifact_accepted(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        event: &Event,
    ) -> Result<bool> {
        let envelope = buzz_core::artifact::validate(event).map_err(|_| denied())?;
        if envelope.scope != ArtifactScope::Community
            || envelope.community != Some(*community.as_uuid())
            || event.pubkey.as_bytes().as_slice() != principal.pubkey
        {
            return Err(denied());
        }
        let mut guard = self
            .community_artifact_read_guard(community, principal)
            .await?;
        let accepted=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND scope='community' AND event_id=$2)")
            .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice()).fetch_one(&mut *guard.tx).await?;
        guard.finish().await?;
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
        if env.scope != ArtifactScope::Community
            || env.community != Some(*community.as_uuid())
            || event.pubkey.as_bytes().as_slice() != principal.pubkey
        {
            return Err(denied());
        }
        let mut tx = self.begin_event_write_transaction(community).await?;
        let authority = authorize(tx.conn(), community, principal).await?;
        policy_lock(tx.conn(), community, &env.artifact_type).await?;
        lock_active_community(tx.conn(), community).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("artifact:{community}:community:{}", env.id))
            .execute(tx.conn())
            .await?;
        if sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND scope='community' AND event_id=$2)")
            .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice()).fetch_one(tx.conn()).await? {return Ok(ArtifactOutcome::Duplicate);}
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
        match (&head, env.op) {
            (Some(_), ArtifactOp::Create) => {
                return Ok(ArtifactOutcome::Conflict("artifact identity is taken"))
            }
            (None, ArtifactOp::Create) => {}
            (None, _) => return Ok(ArtifactOutcome::Conflict(HEAD_UNAVAILABLE)),
            (Some(head), op) => {
                if env.prev.as_deref() != Some(head.try_get::<Vec<u8>, _>("event_id")?.as_slice()) {
                    return Ok(ArtifactOutcome::Conflict("artifact head changed"));
                }
                if head.try_get::<String, _>("artifact_type")? != env.artifact_type {
                    return Ok(ArtifactOutcome::Rejected("artifact type is immutable"));
                }
                if head.try_get::<bool, _>("deleted")? != (op == ArtifactOp::Restore) {
                    return Ok(ArtifactOutcome::Rejected(
                        "deleted artifacts require restore; live artifacts cannot restore",
                    ));
                }
            }
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

fn query_builder(
    community: CommunityId,
    view: ArtifactView,
    tags: &[(String, Vec<String>)],
) -> QueryBuilder<Postgres> {
    let mut qb = QueryBuilder::new("SELECT r.artifact_id,r.community_event,r.modified_at ");
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
        if view != ArtifactView::History && name == "d" {
            let ids: Vec<Uuid> = values.iter().filter_map(|v| v.parse().ok()).collect();
            qb.push(" AND a.artifact_id=ANY(").push_bind(ids).push(")");
            continue;
        }
        qb.push(" AND EXISTS(SELECT 1 FROM jsonb_array_elements(r.community_event->'tags') tag WHERE tag->>0=").push_bind(name).push(" AND tag->>1=ANY(").push_bind(values).push("))");
    }
    qb
}
fn rows_to_events(rows: Vec<sqlx::postgres::PgRow>) -> Result<Vec<StoredEvent>> {
    rows.into_iter()
        .map(|row| {
            let event: Event = serde_json::from_value(row.try_get("community_event")?)
                .map_err(|e| DbError::InvalidData(e.to_string()))?;
            Ok(StoredEvent::with_received_at(
                event,
                row.try_get("modified_at")?,
                None,
                true,
            ))
        })
        .collect()
}
/// One repeatable-read cut of community heads; the sync uses the same keyset
/// paging, buffer bounds, budget and complete-set enqueue protocol as channels.
pub struct CommunityArtifactSnapshot {
    tx: Transaction<'static, Postgres>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    community: CommunityId,
    after: Option<Uuid>,
    done: bool,
}
impl CommunityArtifactSnapshot {
    /// Read one keyset page in this snapshot.
    pub async fn next_page(
        &mut self,
        tags: &[(String, Vec<String>)],
        page: i64,
    ) -> Result<Vec<StoredEvent>> {
        if self.done {
            return Ok(vec![]);
        }
        let mut qb = query_builder(self.community, ArtifactView::Current, tags);
        if let Some(after) = self.after {
            qb.push(" AND a.artifact_id>").push_bind(after);
        }
        qb.push(" ORDER BY a.artifact_id LIMIT ").push_bind(page);
        let rows = qb.build().fetch_all(&mut *self.tx).await?;
        self.done = (rows.len() as i64) < page;
        if let Some(last) = rows.last() {
            self.after = Some(last.try_get("artifact_id")?);
        }
        rows_to_events(rows)
    }
    /// End the complete-set snapshot.
    pub async fn finish(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
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
        let mut guard = self
            .community_artifact_read_guard(community, principal)
            .await?;
        let mut qb = query_builder(community, query.view, &query.tags);
        qb.push(" ORDER BY r.modified_at DESC,r.event_id LIMIT ")
            .push_bind(query.limit)
            .push(" OFFSET ")
            .push_bind(query.offset);
        let events = rows_to_events(qb.build().fetch_all(&mut *guard.tx).await?)?;
        guard.finish().await?;
        Ok(events)
    }
    /// Open a community complete-set snapshot on the writer, after live interest.
    /// Current admission is checked outside it before every enqueue.
    pub async fn open_community_artifact_snapshot(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
    ) -> Result<CommunityArtifactSnapshot> {
        // Admission is bounded independently of the 30 s snapshot budget.
        // A pool with one slot cannot hold a snapshot and recheck admission.
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
        self.community_artifact_read_guard(community, principal)
            .await?
            .finish()
            .await?;
        let connection = crate::observability::acquire_writer_until(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
            tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        )
        .await?;
        let mut tx = Transaction::begin(connection, None).await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        Ok(CommunityArtifactSnapshot {
            _permit: permit,
            tx,
            community,
            after: None,
            done: false,
        })
    }
    /// Reload the current authorized head for a community change signal.
    pub async fn community_artifact_head(
        &self,
        community: CommunityId,
        principal: &CommunityArtifactPrincipal,
        id: Uuid,
    ) -> Result<Option<(StoredEvent, bool)>> {
        let query = ArtifactQuery {
            scope: ArtifactScope::Community,
            view: ArtifactView::Head,
            tags: vec![("d".into(), vec![id.to_string()])],
            limit: 1,
            offset: 0,
        };
        Ok(self
            .query_community_artifacts(community, principal, &query)
            .await?
            .into_iter()
            .next()
            .map(|head| {
                let deleted = head
                    .event
                    .tags
                    .iter()
                    .any(|tag| tag.as_slice() == ["op", "delete"]);
                (head, deleted)
            }))
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
        let authority = authorize(tx.conn(), community, principal).await?;
        if !authority.admin || authority.timed_out {
            return Err(denied());
        }
        lock_active_community(tx.conn(), community).await?;
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

impl Db {
    /// Node-wide idle-sync backstop: check at most 256 distinct principals in
    /// one writer query. This never reads content and never grants delivery;
    /// every actual enqueue still takes a current authorization guard.
    pub async fn community_artifact_admission_batch(
        &self,
        community: CommunityId,
        principals: &[CommunityArtifactPrincipal],
    ) -> Result<Vec<(bool, bool)>> {
        if principals.len() > 256 {
            return Err(DbError::InvalidData("admission batch too large".into()));
        }
        let keys: Vec<Vec<u8>> = principals.iter().map(|p| p.pubkey.clone()).collect();
        let owners: Vec<Vec<u8>> = principals
            .iter()
            .map(|p| p.verified_owner.clone().unwrap_or_default())
            .collect();
        let require: Vec<bool> = principals.iter().map(|p| p.require_membership).collect();
        let owner_admission: Vec<bool> = principals.iter().map(|p| p.owner_admission).collect();
        let connection = crate::observability::acquire_writer_until(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
            tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        )
        .await?;
        let mut tx = Transaction::begin(connection, None).await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        let allowed=sqlx::query_as("SELECT COALESCE((SELECT requires_nip_fi FROM communities WHERE id=$1),true), EXISTS(SELECT 1 FROM communities c WHERE c.id=$1 AND c.deletion_state='active' AND c.archived_at IS NULL) AND (NOT p.roster OR EXISTS(SELECT 1 FROM relay_members m WHERE m.community_id=$1 AND (m.pubkey=encode(p.key,'hex') OR (p.owner_admission AND m.pubkey=encode(p.owner,'hex'))))) AND NOT EXISTS(SELECT 1 FROM community_bans b WHERE b.community_id=$1 AND b.banned AND (b.ban_expires_at IS NULL OR b.ban_expires_at>clock_timestamp()) AND b.pubkey IN (SELECT p.key UNION ALL SELECT p.owner UNION ALL SELECT u.agent_owner_pubkey FROM users u WHERE u.community_id=$1 AND u.pubkey=p.key)) FROM unnest($2::bytea[],$3::bytea[],$4::boolean[],$5::boolean[]) WITH ORDINALITY p(key,owner,roster,owner_admission,ord) ORDER BY p.ord")
            .bind(community.as_uuid()).bind(keys).bind(owners).bind(require).bind(owner_admission).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(allowed)
    }
}
