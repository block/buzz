#![cfg(unix)]
//! Acceptance contract for headless browser plugin installation and calls.

use buzz_lib::plugin_host::{PluginError, PluginHost, PluginHostConfig, ResolveKind};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::unix::AsyncFd;

const PLUGIN_ID: &str = "dev.example.acceptance-browser";
const CONTRIBUTION_ID: &str = "web";
const HOME_URL: &str = "https://example.com/";

macro_rules! require_ok {
    ($result:expr, $context:literal) => {{
        let result = $result;
        assert!(result.is_ok(), "{}: {:?}", $context, result);
        result.ok().expect("the preceding assertion requires Ok")
    }};
}

#[tokio::test]
async fn installs_and_resolves_browser_package() {
    let fixture = FixturePackage::new();
    let host_root = tempfile::tempdir().expect("create plugin host root");
    let host = PluginHost::new(test_config(&host_root));

    let staged = require_ok!(host.stage(fixture.package.path()), "stage fixture package");
    assert_eq!(
        staged.executable_sha256, fixture.executable_sha256,
        "staging records the digest of the copied executable"
    );

    fixture.replace_source_after_staging();
    let installed = require_ok!(host.commit(staged), "commit staged package");
    assert_eq!(
        installed.plugin_id, PLUGIN_ID,
        "commit returns the installed plugin"
    );

    let listed = require_ok!(host.list(), "list installed plugins");
    assert_eq!(listed.len(), 1, "the committed plugin is listed once");
    assert_eq!(
        listed[0].plugin_id, PLUGIN_ID,
        "the listed plugin has the manifest id"
    );

    let session = require_ok!(
        host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await,
        "start the installed browser contribution"
    );
    let home = require_ok!(
        host.resolve(session.id.clone(), ResolveKind::Home, None)
            .await,
        "resolve the contribution home address"
    );
    assert_eq!(
        home.url, HOME_URL,
        "home resolution uses the frozen executable"
    );

    let address = require_ok!(
        host.resolve(session.id, ResolveKind::Address, Some("example.org/docs"),)
            .await,
        "resolve a submitted address"
    );
    assert_eq!(
        address.url, "https://example.org/docs",
        "address resolution returns the plugin's admitted URL"
    );

    let transcript = fs::read_to_string(&fixture.trace_path).expect("read fixture transcript");
    assert!(
        transcript.contains("\"method\":\"server/discover\""),
        "session sends a framed server/discover request: {transcript}"
    );
    assert!(
        transcript.contains("\"method\":\"tools/list\""),
        "session sends a framed tools/list request: {transcript}"
    );
    assert!(
        transcript.contains("\"method\":\"tools/call\"")
            && transcript.contains("io.modelcontextprotocol/protocolVersion")
            && transcript.contains("2026-07-28"),
        "resolution sends schema-versioned tools/call frames: {transcript}"
    );
}

#[tokio::test]
async fn rejects_invalid_and_unresponsive_plugin_calls() {
    assert_refusal_cases(&[
        RefusalCase {
            input: "file-answer",
            expected: ExpectedRefusal::NavigationDenied,
        },
        RefusalCase {
            input: "malformed-frame",
            expected: ExpectedRefusal::Protocol,
        },
        RefusalCase {
            input: "oversized-frame",
            expected: ExpectedRefusal::Protocol,
        },
    ])
    .await;

    let fixture = FixturePackage::new();
    let host_root = tempfile::tempdir().expect("create timeout host root");
    let host = Arc::new(PluginHost::new(test_config(&host_root)));
    let installed = host.install(fixture.package.path());
    assert!(installed.is_ok(), "install timeout fixture: {installed:?}");

    let session = require_ok!(
        host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await,
        "start timeout fixture session"
    );
    let timeout_host = Arc::clone(&host);
    let timeout_session = session.id.clone();
    let timeout_call = tokio::spawn(async move {
        timeout_host
            .resolve(timeout_session, ResolveKind::Address, Some("block-timeout"))
            .await
    });

    require_ok!(
        receive_fifo_signal(fixture.ready_fifo.clone()).await,
        "fixture reports that the timeout call is pending"
    );
    let timeout_result = require_ok!(timeout_call.await, "join timeout call");
    assert_eq!(
        timeout_result,
        Err(PluginError::PluginTimeout),
        "a call held past the configured deadline returns PluginTimeout"
    );
    require_ok!(
        send_fifo_signal(fixture.release_fifo.clone()).await,
        "release the timed-out fixture response"
    );

    let later_call = host
        .resolve(session.id, ResolveKind::Address, Some("example.org/docs"))
        .await;
    assert_eq!(
        later_call.map(|admitted| admitted.url),
        Ok("https://example.org/docs".to_string()),
        "the same session accepts a valid call after discarding the late response"
    );
}

