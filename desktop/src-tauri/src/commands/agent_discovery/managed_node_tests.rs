use super::*;

#[test]
fn test_npm_eacces_hint_guidance_mentions_buzz_private_dir() {
    let hint = npm_eacces_hint("EACCES: permission denied", "npm install -g foo").unwrap();
    assert!(
        hint.contains("Buzz's private Node tools directory"),
        "hint: {hint}"
    );
}

#[test]
fn test_rewrite_npm_install_uses_private_prefix() {
    assert_eq!(
        rewrite_npm_global_install(
            "npm install -g @agentclientprotocol/codex-acp",
            "'/tmp/Buzz Node'"
        ),
        "npm install --global --prefix '/tmp/Buzz Node' @agentclientprotocol/codex-acp"
    );
}

#[test]
fn test_rewrite_npm_i_uses_private_prefix() {
    assert_eq!(
        rewrite_npm_global_install("npm i -g some-package", "'/tmp/buzz'"),
        "npm i --global --prefix '/tmp/buzz' some-package"
    );
}

#[test]
fn test_rewrite_npm_uninstall_uses_private_prefix() {
    assert_eq!(
        rewrite_npm_global_install("npm uninstall -g @zed-industries/codex-acp", "'/tmp/buzz'"),
        "npm uninstall --global --prefix '/tmp/buzz' @zed-industries/codex-acp"
    );
}

#[test]
fn test_rewrite_ignores_non_global_command() {
    assert_eq!(
        rewrite_npm_global_install("npm install foo", "'/tmp/buzz'"),
        "npm install foo"
    );
}

#[test]
fn test_shell_quote_escapes_single_quotes() {
    assert_eq!(
        shell_quote(std::path::Path::new("/tmp/Buzz's Node")),
        "'/tmp/Buzz'\\''s Node'"
    );
}

// ── zip validation tests ──────────────────────────────────────────────────────

/// Build an in-memory zip archive with the supplied entry names and return
/// a temporary file containing it (zip::ZipArchive requires Seek).
fn make_zip_with_entries(entry_names: &[&str]) -> tempfile::NamedTempFile {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default();
        for name in entry_names {
            writer.start_file(*name, opts).unwrap();
        }
        writer.finish().unwrap();
    }
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut tmp, &buf).unwrap();
    tmp
}

#[test]
fn test_validate_zip_accepts_normal_entries() {
    let tmp = make_zip_with_entries(&[
        "node-v24.18.0-win-x64/node.exe",
        "node-v24.18.0-win-x64/npm.cmd",
        "node-v24.18.0-win-x64/npm",
    ]);
    let file = std::fs::File::open(tmp.path()).unwrap();
    let archive = zip::ZipArchive::new(file).unwrap();
    assert!(validate_managed_node_zip_entries(&archive).is_ok());
}

#[test]
fn test_validate_zip_rejects_absolute_path() {
    let tmp = make_zip_with_entries(&["/etc/passwd"]);
    let file = std::fs::File::open(tmp.path()).unwrap();
    let archive = zip::ZipArchive::new(file).unwrap();
    let err = validate_managed_node_zip_entries(&archive).unwrap_err();
    assert!(
        err.contains("absolute path"),
        "expected 'absolute path' in: {err}"
    );
}

#[test]
fn test_validate_zip_rejects_path_traversal() {
    let tmp = make_zip_with_entries(&["../../../etc/passwd"]);
    let file = std::fs::File::open(tmp.path()).unwrap();
    let archive = zip::ZipArchive::new(file).unwrap();
    let err = validate_managed_node_zip_entries(&archive).unwrap_err();
    assert!(
        err.contains("path traversal"),
        "expected 'path traversal' in: {err}"
    );
}

#[test]
fn test_validate_zip_rejects_backslash_rooted() {
    // Windows-style absolute path using backslash — must reject on every host.
    let tmp = make_zip_with_entries(&["\\Windows\\system32\\evil.dll"]);
    let file = std::fs::File::open(tmp.path()).unwrap();
    let archive = zip::ZipArchive::new(file).unwrap();
    let err = validate_managed_node_zip_entries(&archive).unwrap_err();
    assert!(
        err.contains("absolute path"),
        "expected 'absolute path' in: {err}"
    );
}

#[test]
fn test_validate_zip_rejects_drive_prefix() {
    // Windows drive-letter absolute path — must reject on every host.
    let tmp = make_zip_with_entries(&["C:\\evil\\payload.exe"]);
    let file = std::fs::File::open(tmp.path()).unwrap();
    let archive = zip::ZipArchive::new(file).unwrap();
    let err = validate_managed_node_zip_entries(&archive).unwrap_err();
    assert!(
        err.contains("absolute path"),
        "expected 'absolute path' in: {err}"
    );
}

