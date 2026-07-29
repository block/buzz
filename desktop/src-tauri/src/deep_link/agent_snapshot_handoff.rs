#[cfg(target_os = "macos")]
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::time::{Duration, SystemTime};

use url::Url;

use super::{
    agent_snapshot_handoff_url_exceeds_limit, consume_agent_snapshot_handoff_from_dir,
    parse_agent_snapshot_handoff_id, read_agent_snapshot_handoff_from_dir,
    PendingAgentSnapshotImport, PendingAgentSnapshotImports,
};
#[cfg(target_os = "macos")]
use super::{
    validate_agent_snapshot_handoff_directory_metadata, validate_agent_snapshot_handoff_metadata,
    AGENT_SNAPSHOT_HANDOFF_MAX_AGE,
};

const HANDOFF_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn config_only_snapshot_bytes(runtime: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "format": "buzz-agent-snapshot",
        "version": 1,
        "definition": {
            "name": "Hermes helper",
            "systemPrompt": "Be helpful.",
            "runtime": runtime
        },
        "profile": { "displayName": "Hermes helper" },
        "memory": { "level": "none" }
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

#[test]
fn normalized_whitespace_does_not_bypass_handoff_url_limit() {
    let raw = format!(
        "{}buzz://import-agent-snapshot?handoff={HANDOFF_ID}",
        " ".repeat(300)
    );
    let parsed = Url::parse(&raw).unwrap();
    assert_eq!(parsed.host_str(), Some("import-agent-snapshot"));
    assert!(agent_snapshot_handoff_url_exceeds_limit(&parsed, &raw));
}

#[cfg(target_os = "macos")]
fn stage_handoff(dir: &Path, bytes: &[u8], mode: u32) -> PathBuf {
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.join(format!("{HANDOFF_ID}.agent.json"));
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[cfg(target_os = "macos")]
#[test]
fn secure_handoff_read_retains_source_until_preview_acknowledgement() {
    let dir = tempfile::tempdir().unwrap();
    let expected = config_only_snapshot_bytes("hermes");
    let path = stage_handoff(dir.path(), &expected, 0o600);

    let bytes = read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).unwrap();

    assert_eq!(bytes, expected);
    assert!(path.exists(), "source must remain until preview settles");
    consume_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID, &bytes).unwrap();
    assert!(!path.exists(), "acknowledged source must be deleted");
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
#[test]
fn secure_handoff_read_rejects_symlinked_staging_directory() {
    let real_dir = tempfile::tempdir().unwrap();
    let path = stage_handoff(
        real_dir.path(),
        &config_only_snapshot_bytes("hermes"),
        0o600,
    );
    let parent = tempfile::tempdir().unwrap();
    let linked_dir = parent.path().join("buzz-handoffs");
    symlink(real_dir.path(), &linked_dir).unwrap();

    assert!(read_agent_snapshot_handoff_from_dir(&linked_dir, HANDOFF_ID).is_err());
    assert!(path.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn secure_handoff_read_rejects_hard_linked_payload() {
    let dir = tempfile::tempdir().unwrap();
    let path = stage_handoff(dir.path(), &config_only_snapshot_bytes("hermes"), 0o600);
    let second_link = dir.path().join("second-link.agent.json");
    std::fs::hard_link(&path, &second_link).unwrap();

    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
    assert!(path.exists());
    assert!(second_link.exists());
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
#[test]
fn secure_handoff_read_rejects_fields_outside_the_coagent_contract() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{HANDOFF_ID}.agent.json"));
    for (section, field, value) in [
        ("definition", "respondTo", serde_json::json!("anyone")),
        ("definition", "respondToAllowlist", serde_json::json!([])),
        ("definition", "model", serde_json::json!("remote-model")),
        (
            "profile",
            "avatarUrl",
            serde_json::json!("https://example.test/a.png"),
        ),
        ("memory", "entries", serde_json::json!([])),
    ] {
        let mut snapshot: serde_json::Value =
            serde_json::from_slice(&config_only_snapshot_bytes("hermes")).unwrap();
        snapshot[section][field] = value;
        stage_handoff(dir.path(), &serde_json::to_vec(&snapshot).unwrap(), 0o600);
        assert!(
            read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err(),
            "accepted forbidden {section}.{field}"
        );
        std::fs::remove_file(&path).unwrap();
    }

    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&config_only_snapshot_bytes("hermes")).unwrap();
    snapshot["credentialRef"] = serde_json::json!("opaque-but-forbidden");
    stage_handoff(dir.path(), &serde_json::to_vec(&snapshot).unwrap(), 0o600);
    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn secure_handoff_read_requires_the_hermes_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let path = stage_handoff(dir.path(), &config_only_snapshot_bytes("buzz-agent"), 0o600);
    assert!(read_agent_snapshot_handoff_from_dir(dir.path(), HANDOFF_ID).is_err());
    assert!(
        path.exists(),
        "rejected staged file must remain for diagnosis"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn secure_handoff_metadata_rejects_wrong_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = stage_handoff(dir.path(), &config_only_snapshot_bytes("hermes"), 0o600);
    let metadata = path.metadata().unwrap();
    assert!(validate_agent_snapshot_handoff_metadata(
        &metadata,
        metadata.uid() + 1,
        SystemTime::now()
    )
    .is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn secure_handoff_metadata_rejects_expired_and_far_future_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = stage_handoff(dir.path(), &config_only_snapshot_bytes("hermes"), 0o600);
    let metadata = path.metadata().unwrap();
    let modified = metadata.modified().unwrap();

    assert!(validate_agent_snapshot_handoff_metadata(
        &metadata,
        metadata.uid(),
        modified + AGENT_SNAPSHOT_HANDOFF_MAX_AGE + Duration::from_secs(1)
    )
    .is_err());
    assert!(validate_agent_snapshot_handoff_metadata(
        &metadata,
        metadata.uid(),
        modified - Duration::from_secs(61)
    )
    .is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn secure_handoff_directory_requires_owner_only_permissions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let metadata = dir.path().metadata().unwrap();
    assert!(validate_agent_snapshot_handoff_directory_metadata(&metadata, metadata.uid()).is_ok());

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let permissive = dir.path().metadata().unwrap();
    assert!(
        validate_agent_snapshot_handoff_directory_metadata(&permissive, permissive.uid()).is_err()
    );
    assert!(
        validate_agent_snapshot_handoff_directory_metadata(&permissive, permissive.uid() + 1)
            .is_err()
    );
}

#[cfg(target_os = "macos")]
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
    assert!(queue.enqueue(PendingAgentSnapshotImport {
        id: HANDOFF_ID.to_owned(),
        file_bytes: vec![1, 2, 3],
        file_name: format!("{HANDOFF_ID}.agent.json"),
        snapshot_kind: "agent".to_owned(),
    }));

    assert!(!queue.enqueue(PendingAgentSnapshotImport {
        id: "550e8400-e29b-41d4-a716-446655440001".to_owned(),
        file_bytes: vec![4, 5, 6],
        file_name: "second.agent.json".to_owned(),
        snapshot_kind: "agent".to_owned(),
    }));

    assert_eq!(queue.first().unwrap().file_bytes, vec![1, 2, 3]);
    assert!(!queue.acknowledge("other"));
    assert!(queue.acknowledge(HANDOFF_ID));
    assert!(queue.first().is_none());
}
