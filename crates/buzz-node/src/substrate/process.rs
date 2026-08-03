//! Process substrate: runs each workload body as a supervised child process.
//!
//! A deploy spawns the sprig ACP harness (`buzz-acp`) with the same
//! environment contract the desktop launcher uses, derived entirely from the
//! encrypted [`WorkloadSpec`] plus this node's own operator environment. The
//! managed agent's private key exists only in an in-memory map and in the
//! child's environment — never on disk, never in logs, never in receipts.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buzz_core::execution::{SafeErrorCode, WorkloadId, WorkloadSpec};
use tokio::sync::{mpsc, Mutex, Notify};
use tracing::warn;
use zeroize::Zeroizing;

use super::{env, Substrate, SubstrateError, WorkloadExit};

/// Harness binary a workload body runs inside (a sprig personality).
const HARNESS_BINARY: &str = "buzz-acp";

/// Bounded wait for a body to die after the SIGKILL escalation.
const KILL_WAIT: Duration = Duration::from_secs(5);

/// Environment variables copied from the node's own environment into every
/// workload body, on top of the shared provider-credential allowlist
/// ([`env::PROVIDER_ENV`]).
///
/// This is a deliberate allowlist, not blanket inheritance: the minimum a
/// harness and its agent need to resolve tools, use a home directory, and
/// speak TLS. Anything not listed here stays on the node.
const INHERITED_ENV: &[&str] = &[
    // Process basics.
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "TERM",
    // TLS trust stores in hardened environments.
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    // Harness logging.
    "RUST_LOG",
];

/// Configuration for the process substrate.
#[derive(Debug, Clone)]
pub struct ProcessSubstrateConfig {
    /// Node data directory. Per-workload scratch dirs live under
    /// `workloads/<workload-id>/` and body logs under `logs/<workload-id>.log`.
    pub data_dir: PathBuf,
    /// Relay the node itself is connected to — the fallback relay for bodies
    /// whose agent context does not carry one.
    pub relay_url: String,
    /// Explicit harness binary override. When absent the substrate looks for
    /// `buzz-acp` next to the node executable, then on `PATH`.
    pub harness_path: Option<PathBuf>,
    /// Grace period between SIGTERM and the SIGKILL escalation when stopping
    /// a body.
    pub graceful_stop: Duration,
}

impl ProcessSubstrateConfig {
    /// Build a configuration with default stop behavior and harness lookup.
    pub fn new(data_dir: PathBuf, relay_url: impl Into<String>) -> Self {
        Self {
            data_dir,
            relay_url: relay_url.into(),
            harness_path: None,
            graceful_stop: Duration::from_secs(10),
        }
    }
}

/// Owner-scoped substrate identity of one workload.
type BodyKey = (String, WorkloadId);

/// In-memory record for one deployed workload.
///
/// The durable spec lives in the ledger and is handed back on start/restart;
/// the substrate keeps only what must never touch disk (the launch key) and
/// the live body handle.
struct WorkloadEntry {
    /// One-time launch key. Memory only — never persisted or logged; gone
    /// after a node restart, at which point only a redeploy can revive the
    /// workload.
    key: Zeroizing<String>,
    /// The live body, when one is running.
    body: Option<BodyHandle>,
}

impl fmt::Debug for WorkloadEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkloadEntry")
            .field("key", &"[redacted]")
            .field("body", &self.body)
            .finish()
    }
}

/// Shared view of one spawned body, used by stop/restart and the exit watcher.
#[derive(Debug, Clone)]
struct BodyHandle {
    /// Monotonic spawn generation, so a stale watcher cannot clear the slot
    /// of a replacement body.
    generation: u64,
    /// OS process id (also the process-group id; bodies get their own group).
    pid: Option<u32>,
    /// Set before the substrate kills a body so the watcher does not report a
    /// substrate-initiated exit as a self-exit.
    expected_exit: Arc<AtomicBool>,
    /// Notified by the watcher once the body has been reaped.
    exited: Arc<Notify>,
    /// Flag the watcher raises before notifying, closing the wakeup race.
    exit_observed: Arc<AtomicBool>,
}

