use super::*;
use crate::command_services::memory::{CredentialKeys, ReplicationResult};
use chrono::{TimeZone, Utc};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn tempdir() -> tempfile::TempDir {
    let base = std::env::current_dir().expect("current directory");
    tempfile::Builder::new()
        .prefix(".memory-sync-state-test-")
        .tempdir_in(base)
        .expect("temporary state directory")
}

fn state(last_successful_sync: &str) -> MemorySyncState {
    MemorySyncState {
        schema_version: 1,
        local_node_id: "node:macbook-command".to_string(),
        home_node_id: "node:home-command".to_string(),
        local_replication_cursor: 41,
        home_replication_cursor: 73,
        conflict_count: 2,
        last_successful_sync: last_successful_sync.to_string(),
        sync_interval_minutes: 30,
    }
}

#[test]
fn atomic_protected_state_survives_restart_and_preserves_real_metadata() {
    let directory = tempdir();
    let path = directory.path().join("command-memory-sync-state.json");
    persist(&path, &state("2026-07-25T01:30:00Z")).expect("persist state");

    let metadata = std::fs::metadata(&path).expect("state metadata");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

    let restarted = load_status(
        &path,
        Utc.with_ymd_and_hms(2026, 7, 25, 2, 0, 0)
            .single()
            .expect("time"),
    );
    assert_eq!(restarted.freshness, MemorySyncFreshness::Fresh);
    assert_eq!(
        restarted.local_node_id.as_deref(),
        Some("node:macbook-command")
    );
    assert_eq!(restarted.home_node_id.as_deref(), Some("node:home-command"));
    assert_eq!(restarted.local_replication_cursor, Some(41));
    assert_eq!(restarted.home_replication_cursor, Some(73));
    assert_eq!(restarted.conflict_count, Some(2));
    assert_eq!(
        restarted.last_successful_sync.as_deref(),
        Some("2026-07-25T01:30:00Z"),
    );
}

#[test]
fn derives_never_synced_stale_and_corrupt_without_claiming_freshness() {
    let directory = tempdir();
    let path = directory.path().join("command-memory-sync-state.json");
    assert_eq!(
        load_status(
            &path,
            Utc.with_ymd_and_hms(2026, 7, 25, 2, 0, 0)
                .single()
                .expect("time"),
        )
        .freshness,
        MemorySyncFreshness::NeverSynced,
    );

    persist(&path, &state("2026-07-25T00:59:59Z")).expect("persist stale state");
    assert_eq!(
        load_status(
            &path,
            Utc.with_ymd_and_hms(2026, 7, 25, 2, 0, 0)
                .single()
                .expect("time"),
        )
        .freshness,
        MemorySyncFreshness::Stale,
    );

    std::fs::write(&path, b"{not-json").expect("corrupt state");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("protect corrupt fixture");
    let corrupt = load_status(
        &path,
        Utc.with_ymd_and_hms(2026, 7, 25, 2, 0, 0)
            .single()
            .expect("time"),
    );
    assert_eq!(corrupt.freshness, MemorySyncFreshness::Corrupt);
    assert!(corrupt.local_replication_cursor.is_none());
}

fn replication_result(
    operation: &str,
    source: &str,
    target: &str,
    to_cursor: u64,
    conflict_count: u64,
    last_success: &str,
) -> ReplicationResult {
    ReplicationResult {
        status: "ok".to_string(),
        operation: operation.to_string(),
        source_node_id: source.to_string(),
        target_node_id: target.to_string(),
        from_cursor: 0,
        to_cursor,
        accepted: to_cursor,
        duplicates: 0,
        conflicts: 0,
        objects: to_cursor,
        tombstones: 0,
        pages: 1,
        target_conflict_count: conflict_count,
        last_success: last_success.to_string(),
    }
}

#[test]
fn persists_cursors_and_local_conflicts_from_the_completed_bidirectional_sync() {
    let directory = tempdir();
    let path = directory.path().join("command-memory-sync-state.json");
    let config = MemoryConfig {
        schema_version: 1,
        local_port: 8006,
        home_host_alias: "memory-home".to_string(),
        home_user: "memory-sync".to_string(),
        pinned_host_fingerprint: "SHA256:fixture".to_string(),
        known_hosts_path: PathBuf::from("/tmp/known-hosts"),
        identity_file: PathBuf::from("/tmp/identity"),
        remote_loopback_port: 8006,
        local_node_id: "node:macbook-command".to_string(),
        home_node_id: "node:home-command".to_string(),
        sync_interval_minutes: 30,
        tool_allowlist: vec!["get_entity".to_string()],
        credential_keys: CredentialKeys {
            local_read: "memory.local.read".to_string(),
            local_attestation: "memory.local.attestation".to_string(),
            local_replicate: "memory.local.replicate".to_string(),
            remote_read: "memory.remote.read".to_string(),
            remote_replicate: "memory.remote.replicate".to_string(),
        },
    };
    let last_success = "2026-07-25T01:30:00Z";
    let response = MemorySyncResponse {
        status: "ok".to_string(),
        pull: Some(replication_result(
            "pull",
            &config.home_node_id,
            &config.local_node_id,
            73,
            2,
            "2026-07-25T01:29:59Z",
        )),
        push: Some(replication_result(
            "push",
            &config.local_node_id,
            &config.home_node_id,
            41,
            1,
            last_success,
        )),
        pinned_host: None,
        last_success: Some(last_success.to_string()),
        error: None,
    };

    persist_successful_response(&path, &config, &response).expect("persist successful response");
    let loaded = load_status(
        &path,
        Utc.with_ymd_and_hms(2026, 7, 25, 2, 0, 0)
            .single()
            .expect("time"),
    );
    assert_eq!(loaded.local_replication_cursor, Some(41));
    assert_eq!(loaded.home_replication_cursor, Some(73));
    assert_eq!(loaded.conflict_count, Some(2));
}
