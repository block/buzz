use std::os::unix::fs::PermissionsExt as _;

#[test]
fn agent_logs_are_created_owner_only_and_tighten_legacy_permissions() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("agent.log");

    drop(super::super::open_log_file(&path).expect("create log"));
    assert_eq!(
        std::fs::metadata(&path)
            .expect("created log metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("set legacy permissions");
    drop(super::super::open_log_file(&path).expect("reopen legacy log"));
    assert_eq!(
        std::fs::metadata(&path)
            .expect("reopened log metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
