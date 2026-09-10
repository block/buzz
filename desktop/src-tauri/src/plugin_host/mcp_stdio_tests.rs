use super::*;
use crate::plugin_host::types::ResolveKind;
use jsonschema::Validator;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use serde_json::Value;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::time::Duration;

struct TestClient(Arc<Client>);

impl TestClient {
    fn spawn(path: &std::path::Path, plugin_id: &str, contract: &str) -> Result<Self, PluginError> {
        Client::spawn(path, plugin_id, contract).map(Self)
    }
}

impl std::ops::Deref for TestClient {
    type Target = Arc<Client>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TestClient {
    fn drop(&mut self) {
        if let Err(error) =
            terminate_clients_blocking(&[Arc::clone(&self.0)], Duration::from_millis(100))
        {
            if std::thread::panicking() {
                eprintln!("fixture cleanup failed: {error:?}");
            } else {
                panic!("fixture cleanup failed: {error:?}");
            }
        }
    }
}

const PLUGIN_ID: &str = "dev.example.mcp-stdio-test";
const CONTRACT_VERSION: &str = "0.1.0-alpha";

fn write_script(directory: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = directory.join("plugin.sh");
    let mut file = fs::File::create(&path).expect("create fixture script");
    file.write_all(body.as_bytes())
        .expect("write fixture script");
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

const HANDSHAKE_SNIPPET: &str = r#"
respond_discover() {
  printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{"listChanged":false}},"cacheScope":"private","ttlMs":0}}\n' "$1"
}
respond_list() {
  printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","cacheScope":"private","ttlMs":0,"tools":[{"name":"browser.resolve","title":"Resolve","description":"d","inputSchema":{"type":"object"}}]}}\n' "$1"
}
"#;

async fn handshake(client: &Client) {
    client
        .discover(Duration::from_secs(5))
        .await
        .expect("discover succeeds");
    client
        .tools_list(Duration::from_secs(5))
        .await
        .expect("tools/list succeeds");
}

#[tokio::test]
async fn oversized_frame_is_a_protocol_error() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) dd if=/dev/zero bs=1024 count=1025 2>/dev/null | tr '\000' x; printf '\n' ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(PluginError::PluginProtocol(_))));
}

#[tokio::test]
async fn malformed_frame_is_a_protocol_error() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) printf 'not-json-at-all\n' ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(PluginError::PluginProtocol(_))));
}

#[tokio::test]
async fn unknown_response_id_is_a_protocol_error() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) printf '{{"jsonrpc":"2.0","id":99999,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.com/"}}}}}}\n' ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(PluginError::PluginProtocol(_))));
}

#[tokio::test]
async fn duplicate_response_id_is_a_protocol_error() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.com/"}}}}}}\n' "$id"
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.com/"}}}}}}\n' "$id"
      ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let first = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(first.is_ok(), "first response delivers normally: {first:?}");
    // The duplicate arrives asynchronously on the same stdout stream, so
    // there is no single call whose completion proves the reader has already
    // processed it. Retry on a short per-attempt deadline instead of a fixed
    // guessed delay: once the reader has processed the duplicate, `fatal` is
    // set and every subsequent `send()` short-circuits immediately at its own
    // top-of-function check, so this converges as soon as that happens and
    // is bounded overall by the outer timeout rather than by a fixed sleep.
    let second = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let attempt = client
                .resolve(
                    PLUGIN_ID,
                    "web",
                    "session-1",
                    0,
                    CONTRACT_VERSION,
                    ResolveKind::Address,
                    Some("example.org"),
                    Duration::from_millis(200),
                )
                .await;
            if matches!(
                attempt,
                Err(PluginError::PluginProtocol(_)) | Err(PluginError::PluginUnavailable)
            ) {
                return attempt;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the session must become invalid within the bound");
    assert!(
        matches!(
            second,
            Err(PluginError::PluginProtocol(_)) | Err(PluginError::PluginUnavailable)
        ),
        "duplicate id kills the session: {second:?}"
    );
}

