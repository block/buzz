//! Operator identity retirement: archive an exact key and its verified agents,
//! hold their automation, and release that hold explicitly.
//!
//! See `docs/operator-identity-retirement.md`. Each operation is one admitted
//! transaction. The execution hold (`identity_retirement_holds`) is separate
//! from NIP-IA visibility (`archived_identities`): a signed NIP-IA unarchive
//! never releases it, and release never changes visibility.
//!
//! Serialization: retirement takes the community's invite-admission lock
//! (held by every agent-owner link writer) and the identity-retirement lock
//! exclusively. Workflow and run writers take the identity-retirement lock
//! shared ([`admit_shared`]), so none of them commits between the retirement's
//! inventory read and its commit.

use std::collections::BTreeSet;

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::Db;

/// Largest identity or workflow set one retirement accepts.
pub const MAX_RETIREMENT_ITEMS: usize = 200;

/// Stored `error_code` of runs cancelled by retirement.
pub const RETIRED_RUN_ERROR_CODE: &str = "identity_retired";

/// Take the identity-retirement lock shared. Every workflow and run writer
/// calls this inside its admitted transaction before writing.
pub(crate) async fn admit_shared(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended('identity-retirement:' || $1::text, 0))")
        .bind(community.as_uuid())
        .execute(conn)
        .await?;
    Ok(())
}

pub(crate) async fn lock_exclusive(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
) -> Result<()> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('identity-retirement:' || $1::text, 0))",
    )
    .bind(community.as_uuid())
    .execute(conn)
    .await?;
    Ok(())
}

/// Whether a workflow owned by `owner` is held: the owner itself, or the
/// owner's recorded owner, has an execution hold. Call after [`admit_shared`].
pub(crate) async fn owner_is_held(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    owner: &[u8],
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_retirement_holds h \
         WHERE h.community_id = $1 AND (h.pubkey = $2 OR h.pubkey = \
           (SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2)))",
    )
    .bind(community.as_uuid())
    .bind(owner)
    .fetch_one(conn)
    .await?)
}

/// Requested operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RetirementOperation {
    /// Archive the key and verified agents and hold their automation.
    Archive,
    /// Release the key's execution hold and restore its retired workflows.
    Release,
}

impl RetirementOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Release => "release",
        }
    }
}

/// Canonical request payload; a replay must match it exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetirementPayload {
    /// Machine-readable reason code.
    pub reason: Option<String>,
    /// Replacement key (lowercase hex), archive only.
    pub replaced_by: Option<String>,
    /// Inventory version the operator confirmed, archive only.
    pub inventory_version: Option<String>,
}

/// One operator request.
#[derive(Debug, Clone)]
pub struct RetirementRequest {
    /// Community the operation is scoped to.
    pub community: CommunityId,
    /// Caller idempotency key, unique per community.
    pub request_id: Uuid,
    /// Operation.
    pub operation: RetirementOperation,
    /// Exact target key (32 bytes).
    pub target: [u8; 32],
    /// Authenticated signer (32 bytes).
    pub actor: [u8; 32],
    /// Payload compared on replay.
    pub payload: RetirementPayload,
}

/// Historical result of an action, stored with it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetirementResult {
    /// Affected identities, root first.
    pub identities: Vec<ResultIdentity>,
    /// Affected workflows.
    pub workflows: Vec<ResultWorkflow>,
    /// Run outcomes.
    pub runs: RunCounts,
    /// Per-stage outcome, except visibility (derived from publication).
    pub stages: Stages,
}

/// One identity in a result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResultIdentity {
    /// Lowercase hex key.
    pub pubkey: String,
    /// `root` or `agent`.
    pub role: String,
    /// NIP-IA state after the action.
    pub archived: bool,
    /// Own execution hold after the action.
    pub execution_held: bool,
    /// Whether this action changed the identity's archive or hold state.
    pub changed: bool,
}

/// One workflow in a result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResultWorkflow {
    /// Workflow id.
    pub id: Uuid,
    /// Lowercase hex owner.
    pub owner_pubkey: String,
    /// Status after the action.
    pub status: String,
    /// Whether this action changed its status.
    pub changed: bool,
    /// Why a release left it archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// Run outcomes of an archive action.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunCounts {
    /// Runs moved to `cancelled`.
    pub cancelled: i64,
    /// Pending approvals moved to `expired`.
    pub approvals_expired: i64,
    /// Cancelled runs that were executing; stopped only once a worker stamps them.
    pub in_flight: i64,
}

