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
    /// The shell resolved at construction: `Ok((path, display_name))` when a shell
    /// is available, `Err(msg)` when none was found. Stored once so both the
    /// bootstrap hint and every `run()` call read the SAME resolution — no drift.
    pub resolved_shell: Result<(PathBuf, String), String>,
    pub artifacts: Mutex<VecDeque<PathBuf>>,
    next_call_id: Mutex<u64>,
}

impl SharedState {
    pub fn new(cwd: PathBuf, shim: Shim) -> std::io::Result<Self> {
        let session_dir = tempfile::Builder::new()
            .prefix("buzz-dev-mcp-session-")
            .tempdir()?;
        // Resolve the shell ONCE using the same PATH the spawn will use.
        // Both the bootstrap dialect hint and every run() call read this result,
        // so they can never disagree. A failed resolution is stored as Err and
        // surfaces as an actionable error on the first tool call.
        let resolved_shell = resolve_bash(&shim.path_env);
        let shell_hint = match &resolved_shell {
            Ok((_, name)) => name.as_str(),
            Err(_) => "bash",
        };
        let bootstrap_instructions = build_bootstrap(&cwd, shell_hint);
        Ok(Self {
            cwd,
            shim,
            session_dir,
            bootstrap_instructions,
            resolved_shell,
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

fn build_bootstrap(cwd: &Path, shell_hint: &str) -> String {
    let stack = detect_stack(cwd);
    let buzz_hint =
        if std::env::var("BUZZ_RELAY_URL").is_ok() && std::env::var("BUZZ_PRIVATE_KEY").is_ok() {
            "\nBuzz relay configured. Run `buzz --help` to see available commands.\n"
        } else {
            ""
        };
    format!(
        "Working directory: {}\n\
         Detected stack: {}\n\
         Shell: {shell_hint} (set BUZZ_SHELL to override) — write command strings in that shell's syntax.\n\
         Pass `workdir` per call rather than `cd`.\n\
         {buzz_hint}",
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
    /// Defaults to 120000 ms (2 min) if omitted; capped at 600000 ms (10 min).
    /// For long-running commands (git push with hooks, cargo build, test suites), use 300000+.
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

    let bash = match &state.resolved_shell {
        Ok((path, _)) => path.clone(),
        Err(msg) => return Ok(CallToolResult::error(vec![Content::text(msg.clone())])),
    };
    let shell_arg = shell_flag(&bash);
    let mut cmd = Command::new(&bash);
    cmd.arg(shell_arg).arg(&p.command);
    cmd.current_dir(&workdir);
    cmd.env("PATH", &state.shim.path_env);
    // NOSTR_PRIVATE_KEY is already removed from this process's env (shim.rs).
    // BUZZ_PRIVATE_KEY is intentionally inherited — the buzz CLI needs it.
    for (k, v) in &state.shim.git_env {
        cmd.env(k, v);
    }
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
                "failed to spawn shell: {e}"
            ))]));
        }
    };

    let pid = child.id();

    // KillGroup ties the spawned bash and all its descendants to a single kill
    // primitive (Unix process group / Windows Job Object). Built from the live
    // child so the Windows job can take the process handle, which only exists
    // after spawn. Held for the whole run; its Drop is the last-resort reaper.
    let mut kill_group = KillGroup::new(&child, pid);

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
            kill_group.kill_immediate();
            // Bounded reap so we don't leak zombies. If reap times out,
            // KillGroup drop will kill again as a last resort.
            match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
                Ok(Ok(_)) => { kill_group.disarm(); } // reaped; disarm guard
                Ok(Err(e)) => {
                    tracing::debug!("cancel: child wait error: {e}");
                    // Leave kill_group armed for drop-kill.
                }
                Err(_) => {
                    tracing::debug!("cancel: child reap timed out; guard will kill on drop");
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
            kill_group.kill_graceful().await;
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
        kill_group.kill_graceful().await;
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
    kill_group.disarm();
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

/// The flag used to pass a command string to the shell.
///
/// bash/zsh/sh: `-c`
/// cmd.exe:     `/C`
/// powershell/pwsh: `-Command`
///
/// The resolver supports `BUZZ_SHELL=cmd`/`pwsh` (explicit operator overrides
/// resolve without the System32 exclusion, so these shells work). The dispatch
/// here ensures each shell receives the correct flag regardless of which one
/// was resolved.
fn shell_flag(shell: &Path) -> &'static str {
    match shell
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("cmd") => "/C",
        Some("powershell" | "pwsh") => "-Command",
        _ => "-c",
    }
}