#[tokio::test]
async fn deadline_expiry_returns_timeout_and_session_stays_usable() {
    let directory = tempfile::tempdir().unwrap();
    let release_path = directory.path().join("release.fifo");
    nix::unistd::mkfifo(
        &release_path,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    let mut release = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&release_path)
        .unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *'"kind":"home"'*) IFS= read -r release < "{release_path}"; printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://late.example/"}}}}}}\n' "$id" ;;
    *) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.org/docs"}}}}}}\n' "$id" ;;
  esac
done
"#,
            release_path = release_path.display()
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let timed_out = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_millis(150),
        )
        .await;
    assert_eq!(timed_out, Err(PluginError::PluginTimeout));

    release.write_all(b"continue\n").unwrap();

    let recovered = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Address,
            Some("example.org/docs"),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(
        recovered.map(|admitted| admitted.url),
        Ok("https://example.org/docs".to_string()),
        "the session accepts a fresh call after discarding the late timeout response"
    );
}

#[tokio::test]
async fn process_exit_mid_call_is_plugin_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) exit 7 ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(result, Err(PluginError::PluginUnavailable));
}

async fn fill_stdin_pipe(client: &Client) -> bool {
    const PREFILL_CAP_BYTES: usize = 8 * 1024 * 1024;
    const PREFILL_MAX_ITERATIONS: usize = 4096;
    let chunk = vec![b'a'; 65536];
    let mut saw_would_block = false;
    {
        let mut stdin_guard = client.stdin.lock().await;
        let stdin = stdin_guard.as_mut().expect("stdin still open");
        // The fixture never reads its stdin, so pre-filling the pipe here is
        // deliberate test setup, not a protocol exchange; no MCP framing is
        // involved. Ensure the fd is non-blocking so a full pipe reports
        // `EAGAIN` instead of blocking this test itself.
        let flags =
            OFlag::from_bits_truncate(fcntl(&*stdin, FcntlArg::F_GETFL).expect("fcntl F_GETFL"));
        if !flags.contains(OFlag::O_NONBLOCK) {
            fcntl(&*stdin, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))
                .expect("fcntl F_SETFL O_NONBLOCK");
        }
        let mut prefilled = 0usize;
        for _ in 0..PREFILL_MAX_ITERATIONS {
            match nix::unistd::write(&*stdin, &chunk) {
                Ok(written) => {
                    prefilled += written;
                    if prefilled > PREFILL_CAP_BYTES {
                        break;
                    }
                }
                Err(nix::Error::EAGAIN) => {
                    saw_would_block = true;
                    break;
                }
                Err(error) => panic!("prefill write failed: {error}"),
            }
        }
    }

    saw_would_block
}

#[tokio::test]
async fn a_full_stdin_pipe_still_honors_the_deadline() {
    // A child that never reads its stdin can fill the OS pipe buffer purely
    // from unread bytes; once full, `write_all` blocks. The deadline must
    // bound that wait too, not just the response-side wait already covered
    // by `a_deadline_that_never_arrives_never_hangs_the_caller`.
    //
    // Rather than assume how many concurrent requests it takes to exceed an
    // unknown OS pipe buffer size, this test proves the pipe is actually
    // full: it writes raw bytes directly into the child's stdin fd (this test
    // module is a descendant of `mcp_stdio` and can see `Client`'s private
    // `stdin` field) until the kernel itself reports `EAGAIN`, then issues
    // one real request and asserts its write blocks mid-flight and times out.
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        r#"#!/bin/sh
set -eu
exec sleep 60
"#,
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");

    let saw_would_block = fill_stdin_pipe(&client).await;

    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        client.discover(Duration::from_millis(300)),
    )
    .await;

    // Reap the fixture's `sleep 60` before any assertion below can panic and
    // skip it — cleanup must happen regardless of what the assertions find.
    let shutdown_result = client.shutdown(Duration::from_millis(300)).await;

    assert!(
        saw_would_block,
        "expected the kernel to report the pipe as full (EAGAIN) within the prefill limits"
    );
    let outcome = outcome.expect("send() must return within its own deadline, not hang the test");
    assert!(
        matches!(outcome, Err(PluginError::PluginProtocol(_))),
        "a write against a provably full pipe must time out mid-write and invalidate the \
         session: {outcome:?}"
    );
    assert_eq!(shutdown_result, Ok(()));
}

