use crate::shim::Shim;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const MAX_COMMAND_BYTES: usize = 1_000_000;
const CAPTURE_CAP: usize = 10 * 1024 * 1024;
const MAX_BYTES: usize = 50 * 1024;
const MAX_LINES: usize = 2000;
const TAIL_BYTES: usize = 8 * 1024;
const ARTIFACT_RING_SIZE: usize = 8;
const READ_CHUNK: usize = 16 * 1024;

pub struct SharedState {
    pub cwd: PathBuf,
    pub shim: Shim,
    pub session_dir: TempDir,
    pub bootstrap_instructions: String,
    pub artifacts: Mutex<VecDeque<PathBuf>>,
    next_call_id: Mutex<u64>,
}

impl SharedState {
    pub fn new(cwd: PathBuf, shim: Shim) -> std::io::Result<Self> {
        let session_dir = tempfile::Builder::new()
            .prefix("sprout-dev-mcp-session-")
            .tempdir()?;
        let bootstrap_instructions = build_bootstrap(&cwd);
        Ok(Self {
            cwd,
            shim,
            session_dir,
            bootstrap_instructions,
            artifacts: Mutex::new(VecDeque::with_capacity(ARTIFACT_RING_SIZE)),
            next_call_id: Mutex::new(0),
        })
    }

    fn next_id(&self) -> u64 {
        let mut g = match self.next_call_id.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *g += 1;
        *g
    }
}

fn build_bootstrap(cwd: &Path) -> String {
    let stack = detect_stack(cwd);
    let sprout_hint = if std::env::var("SPROUT_RELAY_URL").is_ok()
        && std::env::var("SPROUT_PRIVATE_KEY").is_ok()
    {
        "\nSprout relay configured. Run `sprout --help` to see available commands.\n"
    } else {
        ""
    };
    format!(
        "Working directory: {}\n\
         Detected stack: {}\n\
         Pass `workdir` per call rather than `cd`.\n\
         {sprout_hint}",
        cwd.display(),
        stack,
    )
}