/// Extract a short display name from a resolved shell path (e.g. `pwsh.exe` → `"pwsh"`).
fn shell_name_from_path(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "bash".to_string())
}

/// Resolve the shell to spawn. On Unix, bash on PATH is correct and was never
/// broken, so the resolver only needs to honor BUZZ_SHELL. The probe logic and
/// System32 exclusion are Windows-only.
///
/// Returns `(resolved_path, display_name)`. The display name is derived from the
/// resolved path so the caller can use it for diagnostics without a second lookup.
#[cfg(not(windows))]
fn resolve_bash(_path_env: &str) -> Result<(PathBuf, String), String> {
    // Honor BUZZ_SHELL on Unix so power users can opt into zsh or another shell.
    if let Some(raw) = std::env::var_os("BUZZ_SHELL") {
        let p = PathBuf::from(&raw);
        // Absolute / rooted path: must exist as a file.
        if p.components().count() > 1 || p.has_root() {
            if p.is_file() {
                let name = shell_name_from_path(&p);
                return Ok((p, name));
            }
            // Non-existent path: fall through to bash.
        } else {
            // Bare command name: scan the process PATH directly.
            let path_var = std::env::var_os("PATH").unwrap_or_default();
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join(&p);
                if candidate.is_file() {
                    let name = shell_name_from_path(&candidate);
                    return Ok((candidate, name));
                }
            }
            // Not found: fall through to bash.
        }
    }
    Ok((PathBuf::from("bash"), "bash".to_string()))
}

/// Windows bash resolution. Probe order (first hit wins):
///   1. `BUZZ_SHELL` env override — explicit operator choice, any shell (cmd,
///      PowerShell, bash, etc.). Bare command names are resolved through PATH
///      WITHOUT the System32 exclusion — the operator explicitly chose this shell,
///      and cmd.exe/powershell.exe live in System32 legitimately.
///   2. `GIT_BASH` env override — legacy escape hatch (kept for back-compat).
///   3. Installed Git for Windows (fast path when the user has Git).
///   4. PATH scan for `bash.exe`, EXCLUDING System32 (so we never resolve WSL's
///      `bash.exe` — the `0x8007072c` hazard).
///
/// Returns `(resolved_path, display_name)`. The display name is derived from the
/// resolved path, guaranteeing the dialect hint and the spawned shell agree.
///
/// The previously-bundled PortableGit fallback (probe 3 in the old order) has
/// been removed: Git for Windows is a documented host prerequisite, and shipping
/// a multi-hundred-MB runtime contradicts the VISION_AGENT.md "minimal" principle.
///
/// No bash found -> actionable error pointing at the prerequisite.
#[cfg(windows)]
fn resolve_bash(path_env: &str) -> Result<(PathBuf, String), String> {
    // BUZZ_SHELL: explicit operator override — any shell, including cmd or PowerShell.
    // Bare command names are resolved WITHOUT System32 exclusion: the operator
    // chose this shell on purpose, and cmd/pwsh legitimately live in System32.
    if let Some(raw) = std::env::var_os("BUZZ_SHELL") {
        let p = PathBuf::from(&raw);
        // Absolute / rooted path: must exist as a file.
        if p.components().count() > 1 || p.has_root() {
            if p.is_file() {
                let name = shell_name_from_path(&p);
                return Ok((p, name));
            }
            // Non-existent absolute path: fall through, do NOT report this shell.
        } else {
            // Bare command name (e.g. "pwsh", "cmd"): scan PATH, NO System32
            // exclusion — the operator explicitly wants this shell.
            if let Some(found) = scan_path_for_command(&p, path_env, None) {
                let name = shell_name_from_path(&found);
                return Ok((found, name));
            }
            // Not found on PATH: fall through.
        }
    }

    // GIT_BASH: legacy override kept for back-compat.
    if let Some(p) = std::env::var_os("GIT_BASH").map(PathBuf::from) {
        if p.is_file() {
            let name = shell_name_from_path(&p);
            return Ok((p, name));
        }
    }

    for root in ["ProgramFiles", "LocalAppData"] {
        if let Some(base) = std::env::var_os(root) {
            let candidate = match root {
                "LocalAppData" => PathBuf::from(&base).join("Programs").join("Git"),
                _ => PathBuf::from(&base).join("Git"),
            }
            .join("bin")
            .join("bash.exe");
            if candidate.is_file() {
                return Ok((candidate, "bash".to_string()));
            }
        }
    }

    // PATH scan for bash.exe, skipping System32 to avoid WSL's bash.exe launcher.
    if let Some(p) = scan_path_for_bash(path_env, std::env::var_os("SystemRoot").map(PathBuf::from))
    {
        return Ok((p, "bash".to_string()));
    }

    Err(
        "Git for Windows (git bash) is required but was not found.\n\
         Install it from https://git-scm.com/download/win and re-launch Buzz,\n\
         or set BUZZ_SHELL to the path of any bash-compatible executable (or a bare\n\
         command name like cmd or pwsh if it is on PATH)."
            .into(),
    )
}

