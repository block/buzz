//! Bounded account-table cleanup for one community, shared by CLI and workflow.
//! Deletion assumes relay and external writers are stopped.

use super::{normalize_user_deletion_pubkey, ExecutionMode, UserDeletionStore};
use crate::{DbError, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::io::Write;
use uuid::Uuid;

mod community_memberships;
mod memberships;
mod operations;
mod ownership;
mod scope;
use operations::{Operation, GLOBAL_OPERATIONS, OPERATIONS};
pub use scope::{AccountCommunityScope, AccountScope};

#[cfg(test)]
mod postgres_tests;

/// Scope and output bounds for the offline account command.
pub struct AccountBatchOptions {
    /// Exactly one externally selected surviving community.
    pub community: Uuid,
    /// Human public key, hex or npub. Normalized before any query.
    pub pubkey: String,
    /// Maximum rows per transaction, 1..=100.
    pub batch_size: u16,
    /// Maximum row operations in one invocation, 1..=1,000,000.
    pub max_items: u32,
}

/// Explicit deployment-wide cleanup; never implied by a community invocation.
pub struct GlobalAccountBatchOptions {
    /// Exactly one human or agent key, hex or npub.
    pub pubkey: String,
    /// Operator performing the offline action, recorded for role revocations.
    pub actor: String,
    /// Maximum rows per transaction, 1..=100.
    pub batch_size: u16,
    /// Maximum row operations per invocation, 1..=1,000,000.
    pub max_items: u32,
}

struct Options {
    target: String,
    agents: Vec<String>,
    community: Option<Uuid>,
    pubkey: String,
    actor: Option<String>,
    batch_size: u16,
    max_items: u32,
}

impl Options {
    fn scope_sql(&self) -> &'static str {
        if self.community.is_some() {
            "community_id=$1"
        } else {
            "$1::uuid IS NULL"
        }
    }
}

/// Outcome limited to the explicitly supported account-table operations.
#[derive(Debug, Serialize)]
pub struct AccountBatchSummary {
    record: &'static str,
    /// True only in print-only mode.
    pub read_only: bool,
    /// Proposed row operations in preview; committed operations in delete mode.
    pub items: u32,
    /// Number of nonempty batches printed or committed.
    pub batches: u32,
    /// False if the invocation limit stopped before all supported operations.
    pub supported_scope_exhausted: bool,
    /// Always false: profile, shared-state, device and access verification remain.
    pub account_erasure_complete: bool,
}

impl UserDeletionStore {
    /// Clean global listeners, feedback and DB roles for one explicitly selected
    /// key. The caller supplies the audit actor. No workflow stage can imply this
    /// deployment-wide authority. Config-backed roles must be removed separately.
    pub async fn run_global_account_batches(
        &self,
        options: &GlobalAccountBatchOptions,
        mode: ExecutionMode,
        writer: &mut impl Write,
    ) -> Result<AccountBatchSummary> {
        let options = Options {
            target: normalize_user_deletion_pubkey(&options.pubkey)?,
            agents: Vec::new(),
            community: None,
            pubkey: normalize_user_deletion_pubkey(&options.pubkey)?,
            actor: Some(normalize_user_deletion_pubkey(&options.actor)?),
            batch_size: options.batch_size,
            max_items: options.max_items,
        };
        bounds(options.batch_size, options.max_items)?;
        self.run_batches(&options, GLOBAL_OPERATIONS, mode, writer, None)
            .await
    }

