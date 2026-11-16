//! Offline account-table batches for one community. This is not a workflow
//! completion API: it never starts stages, updates progress, or touches caches.

use super::{normalize_user_deletion_pubkey, ExecutionMode, UserDeletionStore};
use crate::{DbError, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{Connection, PgConnection};
use std::io::Write;
use uuid::Uuid;

mod guards;
mod operations;
use operations::{Operation, OPERATIONS};

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
    /// Always false: archive/replay, moderation, devices and global work remain.
    pub account_erasure_complete: bool,
}

impl UserDeletionStore {
    /// Stream proposed account operations from one repeatable read-only snapshot.
    /// Does not lock rows, mutate data, migrate, or record workflow progress.
    pub async fn preview_account_batches(
        &self,
        options: &AccountBatchOptions,
        writer: &mut impl Write,
    ) -> Result<AccountBatchSummary> {
        self.run_account_batches(options, ExecutionMode::ReadOnly, writer)
            .await
    }

    /// Print and commit bounded offline account operations. Stop the relay and
    /// external workers first. Session locks span batches; cancellation closes
    /// the connection. Earlier committed batches survive errors and reruns
    /// select remaining work. No workflow stage is completed by this method.
    pub async fn delete_account_batches(
        &self,
        options: &AccountBatchOptions,
        writer: &mut impl Write,
    ) -> Result<AccountBatchSummary> {
        self.run_account_batches(options, ExecutionMode::Delete, writer)
            .await
    }

    /// Run the same bounded account planner in either standalone mode.
    pub async fn run_account_batches(
        &self,
        options: &AccountBatchOptions,
        mode: ExecutionMode,
        writer: &mut impl Write,
    ) -> Result<AccountBatchSummary> {
        self.account_batches(options, writer, mode).await
    }

