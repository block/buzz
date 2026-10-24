#[cfg(unix)]
#[test]
fn packaged_sidecar_wins_while_development_keeps_workspace_priority() {
    use std::os::unix::fs::PermissionsExt;
    let bundle = tempfile::tempdir().expect("bundle fixture");
    let sidecar = bundle.path().join("buzz-acp");
    std::fs::write(&sidecar, "packaged sidecar").expect("sidecar fixture");
    std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o755))
        .expect("executable fixture");
    let release_dirs = super::command_search_dirs_for(false, Some(bundle.path().into()));
    let resolved = release_dirs
        .iter()
        .map(|dir| dir.join("buzz-acp"))
        .find(|path| super::is_executable_file(path));
    assert_eq!(resolved, Some(sidecar));
    let dev_dirs = super::command_search_dirs_for(true, Some(bundle.path().into()));
    assert_eq!(dev_dirs.last(), Some(&bundle.path().to_path_buf()));
    assert_ne!(dev_dirs.first(), release_dirs.first());
}