    async fn run_batches(
        &self,
        options: &Options,
        operations: &[Operation],
        mode: ExecutionMode,
        writer: &mut impl Write,
        stage: Option<(&crate::entity_deletion::EntityLease, Uuid)>,
    ) -> Result<AccountBatchSummary> {
        let delete = mode == ExecutionMode::Delete;
        let mut summary = AccountBatchSummary {
            record: "account_batches_summary",
            read_only: !delete,
            items: 0,
            batches: 0,
            supported_scope_exhausted: true,
            account_erasure_complete: false,
        };
        if delete {
            let mut start_tx = self.pool.begin().await?;
            timeouts(&mut start_tx).await?;
            start(&mut start_tx, options, writer, true).await?;
            start_tx.commit().await?;
            for op in operations {
                if op.table == "channel_members" && op.action == "delete" {
                    self.delete_memberships(options, op, writer, stage, &mut summary)
                        .await?;
                    if !summary.supported_scope_exhausted {
                        break;
                    }
                    continue;
                }
                if op.table == "relay_members" && op.action == "delete" {
                    self.delete_community_membership(options, op, writer, stage, &mut summary)
                        .await?;
                    if !summary.supported_scope_exhausted {
                        break;
                    }
                    continue;
                }
                loop {
                    let mut tx = self.pool.begin().await?;
                    timeouts(&mut tx).await?;
                    if let Some((lease, id)) = stage {
                        validate_stage(&mut tx, lease, id, options).await?;
                    }
                    ownership::recheck(&mut tx, options, true).await?;
                    if op.table == "relay_operators" {
                        crate::store::relay_operators::acquire_roster_lock(&mut tx).await?;
                    }
                    let keys = select(&mut tx, options, op, None, true, &summary).await?;
                    if keys.is_empty() {
                        tx.commit().await?;
                        break;
                    }
                    if summary.items == options.max_items {
                        summary.supported_scope_exhausted = false;
                        tx.rollback().await?;
                        break;
                    }
                    batch(writer, options, op, &keys, &summary)?;
                    mutate(&mut tx, options, op, &keys).await?;
                    if let Some((lease, id)) = stage {
                        validate_stage(&mut tx, lease, id, options).await?;
                    }
                    tx.commit().await?;
                    summary.items += keys.len() as u32;
                    summary.batches += 1;
                    line(
                        writer,
                        &json!({"record":"batch_committed", "number":summary.batches,
                        "table":op.table,"action":op.action,"items":keys.len()}),
                    )?;
                }
                if !summary.supported_scope_exhausted {
                    break;
                }
            }
            // Independent absence checks; never infer completion from counters.
            if summary.supported_scope_exhausted {
                let mut verify_tx = self.pool.begin().await?;
                timeouts(&mut verify_tx).await?;
                for op in operations {
                    if !select(&mut verify_tx, options, op, None, false, &summary)
                        .await?
                        .is_empty()
                    {
                        return Err(invalid("supported rows remain; rerun to resume"));
                    }
                }
                verify_tx.commit().await?;
            }
        } else {
            let mut tx = self.pool.begin().await?;
            timeouts(&mut tx).await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
                .execute(&mut *tx)
                .await?;
            ownership::recheck(&mut tx, options, false).await?;
            start(&mut tx, options, writer, false).await?;
            for op in operations {
                let mut cursor = None;
                loop {
                    let keys =
                        select(&mut tx, options, op, cursor.as_ref(), false, &summary).await?;
                    if keys.is_empty() {
                        break;
                    }
                    if summary.items == options.max_items {
                        summary.supported_scope_exhausted = false;
                        break;
                    }
                    batch(writer, options, op, &keys, &summary)?;
                    cursor = keys.last().cloned();
                    summary.items += keys.len() as u32;
                    summary.batches += 1;
                }
                if !summary.supported_scope_exhausted {
                    break;
                }
            }
            tx.commit().await?;
        }
        line(writer, &summary)?;
        Ok(summary)
    }
}

fn bounds(batch_size: u16, max_items: u32) -> Result<()> {
    if !(1..=100).contains(&batch_size) || !(1..=1_000_000).contains(&max_items) {
        return Err(invalid(
            "batch-size must be 1..100 and max-items 1..1000000",
        ));
    }
    Ok(())
}

fn validated(options: &AccountBatchOptions) -> Result<Options> {
    bounds(options.batch_size, options.max_items)?;
    Ok(Options {
        target: normalize_user_deletion_pubkey(&options.pubkey)?,
        agents: Vec::new(),
        community: Some(options.community),
        actor: None,
        pubkey: normalize_user_deletion_pubkey(&options.pubkey)?,
        batch_size: options.batch_size,
        max_items: options.max_items,
    })
}

async fn start(
    conn: &mut PgConnection,
    options: &Options,
    writer: &mut impl Write,
    delete: bool,
) -> Result<()> {
    if let Some(community) = options.community {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM communities WHERE id=$1)")
                .bind(community)
                .fetch_one(&mut *conn)
                .await?;
        if !exists {
            return Err(invalid("community does not exist"));
        }
    }
    let (archives, watermarks): (i64,i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM archived_identities WHERE community_id=$1 AND (pubkey=$2 OR actor=$2)),
                (SELECT count(*) FROM parameterized_event_watermarks WHERE community_id=$1 AND pubkey=decode($2,'hex'))")
        .bind(options.community).bind(&options.pubkey).fetch_one(conn).await?;
    line(
        writer,
        &json!({"record":"account_batches_start","read_only":!delete,"community":options.community,
        "pubkey":options.pubkey,"batch_size":options.batch_size,"max_items":options.max_items,
        "scope":if options.community.is_some() {"community"} else {"deployment_global"},
        "actor":options.actor,
        "deferred":{"archived_identities":archives},
        "retained":{"parameterized_event_watermarks":watermarks,
            "policy":["replay protection and deletion evidence","shared creator and repository routing references",
                "moderation, enforcement and audit history","shared or uncertain device registrations",
                "historical superseded signed membership snapshots"]},
        "unsupported":["profile cleanup","remaining workflow handlers",
            "device ownership attribution",
            "repository and config-backed authority revocation","cache and live-session invalidation"],
        "account_erasure_complete":false}),
    )
}

