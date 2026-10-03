//! Behavior tests for the observer-retention prune worker.
//!
//! `#[path]`-included from `prune.rs`. `super::*` brings the prune API into
//! scope; `super::super::{store, retention}` reach the neighbouring store
//! mutators and the retention accessors. These drive the synchronous prune core
//! (`prune_observer_frames`, `run_prune_if_due`) directly against a temp-file DB
//! — the async single-flight/trigger wiring needs a Tauri `AppState` and is
//! exercised by the live lane, not here.

use super::super::{retention, store};
use super::*;
use tempfile::NamedTempFile;

const ID: &str = "idpk";
const RELAY: &str = "wss://r";
const OWNER: &str = "owner_p";

/// Open a fresh archive DB (full schema + every migration incl. M4).
fn fresh(db: &NamedTempFile) -> Connection {
    store::open_archive_db(db.path()).expect("open_archive_db must succeed")
}

/// Insert an observer frame (kind 24200) with a matching owner_p scope row at a
/// given `archived_at`, plus its `observer_channel_index` status row — the full
/// shape the ingest path writes for one frame.
fn seed_observer(conn: &Connection, id: &str, archived_at: i64) {
    store::upsert_archived_event(conn, ID, RELAY, id, 24200, "agent", 1000, "{}", archived_at)
        .unwrap();
    store::upsert_event_scope(conn, ID, RELAY, id, "owner_p", OWNER, archived_at).unwrap();
    store::upsert_observer_channel_index(conn, ID, RELAY, id, Some("ch"), 1000).unwrap();
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn event_exists(conn: &Connection, id: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM archived_events WHERE id = ?1",
        params![id],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

// ── Cutoff correctness ─────────────────────────────────────────────────────────

#[test]
fn prune_deletes_only_frames_older_than_cutoff() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer(&conn, "old", 100);
    seed_observer(&conn, "recent", 1_000);

    // Cutoff 500: "old" (100 < 500) goes, "recent" (1000) stays.
    let deleted = prune_observer_frames(&conn, ID, RELAY, 500).unwrap();
    assert_eq!(deleted, 1, "only the pre-cutoff frame's event is deleted");
    assert!(!event_exists(&conn, "old"));
    assert!(event_exists(&conn, "recent"));
    // Derived index rows cascaded for the deleted event only.
    assert_eq!(count(&conn, "observer_channel_index"), 1);
}

#[test]
fn prune_uses_archived_at_not_event_created_at() {
    // A late-arriving OLD event (small created_at, but archived_at is recent)
    // must be KEPT — age is "kept locally for N days", the scope-row archived_at.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    // created_at is fixed at 1000 in seed_observer; vary archived_at.
    seed_observer(&conn, "late-old", 9_000); // archived recently
    seed_observer(&conn, "genuinely-old", 100); // archived long ago

    let deleted = prune_observer_frames(&conn, ID, RELAY, 5_000).unwrap();
    assert_eq!(deleted, 1);
    assert!(
        event_exists(&conn, "late-old"),
        "a late-arriving old event archived recently must be kept (archived_at basis)"
    );
    assert!(!event_exists(&conn, "genuinely-old"));
}

#[test]
fn prune_keeps_future_dated_frames() {
    // A future-dated archived_at is never before any realistic cutoff, so it is
    // never pruned — the archived_at basis is immune to a bogus-future stamp.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer(&conn, "future", i64::MAX / 2);

    let deleted = prune_observer_frames(&conn, ID, RELAY, 10_000).unwrap();
    assert_eq!(deleted, 0);
    assert!(event_exists(&conn, "future"));
}

#[test]
fn prune_never_touches_non_observer_kinds() {
    // A kind-44200 metric row with an ancient archived_at must survive: the
    // candidate scan filters `e.kind = 24200`, so metrics are kept forever.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    store::upsert_archived_event(&conn, ID, RELAY, "metric", 44200, "agent", 1, "{}", 1).unwrap();
    store::upsert_event_scope(&conn, ID, RELAY, "metric", "owner_p", OWNER, 1).unwrap();
    seed_observer(&conn, "obs", 1);

    let deleted = prune_observer_frames(&conn, ID, RELAY, 5_000).unwrap();
    assert_eq!(deleted, 1, "only the observer frame is deleted");
    assert!(event_exists(&conn, "metric"), "kind 44200 is kept forever");
    assert!(!event_exists(&conn, "obs"));
}

// ── Multi-scope events ─────────────────────────────────────────────────────────

#[test]
fn prune_keeps_event_still_held_by_another_scope() {
    // An observer frame re-scoped later under a second scope: the OLD owner_p
    // scope row (archived long ago) is a prune candidate, but the RECENT
    // channel_h scope row is not. Deleting only the old scope row must NOT
    // delete the event — it still has a within-window scope proof.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    store::upsert_archived_event(&conn, ID, RELAY, "multi", 24200, "agent", 1000, "{}", 100)
        .unwrap();
    store::upsert_event_scope(&conn, ID, RELAY, "multi", "owner_p", OWNER, 100).unwrap();
    store::upsert_event_scope(&conn, ID, RELAY, "multi", "channel_h", "chan", 1000).unwrap();

    let deleted = prune_observer_frames(&conn, ID, RELAY, 500).unwrap();
    assert_eq!(
        deleted, 0,
        "event retains a within-window channel_h scope, so no event delete"
    );
    assert!(event_exists(&conn, "multi"));
    // The pre-cutoff owner_p scope row was still removed; the recent one survives.
    assert_eq!(count(&conn, "archived_event_scopes"), 1);
}

// ── Orphan cascade ───────────────────────────────────────────────────────────

#[test]
fn prune_cascades_both_derived_indexes() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    // Observer frame with an observer_channel_index row.
    seed_observer(&conn, "obs", 100);
    // A kind-24200 frame that ALSO has a stale agent_metric_index row planted on
    // its id (defensive: the cascade must clear both indexes for a pruned id).
    let metric_row = super::super::metric_store::AgentMetricIndexRow::from_payload(
        "{}", "obs", "agent", 1000, 100,
    );
    super::super::metric_store::insert_metric_index_row(&conn, ID, RELAY, &metric_row).unwrap();
    assert_eq!(count(&conn, "agent_metric_index"), 1);

    prune_observer_frames(&conn, ID, RELAY, 500).unwrap();
    assert_eq!(
        count(&conn, "observer_channel_index"),
        0,
        "observer_channel_index must be cascaded (the whole-scan GC does not cover it)"
    );
    assert_eq!(
        count(&conn, "agent_metric_index"),
        0,
        "agent_metric_index must be cascaded in the same transaction"
    );
}