#[tokio::test]
async fn disabling_invalidates_calls_and_uninstall_removes_package() {
    let fixture = FixturePackage::new();
    let host_root = tempfile::tempdir().expect("create lifecycle host root");
    let host = Arc::new(PluginHost::new(test_config(&host_root)));
    let installed = host.install(fixture.package.path());
    assert!(
        installed.is_ok(),
        "install lifecycle fixture: {installed:?}"
    );

    let session = require_ok!(
        host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await,
        "start lifecycle fixture session"
    );
    let installed_executable = fixture.installed_executable_path();
    assert!(
        installed_executable.is_file(),
        "the running session reports an executable in the frozen package: {}",
        installed_executable.display()
    );

    let pending_host = Arc::clone(&host);
    let pending_session = session.id.clone();
    let pending_call = tokio::spawn(async move {
        pending_host
            .resolve(pending_session, ResolveKind::Address, Some("block-pending"))
            .await
    });
    require_ok!(
        receive_fifo_signal(fixture.ready_fifo.clone()).await,
        "fixture reports that the lifecycle call is pending"
    );

    let disabled = host.set_enabled(PLUGIN_ID, false);
    assert!(disabled.is_ok(), "disable installed plugin: {disabled:?}");
    let pending_result = require_ok!(pending_call.await, "join invalidated call");
    assert_eq!(
        pending_result,
        Err(PluginError::StaleGeneration),
        "disable rejects the pending call as a stale generation"
    );

    let disabled_plugins = require_ok!(host.list(), "list disabled plugins");
    assert_eq!(
        disabled_plugins.len(),
        1,
        "disable retains the package record"
    );
    assert!(
        !disabled_plugins[0].enabled,
        "disable marks the retained package disabled"
    );
    assert!(
        installed_executable.is_file(),
        "disable retains the frozen package files"
    );

    let uninstalled = host.uninstall(PLUGIN_ID);
    assert!(
        uninstalled.is_ok(),
        "uninstall disabled plugin: {uninstalled:?}"
    );
    let remaining_plugins = require_ok!(host.list(), "list plugins after uninstall");
    assert!(
        remaining_plugins.is_empty(),
        "uninstall removes the registry record and grant"
    );
    assert!(
        !installed_executable.exists(),
        "uninstall removes the frozen package directory"
    );

    let later_call = host
        .resolve(session.id, ResolveKind::Address, Some("example.org"))
        .await;
    assert_eq!(
        later_call,
        Err(PluginError::NoSession),
        "uninstall removes the old session"
    );
}

#[derive(Clone, Copy)]
enum ExpectedRefusal {
    NavigationDenied,
    Protocol,
}

struct RefusalCase {
    input: &'static str,
    expected: ExpectedRefusal,
}

async fn assert_refusal_cases(cases: &[RefusalCase]) {
    for case in cases {
        let fixture = FixturePackage::new();
        let host_root = tempfile::tempdir().expect("create refusal host root");
        let host = PluginHost::new(test_config(&host_root));
        let installed = host.install(fixture.package.path());
        assert!(
            installed.is_ok(),
            "install {} fixture: {installed:?}",
            case.input
        );
        let session = require_ok!(
            host.start_session(PLUGIN_ID, CONTRIBUTION_ID).await,
            "start refusal fixture session"
        );

        let result = host
            .resolve(session.id, ResolveKind::Address, Some(case.input))
            .await;
        match case.expected {
            ExpectedRefusal::NavigationDenied => assert_eq!(
                result,
                Err(PluginError::NavigationDenied {
                    address: "file:///etc/passwd".to_string(),
                }),
                "{} response is rejected by navigation policy",
                case.input
            ),
            ExpectedRefusal::Protocol => assert!(
                matches!(result, Err(PluginError::PluginProtocol(_))),
                "{} response is rejected as a protocol error: {result:?}",
                case.input
            ),
        }
    }
}