#[tokio::test]
async fn stderr_with_no_newline_over_8kib_does_not_stall_the_session() {
    // A child that writes far more than the retained 8 KiB stderr tail, with
    // no newline anywhere, must not make the stderr reader block waiting for
    // a line to complete (the previous `read_until` implementation grew an
    // unbounded buffer waiting for a newline that never comes); the session
    // must stay fully responsive while this streams in.
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
dd if=/dev/zero bs=1024 count=4096 2>/dev/null | tr '\000' x >&2 &
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.com/"}}}}}}\n' "$id" ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        handshake(&client).await;
        client
            .resolve(
                PLUGIN_ID,
                "web",
                "session-1",
                0,
                CONTRACT_VERSION,
                ResolveKind::Home,
                None,
                Duration::from_secs(5),
            )
            .await
    })
    .await
    .expect("session must stay responsive while an unbounded newline-free stderr stream is read");
    assert_eq!(
        result.map(|admitted| admitted.url),
        Ok("https://example.com/".to_string())
    );
    let tail = client.stderr_tail_snapshot().await;
    assert_eq!(
        tail.len(),
        STDERR_TAIL_BYTES,
        "retained stderr tail must be capped at exactly the configured bound"
    );
    assert!(
        tail.iter().all(|byte| *byte == b'x'),
        "retained tail must hold only the most recent bytes written by the fixture"
    );
}

/// Polls for `path` to exist, bounded by an outer timeout. Uses a plain
/// regular-file existence check (a fast, non-blocking `stat`) rather than a
/// blocking FIFO open/read: a `spawn_blocking` wrapped around a blocking
/// syscall cannot be cancelled once the timeout elapses (the OS thread stays
/// blocked forever), whereas this poll loop is genuinely cancellable at any
/// `.await` point.
async fn wait_for_marker(path: std::path::PathBuf) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture signaled readiness within the bound");
}

async fn process_is_absent_or_zombie(process_id: &str) -> bool {
    let mut child = tokio::process::Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", process_id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut output = Vec::new();
    let observed = tokio::time::timeout(Duration::from_secs(1), async {
        stdout.take(128).read_to_end(&mut output).await.unwrap();
        child.wait().await.unwrap()
    })
    .await;
    let status = match observed {
        Ok(status) => status,
        Err(error) => {
            child.start_kill().unwrap();
            tokio::time::timeout(Duration::from_secs(1), child.wait())
                .await
                .unwrap()
                .unwrap();
            panic!("bounded process-status query timed out: {error}");
        }
    };
    assert!(output.len() < 128, "status query output must be complete");
    let text = std::str::from_utf8(&output).unwrap().trim();
    (status.code() == Some(1) && text.is_empty()) || (status.success() && text.starts_with('Z'))
}