/// Substrate that runs each workload body as a supervised child process.
#[derive(Debug)]
pub struct ProcessSubstrate {
    config: ProcessSubstrateConfig,
    entries: Arc<Mutex<HashMap<BodyKey, WorkloadEntry>>>,
    exit_tx: mpsc::UnboundedSender<WorkloadExit>,
    generations: AtomicU64,
}

impl ProcessSubstrate {
    /// Create the substrate and the channel on which it reports bodies that
    /// exited on their own (never exits the substrate caused itself).
    pub fn new(config: ProcessSubstrateConfig) -> (Self, mpsc::UnboundedReceiver<WorkloadExit>) {
        let (exit_tx, exit_rx) = mpsc::unbounded_channel();
        (
            Self {
                config,
                entries: Arc::new(Mutex::new(HashMap::new())),
                exit_tx,
                generations: AtomicU64::new(0),
            },
            exit_rx,
        )
    }

    fn workload_dir(&self, workload_id: &WorkloadId) -> PathBuf {
        self.config
            .data_dir
            .join("workloads")
            .join(workload_id.as_str())
    }

    fn log_path(&self, workload_id: &WorkloadId) -> PathBuf {
        self.config
            .data_dir
            .join("logs")
            .join(format!("{}.log", workload_id.as_str()))
    }

