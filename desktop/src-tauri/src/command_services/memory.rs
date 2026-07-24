use crate::command_services::ssh::{
    start_tunnel_with_reservation, validate_host_target, PinnedHostEvidence, ProtectedFile,
    ReservedLoopbackPort, SshError, SshTunnel, SshTunnelConfig,
};
use crate::secret_store::SecretStore;
use base64::Engine;
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::Manager;

const CONFIG_FILE_NAME: &str = "command-memory.json";
const SSH_BINARY: &str = "/usr/bin/ssh";
const READINESS_TIMEOUT: Duration = Duration::from_secs(3);
const TUNNEL_STARTUP_TIMEOUT: Duration = Duration::from_secs(8);
const REPLICATION_TIMEOUT: Duration = Duration::from_secs(90);
const MAXIMUM_CONFIG_BYTES: u64 = 64 * 1024;
const MAXIMUM_EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;
const MAXIMUM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAXIMUM_STDERR_BYTES: usize = 64 * 1024;
const PAGE_SIZE: &str = "50";
const CLI_REQUEST_TIMEOUT_SECONDS: &str = "30";
const EXACT_MEMORY_TOOLS: &[&str] = &[
    "get_entity",
    "get_wiki_page",
    "link_entities",
    "list_entities",
    "memory_graph",
    "recall_for_entity",
    "record_event",
    "search_events",
    "search_wiki",
    "timeline",
    "upsert_entity",
];
static SYNC_GATE: LazyLock<Arc<SyncGate>> = LazyLock::new(|| Arc::new(SyncGate::default()));
static SYNC_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryError {
    NotConfigured,
    InvalidConfig,
    CredentialsUnavailable,
    LocalServiceUnavailable,
    AuthenticationFailed,
    InvalidResponse,
    ResponseTooLarge,
    NodeIdentityMismatch,
    SshPinInvalid,
    SshUnavailable,
    Spawn,
    Timeout,
    Exit,
    Teardown,
    Busy,
    Task,
    Cancelled,
}

