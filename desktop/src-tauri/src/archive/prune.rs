//! Observer-frame retention prune worker (v4).
//!
//! Deletes archived kind-24200 observer-frame rows older than the configured
//! window (`observer_retention_days`, from `archive_meta`). Every other
//! archived kind — NIP-AM metrics (44200) and any custom subscription kind — is
//! kept indefinitely and never touched here (Will's v4 ruling, 2026-08-19).
//!
//! # Scheduling
//!
//! No cron. The worker is triggered opportunistically — once shortly after
//! application startup (after the archive init barrier warms), and again after
//! each archive commit — and gated two ways so those frequent triggers collapse
//! into at most one real prune per day:
//!
//! 1. **In-process single-flight** ([`super::ArchiveDb::try_prune_guard`]): a
//!    `compare_exchange` on an `AtomicBool` admits exactly one prune at a time
//!    in this process; a trigger that arrives while a prune is running is
//!    dropped, not queued. This is a single-process desktop app, so an
//!    in-process flag is sufficient — no lease or fenced token (the v3 design's
//!    fenced prune lease was cut in v4).
//! 2. **≥24h timestamp gate** (`archive_meta.prune_last_success_at`): a prune
//!    that finds the last success within 24h returns immediately. The timestamp
//!    is written after every clean pass (even one that deleted nothing), so a
//!    burst of post-commit triggers costs one meta read apiece, not a scan.
//!
//! The gate is database-global, so a due pass does global work: it enumerates a
//! fixed worklist of every archived `(identity_pubkey, relay_url)` partition and
//! prunes each under the same cutoff, stamping success only after the whole
//! worklist completes. It never scopes to the identity/relay that happened to be
//! active when the trigger fired, so an inactive historical partition is not
//! starved and a partition failure leaves the gate unstamped for a retry.
//!
//! A prune failure is fully detached from the archive commit that triggered it:
//! [`trigger_prune`] spawns onto the async runtime and swallows the result, so a
//! prune error can never fail a user's archive write.
//!
//! # Delete discipline
//!
//! Candidate scope-row primary keys are materialized inside each batch
//! transaction and fixed for that transaction (Thufir's binding condition #4):
//! the batch deletes exactly those rows, then runs a **candidate-scoped** orphan
//! GC over only the event ids it just unscoped — never a widened whole-table
//! anti-join (`store::gc_orphaned_events` stays a rare repair pass, not the
//! normal path) and never a post-delete re-scan to discover new orphans. The GC
//! cascades BOTH derived indexes, `observer_channel_index` (the one the
//! whole-scan GC does not cover) and `agent_metric_index`, so no index row
//! outlives the canonical event it was derived from.

use rusqlite::{params, Connection};
use tauri::{AppHandle, Manager};

use crate::app_state::AppState;

use super::retention;

/// Minimum spacing between successful prune passes. Frequent triggers below
/// this interval no-op against the `prune_last_success_at` gate.
const PRUNE_MIN_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// Seconds per day, for the `archived_at` cutoff arithmetic. `days` is bounded
/// to `retention::MAX_RETENTION_DAYS` (36 500), so `days * SECS_PER_DAY` cannot
/// overflow `i64`.
const SECS_PER_DAY: i64 = 24 * 60 * 60;

/// Scope-row primary keys deleted per batch transaction. Large enough that the
/// first prune over a multi-million-row archive completes in a bounded number
/// of transactions, small enough that each transaction's write lock is brief.
const PRUNE_BATCH_SIZE: i64 = 2_000;

/// The prune-candidate scan: the single global observer window. Filters scope
/// rows on `(identity_pubkey, relay_url, archived_at < cutoff)` and joins
/// `archived_events` for `kind = 24200`, selecting the scope-row primary key so
/// the batch `DELETE` can materialize its candidates. It constrains neither
/// `scope_type` nor `scope_value` — the reason the M4 scope-age index leads
/// with `archived_at` immediately after the identity/relay equality keys.
///
/// This is the production query. The EXPLAIN-shape test in `prune_tests.rs`
/// runs `EXPLAIN QUERY PLAN` against this exact constant so the covering-index
/// access path and the shipped query can never drift (Thufir's carry-forward
/// note — the constant previously lived only in the test file).
pub(super) const PRUNE_CANDIDATE_SQL: &str = "
SELECT s.id, s.scope_type, s.scope_value
FROM archived_event_scopes s
JOIN archived_events e
  ON e.identity_pubkey = s.identity_pubkey
 AND e.relay_url = s.relay_url
 AND e.id = s.id