    /// Resolve the ACP harness this substrate spawns: explicit override
    /// first, then a `buzz-acp` sibling of the node executable, then `PATH`.
    fn resolve_harness(&self) -> Result<PathBuf, SubstrateError> {
        if let Some(path) = &self.config.harness_path {
            if path.is_file() {
                return Ok(path.clone());
            }
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeUnavailable,
                format!("configured harness {} does not exist", path.display()),
            ));
        }
        resolve_executable(HARNESS_BINARY).ok_or_else(|| {
            SubstrateError::new(
                SafeErrorCode::RuntimeUnavailable,
                "no `buzz-acp` harness found next to buzz-node or on PATH; \
                 install sprig or set --harness-path / BUZZ_NODE_HARNESS_PATH",
            )
        })
    }

    /// Gracefully stop the live body for `key`, if any: SIGTERM to its
    /// process group, a bounded wait, then SIGKILL. The exit is marked as
    /// substrate-initiated so the watcher does not report a self-exit.
    async fn stop_body(&self, key: &BodyKey) -> Result<(), SubstrateError> {
        let handle = {
            let entries = self.entries.lock().await;
            match entries.get(key).and_then(|entry| entry.body.clone()) {
                Some(handle) => handle,
                None => return Ok(()),
            }
        };
        handle.expected_exit.store(true, Ordering::Release);
        // Register interest before checking the flag so a concurrent exit
        // cannot slip between the check and the wait.
        let terminated = handle.exited.notified();
        tokio::pin!(terminated);
        if handle.exit_observed.load(Ordering::Acquire) {
            return Ok(());
        }
        let Some(pid) = handle.pid else {
            // The process id was already reaped; the watcher will clear the
            // body slot momentarily.
            return Ok(());
        };
        signal_group(pid, false).await;
        if tokio::time::timeout(self.config.graceful_stop, &mut terminated)
            .await
            .is_ok()
        {
            return Ok(());
        }
        let killed = handle.exited.notified();
        tokio::pin!(killed);
        if handle.exit_observed.load(Ordering::Acquire) {
            return Ok(());
        }
        signal_group(pid, true).await;
        if tokio::time::timeout(KILL_WAIT, &mut killed).await.is_err()
            && !handle.exit_observed.load(Ordering::Acquire)
        {
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("workload body (pid {pid}) survived the kill escalation"),
            ));
        }
        Ok(())
    }

    /// Spawn the harness for one workload and supervise its exit.
    async fn spawn_body(
        &self,
        owner: &str,
        spec: &WorkloadSpec,
        launch_key: &Zeroizing<String>,
    ) -> Result<BodyHandle, SubstrateError> {
        let agent = spec.agent.as_ref().ok_or_else(|| {
            SubstrateError::new(
                SafeErrorCode::Unsupported,
                "the process substrate only runs managed-agent workloads",
            )
        })?;
        let harness = self.resolve_harness()?;
        let plan = resolve_runtime_plan(&spec.runtime)?;

        let workdir = self.workload_dir(&spec.workload_id);
        fs::create_dir_all(&workdir).map_err(|error| {
            SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("create workload directory: {error}"),
            )
        })?;
        let log_path = self.log_path(&spec.workload_id);
        if let Some(log_dir) = log_path.parent() {
            fs::create_dir_all(log_dir).map_err(|error| {
                SubstrateError::new(
                    SafeErrorCode::RuntimeFailed,
                    format!("create log directory: {error}"),
                )
            })?;
        }
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| {
                SubstrateError::new(
                    SafeErrorCode::RuntimeFailed,
                    format!("open body log file: {error}"),
                )
            })?;
        set_private_permissions(&log_path);
        let log_stderr = log.try_clone().map_err(|error| {
            SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("clone body log handle: {error}"),
            )
        })?;

        let mut command = tokio::process::Command::new(&harness);
        // Explicit allowlist instead of blanket inheritance — see
        // INHERITED_ENV and the shared provider-credential allowlist.
        command.env_clear();
        for name in INHERITED_ENV.iter().chain(env::PROVIDER_ENV) {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }

        // ── Shared harness contract, mirroring the desktop launcher
        //    (desktop/src-tauri/src/managed_agents/runtime.rs). The one-time
        //    key handoff lives only in the child environment. ───────────────
        let relay_url = agent
            .relay_url
            .clone()
            .unwrap_or_else(|| self.config.relay_url.clone());
        let launch = env::RuntimeLaunch {
            agent_command: &plan.agent_command,
            mcp_command: plan.mcp_command.as_deref(),
            default_env: plan.default_env,
            model_env: plan.model_env,
            provider_env: plan.provider_env,
        };
        for (name, value) in
            env::harness_environment(spec, agent, launch_key.as_str(), &relay_url, &launch)
        {
            command.env(name, value.as_str());
        }
        if plan.wants_claude_cli {
            if let Some(cli) = resolve_executable("claude") {
                command.env("CLAUDE_CODE_EXECUTABLE", cli);
            }
        }

        // ── Git credential helper for relay-hosted repos (NIP-98), same
        //    ephemeral GIT_CONFIG_* scheme the desktop uses. ────────────────
        if let Some(helper) = resolve_executable("git-credential-nostr") {
            let relay_http = relay_http_base_url(&relay_url);
            command.env("NOSTR_PRIVATE_KEY", launch_key.as_str());
            command.env("GIT_TERMINAL_PROMPT", "0");
            command.env("GIT_CONFIG_COUNT", "2");
            command.env(
                "GIT_CONFIG_KEY_0",
                format!("credential.{relay_http}/git.helper"),
            );
            command.env(
                "GIT_CONFIG_VALUE_0",
                helper.to_string_lossy().replace('\\', "/"),
            );
            command.env(
                "GIT_CONFIG_KEY_1",
                format!("credential.{relay_http}/git.useHttpPath"),
            );
            command.env("GIT_CONFIG_VALUE_1", "true");
        }

        command.current_dir(&workdir);
        command.stdin(Stdio::null());
        command.stdout(Stdio::from(log));
        command.stderr(Stdio::from(log_stderr));
        // Bodies get their own process group so stop can take down the whole
        // tree (harness + MCP servers + agent subprocesses).
        #[cfg(unix)]
        command.process_group(0);
        command.kill_on_drop(false);

        let mut child = command.spawn().map_err(|error| {
            SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("spawn harness {}: {error}", harness.display()),
            )
        })?;

        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let handle = BodyHandle {
            generation,
            pid: child.id(),
            expected_exit: Arc::new(AtomicBool::new(false)),
            exited: Arc::new(Notify::new()),
            exit_observed: Arc::new(AtomicBool::new(false)),
        };
        let watcher_handle = handle.clone();
        let entries = Arc::clone(&self.entries);
        let exit_tx = self.exit_tx.clone();
        let body_key: BodyKey = (owner.to_string(), spec.workload_id.clone());
        tokio::spawn(async move {
            let clean = match child.wait().await {
                Ok(status) => status.success(),
                Err(_) => false,
            };
            watcher_handle.exit_observed.store(true, Ordering::Release);
            watcher_handle.exited.notify_waiters();
            {
                let mut entries = entries.lock().await;
                if let Some(entry) = entries.get_mut(&body_key) {
                    if entry
                        .body
                        .as_ref()
                        .is_some_and(|body| body.generation == watcher_handle.generation)
                    {
                        entry.body = None;
                    }
                }
            }
            if !watcher_handle.expected_exit.load(Ordering::Acquire) {
                // The body exited on its own: it was finished, not killed.
                // Report it and never respawn ("Agents That Know When to
                // Leave").
                let _ = exit_tx.send(WorkloadExit {
                    owner: body_key.0,
                    workload_id: body_key.1,
                    clean,
                });
            }
        });
        Ok(handle)
    }

    fn missing_key_error() -> SubstrateError {
        SubstrateError::new(
            SafeErrorCode::RuntimeUnavailable,
            "no launch key for this workload is held in memory (the node restarted \
             since the last deploy); redeploy the agent from Desktop",
        )
    }
}

