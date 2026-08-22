//! Durability and idempotency tests for the broker execution log.

use super::*;

const RELAY: &str = "wss://relay.example";
const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REQUESTER: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const OTHER: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const DIGEST: &str = "aaaa";

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn
}

fn key() -> ExecutionKey {
    ExecutionKey {
        relay_scope: RELAY.into(),
        owner_pubkey: OWNER.into(),
        requester_pubkey: REQUESTER.into(),
        capability: "agents.create".into(),
        request_id: "req-1".into(),
    }
}

#[test]
fn first_claim_wins_and_records_executing_before_any_side_effect() {
    let mut conn = db();
    assert_eq!(
        claim(&mut conn, &key(), DIGEST, 1, 100).unwrap(),
        ClaimOutcome::Claimed
    );

    // The row exists and is `executing` even though nothing has completed:
    // that is what makes a crash visible instead of invisible.
    let record = get(&conn, &key()).unwrap().unwrap();
    assert_eq!(record.state, ExecutionState::Executing);
    assert_eq!(record.request_digest, DIGEST);
    assert!(record.result_json.is_none());
}

#[test]
fn terminal_row_replays_the_recorded_result_instead_of_re_executing() {
    let mut conn = db();
    claim(&mut conn, &key(), DIGEST, 1, 100).unwrap();
    complete(
        &conn,
        &key(),
        ExecutionState::Succeeded,
        r#"{"status":"succeeded"}"#,
        200,
    )
    .unwrap();

    match claim(&mut conn, &key(), DIGEST, 1, 300).unwrap() {
        ClaimOutcome::Replay(record) => {
            assert_eq!(record.state, ExecutionState::Succeeded);
            assert_eq!(record.result_json.unwrap(), r#"{"status":"succeeded"}"#);
        }
        other => panic!("expected replay, got {other:?}"),
    }
}

#[test]
fn a_failed_result_also_replays_rather_than_retrying() {
    let mut conn = db();
    claim(&mut conn, &key(), DIGEST, 1, 100).unwrap();
    complete(
        &conn,
        &key(),
        ExecutionState::Failed,
        r#"{"status":"failed"}"#,
        200,
    )
    .unwrap();

    match claim(&mut conn, &key(), DIGEST, 1, 300).unwrap() {
        ClaimOutcome::Replay(record) => assert_eq!(record.state, ExecutionState::Failed),
        other => panic!("expected replay, got {other:?}"),
    }
}

/// The core safety property: a request that started and never finished must
/// never be blindly re-executed, because side effects may already exist.
#[test]
fn an_interrupted_execution_becomes_indeterminate_and_is_never_re_executed() {
    let mut conn = db();
    claim(&mut conn, &key(), DIGEST, 1, 100).unwrap();
    // No `complete` — simulates a crash mid-execution.

    match claim(&mut conn, &key(), DIGEST, 1, 200).unwrap() {
        ClaimOutcome::Interrupted(record) => {
            assert_eq!(record.state, ExecutionState::Indeterminate);
        }
        other => panic!("expected interrupted, got {other:?}"),
    }
    assert_eq!(
        get(&conn, &key()).unwrap().unwrap().state,
        ExecutionState::Indeterminate
    );

    // And it stays indeterminate: a third attempt still refuses to run.
    match claim(&mut conn, &key(), DIGEST, 1, 300).unwrap() {
        ClaimOutcome::Replay(record) => {
            assert_eq!(record.state, ExecutionState::Indeterminate)
        }
        other => panic!("expected replay of indeterminate, got {other:?}"),
    }
}

#[test]
fn same_request_id_with_different_content_is_a_conflict() {
    let mut conn = db();
    claim(&mut conn, &key(), DIGEST, 1, 100).unwrap();

    match claim(&mut conn, &key(), "bbbb", 1, 200).unwrap() {
        ClaimOutcome::DigestConflict { expected } => assert_eq!(expected, DIGEST),
        other => panic!("expected conflict, got {other:?}"),
    }
}

/// A conflicting retry must never be answered with the first request's result.
#[test]
fn digest_conflict_is_reported_even_when_a_result_already_exists() {
    let mut conn = db();
    claim(&mut conn, &key(), DIGEST, 1, 100).unwrap();
    complete(
        &conn,
        &key(),
        ExecutionState::Succeeded,
        r#"{"status":"succeeded"}"#,
        200,
    )
    .unwrap();

    match claim(&mut conn, &key(), "different", 1, 300).unwrap() {
        ClaimOutcome::DigestConflict { expected } => assert_eq!(expected, DIGEST),
        other => panic!("a conflicting payload must not receive the stored result: {other:?}"),
    }
}

/// The key includes requester and owner, so one requester's id cannot collide
/// with another's — or leak another's recorded outcome.
#[test]
fn the_key_separates_requesters_owners_relays_and_capabilities() {
    let mut conn = db();
    claim(&mut conn, &key(), DIGEST, 1, 100).unwrap();

    let variants = [
        ExecutionKey {
            requester_pubkey: OTHER.into(),
            ..key()
        },
        ExecutionKey {
            owner_pubkey: OTHER.into(),
            ..key()
        },
        ExecutionKey {
            relay_scope: "wss://other.example".into(),
            ..key()
        },
        ExecutionKey {
            capability: "agents.delete".into(),
            ..key()
        },
    ];
    for variant in variants {
        assert_eq!(
            claim(&mut conn, &variant, DIGEST, 1, 100).unwrap(),
            ClaimOutcome::Claimed,
            "{variant:?} must be a distinct execution"
        );
    }
}

#[test]
fn complete_refuses_a_non_terminal_state() {
    let mut conn = db();
    claim(&mut conn, &key(), DIGEST, 1, 100).unwrap();
    let error = complete(&conn, &key(), ExecutionState::Executing, "{}", 200).unwrap_err();
    assert!(error.contains("cannot complete"), "unexpected: {error}");
}

#[test]
fn complete_refuses_an_unclaimed_key() {
    let conn = db();
    let error = complete(&conn, &key(), ExecutionState::Succeeded, "{}", 200).unwrap_err();
    assert!(error.contains("missing"), "unexpected: {error}");
}

#[test]
fn states_round_trip_through_their_database_encoding() {
    for state in [
        ExecutionState::Executing,
        ExecutionState::Succeeded,
        ExecutionState::Failed,
        ExecutionState::Indeterminate,
    ] {
        assert_eq!(ExecutionState::parse(state.as_str()).unwrap(), state);
    }
    assert!(ExecutionState::parse("bogus").is_err());
    assert!(!ExecutionState::Executing.is_terminal());
    for terminal in [
        ExecutionState::Succeeded,
        ExecutionState::Failed,
        ExecutionState::Indeterminate,
    ] {
        assert!(terminal.is_terminal());
    }
}

/// Equivalent relay URLs must resolve to one scope, so "same relay" cannot
/// disagree with "same database".
#[test]
fn relay_scope_and_db_path_normalize_equivalent_urls() {
    assert_eq!(
        normalized_relay_scope("  wss://r.example/  "),
        "wss://r.example"
    );

    let base = Path::new("/tmp/nest");
    let a = scoped_broker_db_path(base, "wss://r.example", OWNER);
    let b = scoped_broker_db_path(base, " wss://r.example/ ", &OWNER.to_ascii_uppercase());
    assert_eq!(a, b);

    // Different owner or relay must not share a database.
    assert_ne!(a, scoped_broker_db_path(base, "wss://r.example", OTHER));
    assert_ne!(a, scoped_broker_db_path(base, "wss://other.example", OWNER));

    // The relay URL never becomes a path component.
    assert!(!a.to_string_lossy().contains("r.example"));
}

#[test]
fn opening_the_db_twice_is_idempotent_and_preserves_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = scoped_broker_db_path(dir.path(), RELAY, OWNER);

    let mut conn = open_broker_db(&path).unwrap();
    claim(&mut conn, &key(), DIGEST, 1, 100).unwrap();
    complete(&conn, &key(), ExecutionState::Succeeded, "{}", 200).unwrap();
    drop(conn);

    let conn = open_broker_db(&path).unwrap();
    assert_eq!(
        get(&conn, &key()).unwrap().unwrap().state,
        ExecutionState::Succeeded
    );
}