// ── Batching ───────────────────────────────────────────────────────────────────

#[test]
fn prune_clears_more_than_one_batch() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    let n = super::PRUNE_BATCH_SIZE + 250; // spills into a second batch
    for i in 0..n {
        seed_observer(&conn, &format!("e{i}"), 100);
    }
    let deleted = prune_observer_frames(&conn, ID, RELAY, 500).unwrap();
    assert_eq!(
        deleted as i64, n,
        "every old frame across batches is deleted"
    );
    assert_eq!(count(&conn, "archived_events"), 0);
    assert_eq!(count(&conn, "archived_event_scopes"), 0);
    assert_eq!(count(&conn, "observer_channel_index"), 0);
}

// ── 24h gate ─────────────────────────────────────────────────────────────────

#[test]
fn run_prune_if_due_skips_within_24h_window() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer(&conn, "old", 100);
    retention::set_observer_retention_days(&conn, 1).unwrap();

    let now = 10_000_000;
    // Last success 1 hour ago → gated, no prune.
    retention::set_prune_last_success_at(&conn, now - 3_600).unwrap();
    run_prune_if_due(&conn, now).unwrap();
    assert!(
        event_exists(&conn, "old"),
        "prune within 24h must be skipped"
    );
    assert_eq!(
        retention::get_prune_last_success_at(&conn).unwrap(),
        Some(now - 3_600),
        "the gated pass must not advance the timestamp"
    );
}

#[test]
fn run_prune_if_due_runs_after_24h_and_stamps_timestamp() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer(&conn, "old", 100);
    retention::set_observer_retention_days(&conn, 1).unwrap();

    let now = 10_000_000;
    // Last success 25 hours ago → eligible.
    retention::set_prune_last_success_at(&conn, now - 25 * 3_600).unwrap();
    run_prune_if_due(&conn, now).unwrap();
    assert!(
        !event_exists(&conn, "old"),
        "eligible prune deletes stale frames"
    );
    assert_eq!(
        retention::get_prune_last_success_at(&conn).unwrap(),
        Some(now),
        "a clean pass stamps the success timestamp"
    );
}

#[test]
fn run_prune_if_due_runs_on_never_pruned_db() {
    // No prune_last_success_at row → treated as "never pruned" → eligible.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer(&conn, "old", 100);
    retention::set_observer_retention_days(&conn, 1).unwrap();
    assert_eq!(retention::get_prune_last_success_at(&conn).unwrap(), None);

    run_prune_if_due(&conn, 10_000_000).unwrap();
    assert!(!event_exists(&conn, "old"));
    assert!(retention::get_prune_last_success_at(&conn)
        .unwrap()
        .is_some());
}

