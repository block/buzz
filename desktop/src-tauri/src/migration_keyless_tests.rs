use super::copy_app_data;

#[test]
fn remote_legacy_copy_excludes_all_root_user_identity_material_but_keeps_agent_keys() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("legacy");
    let remote = root.path().join("remote");
    let local = root.path().join("local");
    std::fs::create_dir_all(src.join("agents/agent-a")).unwrap();
    let human_files = [
        "identity.key",
        "identity.key.bad.123",
        crate::key_backup::BACKUP_FILE_NAME,
        "identity.migrated",
        "identity.buzz-desktop-dev.example.migrated",
    ];
    for name in human_files {
        std::fs::write(src.join(name), "not-to-be-read-or-copied").unwrap();
    }
    std::fs::write(src.join("settings.json"), "settings").unwrap();
    std::fs::write(
        src.join("agents/agent-a/identity.key"),
        "independent-agent-key",
    )
    .unwrap();
    copy_app_data(&src, &remote, true).unwrap();
    for name in human_files {
        assert!(!remote.join(name).exists(), "{name}");
    }
    assert_eq!(
        std::fs::read_to_string(remote.join("settings.json")).unwrap(),
        "settings"
    );
    assert_eq!(
        std::fs::read_to_string(remote.join("agents/agent-a/identity.key")).unwrap(),
        "independent-agent-key"
    );
    copy_app_data(&src, &local, false).unwrap();
    for name in human_files {
        assert_eq!(
            std::fs::read(local.join(name)).unwrap(),
            std::fs::read(src.join(name)).unwrap()
        );
    }
    // Repeat does not remove or overwrite existing local data in either mode.
    std::fs::write(remote.join("identity.key"), "pre-existing-local-key").unwrap();
    copy_app_data(&src, &remote, true).unwrap();
    assert_eq!(
        std::fs::read_to_string(remote.join("identity.key")).unwrap(),
        "pre-existing-local-key"
    );
}

#[cfg(unix)]
#[test]
fn remote_legacy_copy_skips_user_key_symlink_before_inspecting_target() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("legacy");
    let dst = root.path().join("remote");
    std::fs::create_dir(&src).unwrap();
    std::os::unix::fs::symlink("/nonexistent-user-key", src.join("identity.key")).unwrap();
    copy_app_data(&src, &dst, true).unwrap();
    assert!(!dst.join("identity.key").is_symlink());
}

#[test]
#[ignore = "requires an actual remote native build"]
fn compiled_remote_migration_does_not_copy_user_identity() {
    assert!(crate::native_identity::SignerMode::compiled().is_remote());
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("legacy");
    let dst = root.path().join("remote");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("identity.key"), "must-not-copy").unwrap();
    super::copy_legacy_app_data(&src, &dst).unwrap();
    assert!(!dst.join("identity.key").exists());
}