#[test]
fn test_validate_zip_rejects_backslash_traversal() {
    // Path traversal using Windows separator — must reject on every host.
    let tmp = make_zip_with_entries(&["node-v24.18.0-win-x64\\..\\..\\evil"]);
    let file = std::fs::File::open(tmp.path()).unwrap();
    let archive = zip::ZipArchive::new(file).unwrap();
    let err = validate_managed_node_zip_entries(&archive).unwrap_err();
    assert!(
        err.contains("path traversal"),
        "expected 'path traversal' in: {err}"
    );
}

// ── verify_node_tree layout tests ─────────────────────────────────────────────

#[test]
fn test_verify_node_tree_unix_layout_passes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("node"), b"").unwrap();
    std::fs::write(bin.join("npm"), b"").unwrap();
    // On non-Windows the unix branch is active — this must pass.
    #[cfg(not(windows))]
    assert!(verify_node_tree(tmp.path()).is_ok());
    // On Windows the windows branch is active — unix layout must fail.
    #[cfg(windows)]
    assert!(verify_node_tree(tmp.path()).is_err());
}

#[test]
fn test_verify_node_tree_unix_layout_missing_npm_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("node"), b"").unwrap();
    // npm intentionally absent
    #[cfg(not(windows))]
    {
        let err = verify_node_tree(tmp.path()).unwrap_err();
        assert!(err.contains("bin/npm"), "err: {err}");
    }
}

#[test]
fn test_verify_node_tree_windows_layout_passes() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("node.exe"), b"").unwrap();
    std::fs::write(tmp.path().join("npm.cmd"), b"").unwrap();
    std::fs::write(tmp.path().join("npm"), b"").unwrap();
    // On Windows the windows branch is active — this must pass.
    #[cfg(windows)]
    assert!(verify_node_tree(tmp.path()).is_ok());
    // On non-Windows the unix branch is active — windows-layout root files
    // don't satisfy bin/node + bin/npm, so this must fail.
    #[cfg(not(windows))]
    assert!(verify_node_tree(tmp.path()).is_err());
}

#[test]
fn test_verify_node_tree_windows_layout_missing_npm_shim_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("node.exe"), b"").unwrap();
    std::fs::write(tmp.path().join("npm.cmd"), b"").unwrap();
    // npm POSIX shim intentionally absent
    #[cfg(windows)]
    {
        let err = verify_node_tree(tmp.path()).unwrap_err();
        assert!(err.contains("npm"), "err: {err}");
    }
}

// ── should_invalidate_adapter / orphan policy pure unit tests ─────────────────

#[test]
fn test_should_invalidate_adapter_invalidates_managed_shim_when_orphaned() {
    let prefix = std::path::Path::new("/managed/npm/bin");
    let shim = prefix.join("codex-acp");
    assert!(
        should_invalidate_adapter(&shim, prefix, true),
        "managed shim + orphaned runtime must be invalidated"
    );
}

#[test]
fn test_should_invalidate_adapter_keeps_external_adapter_when_orphaned() {
    let prefix = std::path::Path::new("/managed/npm/bin");
    let external = std::path::Path::new("/usr/local/bin/codex-acp");
    assert!(
        !should_invalidate_adapter(external, prefix, true),
        "external adapter must not be invalidated even when Node is orphaned"
    );
}

#[test]
fn test_should_invalidate_adapter_keeps_managed_shim_when_node_healthy() {
    let prefix = std::path::Path::new("/managed/npm/bin");
    let shim = prefix.join("codex-acp");
    assert!(
        !should_invalidate_adapter(&shim, prefix, false),
        "managed shim must not be invalidated when Node is healthy"
    );
}

#[test]
fn test_resolve_adapter_path_returns_none_when_binary_absent() {
    let commands: &[&str] = &["nonexistent-buzz-test-binary-xyz"];
    let adapter_install_commands: &[&str] = &["curl -fsSL https://example.com | bash"];
    assert!(
        resolve_adapter_path(commands, adapter_install_commands).is_none(),
        "must return None when the command is not on PATH"
    );
}

// ── probe_node seam regressions ───────────────────────────────────────────────
//
// All four scenarios drive probe_node() directly — the same
// tempfile/deadline/cleanup/status/version path used by managed_node_runtime_ready.
// Each test CAN fail if production:
//   - drops process_group(0)          → descendant-holds-stdout assertion (d) fails
//                                       (tempfile transport still returns promptly;
//                                        the sleep survives and kill($!,0) returns 0)
//   - skips group-kill on a path      → hung-binary test exceeds margin
//   - ignores exit_status.success()   → nonzero-exit test returns true
//   - skips version comparison        → wrong-version test returns true
//
// Script files are written into a TempDir (no open write fd at spawn time)
// to avoid ETXTBSY on Linux.