#[async_trait]
impl Substrate for ProcessSubstrate {
    async fn deploy(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload.workload_id.clone());
        let agent = workload.agent.as_ref().ok_or_else(|| {
            SubstrateError::new(
                SafeErrorCode::Unsupported,
                "the process substrate only runs managed-agent workloads",
            )
        })?;
        // One-time key handoff: prefer the key in this deploy, falling back
        // to one already held in memory from a previous deploy of the same
        // workload.
        let launch_key: Zeroizing<String> = match agent.private_key_nsec.clone() {
            Some(nsec) => Zeroizing::new(nsec),
            None => {
                let entries = self.entries.lock().await;
                entries
                    .get(&key)
                    .map(|entry| entry.key.clone())
                    .ok_or_else(|| {
                        SubstrateError::new(
                            SafeErrorCode::InvalidCommand,
                            "deploy carries no launch key and none is held in memory; \
                             redeploy the agent from Desktop",
                        )
                    })?
            }
        };
        // Converge to a single live body no matter how deploys race: any
        // existing body is taken down before the replacement spawns.
        self.stop_body(&key).await?;
        let body = self.spawn_body(owner, workload, &launch_key).await?;
        let mut entries = self.entries.lock().await;
        entries.insert(
            key,
            WorkloadEntry {
                key: launch_key,
                body: Some(body),
            },
        );
        Ok(())
    }

    async fn start(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload.workload_id.clone());
        let launch_key = {
            let entries = self.entries.lock().await;
            let Some(entry) = entries.get(&key) else {
                // Fail closed: a ledger-Running workload whose key is gone
                // (node restart) cannot be started from the node alone.
                return Err(Self::missing_key_error());
            };
            if entry.body.is_some() {
                // Idempotent: one live body already exists.
                return Ok(());
            }
            entry.key.clone()
        };
        let body = self.spawn_body(owner, workload, &launch_key).await?;
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get_mut(&key) {
            entry.body = Some(body);
        }
        Ok(())
    }

    async fn stop(&self, owner: &str, workload_id: &WorkloadId) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload_id.clone());
        self.stop_body(&key).await
    }

    async fn restart(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload.workload_id.clone());
        let launch_key = {
            let entries = self.entries.lock().await;
            let Some(entry) = entries.get(&key) else {
                // Fail closed, same as start: a respawn needs the in-memory key.
                return Err(Self::missing_key_error());
            };
            entry.key.clone()
        };
        self.stop_body(&key).await?;
        let body = self.spawn_body(owner, workload, &launch_key).await?;
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get_mut(&key) {
            entry.body = Some(body);
        }
        Ok(())
    }

    async fn remove(&self, owner: &str, workload_id: &WorkloadId) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload_id.clone());
        self.stop_body(&key).await?;
        // Dropping the entry zeroizes the launch key.
        drop(self.entries.lock().await.remove(&key));
        let workdir = self.workload_dir(workload_id);
        if workdir.exists() {
            if let Err(error) = fs::remove_dir_all(&workdir) {
                // Scratch cleanup is best-effort; the workload itself is gone.
                warn!(workload = workload_id.as_str(), %error, "failed to clear workload scratch directory");
            }
        }
        Ok(())
    }
}

