//! Adoption community-boundary fence (Carl r11 P1): an adoption started in
//! community A but completed after a workspace switch to B must be rejected
//! before ANY store mutation, so A's team is never committed into B and A's
//! owner heads are never enqueued in B's retention db.
//!
//! `add_verified_team` captures the retention scope before the relay round-trip
//! and, under the store lock, runs `assert_adoption_scope_unchanged` against the
//! live workspace before planning or committing. These tests drive that exact
//! sequence — fence, then `plan_add`, then `commit_and_enqueue` against real
//! temp stores and a real retention scope — with the AppHandle reads supplied
//! directly. Deleting the fence lets the commit write both stores and create a
//! retention db, turning the switch tests RED.

use super::super::apply::{assert_adoption_scope_unchanged, commit_and_enqueue, plan_add};
use super::*;
use crate::managed_agents::retention::{scoped_retention_db_path, RetentionScope};

const RELAY_A: &str = "wss://tenant-a.example";
const RELAY_B: &str = "wss://tenant-b.example";

/// A retention scope keyed to `relay` and freshly generated owner keys.
fn scope(dir: &std::path::Path, relay: &str) -> RetentionScope {
    let keys = nostr::Keys::generate();
    let db_path = scoped_retention_db_path(dir, relay, &keys.public_key().to_hex());
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    RetentionScope {
        db_path,
        relay_url: relay.to_string(),
        owner_keys: keys,
    }
}

/// The `add_verified_team` sequence with the AppHandle reads injected: fence
/// against `(live_api_base_url, live_signer_hex)`, then plan + commit the
/// captured `scope`. Returns the fence/commit result plus whether the store
/// write ran, so a test can prove the commit is gated on the fence.
fn run_adoption_with_live_workspace(
    captured: RetentionScope,
    live_api_base_url: &str,
    live_signer_hex: &str,
    personas_path: &std::path::Path,
    teams_path: &std::path::Path,
) -> (Result<(), String>, bool) {
    let committed = std::cell::Cell::new(false);
    let result = (|| {
        assert_adoption_scope_unchanged(&captured, live_api_base_url, live_signer_hex)?;
        let source = source(&"a".repeat(64));
        let body = content(vec![member("m1", "Do the work.")]);
        let plan = plan_add(&[], &[], &source, &body, NOW)?;
        commit_and_enqueue(
            plan,
            |personas, teams| {
                committed.set(true);
                std::fs::write(personas_path, serde_json::to_vec(personas).unwrap())
                    .map_err(|e| e.to_string())?;
                std::fs::write(teams_path, serde_json::to_vec(teams).unwrap())
                    .map_err(|e| e.to_string())?;
                Ok(())
            },
            || Ok(captured),
        )?;
        Ok(())
    })();
    (result, committed.get())
}

/// A relay switch between capture and commit is rejected before any write: the
/// stores stay byte-unchanged and no retention db is created. Deleting the
/// fence lets the commit run, turning this RED.
#[test]
fn a_relay_switch_before_commit_is_rejected_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let personas_path = dir.path().join("personas.json");
    let teams_path = dir.path().join("teams.json");
    std::fs::write(&personas_path, b"[]").unwrap();
    std::fs::write(&teams_path, b"[]").unwrap();

    // Captured in community A; the workspace is now on community B's relay,
    // still the same owner identity (the community changed, not the login).
    let captured = scope(dir.path(), RELAY_A);
    let live_signer = captured.owner_keys.public_key().to_hex();
    let (result, committed) = run_adoption_with_live_workspace(
        captured,
        &crate::relay::relay_http_base_url(RELAY_B),
        &live_signer,
        &personas_path,
        &teams_path,
    );

    let error = result.expect_err("a relay switch must reject the adoption");
    assert!(
        error.contains("active community changed"),
        "the rejection must name the community boundary: {error}"
    );
    assert!(!committed, "the commit must not run when the fence rejects");
    assert_eq!(
        std::fs::read(&personas_path).unwrap(),
        b"[]",
        "the personas store is byte-unchanged on a fenced adoption"
    );
    assert_eq!(
        std::fs::read(&teams_path).unwrap(),
        b"[]",
        "the teams store is byte-unchanged on a fenced adoption"
    );
}

/// A same-relay identity switch is also rejected: relay + owner jointly key the
/// retention scope, so the owner half of the fence is load-bearing. Guards
/// against a future narrowing to a relay-only check.
#[test]
fn a_same_relay_identity_switch_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let personas_path = dir.path().join("personas.json");
    let teams_path = dir.path().join("teams.json");
    std::fs::write(&personas_path, b"[]").unwrap();
    std::fs::write(&teams_path, b"[]").unwrap();

    let captured = scope(dir.path(), RELAY_A);
    // Same relay, different owner — a login switch on the same community.
    let switched_signer = nostr::Keys::generate().public_key().to_hex();
    let (result, committed) = run_adoption_with_live_workspace(
        captured,
        &crate::relay::relay_http_base_url(RELAY_A),
        &switched_signer,
        &personas_path,
        &teams_path,
    );

    let error = result.expect_err("an identity switch must reject the adoption");
    assert!(
        error.contains("active identity changed"),
        "the rejection must name the identity boundary: {error}"
    );
    assert!(!committed, "the commit must not run when the fence rejects");
    assert_eq!(std::fs::read(&teams_path).unwrap(), b"[]");
}

/// The happy path — no switch — passes the fence and commits normally, so the
/// fence does not break ordinary adoption.
#[test]
fn an_unchanged_workspace_passes_the_fence_and_commits() {
    let dir = tempfile::tempdir().unwrap();
    let personas_path = dir.path().join("personas.json");
    let teams_path = dir.path().join("teams.json");
    std::fs::write(&personas_path, b"[]").unwrap();
    std::fs::write(&teams_path, b"[]").unwrap();

    let captured = scope(dir.path(), RELAY_A);
    let live_signer = captured.owner_keys.public_key().to_hex();
    let (result, committed) = run_adoption_with_live_workspace(
        captured,
        &crate::relay::relay_http_base_url(RELAY_A),
        &live_signer,
        &personas_path,
        &teams_path,
    );

    result.expect("an unchanged workspace must adopt normally");
    assert!(committed, "the commit runs when the fence passes");
    assert_ne!(
        std::fs::read(&teams_path).unwrap(),
        b"[]",
        "the adopted team is written"
    );
}