impl MemoryError {
    fn code(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::InvalidConfig => "invalid_config",
            Self::CredentialsUnavailable => "credentials_unavailable",
            Self::LocalServiceUnavailable => "local_service_unavailable",
            Self::AuthenticationFailed => "authentication_failed",
            Self::InvalidResponse => "invalid_response",
            Self::ResponseTooLarge => "response_too_large",
            Self::NodeIdentityMismatch => "node_identity_mismatch",
            Self::SshPinInvalid => "ssh_pin_invalid",
            Self::SshUnavailable => "ssh_unavailable",
            Self::Spawn => "spawn_failed",
            Self::Timeout => "timeout",
            Self::Exit => "operation_failed",
            Self::Teardown => "teardown_failed",
            Self::Busy => "sync_already_running",
            Self::Task => "task_failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl From<SshError> for MemoryError {
    fn from(error: SshError) -> Self {
        match error {
            SshError::HostFingerprintMismatch
            | SshError::InvalidHostAlias
            | SshError::InvalidKnownHosts
            | SshError::UnprotectedFile
            | SshError::InvalidConfiguration => Self::SshPinInvalid,
            SshError::Spawn | SshError::EarlyExit => Self::SshUnavailable,
            SshError::Teardown => Self::Teardown,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialKeys {
    local_read: String,
    local_replicate: String,
    remote_read: String,
    remote_replicate: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryConfig {
    schema_version: u32,
    local_port: u16,
    home_host_alias: String,
    home_user: String,
    pinned_host_fingerprint: String,
    known_hosts_path: PathBuf,
    identity_file: PathBuf,
    remote_loopback_port: u16,
    local_node_id: String,
    home_node_id: String,
    sync_interval_minutes: u32,
    tool_allowlist: Vec<String>,
    replicate_cli_path: PathBuf,
    credential_keys: CredentialKeys,
}

struct MemorySecrets {
    local_read: String,
    local_replicate: String,
    remote_read: String,
    remote_replicate: String,
}

#[cfg(test)]
impl MemorySecrets {
    fn fixture(
        local_read: &str,
        local_replicate: &str,
        remote_read: &str,
        remote_replicate: &str,
    ) -> Self {
        Self {
            local_read: local_read.to_string(),
            local_replicate: local_replicate.to_string(),
            remote_read: remote_read.to_string(),
            remote_replicate: remote_replicate.to_string(),
        }
    }
}

struct TrustedMemoryConfig {
    config: MemoryConfig,
    secrets: MemorySecrets,
}

struct ProtectedExecutable {
    file: ProtectedFile,
    original_path: PathBuf,
}

impl ProtectedExecutable {
    fn open(path: &Path) -> Result<Self, MemoryError> {
        let file = ProtectedFile::open(path, MAXIMUM_EXECUTABLE_BYTES)
            .map_err(|_| MemoryError::InvalidConfig)?;
        if file.mode() & 0o111 == 0 || file.mode() & 0o022 != 0 {
            return Err(MemoryError::InvalidConfig);
        }
        Ok(Self {
            file,
            original_path: path.to_path_buf(),
        })
    }

    fn command(&self) -> Result<(Command, bool), MemoryError> {
        let prefix = self
            .file
            .read_prefix(64)
            .map_err(|_| MemoryError::InvalidConfig)?;
        if prefix.starts_with(b"#!/bin/sh\n") {
            let mut command = Command::new("/bin/sh");
            command.args(["-s", "--"]).stdin(Stdio::from(
                self.file
                    .try_clone_file()
                    .map_err(|_| MemoryError::InvalidConfig)?,
            ));
            Ok((command, true))
        } else {
            Ok((Command::new(&self.original_path), false))
        }
    }

    fn revalidate_binary(&self) -> Result<(), MemoryError> {
        if self.file.matches_path(&self.original_path) {
            Ok(())
        } else {
            Err(MemoryError::InvalidConfig)
        }
    }

    #[cfg(test)]
    fn spawn_for_test(&self) -> Result<String, MemoryError> {
        let (mut command, stdin_bound) = self.command()?;
        if !stdin_bound {
            self.revalidate_binary()?;
        }
        let output = command
            .env_clear()
            .output()
            .map_err(|_| MemoryError::Spawn)?;
        if !output.status.success() {
            return Err(MemoryError::Exit);
        }
        String::from_utf8(output.stdout).map_err(|_| MemoryError::InvalidResponse)
    }
}

#[derive(Default)]
struct SyncGate {
    mutex: Mutex<()>,
}

impl SyncGate {
    fn try_enter(&self) -> Result<MutexGuard<'_, ()>, MemoryError> {
        match self.mutex.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::WouldBlock) => Err(MemoryError::Busy),
            Err(TryLockError::Poisoned(_)) => Err(MemoryError::Task),
        }
    }
}

#[derive(Default)]
struct SchedulerControl {
    stopped: Mutex<bool>,
    wake: Condvar,
}

pub(crate) struct MemorySyncScheduler {
    control: Arc<SchedulerControl>,
    thread: Mutex<Option<JoinHandle<()>>>,
    done: Mutex<Receiver<()>>,
}

impl MemorySyncScheduler {
    fn start<F>(interval: Duration, gate: Arc<SyncGate>, mut task: F) -> Self
    where
        F: FnMut() -> Result<(), MemoryError> + Send + 'static,
    {
        let control = Arc::new(SchedulerControl::default());
        let thread_control = Arc::clone(&control);
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || loop {
            let stopped = match thread_control.stopped.lock() {
                Ok(stopped) => stopped,
                Err(_) => break,
            };
            let waited = thread_control.wake.wait_timeout(stopped, interval);
            let Ok((stopped, _)) = waited else {
                break;
            };
            if *stopped {
                break;
            }
            drop(stopped);
            if let Ok(_guard) = gate.try_enter() {
                let _ = task();
            }
        });
        // The sender is moved into a small reaper so stop can enforce a
        // bounded wait without detaching a live sync silently.
        let thread = std::thread::spawn(move || {
            let _ = thread.join();
            let _ = done_sender.send(());
        });
        Self {
            control,
            thread: Mutex::new(Some(thread)),
            done: Mutex::new(done_receiver),
        }
    }

    #[cfg(test)]
    fn start_for_test<F>(interval: Duration, gate: Arc<SyncGate>, task: F) -> Self
    where
        F: FnMut() -> Result<(), MemoryError> + Send + 'static,
    {
        Self::start(interval, gate, task)
    }

    pub(crate) fn stop_and_join(&self) -> Result<(), MemoryError> {
        {
            let mut stopped = self.control.stopped.lock().map_err(|_| MemoryError::Task)?;
            *stopped = true;
            self.control.wake.notify_all();
        }
        let thread = self.thread.lock().map_err(|_| MemoryError::Task)?.take();
        if let Some(thread) = thread {
            self.done
                .lock()
                .map_err(|_| MemoryError::Task)?
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| MemoryError::Timeout)?;
            thread.join().map_err(|_| MemoryError::Task)?;
        }
        Ok(())
    }
}