fn test_config(root: &TempDir) -> PluginHostConfig {
    PluginHostConfig {
        root: root.path().to_path_buf(),
        resolve_deadline: Duration::from_millis(150),
        discover_deadline: Duration::from_secs(2),
    }
}

struct FixturePackage {
    package: TempDir,
    controls: TempDir,
    executable_path: PathBuf,
    executable_sha256: String,
    ready_fifo: PathBuf,
    release_fifo: PathBuf,
    trace_path: PathBuf,
}

impl FixturePackage {
    fn new() -> Self {
        let package = tempfile::tempdir().expect("create fixture package directory");
        let controls = tempfile::tempdir().expect("create fixture control directory");
        let ready_fifo = controls.path().join("ready.fifo");
        let release_fifo = controls.path().join("release.fifo");
        create_fifo(&ready_fifo);
        create_fifo(&release_fifo);
        let trace_path = controls.path().join("requests.ndjson");

        let binary_directory = package.path().join("bin");
        fs::create_dir(&binary_directory).expect("create fixture binary directory");
        let target_triple = host_target_triple();
        let executable_path = binary_directory.join(&target_triple);
        fs::write(&executable_path, fixture_script()).expect("write fixture executable");
        let mut permissions = fs::metadata(&executable_path)
            .expect("read fixture executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable_path, permissions).expect("mark fixture executable");

        let controls_file = package.path().join("fixture-controls");
        fs::write(
            controls_file,
            format!(
                "{}\n{}\n{}\n",
                ready_fifo.display(),
                release_fifo.display(),
                trace_path.display()
            ),
        )
        .expect("write fixture controls");

        let executable_bytes = fs::read(&executable_path).expect("read fixture executable");
        let executable_sha256 = hex::encode(Sha256::digest(&executable_bytes));
        let mut targets = Map::new();
        targets.insert(
            target_triple.clone(),
            json!({
                "path": format!("bin/{target_triple}"),
                "sha256": executable_sha256,
                "bytes": executable_bytes.len(),
            }),
        );
        let manifest = json!({
            "packageFormatVersion": "0.1.0-alpha",
            "contractVersion": "0.1.0-alpha",
            "id": PLUGIN_ID,
            "name": "Acceptance Browser",
            "version": "0.1.0",
            "publisher": "Buzz Tests",
            "license": "Apache-2.0",
            "runtime": { "type": "stdio", "targets": Value::Object(targets) },
            "grants": ["browser.browse"],
            "contributions": [{
                "kind": "browser",
                "id": CONTRIBUTION_ID,
                "title": "Web",
            }],
        });
        fs::write(
            package.path().join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("serialize fixture manifest"),
        )
        .expect("write fixture manifest");

        Self {
            package,
            controls,
            executable_path,
            executable_sha256,
            ready_fifo,
            release_fifo,
            trace_path,
        }
    }

    fn replace_source_after_staging(&self) {
        fs::write(&self.executable_path, "#!/bin/sh\nexit 91\n")
            .expect("replace source executable");
        fs::write(self.package.path().join("manifest.json"), "{}\n")
            .expect("replace source manifest");
    }

    fn installed_executable_path(&self) -> PathBuf {
        let transcript = fs::read_to_string(&self.trace_path).expect("read fixture transcript");
        let executable = transcript
            .lines()
            .find_map(|line| line.strip_prefix("EXECUTABLE="))
            .expect("fixture reports its installed executable path");
        PathBuf::from(executable)
    }
}

impl Drop for FixturePackage {
    fn drop(&mut self) {
        let _ = &self.controls;
    }
}

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
    receive_fifo_signal_with_deadline(path, Duration::from_secs(5)).await
}

