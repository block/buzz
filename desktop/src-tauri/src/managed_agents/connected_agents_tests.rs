//! Cross-store tests for connected self-hosted agents.
//!
//! The lifecycle-exclusion invariants used to be enforced by a `key_custody`
//! filter inside `load_managed_agents`, and were tested by asserting that the
//! filter returned the right subset. With a separate type in a separate file
//! there is no filter to test — so what these cover instead is that the
//! separation is real: that the two stores cannot see each other's rows, that
//! neither type can be read out of the other's file, and that a write to one
//! cannot disturb the other.
//!
//! Together with the type itself (no key, no command, no pid — see
//! [`super::ConnectedAgentRecord`]) that is the whole of the old invariant set,
//! reproved at the boundary rather than at each consumer.

use std::fs;

use super::{
    load_connected_agents_at, normalize_community_url, save_connected_agents_at,
    ConnectedAgentRecord, ConnectedAgentSummary,
};
use crate::managed_agents::ManagedAgentRecord;

const CONNECTED_HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const OWNED_HEX: &str = "1bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa4591";

fn connected(pubkey: &str, name: &str, host: &str) -> ConnectedAgentRecord {
    ConnectedAgentRecord {
        pubkey: pubkey.to_string(),
        name: name.to_string(),
        host: host.to_string(),
        harness: Some("claude".to_string()),
        community: None,
        created_at: "2026-07-28T00:00:00Z".to_string(),
        updated_at: "2026-07-28T00:00:00Z".to_string(),
    }
}

/// A `managed-agents.json` payload as earlier builds wrote it.
fn managed_store_json() -> String {
    serde_json::json!([{
        "pubkey": OWNED_HEX,
        "name": "Owned",
        "private_key_nsec": "",
        "relay_url": "wss://localhost:3000",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }])
    .to_string()
}

#[test]
fn an_absent_store_is_an_empty_list_not_an_error() {
    // First run, and every run before the user connects anything. This must not
    // surface as a load failure in the agents view.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("connected-agents.json");
    assert_eq!(load_connected_agents_at(&path).unwrap(), Vec::new());
}

#[test]
fn records_round_trip_through_the_store() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("connected-agents.json");
    let records = vec![connected(CONNECTED_HEX, "Scout", "workstation")];

    save_connected_agents_at(&path, &records).expect("save");
    assert_eq!(load_connected_agents_at(&path).unwrap(), records);
}

#[test]
fn a_first_save_creates_the_store_and_a_second_overwrites_it_atomically() {
    // `atomic_write_json` canonicalizes its target to preserve a symlink, which
    // needs the file to exist — so the create-then-write path is load-bearing on
    // the very first connect, and a regression there would make connecting fail
    // only on a machine that had never connected anything.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("connected-agents.json");

    save_connected_agents_at(&path, &[connected(CONNECTED_HEX, "Scout", "workstation")])
        .expect("first");
    save_connected_agents_at(&path, &[connected(CONNECTED_HEX, "Scout", "buildbox")])
        .expect("second");

    let loaded = load_connected_agents_at(&path).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].host, "buildbox");
    assert!(
        !path.with_extension("json.tmp").exists(),
        "the atomic write must not leave its temp file behind"
    );
}

#[test]
fn records_are_sorted_for_stable_diffs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("connected-agents.json");
    let records = vec![
        connected(CONNECTED_HEX, "zeta", "workstation"),
        connected(OWNED_HEX, "Alpha", "buildbox"),
    ];

    save_connected_agents_at(&path, &records).expect("save");

    let names: Vec<String> = load_connected_agents_at(&path)
        .unwrap()
        .into_iter()
        .map(|record| record.name)
        .collect();
    assert_eq!(names, ["Alpha", "zeta"], "case-insensitive name order");
}

#[test]
fn a_malformed_store_fails_loudly_and_preserves_the_evidence() {
    // Matches `load_managed_agents`: a later in-app save rewrites this file
    // wholesale, so swallowing a parse error into an empty list would silently
    // destroy a hand edit.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("connected-agents.json");
    fs::write(&path, b"{ not an array").expect("seed");

    let error = load_connected_agents_at(&path).expect_err("a malformed store must not load as []");
    assert!(error.contains(".invalid"), "message must name the backup");
    assert!(
        path.with_extension("json.invalid").exists(),
        "the malformed content must survive for the user to recover"
    );
}

