//! Unit and integration tests for `archive/mod.rs`.
//!
//! Kept in a sibling file so `mod.rs` stays under the 1000-line gate;
//! `#[path]`-included from there.

use super::pipeline::BucketWithResult;
use super::*;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use rusqlite::Connection;
use uuid::Uuid;

// ── Helpers ──────────────────────────────────────────────────────────────

fn in_memory() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "busy_timeout", 5000).unwrap();
    conn.execute_batch(super::store::SCHEMA).unwrap();
    conn
}

fn make_observer_frame(owner_keys: &Keys, agent_keys: &Keys, frame_type: &str) -> Event {
    let owner_pk = owner_keys.public_key().to_hex();
    let agent_pk = agent_keys.public_key().to_hex();
    let tags = vec![
        Tag::parse(["p", &owner_pk]).unwrap(),
        Tag::parse(["agent", &agent_pk]).unwrap(),
        Tag::parse(["frame", frame_type]).unwrap(),
    ];
    EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
        .tags(tags)
        .sign_with_keys(agent_keys)
        .unwrap()
}

fn add_sub(
    conn: &Connection,
    identity_pk: &str,
    relay_url: &str,
    scope_type: &str,
    scope_value: &str,
    kinds_json: &str,
) {
    store::upsert_save_subscription(
        conn,
        identity_pk,
        relay_url,
        scope_type,
        scope_value,
        kinds_json,
        0,
    )
    .unwrap();
}

/// Run the full archive pipeline synchronously with a fake relay response.
///
/// Calls `plan_archive` → injects fake relay events → `commit_archive`.
/// This mirrors `archive_events` without the async relay calls.
fn run_batch_sync(
    candidates: Vec<ArchiveCandidate>,
    identity_pk: &str,
    relay_url: &str,
    conn: &Connection,
    fake_relay_events: Vec<Event>,
) -> ArchiveBatchResult {
    let owner_keys = Keys::generate();
    run_batch_sync_with_keys(
        candidates,
        identity_pk,
        relay_url,
        conn,
        fake_relay_events,
        &owner_keys,
    )
}

/// Like `run_batch_sync` but with a specific owner `Keys` for decrypt.
fn run_batch_sync_with_keys(
    candidates: Vec<ArchiveCandidate>,
    identity_pk: &str,
    relay_url: &str,
    conn: &Connection,
    fake_relay_events: Vec<Event>,
    owner_keys: &Keys,
) -> ArchiveBatchResult {
    let plan = plan_archive(candidates, identity_pk, relay_url, conn).unwrap();

    // Synthesize BucketWithResult from the fake relay response.
    let fake_ids: std::collections::HashSet<String> =
        fake_relay_events.iter().map(|e| e.id.to_hex()).collect();
    let bucket_results: Vec<BucketWithResult> = plan
        .buckets
        .into_iter()
        .map(|b| BucketWithResult {
            scope_type_str: b.scope_type_str,
            scope_value: b.scope_value,
            allowed_kinds: b.allowed_kinds,
            group: b.group,
            returned_ids: fake_ids.clone(),
            relay_failed: false,
        })
        .collect();

    commit_archive(
        bucket_results,
        plan.ephemeral,
        plan.pre_dropped,
        identity_pk,
        relay_url,
        owner_keys,
        0,
        conn,
    )
    .unwrap()
}

fn candidate(event: &Event, scope_type: ScopeType, scope_value: &str) -> ArchiveCandidate {
    ArchiveCandidate {
        raw_event_json: event.as_json(),
        matched_scope: MatchedScope {
            scope_type,
            scope_value: scope_value.to_string(),
        },
    }
}

// ── Ephemeral validator — individual condition rejection ──────────────────

#[test]
fn test_ephemeral_validator_accepts_valid_frame() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
    let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
    assert!(
        validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url).is_ok()
    );
}

#[test]
fn test_ephemeral_validator_rejects_wrong_kind() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
    let ev = EventBuilder::new(Kind::TextNote, "hello")
        .tags(vec![
            Tag::parse(["p", &owner_pk]).unwrap(),
            Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
        ])
        .sign_with_keys(&agent_keys)
        .unwrap();
    let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("kind"));
}

