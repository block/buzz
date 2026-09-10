//! Process ownership through failed and cancelled lifecycle operations.

use super::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write as _};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::unix::AsyncFd;

const PLUGIN_ID: &str = "dev.example.pending-starts-race";
const CONTRIBUTION_ID: &str = "web";

fn test_config(root: &Path) -> PluginHostConfig {
    PluginHostConfig {
        root: root.to_path_buf(),
        resolve_deadline: Duration::from_secs(30),
        discover_deadline: Duration::from_secs(30),
    }
}

/// A package whose executable blocks inside `server/discover` until released
/// through a FIFO, so a test can deterministically pause a `start_session`
/// call in the exact "spawned, pending, not yet registered" window the
/// lifecycle races described in review land on.
struct PausingFixture {
    package: TempDir,
    controls: TempDir,
    ready_fifo: PathBuf,
    release_file: fs::File,
}

impl PausingFixture {
    fn new() -> Self {
        Self::with_script(PAUSING_SCRIPT)
    }

    fn with_script(script: &[u8]) -> Self {
        let package = tempfile::tempdir().expect("create fixture package directory");
        let controls = tempfile::tempdir().expect("create fixture control directory");
        let ready_fifo = controls.path().join("ready.fifo");
        let release_fifo = controls.path().join("release.fifo");
        create_fifo(&ready_fifo);
        create_fifo(&release_fifo);
        let release_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&release_fifo)
            .unwrap();

        let binary_directory = package.path().join("bin");
        fs::create_dir(&binary_directory).expect("create fixture binary directory");
        let triple = env!("BUZZ_PLUGIN_TARGET_TRIPLE").to_string();
        let executable_path = binary_directory.join(&triple);
        fs::write(&executable_path, script).expect("write fixture executable");
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
            release_file,
        }
    }
}

const PAUSING_SCRIPT: &[u8] = br#"#!/bin/sh
set -eu
trap '' TERM
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
      printf x > "$RELEASE_FIFO.home"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","content":[{"type":"text","text":"https://example.com/"}],"structuredContent":{"outcome":"resolved","url":"https://example.com/"}}}\n' "$request_id"
      ;;
  esac
done
IFS= read -r _finish < "$RELEASE_FIFO" || true
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

fn release_fixture(fixture: &PausingFixture) {
    (&fixture.release_file).write_all(b"continue\n").unwrap();
}

async fn start_fixture(host: &PluginHost, fixture: &PausingFixture) -> Session {
    let start = host.start_session(PLUGIN_ID, CONTRIBUTION_ID);
    let release = async {
        receive_fifo_signal(fixture.ready_fifo.clone())
            .await
            .unwrap();
        release_fixture(fixture);
    };
    let (session, ()) = tokio::join!(start, release);
    session.unwrap()
}

