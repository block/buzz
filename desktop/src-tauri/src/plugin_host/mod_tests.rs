//! Regression tests for the `pending_starts` lifecycle invariants: a
//! `start_session` still mid-discovery (spawned, but not yet registered in
//! `sessions`) must never survive a concurrent `set_enabled(false)`,
//! `uninstall`, or `shutdown()` - including a same-package reinstall racing
//! the same window. These bind the real `PluginHost` API, not a synthetic
//! test-only seam: each test drives an actual subprocess through an actual
//! `start_session` call, paused mid-discovery via a FIFO, and asserts on the
//! host's real internal state after the race.

use super::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::unix::AsyncFd;

const PLUGIN_ID: &str = "dev.example.pending-starts-race";
const CONTRIBUTION_ID: &str = "web";

fn test_config(root: &Path) -> PluginHostConfig {
    PluginHostConfig {
        root: root.to_path_buf(),
        resolve_deadline: Duration::from_millis(500),
        discover_deadline: Duration::from_secs(5),
    }
}

/// A package whose executable blocks inside `server/discover` until released
/// through a FIFO, so a test can deterministically pause a `start_session`
/// call in the exact "spawned, pending, not yet registered" window the
/// lifecycle races described in review land on.
struct PausingFixture {
    package: TempDir,
    #[allow(dead_code)]
    controls: TempDir,
    ready_fifo: PathBuf,
}

impl PausingFixture {
    fn new() -> Self {
        let package = tempfile::tempdir().expect("create fixture package directory");
        let controls = tempfile::tempdir().expect("create fixture control directory");
        let ready_fifo = controls.path().join("ready.fifo");
        let release_fifo = controls.path().join("release.fifo");
        create_fifo(&ready_fifo);
        create_fifo(&release_fifo);

        let binary_directory = package.path().join("bin");
        fs::create_dir(&binary_directory).expect("create fixture binary directory");
        let triple = env!("BUZZ_PLUGIN_TARGET_TRIPLE").to_string();
        let executable_path = binary_directory.join(&triple);
        fs::write(&executable_path, PAUSING_SCRIPT).expect("write fixture executable");
        let mut permissions = fs::metadata(&executable_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable_path, permissions).unwrap();

        fs::write(
            package.path().join("fixture-controls"),
            format!("{}\n{}\n", ready_fifo.display(), release_fifo.display()),
        )
        .expect("write fixture controls");

        let executable_bytes = fs::read(&executable_path).expect("read fixture executable");
        let sha256 = hex::encode(Sha256::digest(&executable_bytes));
        let manifest = serde_json::json!({
            "packageFormatVersion": "0.1.0-alpha",
            "contractVersion": "0.1.0-alpha",
            "id": PLUGIN_ID,
            "name": "Pending Starts Race Fixture",
            "version": "0.1.0",
            "publisher": "Buzz Tests",
            "license": "Apache-2.0",
            "runtime": {
                "type": "stdio",
                "targets": {
                    triple.clone(): {
                        "path": format!("bin/{triple}"),
                        "sha256": sha256,
                        "bytes": executable_bytes.len(),
                    }
                }
            },
            "grants": ["browser.browse"],
            "contributions": [{ "kind": "browser", "id": CONTRIBUTION_ID, "title": "Web" }],
        });
        fs::write(
            package.path().join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .expect("write fixture manifest");

        Self {
            package,
            controls,
            ready_fifo,
        }
    }
}

const PAUSING_SCRIPT: &[u8] = br#"#!/bin/sh
set -eu
SCRIPT_DIRECTORY=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CONTROLS_FILE="$SCRIPT_DIRECTORY/fixture-controls"
READY_FIFO=$(sed -n '1p' "$CONTROLS_FILE")
RELEASE_FIFO=$(sed -n '2p' "$CONTROLS_FILE")

while IFS= read -r request; do
  request_id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*)
      printf x > "$READY_FIFO"
      IFS= read -r _release < "$RELEASE_FIFO" || true
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{"listChanged":false}},"cacheScope":"private","ttlMs":0}}\n' "$request_id"
      ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","cacheScope":"private","ttlMs":0,"tools":[{"name":"browser.resolve","title":"Resolve","description":"d","inputSchema":{"type":"object"}}]}}\n' "$request_id"
      ;;
    *)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","content":[{"type":"text","text":"https://example.com/"}],"structuredContent":{"outcome":"resolved","url":"https://example.com/"}}}\n' "$request_id"
      ;;
  esac
done
"#;

fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run POSIX mkfifo");
    assert!(
        status.success(),
        "create fixture FIFO at {}",
        path.display()
    );
}

async fn receive_fifo_signal(path: PathBuf) -> io::Result<()> {
    let fifo = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let fifo = AsyncFd::new(fifo)?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut readiness = fifo.readable().await?;
            match readiness.try_io(|inner| {
                let mut file = inner.get_ref();
                let mut signal = [0_u8; 1];
                match file.read(&mut signal) {
                    Ok(1) => Ok(()),
                    Ok(_) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
                    Err(error) => Err(error),
                }
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "fixture ready FIFO timed out"))?
}

