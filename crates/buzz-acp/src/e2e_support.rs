//! Process-backed integration-test seam for the packaged durable runtime.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use buzz_runtime::{
    Capability, JobStartRequest, ResumeMode, RuntimeClient, SessionRecord, StoreHandle,
};
use chrono::Utc;
use nostr::{Keys, ToBech32};
use uuid::Uuid;

#[derive(serde::Serialize, serde::Deserialize)]
struct CoworkerReplyAction {
    relay_url: String,
    event: nostr::Event,
}

/// Inputs fixed by the durable-runtime integration fixture.
#[doc(hidden)]
#[derive(Clone)]
pub struct DurableRuntimeTestConfig {
    /// Stable pair-scoped runtime identifier.
    pub runtime_id: String,
    /// Pair-scoped durable state directory.
    pub state_dir: PathBuf,
    /// Owner-only schema-v2 receipt path.
    pub receipt_path: PathBuf,
    /// Canonical fake allowlisted LH executable.
    pub lh_executable: PathBuf,
    /// Canonical workspaces accepted by the privileged supervisor.
    pub workspace_roots: Vec<PathBuf>,
    /// Packaged `buzz-acp` executable, with canonical bundled siblings.
    pub runner_executable: PathBuf,
    /// Stable managed-agent signing identity.
    pub keys: Keys,
    /// Operator/owner identity accepted by the owner-only inbound gate.
    pub owner_pubkey: String,
    /// Additional collaborators accepted by the managed allowlist.
    pub allowed_pubkeys: Vec<String>,
    /// Pair-scoping relay URL used in the receipt.
    pub relay_url: String,
    /// Governed job the process-backed ACP fixture requests on its first assignment prompt.
    pub auto_job: Option<JobStartRequest>,
    /// Signed coworker reply the fixture publishes after consuming its first prompt.
    pub auto_reply: Option<nostr::Event>,
}

/// Real packaged runtime process plus a concurrent handle to its durable store.
#[doc(hidden)]
pub struct DurableRuntimeTestHarness {
    config: DurableRuntimeTestConfig,
    store: StoreHandle,
    child: Option<Child>,
}