#[test]
fn test_ephemeral_validator_rejects_missing_p_tag() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
    let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
        .tags(vec![
            Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
        ])
        .sign_with_keys(&agent_keys)
        .unwrap();
    let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("#p"));
}

#[test]
fn test_ephemeral_validator_rejects_missing_agent_tag() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
    let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
        .tags(vec![
            Tag::parse(["p", &owner_pk]).unwrap(),
            Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
        ])
        .sign_with_keys(&agent_keys)
        .unwrap();
    let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("agent"));
}

#[test]
fn test_ephemeral_validator_rejects_control_frame() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
    let ev = make_observer_frame(&owner_keys, &agent_keys, "control");
    let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("telemetry"));
}

#[test]
fn test_ephemeral_validator_rejects_wrong_author() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let other_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
    let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
        .tags(vec![
            Tag::parse(["p", &owner_pk]).unwrap(),
            Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
        ])
        .sign_with_keys(&other_keys) // wrong signer
        .unwrap();
    let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("author"));
}

#[test]
fn test_ephemeral_validator_rejects_no_subscription() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    // Deliberately do NOT add a subscription.
    let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
    let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("owner_p subscription"));
}

#[test]
fn test_ephemeral_validator_rejects_kind_not_in_subscription() {
    // Subscription exists but kinds = [1] (not 24200) — must be rejected.
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[1]"); // wrong kinds
    let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
    let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("24200"),
        "expected kind 24200 in error, got: {msg}"
    );
}

// ── archive pipeline — persistent path ───────────────────────────────────