#[tokio::test]
async fn shutdown_kills_a_term_ignoring_descendant_in_the_same_group() {
    // Regression for the fixed shutdown() bug: the old implementation only
    // sent a group SIGKILL when the leader was still alive after the grace
    // wait, so a leader that exits promptly on SIGTERM (as this fixture's
    // main script does, since it sets no trap) could leave alive a
    // descendant sharing its process group that explicitly ignores SIGTERM.
    // The descendant needs no `setpgid` of its own: a process forked from
    // the leader inherits the leader's process group automatically.
    let directory = tempfile::tempdir().unwrap();
    let ready_marker = directory.path().join("ready.marker");
    let descendant_pid_file = directory.path().join("descendant.pid");
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
DESCENDANT_PID_FILE="{pidfile}" READY_MARKER="{marker}" sh -c '
  trap "" TERM
  echo $$ > "$DESCENDANT_PID_FILE"
  : > "$READY_MARKER"
  while true; do sleep 3600 & wait "$!"; done
' &
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
  esac
done
"#,
            marker = ready_marker.display(),
            pidfile = descendant_pid_file.display()
        ),
    );

    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    wait_for_marker(ready_marker).await;

    let descendant_pid = fs::read_to_string(&descendant_pid_file)
        .expect("descendant pid file")
        .trim()
        .to_string();
    assert!(
        std::process::Command::new("kill")
            .args(["-0", &descendant_pid])
            .status()
            .expect("run kill -0")
            .success(),
        "descendant must be alive before shutdown"
    );

    let result = client.shutdown(Duration::from_millis(300)).await;
    assert_eq!(result, Ok(()));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if process_is_absent_or_zombie(&descendant_pid).await {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "descendant must stop after group cleanup"
        );
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_completed_id_evicted_from_the_bounded_window_replays_as_a_harmless_no_op() {
    // `completed` retains only the most recent `COMPLETED_ID_RETENTION` ids
    // to keep memory bounded over a long session. This proves eviction never
    // lets a stale id come back as if it were a fresh accepted response: ids
    // are never reissued, so once evicted, a replay finds no waiter in
    // `outstanding` and is discarded as a harmless no-op — not delivered to
    // whatever call happens to be outstanding, and not a false "duplicate"
    // failure either.
    let directory = tempfile::tempdir().unwrap();
    let total_resolves = COMPLETED_ID_RETENTION + 2;
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
COUNT=0
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *)
      COUNT=$((COUNT+1))
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.com/"}}}}}}\n' "$id"
      if [ "$COUNT" = "{total}" ]; then
        printf '{{"jsonrpc":"2.0","id":3,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://late-replay.example/"}}}}}}\n'
      fi
      ;;
  esac
done
"#,
            total = total_resolves
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;

    // Ids 1 and 2 went to the handshake, so this first resolve call is
    // issued id 3 — the id the fixture replays once it has aged out of the
    // bounded `completed` window.
    for _ in 0..total_resolves {
        let result = client
            .resolve(
                PLUGIN_ID,
                "web",
                "session-1",
                0,
                CONTRACT_VERSION,
                ResolveKind::Home,
                None,
                Duration::from_secs(5),
            )
            .await;
        assert!(
            result.is_ok(),
            "a genuine round trip must keep succeeding: {result:?}"
        );
    }

    // By now id 3 has been evicted from the bounded window and the fixture
    // has just sent one extra, unrequested response reusing it. A live
    // health-check call only resolves after the reader has processed every
    // earlier line on the same ordered stdout stream, including that
    // replay — so its success proves the replay did not kill the session,
    // corrupt dispatch, or get delivered to any real waiter.
    let health_check = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Address,
            Some("example.org/health"),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(
        health_check.map(|admitted| admitted.url),
        Ok("https://example.com/".to_string()),
        "an evicted id's late replay must be a harmless no-op, not a session-ending duplicate"
    );
}

#[tokio::test]
async fn wrong_jsonrpc_version_is_a_protocol_error() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) printf '{{"jsonrpc":"1.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.com/"}}}}}}\n' "$id" ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(PluginError::PluginProtocol(_))));
}

#[tokio::test]
async fn response_missing_result_and_error_is_a_protocol_error() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) printf '{{"jsonrpc":"2.0","id":%s}}\n' "$id" ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(PluginError::PluginProtocol(_))));
}

#[tokio::test]
async fn response_with_both_result_and_error_is_a_protocol_error() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.com/"}}}},"error":{{"code":-32000,"message":"also an error"}}}}\n' "$id" ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(PluginError::PluginProtocol(_))));
}

#[tokio::test]
async fn malformed_notification_carrying_id_is_a_protocol_error() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) printf '{{"jsonrpc":"2.0","method":"notifications/progress","id":%s,"result":{{}}}}\n' "$id" ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(PluginError::PluginProtocol(_))));
}

#[tokio::test]
async fn invalid_resolve_result_terminates_the_session() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{}}}}}}\n' "$id" ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let missing_outcome = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(
        matches!(missing_outcome, Err(PluginError::PluginProtocol(_))),
        "a result with no structuredContent.outcome is a protocol violation: {missing_outcome:?}"
    );

    let after = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Address,
            Some("example.org"),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        matches!(
            after,
            Err(PluginError::PluginProtocol(_)) | Err(PluginError::PluginUnavailable)
        ),
        "an invalid resolve result must invalidate the session, not just the one call: {after:?}"
    );
}