impl DurableRuntimeTestHarness {
    /// Launches the packaged `buzz-acp`, then adopts only its authenticated receipt.
    pub async fn start(config: DurableRuntimeTestConfig) -> Result<Self> {
        buzz_runtime::ensure_owner_only_runtime_dir(&config.state_dir)
            .context("prepare durable runtime test directory")?;
        let lock_path = config.state_dir.join("pair.lock");
        let trace_path = config.state_dir.join("packaged-acp-methods.trace");
        let log_path = config.state_dir.join("packaged-runtime.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .context("open packaged runtime test log")?;
        let executable_dir = config
            .runner_executable
            .parent()
            .context("packaged runtime executable has no parent")?;
        let agent = executable_dir.join(if cfg!(windows) {
            "buzz-agent.exe"
        } else {
            "buzz-agent"
        });
        let mcp = executable_dir.join(if cfg!(windows) {
            "buzz-dev-mcp.exe"
        } else {
            "buzz-dev-mcp"
        });
        let workspace_roots = std::env::join_paths(&config.workspace_roots)
            .context("join approved workspace roots")?;
        let private_key = config
            .keys
            .secret_key()
            .to_bech32()
            .context("encode packaged runtime private key")?;
        let mut command = Command::new(&config.runner_executable);
        command
            .env_clear()
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
            )
            .env("HOME", &config.state_dir)
            .env("TMPDIR", &config.state_dir)
            .env("RUST_LOG", "buzz_acp=debug")
            .env("BUZZ_PRIVATE_KEY", private_key)
            .env("BUZZ_RELAY_URL", &config.relay_url)
            .env("BUZZ_ACP_AGENT_OWNER", &config.owner_pubkey)
            .env("BUZZ_ACP_RESPOND_TO", "allowlist")
            .env(
                "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
                config.allowed_pubkeys.join(","),
            )
            .env("BUZZ_ACP_AGENT_COMMAND", agent)
            .env("BUZZ_ACP_AGENT_ARGS", "__e2e-acp-adapter")
            .env("BUZZ_ACP_MCP_COMMAND", mcp)
            .env("BUZZ_ACP_RUNTIME_LOCK_PATH", &lock_path)
            .env("BUZZ_ACP_RUNTIME_STATE_DIR", &config.state_dir)
            .env("BUZZ_ACP_RUNTIME_ID", &config.runtime_id)
            .env("BUZZ_RUNTIME_RECEIPT", &config.receipt_path)
            .env("BUZZ_ACP_LH_COMMAND", &config.lh_executable)
            .env("BUZZ_ACP_JOB_WORKSPACE_ROOTS", workspace_roots)
            .env("BUZZ_ACP_DURABLE_RUNTIME", "true")
            .env("BUZZ_ACP_JOB_EVENT_PUBLICATION", "true")
            .env("BUZZ_ACP_IDLE_TIMEOUT", "1")
            .env("BUZZ_ACP_MAX_TURN_DURATION", "2")
            .env("BUZZ_ACP_DEDUP", "queue")
            .env("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "steer")
            .env("BUZZ_ACP_NO_MEMORY", "true")
            .env("BUZZ_ACP_NO_PRESENCE", "true")
            .env("BUZZ_ACP_NO_TYPING", "true")
            .env("BUZZ_ACP_E2E_FIXTURE", "1")
            .env("BUZZ_ACP_E2E_TRACE", trace_path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                log.try_clone().context("clone packaged runtime log")?,
            ))
            .stderr(Stdio::from(log));
        if let Some(auto_job) = &config.auto_job {
            command.env(
                "BUZZ_ACP_E2E_AUTO_JOB",
                serde_json::to_string(auto_job).context("serialize fixture job request")?,
            );
        }
        if let Some(auto_reply) = &config.auto_reply {
            let action = CoworkerReplyAction {
                relay_url: config.relay_url.clone(),
                event: auto_reply.clone(),
            };
            command.env(
                "BUZZ_ACP_E2E_AUTO_REPLY",
                serde_json::to_string(&action).context("serialize fixture coworker reply")?,
            );
        }
        let mut child = command.spawn().context("spawn packaged durable runtime")?;
        let child_pid = child.id();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let receipt = loop {
            if let Ok(receipt) = buzz_runtime::read_runtime_receipt(&config.receipt_path) {
                if receipt.ready && receipt.pid == child_pid {
                    break receipt;
                }
            }
            if let Some(status) = child.try_wait().context("inspect packaged runtime")? {
                let output = std::fs::read_to_string(&log_path).unwrap_or_default();
                anyhow::bail!(
                    "packaged runtime exited before receipt ({status}): {}",
                    output.trim()
                );
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let output = std::fs::read_to_string(&log_path).unwrap_or_default();
                anyhow::bail!("packaged runtime receipt timed out: {}", output.trim());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        anyhow::ensure!(
            buzz_runtime::process_matches_marker(receipt.pid, &receipt.process_start_marker),
            "packaged runtime receipt process identity is not live"
        );
        let store = StoreHandle::open(config.state_dir.join("runtime.sqlite3"))
            .context("open packaged durable runtime store")?;
        Ok(Self {
            config,
            store,
            child: Some(child),
        })
    }

    /// Returns a handle to the pair-scoped production store.
    pub fn store(&self) -> StoreHandle {
        self.store.clone()
    }

    /// Returns the active owner-only receipt path.
    pub fn receipt_path(&self) -> &Path {
        &self.config.receipt_path
    }

    /// Returns the process-fenced generation currently in the receipt.
    pub fn generation(&self) -> Uuid {
        buzz_runtime::read_runtime_receipt(&self.config.receipt_path)
            .expect("read packaged runtime receipt")
            .generation
    }

    fn kill_runtime(&mut self) -> Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if child.try_wait()?.is_none() {
            child.kill().context("kill packaged runtime")?;
        }
        child.wait().context("reap packaged runtime")?;
        Ok(())
    }

    /// Crashes the packaged runtime process and launches a new process over the same store.
    pub async fn restart(mut self) -> Result<Self> {
        let config = self.config.clone();
        self.kill_runtime()?;
        Self::start(config).await
    }
}

impl Drop for DurableRuntimeTestHarness {
    fn drop(&mut self) {
        let _ = self.kill_runtime();
    }
}

/// Evidence returned after a process-backed ACP adapter is killed and resumed.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterRecoveryEvidence {
    /// Session identifier reused by the replacement adapter process.
    pub session_id: String,
    /// Durable mode recorded only after `session/resume` succeeds.
    pub resume_mode: ResumeMode,
    /// Ordered JSON-RPC methods observed across both adapter processes.
    pub methods: Vec<String>,
}

