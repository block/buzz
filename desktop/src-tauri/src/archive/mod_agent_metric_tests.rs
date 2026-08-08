//! Kind-44200 (NIP-AM agent turn metric) archive and `get_agent_usage_series`
//! integration tests for `archive/mod.rs`.
//!
//! Kept in a sibling file so `mod_tests.rs` stays under the 1000-line gate;
//! `#[path]`-included from there so the shared fixtures (`in_memory`,
//! `add_sub`, `candidate`, `make_observer_frame`, `run_batch_sync_with_keys`)
//! stay private to `mod_tests`.

use super::*;

// ── Kind-44200 agent-turn-metric archive tests ───────────────────────────

fn make_turn_metric_event(owner_keys: &Keys, agent_keys: &Keys) -> Event {
    use buzz_core_pkg::agent_turn_metric::{
        encrypt_agent_turn_metric, AgentTurnMetricPayload, TokenCounts,
    };
    let owner_pk = owner_keys.public_key().to_hex();
    let payload = AgentTurnMetricPayload {
        harness: "test-harness".to_string(),
        model: Some("test-model".to_string()),
        channel_id: None,
        session_id: Some("sess-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        turn_seq: Some(1),
        timestamp: "2026-07-01T00:00:00Z".to_string(),
        turn: Some(TokenCounts {
            input_tokens: Some(100),
            output_tokens: Some(50),
            total_tokens: Some(150),
            cost_usd: Some(0.001),
            cache_read_tokens: None,
            cache_write_tokens: None,
        }),
        cumulative: None,
        delta_reliable: true,
        stop_reason: None,
        pricing_identity: None,
    };
    let ciphertext =
        encrypt_agent_turn_metric(agent_keys, &owner_keys.public_key(), &payload).unwrap();
    let tags = vec![
        Tag::parse(["p", &owner_pk]).unwrap(),
        Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
    ];
    EventBuilder::new(Kind::Custom(44200), &ciphertext)
        .tags(tags)
        .sign_with_keys(agent_keys)
        .unwrap()
}

/// A kind-44200 event with `owner_p` scope must route to the persistent
/// (relay-query) path, NOT the ephemeral path.
#[test]
fn test_owner_p_44200_routes_to_persistent_path() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    // Subscription for kind 44200 under owner_p.
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[44200]");

    let ev = make_turn_metric_event(&owner_keys, &agent_keys);
    let cand = candidate(&ev, ScopeType::OwnerP, &owner_pk);

    let plan = plan_archive(vec![cand], &owner_pk, relay_url, &conn).unwrap();

    // Must be in persistent buckets, NOT ephemeral list.
    assert_eq!(plan.buckets.len(), 1, "kind-44200 must land in a bucket");
    assert_eq!(
        plan.ephemeral.len(),
        0,
        "kind-44200 must NOT be on the ephemeral path"
    );
    assert_eq!(
        plan.buckets[0].scope_type_str, "owner_p",
        "bucket scope_type must be owner_p"
    );
}

/// A kind-24200 event with `owner_p` scope must still route to ephemeral.
#[test]
fn test_owner_p_24200_still_routes_to_ephemeral() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");

    let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
    let cand = candidate(&ev, ScopeType::OwnerP, &owner_pk);

    let plan = plan_archive(vec![cand], &owner_pk, relay_url, &conn).unwrap();

    assert_eq!(
        plan.buckets.len(),
        0,
        "kind-24200 must NOT land in a bucket"
    );
    assert_eq!(
        plan.ephemeral.len(),
        1,
        "kind-24200 must be on the ephemeral path"
    );
}

/// Decrypt success: plaintext payload JSON is stored, not raw ciphertext.
#[test]
fn test_turn_metric_decrypt_success_stores_plaintext() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[44200]");

    let ev = make_turn_metric_event(&owner_keys, &agent_keys);
    let cand = candidate(&ev, ScopeType::OwnerP, &owner_pk);
    let result = run_batch_sync_with_keys(
        vec![cand],
        &owner_pk,
        relay_url,
        &conn,
        vec![ev.clone()],
        &owner_keys,
    );

    assert_eq!(result.persisted, 1, "event must be persisted");
    assert_eq!(result.dropped, 0, "no drops on successful decrypt");
    assert_eq!(
        result.persisted_agent_metrics, 1,
        "one newly-indexed agent_metric_index row on first ingest"
    );

    // The stored raw_json must be plaintext JSON, not NIP-44 ciphertext.
    let raw_json: String = conn
        .query_row("SELECT raw_json FROM archived_events", [], |r| r.get(0))
        .unwrap();
    // Plaintext JSON should be a valid object with "harness" key.
    let parsed: serde_json::Value =
        serde_json::from_str(&raw_json).expect("stored raw_json must be valid JSON");
    assert_eq!(
        parsed["harness"], "test-harness",
        "stored plaintext must decode to AgentTurnMetricPayload"
    );
    // Sanity: must NOT be the original NIP-44 ciphertext (which is not JSON).
    assert_ne!(
        raw_json, ev.content,
        "stored content must differ from original ciphertext"
    );
}