#[tokio::test]
async fn rejected_outcome_does_not_terminate_the_session() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *'"kind":"home"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"rejected","reason":"nope"}}}}}}\n' "$id" ;;
    *) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.org/docs"}}}}}}\n' "$id" ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let rejected = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(
        rejected,
        Err(PluginError::NavigationDenied { .. })
    ));

    let recovered = client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Address,
            Some("example.org/docs"),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(
        recovered.map(|admitted| admitted.url),
        Ok("https://example.org/docs".to_string()),
        "a plugin-side rejection must not kill the process: the next call still succeeds"
    );
}

#[tokio::test]
async fn shutdown_reports_success_once_the_process_exits() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
  esac
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    handshake(&client).await;
    let result = client.shutdown(Duration::from_secs(2)).await;
    assert_eq!(result, Ok(()));
}

#[tokio::test]
async fn a_deadline_that_never_arrives_never_hangs_the_caller() {
    // A child that reads its stdin but responds to nothing must still be
    // bounded by `deadline`: the write/flush succeed immediately, but the
    // response never comes, so this exercises the same wait path as an
    // unresponsive-stdin child without needing a full pipe buffer.
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        r#"#!/bin/sh
set -eu
while IFS= read -r request; do
  :
done
"#,
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.discover(Duration::from_millis(150)),
    )
    .await
    .expect("send() must return within its own deadline, not hang the test");
    assert_eq!(result, Err(PluginError::PluginTimeout));
}

/// Runs a real `Client` against a fixture that records, in order, every byte
/// it receives on stdin and every response line it sends back, then binds
/// those *actual production frames* — not hand-mirrored copies — to the
/// named-payload `$defs` (`DiscoverRequest`/`Result`,
/// `ListToolsRequest`/`Result`, `CallToolRequest`/`CallToolResult`), which are
/// more specific than the generic `JSONRPCResultResponse` wrapper. Ordering is
/// deterministic: the fixture's `while read` loop is single-threaded and
/// appends a request line before dispatching it and a response line before
/// writing it to stdout, so by the time each `Client` call resolves, its
/// request and response are already durably captured — no sleep needed to
/// synchronize with the capture file.
#[tokio::test]
async fn production_frames_conform_to_named_mcp_schema_defs() {
    let directory = tempfile::tempdir().unwrap();
    let capture_path = directory.path().join("capture.jsonl");
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
CAPTURE_FILE="{capture}"
: > "$CAPTURE_FILE"
respond_discover() {{
  resp=$(printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{{"tools":{{"listChanged":false}}}},"cacheScope":"private","ttlMs":0}}}}' "$1")
  printf '%s\n' "$resp" >> "$CAPTURE_FILE"
  printf '%s\n' "$resp"
}}
respond_list() {{
  resp=$(printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","cacheScope":"private","ttlMs":0,"tools":[{{"name":"browser.resolve","title":"Resolve","description":"d","inputSchema":{{"type":"object"}}}}]}}}}' "$1")
  printf '%s\n' "$resp" >> "$CAPTURE_FILE"
  printf '%s\n' "$resp"
}}
respond_call() {{
  resp=$(printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[],"structuredContent":{{"outcome":"resolved","url":"https://example.com/"}}}}}}' "$1")
  printf '%s\n' "$resp" >> "$CAPTURE_FILE"
  printf '%s\n' "$resp"
}}
while IFS= read -r request; do
  printf '%s\n' "$request" >> "$CAPTURE_FILE"
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"server/discover"'*) respond_discover "$id" ;;
    *'"method":"tools/list"'*) respond_list "$id" ;;
    *) respond_call "$id" ;;
  esac
