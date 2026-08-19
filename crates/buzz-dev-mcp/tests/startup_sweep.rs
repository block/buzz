//! Production-boundary coverage for the temp-dir orphan sweep (#6025).
//!
//! The unit tests in `sweep.rs` prove the sweep's policy. They cannot prove
//! that the policy is *wired to anything*: delete the sweep call from `main`,
//! or the claim from `Shim::install`, and they all still pass. So this one
//! drives the real `buzz-dev-mcp` binary and asserts only on what lands in
//! the temp root, which means it fails if any of the three production
//! integration points is removed, and it fails if the on-disk names change
//! under a running session.
//!
//! Both phases run against one temp root, handed to the binary through the
//! platform's temp-dir environment variables, so nothing here touches the
//! real system temp directory.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// On-disk contract, deliberately duplicated rather than imported: these are
/// crate-private in `sweep`, and a test that reads them from the code under
/// test cannot notice the code under test renaming them.
const OWNER_PREFIX: &str = "buzz-dev-mcp-";
const SESSION_PREFIX: &str = "buzz-dev-mcp-session-";
const MARKER_FILE_NAME: &str = ".buzz-dev-mcp-owner";
const LEASE_FILE_NAME: &str = ".buzz-dev-mcp-lease";

/// Generous: on Windows the shim directory is five copies of a debug binary
/// north of 100 MB, so the session directory that follows it can be seconds
/// away on a loaded CI runner.
const DEADLINE: Duration = Duration::from_secs(90);

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < DEADLINE {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out after {DEADLINE:?} waiting for {what}");
}

/// Spawn and immediately reap a trivial child so its pid is known dead.
fn dead_pid() -> u32 {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "exit", "0"]);
        c
    } else {
        Command::new("true")
    };
    let mut child = cmd.spawn().expect("spawn short-lived helper");
    let pid = child.id();
    child.wait().expect("reap helper");
    pid
}

/// A directory shaped exactly like one this crate leaves behind: an unheld
/// lease file plus an owner marker naming `pid`.
fn plant_dir(root: &Path, name: &str, pid: u32) -> PathBuf {
    let dir = root.join(format!("{OWNER_PREFIX}{name}"));
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join(LEASE_FILE_NAME), b"").expect("write lease");
    std::fs::write(
        dir.join(MARKER_FILE_NAME),
        format!("pid={pid}\ncreated=1\n"),
    )
    .expect("write marker");
    dir
}

fn start_server(root: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_buzz-dev-mcp"))
        // TMPDIR is Unix, TMP/TEMP are Windows. Set all three so the binary's
        // `std::env::temp_dir()` lands in our scratch root on every platform.
        .env("TMPDIR", root)
        .env("TMP", root)
        .env("TEMP", root)
        // Held open for the life of the test: the server serves MCP over
        // stdio, so an open stdin is what keeps it running.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn buzz-dev-mcp")
}

fn owner_pid_of(dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(dir.join(MARKER_FILE_NAME)).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("pid="))
        .and_then(|v| v.trim().parse().ok())
}

/// Directories in `root` that this crate created and claimed, split by kind.
fn claimed_dirs(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut shim = Vec::new();
    let mut session = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return (shim, session);
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(OWNER_PREFIX) || !entry.path().is_dir() {
            continue;
        }
        if !entry.path().join(MARKER_FILE_NAME).exists() {
            continue;
        }
        if name.starts_with(SESSION_PREFIX) {
            session.push(entry.path());
        } else {
            shim.push(entry.path());
        }
    }
    (shim, session)
}

#[test]
fn a_real_server_sweeps_orphans_claims_its_own_dirs_and_is_reclaimed_after_a_hard_kill() {
    let root = tempfile::tempdir().expect("tempdir");

    // An orphan exactly like a hard-killed session leaves: dead owner, lease
    // released by the kernel when that process died.
    let orphan = plant_dir(root.path(), "orphan-under-test", dead_pid());
    // And a directory whose owner is very much alive (this test process). The
    // sweep must not touch it, which is what separates "reclaims orphans"
    // from "empties the temp directory".
    let live = plant_dir(root.path(), "live-under-test", std::process::id());

    let mut server = start_server(root.path());
    let server_pid = server.id();

    wait_until("the startup sweep to reclaim the planted orphan", || {
        !orphan.exists()
    });
    assert!(
        live.exists(),
        "a directory owned by a live process must survive the startup sweep"
    );

    // Both directories the server creates must be claimed: marker naming the
    // server, and a lease file beside it. This is what fails if either
    // `Shim::install` or `SharedState::new` stops claiming.
    wait_until("the server to create and claim both temp dirs", || {
        let (shim, session) = claimed_dirs(root.path());
        shim.iter().any(|d| owner_pid_of(d) == Some(server_pid))
            && session.iter().any(|d| owner_pid_of(d) == Some(server_pid))
    });

    let (shim, session) = claimed_dirs(root.path());
    let own: Vec<PathBuf> = shim
        .into_iter()
        .chain(session)
        .filter(|d| owner_pid_of(d) == Some(server_pid))
        .collect();
    assert_eq!(
        own.len(),
        2,
        "expected a shim dir and a session dir: {own:?}"
    );
    for dir in &own {
        assert!(
            dir.join(LEASE_FILE_NAME).exists(),
            "claimed dir {} has no lease file",
            dir.display()
        );
    }

    // Phase two, the actual bug: kill the server without giving it any chance
    // to clean up, then start a replacement. The kernel released the dead
    // server's lease when it died, so the replacement must reclaim both
    // directories.
    server.kill().expect("kill server");
    server.wait().expect("reap server");

    let mut replacement = start_server(root.path());
    wait_until(
        "the replacement to reclaim the killed server's dirs",
        || own.iter().all(|d| !d.exists()),
    );
    assert!(
        live.exists(),
        "the live-owner directory must survive the replacement's sweep too"
    );

    replacement.kill().expect("kill replacement");
    replacement.wait().expect("reap replacement");
}