/// Decrypt fail: event is dropped, nothing written to the store (fail-closed).
#[test]
fn test_turn_metric_decrypt_fail_drops_fail_closed() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let wrong_keys = Keys::generate(); // wrong owner key — decrypt will fail
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    // Register subscription under owner_pk so the event passes plan-phase,
    // but use `wrong_keys` in commit so decrypt fails.
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[44200]");

    let ev = make_turn_metric_event(&owner_keys, &agent_keys);
    let cand = candidate(&ev, ScopeType::OwnerP, &owner_pk);
    let result = run_batch_sync_with_keys(
        vec![cand],
        &owner_pk,
        relay_url,
        &conn,
        vec![ev.clone()],
        &wrong_keys, // wrong key → decrypt fails
    );

    assert_eq!(
        result.persisted, 0,
        "decrypt failure must not persist the event"
    );
    assert_eq!(result.dropped, 1, "decrypt failure must count as dropped");

    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        event_count, 0,
        "no rows must be written to archived_events on decrypt failure"
    );
}

/// Re-ingesting a batch containing an already-archived kind-44200 event must
/// no-op the metric index insert: `persisted` still counts the (idempotent)
/// event/scope upsert, but `persisted_agent_metrics` must be 0 for the
/// duplicate — the row was already indexed by the first ingest (A5: this is
/// exactly the signal the frontend uses to skip a redundant query
/// invalidation).
#[test]
fn test_reingest_of_same_metric_event_does_not_double_count_persisted_agent_metrics() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[44200]");

    let ev = make_turn_metric_event(&owner_keys, &agent_keys);
    let cand1 = candidate(&ev, ScopeType::OwnerP, &owner_pk);

    let first = run_batch_sync_with_keys(
        vec![cand1],
        &owner_pk,
        relay_url,
        &conn,
        vec![ev.clone()],
        &owner_keys,
    );
    assert_eq!(first.persisted, 1);
    assert_eq!(first.persisted_agent_metrics, 1);

    // Same event re-ingested in a second batch (e.g. relay redelivery).
    let cand2 = candidate(&ev, ScopeType::OwnerP, &owner_pk);
    let second = run_batch_sync_with_keys(
        vec![cand2],
        &owner_pk,
        relay_url,
        &conn,
        vec![ev.clone()],
        &owner_keys,
    );
    assert_eq!(
        second.persisted, 1,
        "re-ingest of a duplicate is still an accepted (idempotent) write"
    );
    assert_eq!(
        second.persisted_agent_metrics, 0,
        "re-ingest must NOT double-count the metric index row"
    );

    let index_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_metric_index", [], |r| r.get(0))
        .unwrap();
    assert_eq!(index_count, 1, "exactly one index row must exist total");
}

// ── get_agent_usage_series integration ──────────────────────────────────────
//
// Exercises `agent_usage_series` (the sync core `get_agent_usage_series`
// delegates to) end to end: ingest a real encrypted turn-metric event
// through the full archive pipeline, then read it back through the command
// core, proving backfill/indexing, collection-enabled detection, and the
// pure accounting ladder are wired together correctly — not just each in
// isolation.

/// A freshly ingested single turn-metric event surfaces in the series with
/// its direct (delta-reliable, no-baseline) token counts, and
/// `collectionEnabled` reflects the active owner_p/44200 subscription.
#[test]
fn test_agent_usage_series_surfaces_freshly_ingested_event() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let agent_pk = agent_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[44200]");

    let ev = make_turn_metric_event(&owner_keys, &agent_keys);
    let cand = candidate(&ev, ScopeType::OwnerP, &owner_pk);
    let batch = run_batch_sync_with_keys(
        vec![cand],
        &owner_pk,
        relay_url,
        &conn,
        vec![ev.clone()],
        &owner_keys,
    );
    assert_eq!(batch.persisted_agent_metrics, 1, "event must be indexed");

    // `make_turn_metric_event`'s payload timestamp is 2026-07-01T00:00:00Z.
    const EVENT_DAY_START: i64 = 1_782_864_000;
    let boundaries: Vec<i64> = (0..=7).map(|i| EVENT_DAY_START + i * 86_400).collect();
    let request = agent_usage::AgentUsageSeriesRequest {
        bucket_boundaries: boundaries,
        agent_pubkey: None,
    };

    let series = agent_usage_series(&conn, &owner_pk, relay_url, &request).unwrap();

    assert!(
        series.collection_enabled,
        "owner_p subscription includes kind 44200"
    );
    assert_eq!(series.coverage.report_count, 1);
    assert_eq!(series.coverage.invalid_report_count, 0);
    assert_eq!(series.agents.len(), 1, "exactly one agent reported usage");
    let agent = &series.agents[0];
    assert_eq!(agent.agent_pubkey, agent_pk);
    // No baseline row exists, so the ladder falls back to the direct
    // (delta-reliable) turn values from the payload: 100/50/150.
    assert_eq!(agent.usage.input_tokens.value.as_deref(), Some("100"));
    assert_eq!(agent.usage.output_tokens.value.as_deref(), Some("50"));
    assert_eq!(agent.usage.total_tokens.value.as_deref(), Some("150"));
    assert!(!agent.usage.input_tokens.incomplete);
    assert_eq!(
        series.has_archived_evidence, None,
        "no agentPubkey filter was supplied"
    );
}