/// Per-runtime launch details, resolved from the shared runtime catalog
/// ([`env::known_runtime`]) to host executable paths. Unknown runtime
/// identifiers are attempted verbatim as a command name so custom harness
/// setups keep working; if nothing resolves the deploy fails.
#[derive(Debug)]
struct RuntimeLaunchPlan {
    /// Resolved inner agent command the harness runs (`BUZZ_ACP_AGENT_COMMAND`).
    agent_command: String,
    /// Resolved developer MCP command, when the runtime uses one.
    mcp_command: Option<String>,
    /// Runtime-specific defaults, e.g. Goose's non-interactive mode.
    default_env: &'static [(&'static str, &'static str)],
    /// Env var the runtime reads its model from, when it has one.
    model_env: Option<&'static str>,
    /// Env var the runtime reads its provider from, when it is not locked.
    provider_env: Option<&'static str>,
    /// Whether to point the Claude adapter at a resolved `claude` CLI.
    wants_claude_cli: bool,
}

fn resolve_runtime_plan(runtime: &str) -> Result<RuntimeLaunchPlan, SubstrateError> {
    let normalized = runtime.trim().to_ascii_lowercase();
    let known = env::known_runtime(&normalized);
    // Unknown runtime identifiers are attempted verbatim as a command name.
    let command = known.as_ref().map_or(runtime, |known| known.command);
    let env::KnownRuntime {
        mcp,
        default_env,
        model_env,
        provider_env,
        wants_claude_cli,
        ..
    } = known.unwrap_or(env::UNKNOWN_RUNTIME);
    let agent_command = resolve_executable(command)
        .map(|path| path.display().to_string())
        .ok_or_else(|| {
            SubstrateError::new(
                SafeErrorCode::Unsupported,
                format!(
                    "runtime {runtime:?} is not available on this node \
                     (command {command:?} not found next to buzz-node or on PATH)"
                ),
            )
        })?;
    // A missing MCP helper degrades gracefully, matching the desktop launcher.
    let mcp_command = mcp
        .and_then(resolve_executable)
        .map(|path| path.display().to_string());
    Ok(RuntimeLaunchPlan {
        agent_command,
        mcp_command,
        default_env,
        model_env,
        provider_env,
        wants_claude_cli,
    })
}

/// Resolve a command to an executable path: explicit paths as-is, then a
/// sibling of the current executable (sprig multicall installs), then `PATH`.
fn resolve_executable(command: &str) -> Option<PathBuf> {
    let as_path = Path::new(command);
    if as_path.components().count() > 1 {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }
    let file_name = format!("{command}{}", std::env::consts::EXE_SUFFIX);
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(&file_name);
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(&file_name))
        .find(|candidate| candidate.is_file())
}

/// Derive the relay's HTTP base URL from its WebSocket URL for the git
/// credential-helper scope.
fn relay_http_base_url(relay_url: &str) -> String {
    let trimmed = relay_url.trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        trimmed.to_string()
    }
}