#[test]
fn a_connected_record_cannot_be_deserialized_as_a_managed_record() {
    // The type boundary, stated as data. Even a future reader that pointed at
    // the wrong file could not produce a `ManagedAgentRecord` from a connected
    // row: the fields every lifecycle path needs are not merely empty, they are
    // absent, so serde refuses. This is what replaces the custody filter — the
    // old design's connected rows WERE `ManagedAgentRecord`s and deserialized
    // happily, which is exactly why a missed filter was dangerous.
    let record = connected(CONNECTED_HEX, "Scout", "workstation");
    let json = serde_json::to_value(&record).unwrap();

    let parsed = serde_json::from_value::<ManagedAgentRecord>(json);
    assert!(
        parsed.is_err(),
        "a connected row must not satisfy ManagedAgentRecord"
    );
}

#[test]
fn a_managed_record_cannot_be_deserialized_as_a_connected_record() {
    // The converse, and the reason `host` is a plain `String`: an owned agent's
    // row has no host, so it cannot become a connected agent by being read out
    // of the wrong file. If `host` were `Option<String>` this would silently
    // succeed and produce a connected agent that can never be probed.
    let managed: serde_json::Value = serde_json::from_str(&managed_store_json()).unwrap();
    let first = managed.as_array().unwrap()[0].clone();

    let parsed = serde_json::from_value::<ConnectedAgentRecord>(first);
    assert!(
        parsed.is_err(),
        "an owned agent's row must not satisfy ConnectedAgentRecord"
    );
}

#[test]
fn the_two_stores_are_separate_files_and_a_connected_save_leaves_the_other_untouched() {
    // The invariant that most needed a guard before. Under the shared-file
    // design, `load_managed_agents` filtered connected rows out, so every one of
    // the dozens of existing `load … mutate … save_managed_agents` call sites
    // would have erased them without a deliberate re-read of the connected
    // third. Separate files remove the failure mode rather than compensating for
    // it: there is no shared payload to drop a half of.
    let dir = tempfile::tempdir().expect("temp dir");
    let managed_path = dir.path().join("managed-agents.json");
    let connected_path = dir.path().join("connected-agents.json");
    fs::write(&managed_path, managed_store_json()).expect("seed managed store");
    let before = fs::read(&managed_path).expect("read managed store");

    save_connected_agents_at(
        &connected_path,
        &[connected(CONNECTED_HEX, "Scout", "workstation")],
    )
    .expect("save connected");

    assert_eq!(
        fs::read(&managed_path).expect("re-read managed store"),
        before,
        "a connected save must not rewrite managed-agents.json at all"
    );

    // And the managed store still parses to exactly the agent it started with —
    // no connected row leaked into the reader that feeds spawn and deploy.
    let managed: Vec<ManagedAgentRecord> =
        serde_json::from_slice(&fs::read(&managed_path).unwrap()).unwrap();
    assert_eq!(managed.len(), 1);
    assert_eq!(managed[0].pubkey, OWNED_HEX);
    assert!(
        managed.iter().all(|record| record.pubkey != CONNECTED_HEX),
        "the connected agent must be invisible to the managed-agent reader"
    );
}

#[test]
fn a_connected_store_write_carries_no_secret_and_needs_no_restricted_mode() {
    // `managed-agents.json` is written `0o600` because it can carry plaintext
    // agent nsecs during a keyring outage. This store uses the ordinary write,
    // which is only correct because the type cannot hold a secret — so assert
    // the serialized bytes contain nothing key-shaped.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("connected-agents.json");
    save_connected_agents_at(&path, &[connected(CONNECTED_HEX, "Scout", "workstation")])
        .expect("save");

    let raw = fs::read_to_string(&path).expect("read back");
    for forbidden in ["nsec", "private_key", "auth_tag"] {
        assert!(
            !raw.contains(forbidden),
            "{forbidden} must never appear in the connected store: {raw}"
        );
    }
}

#[test]
fn the_summary_is_a_lossless_projection_of_the_record() {
    // Both types exist so a future storage field is not automatically exposed to
    // the UI. Today they carry the same six facts, and this pins that: if the
    // record gains a field the summary should not have, this test still passes,
    // but if the projection starts dropping or renaming one it fails.
    let record = connected(CONNECTED_HEX, "Scout", "workstation");
    let summary = ConnectedAgentSummary::from(&record);

    assert_eq!(summary.pubkey, record.pubkey);
    assert_eq!(summary.name, record.name);
    assert_eq!(summary.host, record.host);
    assert_eq!(summary.harness, record.harness);
    assert_eq!(summary.community, record.community);
    assert_eq!(summary.created_at, record.created_at);
    assert_eq!(summary.updated_at, record.updated_at);
}

#[test]
fn community_comparison_ignores_trailing_slashes_and_case() {
    assert_eq!(
        normalize_community_url("  wss://Relay.Example.com/ "),
        "wss://relay.example.com"
    );
}