/// True if `dir` is `root` or lives under it, comparing path components
/// case-INsensitively. Windows paths are case-insensitive, but `Path::starts_with`
/// compares components case-sensitively on every platform — so a PATH entry spelled
/// `C:\WINDOWS\System32` would slip past a `%SystemRoot%`=`C:\Windows` prefix test
/// and let WSL's `System32\bash.exe` be resolved, reintroducing the `0x8007072c`
/// spawn failure. Component-wise comparison (not a lowercased substring match) avoids
/// a false hit on a sibling like `C:\Windows2`.
#[cfg(windows)]
fn is_under_dir(dir: &Path, root: &Path) -> bool {
    let mut dir_components = dir.components();
    for root_component in root.components() {
        match dir_components.next() {
            Some(d)
                if d.as_os_str()
                    .eq_ignore_ascii_case(root_component.as_os_str()) => {}
            _ => return false,
        }
    }
    true
}

/// Scan the child's PATH for `bash.exe`, skipping the Windows system directory
/// (`system_root`, normally `%SystemRoot%`) so we never resolve WSL's
/// `System32\bash.exe`. PATH is parsed with `std::env::split_paths` (never a
/// hand-split on ';') so it matches exactly what the spawned child would see.
#[cfg(windows)]
fn scan_path_for_bash(path_env: &str, system_root: Option<PathBuf>) -> Option<PathBuf> {
    scan_path_for_command(Path::new("bash.exe"), path_env, system_root.as_deref())
}

/// Scan `path_env` for `name` (or `name.exe` on Windows if `name` has no
/// extension), skipping any directory under `system_root` to avoid resolving
/// WSL helpers. Returns the first absolute path found.
#[cfg(windows)]
fn scan_path_for_command(
    name: &Path,
    path_env: &str,
    system_root: Option<&Path>,
) -> Option<PathBuf> {
    let needs_exe = name.extension().is_none();
    for dir in std::env::split_paths(path_env) {
        if let Some(root) = system_root {
            if is_under_dir(&dir, root) {
                continue;
            }
        }
        // Try as-is first.
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        // On Windows, also try with .exe suffix when the name has no extension.
        if needs_exe {
            let mut with_exe = dir.join(name);
            with_exe.set_extension("exe");
            if with_exe.is_file() {
                return Some(with_exe);
            }
        }
    }
    None
}

