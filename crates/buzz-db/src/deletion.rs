//! Durable whole-community deletion lifecycle and PostgreSQL adapter.
//!
//! This module owns request inventory, approval, claims, fencing, checkpoints,
//! retries, tombstoning, and logical verification. CLI claim-loop policy and
//! external storage adapters live above it; they never implement state changes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::{DbError, Result};

/// Default PostgreSQL lease duration for one claimed deletion request.
pub const DEFAULT_LEASE_DURATION: Duration = Duration::from_secs(60);
/// Durable name of the schema manifest's PostgreSQL component.
pub const POSTGRES_STORE_NAME: &str = "postgres";
/// Durable name of the object-store manifest component.
pub const OBJECT_STORE_NAME: &str = "object_store";
/// Durable name of the Redis/cache manifest component.
pub const REDIS_STORE_NAME: &str = "redis";

/// Deployment-global advisory-lock key serializing schema migration with
/// destructive deletion.
///
/// [`crate::migration::run_migrations`] holds the exclusive session lock for
/// its entire run; destructive catalog validation, purge, and final logical
/// verification hold the shared transaction-scoped counterpart. Exact catalog
/// equality is therefore stable for the whole destructive interval, not just
/// the instant it is checked. The value is arbitrary (ASCII `buzzdel1`) but
/// permanently stable: changing it silently drops the exclusion contract
/// against replicas still holding the old key during a rolling deploy.
pub const SCHEMA_DESTRUCTION_LOCK_KEY: i64 = 0x62757a7a64656c31;

/// Control-plane tables that survive the community data purge.
pub const CONTROL_PLANE_TABLES: &[&str] = &[
    "community_deletion_approvals",
    "community_deletion_checkpoints",
    "community_deletion_executor_heartbeats",
    "community_deletion_requests",
    "community_serving_write_leases",
];

/// Expected community-scoped tables purged by V1.
///
/// Catalog inventory compares the live database against this exact set before
/// approval and again before PostgreSQL purge. A new tenant table therefore
/// blocks deletion until this manifest is intentionally updated.
pub const EXPECTED_SCOPED_TABLES: &[&str] = &[
    "api_tokens",
    "archived_identities",
    "audit_log",
    "channel_members",
    "channels",
    "community_bans",
    "delivery_log",
    "dm_visibility_dirty_viewers",
    "event_mentions",
    "events",
    "git_repo_names",
    "join_policy_acceptances",
    "moderation_actions",
    "moderation_reports",
    "parameterized_event_watermarks",
    "pubkey_allowlist",
    "push_leases",
    "push_match_queue",
    "push_wake_outbox",
    "reactions",
    "relay_invites",
    "relay_members",
    "scheduled_workflow_fires",
    "subscriptions",
    "thread_metadata",
    "users",
    "workflow_approvals",
    "workflow_runs",
    "workflows",
];

/// Foreign-key-safe child-before-parent order for the PostgreSQL purge.
pub const PURGE_SCOPED_TABLES: &[&str] = &[
    "workflow_approvals",
    "scheduled_workflow_fires",
    "workflow_runs",
    "push_wake_outbox",
    "dm_visibility_dirty_viewers",
    "join_policy_acceptances",
    "moderation_reports",
    "subscriptions",
    "api_tokens",
    "channel_members",
    "thread_metadata",
    "moderation_actions",
    "workflows",
    "event_mentions",
    "reactions",
    "push_match_queue",
    "push_leases",
    "relay_invites",
    "delivery_log",
    "events",
    "parameterized_event_watermarks",
    "git_repo_names",
    "archived_identities",
    "audit_log",
    "community_bans",
    "pubkey_allowlist",
    "relay_members",
    "users",
    "channels",
];

/// Fixed lifecycle order. There are no backwards or skipping transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionStage {
    /// Request exists but has not frozen its inventory.
    Submitted,
    /// PostgreSQL and storage inventory has been frozen.
    Inventoried,
    /// An operator explicitly approved the frozen inventory digest.
    Approved,
    /// Universal serving-path write fence is active.
    Fenced,
    /// In-flight serving writes have drained behind the durable fence.
    Drained,
    /// Tenant-owned S3/media and Git pointer bindings were removed.
    BindingsRemoved,
    /// Tenant-scoped PostgreSQL rows were purged.
    PostgresPurged,
    /// Redis/community process-cache namespace was purged.
    CachePurged,
    /// Cross-store logical absence was verified.
    LogicallyVerified,
    /// Logical deletion complete; shared CAS physical expiry is deferred.
    RetentionPending,
    /// Operator cancelled before irreversible object deletion began.
    Aborted,
}

impl DeletionStage {
    /// Next legal stage, if this is not terminal.
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Submitted => Some(Self::Inventoried),
            Self::Inventoried => Some(Self::Approved),
            Self::Approved => Some(Self::Fenced),
            Self::Fenced => Some(Self::Drained),
            Self::Drained => Some(Self::BindingsRemoved),
            Self::BindingsRemoved => Some(Self::PostgresPurged),
            Self::PostgresPurged => Some(Self::CachePurged),
            Self::CachePurged => Some(Self::LogicallyVerified),
            Self::LogicallyVerified => Some(Self::RetentionPending),
            Self::RetentionPending | Self::Aborted => None,
        }
    }

    /// Whether execution may claim this stage.
    pub const fn runnable(self) -> bool {
        matches!(
            self,
            Self::Approved
                | Self::Fenced
                | Self::Drained
                | Self::BindingsRemoved
                | Self::PostgresPurged
                | Self::CachePurged
                | Self::LogicallyVerified
        )
    }
}

impl fmt::Display for DeletionStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Submitted => "submitted",
            Self::Inventoried => "inventoried",
            Self::Approved => "approved",
            Self::Fenced => "fenced",
            Self::Drained => "drained",
            Self::BindingsRemoved => "bindings_removed",
            Self::PostgresPurged => "postgres_purged",
            Self::CachePurged => "cache_purged",
            Self::LogicallyVerified => "logically_verified",
            Self::RetentionPending => "retention_pending",
            Self::Aborted => "aborted",
        };
        f.write_str(value)
    }
}

impl FromStr for DeletionStage {
    type Err = DbError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "submitted" => Ok(Self::Submitted),
            "inventoried" => Ok(Self::Inventoried),
            "approved" => Ok(Self::Approved),
            "fenced" => Ok(Self::Fenced),
            "drained" => Ok(Self::Drained),
            "bindings_removed" => Ok(Self::BindingsRemoved),
            "postgres_purged" => Ok(Self::PostgresPurged),
            "cache_purged" => Ok(Self::CachePurged),
            "logically_verified" => Ok(Self::LogicallyVerified),
            "retention_pending" => Ok(Self::RetentionPending),
            "aborted" => Ok(Self::Aborted),
            other => Err(DbError::DeletionSafety(format!(
                "unknown community deletion stage: {other}"
            ))),
        }
    }
}

/// Durable community deletion request.
#[derive(Debug, Clone, Serialize)]
pub struct DeletionRequest {
    /// Request identifier.
    pub id: Uuid,
    /// Target community.
    #[serde(serialize_with = "serialize_community_id")]
    pub community_id: CommunityId,
    /// Permanently reserved canonical host.
    pub community_host: String,
    /// Current lifecycle stage.
    pub stage: DeletionStage,
    /// Stage at which the current consecutive retry streak started.
    pub retry_stage: Option<DeletionStage>,
    /// Operator identity that submitted the request.
    pub requested_by: String,
    /// Optional request reason.
    pub reason: Option<String>,
    /// Frozen catalog manifest.
    pub schema_manifest: Option<serde_json::Value>,
    /// Frozen community-prefix storage manifest observed at submission.
    pub storage_manifest: Option<serde_json::Value>,
    /// Destructive storage manifest frozen after the durable fence.
    pub destructive_storage_manifest: Option<serde_json::Value>,
    /// Frozen inventory aggregate.
    pub inventory_manifest: Option<serde_json::Value>,
    /// Hex SHA-256 of the frozen inventory.
    pub inventory_digest: Option<String>,
    /// Durable community fence generation.
    pub fence_generation: Option<i64>,
    /// Current claim owner.
    pub lease_owner: Option<String>,
    /// Monotonic claim generation.
    pub lease_generation: i64,
    /// Claim expiry.
    pub lease_until: Option<DateTime<Utc>>,
    /// Number of claims.
    pub attempts: i32,
    /// Number of consecutive failed execution attempts at `retry_stage`.
    pub retry_count: i32,
    /// Last bounded error.
    pub last_error: Option<String>,
    /// Earliest time a transient failure may be claimed again.
    pub next_attempt_at: DateTime<Utc>,
    /// Permanent fail-closed block reason.
    pub blocked_reason: Option<String>,
    /// Submission time.
    pub created_at: DateTime<Utc>,
    /// Last lifecycle update.
    pub updated_at: DateTime<Utc>,
    /// Archive timestamp captured before quiescing changed serving state.
    pub pre_quiesce_archived_at: Option<DateTime<Utc>>,
    /// Whether the pre-quiesce archive value has been captured (including null).
    pub quiescing_started_at: Option<DateTime<Utc>>,
    /// Operator that terminally aborted the request.
    pub aborted_by: Option<String>,
    /// Reason recorded for terminal abort.
    pub abort_reason: Option<String>,
    /// Abort completion time.
    pub aborted_at: Option<DateTime<Utc>>,
    /// Terminal logical-deletion time.
    pub completed_at: Option<DateTime<Utc>>,
}

/// Frozen PostgreSQL catalog inventory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaManifest {
    /// Sorted community-scoped table names.
    pub scoped_tables: Vec<String>,
    /// Per-table row counts for the target.
    pub row_counts: BTreeMap<String, i64>,
    /// Sorted tables with the universal write-fence trigger.
    pub fenced_tables: Vec<String>,
}

/// Frozen storage inventory supplied by the object-store adapter: slim
/// per-prefix summaries for the target community. The concrete key list never
/// lives on the request row — the destructive freeze persists it as chunked
/// `community_deletion_manifest_keys` rows that must hash to these digests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageManifest {
    /// Adapter schema version.
    pub version: i32,
    /// Per-prefix frozen summaries, strictly sorted by prefix.
    pub prefixes: Vec<PrefixManifest>,
}

/// Frozen summary of one community-scoped key prefix.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrefixManifest {
    /// Exact community-scoped listing prefix.
    pub prefix: String,
    /// Objects under the prefix at enumeration time.
    pub object_count: u64,
    /// Total object bytes under the prefix at enumeration time.
    pub total_bytes: u64,
    /// Hex SHA-256 of the newline-terminated ascending key stream.
    pub keys_digest: String,
}

/// One frozen chunk of the destructive key list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestKeyChunk {
    /// Position in the frozen chunk sequence.
    pub chunk_no: i64,
    /// The tenant prefix every key in this chunk lives under.
    pub prefix: String,
    /// Strictly ascending keys.
    pub keys: Vec<String>,
}

/// One durable fleet-wide object-store taxonomy sweep record.
#[derive(Debug, Clone, Serialize)]
pub struct TaxonomySweep {
    /// Sweep identity.
    pub id: Uuid,
    /// Listing start time.
    pub started_at: DateTime<Utc>,
    /// Record time.
    pub completed_at: DateTime<Utc>,
    /// Total objects listed.
    pub listed_objects: i64,
    /// Exact count of keys outside the known writer taxonomy.
    pub unknown_object_count: i64,
    /// Bounded sample of unknown keys.
    pub unknown_key_sample: Vec<String>,
    /// Fleet object cap the sweep ran under.
    pub object_cap: i64,
}

type TaxonomySweepRow = (
    Uuid,
    DateTime<Utc>,
    DateTime<Utc>,
    i64,
    i64,
    sqlx::types::Json<Vec<String>>,
    i64,
);

/// Streaming SHA-256 over a strictly ascending key stream.
///
/// The executor's prefix enumeration and the destructive freeze's chunk
/// validation both fold keys through this, so "the chunk rows are exactly
/// the frozen enumeration" reduces to digest equality. Each key is hashed
/// with a trailing newline so concatenation cannot alias two streams.
pub struct KeyStreamDigest {
    hasher: Sha256,
    last: Option<String>,
    count: u64,
}

impl Default for KeyStreamDigest {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStreamDigest {
    /// Start an empty stream.
    pub fn new() -> Self {
        Self {
            hasher: Sha256::new(),
            last: None,
            count: 0,
        }
    }

    /// Fold the next key. Keys must arrive strictly ascending — S3
    /// `ListObjectsV2` order — so one out-of-order or duplicate key fails
    /// closed instead of silently producing a different digest.
    pub fn fold(&mut self, key: &str) -> Result<()> {
        if self.last.as_deref().is_some_and(|last| last >= key) {
            return Err(DbError::DeletionSafety(format!(
                "storage key stream is not strictly ascending at {key}"
            )));
        }
        self.hasher.update(key.as_bytes());
        self.hasher.update(b"\n");
        self.last = Some(key.to_owned());
        self.count += 1;
        Ok(())
    }

    /// Hex digest and key count of everything folded.
    pub fn finish(self) -> (String, u64) {
        (hex::encode(self.hasher.finalize()), self.count)
    }
}

/// Full frozen inventory approved at the destructive boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrozenInventory {
    /// PostgreSQL catalog state.
    pub schema: SchemaManifest,
    /// Object-store state.
    pub storage: StorageManifest,
}

impl FrozenInventory {
    /// Canonical JSON bytes and SHA-256 digest used to bind approval.
    pub fn digest(&self) -> Result<Vec<u8>> {
        Ok(Sha256::digest(serde_json::to_vec(self)?).to_vec())
    }
}

/// One durable unit checkpoint.
#[derive(Debug, Clone, Serialize)]
pub struct DeletionCheckpoint {
    /// Stage containing the unit.
    pub stage: String,
    /// Stable unit key.
    pub unit_key: String,
    /// `started`, `completed`, or `failed`.
    pub status: String,
    /// Claim generation that last touched it.
    pub lease_generation: i64,
    /// Attempt count for this unit.
    pub attempts: i32,
    /// Structured bounded details.
    pub detail: serde_json::Value,
    /// Last failure.
    pub error: Option<String>,
    /// Start time.
    pub started_at: DateTime<Utc>,
    /// Completion time.
    pub completed_at: Option<DateTime<Utc>>,
}

/// Full inspect response.
#[derive(Debug, Clone, Serialize)]
pub struct DeletionInspection {
    /// Durable request.
    pub request: DeletionRequest,
    /// Explicit approval evidence, if present.
    pub approval: Option<DeletionApproval>,
    /// Unit checkpoints.
    pub checkpoints: Vec<DeletionCheckpoint>,
}

/// Explicit approval evidence.
#[derive(Debug, Clone, Serialize)]
pub struct DeletionApproval {
    /// Hex frozen inventory digest.
    pub inventory_digest: String,
    /// Approving operator identity.
    pub approved_by: String,
    /// Optional approval note.
    pub note: Option<String>,
    /// Approval timestamp.
    pub approved_at: DateTime<Utc>,
}

/// Monotonic lease token required by every execution mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseToken {
    /// Request id.
    pub request_id: Uuid,
    /// Executor identity.
    pub owner: String,
    /// Monotonic lease generation.
    pub generation: i64,
    /// Target community.
    pub community_id: CommunityId,
    /// Community fence generation, once fenced.
    pub fence_generation: Option<i64>,
}

/// A claimed request with its durable token.
#[derive(Debug, Clone)]
pub struct ClaimedDeletion {
    /// Request snapshot.
    pub request: DeletionRequest,
    /// Required token.
    pub lease: LeaseToken,
}

/// Short-lived durable lease for an external serving side effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServingWriteLease {
    /// Lease row identifier.
    pub id: Uuid,
    /// Community protected by this lease.
    pub community_id: CommunityId,
    /// Operation category for diagnostics.
    pub operation: String,
    /// Process/executor identity.
    pub owner: String,
    /// Monotonic lease generation.
    pub generation: i64,
    /// Community fence generation observed when the lease was acquired.
    pub fence_generation: i64,
    /// Lease expiry.
    pub lease_until: DateTime<Utc>,
}

/// Validate the minimum catalog contract used by serving-path fences.
pub const REQUIRED_SERVING_TABLES: &[&str] = &[
    "communities",
    "community_serving_write_leases",
    "community_deletion_requests",
];

/// Bounded-cardinality operational snapshot for the hot serving-lease table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServingLeaseStats {
    /// Unexpired serving-write leases.
    pub active: i64,
    /// Expired rows awaiting cleanup.
    pub expired: i64,
    /// PostgreSQL's estimated dead tuples for the lease table.
    pub dead_tuples: i64,
}

/// PostgreSQL deletion adapter. Clone is cheap.
#[derive(Clone)]
pub struct DeletionStore {
    pool: PgPool,
}