#[tokio::test]
async fn cancelled_close_keeps_process_owned_for_shutdown() {
    use std::future::Future;
    use std::task::Poll;

    let fixture = PausingFixture::new();
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    let session = start_fixture(&host, &fixture).await;
    let client = Arc::clone(&host.sessions.lock().unwrap()[&session.id.0].client);
    let mut close = Box::pin(host.end_session(session.id.clone()));
    std::future::poll_fn(|context| {
        assert!(close.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(close);
    let retained = host.sessions.lock().unwrap().contains_key(&session.id.0);
    // Always clean the real fixture, including when the ownership assertion fails.
    let cleanup = host.shutdown();
    let host_reaped = client.is_terminated();
    let client_cleanup = client.shutdown(TERMINATE_GRACE).await;
    assert!(
        retained,
        "cancelled end_session must retain the process owner for shutdown"
    );
    cleanup.unwrap();
    client_cleanup.unwrap();
    assert!(host_reaped, "shutdown must reap the retained process");
    host.end_session(session.id).await.unwrap();
}

#[tokio::test]
async fn shutdown_rejects_start_before_reading_or_spawning_package() {
    let fixture = PausingFixture::new();
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    host.shutdown().unwrap();
    let package = host.store.load().unwrap().plugins[PLUGIN_ID]
        .package_path
        .clone();
    fs::remove_file(package.join("manifest.json")).unwrap();
    assert_eq!(
        host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await,
        Err(PluginError::NoSession)
    );
    assert!(host.pending_starts.lock().unwrap().entries.is_empty());
}

#[tokio::test]
async fn registration_registry_error_propagates_after_cleanup() {
    let fixture = PausingFixture::new();
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    let start = host.start_session(PLUGIN_ID, CONTRIBUTION_ID);
    let corrupt_registry = async {
        receive_fifo_signal(fixture.ready_fifo.clone())
            .await
            .unwrap();
        fs::write(root.path().join("registry.json"), b"invalid registry").unwrap();
        release_fixture(&fixture);
    };
    let (result, ()) = tokio::join!(start, corrupt_registry);
    let cleaned = host.pending_starts.lock().unwrap().entries.is_empty();
    host.shutdown().unwrap();
    assert!(
        matches!(result, Err(PluginError::Registry(_))),
        "preserve registry read error: {result:?}"
    );
    assert!(cleaned, "successful startup cleanup removes pending owner");
    assert!(host.sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_start_remains_owned_until_shutdown() {
    let fixture = PausingFixture::new();
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    let mut start = Box::pin(host.start_session(PLUGIN_ID, CONTRIBUTION_ID));
    tokio::select! {
        result = &mut start => panic!("start completed before fixture release: {result:?}"),
        ready = receive_fifo_signal(fixture.ready_fifo.clone()) => ready.unwrap(),
    }
    drop(start);
    assert_eq!(host.pending_starts.lock().unwrap().entries.len(), 1);
    host.shutdown().unwrap();
    assert!(host.pending_starts.lock().unwrap().entries.is_empty());
    assert!(host.sessions.lock().unwrap().is_empty());
}

async fn failed_pending_cleanup_cannot_revive_start(reinstall: bool) {
    let fixture = PausingFixture::new();
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    let reached = Arc::new(tokio::sync::Barrier::new(2));
    let resume = Arc::new(tokio::sync::Barrier::new(2));
    *host.registration_barriers.lock().unwrap() = Some((Arc::clone(&reached), Arc::clone(&resume)));
    let mut start = Box::pin(host.start_session(PLUGIN_ID, CONTRIBUTION_ID));
    tokio::select! {
        result = &mut start => panic!("start completed before release: {result:?}"),
        ready = receive_fifo_signal(fixture.ready_fifo.clone()) => ready.unwrap(),
    }
    release_fixture(&fixture);
    tokio::select! {
        result = &mut start => panic!("start completed before registration barrier: {result:?}"),
        _ = reached.wait() => {},
    }
    let client = Arc::clone(
        &host
            .pending_starts
            .lock()
            .unwrap()
            .entries
            .values()
            .next()
            .unwrap()
            .client,
    );
    client.fail_termination_for_test(true);
    let termination = if reinstall {
        let outcome = host.uninstall(PLUGIN_ID).unwrap();
        outcome.package_removal.unwrap();
        host.install(fixture.package.path()).unwrap();
        outcome.session_termination
    } else {
        let outcome = host.set_enabled(PLUGIN_ID, false).unwrap();
        host.set_enabled(PLUGIN_ID, true).unwrap();
        outcome.session_termination
    };
    let (result, _) = tokio::join!(start, resume.wait());
    let pending_retained = host.pending_starts.lock().unwrap().entries.len();
    let active_count = host.sessions.lock().unwrap().len();
    client.fail_termination_for_test(false);
    host.shutdown().unwrap();
    let host_reaped = client.is_terminated();
    client.shutdown(TERMINATE_GRACE).await.unwrap();
    assert!(host_reaped, "shutdown retry must reap the retained process");
    assert!(
        fixture.controls.path().join("release.fifo.home").is_file(),
        "discovery must finish so the test exercises registration invalidation"
    );
    assert!(
        termination.is_err(),
        "pending cleanup failure must propagate"
    );
    assert!(
        result.is_err(),
        "invalidated start must not register after re-enable/reinstall"
    );
    assert_eq!(pending_retained, 1, "failed startup cleanup remains owned");
    assert_eq!(active_count, 0, "stale start never becomes active");
    assert!(host.pending_starts.lock().unwrap().entries.is_empty());
}

#[tokio::test]
async fn failed_disable_cleanup_then_enable_does_not_revive_pending_start() {
    failed_pending_cleanup_cannot_revive_start(false).await;
}

#[tokio::test]
async fn failed_uninstall_cleanup_then_identical_install_does_not_revive_pending_start() {
    failed_pending_cleanup_cannot_revive_start(true).await;
}

#[tokio::test]
async fn failed_active_cleanup_remains_owned_for_shutdown_retry() {
    for operation in ["disable", "uninstall", "shutdown", "close"] {
        let fixture = PausingFixture::new();
        let root = tempfile::tempdir().unwrap();
        let host = PluginHost::new(test_config(root.path()));
        host.install(fixture.package.path()).unwrap();
        let session = start_fixture(&host, &fixture).await;
        let client = Arc::clone(&host.sessions.lock().unwrap()[&session.id.0].client);
        client.fail_termination_for_test(true);
        let result = match operation {
            "disable" => {
                host.set_enabled(PLUGIN_ID, false)
                    .unwrap()
                    .session_termination
            }
            "uninstall" => host.uninstall(PLUGIN_ID).unwrap().session_termination,
            "shutdown" => host.shutdown(),
            "close" => host.end_session(session.id.clone()).await,
            _ => unreachable!(),
        };
        let retained = host.sessions.lock().unwrap().contains_key(&session.id.0);
        let generation = host.session_generation(&session.id);
        client.fail_termination_for_test(false);
        host.shutdown().unwrap();
        let host_reaped = client.is_terminated();
        client.shutdown(TERMINATE_GRACE).await.unwrap();
        assert!(
            host_reaped,
            "{operation}: shutdown retry must reap the retained process"
        );
        assert!(
            result.is_err(),
            "{operation}: cleanup failure must propagate"
        );
        assert!(
            retained,
            "{operation}: failed cleanup must retain process ownership"
        );
        assert_eq!(
            generation, None,
            "{operation}: closing sessions cannot navigate"
        );
        assert!(host.sessions.lock().unwrap().is_empty());
        host.end_session(session.id).await.unwrap();
    }
}

#[tokio::test]
async fn registry_error_with_failed_cleanup_remains_owned_for_retry() {
    let fixture = PausingFixture::new();
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    let mut start = Box::pin(host.start_session(PLUGIN_ID, CONTRIBUTION_ID));
    tokio::select! {
        result = &mut start => panic!("start completed before release: {result:?}"),
        ready = receive_fifo_signal(fixture.ready_fifo.clone()) => ready.unwrap(),
    }
    let client = Arc::clone(
        &host
            .pending_starts
            .lock()
            .unwrap()
            .entries
            .values()
            .next()
            .unwrap()
            .client,
    );
    client.fail_termination_for_test(true);
    fs::write(root.path().join("registry.json"), b"invalid registry").unwrap();
    release_fixture(&fixture);
    let result = start.await;
    let retained = host.pending_starts.lock().unwrap().entries.len();
    client.fail_termination_for_test(false);
    host.shutdown().unwrap();
    let host_reaped = client.is_terminated();
    client.shutdown(TERMINATE_GRACE).await.unwrap();
    assert!(host_reaped, "shutdown retry must reap the retained process");
    assert!(result.is_err());
    assert_eq!(
        retained, 1,
        "registry-error cleanup must retain the process until terminated"
    );
    assert!(host.pending_starts.lock().unwrap().entries.is_empty());
}

#[tokio::test]
async fn guarded_native_operation_rejects_closing_session_without_holding_lock_across_cleanup() {
    use std::future::Future;
    use std::task::Poll;

    let fixture = PausingFixture::new();
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    let session = start_fixture(&host, &fixture).await;
    assert_eq!(
        host.with_current_session(&session.id, session.generation, || 42),
        Some(42)
    );
    assert_eq!(
        host.with_current_session(&session.id, session.generation + 1, || panic!(
            "stale native operation ran"
        )),
        None
    );
    let mut close = Box::pin(host.end_session(session.id.clone()));
    std::future::poll_fn(|context| {
        assert!(close.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    let lock_released = host.registry_lock.try_lock().is_ok();
    drop(close);
    let guarded = host.with_current_session(&session.id, session.generation, || true);
    host.shutdown().unwrap();
    assert!(
        lock_released,
        "process cleanup must not hold the lifecycle lock across await"
    );
    assert_eq!(
        guarded, None,
        "queued native operation must not run on a closing session"
    );
}

#[tokio::test]
async fn failed_discovery_cleanup_remains_owned_until_shutdown_retry() {
    let script = std::str::from_utf8(PAUSING_SCRIPT)
        .unwrap()
        .replace("2026-07-28", "unsupported");
    let fixture = PausingFixture::with_script(script.as_bytes());
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    let mut start = Box::pin(host.start_session(PLUGIN_ID, CONTRIBUTION_ID));
    tokio::select! {
        result = &mut start => panic!("start completed before release: {result:?}"),
        ready = receive_fifo_signal(fixture.ready_fifo.clone()) => ready.unwrap(),
    }
    let client = Arc::clone(
        &host
            .pending_starts
            .lock()
            .unwrap()
            .entries
            .values()
            .next()
            .unwrap()
            .client,
    );
    client.fail_termination_for_test(true);
    release_fixture(&fixture);
    let result = start.await;
    let retained = host.pending_starts.lock().unwrap().entries.len();
    client.fail_termination_for_test(false);
    host.shutdown().unwrap();
    let host_reaped = client.is_terminated();
    client.shutdown(TERMINATE_GRACE).await.unwrap();
    assert!(host_reaped, "shutdown retry must reap the retained process");
    assert!(result.is_err());
    assert_eq!(
        retained, 1,
        "discovery-error cleanup must retain the process until terminated"
    );
    assert!(host.pending_starts.lock().unwrap().entries.is_empty());
}

#[tokio::test]
async fn failed_shutdown_cleanup_cannot_register_pending_or_start_later_process() {
    let fixture = PausingFixture::new();
    let root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(root.path()));
    host.install(fixture.package.path()).unwrap();
    let mut start = Box::pin(host.start_session(PLUGIN_ID, CONTRIBUTION_ID));
    tokio::select! {
        result = &mut start => panic!("start completed before release: {result:?}"),
        ready = receive_fifo_signal(fixture.ready_fifo.clone()) => ready.unwrap(),
    }
    let client = Arc::clone(
        &host
            .pending_starts
            .lock()
            .unwrap()
            .entries
            .values()
            .next()
            .unwrap()
            .client,
    );
    client.fail_termination_for_test(true);
    let shutdown = host.shutdown();
    release_fixture(&fixture);
    let original_start = start.await;
    let later_start = host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await;
    let retained = host.pending_starts.lock().unwrap().entries.len();
    client.fail_termination_for_test(false);
    host.shutdown().unwrap();
    let host_reaped = client.is_terminated();
    client.shutdown(TERMINATE_GRACE).await.unwrap();
    assert!(host_reaped, "shutdown retry must reap the retained process");
    assert!(shutdown.is_err());
    assert!(original_start.is_err());
    assert_eq!(later_start, Err(PluginError::NoSession));
    assert_eq!(retained, 1);
    assert!(host.pending_starts.lock().unwrap().entries.is_empty());
    assert!(host.sessions.lock().unwrap().is_empty());
}