/// Stage outcomes known at write time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Stages {
    /// `blocked`, `released` or `unchanged`.
    pub new_work_admission: String,
    /// `confirmed`, `partial` or `unchanged`.
    pub workflows: String,
    /// `none` or `draining`.
    pub in_flight_runs: String,
    /// Always `unsupported`: the relay cannot stop agent processes.
    pub agent_runtime: String,
}

/// A stored action.
#[derive(Debug, Clone)]
pub struct RetirementRecord {
    /// Action id.
    pub action_id: Uuid,
    /// Request id.
    pub request_id: Uuid,
    /// Operation.
    pub operation: RetirementOperation,
    /// Lowercase hex target.
    pub target: String,
    /// Historical result.
    pub result: RetirementResult,
    /// Whether the NIP-IA snapshot was published after this action.
    pub published: bool,
}

/// Outcome of [`Db::apply_identity_retirement`].
#[derive(Debug)]
pub enum RetirementOutcome {
    /// Applied now.
    Applied {
        /// Stored action.
        record: RetirementRecord,
        /// Keys this action newly archived.
        archived: Vec<String>,
    },
    /// The same request was already applied; nothing changed.
    Replayed(RetirementRecord),
}

/// Why a request was refused.
#[derive(Debug, thiserror::Error)]
pub enum RetirementError {
    /// The request id was used for a different request.
    #[error("request id was already used for a different request")]
    RequestIdConflict,
    /// The current inventory differs from the confirmed version.
    #[error("inventory changed since it was confirmed")]
    InventoryChanged,
    /// More identities or workflows than one retirement accepts.
    #[error("inventory exceeds {MAX_RETIREMENT_ITEMS} identities or workflows")]
    InventoryTooLarge,
    /// Archive requires a confirmed inventory version.
    #[error("inventoryVersion is required")]
    MissingInventoryVersion,
    /// Database failure.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<sqlx::Error> for RetirementError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e.into())
    }
}

/// Version of an inventory: SHA-256 over sorted lowercase-hex keys and sorted
/// `id:owner:status:enabled` workflow lines.
pub fn inventory_version<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    workflows: impl IntoIterator<Item = (Uuid, &'a str, &'a str, bool)>,
) -> String {
    let keys: BTreeSet<&str> = keys.into_iter().collect();
    let workflows: BTreeSet<String> = workflows
        .into_iter()
        .map(|(id, owner, status, enabled)| format!("{id}:{owner}:{status}:{enabled}"))
        .collect();
    let mut hash = Sha256::new();
    hash.update(b"identity-inventory-v1\n");
    for key in &keys {
        hash.update(key.as_bytes());
        hash.update(b"\n");
    }
    hash.update(b"--\n");
    for line in &workflows {
        hash.update(line.as_bytes());
        hash.update(b"\n");
    }
    hex::encode(hash.finalize())
}

struct OwnedWorkflow {
    id: Uuid,
    owner: Vec<u8>,
    status: String,
    enabled: bool,
}

fn record_from_row(row: &sqlx::postgres::PgRow) -> std::result::Result<RetirementRecord, DbError> {
    let operation: String = row.try_get("operation")?;
    let target: Vec<u8> = row.try_get("target_pubkey")?;
    let result: serde_json::Value = row.try_get("result")?;
    let published: Option<DateTime<Utc>> = row.try_get("published_at")?;
    Ok(RetirementRecord {
        action_id: row.try_get("id")?,
        request_id: row.try_get("request_id")?,
        operation: serde_json::from_value(serde_json::Value::String(operation))?,
        target: hex::encode(target),
        result: serde_json::from_value(result)?,
        published: published.is_some(),
    })
}