/// Two racing frames carrying the same request must not both believe they
/// claimed it — exactly one executes, the other is told to stand down.
#[test]
fn concurrent_claims_produce_exactly_one_winner() {
    use std::sync::{Arc, Barrier};

    let dir = tempfile::tempdir().unwrap();
    let path = scoped_broker_db_path(dir.path(), RELAY, OWNER);
    // Materialize the schema before the threads race on it.
    drop(open_broker_db(&path).unwrap());

    let threads = 8;
    let barrier = Arc::new(Barrier::new(threads));
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let mut conn = open_broker_db(&path).unwrap();
                barrier.wait();
                claim(&mut conn, &key(), DIGEST, 1, 100).unwrap()
            })
        })
        .collect();

    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let claimed = outcomes
        .iter()
        .filter(|o| matches!(o, ClaimOutcome::Claimed))
        .count();
    assert_eq!(claimed, 1, "exactly one claim must win: {outcomes:?}");

    // Every loser saw the in-flight row rather than a free slot.
    for outcome in outcomes
        .iter()
        .filter(|o| !matches!(o, ClaimOutcome::Claimed))
    {
        assert!(
            matches!(
                outcome,
                ClaimOutcome::Interrupted(_) | ClaimOutcome::Replay(_)
            ),
            "unexpected loser outcome: {outcome:?}"
        );
    }
}