fn detect_stack(cwd: &Path) -> String {
    let markers = [
        ("Cargo.toml", "rust (cargo)"),
        ("package.json", "node"),
        ("go.mod", "go"),
        ("pyproject.toml", "python (pyproject)"),
        ("requirements.txt", "python"),
        ("Gemfile", "ruby"),
        ("pom.xml", "java (maven)"),
        ("build.gradle", "java (gradle)"),
        ("build.gradle.kts", "kotlin (gradle)"),
    ];
    let mut found: Vec<&str> = markers
        .iter()
        .filter(|(f, _)| cwd.join(f).exists())
        .map(|(_, name)| *name)
        .collect();
    if found.is_empty() {
        "unknown".into()
    } else {
        found.sort();
        found.join(", ")
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShellParams {
    pub command: String,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

pub async fn run(
    state: &SharedState,
    p: ShellParams,
    ct: CancellationToken,
) -> Result<CallToolResult, ErrorData> {
    if p.command.len() > MAX_COMMAND_BYTES {
        return Err(ErrorData::invalid_params(
            format!("command exceeds {MAX_COMMAND_BYTES} byte limit"),
            None,
        ));
    }
    let timeout_ms = p
        .timeout_ms
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .min(MAX_TIMEOUT_MS);
    let workdir: PathBuf = p
        .workdir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.cwd.clone());

    if !workdir.is_dir() {
        return Err(ErrorData::invalid_params(
            format!(
                "workdir does not exist or is not a directory: {}",
                workdir.display()
            ),
            None,
        ));
    }

    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(&p.command);
    cmd.current_dir(&workdir);
    cmd.env("PATH", &state.shim.path_env);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    set_process_group(&mut cmd);

    let started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "failed to spawn bash: {e}"
            ))]));
        }
    };

    let pid = child.id();

    struct PgidGuard(Option<u32>);
    impl Drop for PgidGuard {
        fn drop(&mut self) {
            if let Some(pid) = self.0 {
                kill_process_group_immediate(pid as i32);
            }
        }
    }
    let mut pgid_guard = PgidGuard(pid);

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let mut stdout_handle = tokio::spawn(async move {
        match stdout_pipe {
            Some(p) => read_capped(p).await,
            None => CapturedStream::default(),
        }
    });
    let mut stderr_handle = tokio::spawn(async move {
        match stderr_pipe {
            Some(p) => read_capped(p).await,
            None => CapturedStream::default(),
        }
    });

    let timeout_dur = Duration::from_millis(timeout_ms);
    let mut notes: Vec<String> = Vec::new();
    let (status, timed_out) = tokio::select! {
        biased;
        _ = ct.cancelled() => {
            // Kill process group, reap child, abort reader tasks.
            if let Some(pid) = pid {
                kill_process_group_immediate(pid as i32);
            }
            // Bounded reap so we don't leak zombies. If reap times out,
            // PgidGuard drop will SIGKILL again as a last resort.
            match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
                Ok(Ok(_)) => { pgid_guard.0 = None; } // reaped; disarm guard
                Ok(Err(e)) => {
                    tracing::debug!("cancel: child wait error: {e}");
                    // Leave pgid_guard armed for drop-kill.
                }
                Err(_) => {
                    tracing::debug!("cancel: child reap timed out; guard will SIGKILL on drop");
                }
            }
            stdout_handle.abort();
            stderr_handle.abort();
            return Ok(CallToolResult::error(vec![Content::text("cancelled")]));
        }
        r = tokio::time::timeout(timeout_dur, child.wait()) => match r {
        Ok(Ok(s)) => (Some(s), false),
        Ok(Err(err)) => {
            notes.push(format!("child wait failed: {err}"));
            (None, false)
        }
        Err(_) => {
            // Kill process group — this closes the pipes, causing reads to EOF.
            if let Some(pid) = pid {
                kill_process_group_graceful(pid as i32).await;
            }
            // Reap the child so it doesn't become a zombie.
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() >= deadline => {
                        if let Err(e) = child.start_kill() {
                            notes.push(format!("force-kill failed: {e}"));
                        }
                        if let Err(e) = child.wait().await {
                            notes.push(format!("post-kill wait: {e}"));
                        }
                        break;
                    }
                    Ok(None) => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(err) => {
                        notes.push(format!("try_wait failed: {err}"));
                        break;
                    }
                }
            }
            (None, true)
        }
        }
    };

    if !timed_out {
        if let Some(pid) = pid {
            kill_process_group_graceful(pid as i32).await;
        }
    }

    let stdout_cap = match tokio::time::timeout(Duration::from_secs(5), &mut stdout_handle).await {
        Ok(Ok(cap)) => cap,
        _ => {
            stdout_handle.abort();
            notes.push("stdout reader did not complete".into());
            CapturedStream::default()
        }
    };
    let stderr_cap = match tokio::time::timeout(Duration::from_secs(5), &mut stderr_handle).await {
        Ok(Ok(cap)) => cap,
        _ => {
            stderr_handle.abort();
            notes.push("stderr reader did not complete".into());
            CapturedStream::default()
        }
    };

    let duration_ms = started.elapsed().as_millis() as u64;
    let exit_code = status
        .as_ref()
        .and_then(|s| s.code())
        .unwrap_or(if timed_out { 124 } else { -1 });

    let id = state.next_id();
    let (stdout_text, stdout_truncated, stdout_artifact) =
        finalize_stream(state, id, "stdout", stdout_cap, &mut notes);
    let (stderr_text, stderr_truncated, stderr_artifact) =
        finalize_stream(state, id, "stderr", stderr_cap, &mut notes);

    let body = serde_json::json!({
        "exit_code": exit_code,
        "stdout": stdout_text,
        "stderr": stderr_text,
        "timed_out": timed_out,
        "duration_ms": duration_ms,
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated,
        "stdout_artifact": stdout_artifact,
        "stderr_artifact": stderr_artifact,
        "notes": notes,
    });
    let text = serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into());
    pgid_guard.0 = None;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