/// Filtering by `agentPubkey` scopes both the returned series and
/// `hasArchivedEvidence` (A13) to that one author; an unrelated agent's
/// events must not leak into either.
#[test]
fn test_agent_usage_series_filters_by_agent_pubkey_and_sets_has_archived_evidence() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let target_agent = Keys::generate();
    let other_agent = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let target_pk = target_agent.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[44200]");

    let target_ev = make_turn_metric_event(&owner_keys, &target_agent);
    let other_ev = make_turn_metric_event(&owner_keys, &other_agent);
    let cands = vec![
        candidate(&target_ev, ScopeType::OwnerP, &owner_pk),
        candidate(&other_ev, ScopeType::OwnerP, &owner_pk),
    ];
    let batch = run_batch_sync_with_keys(
        cands,
        &owner_pk,
        relay_url,
        &conn,
        vec![target_ev.clone(), other_ev.clone()],
        &owner_keys,
    );
    assert_eq!(batch.persisted_agent_metrics, 2);

    const EVENT_DAY_START: i64 = 1_782_864_000;
    let boundaries: Vec<i64> = (0..=7).map(|i| EVENT_DAY_START + i * 86_400).collect();
    let request = agent_usage::AgentUsageSeriesRequest {
        bucket_boundaries: boundaries,
        agent_pubkey: Some(target_pk.clone()),
    };

    let series = agent_usage_series(&conn, &owner_pk, relay_url, &request).unwrap();

    assert_eq!(
        series.agents.len(),
        1,
        "only the filtered agent's usage must be returned"
    );
    assert_eq!(series.agents[0].agent_pubkey, target_pk);
    assert_eq!(
        series.has_archived_evidence,
        Some(true),
        "A13: evidence exists for the filtered author"
    );
}

/// An unindexed pre-existing kind-44200 row (simulating an event archived
/// by a prior build before `agent_metric_index` existed) is picked up by
/// the command's backfill step before the window is read.
#[test]
fn test_agent_usage_series_backfills_unindexed_row_before_reading() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";

    // Insert directly into `archived_events`, bypassing `commit_archive`, so
    // no `agent_metric_index` row is created — the exact state a fresh
    // backfill must repair.
    let ev = make_turn_metric_event(&owner_keys, &agent_keys);
    let plaintext = r#"{"harness":"test-harness","model":"test-model","sessionId":"sess-1","turnId":"turn-1","turnSeq":1,"timestamp":"2026-07-01T00:00:00Z","turn":{"inputTokens":100,"outputTokens":50,"totalTokens":150,"costUsd":0.001},"deltaReliable":true}"#;
    store::upsert_archived_event(
        &conn,
        &owner_pk,
        relay_url,
        &ev.id.to_hex(),
        44200,
        &agent_keys.public_key().to_hex(),
        ev.created_at.as_secs() as i64,
        plaintext,
        0,
    )
    .unwrap();

    let index_count_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_metric_index", [], |r| r.get(0))
        .unwrap();
    assert_eq!(index_count_before, 0, "no index row before backfill");

    const EVENT_DAY_START: i64 = 1_782_864_000;
    let boundaries: Vec<i64> = (0..=7).map(|i| EVENT_DAY_START + i * 86_400).collect();
    let request = agent_usage::AgentUsageSeriesRequest {
        bucket_boundaries: boundaries,
        agent_pubkey: None,
    };

    let series = agent_usage_series(&conn, &owner_pk, relay_url, &request).unwrap();

    assert_eq!(
        series.coverage.report_count, 1,
        "backfill must index the pre-existing row before the window read"
    );
}
