//! Exercise the shipped entrypoint, not a second task runner in test code.
#![cfg(unix)]
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

const KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const PUBKEY: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
struct Fixture {
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("buzz-run-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        Self { dir }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
        for (key, _) in std::env::vars().filter(|(k, _)| k.starts_with("BUZZ_")) {
            command.env_remove(key);
        }
        command
            .args([
                "run",
                "--private-key",
                KEY,
                "--relay-url",
                "ws://127.0.0.1:1",
                "--agent-command",
                "python3",
                "--agent-args",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/task_agent.py"))
            .env("TASK_AGENT_LOG", self.dir.join("wire.jsonl"))
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
    fn task(&self) -> Value {
        json!({"version":1,"taskId":"local-1","agentPubkey":PUBKEY,"prompt":"prepared task","maxDurationMs":10000})
    }
    fn run(&self, task: &Value, flags: &[&str], stdin: bool) -> Output {
        let mut command = self.command();
        command.args(flags).arg("--task");
        if stdin {
            command.arg("-");
        } else {
            fs::write(self.dir.join("task.json"), task.to_string()).unwrap();
            command.arg(self.dir.join("task.json"));
        }
        let mut child = command.spawn().unwrap();
        if stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(task.to_string().as_bytes())
                .unwrap();
        }
        wait(child)
    }
    fn wire(&self) -> Vec<Value> {
        fs::read_to_string(self.dir.join("wire.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn wait(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(18);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("runner exceeded bounded lifetime");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn terminal(output: &Output, code: i32, status: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("exactly one JSON terminal record");
    assert_eq!(value["version"], 1);
    assert_eq!(value["status"], status);
    value
}
fn assert_dead(pid: u32) {
    use nix::{sys::signal::kill, unistd::Pid};
    let deadline = Instant::now() + Duration::from_secs(2);
    while kill(Pid::from_raw(pid as i32), None).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        kill(Pid::from_raw(pid as i32), None).is_err(),
        "child {pid} survived"
    );
}

#[test]
fn file_and_stdin_run_one_fresh_equipped_session_without_service() {
    for protocol in [1, 2] {
        let f = Fixture::new();
        for stdin in [false, true] {
            let mut command = f.command();
            command
                .env("TASK_AGENT_PROTOCOL", protocol.to_string())
                .env("BUZZ_ACP_SETUP_PAYLOAD", "invalid-if-used")
                .env(
                    "BUZZ_ACP_ISOLATED_TURN_SOCKET",
                    f.dir.join("must-not-exist.sock"),
                )
                .env("BUZZ_PRIVATE_KEY", "invalid-inherited-key")
                .args([
                    "--no-memory",
                    "--system-prompt",
                    "PERSONA",
                    "--team-instructions",
                    "TEAM",
                    "--model",
                    "chosen",
                    "--effort-level",
                    "high",
                    "--mcp-command",
                    "/test/mcp",
                    "--session-title",
                    "task title",
                    "--agents",
                    "3",
                    "--heartbeat-interval",
                    "10",
                    "--initial-message",
                    "MUST_NOT_RUN",
                    "--task",
                ]);
            let task = f.task().to_string();
            fs::write(f.dir.join("task.json"), &task).unwrap();
            command.arg(if stdin {
                PathBuf::from("-")
            } else {
                f.dir.join("task.json")
            });
            let mut child = command.spawn().unwrap();
            if stdin {
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(task.as_bytes())
                    .unwrap();
            }
            let result = terminal(&wait(child), 0, "completed");
            assert_eq!(result["stopReason"], "end_turn");
            assert!(!f.dir.join("must-not-exist.sock").exists());
        }
        let wire = f.wire();
        let spawned: Vec<_> = wire.iter().filter(|v| v.get("pid").is_some()).collect();
        assert_eq!(spawned.len(), 2);
        assert_ne!(spawned[0]["pid"], spawned[1]["pid"]);
        for proc in spawned {
            assert_eq!(proc["key"], KEY);
            assert_eq!(proc["relay"], "ws://127.0.0.1:1");
            assert_dead(proc["pid"].as_u64().unwrap() as u32);
            assert!(!std::path::Path::new(proc["gitKeyfile"].as_str().unwrap()).exists());
        }
        let sessions: Vec<_> = wire
            .iter()
            .filter(|v| v["method"] == "session/new")
            .collect();
        assert_eq!(sessions.len(), 2);
        let prompts: Vec<_> = wire
            .iter()
            .filter(|v| v["method"] == "session/prompt")
            .collect();
        assert_eq!(prompts.len(), 2);
        for (session, prompt) in sessions.iter().zip(prompts) {
            let setup = session["params"].to_string();
            assert!(setup.contains("/test/mcp"));
            assert!(setup.contains("task title"));
            let standing = if protocol == 2 {
                setup
            } else {
                prompt["params"].to_string()
            };
            assert!(
                standing.contains("PERSONA")
                    && standing.contains("TEAM")
                    && standing.contains("<base>")
                    && standing.contains("## Task Session"),
                "{standing}"
            );
            assert!(prompt["params"].to_string().contains("prepared task"));
            assert!(!prompt["params"].to_string().contains("MUST_NOT_RUN"));
        }
        let options: Vec<_> = wire
            .iter()
            .filter(|v| v["method"] == "session/set_config_option")
            .collect();
        for expected in ["chosen", "high", "bypassPermissions"] {
            assert!(
                options.iter().any(|v| v["params"]["value"] == expected),
                "missing {expected}: {wire:?}"
            );
        }
    }
}

#[test]
fn invalid_input_never_spawns() {
    let f = Fixture::new();
    let valid = f.task();
    for (field, value) in [
        ("version", json!(2)),
        ("taskId", json!("")),
        ("prompt", json!(" ")),
        ("agentPubkey", json!("0".repeat(64))),
        ("maxDurationMs", json!(0)),
        ("command", json!("malicious")),
    ] {
        let mut task = valid.clone();
        task[field] = value;
        terminal(&f.run(&task, &["--no-memory"], true), 2, "invalid");
    }
    for source in ["https://example.test/task", "./missing.json"] {
        let mut command = f.command();
        command.args(["--task", source]);
        terminal(&wait(command.spawn().unwrap()), 2, "invalid");
    }
    for bytes in [b"{} {}".to_vec(), b"{".to_vec()] {
        let mut command = f.command();
        command.args(["--task", "-"]);
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        terminal(&wait(child), 2, "invalid");
    }
    assert!(f.wire().is_empty());
}

#[test]
fn oversized_file_and_stdin_report_task_too_large_before_spawn() {
    let f = Fixture::new();
    let bytes = vec![b' '; 1024 * 1024 + 1];
    fs::write(f.dir.join("oversized.json"), &bytes).unwrap();
    for stdin in [false, true] {
        let mut command = f.command();
        command.arg("--task").arg(if stdin {
            PathBuf::from("-")
        } else {
            f.dir.join("oversized.json")
        });
        let mut child = command.spawn().unwrap();
        if stdin {
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
        }
        let result = terminal(&wait(child), 2, "invalid");
        assert_eq!(result["error"], "task_too_large");
        assert_eq!(result["taskId"], Value::Null);
        assert!(f.wire().is_empty());
    }
}

#[test]
fn blocked_configuration_reads_are_cancellable_and_bounded() {
    use nix::{
        sys::signal::{kill, Signal},
        unistd::Pid,
    };
    use std::os::unix::fs::OpenOptionsExt;

    for flag in ["--system-prompt-file", "--base-prompt-file"] {
        for signal in [Some(Signal::SIGINT), Some(Signal::SIGTERM), None] {
            let f = Fixture::new();
            let fifo = f.dir.join("prompt.fifo");
            assert!(Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success());
            let started = Instant::now();
            let child = f
                .command()
                .arg(flag)
                .arg(&fifo)
                .args(["--no-memory", "--task", "-"])
                .spawn()
                .unwrap();
            // A successful nonblocking writer open proves Config has opened
            // this FIFO for reading. Hold it without data, so read_to_string
            // cannot finish. No sleep-based assumption about startup readiness.
            let until = Instant::now() + Duration::from_secs(5);
            let writer = loop {
                match fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(nix::libc::O_NONBLOCK)
                    .open(&fifo)
                {
                    Ok(writer) => break writer,
                    Err(error) if error.raw_os_error() == Some(nix::libc::ENXIO) => {
                        assert!(Instant::now() < until, "configuration never opened FIFO");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("FIFO writer: {error}"),
                }
            };
            if let Some(signal) = signal {
                let signalled = Instant::now();
                kill(Pid::from_raw(child.id() as i32), signal).unwrap();
                let code = if signal == Signal::SIGINT { 130 } else { 143 };
                let result = terminal(&wait(child), code, "cancelled");
                assert_eq!(result["taskId"], Value::Null);
                assert!(signalled.elapsed() < Duration::from_secs(3));
            } else {
                let result = terminal(&wait(child), 2, "invalid");
                assert_eq!(result["error"], "configuration_timeout");
                assert!(started.elapsed() < Duration::from_secs(15));
            }
            drop(writer);
            assert!(f.wire().is_empty());
        }
    }
}

#[test]
fn stop_reasons_failures_and_permission_exchange_are_explicit() {
    for (mode, reason, code, status) in [
        ("normal", "refusal", 0, "completed"),
        ("normal", "max_tokens", 0, "completed"),
        ("error", "", 1, "failed"),
        ("permission", "end_turn", 0, "completed"),
    ] {
        let f = Fixture::new();
        let mut command = f.command();
        command
            .env("TASK_AGENT_MODE", mode)
            .env("TASK_STOP_REASON", reason)
            .args(["--no-memory", "--task", "-"]);
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(f.task().to_string().as_bytes())
            .unwrap();
        let output = wait(child);
        let result = terminal(&output, code, status);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("SECRET_TASK_TEXT"));
        if code == 0 {
            assert_eq!(result["stopReason"], reason);
        }
        if mode == "permission" {
            assert!(f
                .wire()
                .iter()
                .any(|v| v["id"] == "permission" && v.get("result").is_some()));
        }
    }
}

#[test]
fn deadline_covers_initialize_session_and_turn_and_cleans_descendants() {
    for mode in [
        "descendant-startup",
        "session-hang",
        "descendant-turn",
        "ignore-cancel",
    ] {
        let f = Fixture::new();
        let mut task = f.task();
        task["maxDurationMs"] = json!(350);
        let mut command = f.command();
        command
            .env("TASK_AGENT_MODE", mode)
            .args(["--no-memory", "--task", "-"]);
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(task.to_string().as_bytes())
            .unwrap();
        terminal(&wait(child), 124, "timed_out");
        for frame in f.wire() {
            for key in ["pid", "descendant"] {
                if let Some(pid) = frame[key].as_u64() {
                    assert_dead(pid as u32);
                }
            }
        }
    }
}

#[test]
fn signals_cancel_startup_turn_and_pending_stdin_without_hanging() {
    use nix::{
        sys::signal::{kill, Signal},
        unistd::Pid,
    };
    for (mode, signal, code) in [
        ("startup-hang", Signal::SIGINT, 130),
        ("turn-hang", Signal::SIGTERM, 143),
        ("stdin", Signal::SIGTERM, 143),
    ] {
        let f = Fixture::new();
        let mut command = f.command();
        command
            .env("TASK_AGENT_MODE", mode)
            .args(["--no-memory", "--task", "-"]);
        let mut child = command.spawn().unwrap();
        let held_stdin = if mode == "stdin" {
            child.stdin.take()
        } else {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(f.task().to_string().as_bytes())
                .unwrap();
            None
        };
        let until = Instant::now() + Duration::from_secs(3);
        if mode == "stdin" {
            std::thread::sleep(Duration::from_millis(250));
        } else {
            let method = if mode == "turn-hang" {
                "session/prompt"
            } else {
                "initialize"
            };
            while !f.wire().iter().any(|v| v["method"] == method) {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        kill(Pid::from_raw(child.id() as i32), signal).unwrap();
        terminal(&wait(child), code, "cancelled");
        drop(held_stdin);
        for frame in f.wire() {
            if let Some(pid) = frame["pid"].as_u64() {
                assert_dead(pid as u32);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_is_loaded_before_session_and_opt_out_makes_no_request() {
    use axum::{body::Bytes, http::HeaderMap, routing::post, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    for (body, expected) in [("[]", true), ("not-json", false)] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let unexpected = Arc::new(AtomicUsize::new(0));
        let rejected = unexpected.clone();
        // Use the real HTTP parser rather than byte-wise timed socket reads.
        // Idle connections and partial packets cannot block another request.
        let app = Router::new()
            .route(
                "/query",
                post(move |headers: HeaderMap, bytes: Bytes| {
                    let observed = observed.clone();
                    async move {
                        assert!(headers["authorization"]
                            .to_str()
                            .unwrap()
                            .starts_with("Nostr "));
                        assert!(String::from_utf8(bytes.to_vec()).unwrap().contains("30174"));
                        observed.fetch_add(1, Ordering::SeqCst);
                        body
                    }
                }),
            )
            .fallback(move || {
                rejected.fetch_add(1, Ordering::SeqCst);
                async { axum::http::StatusCode::NOT_FOUND }
            });
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let mut server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        for disabled in [false, true] {
            let f = Fixture::new();
            let command = f.command();
            // Remove the fixture's default URL, retaining the rest of the argv.
            let argv: Vec<_> = command.get_args().map(|s| s.to_os_string()).collect();
            let mut command = Command::new(command.get_program());
            for (k, v) in f.command().get_envs() {
                match v {
                    Some(v) => {
                        command.env(k, v);
                    }
                    None => {
                        command.env_remove(k);
                    }
                }
            }
            command
                .args(argv.iter().map(|s| {
                    if s == "ws://127.0.0.1:1" {
                        std::ffi::OsStr::new(&url)
                    } else {
                        s.as_os_str()
                    }
                }))
                .args(["--agent-owner", PUBKEY, "--task", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if disabled {
                command.arg("--no-memory");
            }
            let before = calls.load(Ordering::SeqCst);
            let mut child = command.spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(f.task().to_string().as_bytes())
                .unwrap();
            terminal(&wait(child), 0, "completed");
            assert_eq!(
                calls.load(Ordering::SeqCst) - before,
                usize::from(!disabled)
            );
            let wire = f.wire();
            let session = wire.iter().find(|v| v["method"] == "session/new").unwrap();
            assert_eq!(
                session.to_string().contains("<core-memory>"),
                expected && !disabled
            );
        }
        assert_eq!(
            unexpected.load(Ordering::SeqCst),
            0,
            "unexpected service access"
        );
        stop.send(()).unwrap();
        match tokio::time::timeout(Duration::from_secs(2), &mut server).await {
            Ok(result) => result.unwrap(),
            Err(_) => {
                server.abort();
                let _ = server.await;
                panic!("memory server did not drain");
            }
        }
    }
}