WHERE s.identity_pubkey = ?1
  AND s.relay_url = ?2
  AND s.archived_at < ?3
  AND e.kind = 24200
LIMIT ?4
";

/// The fixed prune worklist: every archived `(identity_pubkey, relay_url)`
/// partition holding scope rows. Enumerated once at the start of a due pass so
/// the global gate covers the whole archive rather than just the partition that
/// happened to be active when the trigger fired. A partition with no kind-24200
/// scopes in range simply prunes nothing.
///
/// Ordered so the worklist is deterministic: a pass that fails partway leaves a
/// well-defined prefix pruned, and the idempotent retry resumes the remainder
/// in the same order (already-pruned partitions re-run as no-ops).
pub(super) const PRUNE_PARTITIONS_SQL: &str = "
SELECT DISTINCT identity_pubkey, relay_url FROM archived_event_scopes
ORDER BY identity_pubkey, relay_url
";

/// Spawn a detached prune pass keyed off the app handle. Called post-commit
/// from the archive write path (the unconditional-startup trigger calls
/// [`maybe_prune`] directly, since it already holds the [`AppState`]). The prune
/// runs on the async runtime and its result is swallowed, so a prune error never
/// propagates into the archive write that triggered it.
pub fn trigger_prune(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        maybe_prune(&state).await;
    });
}

/// Run one prune pass if the single-flight flag admits it. The pass is archive
/// global — it prunes every stored partition, not just the active
/// identity/relay — so it dispatches the gate check + worklist prune to the
/// blocking pool via the gated archive adapter without resolving the active
/// identity. Errors are logged, not returned: every caller is fire-and-forget.
pub async fn maybe_prune(state: &AppState) {
    // Single-flight: admit exactly one prune at a time. A trigger arriving
    // while a prune runs is dropped — the next post-commit trigger will pick up
    // any rows this pass could not reach.
    let Some(_guard) = state.archive_db.try_prune_guard() else {
        return;
    };

    let now = super::now_secs();

    if let Err(error) = state
        .archive_db
        .with_conn(move |conn| run_prune_if_due(conn, now))
        .await
    {
        eprintln!("buzz-desktop: archive prune failed: {error}");
    }
}