done
"#,
            capture = capture_path.display()
        ),
    );

    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).expect("spawn fixture");
    client
        .discover(Duration::from_secs(5))
        .await
        .expect("discover succeeds");
    client
        .tools_list(Duration::from_secs(5))
        .await
        .expect("tools/list succeeds");
    client
        .resolve(
            PLUGIN_ID,
            "web",
            "session-1",
            0,
            CONTRACT_VERSION,
            ResolveKind::Home,
            None,
            Duration::from_secs(5),
        )
        .await
        .expect("resolve succeeds");

    let captured = fs::read_to_string(&capture_path).expect("read capture file");
    let frames: Vec<Value> = captured
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("captured frame is valid JSON"))
        .collect();
    assert_eq!(
        frames.len(),
        6,
        "expected 3 requests interleaved with 3 responses, got: {frames:?}"
    );

    let schema = load_schema();
    assert_ref_validates(&schema, "DiscoverRequest", &frames[0]);
    assert_ref_validates(&schema, "DiscoverResult", &frames[1]["result"]);
    assert_ref_validates(&schema, "ListToolsRequest", &frames[2]);
    assert_ref_validates(&schema, "ListToolsResult", &frames[3]["result"]);
    assert_ref_validates(&schema, "CallToolRequest", &frames[4]);
    assert_ref_validates(&schema, "CallToolResult", &frames[5]["result"]);
}

#[test]
fn discover_request_missing_required_field_fails_validation() {
    let schema = load_schema();
    let mut request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/discover",
        "params": { "_meta": {} }
    });
    request.as_object_mut().unwrap().remove("jsonrpc");
    assert_ref_rejects(&schema, "DiscoverRequest", &request);
}

#[test]
fn list_tools_request_missing_params_fails_validation() {
    let schema = load_schema();
    let mut request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": { "_meta": {} }
    });
    request.as_object_mut().unwrap().remove("params");
    assert_ref_rejects(&schema, "ListToolsRequest", &request);
}

#[test]
fn call_tool_request_missing_name_fails_validation() {
    let schema = load_schema();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    assert_ref_rejects(&schema, "CallToolRequest", &request);
}

#[test]
fn discover_result_missing_capabilities_fails_validation() {
    let schema = load_schema();
    let result = serde_json::json!({
        "resultType": "complete",
        "supportedVersions": [PROTOCOL_VERSION],
        "cacheScope": "private",
        "ttlMs": 0
    });
    assert_ref_rejects(&schema, "DiscoverResult", &result);
}

#[test]
fn list_tools_result_missing_tools_fails_validation() {
    let schema = load_schema();
    let result = serde_json::json!({
        "resultType": "complete",
        "cacheScope": "private",
        "ttlMs": 0
    });
    assert_ref_rejects(&schema, "ListToolsResult", &result);
}

#[test]
fn call_tool_result_missing_content_fails_validation() {
    let schema = load_schema();
    let result = serde_json::json!({
        "resultType": "complete",
        "structuredContent": { "outcome": "resolved", "url": "https://example.com/" }
    });
    assert_ref_rejects(&schema, "CallToolResult", &result);
}

fn load_schema() -> Value {
    let bytes = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mcp-2026-07-28.schema.json"
    ))
    .expect("read committed MCP schema fixture");
    serde_json::from_slice(&bytes).expect("parse committed MCP schema fixture")
}

fn compile_ref(schema: &Value, def_name: &str) -> Validator {
    let mut document = schema.clone();
    document["$ref"] = Value::String(format!("#/$defs/{def_name}"));
    jsonschema::validator_for(&document).expect("compile schema $ref")
}

fn assert_ref_validates(schema: &Value, def_name: &str, instance: &Value) {
    let validator = compile_ref(schema, def_name);
    let errors: Vec<_> = validator.iter_errors(instance).collect();
    assert!(errors.is_empty(), "{def_name} should validate: {errors:?}");
}

fn assert_ref_rejects(schema: &Value, def_name: &str, instance: &Value) {
    let validator = compile_ref(schema, def_name);
    assert!(
        !validator.is_valid(instance),
        "{def_name} should reject an instance missing a required field"
    );
}