async fn select(
    conn: &mut PgConnection,
    options: &Options,
    op: &Operation,
    cursor: Option<&Value>,
    lock: bool,
    summary: &AccountBatchSummary,
) -> Result<Vec<Value>> {
    let batch_size = if op.table == "channel_members" && op.action == "delete" {
        1
    } else {
        options.batch_size
    };
    let limit = u32::from(batch_size).min(options.max_items.saturating_sub(summary.items).max(1));
    let lock = if lock { " FOR UPDATE" } else { "" };
    let (order, after) = op.pagination();
    let sql = format!("SELECT {} FROM {} WHERE {} AND ({}) AND ($3::jsonb IS NULL OR {after}) ORDER BY {order} LIMIT $4{lock}", op.key, op.table, options.scope_sql(), op.selector);
    // All interpolated fragments come exclusively from OPERATIONS.
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(options.community)
        .bind(&options.pubkey)
        .bind(cursor)
        .bind(i64::from(limit))
        .fetch_all(conn)
        .await?)
}

async fn mutate(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    options: &Options,
    op: &Operation,
    keys: &[Value],
) -> Result<()> {
    if op.table == "relay_operators" {
        let target = hex::decode(&options.pubkey).map_err(|_| invalid("invalid target"))?;
        let actor = hex::decode(
            options
                .actor
                .as_deref()
                .ok_or_else(|| invalid("missing audit actor"))?,
        )
        .map_err(|_| invalid("invalid actor"))?;
        // Conservatively require a surviving DB operator: local CLI environment
        // is not evidence of the configuration on every serving relay.
        if !crate::store::relay_operators::remove_in_transaction(tx, &target, &actor, false).await?
        {
            return Err(invalid("selected role changed; current batch rolled back"));
        }
        return Ok(());
    }
    let conn = &mut **tx;
    if op.table == "events" {
        // Migration-created NIP-RS guard requires this supported opt-in. The
        // desired-state schema no longer installs the legacy guard.
        sqlx::query("SELECT set_config('buzz.nip_rs_hard_delete','on',true)")
            .execute(&mut *conn)
            .await?;
    }
    let verb = match op.update {
        Some(update) => update.to_owned(),
        None => format!("DELETE FROM {}", op.table),
    };
    let sql = format!(
        "{verb} WHERE {} AND ({}) AND ({})",
        options.scope_sql(),
        op.selector,
        op.identity
    );
    for key in keys {
        let changed = sqlx::query(sqlx::AssertSqlSafe(sql.clone()))
            .bind(options.community)
            .bind(&options.pubkey)
            .bind(key)
            .execute(&mut *conn)
            .await?
            .rows_affected();
        if changed != 1 {
            return Err(invalid("selected row changed; current batch rolled back"));
        }
    }
    Ok(())
}

fn batch(
    writer: &mut impl Write,
    options: &Options,
    op: &Operation,
    keys: &[Value],
    summary: &AccountBatchSummary,
) -> Result<()> {
    line(
        writer,
        &json!({"record":"proposed_batch","number":summary.batches+1,"community":options.community,
        "table":op.table,"action":op.action,"keys":keys,
        "rebuild_saved_channel_lists":op.table == "channel_members" && op.action == "delete"}),
    )
}

fn invalid(message: impl Into<String>) -> DbError {
    DbError::DeletionSafety(message.into())
}

fn line(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 256 * 1024 {
        return Err(invalid("batch output exceeds 256 KiB; reduce batch-size"));
    }
    writer
        .write_all(&bytes)
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
        .map_err(|error| invalid(format!("batch output failed: {error}")))
}

async fn validate_stage(
    tx: &mut PgConnection,
    lease: &crate::entity_deletion::EntityLease,
    stage_id: Uuid,
    options: &Options,
) -> Result<()> {
    use crate::store::entity_deletion::{lock_owned, validate_approved_inventory};
    let request = lock_owned(tx, lease).await?;
    validate_approved_inventory(tx, &request).await?;
    let inputs =
        super::UserDeletionInventory::read(&request)?.inputs(&request.target, options.community)?;
    if request.status != "running"
        || normalize_user_deletion_pubkey(&request.target)? != options.target
    {
        return Err(invalid("account stage target or status changed"));
    }
    let matches: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM entity_deletion_stage_history
         WHERE id=$1 AND request_id=$2 AND stage='purge_account' AND is_current
         AND status='running' AND target_community_id=$3 AND metadata_json->'inputs'=$4)",
    )
    .bind(stage_id)
    .bind(lease.request_id)
    .bind(options.community)
    .bind(inputs)
    .fetch_one(&mut *tx)
    .await?;
    if !matches {
        return Err(invalid(
            "account stage is not current or outside approved scope",
        ));
    }
    Ok(())
}

async fn timeouts(conn: &mut PgConnection) -> Result<()> {
    sqlx::query("SET LOCAL statement_timeout='30s'")
        .execute(&mut *conn)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='3s'")
        .execute(conn)
        .await?;
    Ok(())
}