trait CredentialSource {
    fn load(&self, key: &str) -> Result<Option<String>, String>;
}

impl CredentialSource for SecretStore {
    fn load(&self, key: &str) -> Result<Option<String>, String> {
        SecretStore::load(self, key)
    }
}

fn valid_node_id(value: &str) -> bool {
    value.starts_with("node:")
        && value.len() >= 7
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-'))
}

fn valid_credential_key(value: &str) -> bool {
    value.starts_with("memory.")
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_secret(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'\r' | b'\n'))
}

fn read_protected_config(path: &Path) -> Result<Vec<u8>, MemoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(MemoryError::NotConfigured)
        }
        Err(_) => return Err(MemoryError::InvalidConfig),
    }
    ProtectedFile::open(path, MAXIMUM_CONFIG_BYTES)
        .and_then(|file| file.read_all())
        .map_err(|_| MemoryError::InvalidConfig)
}

fn validate_config(config: &MemoryConfig) -> Result<(), MemoryError> {
    let unique_tools = config
        .tool_allowlist
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let credential_keys = [
        config.credential_keys.local_read.as_str(),
        config.credential_keys.local_replicate.as_str(),
        config.credential_keys.remote_read.as_str(),
        config.credential_keys.remote_replicate.as_str(),
    ];
    let fingerprint_digest = config
        .pinned_host_fingerprint
        .strip_prefix("SHA256:")
        .and_then(|value| {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(value)
                .ok()
        });
    if config.schema_version != 1
        || config.local_port == 0
        || config.remote_loopback_port == 0
        || config.local_node_id == config.home_node_id
        || !valid_node_id(&config.local_node_id)
        || !valid_node_id(&config.home_node_id)
        || !(5..=1440).contains(&config.sync_interval_minutes)
        || config.tool_allowlist.is_empty()
        || config.tool_allowlist.len() > EXACT_MEMORY_TOOLS.len()
        || unique_tools.len() != config.tool_allowlist.len()
        || config
            .tool_allowlist
            .iter()
            .any(|tool| !EXACT_MEMORY_TOOLS.contains(&tool.as_str()))
        || credential_keys.iter().any(|key| !valid_credential_key(key))
        || credential_keys
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
            != credential_keys.len()
        || fingerprint_digest
            .as_deref()
            .is_none_or(|digest| digest.len() != 32)
        || validate_host_target(&config.home_host_alias, &config.home_user).is_err()
    {
        return Err(MemoryError::InvalidConfig);
    }
    Ok(())
}

fn load_secret(source: &dyn CredentialSource, key: &str) -> Result<String, MemoryError> {
    let value = source
        .load(key)
        .map_err(|_| MemoryError::CredentialsUnavailable)?
        .ok_or(MemoryError::CredentialsUnavailable)?;
    if !valid_secret(&value) {
        return Err(MemoryError::CredentialsUnavailable);
    }
    Ok(value)
}

