use super::*;
use std::ffi::OsString;

fn launcher() -> Arc<PiLaunchOverride> {
    PiLaunchOverride::create(Path::new("/unused/buzz-acp")).unwrap()
}

fn restore_args(launcher: &PiLaunchOverride, id: Uuid) -> Vec<OsString> {
    let path = launcher.directory.join(format!("transcript-{id}.jsonl"));
    write_private_file(
        &path,
        format!("{{\"type\":\"session\",\"id\":\"{id}\"}}\n").as_bytes(),
        false,
    )
    .unwrap();
    vec![
        "--mode".into(),
        "rpc".into(),
        "--session".into(),
        path.into_os_string(),
    ]
}

#[test]
fn pi_snapshots_are_isolated_and_restore_the_original_after_another_create() {
    let launcher = launcher();
    let first = launcher
        .begin("<base>first</base>\n<core-memory>A</core-memory>")
        .unwrap();
    let first_path = native::select_prompt(&launcher.directory, &[]).unwrap();
    let id = Uuid::new_v4();
    let first = first.finish(&id.to_string()).unwrap();
    let args = restore_args(&launcher, id);
    let second = launcher
        .begin("<base>second</base>\n<core-memory>B</core-memory>")
        .unwrap();
    let second_path = native::select_prompt(&launcher.directory, &[]).unwrap();
    assert_ne!(first_path, second_path);
    assert_eq!(
        native::select_prompt(&launcher.directory, &args).unwrap(),
        first_path
    );
    assert!(fs::read_to_string(&first_path)
        .unwrap()
        .contains("<core-memory>A</core-memory>"));
    drop(second);
    assert!(!second_path.exists());
    assert!(first_path.exists()); // reload keeps an immutable, existing path
    drop(first);
    assert!(!first_path.exists());
    assert!(native::select_prompt(&launcher.directory, &args).is_err());
}

#[test]
fn pi_pool_workers_do_not_share_pending_prompts() {
    let a = launcher();
    let b = launcher();
    let _a = a.begin("worker A").unwrap();
    let _b = b.begin("worker B").unwrap();
    assert_eq!(
        fs::read_to_string(native::select_prompt(&a.directory, &[]).unwrap()).unwrap(),
        "worker A"
    );
    assert_eq!(
        fs::read_to_string(native::select_prompt(&b.directory, &[]).unwrap()).unwrap(),
        "worker B"
    );
}

#[test]
fn pi_aborted_and_invalid_session_creates_clean_up() {
    let launcher = launcher();
    let pending = launcher.begin("aborted").unwrap();
    let path = native::select_prompt(&launcher.directory, &[]).unwrap();
    drop(pending);
    assert!(!path.exists());
    assert!(!launcher.directory.join("pending").exists());
    assert!(launcher
        .begin("invalid")
        .unwrap()
        .finish("../../outside")
        .is_err());
    assert!(!launcher.directory.join("pending").exists());
    assert!(
        !fs::read_dir(&launcher.directory).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "md"))
    );
}

#[test]
fn pi_restore_never_falls_back_to_another_pending_prompt() {
    let launcher = launcher();
    let args = restore_args(&launcher, Uuid::new_v4());
    let _pending = launcher.begin("unrelated session").unwrap();
    assert!(native::select_prompt(&launcher.directory, &args).is_err());
    assert!(native::select_prompt(&launcher.directory, &["--session".into()]).is_err());
}

#[test]
fn pi_missing_snapshot_is_an_error_not_literal_path_text() {
    let launcher = launcher();
    let _pending = launcher.begin("instructions").unwrap();
    fs::remove_file(native::select_prompt(&launcher.directory, &[]).unwrap()).unwrap();
    assert!(native::select_prompt(&launcher.directory, &[]).is_err());
}

#[cfg(unix)]
#[test]
fn pi_launcher_quotes_paths_and_prompt_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let script = launcher_script(
        Path::new("/path with 'quotes'/buzz-acp"),
        Path::new("/private directory"),
    )
    .unwrap();
    assert!(script.contains("'\"'\"'"));
    assert!(script.ends_with(" \"$@\"\n"));
    let launcher = launcher();
    let _pending = launcher.begin("secret memory").unwrap();
    let path = native::select_prompt(&launcher.directory, &[]).unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&launcher.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}