    async fn account_batches(
        &self,
        options: &AccountBatchOptions,
        writer: &mut impl Write,
        mode: ExecutionMode,
    ) -> Result<AccountBatchSummary> {
        let options = validated(options)?;
        let delete = mode == ExecutionMode::Delete;
        // A session lock must never be returned to the pool on error/cancellation.
        let mut conn = self.pool.acquire().await?.detach();
        sqlx::query("SET statement_timeout='30s'")
            .execute(&mut conn)
            .await?;
        sqlx::query("SET lock_timeout='3s'")
            .execute(&mut conn)
            .await?;
        if delete {
            sqlx::query("SELECT pg_advisory_lock_shared($1)")
                .bind(crate::store::deletion::SCHEMA_DESTRUCTION_LOCK_KEY)
                .execute(&mut conn)
                .await?;
            sqlx::query("SELECT pg_advisory_lock(community_deletion_lock_key($1))")
                .bind(options.community)
                .execute(&mut conn)
                .await?;
            crate::store::deletion::validate_catalog_on(&mut conn).await?;
            guards::community(&mut conn, &options).await?;
        }
        let mut summary = AccountBatchSummary {
            record: "account_batches_summary",
            read_only: !delete,
            items: 0,
            batches: 0,
            supported_scope_exhausted: true,
            account_erasure_complete: false,
        };
        if delete {
            start(&mut conn, &options, writer, true).await?;
            for op in OPERATIONS {
                loop {
                    let mut tx = conn.begin().await?;
                    guards::community(&mut tx, &options).await?;
                    guards::parent(&mut tx, &options, op).await?;
                    let keys = select(&mut tx, &options, op, None, true, &summary).await?;
                    if keys.is_empty() {
                        tx.commit().await?;
                        break;
                    }
                    if summary.items == options.max_items {
                        summary.supported_scope_exhausted = false;
                        tx.rollback().await?;
                        break;
                    }
                    batch(writer, &options, op, &keys, &summary)?;
                    mutate(&mut tx, &options, op, &keys).await?;
                    guards::community(&mut tx, &options).await?;
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
                for op in OPERATIONS {
                    if !select(&mut conn, &options, op, None, false, &summary)
                        .await?
                        .is_empty()
                    {
                        return Err(invalid("supported rows remain; rerun to resume"));
                    }
                }
            }
        } else {
            let mut tx = conn.begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
                .execute(&mut *tx)
                .await?;
            start(&mut tx, &options, writer, false).await?;
            for op in OPERATIONS {
                let mut cursor = None;
                loop {
                    let keys =
                        select(&mut tx, &options, op, cursor.as_ref(), false, &summary).await?;
                    if keys.is_empty() {
                        break;
                    }
                    if summary.items == options.max_items {
                        summary.supported_scope_exhausted = false;
                        break;
                    }
                    batch(writer, &options, op, &keys, &summary)?;
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
        conn.close().await?;
        line(writer, &summary)?;
        Ok(summary)
    }
}

fn validated(options: &AccountBatchOptions) -> Result<AccountBatchOptions> {
    if !(1..=100).contains(&options.batch_size) || !(1..=1_000_000).contains(&options.max_items) {
        return Err(invalid(
            "batch-size must be 1..100 and max-items 1..1000000",
        ));
    }
    Ok(AccountBatchOptions {
        community: options.community,
        pubkey: normalize_user_deletion_pubkey(&options.pubkey)?,
        batch_size: options.batch_size,
        max_items: options.max_items,
    })
}

async fn start(
    conn: &mut PgConnection,
    options: &AccountBatchOptions,
    writer: &mut impl Write,
    delete: bool,
) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM communities WHERE id=$1)")
        .bind(options.community)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Err(invalid("community does not exist"));
    }
    let (archives, watermarks): (i64,i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM archived_identities WHERE community_id=$1 AND (pubkey=$2 OR actor=$2)),
                (SELECT count(*) FROM parameterized_event_watermarks WHERE community_id=$1 AND pubkey=decode($2,'hex'))")
        .bind(options.community).bind(&options.pubkey).fetch_one(conn).await?;
    line(
        writer,
        &json!({"record":"account_batches_start","read_only":!delete,"community":options.community,
        "pubkey":options.pubkey,"batch_size":options.batch_size,"max_items":options.max_items,
        "deferred":{"archived_identities":archives,"parameterized_event_watermarks":watermarks},
        "unsupported":["archive visibility and required actor erasure","replay watermark removal",
            "moderation and audit erasure","global account records and device attribution",
            "cache and live-session invalidation","durable workflow integration"],
        "account_erasure_complete":false}),
    )
}

async fn select(
    conn: &mut PgConnection,
    options: &AccountBatchOptions,
    op: &Operation,
    cursor: Option<&Value>,
    lock: bool,
    summary: &AccountBatchSummary,
) -> Result<Vec<Value>> {
    let limit =
        u32::from(options.batch_size).min(options.max_items.saturating_sub(summary.items).max(1));
    let lock = if lock { " FOR UPDATE" } else { "" };
    let (order, after) = op.pagination();
    let sql = format!("SELECT {} FROM {} WHERE community_id=$1 AND ({}) AND ($3::jsonb IS NULL OR {after}) ORDER BY {order} LIMIT $4{lock}", op.key, op.table, op.selector);
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
    conn: &mut PgConnection,
    options: &AccountBatchOptions,
    op: &Operation,
    keys: &[Value],
) -> Result<()> {
    let verb = match op.update {
        Some(update) => format!("UPDATE {} SET {update}", op.table),
        None => format!("DELETE FROM {}", op.table),
    };
    let sql = format!(
        "{verb} WHERE community_id=$1 AND ({}) AND ({})",
        op.selector, op.identity
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
    options: &AccountBatchOptions,
    op: &Operation,
    keys: &[Value],
    summary: &AccountBatchSummary,
) -> Result<()> {
    line(
        writer,
        &json!({"record":"proposed_batch","number":summary.batches+1,"community":options.community,
        "table":op.table,"action":op.action,"keys":keys}),
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

#[cfg(test)]
mod postgres_tests;