#[cfg(unix)]
fn set_process_group(cmd: &mut Command) {
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn set_process_group(_cmd: &mut Command) {}

/// Kill primitive covering the spawned bash AND every descendant it forks,
/// mirroring the same guarantee across platforms.
///
/// - Unix: the child's process group (set via [`set_process_group`]); kills go
///   to the whole group via `killpg`.
/// - Windows: a Job Object the child is assigned to at construction. A bare
///   `TerminateProcess` on bash leaves MSYS-forked grandchildren (e.g. `sleep`)
///   running — they hold the stdout/stderr pipes open, so the reap blocks until
///   they self-exit. Terminating the job kills the entire tree atomically.
///
/// Held for the whole `run`; `Drop` is the last-resort reaper if an explicit
/// kill was skipped or failed.
#[cfg(unix)]
struct KillGroup(Option<i32>);

#[cfg(unix)]
impl KillGroup {
    fn new(_child: &tokio::process::Child, pid: Option<u32>) -> Self {
        Self(pid.map(|p| p as i32))
    }

    /// Immediate SIGKILL of the process group. Sync; safe to call from Drop.
    /// No grace period — used when the parent task is being torn down.
    fn kill_immediate(&self) {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        if let Some(pid) = self.0 {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
    }

    /// Graceful SIGTERM → 200ms async sleep → SIGKILL. Async; never blocks the runtime.
    async fn kill_graceful(&self) {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        if let Some(pid) = self.0 {
            let pgid = Pid::from_raw(pid);
            let _ = killpg(pgid, Signal::SIGTERM);
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = killpg(pgid, Signal::SIGKILL);
        }
    }

    /// Disarm the Drop-time kill once the child has been reaped explicitly.
    fn disarm(&mut self) {
        self.0 = None;
    }
}

#[cfg(unix)]
impl Drop for KillGroup {
    fn drop(&mut self) {
        self.kill_immediate();
    }
}

#[cfg(windows)]
struct KillGroup {
    job: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: `job` is a raw Win32 HANDLE (`*mut c_void`), which is neither `Send`
// nor `Sync` by default. The shell tool's async future holds a `KillGroup`
// across an `.await`, so it must be `Send` to be spawned. A job-object handle
// is a kernel object reference, not thread-affine: `TerminateJobObject` and
// `CloseHandle` are thread-safe, and Rust's `&self`/`&mut self` borrows still
// serialize access to the field. Moving or sharing it across threads is sound.
#[cfg(windows)]
#[allow(unsafe_code)]
unsafe impl Send for KillGroup {}
#[cfg(windows)]
#[allow(unsafe_code)]
unsafe impl Sync for KillGroup {}

#[cfg(windows)]
#[allow(unsafe_code)]
impl KillGroup {
    fn new(child: &tokio::process::Child, _pid: Option<u32>) -> Self {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        // SAFETY: each call is a documented Win32 FFI call with arguments that
        // satisfy its contract — a null SECURITY_ATTRIBUTES/name for an
        // anonymous job, a zeroed #[repr(C)] info struct sized by size_of, and
        // the live process handle from `child` (valid while it is running).
        // A null job HANDLE on failure makes every later call a harmless no-op.
        let job = unsafe {
            let job: HANDLE = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !job.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                // KILL_ON_JOB_CLOSE: when the LAST handle to the job closes,
                // Windows kills every process still in it. This is both the
                // explicit-kill mechanism and the Drop-time safety net — and the
                // reason the job HANDLE must outlive the child (see Drop).
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if let Some(handle) = child.raw_handle() {
                    AssignProcessToJobObject(job, handle as HANDLE);
                }
            }
            job
        };
        Self { job }
    }

    fn kill_immediate(&self) {
        self.terminate();
    }

    async fn kill_graceful(&self) {
        // A Job Object has no SIGTERM analogue; termination is atomic, so the
        // graceful path is the same single terminate as the immediate path.
        self.terminate();
    }

    fn terminate(&self) {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if !self.job.is_null() {
            // SAFETY: `self.job` is a valid job HANDLE for this struct's
            // lifetime; exit code 137 mirrors the SIGKILL (128+9) we report on
            // Unix.
            unsafe {
                TerminateJobObject(self.job, 137);
            }
        }
    }

    /// No-op on Windows: the job is terminated explicitly, and closing the
    /// handle on Drop with no live processes left is harmless. Kept for a
    /// uniform call shape with the Unix guard.
    fn disarm(&mut self) {}
}

#[cfg(windows)]
#[allow(unsafe_code)]
impl Drop for KillGroup {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        if !self.job.is_null() {
            // Closing the last job handle triggers KILL_ON_JOB_CLOSE, killing any
            // process still in the job — the last-resort reaper. The handle is
            // held until here precisely so this fires no earlier than run end.
            // SAFETY: `self.job` is a valid HANDLE created in `new` and closed
            // exactly once here.
            unsafe {
                CloseHandle(self.job);
            }
        }
    }
}

// Fallback for targets that are neither unix nor windows: no process-tree kill
// primitive is wired up, so timeouts rely on the cross-platform start_kill in
// `run`. Keeps the crate compiling everywhere.
#[cfg(not(any(unix, windows)))]
struct KillGroup;

#[cfg(not(any(unix, windows)))]
impl KillGroup {
    fn new(_child: &tokio::process::Child, _pid: Option<u32>) -> Self {
        Self
    }
    fn kill_immediate(&self) {}
    async fn kill_graceful(&self) {}
    fn disarm(&mut self) {}
}

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
                // Short sleep, not 999: the kill path must actually terminate
                // the process tree on timeout. If a regression leaves the child
                // (or an MSYS grandchild) orphaned, the test stalls until this
                // brief sleep self-exits — ~5s, not ~16min — so the failure
                // stays visible instead of hiding behind a 999s sleep.
                command: "sleep 5".into(),
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

#[cfg(all(test, windows))]
mod windows_resolver_tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;
    use tempfile::tempdir;

    // Process-global env mutation guard: tests that mutate BUZZ_SHELL,
    // SystemRoot, or GIT_BASH must hold this lock for the duration of the
    // test so parallel test threads cannot race on these env vars.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, b"").expect("touch");
    }

    #[test]
    fn buzz_shell_override_wins_over_everything() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        // BUZZ_SHELL pointing at a real file must be returned without probing
        // the standard Git-for-Windows locations or PATH.
        let dir = tempdir().expect("tempdir");
        let fake_bash = dir.path().join("my-bash.exe");
        touch(&fake_bash);
        // Temporarily set BUZZ_SHELL; clean up after the test.
        env::set_var("BUZZ_SHELL", &fake_bash);
        let result = resolve_bash("");
        env::remove_var("BUZZ_SHELL");
        let (resolved, _name) = result.expect("BUZZ_SHELL override should resolve");
        assert_eq!(resolved, fake_bash);
    }

    #[test]
    fn buzz_shell_override_skipped_when_path_absent() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        // If BUZZ_SHELL points at a non-existent path the resolver must fall
        // through rather than returning a dead path.
        env::set_var("BUZZ_SHELL", r"C:\does\not\exist\bash.exe");
        // We cannot easily assert the fallback here without a full Git install,
        // but we can assert the override itself is not returned.
        let result = resolve_bash("");
        env::remove_var("BUZZ_SHELL");
        if let Ok((resolved, _)) = result {
            assert_ne!(
                resolved.to_str().unwrap_or(""),
                r"C:\does\not\exist\bash.exe",
                "non-existent BUZZ_SHELL must not be returned"
            );
        }
        // An Err is also acceptable (no Git installed on test host).
    }

    /// Explicit BUZZ_SHELL bare name resolves through PATH and uses NO System32
    /// exclusion — cmd/pwsh live in System32 legitimately.
    #[test]
    fn buzz_shell_explicit_bare_name_resolves_from_system32() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        // Simulate cmd.exe living in a dir that would be excluded by the WSL guard.
        // The explicit BUZZ_SHELL branch must NOT skip System32.
        let sys32 = tempdir().expect("sys32");
        let fake_cmd = sys32.path().join("cmd.exe");
        touch(&fake_cmd);

        // Build a path_env with only sys32 (the WSL exclusion would skip this dir
        // for bash.exe, but must NOT skip it for an explicit BUZZ_SHELL).
        let path_env = env::join_paths([sys32.path().to_path_buf()]).expect("join");
        env::set_var("BUZZ_SHELL", "cmd");
        // Override SystemRoot so the exclusion would trigger on sys32 if applied.
        let old_sysroot = env::var_os("SystemRoot");
        env::set_var("SystemRoot", sys32.path());

        let result = resolve_bash(path_env.to_str().expect("utf8"));

        env::remove_var("BUZZ_SHELL");
        match old_sysroot {
            Some(v) => env::set_var("SystemRoot", v),
            None => env::remove_var("SystemRoot"),
        }

        let (resolved, name) =
            result.expect("explicit BUZZ_SHELL=cmd should resolve even from System32-like dir");
        assert_eq!(resolved, fake_cmd);
        assert_eq!(name, "cmd");
    }

    /// Implicit bash.exe scan still skips System32 (WSL guard intact).
    #[test]
    fn implicit_bash_scan_still_skips_system32() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        // Same setup: bash.exe is only in a dir that is under SystemRoot.
        // Without an explicit BUZZ_SHELL, the fallback scan must skip it.
        let sys32 = tempdir().expect("sys32");
        touch(&sys32.path().join("bash.exe"));

        let path_env = env::join_paths([sys32.path().to_path_buf()]).expect("join");
        // No BUZZ_SHELL — trigger the implicit bash fallback scan.
        env::remove_var("BUZZ_SHELL");
        env::remove_var("GIT_BASH");
        // Point SystemRoot at sys32's parent so sys32 is "under SystemRoot".
        let parent = sys32.path().parent().unwrap().to_path_buf();
        let old_sysroot = env::var_os("SystemRoot");
        env::set_var("SystemRoot", &parent);

        let result = resolve_bash(path_env.to_str().expect("utf8"));

        match old_sysroot {
            Some(v) => env::set_var("SystemRoot", v),
            None => env::remove_var("SystemRoot"),
        }

        // Should be Err (no Git installed on test host, and the only bash.exe was
        // under SystemRoot so it was skipped). Ok is also acceptable if git bash
        // happens to be installed at the fixed Program Files path — we just assert
        // the System32 bash was NOT returned.
        if let Ok((resolved, _)) = result {
            assert!(
                !resolved.starts_with(sys32.path()),
                "implicit bash scan must not return the System32 bash: {resolved:?}"
            );
        }
    }

    /// The bootstrap hint (resolved_shell field) and the spawn path are the
    /// same object — both come from SharedState.resolved_shell.
    /// Verify that constructing SharedState with BUZZ_SHELL set produces a
    /// resolved_shell whose display name appears in bootstrap_instructions.
    #[test]
    fn shared_state_bootstrap_hint_matches_resolved_shell() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempdir().expect("tempdir");
        let fake_pwsh = dir.path().join("pwsh.exe");
        touch(&fake_pwsh);
        env::set_var("BUZZ_SHELL", &fake_pwsh);

        let shim = crate::shim::Shim::install().expect("shim");
        let state = SharedState::new(dir.path().to_path_buf(), shim).expect("state");

        env::remove_var("BUZZ_SHELL");

        let (_, name) = state.resolved_shell.as_ref().expect("resolved ok");
        assert_eq!(name, "pwsh");
        assert!(
            state.bootstrap_instructions.contains("pwsh"),
            "bootstrap must mention the resolved shell name"
        );
    }

    /// F3: BUZZ_SHELL bare command name (e.g. "pwsh") resolved through PATH.
    /// When pwsh.exe is on PATH, resolve_bash must return it and report "pwsh".
    #[test]
    fn buzz_shell_bare_name_resolved_through_path_when_present() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempdir().expect("tempdir");
        let fake_pwsh = dir.path().join("pwsh.exe");
        touch(&fake_pwsh);

        let path_env = env::join_paths([dir.path().to_path_buf()]).expect("join");
        env::set_var("BUZZ_SHELL", "pwsh");
        let result = resolve_bash(path_env.to_str().expect("utf8"));
        env::remove_var("BUZZ_SHELL");

        let (resolved, name) = result.expect("bare BUZZ_SHELL=pwsh should resolve from PATH");
        assert_eq!(resolved, fake_pwsh, "should resolve to pwsh.exe on PATH");
        assert_eq!(name, "pwsh", "display name must match resolved shell");
    }

    /// F3: BUZZ_SHELL bare command name absent from PATH → fall through, do not
    /// report pwsh as the active shell.
    #[test]
    fn buzz_shell_bare_name_absent_from_path_falls_through() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        // Set BUZZ_SHELL to a command that won't be on any real PATH.
        env::set_var("BUZZ_SHELL", "buzz-shell-does-not-exist-xyz");
        let result = resolve_bash("");
        env::remove_var("BUZZ_SHELL");
        if let Ok((resolved, name)) = result {
            assert_ne!(
                resolved.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                "buzz-shell-does-not-exist-xyz.exe",
                "absent BUZZ_SHELL must not be returned as the resolved path"
            );
            assert_ne!(
                name, "buzz-shell-does-not-exist-xyz",
                "absent BUZZ_SHELL must not be reported as the shell name"
            );
        }
        // Err is also acceptable (no Git on test host).
    }

    #[test]
    fn path_scan_skips_system32_and_returns_absolute() {
        // A bash.exe under %SystemRoot% (where WSL's launcher lives) must be
        // skipped; a bash.exe elsewhere on PATH is returned as an absolute path.
        let sys_root = tempdir().expect("sysroot");
        let real = tempdir().expect("real");
        touch(&sys_root.path().join("System32").join("bash.exe"));
        let real_bash = real.path().join("bash.exe");
        touch(&real_bash);

        let path_env =
            env::join_paths([sys_root.path().join("System32"), real.path().to_path_buf()])
                .expect("join");

        let found = scan_path_for_bash(
            path_env.to_str().expect("utf8"),
            Some(sys_root.path().to_path_buf()),
        )
        .expect("bash found outside System32");
        assert!(found.is_absolute());
        assert!(!found.starts_with(sys_root.path()));
        assert_eq!(found, real_bash);
    }

    #[test]
    fn path_scan_returns_none_when_only_system32_has_bash() {
        // If the ONLY bash.exe on PATH is under System32, the scan finds nothing.
        let sys_root = tempdir().expect("sysroot");
        touch(&sys_root.path().join("System32").join("bash.exe"));
        let path_env = env::join_paths([sys_root.path().join("System32")]).expect("join");

        let found = scan_path_for_bash(
            path_env.to_str().expect("utf8"),
            Some(sys_root.path().to_path_buf()),
        );
        assert!(found.is_none());
    }

    #[test]
    fn path_scan_skips_system32_when_path_case_differs_from_root() {
        // Windows paths are case-insensitive; a PATH entry spelled differently from
        // %SystemRoot% (e.g. `...\WINDOWS\System32` vs root `...\Windows`) must STILL
        // be excluded, or WSL's bash.exe leaks through. Build the System32 dir under a
        // genuinely upper-cased sibling component so the exclusion can only pass via a
        // case-insensitive compare, not a literal `starts_with`.
        let base = tempdir().expect("base");
        let root = base.path().join("Windows");
        let upper = base.path().join("WINDOWS");
        let sys32 = upper.join("System32");
        touch(&sys32.join("bash.exe"));

        let path_env = env::join_paths([sys32]).expect("join");
        let found = scan_path_for_bash(path_env.to_str().expect("utf8"), Some(root));
        assert!(
            found.is_none(),
            "case-divergent System32 must still be excluded"
        );
    }

    /// PATH-only discovery — a bash.exe custom-installed on PATH (not under
    /// the standard Program Files locations) must be found by the runtime
    /// resolver. This verifies the PATH fallback in resolve_bash: a custom
    /// install that lives outside Program Files is still usable as a shell.
    #[test]
    fn path_only_bash_is_found_by_scan() {
        // scan_path_for_bash is the runtime resolver's PATH fallback helper.
        // Verify it returns the bash.
        let real = tempdir().expect("real");
        let real_bash = real.path().join("bash.exe");
        touch(&real_bash);

        let path_env = env::join_paths([real.path().to_path_buf()]).expect("join");
        let sys_root = tempdir().expect("sysroot"); // empty — no System32 here

        let found = scan_path_for_bash(
            path_env.to_str().expect("utf8"),
            Some(sys_root.path().to_path_buf()),
        )
        .expect("bash on PATH must be found");
        assert_eq!(found, real_bash);
    }
}