impl Db {
    /// Apply one retirement operation in a single admitted transaction, or
    /// replay a stored one. A replay never re-applies intent.
    #[datastore_span(name = "apply_identity_retirement", system = "postgresql")]
    pub async fn apply_identity_retirement(
        &self,
        req: &RetirementRequest,
    ) -> std::result::Result<RetirementOutcome, RetirementError> {
        let community = req.community;
        let mut tx = crate::begin_community_write_transaction(
            self.pool(),
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::query("SELECT set_config('lock_timeout', '5s', true), set_config('statement_timeout', '15s', true)")
            .execute(tx.conn())
            .await?;
        crate::relay_invite::lock_admission(tx.conn(), community).await?;
        lock_exclusive(tx.conn(), community).await?;

        let payload = serde_json::to_value(&req.payload).map_err(DbError::from)?;
        if let Some(row) = sqlx::query(
            "SELECT id, request_id, operation, target_pubkey, actor_pubkey, request, result, \
             published_at FROM identity_retirement_actions \
             WHERE community_id = $1 AND request_id = $2",
        )
        .bind(community.as_uuid())
        .bind(req.request_id)
        .fetch_optional(tx.conn())
        .await?
        {
            // Compare the stored row as written before decoding it: an
            // earlier release may have recorded an operation this one no
            // longer accepts, and reusing its request id is a conflict.
            let operation: String = row.try_get("operation")?;
            let target: Vec<u8> = row.try_get("target_pubkey")?;
            let actor: Vec<u8> = row.try_get("actor_pubkey")?;
            let stored: serde_json::Value = row.try_get("request")?;
            tx.rollback().await?;
            if operation != req.operation.as_str()
                || target != req.target
                || actor != req.actor
                || stored != payload
            {
                return Err(RetirementError::RequestIdConflict);
            }
            return Ok(RetirementOutcome::Replayed(record_from_row(&row)?));
        }

        let action_id = Uuid::new_v4();
        let (result, archived) = match req.operation {
            RetirementOperation::Archive => archive(&mut tx, req, action_id).await?,
            RetirementOperation::Release => (release(&mut tx, req).await?, Vec::new()),
        };
        sqlx::query(
            "INSERT INTO identity_retirement_actions \
             (community_id, id, request_id, operation, target_pubkey, actor_pubkey, request, result) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(community.as_uuid())
        .bind(action_id)
        .bind(req.request_id)
        .bind(req.operation.as_str())
        .bind(req.target.as_slice())
        .bind(req.actor.as_slice())
        .bind(&payload)
        .bind(serde_json::to_value(&result).map_err(DbError::from)?)
        .execute(tx.conn())
        .await?;
        tx.commit().await?;
        Ok(RetirementOutcome::Applied {
            record: RetirementRecord {
                action_id,
                request_id: req.request_id,
                operation: req.operation,
                target: hex::encode(req.target),
                result,
                published: false,
            },
            archived,
        })
    }

    /// Record that the NIP-IA snapshot was published after `action_id`.
    #[datastore_span(name = "mark_identity_retirement_published", system = "postgresql")]
    pub async fn mark_identity_retirement_published(
        &self,
        community: CommunityId,
        action_id: Uuid,
    ) -> Result<()> {
        let mut tx = crate::begin_community_write_transaction(
            self.pool(),
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::query(
            "UPDATE identity_retirement_actions SET published_at = COALESCE(published_at, now()) \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(action_id)
        .execute(tx.conn())
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Returns the result and the keys this archive newly archived.
async fn archive(
    tx: &mut crate::AdmittedTx,
    req: &RetirementRequest,
    action_id: Uuid,
) -> std::result::Result<(RetirementResult, Vec<String>), RetirementError> {
    let community = req.community.as_uuid();
    let confirmed = req
        .payload
        .inventory_version
        .as_deref()
        .ok_or(RetirementError::MissingInventoryVersion)?;
    let root = req.target.to_vec();
    let mut keys: Vec<Vec<u8>> = vec![root.clone()];
    keys.extend(
        sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT pubkey FROM users WHERE community_id = $1 AND agent_owner_pubkey = $2 \
             AND pubkey <> $2 ORDER BY pubkey LIMIT $3",
        )
        .bind(community)
        .bind(&root)
        .bind(MAX_RETIREMENT_ITEMS as i64 + 1)
        .fetch_all(tx.conn())
        .await?,
    );
    if keys.len() > MAX_RETIREMENT_ITEMS {
        return Err(RetirementError::InventoryTooLarge);
    }
    let workflows = sqlx::query(
        "SELECT id, owner_pubkey, status::text AS status, enabled FROM workflows \
         WHERE community_id = $1 AND owner_pubkey = ANY($2) ORDER BY id LIMIT $3 FOR UPDATE",
    )
    .bind(community)
    .bind(&keys)
    .bind(MAX_RETIREMENT_ITEMS as i64 + 1)
    .fetch_all(tx.conn())
    .await?
    .iter()
    .map(|row| {
        Ok(OwnedWorkflow {
            id: row.try_get("id")?,
            owner: row.try_get("owner_pubkey")?,
            status: row.try_get("status")?,
            enabled: row.try_get("enabled")?,
        })
    })
    .collect::<std::result::Result<Vec<_>, sqlx::Error>>()?;
    if workflows.len() > MAX_RETIREMENT_ITEMS {
        return Err(RetirementError::InventoryTooLarge);
    }
    let key_hex: Vec<String> = keys.iter().map(hex::encode).collect();
    let owner_hex: Vec<String> = workflows.iter().map(|w| hex::encode(&w.owner)).collect();
    let current = inventory_version(
        key_hex.iter().map(String::as_str),
        workflows
            .iter()
            .zip(&owner_hex)
            .map(|(w, owner)| (w.id, owner.as_str(), w.status.as_str(), w.enabled)),
    );
    if current != confirmed {
        return Err(RetirementError::InventoryChanged);
    }

    let actor_hex = hex::encode(req.actor);
    let archived: BTreeSet<String> = sqlx::query_scalar(
        "INSERT INTO archived_identities \
         (community_id, pubkey, consent_path, actor, reason, replaced_by, request_event_id) \
         SELECT $1, k, 'relay', $3, $4, CASE WHEN k = $6 THEN $5 END, '' FROM unnest($2::text[]) k \
         ON CONFLICT (community_id, pubkey) DO NOTHING RETURNING pubkey",
    )
    .bind(community)
    .bind(&key_hex)
    .bind(&actor_hex)
    .bind(req.payload.reason.as_deref())
    .bind(req.payload.replaced_by.as_deref())
    .bind(&key_hex[0])
    .fetch_all(tx.conn())
    .await?
    .into_iter()
    .collect();
    let held: BTreeSet<Vec<u8>> = sqlx::query_scalar(
        "INSERT INTO identity_retirement_holds (community_id, pubkey, action_id) \
         SELECT $1, k, $3 FROM unnest($2::bytea[]) k \
         ON CONFLICT (community_id, pubkey) DO NOTHING RETURNING pubkey",
    )
    .bind(community)
    .bind(&keys)
    .bind(action_id)
    .fetch_all(tx.conn())
    .await?
    .into_iter()
    .collect();

    let active: Vec<Uuid> = workflows
        .iter()
        .filter(|w| w.status == "active")
        .map(|w| w.id)
        .collect();
    let retired: BTreeSet<Uuid> = sqlx::query_scalar(
        "WITH changed AS (UPDATE workflows SET status = 'archived', updated_at = now() \
           WHERE community_id = $1 AND id = ANY($2) AND status = 'active' \
           RETURNING id, owner_pubkey, updated_at) \
         INSERT INTO identity_retirement_workflows \
           (community_id, workflow_id, owner_pubkey, action_id, archived_at) \
         SELECT $1, id, owner_pubkey, $3, updated_at FROM changed \
         ON CONFLICT (community_id, workflow_id) DO UPDATE \
           SET owner_pubkey = EXCLUDED.owner_pubkey, action_id = EXCLUDED.action_id, \
               archived_at = EXCLUDED.archived_at \
         RETURNING workflow_id",
    )
    .bind(community)
    .bind(&active)
    .bind(action_id)
    .fetch_all(tx.conn())
    .await?
    .into_iter()
    .collect();

    let ids: Vec<Uuid> = workflows.iter().map(|w| w.id).collect();
    let (cancelled, in_flight): (i64, i64) = sqlx::query_as(
        "WITH target AS (SELECT id, status FROM workflow_runs \
           WHERE community_id = $1 AND workflow_id = ANY($2) \
             AND status IN ('pending', 'running', 'waiting_approval') FOR UPDATE), \
         changed AS (UPDATE workflow_runs r SET status = 'cancelled', error_code = $3, \
             error_message = 'workflow owner identity retired', \
             completed_at = CASE WHEN t.status = 'running' THEN NULL ELSE now() END \
           FROM target t WHERE r.community_id = $1 AND r.id = t.id RETURNING t.status) \
         SELECT count(*), count(*) FILTER (WHERE status = 'running') FROM changed",
    )
    .bind(community)
    .bind(&ids)
    .bind(RETIRED_RUN_ERROR_CODE)
    .fetch_one(tx.conn())
    .await?;
    let approvals_expired = sqlx::query(
        "UPDATE workflow_approvals SET status = 'expired' \
         WHERE community_id = $1 AND workflow_id = ANY($2) AND status = 'pending'",
    )
    .bind(community)
    .bind(&ids)
    .execute(tx.conn())
    .await?
    .rows_affected() as i64;

    let identities = keys
        .iter()
        .zip(&key_hex)
        .enumerate()
        .map(|(i, (key, hex))| ResultIdentity {
            pubkey: hex.clone(),
            role: if i == 0 { "root" } else { "agent" }.into(),
            archived: true,
            execution_held: true,
            changed: archived.contains(hex) || held.contains(key),
        })
        .collect();
    let workflows = workflows
        .iter()
        .zip(owner_hex)
        .map(|(w, owner_pubkey)| {
            let changed = retired.contains(&w.id);
            ResultWorkflow {
                id: w.id,
                owner_pubkey,
                status: if changed {
                    "archived".into()
                } else {
                    w.status.clone()
                },
                changed,
                skipped: None,
            }
        })
        .collect();
    let result = RetirementResult {
        identities,
        workflows,
        runs: RunCounts {
            cancelled,
            approvals_expired,
            in_flight,
        },
        stages: Stages {
            new_work_admission: "blocked".into(),
            workflows: if retired.is_empty() {
                "unchanged"
            } else {
                "confirmed"
            }
            .into(),
            in_flight_runs: if in_flight > 0 { "draining" } else { "none" }.into(),
            agent_runtime: "unsupported".into(),
        },
    };
    let archived = key_hex
        .into_iter()
        .filter(|k| archived.contains(k))
        .collect();
    Ok((result, archived))
}

async fn release(
    tx: &mut crate::AdmittedTx,
    req: &RetirementRequest,
) -> std::result::Result<RetirementResult, RetirementError> {
    let community = req.community.as_uuid();
    let key = req.target.as_slice();
    let released = sqlx::query(
        "DELETE FROM identity_retirement_holds WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community)
    .bind(key)
    .execute(tx.conn())
    .await?
    .rows_affected()
        > 0;
    // Restore only workflows still exactly as this retirement left them.
    let rows = sqlx::query(
        "WITH recorded AS (DELETE FROM identity_retirement_workflows \
           WHERE community_id = $1 AND owner_pubkey = $2 \
           RETURNING workflow_id, archived_at), \
         restored AS (UPDATE workflows w SET status = 'active', updated_at = now() \
           FROM recorded p WHERE w.community_id = $1 AND w.id = p.workflow_id \
             AND w.owner_pubkey = $2 AND w.status = 'archived' AND w.updated_at = p.archived_at \
           RETURNING w.id) \
         SELECT p.workflow_id, w.status::text AS status, \
                EXISTS (SELECT 1 FROM restored r WHERE r.id = p.workflow_id) AS restored \
         FROM recorded p LEFT JOIN workflows w ON w.community_id = $1 AND w.id = p.workflow_id \
         ORDER BY p.workflow_id",
    )
    .bind(community)
    .bind(key)
    .fetch_all(tx.conn())
    .await?;
    let owner_hex = hex::encode(key);
    let mut workflows = Vec::with_capacity(rows.len());
    for row in rows {
        let restored: bool = row.try_get("restored")?;
        // The CTE's UPDATE is not visible to the outer read of `workflows`.
        let status: Option<String> = row.try_get("status")?;
        workflows.push(ResultWorkflow {
            id: row.try_get("workflow_id")?,
            owner_pubkey: owner_hex.clone(),
            status: if restored {
                "active".into()
            } else {
                status.unwrap_or_else(|| "deleted".into())
            },
            changed: restored,
            skipped: (!restored).then(|| "modified_since_retirement".into()),
        });
    }
    let owner_held = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM identity_retirement_holds h JOIN users u \
           ON u.community_id = h.community_id AND u.agent_owner_pubkey = h.pubkey \
         WHERE h.community_id = $1 AND u.pubkey = $2)",
    )
    .bind(community)
    .bind(key)
    .fetch_one(tx.conn())
    .await?;
    let archived = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM archived_identities WHERE community_id = $1 AND pubkey = $2)",
    )
    .bind(community)
    .bind(&owner_hex)
    .fetch_one(tx.conn())
    .await?;
    let admission = if owner_held {
        "blocked"
    } else if released {
        "released"
    } else {
        "unchanged"
    };
    let stage = if workflows.is_empty() {
        "unchanged"
    } else if workflows.iter().all(|w| w.changed) {
        "confirmed"
    } else {
        "partial"
    };
    let result = RetirementResult {
        identities: vec![ResultIdentity {
            pubkey: owner_hex,
            role: "root".into(),
            archived,
            execution_held: false,
            changed: released,
        }],
        workflows,
        runs: RunCounts::default(),
        stages: Stages {
            new_work_admission: admission.into(),
            workflows: stage.into(),
            in_flight_runs: "none".into(),
            agent_runtime: "unsupported".into(),
        },
    };
    Ok(result)
}