/// Scenario 1 — descendant holds stdout write-end, direct child exits immediately.
///
/// The script backgrounds a 60-second sleep (inheriting stdout), records both the
/// script's own PID (`$$`) and the sleep's PID (`$!`) to sidecar files, then exits
/// with the expected version string.  Four assertions:
///   (a) bounded return — would hang ~60 s if tempfile transport regressed to pipe;
///   (b) correct result;
///   (c) process group dead after return — catches skipped-cleanup-on-success: if
///       the group-kill is absent but `process_group(0)` is still present, a live
///       member in the group is detectable;
///   (d) descendant PID dead after return — catches dropped `process_group(0)`: if
///       the call is removed the sleep stays in the runner's group (not the probe's),
///       `kill(-pgid,0)` is vacuously ESRCH, but `kill(desc_pid,0)` returns 0 and
///       this assertion fails.  This is the canonical mutation for (c).
#[cfg(unix)]
#[test]
fn test_probe_node_descendant_holds_stdout_returns_promptly_and_kills_group() {
    use std::os::unix::fs::PermissionsExt;
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let script = tmp_dir.path().join("probe.sh");
    let pgid_file = tmp_dir.path().join("pgid");
    let desc_pid_file = tmp_dir.path().join("desc_pid");
    let pgid_file_path = pgid_file.to_str().unwrap().to_owned();
    let desc_pid_file_path = desc_pid_file.to_str().unwrap().to_owned();
    // Line 1 of script: record the script's own PID (= PGID after process_group(0)).
    // Line 2: background the sleep and record its PID.
    // Line 3: emit the expected version and exit so the direct child exits promptly.
    let script_content = format!(
        "#!/bin/sh\necho $$ > {pgid_file_path}\n/bin/sleep 60 &\necho $! > {desc_pid_file_path}\necho v24.18.0\nexit 0\n"
    );
    std::fs::write(&script, script_content.as_bytes()).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let probe_timeout = std::time::Duration::from_secs(3);
    let t = std::time::Instant::now();
    let result = probe_node(&script, "v24.18.0", probe_timeout);
    let elapsed = t.elapsed();

    // Give the group-kill a moment to propagate before checking liveness.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // (a) bounded return — tempfile transport must not hang on the descendant's
    //     retained pipe write-end.
    assert!(
        elapsed < probe_timeout + std::time::Duration::from_secs(2),
        "probe_node hung — likely descendant retained pipe write-end: elapsed {elapsed:?}"
    );
    // (b) correct result
    assert!(result, "probe_node must return true for matching version");

    // (c) process group dead — catches skipped-cleanup: if the group-kill on the
    //     success path is removed while process_group(0) is still present, the
    //     sleep remains in the probe's group and kill(-pgid,0) returns 0.
    let pgid_str = std::fs::read_to_string(&pgid_file)
        .expect("script must have written its PID to the pgid sidecar");
    let pgid: i32 = pgid_str
        .trim()
        .parse()
        .expect("pgid sidecar must contain a numeric PID");
    let group_alive = unsafe { libc::kill(-pgid, 0) } == 0;
    assert!(
        !group_alive,
        "process group {pgid} must be dead after probe_node"
    );

    // (d) descendant PID dead — catches dropped process_group(0): without that
    //     call the sleep is never in the probe's group, so kill(-pgid,0) is
    //     vacuously ESRCH while the sleep survives.  Asserting the descendant's
    //     own PID is dead proves the sleep was actually killed.
    let desc_pid_str = std::fs::read_to_string(&desc_pid_file)
        .expect("script must have written the sleep PID to the desc_pid sidecar");
    let desc_pid: libc::pid_t = desc_pid_str
        .trim()
        .parse()
        .expect("desc_pid sidecar must contain a numeric PID");
    // Pre-assert cleanup: if the descendant is somehow still alive, kill it so
    // a failing test does not leave a 60-second sleep in the runner's process group.
    let desc_alive = unsafe { libc::kill(desc_pid, 0) } == 0;
    if desc_alive {
        unsafe { libc::kill(desc_pid, libc::SIGKILL) };
    }
    assert!(
        !desc_alive,
        "descendant PID {desc_pid} must be dead after probe_node — \
         if process_group(0) is dropped, the sleep escapes into the runner's group \
         and is never killed by the group-kill"
    );
}