/// Spawns, initializes, kills, and replaces an ACP adapter, then resumes its durable session.
#[doc(hidden)]
pub async fn exercise_process_backed_adapter_recovery(
    store: &StoreHandle,
    channel_id: Uuid,
    cwd: &Path,
    packaged_acp: &Path,
    trace_path: &Path,
) -> Result<AdapterRecoveryEvidence> {
    let command = packaged_acp
        .to_str()
        .context("packaged ACP fixture path is not UTF-8")?;
    let args = vec!["__e2e-acp-adapter".to_owned()];
    let environment = vec![
        ("BUZZ_ACP_E2E_FIXTURE".to_owned(), "1".to_owned()),
        (
            "BUZZ_ACP_E2E_TRACE".to_owned(),
            trace_path.to_string_lossy().into_owned(),
        ),
    ];
    let cwd = cwd
        .to_str()
        .context("ACP recovery workspace path is not UTF-8")?;

    let mut first = crate::acp::AcpClient::spawn(command, &args, &environment, false).await?;
    first.initialize().await?;
    let created = first.session_new_full(cwd, Vec::new(), None, None).await?;
    store
        .upsert_channel_session(SessionRecord {
            channel_id,
            session_id: created.session_id.clone(),
            adapter_fingerprint: "process-backed-e2e-adapter".to_owned(),
            cwd: cwd.to_owned(),
            config_hash: "jac-575-e2e-config".to_owned(),
            resume_mode: ResumeMode::Fresh,
            updated_at: Utc::now(),
        })
        .await?;
    let _ = first.shutdown().await;

    let mut replacement = crate::acp::AcpClient::spawn(command, &args, &environment, false).await?;
    replacement.initialize().await?;
    anyhow::ensure!(
        replacement.session_resume_supported(),
        "replacement ACP adapter did not advertise session/resume"
    );
    let persisted = store
        .get_channel_session(channel_id)
        .await?
        .context("durable ACP session mapping disappeared during adapter restart")?;
    replacement
        .session_resume(&persisted.session_id, cwd, Vec::new())
        .await?;
    store
        .upsert_channel_session(SessionRecord {
            resume_mode: ResumeMode::Resume,
            updated_at: Utc::now(),
            ..persisted.clone()
        })
        .await?;
    let _ = replacement.shutdown().await;

    let resumed = store
        .get_channel_session(channel_id)
        .await?
        .context("resumed ACP session mapping was not persisted")?;
    let methods = std::fs::read_to_string(trace_path)?
        .lines()
        .map(str::to_owned)
        .collect();
    Ok(AdapterRecoveryEvidence {
        session_id: resumed.session_id,
        resume_mode: resumed.resume_mode,
        methods,
    })
}

pub(crate) fn run_process_backed_adapter_fixture() -> Result<()> {
    anyhow::ensure!(
        std::env::var("BUZZ_ACP_E2E_FIXTURE").as_deref() == Ok("1"),
        "process-backed ACP fixture is disabled"
    );
    let trace_path = PathBuf::from(
        std::env::var_os("BUZZ_ACP_E2E_TRACE").context("missing ACP fixture trace path")?,
    );
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let auto_job = std::env::var("BUZZ_ACP_E2E_AUTO_JOB")
        .ok()
        .map(|raw| serde_json::from_str::<JobStartRequest>(&raw))
        .transpose()
        .context("parse fixture auto-job request")?;
    let receipt_path = std::env::var_os("BUZZ_RUNTIME_RECEIPT").map(PathBuf::from);
    let auto_reply = std::env::var("BUZZ_ACP_E2E_AUTO_REPLY")
        .ok()
        .map(|raw| serde_json::from_str::<CoworkerReplyAction>(&raw))
        .transpose()
        .context("parse fixture coworker reply")?;
    let mut job_started = false;
    for line in stdin.lock().lines() {
        let line = line?;
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request
            .get("method")
            .and_then(serde_json::Value::as_str)
            .context("ACP fixture request omitted method")?;
        let mut trace = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&trace_path)?;
        writeln!(trace, "{method}")?;
        let result = match method {
            "initialize" => serde_json::json!({
                "protocolVersion": 2,
                "agentCapabilities": {
                    "sessionCapabilities": {"resume": true},
                    "loadSession": true
                },
                "authMethods": []
            }),
            "session/new" => serde_json::json!({"sessionId": "jac-575-acp-session"}),
            "session/prompt" => {
                if !job_started {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("build fixture action executor")?;
                    if let Some(request) = auto_job.as_ref() {
                        let receipt_path = receipt_path
                            .as_ref()
                            .context("missing fixture runtime receipt for auto-job")?;
                        let status = runtime.block_on(async {
                            let client =
                                RuntimeClient::from_receipt(receipt_path, Capability::Model)
                                    .await?;
                            client.jobs_start(request.clone()).await
                        })?;
                        anyhow::ensure!(
                            matches!(
                                status.state,
                                buzz_runtime::JobState::Accepted | buzz_runtime::JobState::Running
                            ),
                            "managed assignment did not accept governed job"
                        );
                    }
                    if let Some(reply) = auto_reply.as_ref() {
                        let url = reply
                            .relay_url
                            .replacen("ws://", "http://", 1)
                            .replacen("wss://", "https://", 1);
                        runtime.block_on(async {
                            reqwest::Client::new()
                                .post(format!("{url}/events"))
                                .json(&reply.event)
                                .send()
                                .await?
                                .error_for_status()?;
                            anyhow::Ok(())
                        })?;
                    }
                    job_started = true;
                }
                serde_json::json!({"stopReason": "end_turn"})
            }
            "session/resume" => serde_json::json!({}),
            other => {
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("unsupported fixture method: {other}")}
                });
                serde_json::to_writer(&mut stdout, &response)?;
                stdout.write_all(b"\n")?;
                stdout.flush()?;
                continue;
            }
        };
        let response = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result});
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
