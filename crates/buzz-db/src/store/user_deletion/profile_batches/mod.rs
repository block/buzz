//! Offline profile erasure. Preview and deletion share the exact immutable plan.
//! Writers must remain stopped until the caller's full deletion verification ends.
use super::{content_batches::ContentRowCounts, normalize_user_deletion_pubkey, UserDeletionStore};
use crate::{DbError, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{Connection, PgConnection};
use std::io::Write;
use uuid::Uuid;

mod events;
mod ownership;
mod plan;
#[cfg(test)]
mod postgres_tests;
mod rows;

/// Explicit scope for standalone profile preview and deletion.
pub struct ProfileBatchOptions {
    /// Surviving community whose profiles are selected.
    pub community: Uuid,
    /// Human key as hex or npub.
    pub pubkey: String,
    /// Explicitly trusted current/historical relay signing keys.
    pub relay_pubkeys: Vec<String>,
    /// Row mutations per transaction, 1..=100.
    pub batch_size: u16,
    /// Maximum planned mutations, 1..=1,000,000; overflow makes no changes.
    pub max_items: u32,
}

/// Result for this profile scope, never a claim of complete account erasure.
#[derive(Debug, Serialize)]
pub struct ProfileBatchSummary {
    /// Action counts returned by the workflow profile preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<ProfileCounts>,
    /// preview_complete, preview_with_prerequisites, or delete_complete.
    pub record: &'static str,
    /// Proposed or committed row mutations.
    pub items: usize,
    /// Proposed or committed transactions.
    pub batches: usize,
    /// Human and current community agents discovered for this invocation.
    pub target_pubkeys: Vec<String>,
    /// Always false; the independent final verifier owns that claim.
    pub account_erasure_complete: bool,
    /// Observed prerequisites, still present because prior previews do not mutate data.
    pub prerequisites: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct ProfileCounts {
    targets: Vec<ProfileTarget>,
    delete: ContentRowCounts,
    replace_archive_snapshot: ContentRowCounts,
}

#[derive(Debug, Serialize)]
struct ProfileTarget {
    pubkey: String,
    role: &'static str,
    delete_users_row: bool,
}

impl ProfileCounts {
    fn from_plan(plan: &Plan) -> Self {
        let mut counts = Self::default();
        let mut selected_users = std::collections::HashSet::new();
        for op in &plan.ops {
            if op.table.name == "users" {
                selected_users.insert(&op.key["pubkey"]);
            }
            let group = if op.snapshot.is_some() {
                &mut counts.replace_archive_snapshot
            } else {
                &mut counts.delete
            };
            group.rows += 1;
            *group.by_table.entry(op.table.name.to_owned()).or_default() += 1;
            if let Some(kind) = op.event_kind {
                *group.events_by_kind.entry(i64::from(kind)).or_default() += 1;
            }
        }
        counts.targets = plan
            .scope
            .targets
            .iter()
            .map(|key| ProfileTarget {
                pubkey: key.clone(),
                role: if key == &plan.scope.human {
                    "human"
                } else {
                    "owned_agent"
                },
                delete_users_row: selected_users.contains(&Value::String(format!("\\x{key}"))),
            })
            .collect();
        counts
    }
}

#[derive(Clone)]
struct Scope {
    community: Uuid,
    human: String,
    targets: Vec<String>,
    bytes: Vec<Vec<u8>>,
    relays: Vec<String>,
}

struct Plan {
    scope: Scope,
    ops: Vec<rows::Operation>,
    blockers: Vec<String>,
}

impl UserDeletionStore {
    /// Preview or execute a standalone profile plan with stopped writers.
    ///
    /// No workflow is enabled, lease acquired or deletion journal modified.
    /// `relay_secret` is required only to replace a shared archive snapshot;
    /// it must match that snapshot's trusted signer and is never printed.
    pub async fn run_profile_batches(
        &self,
        options: &ProfileBatchOptions,
        delete: bool,
        relay_secret: Option<&str>,
        writer: &mut impl Write,
    ) -> Result<ProfileBatchSummary> {
        self.profile_batches(options, delete, relay_secret, false, writer)
            .await
    }

    /// Inspect the same profile plan while reporting unmet prerequisites as data.
    /// This cannot delete, sign replacements, or certify erasure. Query and planning
    /// errors still fail; only successfully inspected prerequisites are reported.
    pub async fn preview_profile_plan(
        &self,
        options: &ProfileBatchOptions,
        writer: &mut impl Write,
    ) -> Result<ProfileBatchSummary> {
        self.profile_batches(options, false, None, true, writer)
            .await
    }

    async fn profile_batches(
        &self,
        options: &ProfileBatchOptions,
        delete: bool,
        relay_secret: Option<&str>,
        report_prerequisites: bool,
        writer: &mut impl Write,
    ) -> Result<ProfileBatchSummary> {
        if !(1..=100).contains(&options.batch_size) || !(1..=1_000_000).contains(&options.max_items)
        {
            return Err(invalid(
                "batch-size must be 1..100 and max-items 1..1000000",
            ));
        }
        let mut conn = self.pool.acquire().await?.detach();
        // Detached connection closes on every return/cancellation; settings never leak to the pool.
        sqlx::query("SET statement_timeout='30s'")
            .execute(&mut conn)
            .await?;
        sqlx::query("SET lock_timeout='3s'")
            .execute(&mut conn)
            .await?;
        sqlx::query("SET TIME ZONE 'UTC'")
            .execute(&mut conn)
            .await?;
        let mut snapshot = conn.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *snapshot)
            .await?;
        let plan = plan::build(&mut snapshot, options).await?;
        snapshot.rollback().await?;
        line(
            writer,
            &json!({"record":"profile_batches_start", "community":options.community,
            "target_pubkeys":plan.scope.targets, "read_only":!delete,
            "blockers":plan.blockers,
            "retained":["deletion records", "replay protection", "repositories"],
            "external_work":["media objects", "client caches", "online admission and writer fencing"],
            "account_erasure_complete":false}),
        )?;
        let signer = if delete && plan.ops.iter().any(|o| o.snapshot.is_some()) {
            Some(
                nostr::Keys::parse(relay_secret.ok_or_else(|| {
                    invalid("archive snapshot replacement requires BUZZ_RELAY_PRIVATE_KEY")
                })?)
                .map_err(|_| invalid("invalid relay signing key"))?,
            )
        } else {
            None
        };
        if delete && !plan.blockers.is_empty() {
            return Err(invalid(
                "profile prerequisites unresolved; see printed blockers; no changes made",
            ));
        }
        // Sign before the first mutation: a missing/wrong key cannot strand a partial deletion.
        let replacements = events::sign_snapshots(&plan, signer.as_ref())?;
        let mut groups = Vec::new();
        let mut start = 0;
        let mut count = 0;
        for (index, op) in plan.ops.iter().enumerate() {
            if op.cost() > usize::from(options.batch_size) {
                return Err(invalid(
                    "archive replacement needs batch-size of at least 2",
                ));
            }
            if count + op.cost() > usize::from(options.batch_size) {
                groups.push(&plan.ops[start..index]);
                start = index;
                count = 0;
            }
            count += op.cost();
        }
        if start < plan.ops.len() {
            groups.push(&plan.ops[start..]);
        }
        let items: usize = plan.ops.iter().map(rows::Operation::cost).sum();
        if items > options.max_items as usize {
            return Err(invalid(
                "profile mutations exceed max-items; no changes made",
            ));
        }
        let mut batches = 0;
        let mut remaining_targets = plan.scope.targets.clone();
        for (batch_index, batch) in groups.iter().enumerate() {
            line(
                writer,
                &json!({"record":"proposed_batch", "number":batch_index+1,
                "items":batch.iter().map(rows::Operation::description).collect::<Vec<_>>()}),
            )?;
            if delete {
                let mut raw = self.pool.begin().await?;
                for setting in [
                    "SET LOCAL statement_timeout='30s'",
                    "SET LOCAL lock_timeout='3s'",
                    "SET LOCAL TIME ZONE 'UTC'",
                ] {
                    sqlx::query(setting).execute(&mut *raw).await?;
                }
                lock(&mut raw, &plan.scope).await?;
                let mut tx = crate::AdmittedTx::admit(
                    raw,
                    &crate::deletion::DeletionStore::new(self.pool.clone()),
                    buzz_core::CommunityId::from_uuid(plan.scope.community),
                )
                .await?;
                ownership::recheck(tx.conn(), &plan.scope, &remaining_targets).await?;
                for op in *batch {
                    rows::apply(&mut tx, &plan.scope, op, &replacements).await?;
                }
                tx.commit().await?;
                for op in *batch {
                    if op.table.name == "users" {
                        let key = op.key["pubkey"]
                            .as_str()
                            .ok_or_else(|| invalid("invalid identity key"))?;
                        let key = key
                            .strip_prefix("\\x")
                            .ok_or_else(|| invalid("invalid identity key"))?;
                        if key != plan.scope.human {
                            remaining_targets.retain(|p| p != key);
                        }
                    }
                }
                line(
                    writer,
                    &json!({"record":"batch_committed","number":batch_index+1,"items":batch.iter().map(rows::Operation::cost).sum::<usize>()}),
                )?;
            }
            batches += 1;
        }
        if !plan.blockers.is_empty() && !report_prerequisites {
            return Err(invalid(
                "preview has unresolved prerequisites; no completion marker",
            ));
        }
        if delete {
            // Original proven keys are inspection-only: removed identities must not shrink
            // final verification or become imported authority for a later invocation.
            let mut tx = conn.begin().await?;
            lock(&mut tx, &plan.scope).await?;
            ownership::recheck(&mut tx, &plan.scope, &remaining_targets).await?;
            let remaining = plan::inspect(&mut tx, plan.scope.clone(), options.max_items).await?;
            if !remaining.ops.is_empty() || !remaining.blockers.is_empty() {
                return Err(invalid(
                    "profile rows or dependencies remain; cleanup did not complete",
                ));
            }
            tx.commit().await?;
        }
        let summary = ProfileBatchSummary {
            profile: report_prerequisites.then(|| ProfileCounts::from_plan(&plan)),
            record: if delete {
                "delete_complete"
            } else if plan.blockers.is_empty() {
                "preview_complete"
            } else {
                "preview_with_prerequisites"
            },
            items,
            batches,
            target_pubkeys: plan.scope.targets,
            account_erasure_complete: false,
            prerequisites: plan.blockers,
        };
        line(writer, &summary)?;
        conn.close().await?;
        Ok(summary)
    }
}

async fn lock(conn: &mut PgConnection, scope: &Scope) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind(crate::deletion::SCHEMA_DESTRUCTION_LOCK_KEY)
        .execute(&mut *conn)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(crate::deletion::community_deletion_lock_key(
            buzz_core::CommunityId::from_uuid(scope.community),
        ))
        .execute(&mut *conn)
        .await?;
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM communities WHERE id=$1 AND deletion_state='active' AND deleted_at IS NULL)")
        .bind(scope.community).fetch_one(conn).await?;
    if !active {
        return Err(invalid(
            "community is absent or undergoing whole-community deletion",
        ));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> DbError {
    DbError::DeletionSafety(message.into())
}
fn line(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid("output exceeds 2 MiB; reduce batch-size"));
    }
    writer
        .write_all(&bytes)
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
        .map_err(|e| invalid(format!("profile batch output failed: {e}")))
}