#[tokio::test]
async fn completed_cleanup_never_signals_again_from_any_transport_path() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(directory.path(), "#!/bin/sh\nexec sleep 60\n");
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).unwrap();
    client.shutdown(Duration::from_millis(100)).await.unwrap();
    assert!(client.is_terminated());
    client.fail_termination_for_test(true);
    // A signal attempt now would fail. Every entry point must share the
    // completed process owner, including background protocol violations.
    client.process.signal(GroupSignal::Kill).unwrap();
    client.kill_process_group().await;
    terminate(
        &client.pending,
        &client.process,
        protocol_error("late frame"),
    )
    .await;
    terminate_clients_blocking(&[Arc::clone(&client)], Duration::ZERO).unwrap();
    client.shutdown(Duration::ZERO).await.unwrap();
    assert!(client.is_terminated());
}

#[tokio::test]
async fn failed_signal_cannot_reap_or_mark_process_cleanup_complete() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(directory.path(), "#!/bin/sh\nexec sleep 60\n");
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).unwrap();
    client.fail_termination_for_test(true);
    let result = client.shutdown(Duration::ZERO).await;
    let reaped = client.process.try_reap();
    let terminated = client.is_terminated();
    client.fail_termination_for_test(false);
    client.shutdown(Duration::from_millis(100)).await.unwrap();
    assert!(result.is_err());
    assert!(
        reaped.is_none(),
        "failed group cleanup must not permit leader reaping"
    );
    assert!(!terminated);
    assert!(client.is_terminated());
}

#[tokio::test]
async fn concurrent_async_and_blocking_cleanup_share_process_completion() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(directory.path(), "#!/bin/sh\nexec sleep 60\n");
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let blocking_client = Arc::clone(&client);
    let blocking_barrier = Arc::clone(&barrier);
    let blocking = std::thread::spawn(move || {
        blocking_barrier.wait();
        terminate_clients_blocking(&[blocking_client], Duration::from_millis(100))
    });
    barrier.wait();
    let asynchronous = client.shutdown(Duration::from_millis(100)).await;
    let synchronous = blocking.join().unwrap();
    assert_eq!(asynchronous, Ok(()));
    assert_eq!(synchronous, Ok(()));
    assert!(client.is_terminated());
}

#[tokio::test]
async fn waiting_for_stdin_honors_deadline_without_invalidating_session() {
    let directory = tempfile::tempdir().unwrap();
    let script = write_script(
        directory.path(),
        &format!(
            r#"#!/bin/sh
set -eu
{HANDSHAKE_SNIPPET}
while IFS= read -r request; do
  request_id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  respond_discover "$request_id"
done
"#
        ),
    );
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).unwrap();
    let guard = client.stdin.lock().await;
    let waiting = tokio::time::timeout(
        Duration::from_secs(1),
        client.discover(Duration::from_millis(10)),
    )
    .await;
    drop(guard);
    assert_eq!(waiting, Ok(Err(PluginError::PluginTimeout)));
    client.discover(Duration::from_secs(2)).await.unwrap();
}

#[tokio::test]
async fn cancelling_blocked_writer_prevents_queued_frame_from_using_partial_stream() {
    use std::future::Future;
    use std::task::Poll;

    let directory = tempfile::tempdir().unwrap();
    let script = write_script(directory.path(), "#!/bin/sh\nexec sleep 60\n");
    let client = TestClient::spawn(&script, PLUGIN_ID, CONTRACT_VERSION).unwrap();
    assert!(
        fill_stdin_pipe(&client).await,
        "kernel confirms pipe is full before cancellation"
    );
    let mut writer = Box::pin(client.send(
        "tools/call",
        json!({"padding": "x".repeat(MAX_FRAME_BYTES / 2)}),
        true,
        Duration::from_secs(5),
    ));
    std::future::poll_fn(|context| {
        assert!(writer.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(
        client.stdin.try_lock().is_err(),
        "writer has started while holding stdin"
    );
    drop(writer);
    assert!(
        client.stdin.lock().await.is_none(),
        "cancellation must close the partial stream"
    );
    let queued = client
        .send("tools/list", json!({}), false, Duration::from_millis(100))
        .await;
    assert_eq!(queued, Err(PluginError::PluginUnavailable));
}