/// Waits until `host`'s `pending_starts` no longer contains an entry for
/// `plugin_id`. Used to bound the race between a synchronous lifecycle call
/// returning and its process-group `SIGTERM`/`SIGKILL` actually landing.
async fn wait_until_no_pending(host: &PluginHost, plugin_id: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let has_pending = host
                .pending_starts
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .entries
                .values()
                .any(|entry| entry.plugin_id == plugin_id);
            if !has_pending {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("pending start cleared within deadline");
}

#[tokio::test]
async fn disable_during_pending_discovery_prevents_registration() {
    let fixture = PausingFixture::new();
    let host_root = tempfile::tempdir().unwrap();
    let host = Arc::new(PluginHost::new(test_config(host_root.path())));
    host.install(fixture.package.path())
        .expect("install fixture");

    let start_host = Arc::clone(&host);
    let start_call =
        tokio::spawn(async move { start_host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await });

    receive_fifo_signal(fixture.ready_fifo.clone())
        .await
        .expect("fixture reports discover is pending");

    let disable_result = host.set_enabled(PLUGIN_ID, false);
    assert!(
        disable_result.is_ok(),
        "disable during pending discovery: {disable_result:?}"
    );

    let start_result = start_call.await.expect("join start_session");
    assert!(
        start_result.is_err(),
        "a start_session killed mid-discovery never succeeds: {start_result:?}"
    );

    wait_until_no_pending(&host, PLUGIN_ID).await;
    assert!(
        host.sessions.lock().unwrap().is_empty(),
        "no session is ever registered for a start_session invalidated mid-discovery"
    );

    let listed = host.list().expect("list after disable");
    assert_eq!(listed.len(), 1, "disable retains the registry record");
    assert!(!listed[0].enabled, "disable marks the record disabled");
}

#[tokio::test]
async fn shutdown_during_pending_discovery_prevents_registration() {
    let fixture = PausingFixture::new();
    let host_root = tempfile::tempdir().unwrap();
    let host = Arc::new(PluginHost::new(test_config(host_root.path())));
    host.install(fixture.package.path())
        .expect("install fixture");

    let start_host = Arc::clone(&host);
    let start_call =
        tokio::spawn(async move { start_host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await });

    receive_fifo_signal(fixture.ready_fifo.clone())
        .await
        .expect("fixture reports discover is pending");

    let shutdown_result = host.shutdown();
    assert!(
        shutdown_result.is_ok(),
        "shutdown during pending discovery: {shutdown_result:?}"
    );

    let start_result = start_call.await.expect("join start_session");
    assert!(
        start_result.is_err(),
        "a start_session killed by shutdown mid-discovery never succeeds: {start_result:?}"
    );

    assert!(
        host.sessions.lock().unwrap().is_empty(),
        "shutdown leaves no session registered, including one that was mid-discovery"
    );
    assert!(
        host.pending_starts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entries
            .is_empty(),
        "shutdown drains pending_starts synchronously before returning"
    );
}

#[tokio::test]
async fn uninstall_then_identical_reinstall_does_not_resurrect_the_stale_session() {
    let fixture = PausingFixture::new();
    let host_root = tempfile::tempdir().unwrap();
    let host = Arc::new(PluginHost::new(test_config(host_root.path())));
    host.install(fixture.package.path())
        .expect("install fixture");

    let start_host = Arc::clone(&host);
    let start_call =
        tokio::spawn(async move { start_host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await });

    receive_fifo_signal(fixture.ready_fifo.clone())
        .await
        .expect("fixture reports discover is pending");

    let uninstall_result = host.uninstall(PLUGIN_ID);
    assert!(
        uninstall_result.is_ok(),
        "uninstall during pending discovery: {uninstall_result:?}"
    );

    // Byte-identical reinstall: same manifest/executable bytes, so the same
    // digest-derived `package_path` as the install the stale pending client
    // was spawned from - the exact condition under which a plain
    // `package_path` comparison alone could previously mistake the killed
    // process's late success for this new install's session.
    host.install(fixture.package.path())
        .expect("byte-identical reinstall");

    let start_result = start_call.await.expect("join start_session");
    assert!(
        start_result.is_err(),
        "the pre-uninstall start_session must never resurrect as the reinstall's session: {start_result:?}"
    );

    wait_until_no_pending(&host, PLUGIN_ID).await;
    assert!(
        host.sessions.lock().unwrap().is_empty(),
        "the reinstall starts with no adopted session"
    );

    let listed = host.list().expect("list after reinstall");
    assert_eq!(listed.len(), 1, "exactly the reinstalled record remains");
    assert!(listed[0].enabled, "the reinstall is enabled");
}