impl DeletionStore {
    /// Construct from the writer pool used by [`crate::Db`].
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Check deletion control-plane/schema connectivity.
    ///
    /// Probe the deployed catalog rather than SQLx's migration ledger. Buzz also
    /// supports desired-state schema application through `pgschema`, which creates
    /// the same deletion objects without creating `_sqlx_migrations`.
    pub async fn ping(&self) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT to_regclass('community_deletion_requests') IS NOT NULL",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(false)
    }

    /// Persist a request. Only active non-tombstone communities may be submitted.
    pub async fn submit(
        &self,
        community_host: &str,
        requested_by: &str,
        reason: Option<&str>,
    ) -> Result<DeletionRequest> {
        let row = sqlx::query(
            r#"
            WITH target AS (
                SELECT id, host
                FROM communities
                WHERE lower(host) = lower($1)
                  AND deletion_state = 'active'
                  AND deleted_at IS NULL
            ), inserted AS (
                INSERT INTO community_deletion_requests
                    (community_id, community_host, requested_by, reason)
                SELECT id, host, $2, $3 FROM target
                ON CONFLICT (community_id) WHERE stage <> 'aborted' DO NOTHING
                RETURNING *
            )
            SELECT * FROM inserted
            UNION ALL
            SELECT request.*
            FROM community_deletion_requests request
            JOIN target ON target.id = request.community_id
            WHERE request.stage = 'submitted'
              AND request.requested_by = $2
              AND NOT EXISTS (SELECT 1 FROM inserted)
            LIMIT 1
            "#,
        )
        .bind(community_host)
        .bind(requested_by)
        .bind(reason)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => row_to_request(row),
            None => Err(DbError::DeletionSafety(format!(
                "community {community_host:?} is missing, already requested, fenced, or tombstoned"
            ))),
        }
    }

    /// List requests newest first with a hard bound.
    pub async fn list(&self, limit: i64) -> Result<Vec<DeletionRequest>> {
        let rows = sqlx::query(
            "SELECT * FROM community_deletion_requests ORDER BY created_at DESC LIMIT $1",
        )
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_request).collect()
    }

    /// Read one request.
    pub async fn get(&self, request_id: Uuid) -> Result<DeletionRequest> {
        let row = sqlx::query("SELECT * FROM community_deletion_requests WHERE id = $1")
            .bind(request_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("community deletion {request_id}")))?;
        row_to_request(row)
    }

    /// Inspect request, approval, checkpoints, and retention holds.
    pub async fn inspect(&self, request_id: Uuid) -> Result<DeletionInspection> {
        let request = self.get(request_id).await?;
        let approval_row = sqlx::query(
            "SELECT inventory_digest, approved_by, note, approved_at \
             FROM community_deletion_approvals WHERE request_id = $1",
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?;
        let approval = approval_row
            .map(|row| {
                Ok::<DeletionApproval, DbError>(DeletionApproval {
                    inventory_digest: hex::encode(row.try_get::<Vec<u8>, _>("inventory_digest")?),
                    approved_by: row.try_get("approved_by")?,
                    note: row.try_get("note")?,
                    approved_at: row.try_get("approved_at")?,
                })
            })
            .transpose()?;
        let checkpoints = sqlx::query(
            "SELECT stage, unit_key, status, lease_generation, attempts, detail, error, \
                    started_at, completed_at \
             FROM community_deletion_checkpoints WHERE request_id = $1 ORDER BY sequence",
        )
        .bind(request_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            Ok(DeletionCheckpoint {
                stage: row.try_get("stage")?,
                unit_key: row.try_get("unit_key")?,
                status: row.try_get("status")?,
                lease_generation: row.try_get("lease_generation")?,
                attempts: row.try_get("attempts")?,
                detail: row.try_get("detail")?,
                error: row.try_get("error")?,
                started_at: row.try_get("started_at")?,
                completed_at: row.try_get("completed_at")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
        Ok(DeletionInspection {
            request,
            approval,
            checkpoints,
        })
    }

    /// Validate the deletion catalog contract required by relay serving.
    pub async fn validate_serving_catalog(&self) -> Result<()> {
        let runtime_columns = sqlx::query(
            "SELECT attname, format_type(atttypid, atttypmod) AS type_name, attnotnull \
             FROM pg_attribute WHERE attrelid = 'communities'::regclass \
               AND attname IN ('deletion_state', 'deletion_fence_generation', 'deleted_at') \
               AND NOT attisdropped ORDER BY attname",
        )
        .fetch_all(&self.pool)
        .await?;
        let column_contract = runtime_columns
            .iter()
            .map(|row| {
                Ok::<_, DbError>((
                    row.try_get::<String, _>("attname")?,
                    row.try_get::<String, _>("type_name")?,
                    row.try_get::<bool, _>("attnotnull")?,
                ))
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let expected_columns = BTreeSet::from([
            (
                "deleted_at".to_string(),
                "timestamp with time zone".to_string(),
                false,
            ),
            (
                "deletion_fence_generation".to_string(),
                "bigint".to_string(),
                true,
            ),
            ("deletion_state".to_string(), "text".to_string(), true),
        ]);
        if column_contract != expected_columns {
            return Err(DbError::DeletionSafety(
                "community serving fence columns are missing or incompatible".to_string(),
            ));
        }

        let required_tables = REQUIRED_SERVING_TABLES
            .iter()
            .copied()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let required_table_names = REQUIRED_SERVING_TABLES
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let live_tables: BTreeSet<String> = sqlx::query_scalar(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = ANY($1) \
             ORDER BY table_name",
        )
        .bind(&required_table_names)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .collect();
        if live_tables != required_tables {
            return Err(DbError::DeletionSafety(format!(
                "community serving fence tables missing: {}",
                required_tables
                    .difference(&live_tables)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(",")
            )));
        }

        let required_fences = EXPECTED_SCOPED_TABLES
            .iter()
            .copied()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let live_fences = self.live_fenced_tables().await?;
        let missing_fences = required_fences
            .difference(&live_fences)
            .cloned()
            .collect::<Vec<_>>();
        if !missing_fences.is_empty() {
            return Err(DbError::DeletionSafety(format!(
                "community serving write fences missing: {}",
                missing_fences.join(",")
            )));
        }

        let required_objects_present: bool = sqlx::query_scalar(
            "SELECT to_regprocedure('community_deletion_lock_key(uuid)') IS NOT NULL \
                AND to_regprocedure('community_write_allowed(uuid)') IS NOT NULL \
                AND (SELECT provolatile = 'v' FROM pg_proc \
                     WHERE oid = 'community_write_allowed(uuid)'::regprocedure) \
                AND to_regprocedure('assert_community_write_allowed(uuid)') IS NOT NULL \
                AND to_regprocedure('enforce_community_write_fence()') IS NOT NULL \
                AND EXISTS (SELECT 1 FROM pg_trigger t \
                    JOIN pg_class c ON c.oid = t.tgrelid \
                    JOIN pg_proc p ON p.oid = t.tgfoid \
                    WHERE c.relname = 'communities' \
                      AND p.proname = 'enforce_community_tombstone' \
                      AND NOT t.tgisinternal AND t.tgenabled = 'O')",
        )
        .fetch_one(&self.pool)
        .await?;
        if !required_objects_present {
            return Err(DbError::DeletionSafety(
                "community serving fence functions or tombstone trigger are missing".to_string(),
            ));
        }
        Ok(())
    }

    /// Validate the exact live scoped-table and write-fence catalog for destruction.
    ///
    /// Exact table and fence equality rejects unknown tenant data even
    /// while unrelated SQLx migrations continue to advance. This pool-based
    /// check is an early rejection only; destructive transactions revalidate
    /// on their own connection under [`SCHEMA_DESTRUCTION_LOCK_KEY`].
    pub async fn validate_catalog(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        validate_catalog_on(&mut conn).await
    }

    /// Build and validate a live PostgreSQL schema inventory.
    ///
    /// Row counts are observational evidence captured at submission. They bind
    /// operator approval to the target's visible PostgreSQL footprint, but the
    /// executor still revalidates the structural catalog and proves zero rows
    /// after purge rather than requiring these live counts to remain unchanged.
    pub async fn inventory_schema(&self, community: CommunityId) -> Result<SchemaManifest> {
        self.validate_catalog().await?;
        let live_tables = self.live_scoped_tables().await?;
        let fenced_tables = self.live_fenced_tables().await?;
        let mut row_counts = BTreeMap::new();
        for table in &live_tables {
            let sql = format!("SELECT count(*)::BIGINT FROM {table} WHERE community_id = $1");
            let count: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
                .bind(community.as_uuid())
                .fetch_one(&self.pool)
                .await?;
            row_counts.insert(table.clone(), count);
        }
        Ok(SchemaManifest {
            scoped_tables: live_tables.into_iter().collect(),
            row_counts,
            fenced_tables: fenced_tables.into_iter().collect(),
        })
    }

    /// Freeze inventory and move submitted → inventoried atomically.
    pub async fn freeze_inventory(
        &self,
        request_id: Uuid,
        inventory: &FrozenInventory,
    ) -> Result<DeletionRequest> {
        validate_storage_manifest(&inventory.storage)?;
        let digest = inventory.digest()?;
        let schema = serde_json::to_value(&inventory.schema)?;
        let storage = serde_json::to_value(&inventory.storage)?;
        let frozen = serde_json::to_value(inventory)?;
        let row = sqlx::query(
            r#"
            UPDATE community_deletion_requests
            SET stage = 'inventoried', schema_manifest = $2, storage_manifest = $3,
                inventory_manifest = $4, inventory_digest = $5,
                inventory_frozen_at = now(), updated_at = now(),
                last_error = NULL, last_error_at = NULL
            WHERE id = $1 AND stage = 'submitted' AND blocked_at IS NULL
            RETURNING *
            "#,
        )
        .bind(request_id)
        .bind(schema)
        .bind(storage)
        .bind(frozen)
        .bind(digest)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::DeletionSafety(format!(
                "deletion {request_id} is not an unblocked submitted request"
            ))
        })?;
        row_to_request(row)
    }

    /// Approve the exact frozen inventory and move inventoried → approved.
    pub async fn approve(
        &self,
        request_id: Uuid,
        approved_by: &str,
        note: Option<&str>,
    ) -> Result<DeletionRequest> {
        let mut tx = self.pool.begin().await?;
        let (community_id, digest, inventory_manifest): (Uuid, Vec<u8>, serde_json::Value) =
            sqlx::query_as(
                "SELECT community_id, inventory_digest, inventory_manifest \
                 FROM community_deletion_requests \
                 WHERE id = $1 AND stage = 'inventoried' AND blocked_at IS NULL FOR UPDATE",
            )
            .bind(request_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                DbError::DeletionSafety(format!(
                    "deletion {request_id} is not an unblocked inventoried request"
                ))
            })?;
        let inventory: FrozenInventory = serde_json::from_value(inventory_manifest)?;
        let recomputed_digest = inventory.digest()?;
        if digest.as_slice() != recomputed_digest {
            return Err(DbError::DeletionSafety(format!(
                "deletion {request_id} frozen inventory digest does not match its manifest"
            )));
        }
        sqlx::query(
            "INSERT INTO community_deletion_approvals \
             (request_id, community_id, inventory_digest, approved_by, note) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(request_id)
        .bind(community_id)
        .bind(&digest)
        .bind(approved_by)
        .bind(note)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query(
            "UPDATE community_deletion_requests \
             SET stage = 'approved', updated_at = now(), next_attempt_at = now() \
             WHERE id = $1 AND stage = 'inventoried' AND blocked_at IS NULL \
             RETURNING *",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            DbError::DeletionSafety(format!(
                "deletion {request_id} changed before approval could be recorded"
            ))
        })?;
        tx.commit().await?;
        row_to_request(row)
    }

    /// Claim a specific runnable request. Expired claims may be reclaimed.
    pub async fn claim_specific(
        &self,
        request_id: Uuid,
        owner: &str,
        lease_duration: Duration,
    ) -> Result<Option<ClaimedDeletion>> {
        self.claim(Some(request_id), owner, lease_duration).await
    }

    /// Claim the oldest runnable request. Expired claims may be reclaimed.
    pub async fn claim_next(
        &self,
        owner: &str,
        lease_duration: Duration,
    ) -> Result<Option<ClaimedDeletion>> {
        self.claim(None, owner, lease_duration).await
    }

    async fn claim(
        &self,
        request_id: Option<Uuid>,
        owner: &str,
        lease_duration: Duration,
    ) -> Result<Option<ClaimedDeletion>> {
        let lease_seconds = i64::try_from(lease_duration.as_secs()).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await?;
        let candidate = sqlx::query(
            r#"SELECT request.* FROM community_deletion_requests request
            JOIN community_deletion_approvals approval ON approval.request_id = request.id
             AND approval.community_id = request.community_id
             AND approval.inventory_digest = request.inventory_digest
            WHERE ($1::uuid IS NULL OR request.id = $1)
              AND request.stage IN ('approved', 'fenced', 'drained', 'bindings_removed',
                                    'postgres_purged', 'cache_purged', 'logically_verified')
              AND request.blocked_at IS NULL AND request.next_attempt_at <= now()
              AND (request.lease_until IS NULL OR request.lease_until < now())
            ORDER BY request.created_at, request.id
            FOR UPDATE OF request SKIP LOCKED LIMIT 1"#,
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(candidate_row) = candidate else {
            tx.commit().await?;
            return Ok(None);
        };
        let candidate = row_to_request(candidate_row)?;
        if let Err(error) = validate_catalog_on(&mut tx).await {
            let message = bound_text(&error.to_string(), 4096);
            sqlx::query(
                r#"INSERT INTO community_deletion_checkpoints
                    (request_id, stage, unit_key, status, lease_generation, error)
                VALUES ($1, $2, 'claim:catalog_validation', 'failed', $3, $4)
                ON CONFLICT (request_id, stage, unit_key) DO UPDATE
                SET status = 'failed', lease_generation = EXCLUDED.lease_generation,
                    attempts = community_deletion_checkpoints.attempts + 1,
                    error = EXCLUDED.error, completed_at = NULL"#,
            )
            .bind(candidate.id)
            .bind(candidate.stage.to_string())
            .bind(candidate.lease_generation.max(1))
            .bind(&message)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE community_deletion_requests SET blocked_at = now(), blocked_reason = $2, \
                 last_error = $2, last_error_at = now(), lease_owner = NULL, lease_until = NULL, \
                 updated_at = now() WHERE id = $1",
            )
            .bind(candidate.id)
            .bind(&message)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(None);
        }
        let row = sqlx::query(
            "UPDATE community_deletion_requests SET lease_owner = $2, \
             lease_generation = lease_generation + 1, \
             lease_until = now() + make_interval(secs => $3), attempts = attempts + 1, \
             updated_at = now() WHERE id = $1 RETURNING *",
        )
        .bind(candidate.id)
        .bind(owner)
        .bind(lease_seconds)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        let request = row_to_request(row)?;
        let lease = LeaseToken {
            request_id: request.id,
            owner: owner.to_owned(),
            generation: request.lease_generation,
            community_id: request.community_id,
            fence_generation: request.fence_generation,
        };
        Ok(Some(ClaimedDeletion { request, lease }))
    }

    /// Verify that a deletion lease/fence token is still current for a stage.
    pub async fn verify_execution_token(
        &self,
        token: &LeaseToken,
        stage: DeletionStage,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        if let Some(generation) = token.fence_generation {
            verify_lease_and_fence(&mut tx, token, stage, generation).await?;
        } else {
            verify_lease(&mut tx, token, stage).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Renew an owned claim and persist executor liveness.
    pub async fn heartbeat(
        &self,
        token: &LeaseToken,
        executor_mode: &str,
        lease_duration: Duration,
        draining: bool,
    ) -> Result<()> {
        let lease_seconds = i64::try_from(lease_duration.as_secs()).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await?;
        let affected = sqlx::query(
            "UPDATE community_deletion_requests request \
             SET lease_until = now() + make_interval(secs => $4), updated_at = now() \
             WHERE request.id = $1 AND request.lease_owner = $2 \
               AND request.lease_generation = $3 AND request.lease_until >= now() \
               AND request.blocked_at IS NULL \
               AND request.stage IN ('approved', 'fenced', 'drained', 'bindings_removed', \
                                      'postgres_purged', 'cache_purged', 'logically_verified') \
               AND EXISTS (SELECT 1 FROM community_deletion_approvals approval \
                   WHERE approval.request_id = request.id \
                     AND approval.community_id = request.community_id \
                     AND approval.inventory_digest = request.inventory_digest)",
        )
        .bind(token.request_id)
        .bind(&token.owner)
        .bind(token.generation)
        .bind(lease_seconds)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(stale_lease_error(token));
        }
        sqlx::query(
            "INSERT INTO community_deletion_executor_heartbeats \
             (executor_id, mode, request_id, draining) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (executor_id) DO UPDATE SET mode = EXCLUDED.mode, \
                 request_id = EXCLUDED.request_id, heartbeat_at = now(), \
                 draining = EXCLUDED.draining, stopped_at = NULL",
        )
        .bind(&token.owner)
        .bind(executor_mode)
        .bind(token.request_id)
        .bind(draining)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Mark an executor stopped and release its current claim if still owned.
    pub async fn stop_executor(&self, token: Option<&LeaseToken>, executor_id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        if let Some(token) = token {
            sqlx::query(
                "UPDATE community_deletion_requests \
                 SET lease_owner = NULL, lease_until = NULL, updated_at = now() \
                 WHERE id = $1 AND lease_owner = $2 AND lease_generation = $3",
            )
            .bind(token.request_id)
            .bind(&token.owner)
            .bind(token.generation)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE community_deletion_executor_heartbeats \
             SET request_id = NULL, draining = true, heartbeat_at = now(), stopped_at = now() \
             WHERE executor_id = $1",
        )
        .bind(executor_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Persist quiescing intent before waiting for active serving leases.
    ///
    /// This is the irreversible fail-closed point: a request intentionally has
    /// no automatic unquiesce/unblock transition after operator approval.
    ///
    /// The transition takes the same exclusive advisory lock as serving lease
    /// acquisition, so after commit no newer external effect can be admitted.
    /// Already-acquired leases remain renewable, verifiable, and releasable so
    /// admitted remote effects retain their exclusion proof until completion.
    pub async fn begin_quiescing(&self, token: &LeaseToken) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        verify_lease(&mut tx, token, DeletionStage::Approved).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(token.community_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        verify_lease(&mut tx, token, DeletionStage::Approved).await?;
        let (generation, archived_at): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT deletion_fence_generation, archived_at FROM communities WHERE id = $1 FOR UPDATE",
        )
        .bind(token.community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE community_deletion_requests SET pre_quiesce_archived_at = $2, \
                    quiescing_started_at = now(), updated_at = now() \
             WHERE id = $1 AND quiescing_started_at IS NULL",
        )
        .bind(token.request_id)
        .bind(archived_at)
        .execute(&mut *tx)
        .await?;
        set_executor_gucs(&mut tx, token.community_id, generation).await?;
        let affected = sqlx::query(
            "UPDATE communities SET deletion_state = 'quiescing', \
                    archived_at = COALESCE(archived_at, now()) \
             WHERE id = $1 AND deletion_state IN ('active', 'quiescing') \
               AND deleted_at IS NULL",
        )
        .bind(token.community_id.as_uuid())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(DbError::DeletionSafety(format!(
                "community {} cannot enter quiescing",
                token.community_id
            )));
        }
        checkpoint_completed_tx(
            &mut tx,
            token,
            DeletionStage::Approved,
            "quiesce_serving_writes",
            serde_json::json!({"community_state": "quiescing"}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Acquire the universal durable fence after all pre-quiesce serving leases drain.
    pub async fn fence(&self, token: &LeaseToken) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        verify_lease(&mut tx, token, DeletionStage::Approved).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(token.community_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        verify_lease(&mut tx, token, DeletionStage::Approved).await?;
        let active_serving_writes = sqlx::query(
            "SELECT count(*)::BIGINT AS active_count, \
                    COALESCE(array_agg(DISTINCT operation ORDER BY operation), ARRAY[]::TEXT[]) AS operations \
             FROM community_serving_write_leases \
             WHERE community_id = $1 AND lease_until >= now()",
        )
        .bind(token.community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        let active_count: i64 = active_serving_writes.try_get("active_count")?;
        if active_count > 0 {
            return Err(DbError::ServingWritesNotDrained {
                community_id: *token.community_id.as_uuid(),
                active_count,
                operations: active_serving_writes.try_get("operations")?,
            });
        }
        let current_generation: i64 = sqlx::query_scalar(
            "SELECT deletion_fence_generation FROM communities WHERE id = $1 FOR UPDATE",
        )
        .bind(token.community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        let generation = current_generation.checked_add(1).ok_or_else(|| {
            DbError::DeletionSafety("community deletion fence generation overflow".to_string())
        })?;
        set_executor_gucs(&mut tx, token.community_id, generation).await?;
        let affected = sqlx::query(
            "UPDATE communities SET deletion_state = 'fenced', \
                    deletion_fence_generation = $2, archived_at = COALESCE(archived_at, now()) \
             WHERE id = $1 AND deletion_state = 'quiescing'",
        )
        .bind(token.community_id.as_uuid())
        .bind(generation)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(DbError::DeletionSafety(format!(
                "community {} is no longer quiescing while fencing",
                token.community_id
            )));
        }
        advance_request_tx(
            &mut tx,
            token,
            DeletionStage::Approved,
            DeletionStage::Fenced,
            Some(generation),
        )
        .await?;
        checkpoint_completed_tx(
            &mut tx,
            token,
            DeletionStage::Approved,
            "activate_fence",
            serde_json::json!({"fence_generation": generation}),
        )
        .await?;
        tx.commit().await?;
        Ok(generation)
    }

    /// Freeze the exact post-fence storage binding manifest.
    pub async fn freeze_destructive_storage_manifest(
        &self,
        token: &LeaseToken,
        manifest: &StorageManifest,
    ) -> Result<()> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        // Serialize the freeze boundary with chunk INSERTs. The database trigger
        // takes the same request-row lock before admitting each new chunk.
        sqlx::query("SELECT id FROM community_deletion_requests WHERE id = $1 FOR UPDATE")
            .bind(token.request_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                DbError::DeletionSafety(format!(
                    "deletion request {} disappeared before manifest freeze",
                    token.request_id
                ))
            })?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::Fenced, generation).await?;
        validate_storage_manifest(manifest)?;
        // The chunk rows are the concrete delete list; the freeze commits only
        // if they hash to the manifest's frozen per-prefix digests. Loading the
        // full chunk stream is a one-time freeze-boundary cost proportional to
        // this community's bindings, never the fleet bucket.
        let chunks: Vec<(i64, String, sqlx::types::Json<Vec<String>>)> = sqlx::query_as(
            "SELECT chunk_no, prefix, keys FROM community_deletion_manifest_keys \
             WHERE request_id = $1 ORDER BY chunk_no",
        )
        .bind(token.request_id)
        .fetch_all(&mut *tx)
        .await?;
        validate_manifest_key_chunks(manifest, &chunks)?;
        let affected = sqlx::query(
            "UPDATE community_deletion_requests \
             SET destructive_storage_manifest = COALESCE(destructive_storage_manifest, $4), \
                 destructive_storage_frozen_at = COALESCE(destructive_storage_frozen_at, now()), \
                 updated_at = now() \
             WHERE id = $1 AND lease_owner = $2 AND lease_generation = $3 \
               AND stage = 'fenced' \
               AND (destructive_storage_manifest IS NULL \
                    OR destructive_storage_manifest = $4) \
             RETURNING id",
        )
        .bind(token.request_id)
        .bind(&token.owner)
        .bind(token.generation)
        .bind(serde_json::to_value(manifest)?)
        .fetch_optional(&mut *tx)
        .await?;
        if affected.is_none() {
            return Err(DbError::DeletionSafety(format!(
                "destructive storage manifest changed or deletion lease is stale for request {}",
                token.request_id
            )));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Remove key chunks left by an interrupted destructive freeze.
    ///
    /// The chunk-table guard rejects this once the destructive manifest has
    /// frozen, so a retried freeze can only rewrite chunks that were never
    /// bound to a committed manifest.
    pub async fn clear_manifest_key_chunks(&self, token: &LeaseToken) -> Result<()> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::Fenced, generation).await?;
        sqlx::query("DELETE FROM community_deletion_manifest_keys WHERE request_id = $1")
            .bind(token.request_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Append one immutable chunk of the destructive key list.
    pub async fn append_manifest_key_chunk(
        &self,
        token: &LeaseToken,
        chunk_no: i64,
        prefix: &str,
        keys: &[String],
    ) -> Result<()> {
        if keys.is_empty() {
            return Err(DbError::DeletionSafety(
                "refusing to persist an empty manifest key chunk".to_string(),
            ));
        }
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::Fenced, generation).await?;
        sqlx::query(
            "INSERT INTO community_deletion_manifest_keys \
             (request_id, chunk_no, prefix, keys) VALUES ($1, $2, $3, $4)",
        )
        .bind(token.request_id)
        .bind(chunk_no)
        .bind(prefix)
        .bind(sqlx::types::Json(keys))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Return the next frozen chunk not yet confirmed deleted, in chunk order.
    pub async fn next_pending_manifest_chunk(
        &self,
        token: &LeaseToken,
    ) -> Result<Option<ManifestKeyChunk>> {
        let row: Option<(i64, String, sqlx::types::Json<Vec<String>>)> = sqlx::query_as(
            "SELECT chunk_no, prefix, keys FROM community_deletion_manifest_keys \
             WHERE request_id = $1 AND deleted_at IS NULL ORDER BY chunk_no LIMIT 1",
        )
        .bind(token.request_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(chunk_no, prefix, keys)| ManifestKeyChunk {
            chunk_no,
            prefix,
            keys: keys.0,
        }))
    }

    /// Return `(total, deleted)` chunk counts for one request.
    pub async fn manifest_chunk_progress(&self, request_id: Uuid) -> Result<(i64, i64)> {
        sqlx::query_as(
            "SELECT count(*), count(deleted_at) FROM community_deletion_manifest_keys \
             WHERE request_id = $1",
        )
        .bind(request_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Stamp one chunk's keys durably removed and checkpoint it atomically.
    pub async fn mark_manifest_chunk_deleted(
        &self,
        token: &LeaseToken,
        chunk_no: i64,
        detail: serde_json::Value,
    ) -> Result<()> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::Drained, generation).await?;
        let affected = sqlx::query(
            "UPDATE community_deletion_manifest_keys SET deleted_at = now() \
             WHERE request_id = $1 AND chunk_no = $2 AND deleted_at IS NULL",
        )
        .bind(token.request_id)
        .bind(chunk_no)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(DbError::DeletionSafety(format!(
                "manifest key chunk {chunk_no} is missing or already stamped for request {}",
                token.request_id
            )));
        }
        checkpoint_completed_tx(
            &mut tx,
            token,
            DeletionStage::Drained,
            &format!("chunk:{chunk_no}"),
            detail,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Record one completed fleet-wide taxonomy sweep.
    pub async fn record_taxonomy_sweep(
        &self,
        started_at: DateTime<Utc>,
        listed_objects: u64,
        unknown_object_count: u64,
        unknown_key_sample: &[String],
        object_cap: u64,
    ) -> Result<TaxonomySweep> {
        let listed = i64::try_from(listed_objects)
            .map_err(|_| DbError::DeletionSafety("sweep object count overflow".to_string()))?;
        let unknown = i64::try_from(unknown_object_count)
            .map_err(|_| DbError::DeletionSafety("sweep unknown count overflow".to_string()))?;
        let cap = i64::try_from(object_cap)
            .map_err(|_| DbError::DeletionSafety("sweep object cap overflow".to_string()))?;
        // Completion is authoritative database time. Small positive sweeper
        // skew is clamped at that boundary; materially future starts are rejected.
        let row: Option<(Uuid, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
            "INSERT INTO storage_taxonomy_sweeps \
             (started_at, completed_at, listed_objects, unknown_object_count, \
              unknown_key_sample, object_cap) \
             SELECT LEAST($1, db_now), db_now, $2, $3, $4, $5 \
             FROM (SELECT clock_timestamp() AS db_now) clock \
             WHERE $1 <= db_now + interval '5 minutes' \
             RETURNING id, started_at, completed_at",
        )
        .bind(started_at)
        .bind(listed)
        .bind(unknown)
        .bind(sqlx::types::Json(unknown_key_sample))
        .bind(cap)
        .fetch_optional(&self.pool)
        .await?;
        let (id, started_at, completed_at) = row.ok_or_else(|| {
            DbError::DeletionSafety(
                "taxonomy sweep start time is more than five minutes in the future".to_string(),
            )
        })?;
        Ok(TaxonomySweep {
            id,
            started_at,
            completed_at,
            listed_objects: listed,
            unknown_object_count: unknown,
            unknown_key_sample: unknown_key_sample.to_vec(),
            object_cap: cap,
        })
    }

    /// Return the most recently completed taxonomy sweep, if any.
    pub async fn latest_taxonomy_sweep(&self) -> Result<Option<TaxonomySweep>> {
        let row: Option<TaxonomySweepRow> = sqlx::query_as(
            "SELECT id, started_at, completed_at, listed_objects, unknown_object_count, \
                    unknown_key_sample, object_cap \
             FROM storage_taxonomy_sweeps ORDER BY completed_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(id, started_at, completed_at, listed, unknown, sample, cap)| TaxonomySweep {
                id,
                started_at,
                completed_at,
                listed_objects: listed,
                unknown_object_count: unknown,
                unknown_key_sample: sample.0,
                object_cap: cap,
            },
        ))
    }

    /// Return whether all pre-fence external side-effect leases have expired or released.
    pub async fn serving_writes_drained(&self, community: CommunityId) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT 1 FROM community_serving_write_leases \
             WHERE community_id = $1 AND lease_until >= now())",
        )
        .bind(community.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Verify fence ownership and record that serving writes drained.
    pub async fn mark_drained(&self, token: &LeaseToken) -> Result<()> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::Fenced, generation).await?;
        let active_serving_writes: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_serving_write_leases \
             WHERE community_id = $1 AND lease_until >= now())",
        )
        .bind(token.community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        if active_serving_writes {
            return Err(DbError::DeletionSafety(
                "serving writes have not drained".to_string(),
            ));
        }
        advance_request_tx(
            &mut tx,
            token,
            DeletionStage::Fenced,
            DeletionStage::Drained,
            Some(generation),
        )
        .await?;
        checkpoint_completed_tx(
            &mut tx,
            token,
            DeletionStage::Fenced,
            "serving_writes_drained",
            serde_json::json!({"fence_generation": generation}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Mark storage binding removal after adapter verification.
    pub async fn mark_bindings_removed(
        &self,
        token: &LeaseToken,
        detail: serde_json::Value,
    ) -> Result<()> {
        self.advance_with_checkpoint(
            token,
            DeletionStage::Drained,
            DeletionStage::BindingsRemoved,
            "remove_storage_bindings",
            detail,
        )
        .await
    }

    /// Purge every scoped PostgreSQL table, preserve the community tombstone, and
    /// move bindings_removed → postgres_purged in one transaction.
    pub async fn purge_postgres(&self, token: &LeaseToken) -> Result<BTreeMap<String, u64>> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        // Revalidate the exact catalog inside the purge transaction under the
        // shared schema/destruction lock. Migrations hold the exclusive
        // counterpart for their entire run, so no migration can commit a new
        // scoped table between this validation and the purge commit.
        lock_schema_destruction_shared(&mut tx).await?;
        validate_catalog_on(&mut tx).await?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::BindingsRemoved, generation).await?;
        set_executor_gucs(&mut tx, token.community_id, generation).await?;
        // Migration 0011 fences hard deletion of NIP-RS rows against legacy
        // writers. Whole-community deletion is an intentional hard-delete path,
        // and the transaction is already bound to an approved, fenced tenant.
        sqlx::query("SELECT set_config('buzz.nip_rs_hard_delete', 'on', true)")
            .execute(&mut *tx)
            .await?;

        // Preserve deployment-global operator evidence while severing tenant provenance.
        for table in ["product_feedback", "rate_limit_violations"] {
            let sql = format!("UPDATE {table} SET community_id = NULL WHERE community_id = $1");
            let affected = sqlx::query(AssertSqlSafe(sql))
                .bind(token.community_id.as_uuid())
                .execute(&mut *tx)
                .await?
                .rows_affected();
            checkpoint_completed_tx(
                &mut tx,
                token,
                DeletionStage::BindingsRemoved,
                &format!("clear_provenance:{table}"),
                serde_json::json!({"rows": affected}),
            )
            .await?;
        }

        let mut deleted = BTreeMap::new();
        // The order is child-before-parent/FK-safe, not alphabetical. Cascades
        // can make later units observe zero rows; each scoped WHERE stays idempotent.
        for table in PURGE_SCOPED_TABLES {
            let sql = format!("DELETE FROM {table} WHERE community_id = $1");
            let affected = sqlx::query(AssertSqlSafe(sql))
                .bind(token.community_id.as_uuid())
                .execute(&mut *tx)
                .await?
                .rows_affected();
            deleted.insert((*table).to_owned(), affected);
            checkpoint_completed_tx(
                &mut tx,
                token,
                DeletionStage::BindingsRemoved,
                &format!("purge:{table}"),
                serde_json::json!({"rows": affected}),
            )
            .await?;
        }

        let affected = sqlx::query(
            "UPDATE communities SET deletion_state = 'tombstone', \
                    deleted_at = COALESCE(deleted_at, now()), \
                    archived_at = COALESCE(archived_at, now()), \
                    signing_key = NULL, icon = NULL \
             WHERE id = $1 AND deletion_state = 'fenced' \
               AND deletion_fence_generation = $2",
        )
        .bind(token.community_id.as_uuid())
        .bind(generation)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(DbError::DeletionSafety(format!(
                "community {} tombstone update affected {affected} rows",
                token.community_id
            )));
        }
        advance_request_tx(
            &mut tx,
            token,
            DeletionStage::BindingsRemoved,
            DeletionStage::PostgresPurged,
            Some(generation),
        )
        .await?;
        checkpoint_completed_tx(
            &mut tx,
            token,
            DeletionStage::BindingsRemoved,
            "postgres_tombstone_committed",
            serde_json::to_value(&deleted)?,
        )
        .await?;
        tx.commit().await?;
        Ok(deleted)
    }

    /// Mark cache purge after Redis adapter verification.
    pub async fn mark_cache_purged(
        &self,
        token: &LeaseToken,
        detail: serde_json::Value,
    ) -> Result<()> {
        self.advance_with_checkpoint(
            token,
            DeletionStage::PostgresPurged,
            DeletionStage::CachePurged,
            "purge_cache_namespace",
            detail,
        )
        .await
    }

    /// Verify PostgreSQL logical absence without advancing the cross-store stage.
    ///
    /// The caller must verify object storage and Redis too, then call
    /// [`Self::mark_logically_verified`]. Keeping the transition separate makes
    /// a crash after any partial verification safely repeat the whole proof.
    pub async fn verify_postgres_logically_deleted(&self, token: &LeaseToken) -> Result<()> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        // The absence proof is only as strong as the surface it iterates:
        // validate the live catalog under the shared schema/destruction lock
        // so a scoped table committed after the purge fails this stage closed
        // instead of silently escaping verification.
        lock_schema_destruction_shared(&mut tx).await?;
        validate_catalog_on(&mut tx).await?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::CachePurged, generation).await?;
        let tombstone: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM communities WHERE id = $1 \
             AND deletion_state = 'tombstone' AND deleted_at IS NOT NULL \
             AND deletion_fence_generation = $2)",
        )
        .bind(token.community_id.as_uuid())
        .bind(generation)
        .fetch_one(&mut *tx)
        .await?;
        if !tombstone {
            return Err(DbError::DeletionSafety(format!(
                "community {} tombstone/fence verification failed",
                token.community_id
            )));
        }
        for table in EXPECTED_SCOPED_TABLES {
            let sql =
                format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE community_id = $1 LIMIT 1)");
            let remains: bool = sqlx::query_scalar(AssertSqlSafe(sql))
                .bind(token.community_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
            if remains {
                return Err(DbError::DeletionSafety(format!(
                    "logical verification found tenant rows in {table}"
                )));
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Commit the cross-store logical verification checkpoint and drop the
    /// frozen key chunks in the same transaction.
    ///
    /// The chunk rows are working data, not audit evidence — per-prefix
    /// counts, digests, and checkpoint history stay on the request row, and
    /// the raw key list of a deleted community should not be retained.
    /// Blocked requests never reach this transition, so their chunks survive
    /// for resumption or operator inspection.
    pub async fn mark_logically_verified(
        &self,
        token: &LeaseToken,
        detail: serde_json::Value,
    ) -> Result<()> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::CachePurged, generation).await?;
        advance_request_tx(
            &mut tx,
            token,
            DeletionStage::CachePurged,
            DeletionStage::LogicallyVerified,
            Some(generation),
        )
        .await?;
        checkpoint_completed_tx(
            &mut tx,
            token,
            DeletionStage::CachePurged,
            "verify_cross_store_absence",
            detail,
        )
        .await?;
        sqlx::query("DELETE FROM community_deletion_manifest_keys WHERE request_id = $1")
            .bind(token.request_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Finish logical deletion and enter the physical-expiry pending state.
    pub async fn mark_retention_pending(
        &self,
        token: &LeaseToken,
        detail: serde_json::Value,
    ) -> Result<()> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        verify_lease_and_fence(&mut tx, token, DeletionStage::LogicallyVerified, generation)
            .await?;
        checkpoint_completed_tx(
            &mut tx,
            token,
            DeletionStage::LogicallyVerified,
            "retention_physical_expiry_pending",
            detail,
        )
        .await?;
        let affected = sqlx::query(
            "UPDATE community_deletion_requests \
             SET stage = 'retention_pending', completed_at = now(), updated_at = now(), \
                 lease_owner = NULL, lease_until = NULL, retry_count = 0, retry_stage = NULL, \
                 last_error = NULL, last_error_at = NULL \
             WHERE id = $1 AND stage = 'logically_verified' \
               AND lease_owner = $2 AND lease_generation = $3 AND lease_until >= now() \
               AND fence_generation = $4",
        )
        .bind(token.request_id)
        .bind(&token.owner)
        .bind(token.generation)
        .bind(generation)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(stale_lease_error(token));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Persist a retryable unit failure and release the claim.
    ///
    /// The eighth consecutive failure at the same stage becomes a durable
    /// block. A successful stage transition clears the streak; an operator may
    /// use [`Self::unblock`] after remediating an exhausted dependency failure.
    pub async fn record_retry(
        &self,
        token: &LeaseToken,
        stage: DeletionStage,
        unit_key: &str,
        error: &str,
        retry_after: Duration,
    ) -> Result<()> {
        let bounded = bound_text(error, 4096);
        let retry_seconds = i64::try_from(retry_after.as_secs()).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await?;
        verify_lease(&mut tx, token, stage).await?;
        let (retry_count, retry_stage): (i32, Option<String>) = sqlx::query_as(
            "SELECT retry_count, retry_stage FROM community_deletion_requests WHERE id = $1 FOR UPDATE",
        )
        .bind(token.request_id)
        .fetch_one(&mut *tx)
        .await?;
        let stage_name = stage.to_string();
        let consecutive_retries = if retry_stage.as_deref() == Some(stage_name.as_str()) {
            retry_count.saturating_add(1)
        } else {
            1
        };
        let exhausted = consecutive_retries >= 8;
        checkpoint_failed_tx(&mut tx, token, stage, unit_key, &bounded).await?;
        sqlx::query(
            "UPDATE community_deletion_requests \
             SET retry_count = $7, retry_stage = $8, last_error = $4, last_error_at = now(), \
                 next_attempt_at = CASE WHEN $6 THEN next_attempt_at \
                                        ELSE now() + make_interval(secs => $5) END, \
                 blocked_at = CASE WHEN $6 THEN now() ELSE blocked_at END, \
                 blocked_reason = CASE WHEN $6 THEN $4 ELSE blocked_reason END, \
                 lease_owner = NULL, lease_until = NULL, updated_at = now() \
             WHERE id = $1 AND lease_owner = $2 AND lease_generation = $3",
        )
        .bind(token.request_id)
        .bind(&token.owner)
        .bind(token.generation)
        .bind(&bounded)
        .bind(retry_seconds)
        .bind(exhausted)
        .bind(consecutive_retries)
        .bind(stage_name)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Terminally abort an approved or fenced request before object deletion begins.
    pub async fn abort(
        &self,
        request_id: Uuid,
        aborted_by: &str,
        reason: &str,
    ) -> Result<DeletionRequest> {
        let aborted_by = aborted_by.trim();
        let reason = reason.trim();
        if aborted_by.is_empty() || reason.is_empty() {
            return Err(DbError::DeletionSafety(
                "abort requires non-empty operator identity and reason".to_string(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let community_id: CommunityId = sqlx::query_scalar::<_, Uuid>(
            "SELECT community_id FROM community_deletion_requests WHERE id = $1",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(CommunityId::from_uuid)
        .ok_or_else(|| DbError::NotFound(format!("community deletion {request_id}")))?;
        // Every lifecycle transition takes the community lock before any row lock.
        // Inverting this order lets abort and the executor deadlock each other.
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(community_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT * FROM community_deletion_requests WHERE id = $1 FOR UPDATE")
            .bind(request_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("community deletion {request_id}")))?;
        let request = row_to_request(row)?;
        if request.community_id != community_id {
            return Err(DbError::DeletionSafety(format!(
                "deletion {request_id} changed community while abort waited for the community lock"
            )));
        }
        if !matches!(
            request.stage,
            DeletionStage::Approved | DeletionStage::Fenced
        ) {
            return Err(DbError::DeletionSafety(format!(
                "deletion {request_id} at stage {} cannot be aborted",
                request.stage
            )));
        }
        let active_writes: i64 = sqlx::query_scalar(
            "SELECT count(*)::BIGINT FROM community_serving_write_leases \
             WHERE community_id = $1 AND lease_until >= now()",
        )
        .bind(request.community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        if active_writes > 0 {
            return Err(DbError::DeletionSafety(format!(
                "deletion {request_id} cannot abort while {active_writes} serving write lease(s) remain active"
            )));
        }
        let (old_generation, current_archived_at): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT deletion_fence_generation, archived_at FROM communities WHERE id = $1 FOR UPDATE",
        )
        .bind(request.community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        let new_generation = old_generation.checked_add(1).ok_or_else(|| {
            DbError::DeletionSafety("community deletion fence generation overflow".to_string())
        })?;
        let restored_archived_at = if request.quiescing_started_at.is_some() {
            request.pre_quiesce_archived_at
        } else {
            current_archived_at
        };
        set_executor_gucs(&mut tx, request.community_id, new_generation).await?;
        let restored = sqlx::query(
            "UPDATE communities SET deletion_state = 'active', deletion_fence_generation = $2, \
             archived_at = $3 WHERE id = $1 AND deletion_state IN ('active', 'quiescing', 'fenced') \
             AND deleted_at IS NULL",
        )
        .bind(request.community_id.as_uuid())
        .bind(new_generation)
        .bind(restored_archived_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if restored != 1 {
            return Err(DbError::DeletionSafety(format!(
                "community {} cannot be restored during abort",
                request.community_id
            )));
        }
        sqlx::query(
            "INSERT INTO community_deletion_checkpoints \
             (request_id, stage, unit_key, status, lease_generation, detail, completed_at) \
             VALUES ($1, $2, $3, 'completed', $4, $5, now())",
        )
        .bind(request.id)
        .bind(request.stage.to_string())
        .bind(format!("operator_abort:{}", Uuid::new_v4()))
        .bind(request.lease_generation.max(1))
        .bind(serde_json::json!({
            "aborted_by": bound_text(aborted_by, 512),
            "reason": bound_text(reason, 4096),
            "old_fence_generation": old_generation,
            "new_fence_generation": new_generation,
        }))
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query(
            "UPDATE community_deletion_requests SET stage = 'aborted', aborted_by = $2, \
             abort_reason = $3, aborted_at = now(), completed_at = now(), fence_generation = $4, \
             lease_owner = NULL, lease_until = NULL, lease_generation = lease_generation + 1, \
             blocked_at = NULL, blocked_reason = NULL, retry_count = 0, retry_stage = NULL, \
             next_attempt_at = now(), updated_at = now() WHERE id = $1 RETURNING *",
        )
        .bind(request.id)
        .bind(bound_text(aborted_by, 512))
        .bind(bound_text(reason, 4096))
        .bind(new_generation)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        row_to_request(row)
    }

    /// operator checkpoint and only makes runnable stages immediately claimable.
    /// Clear a fail-closed block after an operator has remediated its cause.
    ///
    /// Recovery preserves the immutable target, approval, inventory, stage,
    /// fence generation, and prior failure checkpoint. It appends an auditable
    pub async fn unblock(
        &self,
        request_id: Uuid,
        unblocked_by: &str,
        reason: &str,
    ) -> Result<DeletionRequest> {
        let unblocked_by = unblocked_by.trim();
        let reason = reason.trim();
        if unblocked_by.is_empty() || reason.is_empty() {
            return Err(DbError::InvalidData(
                "unblock requires non-empty operator identity and remediation reason".to_string(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        let request_row = sqlx::query(
            "SELECT * FROM community_deletion_requests \
             WHERE id = $1 AND blocked_at IS NOT NULL FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            DbError::DeletionSafety(format!(
                "deletion {request_id} is missing or is not blocked"
            ))
        })?;
        let request = row_to_request(request_row)?;
        if matches!(
            request.stage,
            DeletionStage::RetentionPending | DeletionStage::Aborted
        ) {
            return Err(DbError::DeletionSafety(format!(
                "blocked deletion {request_id} at terminal stage {} cannot resume",
                request.stage
            )));
        }
        if request.lease_owner.is_some()
            && request
                .lease_until
                .is_some_and(|lease_until| lease_until >= Utc::now())
        {
            return Err(DbError::DeletionSafety(format!(
                "blocked deletion {request_id} still has a live executor lease"
            )));
        }

        let prior_block = request.blocked_reason.clone();
        sqlx::query(
            "INSERT INTO community_deletion_checkpoints \
             (request_id, stage, unit_key, status, lease_generation, detail, completed_at) \
             VALUES ($1, $2, $3, 'completed', $4, $5, now())",
        )
        .bind(request.id)
        .bind(request.stage.to_string())
        .bind(format!("operator_unblock:{}", Uuid::new_v4()))
        .bind(request.lease_generation.max(1))
        .bind(serde_json::json!({
            "unblocked_by": bound_text(unblocked_by, 512),
            "reason": bound_text(reason, 4096),
            "previous_block": prior_block,
        }))
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query(
            "UPDATE community_deletion_requests \
             SET blocked_at = NULL, blocked_reason = NULL, retry_count = 0, retry_stage = NULL, \
                 last_error = NULL, last_error_at = NULL, next_attempt_at = now(), \
                 lease_owner = NULL, lease_until = NULL, updated_at = now() \
             WHERE id = $1 AND blocked_at IS NOT NULL RETURNING *",
        )
        .bind(request_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        row_to_request(row)
    }

    /// Persist a fail-closed setup failure before an identifiable request is claimed.
    pub async fn block_preclaim_setup(
        &self,
        request_id: Uuid,
        unit_key: &str,
        error: &str,
    ) -> Result<DeletionRequest> {
        let bounded = bound_text(error, 4096);
        let mut tx = self.pool.begin().await?;
        let request =
            sqlx::query("SELECT * FROM community_deletion_requests WHERE id = $1 FOR UPDATE")
                .bind(request_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| DbError::NotFound(format!("community deletion {request_id}")))?;
        let request = row_to_request(request)?;
        if matches!(
            request.stage,
            DeletionStage::RetentionPending | DeletionStage::Aborted
        ) {
            return Err(DbError::DeletionSafety(format!(
                "deletion {request_id} at terminal stage {} cannot record a setup failure",
                request.stage
            )));
        }
        if request.lease_owner.is_some()
            && request
                .lease_until
                .is_some_and(|lease_until| lease_until >= Utc::now())
        {
            return Err(DbError::DeletionSafety(format!(
                "deletion {request_id} is leased by another executor"
            )));
        }
        sqlx::query(
            r#"
            INSERT INTO community_deletion_checkpoints
                (request_id, stage, unit_key, status, lease_generation, detail, error)
            VALUES ($1, $2, $3, 'failed', $4, $5, $6)
            ON CONFLICT (request_id, stage, unit_key) DO UPDATE
            SET status = 'failed', lease_generation = EXCLUDED.lease_generation,
                attempts = community_deletion_checkpoints.attempts + 1,
                detail = EXCLUDED.detail, error = EXCLUDED.error, completed_at = NULL
            "#,
        )
        .bind(request.id)
        .bind(request.stage.to_string())
        .bind(unit_key)
        .bind(request.lease_generation.max(1))
        .bind(serde_json::json!({"error": &bounded}))
        .bind(&bounded)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query(
            "UPDATE community_deletion_requests \
             SET blocked_at = now(), blocked_reason = $2, last_error = $2, \
                 last_error_at = now(), updated_at = now() \
             WHERE id = $1 RETURNING *",
        )
        .bind(request_id)
        .bind(&bounded)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        row_to_request(row)
    }

    /// Persist a fail-closed permanent block and release the claim.
    pub async fn block(
        &self,
        token: &LeaseToken,
        stage: DeletionStage,
        unit_key: &str,
        error: &str,
    ) -> Result<()> {
        let bounded = bound_text(error, 4096);
        let mut tx = self.pool.begin().await?;
        verify_lease(&mut tx, token, stage).await?;
        checkpoint_failed_tx(&mut tx, token, stage, unit_key, &bounded).await?;
        sqlx::query(
            "UPDATE community_deletion_requests \
             SET blocked_at = now(), blocked_reason = $4, last_error = $4, \
                 last_error_at = now(), lease_owner = NULL, lease_until = NULL, updated_at = now() \
             WHERE id = $1 AND lease_owner = $2 AND lease_generation = $3",
        )
        .bind(token.request_id)
        .bind(&token.owner)
        .bind(token.generation)
        .bind(&bounded)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Take the shared community deletion lock inside an existing transaction.
    pub async fn guard_transaction(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: CommunityId,
    ) -> Result<()> {
        sqlx::query("SELECT pg_advisory_xact_lock_shared(community_deletion_lock_key($1))")
            .bind(community.as_uuid())
            .execute(&mut **tx)
            .await?;
        let state: Option<String> = sqlx::query_scalar(
            "SELECT deletion_state FROM communities WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .fetch_optional(&mut **tx)
        .await?;
        match state.as_deref() {
            Some("active") => Ok(()),
            Some(other) => Err(DbError::AccessDenied(format!(
                "community {community} is write-fenced ({other})"
            ))),
            None => Err(DbError::AccessDenied(format!(
                "community {community} is missing or tombstoned"
            ))),
        }
    }

    /// Take the shared community deletion lock inside an existing transaction
    /// and authorize a final mutation under an already-admitted serving lease.
    ///
    /// The lease is checked in the same transaction as the mutation. During
    /// quiescing, only this exact unexpired lease and fence generation may
    /// finish; active communities continue to accept the admitted write too.
    pub async fn guard_transaction_with_serving_lease(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        lease: &ServingWriteLease,
    ) -> Result<()> {
        sqlx::query("SELECT pg_advisory_xact_lock_shared(community_deletion_lock_key($1))")
            .bind(lease.community_id.as_uuid())
            .execute(&mut **tx)
            .await?;
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_serving_write_leases lease \
             JOIN communities community ON community.id = lease.community_id \
             WHERE lease.id = $1 AND lease.community_id = $2 AND lease.owner = $3 \
               AND lease.generation = $4 AND lease.fence_generation = $5 \
               AND lease.lease_until >= now() AND community.deleted_at IS NULL \
               AND community.deletion_state IN ('active', 'quiescing') \
               AND community.deletion_fence_generation = lease.fence_generation)",
        )
        .bind(lease.id)
        .bind(lease.community_id.as_uuid())
        .bind(&lease.owner)
        .bind(lease.generation)
        .bind(lease.fence_generation)
        .fetch_one(&mut **tx)
        .await?;
        if !valid {
            return Err(DbError::AccessDenied(format!(
                "stale serving write lease {}",
                lease.id
            )));
        }
        sqlx::query(
            "SELECT set_config('buzz.serving_write_community', $1, true), \
                    set_config('buzz.serving_write_lease_id', $2, true), \
                    set_config('buzz.serving_write_owner', $3, true), \
                    set_config('buzz.serving_write_generation', $4, true), \
                    set_config('buzz.serving_write_fence_generation', $5, true)",
        )
        .bind(lease.community_id.to_string())
        .bind(lease.id.to_string())
        .bind(&lease.owner)
        .bind(lease.generation.to_string())
        .bind(lease.fence_generation.to_string())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Acquire a durable, expiring lease for an external serving side effect.
    ///
    /// The short transaction shares the same advisory lock as the destructive
    /// fence. The fence therefore orders after all acquisitions that began
    /// first, changes lifecycle state, then refuses every later acquisition.
    pub async fn acquire_serving_write_lease(
        &self,
        community: CommunityId,
        operation: &str,
        owner: &str,
        lease_duration: Duration,
    ) -> Result<ServingWriteLease> {
        let lease_seconds = i64::try_from(lease_duration.as_secs()).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await?;
        // The assertion owns both the shared ordering lock and the supported
        // READ COMMITTED check. The lease table is trigger-excluded, so this
        // explicit admission is its database-enforced write fence.
        if let Err(error) = sqlx::query("SELECT assert_community_write_allowed($1)")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
        {
            if error.as_database_error().is_some_and(|database_error| {
                database_error.code().as_deref() == Some("55000")
                    && database_error.message().starts_with("community write")
            }) {
                return Err(DbError::AccessDenied(format!(
                    "community {community} is write-fenced or missing"
                )));
            }
            return Err(error.into());
        }
        let row = sqlx::query(
            "INSERT INTO community_serving_write_leases \
             (community_id, operation, owner, fence_generation, lease_until) \
             SELECT id, $2, $3, deletion_fence_generation, \
                    now() + make_interval(secs => $4) \
             FROM communities WHERE id = $1 AND deletion_state = 'active' \
               AND deleted_at IS NULL \
             RETURNING id, generation, fence_generation, lease_until",
        )
        .bind(community.as_uuid())
        .bind(operation)
        .bind(owner)
        .bind(lease_seconds)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            DbError::AccessDenied(format!("community {community} is write-fenced or missing"))
        })?;
        let lease = ServingWriteLease {
            id: row.try_get("id")?,
            community_id: community,
            operation: operation.to_owned(),
            owner: owner.to_owned(),
            generation: row.try_get("generation")?,
            fence_generation: row.try_get("fence_generation")?,
            lease_until: row.try_get("lease_until")?,
        };
        tx.commit().await?;
        Ok(lease)
    }

    /// Renew an already-admitted external side-effect lease while the community
    /// is active or quiescing.
    ///
    /// Quiescing rejects new acquisition, but the exact existing, unexpired
    /// lease must remain renewable until its operation finishes. Otherwise the
    /// heartbeat would abandon the exclusion proof while remote I/O may still
    /// commit. Fence generation, owner, generation, expiry, and tombstone checks
    /// continue to reject stale or post-fence renewal.
    pub async fn renew_serving_write_lease(
        &self,
        lease: &mut ServingWriteLease,
        lease_duration: Duration,
    ) -> Result<()> {
        let lease_seconds = i64::try_from(lease_duration.as_secs()).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock_shared(community_deletion_lock_key($1))")
            .bind(lease.community_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        let lease_until: Option<DateTime<Utc>> = sqlx::query_scalar(
            "UPDATE community_serving_write_leases lease \
             SET lease_until = now() + make_interval(secs => $6), heartbeat_at = now() \
             FROM communities community \
             WHERE lease.id = $1 AND lease.community_id = $2 AND lease.owner = $3 \
               AND lease.generation = $4 AND lease.fence_generation = $5 \
               AND lease.lease_until >= now() \
               AND community.id = lease.community_id \
               AND community.deletion_state IN ('active', 'quiescing') \
               AND community.deleted_at IS NULL \
               AND community.deletion_fence_generation = lease.fence_generation \
             RETURNING lease.lease_until",
        )
        .bind(lease.id)
        .bind(lease.community_id.as_uuid())
        .bind(&lease.owner)
        .bind(lease.generation)
        .bind(lease.fence_generation)
        .bind(lease_seconds)
        .fetch_optional(&mut *tx)
        .await?;
        let lease_until = lease_until.ok_or_else(|| {
            DbError::AccessDenied(format!("stale serving write lease {}", lease.id))
        })?;
        tx.commit().await?;
        lease.lease_until = lease_until;
        Ok(())
    }

    /// Release a serving side-effect lease. A stale release is harmless.
    pub async fn release_serving_write_lease(&self, lease: &ServingWriteLease) -> Result<bool> {
        let deleted = sqlx::query(
            "DELETE FROM community_serving_write_leases \
             WHERE id = $1 AND community_id = $2 AND owner = $3 AND generation = $4 \
               AND fence_generation = $5",
        )
        .bind(lease.id)
        .bind(lease.community_id.as_uuid())
        .bind(&lease.owner)
        .bind(lease.generation)
        .bind(lease.fence_generation)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(deleted == 1)
    }

    /// Check that an external side-effect lease remains current for finalization.
    ///
    /// A lease admitted before quiescing may renew, complete, and release; new
    /// work remains blocked, preserving an accurate drain without abandoning an
    /// admitted remote effect.
    pub async fn verify_serving_write_lease(&self, lease: &ServingWriteLease) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock_shared(community_deletion_lock_key($1))")
            .bind(lease.community_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_serving_write_leases lease \
             JOIN communities community ON community.id = lease.community_id \
             WHERE lease.id = $1 AND lease.community_id = $2 AND lease.owner = $3 \
               AND lease.generation = $4 AND lease.fence_generation = $5 \
               AND lease.lease_until >= now() \
               AND community.deleted_at IS NULL \
               AND community.deletion_state IN ('active', 'quiescing') \
               AND community.deletion_fence_generation = lease.fence_generation)",
        )
        .bind(lease.id)
        .bind(lease.community_id.as_uuid())
        .bind(&lease.owner)
        .bind(lease.generation)
        .bind(lease.fence_generation)
        .fetch_one(&mut *tx)
        .await?;
        if valid {
            tx.commit().await?;
            Ok(())
        } else {
            Err(DbError::AccessDenied(format!(
                "stale serving write lease {}",
                lease.id
            )))
        }
    }

    /// Delete expired serving leases in a bounded batch.
    pub async fn reap_expired_serving_write_leases(&self, limit: i64) -> Result<u64> {
        let affected = sqlx::query(
            "WITH expired AS ( \
                 SELECT id FROM community_serving_write_leases \
                 WHERE lease_until < now() ORDER BY lease_until LIMIT $1 \
                 FOR UPDATE SKIP LOCKED \
             ) DELETE FROM community_serving_write_leases lease \
               USING expired WHERE lease.id = expired.id",
        )
        .bind(limit.clamp(1, 10_000))
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(affected)
    }

    /// Return serving-lease counts and dead-tuple estimate for observability.
    pub async fn serving_lease_stats(&self) -> Result<ServingLeaseStats> {
        let row = sqlx::query(
            "SELECT count(*) FILTER (WHERE lease_until >= now())::BIGINT AS active, \
                    count(*) FILTER (WHERE lease_until < now())::BIGINT AS expired, \
                    COALESCE((SELECT n_dead_tup::BIGINT FROM pg_stat_user_tables \
                              WHERE relname = 'community_serving_write_leases'), 0) AS dead_tuples \
             FROM community_serving_write_leases",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(ServingLeaseStats {
            active: row.try_get("active")?,
            expired: row.try_get("expired")?,
            dead_tuples: row.try_get("dead_tuples")?,
        })
    }

    /// Whether a community remains active and serving-write eligible.
    pub async fn is_serving_active(&self, community: CommunityId) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM communities WHERE id = $1 \
             AND archived_at IS NULL AND deleted_at IS NULL AND deletion_state = 'active')",
        )
        .bind(community.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    async fn advance_with_checkpoint(
        &self,
        token: &LeaseToken,
        from: DeletionStage,
        to: DeletionStage,
        unit_key: &str,
        detail: serde_json::Value,
    ) -> Result<()> {
        let generation = require_fence_generation(token)?;
        let mut tx = self.pool.begin().await?;
        verify_lease_and_fence(&mut tx, token, from, generation).await?;
        advance_request_tx(&mut tx, token, from, to, Some(generation)).await?;
        checkpoint_completed_tx(&mut tx, token, from, unit_key, detail).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn live_scoped_tables(&self) -> Result<BTreeSet<String>> {
        let mut conn = self.pool.acquire().await?;
        live_scoped_tables_on(&mut conn).await
    }

    async fn live_fenced_tables(&self) -> Result<BTreeSet<String>> {
        let mut conn = self.pool.acquire().await?;
        live_fenced_tables_on(&mut conn).await
    }
}

/// Take the shared schema/destruction advisory lock for the current
/// transaction.
///
/// Transaction-scoped so every abort path — including executor death —
/// releases it. Migrations hold the exclusive session counterpart for their
/// whole run (see [`crate::migration::run_migrations`]); shared holders do
/// not block each other, so concurrent deletion executors are unaffected.
async fn lock_schema_destruction_shared(conn: &mut PgConnection) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
        .execute(conn)
        .await?;
    Ok(())
}

/// Connection-bound form of [`DeletionStore::validate_catalog`].
///
/// Destructive transactions call this on their own transaction after taking
/// the shared schema/destruction lock, so the validated surface cannot change
/// before the transaction commits.
async fn validate_catalog_on(conn: &mut PgConnection) -> Result<()> {
    let expected = EXPECTED_SCOPED_TABLES
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let live_tables = live_scoped_tables_on(conn).await?;
    if live_tables != expected {
        let missing = expected
            .difference(&live_tables)
            .cloned()
            .collect::<Vec<_>>();
        let unknown = live_tables
            .difference(&expected)
            .cloned()
            .collect::<Vec<_>>();
        return Err(DbError::DeletionSafety(format!(
            "community deletion catalog drift (missing={}, unknown={})",
            missing.join(","),
            unknown.join(",")
        )));
    }

    let fenced_tables = live_fenced_tables_on(conn).await?;
    if fenced_tables != expected {
        let missing = expected
            .difference(&fenced_tables)
            .cloned()
            .collect::<Vec<_>>();
        let unknown = fenced_tables
            .difference(&expected)
            .cloned()
            .collect::<Vec<_>>();
        return Err(DbError::DeletionSafety(format!(
            "community deletion write-fence drift (missing={}, unknown={})",
            missing.join(","),
            unknown.join(",")
        )));
    }
    Ok(())
}

async fn live_scoped_tables_on(conn: &mut PgConnection) -> Result<BTreeSet<String>> {
    let rows: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_attribute a ON a.attrelid = c.oid
        WHERE n.nspname = 'public'
          AND c.relkind IN ('r', 'p')
          AND NOT c.relispartition
          AND a.attname = 'community_id'
          AND NOT a.attisdropped
          AND NOT community_write_fence_excluded_table(c.relname)
        ORDER BY c.relname
        "#,
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().collect())
}

async fn live_fenced_tables_on(conn: &mut PgConnection) -> Result<BTreeSet<String>> {
    let rows: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname
        FROM pg_trigger trigger
        JOIN pg_class c ON c.oid = trigger.tgrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_proc procedure ON procedure.oid = trigger.tgfoid
        WHERE n.nspname = 'public'
          AND NOT trigger.tgisinternal
          AND NOT c.relispartition
          AND procedure.proname = 'enforce_community_write_fence'
          AND trigger.tgenabled = 'O'
          AND (trigger.tgtype & 1) = 1
          AND (trigger.tgtype & 2) = 2
          AND (trigger.tgtype & 4) = 4
          AND (trigger.tgtype & 8) = 8
          AND (trigger.tgtype & 16) = 16
        ORDER BY c.relname
        "#,
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Fail closed when a community-prefix inventory has an unsafe shape.
pub fn validate_storage_manifest(manifest: &StorageManifest) -> Result<()> {
    if manifest.version != 4 {
        return Err(DbError::DeletionSafety(format!(
            "unsupported storage manifest version {}",
            manifest.version
        )));
    }
    if manifest.prefixes.is_empty() {
        return Err(DbError::DeletionSafety(
            "storage manifest has no tenant prefixes".to_string(),
        ));
    }
    if manifest
        .prefixes
        .windows(2)
        .any(|pair| pair[0].prefix >= pair[1].prefix)
    {
        return Err(DbError::DeletionSafety(
            "storage manifest prefixes are not strictly sorted".to_string(),
        ));
    }
    for prefix in &manifest.prefixes {
        // An empty prefix would enumerate — and delete — the whole bucket.
        if prefix.prefix.is_empty() {
            return Err(DbError::DeletionSafety(
                "storage manifest contains an empty prefix".to_string(),
            ));
        }
        if prefix.keys_digest.len() != 64
            || !prefix
                .keys_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DbError::DeletionSafety(format!(
                "storage manifest digest for {} is not lowercase hex sha-256",
                prefix.prefix
            )));
        }
    }
    Ok(())
}

/// Verify the persisted chunk stream is exactly the frozen enumeration:
/// contiguous chunk numbers, chunks grouped by manifest prefix order, every
/// key under its chunk's prefix, and per-prefix digest/count equality.
fn validate_manifest_key_chunks(
    manifest: &StorageManifest,
    chunks: &[(i64, String, sqlx::types::Json<Vec<String>>)],
) -> Result<()> {
    let close = |summary: &PrefixManifest, digest: KeyStreamDigest| -> Result<()> {
        let (hex_digest, count) = digest.finish();
        if hex_digest != summary.keys_digest || count != summary.object_count {
            return Err(DbError::DeletionSafety(format!(
                "frozen key chunks do not match the destructive manifest for prefix {}",
                summary.prefix
            )));
        }
        Ok(())
    };
    let mut remaining = manifest.prefixes.iter();
    let mut current = remaining.next();
    let mut digest = KeyStreamDigest::new();
    for (index, (chunk_no, chunk_prefix, keys)) in chunks.iter().enumerate() {
        if *chunk_no != i64::try_from(index).unwrap_or(i64::MAX) {
            return Err(DbError::DeletionSafety(
                "frozen key chunk sequence has gaps".to_string(),
            ));
        }
        loop {
            match current {
                Some(summary) if summary.prefix == *chunk_prefix => break,
                Some(summary) => {
                    close(summary, std::mem::take(&mut digest))?;
                    current = remaining.next();
                }
                None => {
                    return Err(DbError::DeletionSafety(format!(
                        "frozen key chunk prefix {chunk_prefix} is not in the destructive manifest"
                    )));
                }
            }
        }
        if keys.0.is_empty() {
            return Err(DbError::DeletionSafety(
                "frozen key chunk is empty".to_string(),
            ));
        }
        for key in &keys.0 {
            if !key.starts_with(chunk_prefix.as_str()) {
                return Err(DbError::DeletionSafety(format!(
                    "frozen key {key} is outside its chunk prefix {chunk_prefix}"
                )));
            }
            digest.fold(key)?;
        }
    }
    if let Some(summary) = current {
        close(summary, digest)?;
    }
    for summary in remaining {
        close(summary, KeyStreamDigest::new())?;
    }
    Ok(())
}

async fn verify_lease(
    tx: &mut Transaction<'_, Postgres>,
    token: &LeaseToken,
    stage: DeletionStage,
) -> Result<()> {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM community_deletion_requests request \
         JOIN community_deletion_approvals approval ON approval.request_id = request.id \
          AND approval.community_id = request.community_id \
          AND approval.inventory_digest = request.inventory_digest \
         WHERE request.id = $1 AND request.community_id = $5 AND request.stage = $2 \
           AND request.lease_owner = $3 AND request.lease_generation = $4 \
           AND request.lease_until >= now() AND request.blocked_at IS NULL)",
    )
    .bind(token.request_id)
    .bind(stage.to_string())
    .bind(&token.owner)
    .bind(token.generation)
    .bind(token.community_id.as_uuid())
    .fetch_one(&mut **tx)
    .await?;
    if valid {
        Ok(())
    } else {
        Err(stale_lease_error(token))
    }
}

async fn verify_lease_and_fence(
    tx: &mut Transaction<'_, Postgres>,
    token: &LeaseToken,
    stage: DeletionStage,
    fence_generation: i64,
) -> Result<()> {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM community_deletion_requests request \
         JOIN communities community ON community.id = request.community_id \
         JOIN community_deletion_approvals approval ON approval.request_id = request.id \
          AND approval.community_id = request.community_id \
          AND approval.inventory_digest = request.inventory_digest \
         WHERE request.id = $1 AND request.community_id = $6 \
           AND request.stage = $2 AND request.lease_owner = $3 \
           AND request.lease_generation = $4 AND request.lease_until >= now() \
           AND request.blocked_at IS NULL AND request.fence_generation = $5 \
           AND community.deletion_state IN ('fenced', 'tombstone') \
           AND community.deletion_fence_generation = $5)",
    )
    .bind(token.request_id)
    .bind(stage.to_string())
    .bind(&token.owner)
    .bind(token.generation)
    .bind(fence_generation)
    .bind(token.community_id.as_uuid())
    .fetch_one(&mut **tx)
    .await?;
    if valid {
        Ok(())
    } else {
        Err(DbError::AccessDenied(format!(
            "stale lease or fencing generation for deletion {}",
            token.request_id
        )))
    }
}

async fn set_executor_gucs(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    generation: i64,
) -> Result<()> {
    sqlx::query(
        "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                set_config('buzz.deletion_fence_generation', $2, true)",
    )
    .bind(community.to_string())
    .bind(generation.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn advance_request_tx(
    tx: &mut Transaction<'_, Postgres>,
    token: &LeaseToken,
    from: DeletionStage,
    to: DeletionStage,
    fence_generation: Option<i64>,
) -> Result<()> {
    if from.next() != Some(to) {
        return Err(DbError::DeletionSafety(format!(
            "illegal deletion transition {from} -> {to}"
        )));
    }
    let affected = sqlx::query(
        "UPDATE community_deletion_requests \
         SET stage = $5, fence_generation = COALESCE($6, fence_generation), \
             updated_at = now(), retry_count = 0, retry_stage = NULL, \
             last_error = NULL, last_error_at = NULL \
         WHERE id = $1 AND stage = $4 AND lease_owner = $2 \
           AND lease_generation = $3 AND lease_until >= now() AND blocked_at IS NULL",
    )
    .bind(token.request_id)
    .bind(&token.owner)
    .bind(token.generation)
    .bind(from.to_string())
    .bind(to.to_string())
    .bind(fence_generation)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if affected == 1 {
        Ok(())
    } else {
        Err(stale_lease_error(token))
    }
}

async fn checkpoint_completed_tx(
    tx: &mut Transaction<'_, Postgres>,
    token: &LeaseToken,
    stage: DeletionStage,
    unit_key: &str,
    detail: serde_json::Value,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO community_deletion_checkpoints
            (request_id, stage, unit_key, status, lease_generation, detail, completed_at)
        VALUES ($1, $2, $3, 'completed', $4, $5, now())
        ON CONFLICT (request_id, stage, unit_key) DO UPDATE
        SET status = 'completed', lease_generation = EXCLUDED.lease_generation,
            attempts = community_deletion_checkpoints.attempts + 1,
            detail = EXCLUDED.detail, error = NULL, completed_at = now()
        "#,
    )
    .bind(token.request_id)
    .bind(stage.to_string())
    .bind(unit_key)
    .bind(token.generation)
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn checkpoint_failed_tx(
    tx: &mut Transaction<'_, Postgres>,
    token: &LeaseToken,
    stage: DeletionStage,
    unit_key: &str,
    error: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO community_deletion_checkpoints
            (request_id, stage, unit_key, status, lease_generation, error)
        VALUES ($1, $2, $3, 'failed', $4, $5)
        ON CONFLICT (request_id, stage, unit_key) DO UPDATE
        SET status = 'failed', lease_generation = EXCLUDED.lease_generation,
            attempts = community_deletion_checkpoints.attempts + 1,
            error = EXCLUDED.error, completed_at = NULL
        "#,
    )
    .bind(token.request_id)
    .bind(stage.to_string())
    .bind(unit_key)
    .bind(token.generation)
    .bind(error)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn serialize_community_id<S>(
    community: &CommunityId,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&community.to_string())
}

fn row_to_request(row: sqlx::postgres::PgRow) -> Result<DeletionRequest> {
    let community_id: Uuid = row.try_get("community_id")?;
    let digest: Option<Vec<u8>> = row.try_get("inventory_digest")?;
    Ok(DeletionRequest {
        id: row.try_get("id")?,
        community_id: CommunityId::from_uuid(community_id),
        community_host: row.try_get("community_host")?,
        stage: row.try_get::<String, _>("stage")?.parse()?,
        retry_stage: row
            .try_get::<Option<String>, _>("retry_stage")?
            .map(|stage| stage.parse())
            .transpose()?,
        requested_by: row.try_get("requested_by")?,
        reason: row.try_get("reason")?,
        schema_manifest: row.try_get("schema_manifest")?,
        storage_manifest: row.try_get("storage_manifest")?,
        destructive_storage_manifest: row.try_get("destructive_storage_manifest")?,
        inventory_manifest: row.try_get("inventory_manifest")?,
        inventory_digest: digest.map(hex::encode),
        fence_generation: row.try_get("fence_generation")?,
        lease_owner: row.try_get("lease_owner")?,
        lease_generation: row.try_get("lease_generation")?,
        lease_until: row.try_get("lease_until")?,
        attempts: row.try_get("attempts")?,
        retry_count: row.try_get("retry_count")?,
        last_error: row.try_get("last_error")?,
        next_attempt_at: row.try_get("next_attempt_at")?,
        blocked_reason: row.try_get("blocked_reason")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        pre_quiesce_archived_at: row.try_get("pre_quiesce_archived_at")?,
        quiescing_started_at: row.try_get("quiescing_started_at")?,
        aborted_by: row.try_get("aborted_by")?,
        abort_reason: row.try_get("abort_reason")?,
        aborted_at: row.try_get("aborted_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

/// Return whether an error is the deletion store's typed ownership-loss class.
pub fn is_stale_deletion_lease(error: &DbError) -> bool {
    matches!(error, DbError::AccessDenied(message) if message.starts_with("stale deletion lease ") || message.starts_with("stale lease or fencing generation for deletion "))
}

fn stale_lease_error(token: &LeaseToken) -> DbError {
    DbError::AccessDenied(format!(
        "stale deletion lease {} owner {:?} generation {}",
        token.request_id, token.owner, token.generation
    ))
}

fn require_fence_generation(token: &LeaseToken) -> Result<i64> {
    token.fence_generation.ok_or_else(|| {
        DbError::DeletionSafety(format!(
            "deletion {} has no durable fence generation",
            token.request_id
        ))
    })
}

fn bound_text(input: &str, max: usize) -> String {
    if input.len() <= max {
        return input.to_owned();
    }
    let mut end = max;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_prefix(prefix: &str) -> PrefixManifest {
        PrefixManifest {
            prefix: prefix.to_string(),
            object_count: 0,
            total_bytes: 0,
            keys_digest: KeyStreamDigest::new().finish().0,
        }
    }

    fn storage_manifest() -> StorageManifest {
        StorageManifest {
            version: 4,
            prefixes: vec![
                empty_prefix("_meta/c/"),
                empty_prefix("_uploads/c/"),
                empty_prefix("repos/c/"),
            ],
        }
    }

    #[test]
    fn stage_order_is_exact_and_terminal() {
        let mut stage = DeletionStage::Submitted;
        let mut seen = vec![stage];
        while let Some(next) = stage.next() {
            stage = next;
            seen.push(stage);
        }
        assert_eq!(
            seen,
            vec![
                DeletionStage::Submitted,
                DeletionStage::Inventoried,
                DeletionStage::Approved,
                DeletionStage::Fenced,
                DeletionStage::Drained,
                DeletionStage::BindingsRemoved,
                DeletionStage::PostgresPurged,
                DeletionStage::CachePurged,
                DeletionStage::LogicallyVerified,
                DeletionStage::RetentionPending,
            ]
        );
        assert!(!DeletionStage::Submitted.runnable());
        assert!(!DeletionStage::Inventoried.runnable());
        assert!(DeletionStage::Approved.runnable());
        assert!(!DeletionStage::RetentionPending.runnable());
        assert!(!DeletionStage::Aborted.runnable());
    }

    #[test]
    fn stale_lease_classifier_does_not_swallow_other_access_denials() {
        let stale = stale_lease_error(&LeaseToken {
            request_id: Uuid::new_v4(),
            owner: "owner".to_string(),
            generation: 1,
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            fence_generation: None,
        });
        assert!(is_stale_deletion_lease(&stale));
        assert!(!is_stale_deletion_lease(&DbError::AccessDenied(
            "ordinary authorization failure".to_string()
        )));
    }

    #[test]
    fn storage_manifest_shape_invariants_fail_closed() {
        assert!(validate_storage_manifest(&storage_manifest()).is_ok());

        let mut unsorted = storage_manifest();
        unsorted.prefixes.swap(0, 1);
        assert!(validate_storage_manifest(&unsorted).is_err());

        let mut whole_bucket = storage_manifest();
        whole_bucket.prefixes[0].prefix = String::new();
        assert!(validate_storage_manifest(&whole_bucket).is_err());

        let mut malformed_digest = storage_manifest();
        malformed_digest.prefixes[0].keys_digest = "not-hex".to_string();
        assert!(validate_storage_manifest(&malformed_digest).is_err());
    }

    #[test]
    fn key_stream_digest_requires_strict_order_and_is_chunking_invariant() {
        let keys = ["a/1", "a/2", "a/3"];
        let mut whole = KeyStreamDigest::new();
        for key in keys {
            whole.fold(key).expect("ascending fold");
        }
        // The digest must not depend on where chunk boundaries fall.
        let mut split = KeyStreamDigest::new();
        split.fold(keys[0]).expect("chunk one");
        split.fold(keys[1]).expect("chunk one");
        split.fold(keys[2]).expect("chunk two");
        assert_eq!(whole.finish(), split.finish());

        let mut out_of_order = KeyStreamDigest::new();
        out_of_order.fold("b").expect("first key");
        assert!(out_of_order.fold("a").is_err());
        let mut duplicate = KeyStreamDigest::new();
        duplicate.fold("a").expect("first key");
        assert!(duplicate.fold("a").is_err());
    }

    #[test]
    fn manifest_key_chunks_must_hash_to_the_frozen_summaries() {
        let keys = vec!["_meta/c/1".to_string(), "_meta/c/2".to_string()];
        let mut digest = KeyStreamDigest::new();
        for key in &keys {
            digest.fold(key).expect("fold");
        }
        let (hex_digest, count) = digest.finish();
        let mut manifest = storage_manifest();
        manifest.prefixes[0].object_count = count;
        manifest.prefixes[0].keys_digest = hex_digest;

        let chunk = |chunk_no: i64, keys: &[String]| {
            (
                chunk_no,
                "_meta/c/".to_string(),
                sqlx::types::Json(keys.to_vec()),
            )
        };
        assert!(validate_manifest_key_chunks(
            &manifest,
            &[chunk(0, &keys[..1]), chunk(1, &keys[1..])]
        )
        .is_ok());
        // Missing, reordered, or extra keys change the digest.
        assert!(validate_manifest_key_chunks(&manifest, &[chunk(0, &keys[..1])]).is_err());
        // A gap in the chunk sequence is an interrupted write, not a manifest.
        assert!(validate_manifest_key_chunks(&manifest, &[chunk(1, &keys)]).is_err());
        // A key outside its chunk's prefix must never freeze.
        let foreign = vec!["_uploads/other/1".to_string()];
        assert!(
            validate_manifest_key_chunks(&manifest, &[chunk(0, &keys), chunk(1, &foreign)])
                .is_err()
        );
        // No chunks at all only matches an all-empty manifest.
        assert!(validate_manifest_key_chunks(&manifest, &[]).is_err());
        assert!(validate_manifest_key_chunks(&storage_manifest(), &[]).is_ok());
    }

    #[test]
    fn frozen_inventory_digest_is_stable() {
        let inventory = FrozenInventory {
            schema: SchemaManifest {
                scoped_tables: vec!["events".to_string()],
                row_counts: BTreeMap::from([("events".to_string(), 3)]),
                fenced_tables: vec!["events".to_string()],
            },
            storage: storage_manifest(),
        };
        assert_eq!(inventory.digest().unwrap(), inventory.digest().unwrap());
        assert_eq!(inventory.digest().unwrap().len(), 32);
    }

    #[test]
    fn errors_are_utf8_bounded() {
        let input = format!("{}🛸", "x".repeat(4095));
        let bounded = bound_text(&input, 4096);
        assert!(bounded.len() <= 4096);
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::{CreateCommunityWithOwnerResult, Db, DbConfig};

    async fn store() -> (Db, DeletionStore) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
        let db = Db::new(&DbConfig {
            database_url,
            max_connections: 5,
            min_connections: 0,
            ..DbConfig::default()
        })
        .await
        .expect("connect deletion test DB");
        db.migrate().await.expect("migrate deletion test DB");
        let store = db.deletion_store();
        (db, store)
    }

    fn empty_prefix_manifest(prefix: String) -> PrefixManifest {
        PrefixManifest {
            prefix,
            object_count: 0,
            total_bytes: 0,
            keys_digest: KeyStreamDigest::new().finish().0,
        }
    }

    fn empty_storage_manifest(community: CommunityId) -> StorageManifest {
        StorageManifest {
            version: 4,
            prefixes: vec![
                empty_prefix_manifest(format!("_meta/{community}/")),
                empty_prefix_manifest(format!("_uploads/{community}/")),
                empty_prefix_manifest(format!("repos/{community}/")),
            ],
        }
    }

    async fn inventoried_request(
        db: &Db,
        store: &DeletionStore,
    ) -> (DeletionRequest, FrozenInventory) {
        let host = format!("deletion-{}.example", Uuid::new_v4().simple());
        let community = db
            .ensure_configured_community(&host)
            .await
            .expect("create community");
        let submitted = store
            .submit(&host, "test-operator", Some("test deletion"))
            .await
            .expect("submit");
        assert_eq!(submitted.community_id, community.id);
        let inventory = FrozenInventory {
            schema: store
                .inventory_schema(community.id)
                .await
                .expect("schema inventory"),
            storage: empty_storage_manifest(community.id),
        };
        let request = store
            .freeze_inventory(submitted.id, &inventory)
            .await
            .expect("freeze inventory");
        (request, inventory)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn approval_boundary_blocks_claim_until_exact_inventory_is_approved() {
        let (db, store) = store().await;
        let (request, inventory) = inventoried_request(&db, &store).await;
        assert_eq!(request.stage, DeletionStage::Inventoried);
        assert!(store
            .claim_specific(request.id, "executor-a", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim before approval")
            .is_none());

        // Row counts are frozen observational evidence. Ordinary serving
        // churn after inventory does not invalidate approval: execution fences,
        // purges, and verifies the live tenant state independently.
        db.add_to_allowlist(request.community_id, &[7_u8; 32], &[8_u8; 32], None)
            .await
            .expect("post-inventory serving write");
        let current_schema = store
            .inventory_schema(request.community_id)
            .await
            .expect("live schema after row churn");
        assert_eq!(
            current_schema.row_counts["pubkey_allowlist"],
            inventory.schema.row_counts["pubkey_allowlist"] + 1
        );
        assert_eq!(current_schema.scoped_tables, inventory.schema.scoped_tables);
        assert_eq!(current_schema.fenced_tables, inventory.schema.fenced_tables);

        let mismatched_insert = sqlx::query(
            "INSERT INTO community_deletion_approvals \
             (request_id, community_id, inventory_digest, approved_by) \
             VALUES ($1, $2, $3, 'tampered')",
        )
        .bind(request.id)
        .bind(*request.community_id.as_uuid())
        .bind(vec![0_u8; 32])
        .execute(&db.pool)
        .await;
        assert!(
            mismatched_insert.is_err(),
            "a mismatched approval must be unrepresentable"
        );
        let approved = store
            .approve(request.id, "approver-a", Some("reviewed"))
            .await
            .expect("approve");
        assert_eq!(approved.stage, DeletionStage::Approved);
        assert_eq!(
            approved.inventory_digest,
            Some(hex::encode(inventory.digest().unwrap()))
        );
        let mismatched_approval = sqlx::query(
            "UPDATE community_deletion_approvals SET inventory_digest = $2 WHERE request_id = $1",
        )
        .bind(request.id)
        .bind(vec![0_u8; 32])
        .execute(&db.pool)
        .await;
        assert!(
            mismatched_approval.is_err(),
            "approval digest must remain database-bound to the frozen request digest"
        );
        let mismatched_request = sqlx::query(
            "UPDATE community_deletion_requests SET inventory_digest = $2 WHERE id = $1",
        )
        .bind(request.id)
        .bind(vec![1_u8; 32])
        .execute(&db.pool)
        .await;
        assert!(
            mismatched_request.is_err(),
            "the frozen request digest must remain bound to its approval"
        );
        assert!(store
            .claim_specific(request.id, "executor-a", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim approved")
            .is_some());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn approved_request_cannot_be_retargeted_rewritten_or_claimed_without_approval() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        let other_host = format!("control-{}.example", Uuid::new_v4().simple());
        let control = db
            .ensure_configured_community(&other_host)
            .await
            .expect("create control community");

        for mutation in [
            sqlx::query("UPDATE community_deletion_requests SET community_id = $2 WHERE id = $1")
                .bind(request.id)
                .bind(*control.id.as_uuid())
                .execute(&db.pool)
                .await,
            sqlx::query("UPDATE community_deletion_requests SET community_host = $2 WHERE id = $1")
                .bind(request.id)
                .bind(&other_host)
                .execute(&db.pool)
                .await,
            sqlx::query(
                "UPDATE community_deletion_requests SET inventory_manifest = '{}'::jsonb WHERE id = $1",
            )
            .bind(request.id)
            .execute(&db.pool)
            .await,
            sqlx::query(
                "UPDATE community_deletion_requests SET storage_manifest = '{}'::jsonb WHERE id = $1",
            )
            .bind(request.id)
            .execute(&db.pool)
            .await,
        ] {
            assert!(mutation.is_err(), "frozen deletion target and inventory must be immutable");
        }

        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve request");
        let claim = store
            .claim_specific(request.id, "forged-executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim approved request")
            .expect("approved request is claimable");
        let approval_delete =
            sqlx::query("DELETE FROM community_deletion_approvals WHERE request_id = $1")
                .bind(request.id)
                .execute(&db.pool)
                .await;
        assert!(
            approval_delete.is_err(),
            "approval evidence must be immutable"
        );
        for approval_update in [
            "UPDATE community_deletion_approvals SET approved_by = 'forged' WHERE request_id = $1",
            "UPDATE community_deletion_approvals SET approved_at = now() + interval '1 hour' WHERE request_id = $1",
            "UPDATE community_deletion_approvals SET note = 'rewritten' WHERE request_id = $1",
        ] {
            assert!(
                sqlx::query(approval_update)
                    .bind(request.id)
                    .execute(&db.pool)
                    .await
                    .is_err(),
                "approval evidence updates must be rejected"
            );
        }
        store
            .verify_execution_token(&claim.lease, DeletionStage::Approved)
            .await
            .expect("matching approval keeps lease valid");
        sqlx::query(
            "UPDATE community_deletion_requests \
             SET blocked_at = now(), blocked_reason = 'operator hold' WHERE id = $1",
        )
        .bind(request.id)
        .execute(&db.pool)
        .await
        .expect("block claimed request");
        assert!(
            store
                .heartbeat(&claim.lease, "worker", DEFAULT_LEASE_DURATION, false,)
                .await
                .is_err(),
            "blocked requests must not renew destructive leases"
        );

        let (forged, _) = inventoried_request(&db, &store).await;
        sqlx::query("UPDATE community_deletion_requests SET stage = 'approved' WHERE id = $1")
            .bind(forged.id)
            .execute(&db.pool)
            .await
            .expect("forge runnable stage without approval");
        assert!(store
            .claim_specific(forged.id, "forged-executor-2", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim forged request")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn retry_exhaustion_blocks_only_the_consecutive_stage_and_progress_resets_it() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");

        for attempt in 1..=8 {
            let claim = store
                .claim_specific(
                    request.id,
                    &format!("executor-{attempt}"),
                    DEFAULT_LEASE_DURATION,
                )
                .await
                .expect("claim retryable request")
                .expect("request remains claimable before exhaustion");
            store
                .record_retry(
                    &claim.lease,
                    DeletionStage::Approved,
                    "dependency",
                    "dependency unavailable",
                    Duration::ZERO,
                )
                .await
                .expect("record retry");

            let observed = store.get(request.id).await.expect("load retry state");
            assert_eq!(observed.retry_count, attempt);
            assert_eq!(observed.retry_stage, Some(DeletionStage::Approved));
            assert_eq!(observed.blocked_reason.is_some(), attempt == 8);
        }
        assert!(store
            .claim_specific(request.id, "blocked-executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim blocked request")
            .is_none());

        let recovered = store
            .unblock(request.id, "operator", "dependency repaired")
            .await
            .expect("unblock exhausted request");
        assert_eq!(recovered.retry_count, 0);
        assert_eq!(recovered.retry_stage, None);
        assert!(recovered.blocked_reason.is_none());

        let claim = store
            .claim_specific(request.id, "successor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim recovered request")
            .expect("recovered request is claimable");
        store
            .begin_quiescing(&claim.lease)
            .await
            .expect("begin quiescing after recovery");
        store.fence(&claim.lease).await.expect("advance stage");
        let advanced = store.get(request.id).await.expect("load advanced request");
        assert_eq!(advanced.stage, DeletionStage::Fenced);
        assert_eq!(advanced.retry_count, 0);
        assert_eq!(advanced.retry_stage, None);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn abort_serializes_before_quiescing_without_deadlock() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        let mut gate = db.pool.begin().await.expect("begin lock gate");
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(request.community_id.as_uuid())
            .execute(&mut *gate)
            .await
            .expect("hold community lock");

        let abort_store = store.clone();
        let aborting = tokio::spawn(async move {
            abort_store
                .abort(request.id, "operator", "race recovery")
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !aborting.is_finished(),
            "abort must wait for the community lock"
        );
        let forward_store = store.clone();
        let lease = claim.lease.clone();
        let forwarding = tokio::spawn(async move { forward_store.begin_quiescing(&lease).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !forwarding.is_finished(),
            "forward transition must queue on the same lock"
        );
        gate.commit().await.expect("release lock gate");

        let aborted = tokio::time::timeout(Duration::from_secs(5), aborting)
            .await
            .expect("abort must not deadlock")
            .expect("abort task")
            .expect("abort wins lock queue");
        assert_eq!(aborted.stage, DeletionStage::Aborted);
        let forward_error = tokio::time::timeout(Duration::from_secs(5), forwarding)
            .await
            .expect("forward transition must not deadlock")
            .expect("forward task")
            .expect_err("post-lock lease verification rejects aborted request");
        assert!(
            !matches!(
                &forward_error,
                DbError::Sqlx(sqlx::Error::Database(error)) if error.code().as_deref() == Some("40P01")
            ),
            "serialization must not report a PostgreSQL deadlock"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn abort_serializes_before_fence_without_deadlock() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        store.begin_quiescing(&claim.lease).await.expect("quiesce");
        let mut gate = db.pool.begin().await.expect("begin lock gate");
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(request.community_id.as_uuid())
            .execute(&mut *gate)
            .await
            .expect("hold community lock");

        let abort_store = store.clone();
        let aborting = tokio::spawn(async move {
            abort_store
                .abort(request.id, "operator", "race recovery")
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !aborting.is_finished(),
            "abort must wait for the community lock"
        );
        let forward_store = store.clone();
        let lease = claim.lease.clone();
        let forwarding = tokio::spawn(async move { forward_store.fence(&lease).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !forwarding.is_finished(),
            "fence must queue on the same lock"
        );
        gate.commit().await.expect("release lock gate");

        let aborted = tokio::time::timeout(Duration::from_secs(5), aborting)
            .await
            .expect("abort must not deadlock")
            .expect("abort task")
            .expect("abort wins lock queue");
        assert_eq!(aborted.stage, DeletionStage::Aborted);
        let forward_error = tokio::time::timeout(Duration::from_secs(5), forwarding)
            .await
            .expect("fence must not deadlock")
            .expect("fence task")
            .expect_err("post-lock lease verification rejects aborted request");
        assert!(
            !matches!(
                &forward_error,
                DbError::Sqlx(sqlx::Error::Database(error)) if error.code().as_deref() == Some("40P01")
            ),
            "serialization must not report a PostgreSQL deadlock"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn abort_preserves_audit_and_allows_fresh_request() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let aborted = store
            .abort(request.id, "operator", "cancel deletion")
            .await
            .expect("abort");
        assert_eq!(aborted.stage, DeletionStage::Aborted);

        let replacement = store
            .submit(
                &request.community_host,
                "second-operator",
                Some("fresh review"),
            )
            .await
            .expect("submit replacement request");
        assert_ne!(replacement.id, request.id);
        assert_eq!(replacement.stage, DeletionStage::Submitted);
        assert!(replacement.inventory_digest.is_none());
        assert_eq!(
            store.get(request.id).await.expect("preserved audit").stage,
            DeletionStage::Aborted
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn preclaim_setup_failure_is_durable_without_a_lease() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let blocked = store
            .block_preclaim_setup(
                request.id,
                "pre_claim:service_setup",
                "BUZZ_S3_ENDPOINT is required",
            )
            .await
            .expect("record setup failure");
        assert_eq!(blocked.stage, DeletionStage::Approved);
        assert_eq!(
            blocked.blocked_reason.as_deref(),
            Some("BUZZ_S3_ENDPOINT is required")
        );
        assert_eq!(
            blocked.last_error.as_deref(),
            Some("BUZZ_S3_ENDPOINT is required")
        );
        assert!(blocked.lease_owner.is_none());
        let inspection = store.inspect(request.id).await.expect("inspect failure");
        let checkpoint = inspection
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.unit_key == "pre_claim:service_setup")
            .expect("setup failure checkpoint");
        assert_eq!(checkpoint.status, "failed");
        assert_eq!(
            checkpoint.error.as_deref(),
            Some("BUZZ_S3_ENDPOINT is required")
        );
        assert_eq!(checkpoint.attempts, 1);
        assert!(checkpoint.completed_at.is_none());

        store
            .unblock(request.id, "operator", "dependency repaired")
            .await
            .expect("unblock after first setup failure");
        let blocked_again = store
            .block_preclaim_setup(
                request.id,
                "pre_claim:service_setup",
                "BUZZ_REDIS_URL is required",
            )
            .await
            .expect("record repeated setup failure");
        assert_eq!(
            blocked_again.blocked_reason.as_deref(),
            Some("BUZZ_REDIS_URL is required")
        );
        assert_eq!(
            blocked_again.last_error.as_deref(),
            Some("BUZZ_REDIS_URL is required")
        );
        let repeated = store
            .inspect(request.id)
            .await
            .expect("inspect repeated failure");
        let checkpoint = repeated
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.unit_key == "pre_claim:service_setup")
            .expect("repeated setup failure checkpoint");
        assert_eq!(checkpoint.status, "failed");
        assert_eq!(checkpoint.attempts, 2);
        assert_eq!(
            checkpoint.error.as_deref(),
            Some("BUZZ_REDIS_URL is required")
        );
        assert_eq!(
            checkpoint
                .detail
                .get("error")
                .and_then(|value| value.as_str()),
            Some("BUZZ_REDIS_URL is required")
        );
        assert!(checkpoint.completed_at.is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn operator_unblock_preserves_approval_and_records_recovery() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("approved request is claimable");
        store
            .block(
                &claim.lease,
                DeletionStage::Approved,
                "dependency",
                "operator repair required",
            )
            .await
            .expect("block request");

        assert!(store.unblock(request.id, "", "repair").await.is_err());
        let recovered = store
            .unblock(request.id, "operator", "bucket policy repaired")
            .await
            .expect("unblock after remediation");
        assert_eq!(recovered.stage, DeletionStage::Approved);
        assert!(recovered.blocked_reason.is_none());
        assert!(recovered.last_error.is_none());
        assert_eq!(recovered.inventory_digest, request.inventory_digest);
        assert_eq!(recovered.fence_generation, request.fence_generation);
        assert!(store
            .unblock(request.id, "operator", "again")
            .await
            .is_err());
        assert!(store
            .claim_specific(request.id, "successor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim recovered request")
            .is_some());

        let inspection = store.inspect(request.id).await.expect("inspect recovery");
        assert!(inspection.checkpoints.iter().any(|checkpoint| {
            checkpoint.unit_key.starts_with("operator_unblock:")
                && checkpoint.detail["unblocked_by"] == "operator"
                && checkpoint.detail["reason"] == "bucket policy repaired"
                && checkpoint.detail["previous_block"] == "operator repair required"
        }));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_claim_and_fence_generation_fail_closed() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        let mut stale = claim.lease.clone();
        stale.generation -= 1;
        assert!(
            store.fence(&stale).await.is_err(),
            "stale lease must reject"
        );
        let mut wrong_community = claim.lease.clone();
        wrong_community.community_id = db
            .ensure_configured_community(&format!(
                "wrong-lease-community-{}.example",
                Uuid::new_v4().simple()
            ))
            .await
            .expect("create unrelated community")
            .id;
        assert!(
            store.begin_quiescing(&wrong_community).await.is_err(),
            "a lease token must remain bound to its durable request community"
        );

        store.begin_quiescing(&claim.lease).await.expect("quiesce");
        let generation = store.fence(&claim.lease).await.expect("fence");
        let mut wrong_fence = claim.lease.clone();
        wrong_fence.fence_generation = Some(generation + 1);
        assert!(
            store.mark_drained(&wrong_fence).await.is_err(),
            "wrong fence generation must reject"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn fence_waits_for_open_write_and_rejects_it_after_transition() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");

        let mut open_write = db
            .begin_transaction()
            .await
            .expect("open write transaction");
        sqlx::query("INSERT INTO pubkey_allowlist (community_id, pubkey) VALUES ($1, $2)")
            .bind(request.community_id.as_uuid())
            .bind(vec![7_u8; 32])
            .execute(&mut *open_write)
            .await
            .expect("write acquires shared deletion lock");

        let store_for_fence = store.clone();
        let lease = claim.lease.clone();
        let fencing = tokio::spawn(async move {
            store_for_fence.begin_quiescing(&lease).await?;
            store_for_fence.fence(&lease).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !fencing.is_finished(),
            "exclusive fence must wait for open writer"
        );
        open_write
            .commit()
            .await
            .expect("pre-fence writer commits first");
        fencing.await.expect("fence task").expect("fence completes");

        assert!(
            db.add_to_allowlist(request.community_id, &[8_u8; 32], &[9_u8; 32], None)
                .await
                .is_err(),
            "post-fence serving write must fail"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn write_assertion_rejects_pinned_snapshot_isolation_before_authorization() {
        let (db, _) = store().await;
        let community = db
            .ensure_configured_community(&format!(
                "isolation-guard-{}.example",
                Uuid::new_v4().simple()
            ))
            .await
            .expect("create community")
            .id;

        for isolation in ["REPEATABLE READ", "SERIALIZABLE"] {
            let mut tx = db.pool.begin().await.expect("begin isolation probe");
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "SET TRANSACTION ISOLATION LEVEL {isolation}"
            )))
            .execute(&mut *tx)
            .await
            .expect("set transaction isolation");
            sqlx::query(
                "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                        set_config('buzz.deletion_fence_generation', '0', true)",
            )
            .bind(community.to_string())
            .execute(&mut *tx)
            .await
            .expect("forge executor authorization");
            let error = sqlx::query("SELECT assert_community_write_allowed($1)")
                .bind(community.as_uuid())
                .execute(&mut *tx)
                .await
                .expect_err("pinned snapshot isolation must fail before authorization");
            assert_eq!(
                error
                    .as_database_error()
                    .and_then(|error| error.code())
                    .as_deref(),
                Some("25000")
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn quiescing_rejects_new_leases_but_renews_admitted_lease_until_release() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        let mut serving = store
            .acquire_serving_write_lease(
                request.community_id,
                "test_external",
                "test-owner",
                DEFAULT_LEASE_DURATION,
            )
            .await
            .expect("serving lease");

        store
            .begin_quiescing(&claim.lease)
            .await
            .expect("persist quiescing");
        assert!(matches!(
            store
                .acquire_serving_write_lease(
                    request.community_id,
                    "late_external",
                    "late-owner",
                    DEFAULT_LEASE_DURATION,
                )
                .await,
            Err(DbError::AccessDenied(_))
        ));
        assert!(store.verify_serving_write_lease(&serving).await.is_ok());
        let lease_until_before_renewal = serving.lease_until;
        tokio::time::sleep(Duration::from_millis(10)).await;
        store
            .renew_serving_write_lease(&mut serving, DEFAULT_LEASE_DURATION)
            .await
            .expect("admitted lease renews while quiescing");
        assert!(
            serving.lease_until > lease_until_before_renewal,
            "renewal must extend the admitted lease"
        );
        assert!(matches!(
            store.fence(&claim.lease).await,
            Err(DbError::ServingWritesNotDrained {
                active_count: 1,
                ..
            })
        ));
        assert!(store
            .release_serving_write_lease(&serving)
            .await
            .expect("release"));
        assert_eq!(store.fence(&claim.lease).await.expect("fence"), 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn sustained_admission_cannot_starve_fence_after_quiescing() {
        let (db, store) = store().await;
        let (request, _) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        store.begin_quiescing(&claim.lease).await.expect("quiesce");

        for attempt in 0..100 {
            assert!(matches!(
                store
                    .acquire_serving_write_lease(
                        request.community_id,
                        "sustained_admission",
                        &format!("owner-{attempt}"),
                        DEFAULT_LEASE_DURATION,
                    )
                    .await,
                Err(DbError::AccessDenied(_))
            ));
        }
        assert_eq!(store.fence(&claim.lease).await.expect("fence"), 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn serving_lease_reaper_is_bounded_and_reports_stats() {
        let (db, store) = store().await;
        let host = format!("lease-reaper-{}.example", Uuid::new_v4().simple());
        let community = db
            .ensure_configured_community(&host)
            .await
            .expect("community")
            .id;
        for owner in ["expired-a", "expired-b", "expired-c"] {
            let lease = store
                .acquire_serving_write_lease(
                    community,
                    "reaper_test",
                    owner,
                    Duration::from_secs(1),
                )
                .await
                .expect("lease");
            sqlx::query("UPDATE community_serving_write_leases SET lease_until = now() - interval '1 second' WHERE id = $1")
                .bind(lease.id)
                .execute(&db.pool)
                .await
                .expect("expire lease");
        }
        let before = store.serving_lease_stats().await.expect("stats before");
        assert!(before.expired >= 3);
        assert_eq!(store.reap_expired_serving_write_leases(2).await.unwrap(), 2);
        let after = store.serving_lease_stats().await.expect("stats after");
        assert_eq!(after.expired, before.expired - 2);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn serving_lease_reaper_remains_global_across_tombstoned_tenant() {
        let (db, store) = store().await;
        let active_a = db
            .ensure_configured_community(&format!("lease-a-{}.example", Uuid::new_v4().simple()))
            .await
            .expect("active A")
            .id;
        let target = db
            .ensure_configured_community(&format!("lease-t-{}.example", Uuid::new_v4().simple()))
            .await
            .expect("target T")
            .id;
        let active_x = db
            .ensure_configured_community(&format!("lease-x-{}.example", Uuid::new_v4().simple()))
            .await
            .expect("active X")
            .id;
        store
            .reap_expired_serving_write_leases(10_000)
            .await
            .expect("clear unrelated expired leases");
        for (community, owner) in [(active_a, "a"), (target, "t"), (active_x, "x")] {
            let lease = store
                .acquire_serving_write_lease(
                    community,
                    "global_reaper_test",
                    owner,
                    DEFAULT_LEASE_DURATION,
                )
                .await
                .expect("acquire lease");
            sqlx::query(
                "UPDATE community_serving_write_leases \
                 SET lease_until = now() - interval '1 second' WHERE id = $1",
            )
            .bind(lease.id)
            .execute(&db.pool)
            .await
            .expect("expire lease");
        }
        let mut lifecycle = db.pool.begin().await.expect("begin target lifecycle");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                    set_config('buzz.deletion_fence_generation', '1', true)",
        )
        .bind(target.to_string())
        .execute(&mut *lifecycle)
        .await
        .expect("authorize tombstone fixture");
        sqlx::query(
            "UPDATE communities SET deletion_state = 'tombstone', \
                    deletion_fence_generation = 1, deleted_at = now() WHERE id = $1",
        )
        .bind(target.as_uuid())
        .execute(&mut *lifecycle)
        .await
        .expect("tombstone target");
        lifecycle.commit().await.expect("commit tombstone fixture");

        assert_eq!(
            store
                .reap_expired_serving_write_leases(10)
                .await
                .expect("global lease reap"),
            3
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn checkpointed_resume_is_idempotent_and_tombstone_blocks_name_reuse() {
        let (db, store) = store().await;
        let (request, inventory) = inventoried_request(&db, &store).await;
        let host = request.community_host.clone();
        let read_state_d_tag = format!("read-state:{}", "a".repeat(32));
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, d_tag) \
             VALUES ($1, $2, $3, now(), 30078, $4, '', $5, $6)",
        )
        .bind(request.community_id.as_uuid())
        .bind(vec![1_u8; 32])
        .bind(vec![2_u8; 32])
        .bind(serde_json::json!([
            ["d", &read_state_d_tag],
            ["t", "read-state"]
        ]))
        .bind(vec![3_u8; 64])
        .bind(&read_state_d_tag)
        .execute(&db.pool)
        .await
        .expect("insert guarded NIP-RS row");
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        store.begin_quiescing(&claim.lease).await.expect("quiesce");
        let generation = store.fence(&claim.lease).await.expect("fence");
        let token = LeaseToken {
            fence_generation: Some(generation),
            ..claim.lease
        };
        store
            .freeze_destructive_storage_manifest(&token, &inventory.storage)
            .await
            .expect("freeze destructive storage");
        store
            .freeze_destructive_storage_manifest(&token, &inventory.storage)
            .await
            .expect("identical destructive manifest retry");
        let mut drifted_storage = inventory.storage.clone();
        let mut drifted_digest = KeyStreamDigest::new();
        drifted_digest
            .fold("media/drifted-after-fence")
            .expect("fold drifted key");
        let (drifted_hex, drifted_count) = drifted_digest.finish();
        drifted_storage.prefixes[0].object_count = drifted_count;
        drifted_storage.prefixes[0].keys_digest = drifted_hex;
        assert!(matches!(
            store
                .freeze_destructive_storage_manifest(&token, &drifted_storage)
                .await,
            Err(DbError::DeletionSafety(_))
        ));
        for mutation in [
            sqlx::query(
                "UPDATE community_deletion_requests \
                 SET destructive_storage_manifest = '{}'::jsonb WHERE id = $1",
            )
            .bind(request.id)
            .execute(&db.pool)
            .await,
            sqlx::query(
                "UPDATE community_deletion_requests \
                 SET destructive_storage_frozen_at = destructive_storage_frozen_at + interval '1 second' \
                 WHERE id = $1",
            )
            .bind(request.id)
            .execute(&db.pool)
            .await,
        ] {
            assert!(
                mutation.is_err(),
                "frozen destructive storage evidence must be immutable"
            );
        }
        store.mark_drained(&token).await.expect("drain");
        store
            .mark_bindings_removed(&token, serde_json::json!({"keys": 0}))
            .await
            .expect("bindings");
        let first = store.purge_postgres(&token).await.expect("purge postgres");
        assert_eq!(first.len(), EXPECTED_SCOPED_TABLES.len());
        assert!(
            store.purge_postgres(&token).await.is_err(),
            "completed stage cannot be replayed under stale checkpoint state"
        );
        store
            .mark_cache_purged(&token, serde_json::json!({"keys": 0}))
            .await
            .expect("cache");
        store
            .verify_postgres_logically_deleted(&token)
            .await
            .expect("logical postgres verify");
        store
            .mark_logically_verified(&token, serde_json::json!({"all": true}))
            .await
            .expect("mark verified");
        store
            .mark_retention_pending(&token, serde_json::json!({"shared_cas": "retained"}))
            .await
            .expect("terminal");

        let terminal = store.get(request.id).await.expect("terminal request");
        assert_eq!(terminal.stage, DeletionStage::RetentionPending);
        let recreated = db
            .create_community_with_owner(
                &host,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .await
            .expect("recreate attempt");
        assert_eq!(recreated, CreateCommunityWithOwnerResult::HostExists);
        assert!(db
            .lookup_community_by_host_for_management(&host)
            .await
            .expect("tombstone lookup")
            .is_some());
        assert!(db
            .lookup_community_by_host(&host)
            .await
            .expect("serving lookup")
            .is_none());
        let direct_delete = sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(request.community_id.as_uuid())
            .execute(&db.pool)
            .await
            .expect_err("tombstone row must be permanent");
        assert!(direct_delete
            .to_string()
            .contains("tombstones are permanent"));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn taxonomy_sweep_uses_database_completion_order() {
        let (_, store) = store().await;
        store
            .record_taxonomy_sweep(Utc::now() + chrono::Duration::minutes(1), 1, 0, &[], 100)
            .await
            .expect("record skewed clean sweep");
        let dirty = store
            .record_taxonomy_sweep(Utc::now(), 1, 1, &["unknown".to_string()], 100)
            .await
            .expect("record later dirty sweep");

        assert_eq!(
            store
                .latest_taxonomy_sweep()
                .await
                .expect("latest sweep")
                .unwrap()
                .id,
            dirty.id
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn manifest_key_chunks_bind_freeze_execution_and_cleanup() {
        let (db, store) = store().await;
        let (request, inventory) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        store.begin_quiescing(&claim.lease).await.expect("quiesce");
        let generation = store.fence(&claim.lease).await.expect("fence");
        let token = LeaseToken {
            fence_generation: Some(generation),
            ..claim.lease
        };

        let meta_prefix = format!("_meta/{}/", request.community_id);
        let keys = vec![
            format!("{meta_prefix}{}.json", "a".repeat(64)),
            format!("{meta_prefix}{}.json", "b".repeat(64)),
        ];
        store
            .append_manifest_key_chunk(&token, 0, &meta_prefix, &keys[..1])
            .await
            .expect("append chunk 0");
        store
            .append_manifest_key_chunk(&token, 1, &meta_prefix, &keys[1..])
            .await
            .expect("append chunk 1");

        // A manifest whose digests do not cover the chunk stream must not freeze.
        assert!(matches!(
            store
                .freeze_destructive_storage_manifest(&token, &inventory.storage)
                .await,
            Err(DbError::DeletionSafety(_))
        ));
        let mut digest = KeyStreamDigest::new();
        for key in &keys {
            digest.fold(key).expect("fold key");
        }
        let (hex_digest, count) = digest.finish();
        let mut storage = inventory.storage.clone();
        storage.prefixes[0].object_count = count;
        storage.prefixes[0].total_bytes = 2;
        storage.prefixes[0].keys_digest = hex_digest;
        store
            .freeze_destructive_storage_manifest(&token, &storage)
            .await
            .expect("freeze manifest matching chunks");

        // Frozen chunks are immutable working data until terminal cleanup.
        assert!(sqlx::query(
            "UPDATE community_deletion_manifest_keys SET keys = '[]'::jsonb \
             WHERE request_id = $1 AND chunk_no = 0",
        )
        .bind(request.id)
        .execute(&db.pool)
        .await
        .is_err());
        assert!(
            sqlx::query("DELETE FROM community_deletion_manifest_keys WHERE request_id = $1")
                .bind(request.id)
                .execute(&db.pool)
                .await
                .is_err()
        );
        assert!(store.clear_manifest_key_chunks(&token).await.is_err());
        assert!(
            sqlx::query(
                "INSERT INTO community_deletion_manifest_keys \
                 (request_id, chunk_no, prefix, keys) VALUES ($1, 2, $2, $3)",
            )
            .bind(request.id)
            .bind(&meta_prefix)
            .bind(sqlx::types::Json(&keys[..1]))
            .execute(&db.pool)
            .await
            .is_err(),
            "the database must reject chunks appended after freeze"
        );

        store.mark_drained(&token).await.expect("drained");
        let first = store
            .next_pending_manifest_chunk(&token)
            .await
            .expect("pending chunk")
            .expect("chunk 0 pending");
        assert_eq!(first.chunk_no, 0);
        assert_eq!(first.keys, keys[..1]);
        store
            .mark_manifest_chunk_deleted(&token, 0, serde_json::json!({"deleted": 1}))
            .await
            .expect("stamp chunk 0");
        assert!(
            matches!(
                store
                    .mark_manifest_chunk_deleted(&token, 0, serde_json::json!({}))
                    .await,
                Err(DbError::DeletionSafety(_))
            ),
            "a chunk stamp is one-way"
        );
        let second = store
            .next_pending_manifest_chunk(&token)
            .await
            .expect("pending chunk")
            .expect("chunk 1 pending after resume");
        assert_eq!(second.chunk_no, 1);
        store
            .mark_manifest_chunk_deleted(&token, 1, serde_json::json!({"deleted": 1}))
            .await
            .expect("stamp chunk 1");
        assert!(store
            .next_pending_manifest_chunk(&token)
            .await
            .expect("pending chunk")
            .is_none());
        assert_eq!(
            store
                .manifest_chunk_progress(request.id)
                .await
                .expect("progress"),
            (2, 2)
        );

        store
            .mark_bindings_removed(&token, serde_json::json!({"deleted_keys": 2}))
            .await
            .expect("bindings removed");
        store.purge_postgres(&token).await.expect("purge postgres");
        store
            .mark_cache_purged(&token, serde_json::json!({"keys": 0}))
            .await
            .expect("cache purged");
        store
            .verify_postgres_logically_deleted(&token)
            .await
            .expect("verify postgres");
        store
            .mark_logically_verified(&token, serde_json::json!({"all": true}))
            .await
            .expect("logically verified");
        assert_eq!(
            store
                .manifest_chunk_progress(request.id)
                .await
                .expect("progress after terminal cleanup"),
            (0, 0)
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn destructive_stages_serialize_with_migrations_and_fail_closed_on_new_scoped_tables() {
        // The probe table below mutates the live catalog, which every other
        // test in the shared database validates against. Run the whole
        // scenario in a dedicated database so concurrent purge/verify tests
        // never observe the drifted surface; advisory locks are also
        // per-database, so the parked migration lock cannot stall them.
        let base_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
        let admin = PgPool::connect(&base_url)
            .await
            .expect("connect admin database");
        let probe_db = format!("buzz_lock_probe_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {probe_db}")))
            .execute(&admin)
            .await
            .expect("create probe database");
        let (base_prefix, _) = base_url.rsplit_once('/').expect("database url has a path");
        let db = Db::new(&DbConfig {
            database_url: format!("{base_prefix}/{probe_db}"),
            max_connections: 5,
            min_connections: 0,
            ..DbConfig::default()
        })
        .await
        .expect("connect probe database");
        db.migrate().await.expect("migrate probe database");
        let store = db.deletion_store();
        let (request, inventory) = inventoried_request(&db, &store).await;
        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        store.begin_quiescing(&claim.lease).await.expect("quiesce");
        let generation = store.fence(&claim.lease).await.expect("fence");
        let token = LeaseToken {
            fence_generation: Some(generation),
            ..claim.lease
        };
        store
            .freeze_destructive_storage_manifest(&token, &inventory.storage)
            .await
            .expect("freeze destructive storage");
        store.mark_drained(&token).await.expect("drain");
        store
            .mark_bindings_removed(&token, serde_json::json!({"keys": 0}))
            .await
            .expect("bindings");

        // Park a migration mid-run through the production lock path: the op
        // runs on the same connection that owns the exclusive session lock,
        // exactly as `run_migrations` executes migration SQL.
        let probe_table = format!("deletion_probe_{}", Uuid::new_v4().simple());
        let create_probe =
            format!("CREATE TABLE {probe_table} (community_id UUID NOT NULL, payload TEXT)");
        let attach_probe =
            format!("SELECT attach_community_write_fence('{probe_table}'::regclass)");
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let migration_pool = db.pool.clone();
        let (create_probe_sql, attach_probe_sql) = (create_probe.clone(), attach_probe.clone());
        let migration_run = tokio::spawn(async move {
            crate::migration::with_exclusive_schema_destruction_lock(
                &migration_pool,
                move |mut conn| async move {
                    let _ = started_tx.send(());
                    let _ = release_rx.await;
                    // Explicit DDL transaction: the new scoped table commits
                    // before the production path releases the exclusive lock.
                    let outcome: Result<()> = async {
                        let mut ddl = sqlx::Connection::begin(&mut conn).await?;
                        sqlx::query(AssertSqlSafe(create_probe_sql))
                            .execute(&mut *ddl)
                            .await?;
                        sqlx::query(AssertSqlSafe(attach_probe_sql))
                            .execute(&mut *ddl)
                            .await?;
                        ddl.commit().await?;
                        Ok(())
                    }
                    .await;
                    (conn, outcome)
                },
            )
            .await
        });
        started_rx.await.expect("parked migration holds the lock");
        let blocked =
            tokio::time::timeout(Duration::from_millis(750), store.purge_postgres(&token)).await;
        assert!(
            blocked.is_err(),
            "purge must wait for the in-flight migration instead of validating a stale surface"
        );

        // The migration commits its new fenced scoped table, then finishes.
        release_tx.send(()).expect("unpark migration");
        migration_run
            .await
            .expect("join migration run")
            .expect("locked migration op");

        // Purge revalidates inside its own transaction and fails closed on
        // the surface this executor does not know.
        let denied = store.purge_postgres(&token).await;
        let denied_on_probe = matches!(
            &denied,
            Err(DbError::DeletionSafety(message)) if message.contains(&probe_table)
        );
        sqlx::query(AssertSqlSafe(format!("DROP TABLE {probe_table}")))
            .execute(&db.pool)
            .await
            .expect("drop probe table");
        assert!(
            denied_on_probe,
            "purge must fail closed on a migration-committed scoped table: {denied:?}"
        );
        let after_denied = store.get(request.id).await.expect("request after denial");
        assert_eq!(after_denied.stage, DeletionStage::BindingsRemoved);

        store
            .purge_postgres(&token)
            .await
            .expect("purge after catalog restored");
        store
            .mark_cache_purged(&token, serde_json::json!({"keys": 0}))
            .await
            .expect("cache");

        // A scoped table committed after the purge must fail the absence
        // proof closed rather than silently escaping verification.
        sqlx::query(AssertSqlSafe(create_probe))
            .execute(&db.pool)
            .await
            .expect("recreate probe scoped table");
        sqlx::query(AssertSqlSafe(attach_probe))
            .execute(&db.pool)
            .await
            .expect("attach probe fence again");
        let verify_denied = store.verify_postgres_logically_deleted(&token).await;
        let verify_denied_on_probe = matches!(
            &verify_denied,
            Err(DbError::DeletionSafety(message)) if message.contains(&probe_table)
        );
        sqlx::query(AssertSqlSafe(format!("DROP TABLE {probe_table}")))
            .execute(&db.pool)
            .await
            .expect("drop probe table again");
        assert!(
            verify_denied_on_probe,
            "verification must fail closed on a post-purge scoped table: {verify_denied:?}"
        );

        store
            .verify_postgres_logically_deleted(&token)
            .await
            .expect("verify after catalog restored");
        store
            .mark_logically_verified(&token, serde_json::json!({"all": true}))
            .await
            .expect("mark verified");
        store
            .mark_retention_pending(&token, serde_json::json!({"probe": "clean"}))
            .await
            .expect("terminal");

        db.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {probe_db} WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop probe database");
    }

    /// A database bootstrapped from `schema/schema.sql` (the pgschema
    /// desired-state path — no migrations) must carry the complete 0028
    /// deletion surface and run a deletion through every stage.
    ///
    /// Before the parity restoration this wedged post-fence:
    /// `freeze_destructive_storage_manifest` hit the missing
    /// `community_deletion_manifest_keys` relation only after the write fence
    /// was already up, leaving the request with no forward path — and even a
    /// hand-created table would have lacked the immutability guard trigger.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn desired_state_schema_bootstrap_progresses_beyond_fencing() {
        let base_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
        let admin = PgPool::connect(&base_url)
            .await
            .expect("connect admin database");
        let probe_db = format!("buzz_desired_state_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {probe_db}")))
            .execute(&admin)
            .await
            .expect("create probe database");
        let (base_prefix, _) = base_url.rsplit_once('/').expect("database url has a path");
        let probe_url = format!("{base_prefix}/{probe_db}");

        let schema_sql = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/schema.sql"),
        )
        .expect("read schema/schema.sql");
        let bootstrap = PgPool::connect(&probe_url)
            .await
            .expect("connect probe database");
        sqlx::raw_sql(AssertSqlSafe(schema_sql))
            .execute(&bootstrap)
            .await
            .expect("apply desired-state schema");
        bootstrap.close().await;

        let db = Db::new(&DbConfig {
            database_url: probe_url,
            max_connections: 5,
            min_connections: 0,
            ..DbConfig::default()
        })
        .await
        .expect("connect desired-state database");
        let store = db.deletion_store();
        let (request, inventory) = inventoried_request(&db, &store).await;

        // The immutability guard must exist and enforce: chunk rows are
        // rejected outside an unfrozen fenced request (this request is still
        // `inventoried`).
        let premature_chunk = sqlx::query(
            "INSERT INTO community_deletion_manifest_keys (request_id, chunk_no, prefix, keys) \
             VALUES ($1, 0, '_meta/premature/', '[]'::jsonb)",
        )
        .bind(request.id)
        .execute(&db.pool)
        .await;
        let guard_enforced = matches!(
            &premature_chunk,
            Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23000")
        );
        assert!(
            guard_enforced,
            "manifest-keys immutability guard must reject pre-fence chunks \
             with integrity_constraint_violation: {premature_chunk:?}"
        );

        store
            .approve(request.id, "approver", None)
            .await
            .expect("approve");
        let claim = store
            .claim_specific(request.id, "executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim")
            .expect("won claim");
        store.begin_quiescing(&claim.lease).await.expect("quiesce");
        let generation = store.fence(&claim.lease).await.expect("fence");
        let token = LeaseToken {
            fence_generation: Some(generation),
            ..claim.lease
        };
        // The previously wedging stage: first touch of the manifest-keys
        // relation happens here, after the fence is already up.
        store
            .freeze_destructive_storage_manifest(&token, &inventory.storage)
            .await
            .expect("freeze destructive storage on desired-state bootstrap");
        store.mark_drained(&token).await.expect("drain");
        store
            .mark_bindings_removed(&token, serde_json::json!({"keys": 0}))
            .await
            .expect("bindings");
        store.purge_postgres(&token).await.expect("purge postgres");
        store
            .mark_cache_purged(&token, serde_json::json!({"keys": 0}))
            .await
            .expect("cache");
        store
            .verify_postgres_logically_deleted(&token)
            .await
            .expect("verify postgres");
        store
            .mark_logically_verified(&token, serde_json::json!({"all": true}))
            .await
            .expect("logically verified");
        store
            .mark_retention_pending(&token, serde_json::json!({"bootstrap": "desired-state"}))
            .await
            .expect("terminal");
        let terminal = store.get(request.id).await.expect("terminal request");
        assert_eq!(terminal.stage, DeletionStage::RetentionPending);

        db.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {probe_db} WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop probe database");
    }
}