#[test]
fn run_prune_if_due_stamps_even_when_nothing_deleted() {
    // A clean pass that deletes nothing still advances the gate, so a burst of
    // post-commit triggers is bounded to one scan per day.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer(&conn, "recent", 9_999_999);
    retention::set_observer_retention_days(&conn, 1).unwrap();

    let now = 10_000_000;
    run_prune_if_due(&conn, now).unwrap();
    assert!(
        event_exists(&conn, "recent"),
        "nothing is old enough to prune"
    );
    assert_eq!(
        retention::get_prune_last_success_at(&conn).unwrap(),
        Some(now),
        "an empty-but-clean pass still stamps the gate"
    );
}

// ── Global gate covers every partition (not just the active one) ────────────────

/// Seed an observer frame in an explicit `(identity, relay)` partition. The
/// partition-scoped analogue of `seed_observer`, used to prove one due pass
/// prunes every stored partition — not just the active identity/relay.
fn seed_observer_in(conn: &Connection, id_pk: &str, relay: &str, id: &str, archived_at: i64) {
    store::upsert_archived_event(
        conn,
        id_pk,
        relay,
        id,
        24200,
        "agent",
        1000,
        "{}",
        archived_at,
    )
    .unwrap();
    store::upsert_event_scope(conn, id_pk, relay, id, "owner_p", OWNER, archived_at).unwrap();
    store::upsert_observer_channel_index(conn, id_pk, relay, id, Some("ch"), 1000).unwrap();
}

#[test]
fn run_prune_if_due_prunes_every_partition_before_stamping() {
    // The gate is database-global, so one due pass must prune ALL stored
    // partitions — including an inactive/historical one that no active sync is
    // ever scoped to — before stamping success. Two partitions, both with an
    // expired frame; neither is "the active pair" the pass resolves (it no
    // longer resolves one).
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer_in(&conn, "idA", "wss://a", "old_a", 100);
    seed_observer_in(&conn, "idB", "wss://b", "old_b", 100);
    retention::set_observer_retention_days(&conn, 1).unwrap();

    run_prune_if_due(&conn, 10_000_000).unwrap();
    assert!(
        !event_exists(&conn, "old_a"),
        "partition A's expired frame must be pruned"
    );
    assert!(
        !event_exists(&conn, "old_b"),
        "partition B's expired frame must be pruned in the same pass"
    );
    assert_eq!(count(&conn, "archived_events"), 0);
    assert_eq!(count(&conn, "observer_channel_index"), 0);
    assert!(retention::get_prune_last_success_at(&conn)
        .unwrap()
        .is_some());
}

#[test]
fn run_prune_if_due_leaves_gate_unstamped_then_retry_completes_remainder() {
    // A mid-worklist failure must commit the prefix, leave the gate unstamped,
    // and let the idempotent retry finish the remainder. The worklist is
    // ordered by (identity_pubkey, relay_url), so "idA" prunes before "idB".
    // A BEFORE DELETE trigger that aborts only on partition idB's events fails
    // the LATER partition — after idA has already committed its own batch.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer_in(&conn, "idA", "wss://a", "old_a", 100);
    seed_observer_in(&conn, "idB", "wss://b", "old_b", 100);
    retention::set_observer_retention_days(&conn, 1).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_idB BEFORE DELETE ON archived_events
         WHEN OLD.identity_pubkey = 'idB'
         BEGIN SELECT RAISE(ABORT, 'forced idB failure'); END",
    )
    .unwrap();

    let now = 10_000_000;
    let result = run_prune_if_due(&conn, now);
    assert!(
        result.is_err(),
        "the later partition's forced failure must surface"
    );
    assert!(
        !event_exists(&conn, "old_a"),
        "partition idA was pruned and committed before idB failed"
    );
    assert!(
        event_exists(&conn, "old_b"),
        "partition idB's batch rolled back on the forced failure"
    );
    assert_eq!(
        retention::get_prune_last_success_at(&conn).unwrap(),
        None,
        "a partial worklist must not stamp success — the pass retries"
    );

    // Clear the fault and rerun: idA is already pruned (a no-op), idB now
    // prunes, and the complete worklist stamps success.
    conn.execute_batch("DROP TRIGGER fail_idB").unwrap();
    run_prune_if_due(&conn, now).unwrap();
    assert!(
        !event_exists(&conn, "old_b"),
        "the retry prunes the remainder"
    );
    assert_eq!(count(&conn, "archived_events"), 0);
    assert_eq!(
        retention::get_prune_last_success_at(&conn).unwrap(),
        Some(now),
        "the completed worklist stamps success"
    );
}