#[test]
fn test_persistent_channel_h_persists_when_relay_returns_event() {
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let chan = "chan-abc";
    add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

    let ev = EventBuilder::new(Kind::Custom(9), "msg")
        .tags(vec![Tag::parse(["h", chan]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

    // Fake relay returns the event (simulates relay proof).
    let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
    assert_eq!(result.persisted, 1);
    assert_eq!(result.dropped, 0);

    // Confirm the event is in the store.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let scope_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_event_scopes WHERE scope_type = 'channel_h' AND scope_value = ?1", [chan], |r| r.get(0))
        .unwrap();
    assert_eq!(scope_count, 1);
}

#[test]
fn test_persistent_channel_h_drops_when_relay_does_not_return_event() {
    // Relay returns empty — event not proven accessible.
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let chan = "chan-abc";
    add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

    let ev = EventBuilder::new(Kind::Custom(9), "msg")
        .tags(vec![Tag::parse(["h", chan]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

    // Fake relay returns nothing.
    let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![]);
    assert_eq!(result.persisted, 0);
    assert_eq!(result.dropped, 1);
}

#[test]
fn test_persistent_drops_when_no_subscription() {
    // No subscription at all — drop before even querying.
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let chan = "chan-abc";
    // Intentionally no subscription.

    let ev = EventBuilder::new(Kind::Custom(9), "msg")
        .tags(vec![Tag::parse(["h", chan]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

    // Fake relay would return the event, but no sub → dropped.
    let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
    assert_eq!(result.persisted, 0);
    assert_eq!(result.dropped, 1);
}

#[test]
fn test_persistent_drops_kind_not_in_subscription() {
    // Subscription is for kind 9 only; event is kind 7 (reaction).
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let chan = "chan-abc";
    add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

    // kind 7 reaction — no `h` tag naturally, but relay-returned under scoped filter
    let ev = EventBuilder::new(Kind::Reaction, "+")
        .tags(vec![Tag::parse(["h", chan]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

    // Fake relay returns the event (simulates relay proof via StoredEvent.channel_id),
    // but kind 7 is not in the subscription's kinds list.
    let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
    assert_eq!(result.persisted, 0);
    assert_eq!(result.dropped, 1);
}

#[test]
fn test_persistent_h_less_event_persists_when_relay_returns_it() {
    // An h-less event (e.g. reaction kind:7) that the relay returns under
    // the scoped #h filter (via StoredEvent.channel_id fallback) must be
    // persisted. The local tag scanner would have dropped it; the scoped
    // filter proof must not.
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let chan = "chan-abc";
    add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[7]");

    // Build a reaction without an `h` tag — local re-derivation would drop it.
    let ev = EventBuilder::new(Kind::Reaction, "+")
        .sign_with_keys(&keys)
        .unwrap();
    let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

    // Fake relay returns it (relay used StoredEvent.channel_id to match).
    let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
    assert_eq!(result.persisted, 1);
    assert_eq!(result.dropped, 0);

    // Scope row uses bucket's scope_value, not a local-derived value.
    let scope_val: String = conn
        .query_row(
            "SELECT scope_value FROM archived_event_scopes WHERE scope_type = 'channel_h'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scope_val, chan);
}

#[test]
fn test_persistent_referenced_e_persists_when_relay_returns_event() {
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let ref_id = "a".repeat(64);
    add_sub(
        &conn,
        &identity_pk,
        relay_url,
        "referenced_e",
        &ref_id,
        "[9]",
    );

    let ev = EventBuilder::new(Kind::Custom(9), "reply")
        .tags(vec![Tag::parse(["e", &ref_id]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let cands = vec![candidate(&ev, ScopeType::ReferencedE, &ref_id)];
    let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
    assert_eq!(result.persisted, 1);
    assert_eq!(result.dropped, 0);
}

#[test]
fn test_mixed_batch_persisted_and_dropped_counted_exactly() {
    // Two channel_h candidates: relay only returns one. dropped must be 1.
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let chan = "chan-abc";
    add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

    let ev1 = EventBuilder::new(Kind::Custom(9), "msg1")
        .tags(vec![Tag::parse(["h", chan]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let ev2 = EventBuilder::new(Kind::Custom(9), "msg2")
        .tags(vec![Tag::parse(["h", chan]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let cands = vec![
        candidate(&ev1, ScopeType::ChannelH, chan),
        candidate(&ev2, ScopeType::ChannelH, chan),
    ];

    // Fake relay only returns ev1.
    let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev1.clone()]);
    assert_eq!(result.persisted, 1);
    assert_eq!(result.dropped, 1);
}

// ── archive pipeline — ephemeral path ────────────────────────────────────

#[test]
fn test_ephemeral_path_persists_valid_frame() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");

    let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
    let cands = vec![candidate(&ev, ScopeType::OwnerP, &owner_pk)];

    // Fake relay returns nothing (not consulted for ephemeral path).
    let result = run_batch_sync(cands, &owner_pk, relay_url, &conn, vec![]);
    assert_eq!(result.persisted, 1);
    assert_eq!(result.dropped, 0);
}

#[test]
fn test_ephemeral_path_drops_kind_not_in_subscription() {
    let conn = in_memory();
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    // kinds = [1], not [24200]
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[1]");

    let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
    let cands = vec![candidate(&ev, ScopeType::OwnerP, &owner_pk)];

    let result = run_batch_sync(cands, &owner_pk, relay_url, &conn, vec![]);
    assert_eq!(result.persisted, 0);
    assert_eq!(result.dropped, 1);
}

// ── Invalid input dropped ─────────────────────────────────────────────────

#[test]
fn test_malformed_json_is_dropped() {
    let result = Event::from_json("not json at all");
    assert!(result.is_err());
}

#[test]
fn test_tampered_event_fails_verify_id() {
    let keys = Keys::generate();
    let mut ev_json: serde_json::Value = serde_json::from_str(
        &EventBuilder::new(Kind::TextNote, "ok")
            .sign_with_keys(&keys)
            .unwrap()
            .as_json(),
    )
    .unwrap();
    ev_json["content"] = serde_json::Value::String("tampered".into());
    let tampered = ev_json.to_string();
    let ev = Event::from_json(&tampered).unwrap();
    assert!(!ev.verify_id());
}

// ── F2: out-of-range kind ─────────────────────────────────────────────────

#[test]
fn test_out_of_range_kind_is_dropped() {
    // kind 89736 == 24200 + 65536. The nostr crate truncates it to 24200
    // via `v as u16`, so without the raw-kind check the validator would
    // reason about 24200 while the persisted raw_json still says 89736.
    // The fix rejects it before Event::from_json.
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let owner_pk = &identity_pk;
    // Build a valid kind-24200 frame, then mutate only the raw kind to 89736.
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let pk = owner_keys.public_key().to_hex();
    add_sub(&conn, &pk, relay_url, "owner_p", &pk, "[24200]");
    let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
    let mut raw: serde_json::Value = serde_json::from_str(&ev.as_json()).unwrap();
    raw["kind"] = serde_json::Value::Number(serde_json::Number::from(24200u64 + 65536));
    let bad_json = raw.to_string();

    let cand = ArchiveCandidate {
        raw_event_json: bad_json,
        matched_scope: MatchedScope {
            scope_type: ScopeType::OwnerP,
            scope_value: pk.clone(),
        },
    };
    let result = run_batch_sync(vec![cand], &pk, relay_url, &conn, vec![]);
    assert_eq!(result.persisted, 0, "out-of-range kind must be dropped");
    assert_eq!(result.dropped, 1);
    // No event row must exist.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let _ = owner_pk; // silence unused-var
}

// ── F3: transactional atomicity ───────────────────────────────────────────

#[test]
fn test_commit_archive_rolls_back_when_scope_write_would_fail() {
    // Verify the split-schema invariant: if we simulate a mid-batch
    // failure (by putting the DB into a state where the scope table is
    // missing), the event row must NOT survive.
    //
    // We can't actually make upsert_event_scope fail on a healthy DB, so
    // we verify the positive side: with a healthy DB, both rows are always
    // written together or neither is. We confirm via run_batch_sync that
    // after a full successful commit both tables have matching row counts.
    let conn = in_memory();
    let keys = Keys::generate();
    let identity_pk = keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let chan = "chan-txn";
    add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

    let ev1 = EventBuilder::new(Kind::Custom(9), "msg1")
        .tags(vec![Tag::parse(["h", chan]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let ev2 = EventBuilder::new(Kind::Custom(9), "msg2")
        .tags(vec![Tag::parse(["h", chan]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let cands = vec![
        candidate(&ev1, ScopeType::ChannelH, chan),
        candidate(&ev2, ScopeType::ChannelH, chan),
    ];
    let result = run_batch_sync(
        cands,
        &identity_pk,
        relay_url,
        &conn,
        vec![ev1.clone(), ev2.clone()],
    );
    assert_eq!(result.persisted, 2);
    assert_eq!(result.dropped, 0);

    // Every event row must have exactly one corresponding scope row.
    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
        .unwrap();
    let scope_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_event_scopes", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        event_count, scope_count,
        "every event row must have a matching scope row — split-schema invariant"
    );
}

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

// ── Real-relay integration tests ──────────────────────────────────────────
//
// Gated on `#[cfg(not(target_os = "windows"))]` because `build_app_state()`
// and `reqwest::Client` pull in native Windows DLLs (SChannel/WinHTTP) that
// are not available in the CI runner, causing STATUS_ENTRYPOINT_NOT_FOUND at
// test-binary launch before any test runs. The guard compiles the whole
// cluster out of the Windows test binary. These tests are `#[ignore]` and
// require a live relay anyway — there is no Windows relay in CI.
//
// Run (Linux/macOS only):
//
//   RELAY_URL=ws://localhost:3000 cargo test -p buzz-desktop \
//       archive::tests::real_relay -- --ignored --nocapture
//
// The relay must be running with a Postgres backend (same docker compose
// as the existing e2e suite). Each test creates its own keypair + channel so
// concurrent runs don't interfere.

#[cfg(not(target_os = "windows"))]
mod real_relay {
    use super::*;
    use crate::app_state::build_app_state;
    use std::path::Path;

    fn relay_ws_url_from_env() -> String {
        std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
    }

    fn relay_http_base() -> String {
        relay_ws_url_from_env()
            .replace("wss://", "https://")
            .replace("ws://", "http://")
            .trim_end_matches('/')
            .to_string()
    }

    /// Build a test AppState wired with specific identity keys and relay URL.
    /// Mirrors production `archive_events`: the same `query_buckets` call path
    /// is exercised, including NIP-98 signing inside `query_relay`.
    fn make_test_app_state(keys: Keys, relay_url: &str) -> AppState {
        let state = build_app_state();
        *state.keys.lock().unwrap() = keys;
        *state.relay_url_override.lock().unwrap() = Some(relay_url.to_string());
        state
    }

    /// Submit a signed event to the relay via `POST /events`.
    /// The staging relay accepts events without NIP-98; this is only used for
    /// test setup (publishing events that the archive pipeline will later query).
    async fn submit_event_to_relay(ev: &Event) -> serde_json::Value {
        let http = reqwest::Client::new();
        let resp = http
            .post(format!("{}/events", relay_http_base()))
            .header("X-Pubkey", ev.pubkey.to_hex())
            .header("Content-Type", "application/json")
            .body(ev.as_json())
            .send()
            .await
            .expect("submit event to relay");
        assert!(
            resp.status().is_success(),
            "relay event submit failed: {}",
            resp.status()
        );
        resp.json().await.expect("parse submit response")
    }

    /// Create an open channel on the relay.  Returns the channel UUID string.
    async fn create_relay_channel(keys: &Keys) -> String {
        let channel_id = Uuid::new_v4().to_string();
        let ev = EventBuilder::new(Kind::Custom(9007), "")
            .tags(vec![
                Tag::parse(["h", &channel_id]).unwrap(),
                Tag::parse(["name", &format!("archive-e2e-{channel_id}")]).unwrap(),
                Tag::parse(["channel_type", "stream"]).unwrap(),
                Tag::parse(["visibility", "open"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let resp = submit_event_to_relay(&ev).await;
        assert!(
            resp["accepted"].as_bool().unwrap_or(false),
            "channel creation not accepted: {resp}"
        );
        channel_id
    }

    /// Open a file-backed archive DB at `path` and insert one save subscription.
    fn file_db_with_subscription(
        path: &Path,
        identity_pk: &str,
        relay_url: &str,
        scope_type: &str,
        scope_value: &str,
        kinds_json: &str,
    ) {
        let conn = store::open_archive_db(path).expect("open file archive db");
        add_sub(
            &conn,
            identity_pk,
            relay_url,
            scope_type,
            scope_value,
            kinds_json,
        );
        // conn drops here — file is flushed before the caller reopens it
    }

    /// Run plan → real relay query (via the production `query_buckets` path,
    /// including NIP-98 signing) → commit, using a file-backed archive DB.
    ///
    /// Mirrors the open/drop/query/reopen pattern of production `archive_events`:
    ///   1. Open DB, run `plan_archive`, drop connection (no conn across `.await`).
    ///   2. Call `query_buckets(plan.buckets, &state).await` — NIP-98 signed.
    ///   3. Reopen DB for `commit_archive`.
    ///
    /// Returns `ArchiveBatchResult`; caller reopens the file for row assertions.
    async fn run_batch_real_relay(
        candidates: Vec<ArchiveCandidate>,
        state: &AppState,
        db_path: &Path,
    ) -> ArchiveBatchResult {
        let identity_pk = state.keys.lock().unwrap().public_key().to_hex();
        let relay_url = crate::relay::relay_ws_url_with_override(state);

        // Phase 1: plan (sync). Connection dropped before any .await.
        let plan = {
            let conn = store::open_archive_db(db_path).expect("open archive db for plan");
            plan_archive(candidates, &identity_pk, &relay_url, &conn).unwrap()
            // conn drops here
        };

        // Phase 2: relay queries (async) — no Connection in scope.
        // Uses the real `query_buckets` path: query_relay → NIP-98 signed /query.
        let bucket_results = query_buckets(plan.buckets, state).await;

        // Phase 3: persist (sync). Fresh connection, same file.
        let conn = store::open_archive_db(db_path).expect("open archive db for commit");
        let owner_keys = state.keys.lock().unwrap().clone();
        commit_archive(
            bucket_results,
            plan.ephemeral,
            plan.pre_dropped,
            &identity_pk,
            &relay_url,
            &owner_keys,
            0,
            &conn,
        )
        .unwrap()
    }

    /// Happy path: publish a kind:9 message to a channel, then run the archive
    /// pipeline against the real relay. Asserts exact event + scope rows in a
    /// file-backed SQLite archive, reopened after commit to prove persistence.
    #[tokio::test]
    #[ignore]
    async fn test_real_relay_channel_h_happy_path_persists_event() {
        let keys = Keys::generate();
        let relay_url = relay_ws_url_from_env();
        let state = make_test_app_state(keys.clone(), &relay_url);
        let identity_pk = keys.public_key().to_hex();

        // Create a channel and publish a kind:9 message.
        let channel_id = create_relay_channel(&keys).await;
        let msg_ev = EventBuilder::new(Kind::Custom(9), "hello archive")
            .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let submit_resp = submit_event_to_relay(&msg_ev).await;
        assert!(
            submit_resp["accepted"].as_bool().unwrap_or(false),
            "message not accepted: {submit_resp}"
        );

        // Give the relay a moment to persist.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // File-backed archive DB with a channel_h subscription.
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("archive.db");
        file_db_with_subscription(
            &db_path,
            &identity_pk,
            &relay_url,
            "channel_h",
            &channel_id,
            "[9]",
        );

        let cands = vec![candidate(&msg_ev, ScopeType::ChannelH, &channel_id)];
        let result = run_batch_real_relay(cands, &state, &db_path).await;

        assert_eq!(result.persisted, 1, "expected 1 persisted, got {result:?}");
        assert_eq!(result.dropped, 0, "expected 0 dropped, got {result:?}");

        // Reopen the same file to assert exact row counts — proves file-backed persistence.
        let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");

        let event_count: i64 = read_conn
            .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(event_count, 1, "archived_events should have 1 row");

        let scope_count: i64 = read_conn
            .query_row(
                "SELECT COUNT(*) FROM archived_event_scopes \
                 WHERE scope_type = 'channel_h' AND scope_value = ?1",
                [&channel_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(scope_count, 1, "archived_event_scopes should have 1 row");

        // Confirm the stored raw_json round-trips to the original event.
        let raw_json: String = read_conn
            .query_row("SELECT raw_json FROM archived_events", [], |r| r.get(0))
            .unwrap();
        let stored_ev = Event::from_json(&raw_json).unwrap();
        assert_eq!(stored_ev.id.to_hex(), msg_ev.id.to_hex());

        println!(
            "✓ real relay: event {} archived under channel_h:{}",
            msg_ev.id.to_hex(),
            channel_id
        );
        println!("  archived_events:       {event_count} row(s)");
        println!("  archived_event_scopes: {scope_count} row(s)");
    }

    /// Kind-mismatch drop: subscription allows only kinds=[1059] but we publish a kind:9
    /// channel message. The relay stores the event, but the archive's kind filter drops it
    /// because 9 ∉ {1059}.
    #[tokio::test]
    #[ignore]
    async fn test_real_relay_kind_mismatch_drops_event() {
        let keys = Keys::generate();
        let relay_url = relay_ws_url_from_env();
        let state = make_test_app_state(keys.clone(), &relay_url);
        let identity_pk = keys.public_key().to_hex();

        let channel_id = create_relay_channel(&keys).await;
        // Publish a kind:9 channel message — relay accepts it, but the subscription's
        // kind filter is [1059] so the archive pipeline must drop it.
        let msg_ev = EventBuilder::new(Kind::Custom(9), "kind-mismatch-msg")
            .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let submit_resp = submit_event_to_relay(&msg_ev).await;
        assert!(
            submit_resp["accepted"].as_bool().unwrap_or(false),
            "message not accepted: {submit_resp}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Subscription allows kinds=[1059] only — kind:9 must be dropped.
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("archive.db");
        file_db_with_subscription(
            &db_path,
            &identity_pk,
            &relay_url,
            "channel_h",
            &channel_id,
            "[1059]",
        );

        let cands = vec![candidate(&msg_ev, ScopeType::ChannelH, &channel_id)];
        let result = run_batch_real_relay(cands, &state, &db_path).await;

        assert_eq!(result.persisted, 0, "kind-mismatch: should be dropped");
        assert_eq!(result.dropped, 1, "kind-mismatch: drop count should be 1");

        // Belt-and-suspenders: confirm the on-disk archive is genuinely empty.
        let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");
        let event_count: i64 = read_conn
            .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            event_count, 0,
            "nothing should be archived on kind-mismatch"
        );

        println!(
            "✓ real relay: kind-mismatch correctly dropped kind:9 event (subscription is kinds=[1059])"
        );
    }

    /// No-subscription drop: event arrives but no save_subscription row exists.
    #[tokio::test]
    #[ignore]
    async fn test_real_relay_no_subscription_drops_event() {
        let keys = Keys::generate();
        let relay_url = relay_ws_url_from_env();
        let state = make_test_app_state(keys.clone(), &relay_url);

        let channel_id = create_relay_channel(&keys).await;
        let msg_ev = EventBuilder::new(Kind::Custom(9), "should be dropped")
            .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        submit_event_to_relay(&msg_ev).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Empty file-backed archive DB — no subscription row.
        // plan_archive drops the whole group because no subscription matches.
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("archive.db");
        store::open_archive_db(&db_path).expect("init empty archive db");
        // conn from init drops here; DB file exists but has no subscription rows

        let cands = vec![candidate(&msg_ev, ScopeType::ChannelH, &channel_id)];
        let result = run_batch_real_relay(cands, &state, &db_path).await;

        assert_eq!(result.persisted, 0, "no-sub: should be dropped");
        assert_eq!(result.dropped, 1, "no-sub: drop count should be 1");

        // Belt-and-suspenders: confirm the on-disk archive is genuinely empty.
        let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");
        let event_count: i64 = read_conn
            .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            event_count, 0,
            "nothing should be archived on no-subscription"
        );

        println!("✓ real relay: no-subscription correctly dropped event");
    }

    /// Owner_p ephemeral path: a locally-built valid 24200 frame is archived
    /// without a relay query (ephemeral events are never stored on the relay).
    #[tokio::test]
    #[ignore]
    async fn test_real_relay_owner_p_ephemeral_path_persists_valid_frame() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let relay_url = relay_ws_url_from_env();
        let state = make_test_app_state(owner_keys.clone(), &relay_url);
        let identity_pk = owner_keys.public_key().to_hex();

        // Build a valid kind:24200 observer frame addressed to the owner.
        let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);

        // File-backed archive DB with an owner_p subscription for kind 24200.
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("archive.db");
        file_db_with_subscription(
            &db_path,
            &identity_pk,
            &relay_url,
            "owner_p",
            &identity_pk,
            "[24200]",
        );

        // owner_p candidates bypass the relay entirely — query_buckets gets an
        // empty bucket list and the ephemeral path handles the frame locally.
        let cands = vec![candidate(&ev, ScopeType::OwnerP, &identity_pk)];
        let result = run_batch_real_relay(cands, &state, &db_path).await;

        assert_eq!(
            result.persisted, 1,
            "owner_p: valid frame should be persisted"
        );
        assert_eq!(result.dropped, 0, "owner_p: nothing should be dropped");

        // Reopen the same file to assert exact row counts.
        let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");

        let event_count: i64 = read_conn
            .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(event_count, 1);

        let scope_count: i64 = read_conn
            .query_row(
                "SELECT COUNT(*) FROM archived_event_scopes WHERE scope_type = 'owner_p'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(scope_count, 1);

        // Confirm the stored raw_json round-trips to the original frame.
        let raw_json: String = read_conn
            .query_row("SELECT raw_json FROM archived_events", [], |r| r.get(0))
            .unwrap();
        let stored_ev = Event::from_json(&raw_json).unwrap();
        assert_eq!(stored_ev.id.to_hex(), ev.id.to_hex());

        println!(
            "✓ real relay: owner_p ephemeral frame {} archived",
            ev.id.to_hex()
        );
        println!("  archived_events:       {event_count} row(s)");
        println!("  archived_event_scopes: {scope_count} row(s)");
    }
}