fn load_trusted_config(
    path: &Path,
    credentials: &dyn CredentialSource,
) -> Result<TrustedMemoryConfig, MemoryError> {
    let bytes = read_protected_config(path)?;
    let config: MemoryConfig =
        serde_json::from_slice(&bytes).map_err(|_| MemoryError::InvalidConfig)?;
    validate_config(&config)?;
    let _replicate_cli = ProtectedExecutable::open(&config.replicate_cli_path)?;
    let secrets = MemorySecrets {
        local_read: load_secret(credentials, &config.credential_keys.local_read)?,
        local_replicate: load_secret(credentials, &config.credential_keys.local_replicate)?,
        remote_read: load_secret(credentials, &config.credential_keys.remote_read)?,
        remote_replicate: load_secret(credentials, &config.credential_keys.remote_replicate)?,
    };
    Ok(TrustedMemoryConfig { config, secrets })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceReadiness {
    status: String,
    schema_version: u32,
    node_id: String,
    revision_count: u64,
    conflict_count: u64,
    max_page_items: u64,
    max_envelope_bytes: u64,
    markdown_canonical: bool,
    sqlite_derived: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryServiceStatus {
    Ready,
    NotConfigured,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryServiceReadiness {
    status: MemoryServiceStatus,
    node_id: Option<String>,
    revision_count: u64,
    conflict_count: u64,
    endpoint: Option<String>,
    sync_interval_minutes: Option<u32>,
    tool_allowlist: Vec<String>,
    observed_at: String,
    error: Option<String>,
}

fn fail_soft_readiness(error: MemoryError, _private_path: Option<&str>) -> MemoryServiceReadiness {
    MemoryServiceReadiness {
        status: if error == MemoryError::NotConfigured {
            MemoryServiceStatus::NotConfigured
        } else {
            MemoryServiceStatus::Unavailable
        },
        node_id: None,
        revision_count: 0,
        conflict_count: 0,
        endpoint: None,
        sync_interval_minutes: None,
        tool_allowlist: Vec::new(),
        observed_at: Utc::now().to_rfc3339(),
        error: Some(error.code().to_string()),
    }
}

fn query_readiness(
    trusted: &TrustedMemoryConfig,
    timeout: Duration,
) -> Result<MemoryServiceReadiness, MemoryError> {
    let endpoint = format!("http://127.0.0.1:{}", trusted.config.local_port);
    let readiness = query_node_readiness(
        &endpoint,
        &trusted.secrets.local_read,
        &trusted.config.local_node_id,
        timeout,
    )?;
    Ok(MemoryServiceReadiness {
        status: MemoryServiceStatus::Ready,
        node_id: Some(readiness.node_id),
        revision_count: readiness.revision_count,
        conflict_count: readiness.conflict_count,
        endpoint: Some(endpoint),
        sync_interval_minutes: Some(trusted.config.sync_interval_minutes),
        tool_allowlist: trusted.config.tool_allowlist.clone(),
        observed_at: Utc::now().to_rfc3339(),
        error: None,
    })
}

fn query_node_readiness(
    endpoint: &str,
    bearer: &str,
    expected_node_id: &str,
    timeout: Duration,
) -> Result<ServiceReadiness, MemoryError> {
    if timeout.is_zero() || timeout > Duration::from_secs(10) {
        return Err(MemoryError::InvalidConfig);
    }
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|_| MemoryError::LocalServiceUnavailable)?;
    let response = client
        .get(format!("{endpoint}/replication/readiness"))
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, format!("Bearer {bearer}"))
        .send()
        .map_err(|_| MemoryError::LocalServiceUnavailable)?;
    if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
        return Err(MemoryError::AuthenticationFailed);
    }
    if response.status().is_redirection() || !response.status().is_success() {
        return Err(MemoryError::LocalServiceUnavailable);
    }
    if response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| value.split(';').next() != Some("application/json"))
    {
        return Err(MemoryError::InvalidResponse);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAXIMUM_RESPONSE_BYTES as u64)
    {
        return Err(MemoryError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    response
        .take((MAXIMUM_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| MemoryError::InvalidResponse)?;
    if bytes.len() > MAXIMUM_RESPONSE_BYTES {
        return Err(MemoryError::ResponseTooLarge);
    }
    let readiness: ServiceReadiness =
        serde_json::from_slice(&bytes).map_err(|_| MemoryError::InvalidResponse)?;
    if readiness.status != "ready"
        || readiness.schema_version != 1
        || readiness.node_id != expected_node_id
        || readiness.max_page_items == 0
        || readiness.max_page_items > 200
        || readiness.max_envelope_bytes == 0
        || readiness.max_envelope_bytes > MAXIMUM_RESPONSE_BYTES as u64
        || !readiness.markdown_canonical
        || !readiness.sqlite_derived
    {
        return if readiness.node_id != expected_node_id {
            Err(MemoryError::NodeIdentityMismatch)
        } else {
            Err(MemoryError::InvalidResponse)
        };
    }
    Ok(readiness)
}

fn wait_for_remote_readiness(
    tunnel: &mut SshTunnel,
    trusted: &TrustedMemoryConfig,
    timeout: Duration,
) -> Result<(), MemoryError> {
    let deadline = Instant::now() + timeout;
    let endpoint = format!("http://127.0.0.1:{}", tunnel.local_forward_port);
    loop {
        if SYNC_CANCELLED.load(Ordering::SeqCst) {
            return Err(MemoryError::Cancelled);
        }
        tunnel.ensure_running()?;
        match query_node_readiness(
            &endpoint,
            &trusted.secrets.remote_read,
            &trusted.config.home_node_id,
            Duration::from_millis(500),
        ) {
            Ok(_) => return Ok(()),
            Err(MemoryError::LocalServiceUnavailable) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(MemoryError::LocalServiceUnavailable) => return Err(MemoryError::SshUnavailable),
            Err(error) => return Err(error),
        }
    }
}

fn run_after_remote_preflight<T>(
    preflight: impl FnOnce() -> Result<(), MemoryError>,
    operation: impl FnOnce() -> Result<T, MemoryError>,
) -> Result<T, MemoryError> {
    preflight()?;
    operation()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    deny_unknown_fields,
    rename_all(deserialize = "snake_case", serialize = "camelCase")
)]
struct ReplicationResult {
    status: String,
    operation: String,
    source_node_id: String,
    target_node_id: String,
    from_cursor: u64,
    to_cursor: u64,
    accepted: u64,
    duplicates: u64,
    conflicts: u64,
    objects: u64,
    tombstones: u64,
    pages: u64,
    target_conflict_count: u64,
    last_success: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PipeError {
    Read,
    Limit,
    Task,
}

fn read_bounded<R: Read>(reader: R, maximum: usize) -> Result<Vec<u8>, PipeError> {
    let mut bytes = Vec::with_capacity(maximum.min(8192));
    reader
        .take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| PipeError::Read)?;
    if bytes.len() > maximum {
        return Err(PipeError::Limit);
    }
    Ok(bytes)
}

fn receive_pipe(
    receiver: &Receiver<Result<Vec<u8>, PipeError>>,
    cached: &mut Option<Vec<u8>>,
) -> Result<(), PipeError> {
    if cached.is_some() {
        return Ok(());
    }
    match receiver.try_recv() {
        Ok(Ok(bytes)) => {
            *cached = Some(bytes);
            Ok(())
        }
        Ok(Err(error)) => Err(error),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(PipeError::Task),
    }
}

fn terminate(child: &mut Child) -> Result<(), MemoryError> {
    match child.try_wait() {
        Ok(Some(_)) => Ok(()),
        Ok(None) => {
            child.kill().map_err(|_| MemoryError::Teardown)?;
            child.wait().map_err(|_| MemoryError::Teardown)?;
            Ok(())
        }
        Err(_) => Err(MemoryError::Teardown),
    }
}

fn run_replication_cli(
    binary: &Path,
    operation: &str,
    trusted: &TrustedMemoryConfig,
    tunnel_port: u16,
    timeout: Duration,
) -> Result<ReplicationResult, MemoryError> {
    if !matches!(operation, "pull" | "push")
        || tunnel_port == 0
        || timeout.is_zero()
        || timeout > Duration::from_secs(300)
    {
        return Err(MemoryError::InvalidConfig);
    }
    let executable = ProtectedExecutable::open(binary)?;
    let local_url = format!("http://127.0.0.1:{}", trusted.config.local_port);
    let remote_url = format!("http://127.0.0.1:{tunnel_port}");
    let (mut command, stdin_bound) = executable.command()?;
    command
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin:/bin")
        .env("MEMORY_LOCAL_READ_TOKEN", &trusted.secrets.local_read)
        .env(
            "MEMORY_LOCAL_REPLICATE_TOKEN",
            &trusted.secrets.local_replicate,
        )
        .env("MEMORY_REMOTE_READ_TOKEN", &trusted.secrets.remote_read)
        .env(
            "MEMORY_REMOTE_REPLICATE_TOKEN",
            &trusted.secrets.remote_replicate,
        )
        .args([
            operation,
            "--local-url",
            local_url.as_str(),
            "--remote-url",
            remote_url.as_str(),
            "--page-size",
            PAGE_SIZE,
            "--timeout",
            CLI_REQUEST_TIMEOUT_SECONDS,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !stdin_bound {
        command.stdin(Stdio::null());
        executable.revalidate_binary()?;
    }
    let mut child = command.spawn().map_err(|_| MemoryError::Spawn)?;
    let stdout = child.stdout.take().ok_or(MemoryError::Spawn)?;
    let stderr = child.stderr.take().ok_or(MemoryError::Spawn)?;
    let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
    let stdout_thread = std::thread::spawn(move || {
        let _ = stdout_sender.send(read_bounded(stdout, MAXIMUM_RESPONSE_BYTES));
    });
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    let stderr_thread = std::thread::spawn(move || {
        let _ = stderr_sender.send(read_bounded(stderr, MAXIMUM_STDERR_BYTES));
    });
    let started = Instant::now();
    let mut stdout_bytes = None;
    let mut stderr_bytes = None;
    let status = loop {
        if SYNC_CANCELLED.load(Ordering::SeqCst) {
            break terminate(&mut child).and(Err(MemoryError::Cancelled));
        }
        if receive_pipe(&stdout_receiver, &mut stdout_bytes).is_err()
            || receive_pipe(&stderr_receiver, &mut stderr_bytes).is_err()
        {
            break terminate(&mut child).and(Err(MemoryError::ResponseTooLarge));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => break terminate(&mut child).and(Err(MemoryError::Timeout)),
            Err(_) => break terminate(&mut child).and(Err(MemoryError::Teardown)),
        }
    };
    stdout_thread.join().map_err(|_| MemoryError::Task)?;
    stderr_thread.join().map_err(|_| MemoryError::Task)?;
    let status = status?;
    if stdout_bytes.is_none() {
        stdout_bytes = Some(
            stdout_receiver
                .recv()
                .map_err(|_| MemoryError::Task)?
                .map_err(|_| MemoryError::ResponseTooLarge)?,
        );
    }
    if stderr_bytes.is_none() {
        stderr_bytes = Some(
            stderr_receiver
                .recv()
                .map_err(|_| MemoryError::Task)?
                .map_err(|_| MemoryError::ResponseTooLarge)?,
        );
    }
    let _redacted_stderr = stderr_bytes.ok_or(MemoryError::Task)?;
    if !status.success() {
        return Err(MemoryError::Exit);
    }
    let stdout = stdout_bytes.ok_or(MemoryError::Task)?;
    if stdout.is_empty() || stdout.len() > MAXIMUM_RESPONSE_BYTES {
        return Err(MemoryError::InvalidResponse);
    }
    let result: ReplicationResult =
        serde_json::from_slice(&stdout).map_err(|_| MemoryError::InvalidResponse)?;
    let (source, target) = if operation == "pull" {
        (
            trusted.config.home_node_id.as_str(),
            trusted.config.local_node_id.as_str(),
        )
    } else {
        (
            trusted.config.local_node_id.as_str(),
            trusted.config.home_node_id.as_str(),
        )
    };
    if result.status != "ok"
        || result.operation != operation
        || result.source_node_id != source
        || result.target_node_id != target
        || result.to_cursor < result.from_cursor
        || DateTime::parse_from_rfc3339(&result.last_success).is_err()
    {
        return Err(MemoryError::NodeIdentityMismatch);
    }
    Ok(result)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PinnedHostResponse {
    host_alias: String,
    fingerprint: String,
    key_type: String,
}

impl From<PinnedHostEvidence> for PinnedHostResponse {
    fn from(value: PinnedHostEvidence) -> Self {
        Self {
            host_alias: value.host_alias,
            fingerprint: value.fingerprint,
            key_type: value.key_type,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemorySyncResponse {
    status: String,
    pull: Option<ReplicationResult>,
    push: Option<ReplicationResult>,
    pinned_host: Option<PinnedHostResponse>,
    last_success: Option<String>,
    error: Option<String>,
}

fn fail_soft_sync(error: MemoryError) -> MemorySyncResponse {
    MemorySyncResponse {
        status: "error".to_string(),
        pull: None,
        push: None,
        pinned_host: None,
        last_success: None,
        error: Some(error.code().to_string()),
    }
}

fn finish_after_tunnel_close<T>(
    operation: Result<T, MemoryError>,
    close: Result<(), SshError>,
) -> Result<T, MemoryError> {
    match (operation, close) {
        (Ok(value), Ok(())) => Ok(value),
        (_, Err(error)) => Err(error.into()),
        (Err(error), Ok(())) => Err(error),
    }
}

fn sync_memory_blocking(trusted: &TrustedMemoryConfig) -> Result<MemorySyncResponse, MemoryError> {
    if SYNC_CANCELLED.load(Ordering::SeqCst) {
        return Err(MemoryError::Cancelled);
    }
    let _local = query_readiness(trusted, READINESS_TIMEOUT)?;
    let reservation = ReservedLoopbackPort::new().map_err(|_| MemoryError::SshUnavailable)?;
    let tunnel_port = reservation.port();
    if tunnel_port == trusted.config.local_port {
        return Err(MemoryError::SshUnavailable);
    }
    let mut tunnel_config = SshTunnelConfig {
        home_host_alias: trusted.config.home_host_alias.clone(),
        home_user: trusted.config.home_user.clone(),
        pinned_host_fingerprint: trusted.config.pinned_host_fingerprint.clone(),
        known_hosts_path: trusted.config.known_hosts_path.clone(),
        identity_file: trusted.config.identity_file.clone(),
        remote_loopback_port: trusted.config.remote_loopback_port,
        local_forward_port: tunnel_port,
    };
    let mut tunnel =
        start_tunnel_with_reservation(Path::new(SSH_BINARY), &mut tunnel_config, reservation)?;
    let pin = tunnel.evidence.clone();
    let operation = run_after_remote_preflight(
        || wait_for_remote_readiness(&mut tunnel, trusted, TUNNEL_STARTUP_TIMEOUT),
        || {
            let pull = run_replication_cli(
                &trusted.config.replicate_cli_path,
                "pull",
                trusted,
                tunnel_port,
                REPLICATION_TIMEOUT,
            )?;
            let push = run_replication_cli(
                &trusted.config.replicate_cli_path,
                "push",
                trusted,
                tunnel_port,
                REPLICATION_TIMEOUT,
            )?;
            let last_success = push.last_success.clone();
            Ok(MemorySyncResponse {
                status: "ok".to_string(),
                pull: Some(pull),
                push: Some(push),
                pinned_host: Some(pin.into()),
                last_success: Some(last_success),
                error: None,
            })
        },
    );
    let close = tunnel.close();
    finish_after_tunnel_close(operation, close)
}

fn trusted_config_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<PathBuf, MemoryError> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(CONFIG_FILE_NAME))
        .map_err(|_| MemoryError::InvalidConfig)
}

pub(crate) fn start_memory_sync_scheduler(
    app: tauri::AppHandle,
) -> Option<Arc<MemorySyncScheduler>> {
    let path = trusted_config_path(&app).ok()?;
    let store = SecretStore::shared(crate::app_state::keyring_service());
    let trusted = load_trusted_config(&path, store).ok()?;
    let interval = Duration::from_secs(u64::from(trusted.config.sync_interval_minutes) * 60);
    let scheduler_app = app.clone();
    Some(Arc::new(MemorySyncScheduler::start(
        interval,
        Arc::clone(&SYNC_GATE),
        move || {
            let path = trusted_config_path(&scheduler_app)?;
            let store = SecretStore::shared(crate::app_state::keyring_service());
            let trusted = load_trusted_config(&path, store)?;
            sync_memory_blocking(&trusted).map(|_| ())
        },
    )))
}

pub(crate) fn cancel_active_memory_sync() {
    SYNC_CANCELLED.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub(crate) async fn get_memory_service_readiness(app: tauri::AppHandle) -> MemoryServiceReadiness {
    let task = tauri::async_runtime::spawn_blocking(move || {
        let path = trusted_config_path(&app)?;
        let store = SecretStore::shared(crate::app_state::keyring_service());
        let trusted = load_trusted_config(&path, store)?;
        query_readiness(&trusted, READINESS_TIMEOUT)
    });
    match task.await {
        Ok(Ok(readiness)) => readiness,
        Ok(Err(error)) => fail_soft_readiness(error, None),
        Err(_) => fail_soft_readiness(MemoryError::Task, None),
    }
}

#[tauri::command]
pub(crate) async fn sync_memory_service(app: tauri::AppHandle) -> MemorySyncResponse {
    let task = tauri::async_runtime::spawn_blocking(move || {
        let _guard = SYNC_GATE.try_enter()?;
        let path = trusted_config_path(&app)?;
        let store = SecretStore::shared(crate::app_state::keyring_service());
        let trusted = load_trusted_config(&path, store)?;
        sync_memory_blocking(&trusted)
    });
    match task.await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => fail_soft_sync(error),
        Err(_) => fail_soft_sync(MemoryError::Task),
    }
}

#[cfg(all(test, unix))]
#[path = "memory_tests.rs"]
mod memory_tests;