/// Enforce the ≥24h gate, then prune every partition and stamp the global
/// success timestamp.
///
/// The gate (`archive_meta.prune_last_success_at`) is database-global, so the
/// work it gates must be too: a due pass enumerates a fixed worklist of every
/// archived `(identity_pubkey, relay_url)` partition ([`PRUNE_PARTITIONS_SQL`])
/// and prunes each under the same cutoff. The timestamp is stamped only after
/// the entire worklist succeeds — including a pass that deleted nothing — so a
/// burst of triggers is bounded to a single scan per day, and a mid-worklist
/// failure leaves the gate unstamped for the idempotent pass to retry.
fn run_prune_if_due(conn: &Connection, now: i64) -> Result<(), String> {
    if let Some(last) = retention::get_prune_last_success_at(conn)? {
        if now - last < PRUNE_MIN_INTERVAL_SECS {
            return Ok(());
        }
    }

    let days = retention::get_observer_retention_days(conn)?;
    let cutoff = now - days * SECS_PER_DAY;

    let mut stmt = conn
        .prepare(PRUNE_PARTITIONS_SQL)
        .map_err(|e| format!("prune: prepare partition scan: {e}"))?;
    let partitions: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("prune: query partitions: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("prune: read partition row: {e}"))?;
    drop(stmt);

    for (identity_pk, relay_url) in &partitions {
        prune_observer_frames(conn, identity_pk, relay_url, cutoff)?;
    }

    retention::set_prune_last_success_at(conn, now)?;
    Ok(())
}

/// Delete every observer-frame (kind 24200) scope row whose `archived_at` is
/// before `cutoff`, in bounded batches, cascading the orphan GC to both derived
/// indexes for the events those deletes leave unscoped.
///
/// Each batch is one transaction: materialize the candidate scope keys, delete
/// them, then GC only those candidate events. The candidate set is fixed per
/// transaction (binding condition #4). The loop ends when a batch returns fewer
/// candidates than the batch size — i.e. the last, partial batch.
pub(super) fn prune_observer_frames(
    conn: &Connection,
    identity_pk: &str,
    relay_url: &str,
    cutoff: i64,
) -> Result<usize, String> {
    let mut total_events_deleted = 0usize;

    loop {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("prune: begin batch transaction: {e}"))?;

        // Materialize the candidate scope rows in-tx and fix them for this
        // transaction — (id, scope_type, scope_value); identity/relay are the
        // fixed ?1/?2 params.
        let mut stmt = tx
            .prepare(PRUNE_CANDIDATE_SQL)
            .map_err(|e| format!("prune: prepare candidate scan: {e}"))?;
        let candidates: Vec<(String, String, String)> = stmt
            .query_map(
                params![identity_pk, relay_url, cutoff, PRUNE_BATCH_SIZE],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|e| format!("prune: query candidates: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("prune: read candidate row: {e}"))?;
        drop(stmt);

        if candidates.is_empty() {
            // Nothing left in range; the empty read did no writes, so rolling
            // back is the honest close (commit would be equally harmless).
            drop(candidates);
            let _ = tx.rollback();
            break;
        }
        let batch_len = candidates.len();

        // 1. Delete the exact candidate scope rows by full primary key.
        {
            let mut del_scope = tx
                .prepare(
                    "DELETE FROM archived_event_scopes
                     WHERE identity_pubkey = ?1 AND relay_url = ?2
                       AND id = ?3 AND scope_type = ?4 AND scope_value = ?5",
                )
                .map_err(|e| format!("prune: prepare scope delete: {e}"))?;
            for (id, scope_type, scope_value) in &candidates {
                del_scope
                    .execute(params![identity_pk, relay_url, id, scope_type, scope_value])
                    .map_err(|e| format!("prune: delete scope row: {e}"))?;
            }
        }

        // 2. Candidate-scoped orphan GC. Only the distinct event ids just
        //    unscoped are checked — no widened anti-join, no post-delete
        //    re-scan for other orphans. An event still holding another scope
        //    row survives; one left with zero scope rows is deleted, and its
        //    derived-index rows in BOTH indexes are cascaded away in the same
        //    transaction so neither can outlive the canonical event.
        {
            let mut del_event = tx
                .prepare(
                    "DELETE FROM archived_events
                     WHERE identity_pubkey = ?1 AND relay_url = ?2 AND id = ?3
                       AND NOT EXISTS (
                           SELECT 1 FROM archived_event_scopes
                           WHERE identity_pubkey = ?1 AND relay_url = ?2 AND id = ?3
                       )",
                )
                .map_err(|e| format!("prune: prepare event GC: {e}"))?;
            let mut del_observer = tx
                .prepare(
                    "DELETE FROM observer_channel_index
                     WHERE identity_pubkey = ?1 AND relay_url = ?2 AND id = ?3",
                )
                .map_err(|e| format!("prune: prepare observer-index cascade: {e}"))?;
            let mut del_metric = tx
                .prepare(
                    "DELETE FROM agent_metric_index
                     WHERE identity_pubkey = ?1 AND relay_url = ?2 AND id = ?3",
                )
                .map_err(|e| format!("prune: prepare metric-index cascade: {e}"))?;

            let mut seen = std::collections::HashSet::new();
            for (id, _, _) in &candidates {
                if !seen.insert(id.as_str()) {
                    continue;
                }
                let orphaned = del_event
                    .execute(params![identity_pk, relay_url, id])
                    .map_err(|e| format!("prune: gc orphaned event: {e}"))?;
                if orphaned > 0 {
                    total_events_deleted += orphaned;
                    del_observer
                        .execute(params![identity_pk, relay_url, id])
                        .map_err(|e| format!("prune: cascade observer index: {e}"))?;
                    del_metric
                        .execute(params![identity_pk, relay_url, id])
                        .map_err(|e| format!("prune: cascade metric index: {e}"))?;
                }
            }
        }

        tx.commit()
            .map_err(|e| format!("prune: commit batch: {e}"))?;

        if (batch_len as i64) < PRUNE_BATCH_SIZE {
            break;
        }
    }

    Ok(total_events_deleted)
}

#[cfg(test)]
#[path = "prune_tests.rs"]
mod prune_tests;