#[cfg(unix)]
fn set_process_group(cmd: &mut Command) {
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn set_process_group(_cmd: &mut Command) {}

/// Immediate SIGKILL of the process group. Sync; safe to call from Drop.
/// No grace period — used when the parent task is being torn down.
#[cfg(unix)]
fn kill_process_group_immediate(pid: i32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
}

#[cfg(not(unix))]
fn kill_process_group_immediate(_pid: i32) {}

/// Graceful SIGTERM → 200ms async sleep → SIGKILL. Async; never blocks the runtime.
#[cfg(unix)]
async fn kill_process_group_graceful(pid: i32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    let pgid = Pid::from_raw(pid);
    let _ = killpg(pgid, Signal::SIGTERM);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = killpg(pgid, Signal::SIGKILL);
}

#[cfg(not(unix))]
async fn kill_process_group_graceful(_pid: i32) {}

#[derive(Default)]
struct CapturedStream {
    bytes: Vec<u8>,
    /// Total bytes the process produced (may exceed bytes.len() if capped).
    total_bytes: usize,
    capped: bool,
}

async fn read_capped<R: AsyncRead + Unpin>(mut r: R) -> CapturedStream {
    let mut out = CapturedStream::default();
    let mut chunk = vec![0u8; READ_CHUNK];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                out.total_bytes = out.total_bytes.saturating_add(n);
                if !out.capped {
                    let remaining = CAPTURE_CAP.saturating_sub(out.bytes.len());
                    if remaining == 0 {
                        out.capped = true;
                    } else {
                        let take = n.min(remaining);
                        out.bytes.extend_from_slice(&chunk[..take]);
                        if out.bytes.len() >= CAPTURE_CAP {
                            out.capped = true;
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
    out
}

fn finalize_stream(
    state: &SharedState,
    call_id: u64,
    label: &str,
    cap: CapturedStream,
    notes: &mut Vec<String>,
) -> (String, bool, Option<String>) {
    let CapturedStream {
        bytes: buf,
        total_bytes,
        capped,
    } = cap;
    let captured_len = buf.len();
    let line_count = buf.iter().filter(|b| **b == b'\n').count();
    let needs_truncate = capped || captured_len > MAX_BYTES || line_count > MAX_LINES;

    if !needs_truncate {
        return (lossy(buf), false, None);
    }

    let artifact_path = crate::shim::artifact_dir(state.session_dir.path())
        .join(format!("{call_id:06}.{label}.txt"));
    let artifact_str = match std::fs::write(&artifact_path, &buf) {
        Ok(()) => {
            rotate_artifacts(state, artifact_path.clone());
            Some(artifact_path.to_string_lossy().into_owned())
        }
        Err(e) => {
            notes.push(format!(
                "{label}: artifact write failed ({}): {e}",
                artifact_path.display()
            ));
            None
        }
    };

    let tail_start = captured_len.saturating_sub(TAIL_BYTES);
    let tail_aligned = align_to_char_boundary(&buf, tail_start);
    let tail = lossy(buf[tail_aligned..].to_vec());

    let cap_note = if capped {
        format!(
            " (capture capped at {} bytes; further output discarded)",
            CAPTURE_CAP
        )
    } else {
        String::new()
    };
    let artifact_suffix = match &artifact_str {
        Some(p) => format!("; captured output (first 10MB) at {p}"),
        None => "; artifact unavailable".into(),
    };
    let notice = format!(
        "[truncated: showing last {} bytes; {} bytes captured / {} lines / {} bytes total{cap_note}{artifact_suffix}]\n",
        tail.len(),
        captured_len,
        line_count,
        total_bytes,
    );
    let mut out = String::with_capacity(notice.len() + tail.len());
    out.push_str(&notice);
    out.push_str(&tail);
    (out, true, artifact_str)
}

fn align_to_char_boundary(buf: &[u8], start: usize) -> usize {
    let mut i = start.min(buf.len());
    while i < buf.len() && (buf[i] & 0xC0) == 0x80 {
        i += 1;
    }
    i
}

fn lossy(buf: Vec<u8>) -> String {
    String::from_utf8(buf).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn rotate_artifacts(state: &SharedState, new_path: PathBuf) {
    let mut ring = match state.artifacts.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    ring.push_back(new_path);
    while ring.len() > ARTIFACT_RING_SIZE {
        if let Some(old) = ring.pop_front() {
            let _ = std::fs::remove_file(old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shim::Shim;
    use serde_json::Value;
    use tempfile::tempdir;

    fn make_state(cwd: &std::path::Path) -> SharedState {
        let shim = Shim::install().expect("shim install");
        SharedState::new(cwd.to_path_buf(), shim).expect("state new")
    }

    /// Pull the JSON body out of a CallToolResult so tests can assert on fields.
    fn body(r: rmcp::model::CallToolResult) -> Value {
        let text = match r.content.first().and_then(|c| c.as_text()) {
            Some(t) => t.text.clone(),
            None => panic!("no text content"),
        };
        serde_json::from_str(&text).expect("json")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn basic_echo() {
        let dir = tempdir().expect("tempdir");
        let state = make_state(dir.path());
        let r = run(
            &state,
            ShellParams {
                command: "echo hello".into(),
                workdir: None,
                timeout_ms: Some(5_000),
            },
            CancellationToken::new(),
        )
        .await
        .expect("ok");
        let v = body(r);
        assert_eq!(v["exit_code"], 0);
        assert_eq!(v["stdout"], "hello\n");
        assert_eq!(v["timed_out"], false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timeout_fires() {
        let dir = tempdir().expect("tempdir");
        let state = make_state(dir.path());
        let r = run(
            &state,
            ShellParams {
                command: "sleep 999".into(),
                workdir: None,
                timeout_ms: Some(150),
            },
            CancellationToken::new(),
        )
        .await
        .expect("ok");
        let v = body(r);
        assert_eq!(v["timed_out"], true);
        assert_eq!(v["exit_code"], 124);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn workdir_is_honored() {
        let dir = tempdir().expect("tempdir");
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).expect("mkdir sub");
        let state = make_state(dir.path());
        let r = run(
            &state,
            ShellParams {
                command: "pwd".into(),
                workdir: Some(sub.display().to_string()),
                timeout_ms: Some(5_000),
            },
            CancellationToken::new(),
        )
        .await
        .expect("ok");
        let v = body(r);
        let stdout = v["stdout"].as_str().unwrap_or("");
        // Compare canonicalized paths (macOS /tmp -> /private/tmp, etc.).
        let sub_canon = std::fs::canonicalize(&sub).expect("canon");
        assert!(
            stdout
                .trim()
                .ends_with(sub_canon.to_string_lossy().as_ref())
                || stdout.contains(sub.file_name().unwrap().to_str().unwrap()),
            "stdout: {stdout}"
        );
    }
}