// ── Stale-backfill orphan race (conditional observer-index insert) ──────────────

#[test]
fn observer_index_insert_is_skipped_when_event_is_gone() {
    // The stale-backfill race: the TS driver snapshots an unindexed id, the
    // frame is pruned, then the driver calls back to index it. The conditional
    // INSERT ... SELECT must NOT resurrect an index row for the absent event.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    // No archived_events row for "gone" — simulate it already pruned away.
    store::upsert_observer_channel_index(&conn, ID, RELAY, "gone", Some("ch"), 1000).unwrap();
    assert_eq!(
        count(&conn, "observer_channel_index"),
        0,
        "no index row may be created for an event that no longer exists"
    );
}

#[test]
fn observer_index_insert_succeeds_when_event_present() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    store::upsert_archived_event(&conn, ID, RELAY, "here", 24200, "agent", 1000, "{}", 100)
        .unwrap();
    store::upsert_observer_channel_index(&conn, ID, RELAY, "here", Some("ch"), 1000).unwrap();
    assert_eq!(count(&conn, "observer_channel_index"), 1);
}

// ── First-archive ON CONFLICT DO NOTHING pin ────────────────────────────────────

#[test]
fn observer_index_insert_is_idempotent_on_conflict() {
    // Re-indexing an already-indexed frame is a no-op (first write wins), even
    // now that the insert is gated on the canonical event.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    store::upsert_archived_event(&conn, ID, RELAY, "e", 24200, "agent", 1000, "{}", 100).unwrap();
    store::upsert_observer_channel_index(&conn, ID, RELAY, "e", Some("first"), 1000).unwrap();
    store::upsert_observer_channel_index(&conn, ID, RELAY, "e", Some("second"), 2000).unwrap();
    let ch: Option<String> = conn
        .query_row(
            "SELECT channel_id FROM observer_channel_index WHERE id = 'e'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ch.as_deref(),
        Some("first"),
        "first write wins on PK conflict"
    );
}

// ── Batch atomicity (forced error between delete stages) ────────────────────────

#[test]
fn prune_batch_rolls_back_fully_when_a_cascade_fails() {
    // The delete stages run in one transaction: scope delete → event GC →
    // observer-index cascade → metric-index cascade. If a later stage errors,
    // the whole batch must roll back so no event is left with orphaned index
    // rows (or, conversely, an index row orphaned by a half-applied delete).
    // Force the failure by removing agent_metric_index, so the metric cascade —
    // the last stage — errors after the earlier deletes have run in-tx.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    seed_observer(&conn, "old", 100);
    conn.execute_batch("DROP TABLE agent_metric_index").unwrap();

    let result = prune_observer_frames(&conn, ID, RELAY, 500);
    assert!(result.is_err(), "the failed metric cascade must surface");
    // Every earlier delete in the batch is rolled back: the frame, its scope
    // row, and its observer index row all survive intact.
    assert!(
        event_exists(&conn, "old"),
        "a mid-batch cascade failure must roll back the event delete"
    );
    assert_eq!(count(&conn, "archived_event_scopes"), 1);
    assert_eq!(count(&conn, "observer_channel_index"), 1);
}

// ── EXPLAIN-shape against the real worker SQL ───────────────────────────────────

#[test]
fn prune_candidate_scan_seeks_archived_at_through_the_index() {
    // Pinned against the SHIPPED PRUNE_CANDIDATE_SQL constant (the same one the
    // worker prepares), so the covering-index access path and the production
    // query cannot drift.
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);

    let plan: Vec<String> = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {PRUNE_CANDIDATE_SQL}"))
        .unwrap()
        .query_map(params![ID, RELAY, 0_i64, PRUNE_BATCH_SIZE], |r| {
            r.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let scope_step = plan
        .iter()
        .find(|d| d.contains(retention::SCOPE_AGE_INDEX_NAME))
        .unwrap_or_else(|| {
            panic!(
                "prune scan must use {}; plan was {plan:?}",
                retention::SCOPE_AGE_INDEX_NAME
            )
        });
    assert!(
        scope_step.contains("archived_at<?"),
        "scope step must range-seek archived_at, got: {scope_step}"
    );
    assert!(
        !plan.iter().any(|d| d.contains("USE TEMP B-TREE")),
        "age-first index must remove the ORDER-BY temp b-tree; plan was {plan:?}"
    );
}