pub(crate) async fn receive_fifo_signal_with_deadline(
    path: PathBuf,
    deadline: Duration,
) -> io::Result<()> {
    let fifo = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let fifo = AsyncFd::new(fifo)?;
    tokio::time::timeout(deadline, async {
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

async fn send_fifo_signal(path: PathBuf) -> io::Result<()> {
    send_fifo_signal_with_deadline(path, Duration::from_secs(5)).await
}

pub(crate) async fn send_fifo_signal_with_deadline(
    path: PathBuf,
    deadline: Duration,
) -> io::Result<()> {
    let fifo = fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let fifo = AsyncFd::new(fifo)?;
    tokio::time::timeout(deadline, async {
        loop {
            let mut readiness = fifo.writable().await?;
            match readiness.try_io(|inner| {
                let mut file = inner.get_ref();
                file.write_all(b"x")
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "fixture release FIFO timed out"))?
}

fn host_target_triple() -> String {
    env!("BUZZ_PLUGIN_TARGET_TRIPLE").to_string()
}

fn fixture_script() -> &'static str {
    r#"#!/bin/sh
set -eu

SCRIPT_DIRECTORY=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CONTROLS_FILE="$SCRIPT_DIRECTORY/fixture-controls"
READY_FIFO=$(sed -n '1p' "$CONTROLS_FILE")
RELEASE_FIFO=$(sed -n '2p' "$CONTROLS_FILE")
TRACE_PATH=$(sed -n '3p' "$CONTROLS_FILE")
printf 'EXECUTABLE=%s\n' "$0" >> "$TRACE_PATH"

while IFS= read -r request; do
  printf '%s\n' "$request" >> "$TRACE_PATH"
  request_id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{"listChanged":false}},"cacheScope":"private","ttlMs":0,"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"dev.example.acceptance-browser","version":"0.1.0"}}}}\n' "$request_id"
      ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","cacheScope":"private","ttlMs":0,"tools":[{"name":"browser.resolve","title":"Resolve an address","description":"Turn what the user typed into a URL to open.","inputSchema":{"type":"object","additionalProperties":false,"required":["context","kind"],"properties":{"context":{"type":"object","additionalProperties":false,"required":["contractVersion","pluginId","contributionId","sessionId","generation"],"properties":{"contractVersion":{"type":"string"},"pluginId":{"type":"string"},"contributionId":{"type":"string"},"sessionId":{"type":"string"},"generation":{"type":"integer","minimum":0}}},"kind":{"enum":["home","address"]},"input":{"type":"string","maxLength":2048}}},"outputSchema":{"type":"object","additionalProperties":false,"required":["outcome"],"properties":{"outcome":{"enum":["resolved","rejected"]},"url":{"type":"string","maxLength":2048},"title":{"type":"string","maxLength":200},"reason":{"type":"string","maxLength":200}}}}]}}\n' "$request_id"
      ;;
    *'"input":"file-answer"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","content":[{"type":"text","text":"file:///etc/passwd"}],"structuredContent":{"outcome":"resolved","url":"file:///etc/passwd"}}}\n' "$request_id"
      ;;
    *'"input":"malformed-frame"'*)
      printf 'not-json\n'
      ;;
    *'"input":"oversized-frame"'*)
      dd if=/dev/zero bs=1024 count=1025 2>/dev/null | tr '\000' x
      printf '\n'
      ;;
    *'"input":"block-timeout"'*|*'"input":"block-pending"'*)
      printf x > "$READY_FIFO"
      IFS= read -r _release < "$RELEASE_FIFO" || true
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","content":[{"type":"text","text":"https://late.example/"}],"structuredContent":{"outcome":"resolved","url":"https://late.example/"}}}\n' "$request_id"
      ;;
    *'"kind":"home"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","content":[{"type":"text","text":"https://example.com/"}],"structuredContent":{"outcome":"resolved","url":"https://example.com/","title":"Example"}}}\n' "$request_id"
      ;;
    *'"input":"example.org/docs"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","content":[{"type":"text","text":"https://example.org/docs"}],"structuredContent":{"outcome":"resolved","url":"https://example.org/docs","title":"example.org"}}}\n' "$request_id"
      ;;
    *)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","content":[{"type":"text","text":"https://example.org/"}],"structuredContent":{"outcome":"resolved","url":"https://example.org/"}}}\n' "$request_id"
      ;;
  esac
done
"#
}