/// Scenario 2 — direct hang: probe_node must traverse the real try_wait
/// deadline and return false.  Does NOT call kill_probe_group directly.
///
/// This test FAILS if the deadline loop in probe_node is broken or if the
/// timeout/kill path is not exercised (e.g., missing group-kill exits the
/// loop early via a different mechanism).
#[cfg(unix)]
#[test]
fn test_probe_node_times_out_on_hung_binary() {
    use std::os::unix::fs::PermissionsExt;
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let script = tmp_dir.path().join("hung.sh");
    std::fs::write(&script, b"#!/bin/sh\n/bin/sleep 30\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let probe_timeout = std::time::Duration::from_secs(3);
    let t = std::time::Instant::now();
    let result = probe_node(&script, "v24.18.0", probe_timeout);
    let elapsed = t.elapsed();

    assert!(!result, "probe_node must return false for a hung binary");
    // Must have traversed the deadline (not returned early via a bug).
    assert!(
        elapsed >= probe_timeout,
        "probe_node returned before deadline: {elapsed:?} < {probe_timeout:?}"
    );
    // Must not hang past the deadline by more than the poll interval + margin.
    assert!(
        elapsed < probe_timeout + std::time::Duration::from_secs(3),
        "probe_node exceeded deadline by too much: {elapsed:?}"
    );
}

/// Scenario 3 — non-zero exit: probe_node must return false even when stdout
/// contains the expected version string.
///
/// This test FAILS if probe_node skips or inverts the exit_status.success() check.
#[cfg(unix)]
#[test]
fn test_probe_node_returns_false_on_nonzero_exit() {
    use std::os::unix::fs::PermissionsExt;
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let script = tmp_dir.path().join("fail.sh");
    // Prints the expected version string but exits non-zero.
    std::fs::write(&script, b"#!/bin/sh\necho v24.18.0\nexit 1\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let result = probe_node(&script, "v24.18.0", std::time::Duration::from_secs(3));
    assert!(
        !result,
        "probe_node must return false when the process exits non-zero"
    );
}

/// Scenario 4 — wrong version output: probe_node must return false when stdout
/// does not match expected_version.
///
/// This test FAILS if probe_node skips or incorrectly performs the version comparison.
#[cfg(unix)]
#[test]
fn test_probe_node_returns_false_on_wrong_version_output() {
    use std::os::unix::fs::PermissionsExt;
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let script = tmp_dir.path().join("wrongver.sh");
    std::fs::write(&script, b"#!/bin/sh\necho v99.0.0\nexit 0\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let result = probe_node(&script, "v24.18.0", std::time::Duration::from_secs(3));
    assert!(
        !result,
        "probe_node must return false when stdout version does not match expected"
    );
}

/// Windows-shaped seam: non-zero exit via .bat file.
///
/// Drives the same probe_node path on Windows (terminate_process / taskkill /T /F).
/// This test FAILS if probe_node ignores exit_status.success() on Windows.
#[cfg(windows)]
#[test]
fn test_probe_node_windows_returns_false_on_nonzero_exit() {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let bat = tmp_dir.path().join("fail.bat");
    // Prints the expected version but exits non-zero — must still fail.
    std::fs::write(&bat, b"@echo off\r\necho v24.18.0\r\nexit /b 1\r\n").unwrap();

    let result = probe_node(&bat, "v24.18.0", std::time::Duration::from_secs(3));
    assert!(
        !result,
        "probe_node must return false when the .bat exits non-zero (Windows)"
    );
}

/// Windows-shaped seam: wrong version output via .bat file.
///
/// This test FAILS if probe_node skips the version comparison on Windows.
#[cfg(windows)]
#[test]
fn test_probe_node_windows_returns_false_on_wrong_version_output() {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let bat = tmp_dir.path().join("wrongver.bat");
    std::fs::write(&bat, b"@echo off\r\necho v99.0.0\r\nexit /b 0\r\n").unwrap();

    let result = probe_node(&bat, "v24.18.0", std::time::Duration::from_secs(3));
    assert!(
        !result,
        "probe_node must return false when stdout version does not match (Windows)"
    );
}

/// Returns false when the node binary path does not exist (fast path, no spawn).
#[test]
fn test_managed_node_runtime_ready_returns_false_when_binary_absent() {
    let Some(node) = crate::managed_agents::buzz_managed_node_bin_path() else {
        assert!(
            !managed_node_runtime_ready(),
            "managed_node_runtime_ready must return false when no path resolves"
        );
        return;
    };
    if node.is_file() {
        return;
    }
    assert!(
        !managed_node_runtime_ready(),
        "managed_node_runtime_ready must return false when the binary file does not exist"
    );
}
