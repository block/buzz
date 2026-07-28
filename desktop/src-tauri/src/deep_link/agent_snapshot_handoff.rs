#[cfg(unix)]
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
#[cfg(unix)]
use std::path::{Path, PathBuf};

use url::Url;

#[cfg(unix)]
use super::validate_agent_snapshot_handoff_metadata;
use super::{
    parse_agent_snapshot_handoff_id, read_agent_snapshot_handoff_from_dir,
    PendingAgentSnapshotImport, PendingAgentSnapshotImports,
};

const HANDOFF_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn config_only_snapshot_bytes(runtime: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "format": "buzz-agent-snapshot",
        "version": 1,
        "definition": {
            "name": "Hermes helper",
            "runtime": runtime,
            "respondToAllowlist": []
        },
        "profile": { "displayName": "Hermes helper" },
        "memory": { "level": "none", "entries": [] }
    }))
    .unwrap()
}

#[test]
fn agent_snapshot_handoff_requires_one_canonical_lowercase_uuid() {
    let valid = Url::parse(&format!(
        "buzz://import-agent-snapshot?handoff={HANDOFF_ID}"
    ))
    .unwrap();
    assert_eq!(
        parse_agent_snapshot_handoff_id(&valid).as_deref(),
        Some(HANDOFF_ID)
    );

    for invalid in [
        "buzz://import-agent-snapshot",
        "buzz://import-agent-snapshot?handoff=550E8400-E29B-41D4-A716-446655440000",
        "buzz://import-agent-snapshot?handoff=550e8400e29b41d4a716446655440000",
        "buzz://import-agent-snapshot?handoff=../550e8400-e29b-41d4-a716-446655440000",
        "buzz://import-agent-snapshot?handoff=550e8400-e29b-41d4-a716-446655440000&extra=1",
        "buzz://import-agent-snapshot?handoff=550e8400-e29b-41d4-a716-446655440000&handoff=550e8400-e29b-41d4-a716-446655440000",
    ] {
        assert!(
            parse_agent_snapshot_handoff_id(&Url::parse(invalid).unwrap()).is_none(),
            "accepted invalid handoff URL: {invalid}"
        );
    }
}

#[cfg(unix)]
fn stage_handoff(dir: &Path, bytes: &[u8], mode: u32) -> PathBuf {
    let path = dir.join(format!("{HANDOFF_ID}.agent.json"));
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn secure_handoff_read_accepts_valid_snapshot_and_deletes_staged_file() {
    let dir = tempfile::tempdir().unwrap();
    let expected = config_only_snapshot_bytes("hermes");
    let path = stage_handoff(dir.path(), &expected, 0o600);

    let bytes = read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).unwrap();

    assert_eq!(bytes, expected);
    assert!(!path.exists(), "accepted staged file must be deleted");
}

#[cfg(unix)]
#[test]
fn secure_handoff_read_rejects_symlink_and_leaves_target_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.json");
    std::fs::write(&target, config_only_snapshot_bytes("hermes")).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.path().join(format!("{HANDOFF_ID}.agent.json"));
    symlink(&target, &link).unwrap();

    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
    assert!(target.exists());
    assert!(link.symlink_metadata().is_ok());
}

#[cfg(unix)]
#[test]
fn secure_handoff_read_rejects_non_regular_permissive_oversize_and_invalid_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{HANDOFF_ID}.agent.json"));
    std::fs::create_dir(&path).unwrap();
    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
    std::fs::remove_dir(&path).unwrap();

    stage_handoff(dir.path(), &config_only_snapshot_bytes("hermes"), 0o644);
    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
    std::fs::remove_file(&path).unwrap();

    let file = std::fs::File::create(&path).unwrap();
    file.set_len((crate::MAX_SNAPSHOT_JSON_BYTES + 1) as u64)
        .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
    std::fs::remove_file(&path).unwrap();

    stage_handoff(dir.path(), br#"{"not":"a snapshot"}"#, 0o600);
    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
    assert!(
        path.exists(),
        "rejected staged file must remain for diagnosis"
    );
}

#[cfg(unix)]
#[test]
fn secure_handoff_metadata_rejects_wrong_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = stage_handoff(dir.path(), &config_only_snapshot_bytes("hermes"), 0o600);
    let metadata = path.metadata().unwrap();
    assert!(validate_agent_snapshot_handoff_metadata(&metadata, metadata.uid() + 1).is_err());
}

#[cfg(unix)]
#[test]
fn secure_handoff_read_rejects_snapshot_with_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&config_only_snapshot_bytes("hermes")).unwrap();
    value["memory"] = serde_json::json!({
        "level": "everything",
        "entries": [{ "slug": "secret", "body": "must not cross handoff" }]
    });
    let path = stage_handoff(dir.path(), &serde_json::to_vec(&value).unwrap(), 0o600);

    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
    assert!(path.exists());
}

#[test]
fn pending_agent_snapshot_import_is_peeked_then_acknowledged() {
    let queue = PendingAgentSnapshotImports::default();
    queue.enqueue(PendingAgentSnapshotImport {
        id: HANDOFF_ID.to_owned(),
        file_bytes: vec![1, 2, 3],
        file_name: format!("{HANDOFF_ID}.agent.json"),
        snapshot_kind: "agent".to_owned(),
    });

    assert_eq!(queue.first().unwrap().file_bytes, vec![1, 2, 3]);
    assert!(!queue.acknowledge("other"));
    assert!(queue.acknowledge(HANDOFF_ID));
    assert!(queue.first().is_none());
}