/// Signal a body's whole process group. `lethal` escalates to SIGKILL.
///
/// Uses the system `kill`/`taskkill` utilities so the crate stays free of
/// `unsafe` signal FFI. Failures are best-effort — the caller's bounded wait
/// on the exit watcher decides whether the stop ultimately failed.
async fn signal_group(pid: u32, lethal: bool) {
    #[cfg(unix)]
    {
        let signal = if lethal { "KILL" } else { "TERM" };
        let _ = tokio::process::Command::new("kill")
            .args(["-s", signal, "--"])
            .arg(format!("-{pid}"))
            .status()
            .await;
    }
    #[cfg(not(unix))]
    {
        // Windows has no graceful console signal for detached children; both
        // phases force-terminate the tree.
        let _ = lethal;
        let _ = tokio::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID"])
            .arg(pid.to_string())
            .status()
            .await;
    }
}

fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use buzz_core::execution::AgentWorkloadContext;
    use nostr::{Keys, ToBech32};
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let counter = TEMP_DIR_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let dir = std::env::temp_dir().join(format!("buzz-node-substrate-{suffix}-{counter}"));
        fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("chmod script");
        path
    }

    fn agent_spec(nsec: Option<&str>, pubkey: &str) -> WorkloadSpec {
        let mut agent =
            AgentWorkloadContext::new(pubkey.to_string(), None, None, None, None, Vec::new(), None)
                .expect("agent context");
        if let Some(nsec) = nsec {
            agent = agent.with_private_key(nsec).expect("attach key");
        }
        let mut spec = WorkloadSpec::agent(
            WorkloadId::random(),
            "Process substrate test agent",
            // `sh` is not a known runtime; the plan resolves it on PATH,
            // keeping the test hermetic.
            "sh",
            None,
            None,
            Vec::new(),
        )
        .expect("workload spec");
        spec.agent = Some(agent);
        spec
    }

    fn substrate_with_harness(
        dir: &Path,
        harness: PathBuf,
    ) -> (ProcessSubstrate, mpsc::UnboundedReceiver<WorkloadExit>) {
        let mut config =
            ProcessSubstrateConfig::new(dir.to_path_buf(), "ws://relay.example".to_string());
        config.harness_path = Some(harness);
        config.graceful_stop = Duration::from_millis(300);
        ProcessSubstrate::new(config)
    }

    fn generated_agent_key() -> (String, String) {
        let keys = Keys::generate();
        (
            keys.secret_key().to_bech32().expect("nsec"),
            keys.public_key().to_hex(),
        )
    }

    #[tokio::test]
    async fn self_exit_is_reported_with_exit_status_and_key_stays_out_of_debug_output() {
        let dir = temp_dir();
        let harness = write_script(&dir, "clean-exit.sh", "exit 0");
        let (substrate, mut exits) = substrate_with_harness(&dir, harness);
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);

        substrate.deploy("owner", &spec).await.expect("deploy");
        assert!(
            !format!("{substrate:?}").contains(&nsec),
            "launch key must never appear in diagnostics"
        );
        let exit = tokio::time::timeout(Duration::from_secs(10), exits.recv())
            .await
            .expect("exit before timeout")
            .expect("exit event");
        assert_eq!(exit.workload_id, spec.workload_id);
        assert_eq!(exit.owner, "owner");
        assert!(exit.clean);

        let failing = write_script(&dir, "failing-exit.sh", "exit 3");
        let (substrate, mut exits) = substrate_with_harness(&dir, failing);
        let spec = agent_spec(Some(&nsec), &pubkey);
        substrate.deploy("owner", &spec).await.expect("deploy");
        let exit = tokio::time::timeout(Duration::from_secs(10), exits.recv())
            .await
            .expect("exit before timeout")
            .expect("exit event");
        assert!(!exit.clean);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn stop_kills_the_body_without_reporting_a_self_exit() {
        let dir = temp_dir();
        let harness = write_script(&dir, "long-runner.sh", "sleep 30");
        let (substrate, mut exits) = substrate_with_harness(&dir, harness);
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);

        substrate.deploy("owner", &spec).await.expect("deploy");
        substrate
            .stop("owner", &spec.workload_id)
            .await
            .expect("stop");
        // A substrate-initiated exit must not surface on the self-exit channel.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(exits.try_recv().is_err());
        // Stop is idempotent when nothing is running.
        substrate
            .stop("owner", &spec.workload_id)
            .await
            .expect("stop again");
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn start_without_an_in_memory_key_fails_closed() {
        let dir = temp_dir();
        let harness = write_script(&dir, "unused.sh", "exit 0");
        let (substrate, _exits) = substrate_with_harness(&dir, harness);
        let (_, pubkey) = generated_agent_key();
        // The durable ledger spec never carries a key — exactly what a node
        // restart leaves behind.
        let spec = agent_spec(None, &pubkey);

        let error = substrate
            .start("owner", &spec)
            .await
            .expect_err("start must fail closed");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        assert!(error.message.contains("redeploy"));

        let error = substrate
            .restart("owner", &spec)
            .await
            .expect_err("restart must fail closed");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn missing_harness_fails_deploy_with_runtime_unavailable() {
        let dir = temp_dir();
        let (substrate, _exits) = substrate_with_harness(&dir, dir.join("missing-harness"));
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);

        let error = substrate
            .deploy("owner", &spec)
            .await
            .expect_err("deploy must fail without a harness");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn remove_drops_the_key_and_clears_the_scratch_directory() {
        let dir = temp_dir();
        let harness = write_script(&dir, "long-runner.sh", "sleep 30");
        let (substrate, _exits) = substrate_with_harness(&dir, harness);
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);

        substrate.deploy("owner", &spec).await.expect("deploy");
        let workdir = substrate.workload_dir(&spec.workload_id);
        assert!(workdir.exists());
        substrate
            .remove("owner", &spec.workload_id)
            .await
            .expect("remove");
        assert!(!workdir.exists());
        // With the key dropped, a start must fail closed.
        let error = substrate
            .start("owner", &spec.clone().without_private_key())
            .await
            .expect_err("start after remove");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        // Remove is idempotent.
        substrate
            .remove("owner", &spec.workload_id)
            .await
            .expect("remove again");
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deploy_replaces_a_running_body_instead_of_running_two() {
        let dir = temp_dir();
        let harness = write_script(&dir, "long-runner.sh", "sleep 30");
        let (substrate, mut exits) = substrate_with_harness(&dir, harness);
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);

        substrate.deploy("owner", &spec).await.expect("deploy");
        let first_pid = {
            let entries = substrate.entries.lock().await;
            entries
                .get(&("owner".to_string(), spec.workload_id.clone()))
                .and_then(|entry| entry.body.as_ref())
                .and_then(|body| body.pid)
                .expect("first body pid")
        };
        // A redeploy without a key reuses the in-memory key and replaces the
        // body (kill-before-spawn).
        let redeploy = spec.clone().without_private_key();
        substrate
            .deploy("owner", &redeploy)
            .await
            .expect("redeploy");
        let second_pid = {
            let entries = substrate.entries.lock().await;
            entries
                .get(&("owner".to_string(), spec.workload_id.clone()))
                .and_then(|entry| entry.body.as_ref())
                .and_then(|body| body.pid)
                .expect("replacement body pid")
        };
        assert_ne!(first_pid, second_pid);
        // The replaced body's death was substrate-initiated: no self-exit event.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(exits.try_recv().is_err());
        substrate
            .remove("owner", &spec.workload_id)
            .await
            .expect("cleanup");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn relay_http_base_url_maps_ws_schemes() {
        assert_eq!(
            relay_http_base_url("wss://relay.example/"),
            "https://relay.example"
        );
        assert_eq!(
            relay_http_base_url("ws://localhost:3000"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn unknown_runtime_that_does_not_resolve_fails_with_unsupported() {
        let error = resolve_runtime_plan("definitely-not-a-real-runtime-binary")
            .expect_err("unresolvable runtime");
        assert_eq!(error.code, SafeErrorCode::Unsupported);
    }
}
