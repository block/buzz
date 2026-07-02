//! ACP client module — manages communication with an AI agent subprocess over stdio
//! using JSON-RPC 2.0 (newline-delimited / NDJSON).
//!
//! # Lifecycle
//! 1. [`AcpClient::spawn`] — launch agent binary as subprocess
//! 2. [`AcpClient::initialize`] — protocol version negotiation
//! 3. [`AcpClient::session_new`] — create session with MCP server config
//! 4. [`AcpClient::session_prompt_with_idle_timeout`] — send prompt with idle/hard deadline, return stop reason
//! 5. [`AcpClient::session_cancel`] / [`AcpClient::cancel_with_cleanup`] — cancel in-flight turn

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

use crate::observer::{ObserverContext, ObserverHandle};
use crate::usage::{TurnUsage, UsageTracker};

/// Maximum allowed size of a single NDJSON line from the agent's stdout.
/// Lines exceeding this limit are rejected to prevent OOM from rogue agents.
const MAX_LINE_SIZE: usize = 10_000_000; // 10 MB

/// An MCP server configuration passed to `session/new`.
///
/// Corresponds to the `McpServerStdio` variant in the ACP schema.
/// All four fields are **required** by the schema (`args` and `env` may be empty arrays).
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
}

/// A single environment variable for an MCP server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Stop reason returned by `session/prompt` when the agent finishes a turn.
///
/// Maps to the `stopReason` field in the `SessionPromptResponse`.
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    /// Agent completed the turn normally (`"end_turn"`).
    EndTurn,
    /// Turn was cancelled via `session/cancel` (`"cancelled"`).
    Cancelled,
    /// Agent hit its token limit (`"max_tokens"`).
    MaxTokens,
    /// Agent hit its per-turn request limit (`"max_turn_requests"`).
    MaxTurnRequests,
    /// Agent refused the prompt (`"refusal"`).
    /// Note: refused turns are dropped from history by the agent.
    Refusal,
}

impl StopReason {
    /// Parse a `stopReason` string from the ACP wire format.
    ///
    /// Matching is case-insensitive so agents that send `"END_TURN"` or
    /// `"Cancelled"` are handled correctly without a protocol error.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "end_turn" => Some(Self::EndTurn),
            "cancelled" => Some(Self::Cancelled),
            "max_tokens" => Some(Self::MaxTokens),
            "max_turn_requests" => Some(Self::MaxTurnRequests),
            "refusal" => Some(Self::Refusal),
            _ => None,
        }
    }
}

/// Errors that can occur in the ACP client.
#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Agent process exited unexpectedly")]
    AgentExited,

    #[error("Idle timeout — no agent activity for {0:?}")]
    IdleTimeout(std::time::Duration),

    #[error("Hard turn timeout exceeded")]
    HardTimeout,

    #[error("Request timeout — agent did not respond within {0:?}")]
    Timeout(std::time::Duration),

    #[error("Write timeout — agent stopped reading stdin (blocked for {0:?})")]
    WriteTimeout(std::time::Duration),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Agent reported error: {0}")]
    AgentError(String),
}

/// ACP client that owns an agent subprocess and communicates over its stdio.
///
/// One `AcpClient` per agent process. Multiple sessions can be created on the
/// same client via repeated calls to [`session_new`](AcpClient::session_new).
pub struct AcpClient {
    /// The agent child process (kept alive to prevent zombie).
    child: Child,
    /// Write end of the agent's stdin pipe.
    stdin: ChildStdin,
    /// Framed reader over the agent's stdout pipe (line-oriented, bounded).
    /// Uses `LinesCodec::new_with_max_length` to enforce MAX_LINE_SIZE at the
    /// read level — prevents OOM from rogue agents writing infinite non-newline bytes.
    reader: FramedRead<ChildStdout, LinesCodec>,
    /// Monotonically increasing JSON-RPC request id counter.
    /// Harness-generated IDs are always numeric.
    next_id: u64,
    /// The id of a `session/request_permission` request that has been received
    /// but not yet responded to. Stored as `serde_json::Value` because JSON-RPC 2.0
    /// permits both numeric and string IDs from the agent.
    /// Used by [`cancel_with_cleanup`](AcpClient::cancel_with_cleanup) to send
    /// a `cancelled` outcome before the agent returns from `session/prompt`.
    pending_permission_id: Option<serde_json::Value>,
    /// Whether we have already sent a response to the pending permission request.
    /// Guards against double-response if a timeout fires after the allow_once
    /// response was written but before `pending_permission_id` was cleared.
    permission_responded: bool,
    /// The JSON-RPC id of the most recently sent `session/prompt` request.
    /// Used by [`cancel_with_cleanup`] to drain the correct response.
    /// Set in [`session_prompt_with_idle_timeout`]; consumed in [`cancel_with_cleanup`].
    last_prompt_id: Option<u64>,
    /// Hard deadline for the current turn, set by `session_prompt_with_idle_timeout`.
    /// Inherited by `cancel_with_cleanup` so the drain loop shares the same budget
    /// rather than starting a fresh timer (prevents double-jeopardy).
    current_hard_deadline: Option<tokio::time::Instant>,
    /// Optional local observer feed used by the desktop app.
    observer: Option<ObserverHandle>,
    /// Pool slot index for this agent process.
    observer_agent_index: Option<usize>,
    /// Best-effort context attached to raw ACP wire events.
    observer_context: ObserverContext,
    /// Most recently observed `_meta.goose.activeRunId` from a
    /// `session/update` notification of kind `session_info_update`.
    ///
    /// Both goose and buzz-agent emit `session_info_update` with this field;
    /// goose emits it whenever it starts or clears an active prompt run
    /// (`crates/goose/src/acp/server.rs:2277` `send_active_run_update`).
    /// Required as `expectedRunId` when calling the non-standard
    /// `_goose/unstable/session/steer` method to inject a message into an
    /// in-flight turn without cancelling it.
    ///
    /// `None` until the first `session_info_update` arrives, or after the
    /// run clears (goose/buzz-agent emit `activeRunId: null` at end of turn).
    /// Other agents may leave this unset — readers must treat `None` as
    /// "no active run to steer into" and fall back to cancel+merge.
    active_run_id: Option<String>,
    /// Per-turn channel for receiving goose-native non-cancelling steer
    /// requests from the main loop. Installed by
    /// [`install_steer_rx`](Self::install_steer_rx) at dispatch and
    /// consumed (via `take()`) by `session_prompt_with_idle_timeout` so it
    /// is dropped at scope exit alongside the turn it served. `None`
    /// outside of a goose-native turn — the read loop's steer arm is
    /// disabled in that case.
    steer_rx: Option<tokio::sync::mpsc::Receiver<crate::pool::SteerRequest>>,
    /// Usage tracker — accumulates cumulative token counts from
    /// `_goose/unstable/session/update` notifications and computes per-turn
    /// deltas. Both goose and buzz-agent emit this notification; goose gates
    /// on client capability advertisement, buzz-agent emits unconditionally.
    goose_usage: UsageTracker,
}

impl AcpClient {
    /// Kill the agent subprocess and wait for it to exit (no zombies).
    ///
    /// `Drop` only calls `start_kill()` (sends SIGKILL but doesn't reap).
    /// Call this when you need guaranteed cleanup — e.g., in `run_models`
    /// before process exit.
    pub async fn shutdown(&mut self) {
        // Kill the entire process group when possible. The child was spawned
        // with process_group(0), so its PID == its PGID. Killing the group
        // ensures subprocesses (MCP servers, tool processes) are cleaned up
        // rather than orphaned to init.
        //
        // Falls back to start_kill() (direct child only) on non-Unix or if
        // the child has been polled to completion (id() returns None).
        match self.child.id() {
            Some(pid) if kill_process_group(pid) => {}
            _ => {
                let _ = self.child.start_kill();
            }
        }
        // Bounded wait: if the child doesn't exit within 5s after SIGKILL,
        // give up and let Drop/OS handle it. An unbounded wait here would
        // wedge the harness during respawn or shutdown if a child is stuck.
        match tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::debug!("child wait error after kill: {e}"),
            Err(_) => tracing::warn!("child did not exit within 5s after SIGKILL — abandoning"),
        }
    }

    /// Spawn the agent binary as a subprocess and connect to its stdio pipes.
    ///
    /// After spawning, call [`initialize`](Self::initialize) before any other method.
    pub async fn spawn(
        command: &str,
        args: &[String],
        extra_env: &[(String, String)],
    ) -> Result<Self, AcpError> {
        use std::process::Stdio;

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherit stderr so agent logs are visible in the harness terminal.
            .stderr(Stdio::inherit())
            // Ensure the child is killed when the AcpClient is dropped (best-effort).
            // Callers MUST still call shutdown().await for guaranteed cleanup.
            .kill_on_drop(true);

        // Per-persona env vars (e.g., GOOSE_PROVIDER, BUZZ_AGENT_PROVIDER).
        // Only injected if not already set in parent env (operator precedence).
        for (key, value) in extra_env {
            if std::env::var(key).is_err() {
                cmd.env(key, value);
            }
        }

        // Spawn the agent in its own process group so SIGKILL doesn't propagate
        // to the harness's own process group on Unix.
        // tokio::process::Command::process_group is a stable tokio API (no extra imports needed).
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd.spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Protocol("failed to open agent stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Protocol("failed to open agent stdout".into()))?;

        Ok(Self {
            child,
            stdin,
            reader: FramedRead::new(stdout, LinesCodec::new_with_max_length(MAX_LINE_SIZE)),
            next_id: 0,
            pending_permission_id: None,
            permission_responded: false,
            last_prompt_id: None,
            current_hard_deadline: None,
            observer: None,
            observer_agent_index: None,
            observer_context: ObserverContext::default(),
            active_run_id: None,
            steer_rx: None,
            goose_usage: UsageTracker::default(),
        })
    }

    /// Attach a local observer feed to this ACP client.
    pub fn set_observer(&mut self, observer: Option<ObserverHandle>, agent_index: usize) {
        self.observer = observer;
        self.observer_agent_index = Some(agent_index);
    }

    /// Update metadata that will be attached to subsequent raw wire events.
    pub fn set_observer_context(&mut self, context: ObserverContext) {
        self.observer_context = context;
    }

    /// Return a clone of the observer handle, if attached.
    pub(crate) fn observer_handle(&self) -> Option<ObserverHandle> {
        self.observer.clone()
    }

    /// Return the pool slot index for this agent process.
    pub(crate) fn observer_agent_index(&self) -> Option<usize> {
        self.observer_agent_index
    }

    /// Emit a semantic event to the local observer feed, if enabled.
    pub fn observe(&self, kind: impl Into<String>, payload: serde_json::Value) {
        if let Some(observer) = &self.observer {
            observer.emit(
                kind,
                self.observer_agent_index,
                &self.observer_context,
                payload,
            );
        }
    }

    /// Send the `initialize` request and return the agent's response result value.
    ///
    /// Must be called exactly once, before any other ACP method.
    /// The caller may inspect `agentCapabilities` in the returned value.
    pub async fn initialize(&mut self) -> Result<serde_json::Value, AcpError> {
        // Requesting version 2 is an intentional temporary pin — we are squatting
        // on ACP v2 ahead of the upstream ACP RFD. Revisit when that RFD merges.
        let params = serde_json::json!({
            "protocolVersion": 2,
            "clientCapabilities": {
                // Signal to goose that we handle `_goose/unstable/session/update`
                // notifications. Without this the custom notification is suppressed
                // on goose's side and usage data is never emitted.
                "_meta": {
                    "goose": {
                        "customNotifications": true
                    }
                }
            },
            "clientInfo": {
                "name": "buzz-acp",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        let result = self.send_request("initialize", params).await?;
        tracing::debug!(target: "acp::init", "initialize response: {result}");
        Ok(result)
    }

    /// Send `session/new` and return the full response alongside the session ID.
    ///
    /// `cwd` must be an absolute path. `mcp_servers` may be empty.
    /// `system_prompt` is included in the request when `Some` — agents that
    /// support the field will use it; others ignore unknown fields per JSON-RPC.
    /// Callers use [`extract_model_config_options`] and [`extract_model_state`]
    /// to pull model info from the raw result.
    pub async fn session_new_full(
        &mut self,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
        system_prompt: Option<&str>,
    ) -> Result<SessionNewResponse, AcpError> {
        let mut params = serde_json::json!({
            "cwd": cwd,
            "mcpServers": mcp_servers,
        });
        if let Some(sp) = system_prompt {
            params["systemPrompt"] = serde_json::Value::String(sp.to_owned());
        }
        let result = self.send_request("session/new", params).await?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("session/new response missing sessionId".into()))?
            .to_owned();
        tracing::info!(target: "acp::session", "session created: {session_id}");
        Ok(SessionNewResponse {
            session_id,
            raw: result,
        })
    }

    /// Send `session/new` and return only the `sessionId` string.
    ///
    /// Convenience wrapper around [`session_new_full`].
    #[allow(dead_code)] // Public API — callers outside the harness may use this.
    pub async fn session_new(
        &mut self,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
        system_prompt: Option<&str>,
    ) -> Result<String, AcpError> {
        Ok(self
            .session_new_full(cwd, mcp_servers, system_prompt)
            .await?
            .session_id)
    }

    /// Send `session/set_config_option` (stable ACP path).
    pub async fn session_set_config_option(
        &mut self,
        session_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "configId": config_id,
            "value": value,
        });
        self.send_request("session/set_config_option", params).await
    }

    /// Send `session/set_model` (unstable ACP path).
    pub async fn session_set_model(
        &mut self,
        session_id: &str,
        model_id: &str,
    ) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "modelId": model_id,
        });
        self.send_request("session/set_model", params).await
    }

    /// Send `session/prompt` with idle-based timeout instead of wall-clock.
    ///
    /// The idle deadline resets on any stdout activity from the agent. The hard
    /// deadline is an absolute wall-clock cap (safety valve).
    pub async fn session_prompt_with_idle_timeout(
        &mut self,
        session_id: &str,
        prompt_text: &str,
        idle_timeout: std::time::Duration,
        max_duration: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        self.session_prompt_blocks_with_idle_timeout(
            session_id,
            std::slice::from_ref(&prompt_text),
            idle_timeout,
            max_duration,
        )
        .await
    }

    /// Like [`session_prompt_with_idle_timeout`](Self::session_prompt_with_idle_timeout),
    /// but sends each entry in `prompt_blocks` as a separate text content block.
    ///
    /// Used for slash-command pass-through: ACP connectors detect commands via
    /// the **first** block's text starting with `/`, so the harness sends
    /// `["/cmd args", "<buzz context>"]` instead of one wrapped block.
    pub async fn session_prompt_blocks_with_idle_timeout(
        &mut self,
        session_id: &str,
        prompt_blocks: &[&str],
        idle_timeout: std::time::Duration,
        max_duration: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        let params = build_prompt_params(session_id, prompt_blocks);
        let hard_deadline = tokio::time::Instant::now() + max_duration;
        self.current_hard_deadline = Some(hard_deadline);

        // Mark the usage tracker as in-flight for this turn BEFORE sending the
        // prompt so that any setup notifications recorded earlier are not
        // misattributed to this turn.
        self.goose_usage.begin_turn(session_id);

        self.last_prompt_id = Some(self.next_id);
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/prompt",
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ {}", &serde_json::to_string(&msg).unwrap_or_default());
        if let Err(e) = self.write_ndjson(&msg).await {
            self.last_prompt_id = None;
            self.current_hard_deadline = None;
            return Err(e);
        }

        let result = self
            .read_until_response_with_idle_timeout(session_id, id, idle_timeout, hard_deadline)
            .await;

        // On timeout errors, leave current_hard_deadline set so cancel_with_cleanup
        // can inherit the remaining budget. Clear it on all other outcomes.
        match &result {
            Ok(_) => {
                self.last_prompt_id = None;
                self.current_hard_deadline = None;
            }
            Err(AcpError::IdleTimeout(_) | AcpError::HardTimeout) => {
                // Leave last_prompt_id and current_hard_deadline set —
                // caller will invoke cancel_with_cleanup.
            }
            Err(_) => {
                self.last_prompt_id = None;
                self.current_hard_deadline = None;
            }
        }
        self.parse_stop_reason(&result?)
    }

    /// Send a `session/cancel` **notification** (no `id` field, no response expected).
    ///
    /// After calling this, the agent will eventually respond to the in-flight
    /// `session/prompt` with `stopReason: "cancelled"`. Use
    /// [`cancel_with_cleanup`](Self::cancel_with_cleanup) if you need to drain
    /// that response.
    ///
    /// Note: async because writing to stdin requires async I/O.
    pub async fn session_cancel(&mut self, session_id: &str) -> Result<(), AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
        });
        self.send_notification("session/cancel", params).await
    }

    /// Returns `true` if a `session/prompt` request is currently in flight.
    pub fn has_in_flight_prompt(&self) -> bool {
        self.last_prompt_id.is_some()
    }

    /// Most recently observed goose `_meta.goose.activeRunId` from a
    /// `session_info_update`, if any.
    ///
    /// Both goose and buzz-agent emit `session_info_update`; other agents
    /// leave this `None` for the lifetime of the client. Read directly by
    /// `read_until_response_with_idle_timeout`'s
    /// steer arm at write time (see [`crate::pool::SteerRequest`] for
    /// why the read loop owns this); production callers do not need this
    /// accessor. Kept as `pub` so tests can introspect the field.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active_run_id(&self) -> Option<&str> {
        self.active_run_id.as_deref()
    }

    /// Consume and return the per-turn usage record computed from the most
    /// recent `_goose/unstable/session/update` notification.
    ///
    /// Returns `None` if no usage update arrived since the last call (i.e.
    /// the harness did not emit one for this turn, or this is not a goose
    /// agent). Must be called at most once per turn; subsequent calls return
    /// `None` until the next `usage_update` notification is recorded.
    ///
    /// Intended for consumption by `publish_agent_turn_metric` in `pool.rs` to
    /// publish a kind 44200 NIP-AM event.
    pub fn take_turn_usage(&mut self) -> Option<TurnUsage> {
        self.goose_usage.take()
    }

    /// Install a per-turn steer request channel for goose-native
    /// non-cancelling mid-turn delivery.
    ///
    /// Called by the dispatch path immediately before
    /// [`session_prompt_with_idle_timeout`] for all prompt tasks.
    /// The matching `Sender` is stored in `TaskMeta.steer_tx` for the
    /// main loop's mode-gate fork to drive.
    ///
    /// Panics if a receiver is already installed — there is exactly one
    /// turn per `AcpClient` at a time, and stacking receivers would
    /// silently misroute steer requests across turns. The previous
    /// turn's receiver must have been consumed by the read loop and
    /// dropped at scope exit before the next turn dispatches.
    pub fn install_steer_rx(&mut self, rx: tokio::sync::mpsc::Receiver<crate::pool::SteerRequest>) {
        assert!(
            self.steer_rx.is_none(),
            "install_steer_rx: previous turn's receiver was not consumed — \
             stacking receivers would misroute steer requests across turns"
        );
        self.steer_rx = Some(rx);
    }

    /// Clear any installed steer receiver without consuming it.
    ///
    /// Called by `send_prompt_result` on every exit path of `run_prompt_task`
    /// so that `install_steer_rx`'s `is_none()` invariant holds for the next
    /// dispatch even when the turn ended before the read loop ran `take()`.
    /// Idempotent — safe to call when `steer_rx` is already `None`.
    pub fn clear_steer_rx(&mut self) {
        self.steer_rx = None;
    }

    /// Returns `true` if no steer receiver is currently installed.
    ///
    /// Test-only: used by `pool` tests to assert the post-return invariant
    /// without exposing the private field directly.
    #[cfg(test)]
    pub fn steer_rx_is_none(&self) -> bool {
        self.steer_rx.is_none()
    }

    /// Cancel a turn cleanly, handling any pending permission request first.
    ///
    /// Steps:
    /// 1. If there is a pending `session/request_permission` that hasn't been
    ///    responded to yet, respond with `outcome: "cancelled"`.
    /// 2. Send `session/cancel` notification (no id).
    /// 3. Continue reading until the `session/prompt` response arrives with `stopReason: "cancelled"`.
    ///
    /// Returns the final [`StopReason`] (almost always [`StopReason::Cancelled`]).
    pub async fn cancel_with_cleanup(
        &mut self,
        session_id: &str,
        _idle_timeout: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        // Inherit the hard deadline from the timed-out turn so the drain loop
        // doesn't start a fresh timer (prevents double-jeopardy). If the original
        // deadline is already expired or near-expired, grant a 30s floor so the
        // cancel notification has time to propagate and the agent can respond.
        let stored_deadline = self.current_hard_deadline.take();
        let min_cleanup_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let hard_deadline = match stored_deadline {
            Some(d) if d > min_cleanup_deadline => d,
            Some(_) => {
                tracing::debug!(
                    "original hard deadline expired or near-expired — using 30s cleanup grace"
                );
                min_cleanup_deadline
            }
            None => {
                tracing::warn!(
                    "cancel_with_cleanup called without current_hard_deadline — using 30s fallback"
                );
                min_cleanup_deadline
            }
        };

        self.cancel_with_cleanup_until(session_id, hard_deadline)
            .await
    }

    /// Cancel a user-interrupted turn with a bounded grace window.
    ///
    /// Some ACP servers currently keep streaming after `session/cancel`. For an
    /// explicit Stop button, waiting until the original turn deadline can make
    /// cancellation look broken. This variant gives the agent a short chance to
    /// acknowledge cancellation, then returns a timeout so the caller can respawn
    /// the agent process and actually stop the work.
    pub async fn cancel_with_cleanup_grace(
        &mut self,
        session_id: &str,
        grace: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        let _ = self.current_hard_deadline.take();
        let hard_deadline = tokio::time::Instant::now() + grace;
        self.cancel_with_cleanup_until(session_id, hard_deadline)
            .await
    }

    async fn cancel_with_cleanup_until(
        &mut self,
        session_id: &str,
        hard_deadline: tokio::time::Instant,
    ) -> Result<StopReason, AcpError> {
        // Validate precondition before any side effects — fail fast if there's
        // no in-flight prompt (prevents writing permission responses or cancel
        // notifications to the agent when no prompt is active).
        let prompt_id = self.last_prompt_id.take().ok_or_else(|| {
            AcpError::Protocol("cancel_with_cleanup called with no in-flight prompt".into())
        })?;

        // Step 1: respond to any pending permission request with "cancelled",
        // but only if we haven't already responded (guards against double-response race).
        if let Some(perm_id) = self.pending_permission_id.clone() {
            if !self.permission_responded {
                let response = permission_response_cancelled(&perm_id);
                self.write_ndjson(&response).await?;
                tracing::debug!(
                    target: "acp::cancel",
                    "responded cancelled to pending permission id={perm_id}"
                );
            }
            self.pending_permission_id = None;
            self.permission_responded = false;
        }

        // Step 2: send session/cancel notification (no id)
        self.session_cancel(session_id).await?;
        tracing::info!(target: "acp::cancel", "sent session/cancel for {session_id}");
        // Use a fixed 30s idle timeout during cleanup — the cancel notification
        // needs time to propagate and the agent may go silent while winding down.
        // The separate hard_deadline bounds agents that keep producing output
        // but ignore cancellation.
        let cleanup_idle = std::time::Duration::from_secs(30);
        let result = self
            .read_until_response_with_idle_timeout(
                session_id,
                prompt_id,
                cleanup_idle,
                hard_deadline,
            )
            .await?;
        self.parse_stop_reason(&result)
    }

    /// Serialize `value` as a single NDJSON line and flush to the agent's stdin.
    ///
    /// Bounded by a 30-second write timeout. If the agent stops reading stdin
    /// (e.g., it's stuck or dead), the write would otherwise block forever.
    async fn write_ndjson(&mut self, value: &serde_json::Value) -> Result<(), AcpError> {
        const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        let line = serde_json::to_string(value)?;
        tokio::time::timeout(WRITE_TIMEOUT, async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.write_all(b"\n").await?;
            self.stdin.flush().await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| AcpError::WriteTimeout(WRITE_TIMEOUT))?
        .map_err(AcpError::Io)?;
        self.observe("acp_write", value.clone());
        Ok(())
    }

    /// Default timeout for non-prompt RPCs (initialize, session/new, etc.).
    const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

    /// Send a JSON-RPC request and wait for the matching response.
    ///
    /// Assigns the next available id, writes the NDJSON line to stdin,
    /// then calls [`read_until_response`](Self::read_until_response).
    ///
    /// The write phase is bounded by `WRITE_TIMEOUT` (30s) and the read phase
    /// by `REQUEST_TIMEOUT` (60s), so worst-case wall clock is ~90s. Non-prompt
    /// RPCs like `initialize` and `session/new` should complete in seconds;
    /// if they don't, the agent is likely stuck and we must not block forever.
    async fn send_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ {}", &serde_json::to_string(&msg).unwrap_or_default());

        // Wrap write + read in a single timeout so a hung agent can't block forever.
        // We cannot use an async block that borrows `self` mutably across two awaits
        // inside timeout(), so we sequence them with early-return on timeout.
        let timeout = Self::REQUEST_TIMEOUT;
        match tokio::time::timeout(timeout, self.write_ndjson(&msg)).await {
            Ok(result) => result?,
            Err(_) => return Err(AcpError::Timeout(timeout)),
        }

        match tokio::time::timeout(timeout, self.read_until_response(id)).await {
            Ok(result) => result,
            Err(_) => Err(AcpError::Timeout(timeout)),
        }
    }

    /// Drain any buffered lines from the agent's stdout without blocking.
    ///
    /// After a [`AcpError::Timeout`] from [`send_request`], the agent may
    /// eventually send the late response. That stale message will sit in the
    /// `BufReader` buffer and be silently skipped by the next `read_until_response`
    /// call (ID mismatch). However, if the caller wants a clean slate — e.g.
    /// before retrying the same method — they can call this to consume any
    /// buffered data with a short deadline.
    ///
    /// This is a best-effort drain: it reads until the buffer is empty or
    /// `drain_timeout` elapses, whichever comes first. Errors are ignored.
    #[allow(dead_code)] // Scaffolding for future model-switch timeout cleanup; not yet wired.
    pub async fn drain_stale_responses(&mut self, drain_timeout: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + drain_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let read_result = tokio::time::timeout(remaining, self.reader.next()).await;
            match read_result {
                // Timeout or stream ended — buffer is empty or agent exited.
                Err(_) | Ok(None) => break,
                Ok(Some(Ok(_))) => {
                    // Consumed one buffered line; loop to drain more.
                    tracing::debug!(target: "acp::wire", "drained stale buffered line");
                }
                Ok(Some(Err(_))) => break,
            }
        }
    }

    /// Send a JSON-RPC **notification** — no `id` field, no response expected.
    ///
    /// Used for `session/cancel`. The absence of `id` is the JSON-RPC 2.0
    /// distinguisher between requests and notifications.
    async fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), AcpError> {
        // Notifications deliberately have NO "id" field.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ (notification) {}", &serde_json::to_string(&msg).unwrap_or_default());
        self.write_ndjson(&msg).await?;
        Ok(())
    }

    /// Core message loop: read NDJSON lines until we get a response matching `expected_id`.
    ///
    /// While waiting, handles:
    /// - `session/update` notifications → logged via tracing
    /// - `session/request_permission` requests → auto-approved with `allow_once`
    /// - Any other messages → debug-logged and ignored; if they carry an `id`
    ///   (i.e. they are requests, not notifications), a JSON-RPC -32601 error is sent.
    ///
    /// Compares the incoming `id` field as a `serde_json::Value` against
    /// `json!(expected_id)` so that both numeric and string IDs work correctly.
    async fn read_until_response(
        &mut self,
        expected_id: u64,
    ) -> Result<serde_json::Value, AcpError> {
        loop {
            // LinesCodec::new_with_max_length enforces MAX_LINE_SIZE at the
            // read level — the buffer never grows beyond the limit, preventing
            // OOM from rogue agents writing infinite non-newline bytes.
            let line = match self.reader.next().await {
                None => return Err(AcpError::AgentExited),
                Some(Err(LinesCodecError::MaxLineLengthExceeded)) => {
                    return Err(AcpError::Protocol(
                        "agent stdout line exceeded 10MB limit".into(),
                    ));
                }
                Some(Err(e)) => {
                    return Err(AcpError::Io(std::io::Error::other(e)));
                }
                Some(Ok(line)) => line,
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Only log and reset idle after we have a valid non-empty line.
            tracing::debug!(target: "acp::wire", "← {trimmed}");

            let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    self.observe(
                        "acp_parse_error",
                        serde_json::json!({
                            "line": trimmed,
                            "error": e.to_string(),
                        }),
                    );
                    tracing::warn!(
                        target: "acp::wire",
                        "failed to parse line as JSON: {e} — skipping"
                    );
                    continue;
                }
            };
            self.observe("acp_read", msg.clone());

            // Check if this is a response to our expected request (has matching id
            // AND no `method` field — a `method` field means it's an agent-initiated
            // request, not a response, even if the id happens to match).
            if let Some(id) = msg.get("id") {
                if *id == serde_json::json!(expected_id) && msg.get("method").is_none() {
                    if let Some(error) = msg.get("error") {
                        return Err(AcpError::AgentError(error.to_string()));
                    }
                    return Ok(msg["result"].clone());
                }
            }

            // Dispatch by method name (notifications and agent-initiated requests).
            if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                match method {
                    "session/update" => {
                        let _ = self.handle_session_update(&msg);
                    }
                    "_goose/unstable/session/update" => {
                        self.handle_goose_usage_update(&msg);
                    }
                    "session/request_permission" => {
                        self.handle_permission_request(&msg).await?;
                    }
                    other => {
                        // If the unknown message has an id, it's a request expecting a reply.
                        // Silence would cause the agent to hang waiting for a response.
                        // Send a JSON-RPC -32601 "Method not found" error.
                        if msg.get("id").is_some() {
                            let err_resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": msg["id"],
                                "error": {"code": -32601, "message": format!("Method not found: {other}")}
                            });
                            // Surface write failures — a broken pipe means the
                            // agent process is dead and continuing would hang.
                            self.write_ndjson(&err_resp).await?;
                        }
                        tracing::debug!(target: "acp::wire", "ignoring unknown method: {other}");
                    }
                }
            }
        }
    }

    /// Idle-aware message loop: like [`read_until_response`] but resets an idle
    /// deadline on every stdout line. Fires [`AcpError::IdleTimeout`] on silence
    /// or [`AcpError::HardTimeout`] on absolute wall-clock cap.
    ///
    /// `hard_deadline` is an absolute `Instant` (pre-computed by the caller) so
    /// that `cancel_with_cleanup` can inherit the remaining budget from the
    /// original turn rather than starting a fresh timer.
    /// Read agent messages until the response with `expected_id` arrives, or
    /// either of two timeouts fires. Returns `Result<value, IdleTimeout |
    /// HardTimeout | other>`.
    ///
    /// - `idle_timeout`: silent-agent guard, **reset on every line of valid
    ///   JSON** (and explicitly on `session/update` notifications).
    /// - `hard_deadline`: absolute wall-clock cap on the whole call, passed
    ///   in so that `cancel_with_cleanup` can inherit the remaining budget
    ///   from the original turn rather than starting a fresh timer.
    ///
    /// While reading, the loop interleaves goose-native non-cancelling steer
    /// requests via `tokio::select!`. The select uses `biased` for
    /// reader-first throughput, with a pre-select deadline check at the top
    /// of every loop iteration so a continuously-ready reader arm cannot
    /// starve the hard deadline (Max's review gate). The steer arm is
    /// guarded by `pending_steer.is_none()` so at most one steer is in
    /// flight at a time; a successful steer response is routed to the
    /// caller's oneshot ack instead of being returned as the prompt result.
    ///
    /// `session_id` is threaded in lexically by callers so the goose-native
    /// steer arm can complete `sessionId` in the steer JSON-RPC params at
    /// write time without needing access to outer state. See
    /// [`crate::pool::SteerRequest`] for why params are built here and not
    /// in the main loop.
    async fn read_until_response_with_idle_timeout(
        &mut self,
        session_id: &str,
        expected_id: u64,
        idle_timeout: std::time::Duration,
        hard_deadline: tokio::time::Instant,
    ) -> Result<serde_json::Value, AcpError> {
        use tokio::time::Instant;

        // Take the per-turn steer receiver into a local so it can be
        // borrowed independently of `self.reader` inside `select!`.
        // Dropped at scope exit (return paths drain `pending_steer` first
        // so the ack_tx oneshot is never leaked silently).
        let mut steer_rx = self.steer_rx.take();

        // Tracks the in-flight steer write: `(request_id, ack_tx)`. While
        // `Some`, the steer arm is gated off so we don't stack writes,
        // and a response matching `id` is routed to the ack_tx instead
        // of being treated as the prompt result. Drained on every return
        // path with `PromptCompletedNeutral` so callers are never left
        // hanging.
        let mut pending_steer: Option<(u64, tokio::sync::oneshot::Sender<crate::pool::SteerAck>)> =
            None;

        let mut idle_deadline = Instant::now() + idle_timeout;

        loop {
            // Determine which deadline fires first BEFORE sleeping — this is
            // the classification we'll use on timeout, immune to scheduler jitter.
            let idle_fires_first = idle_deadline < hard_deadline;
            let next_deadline = if idle_fires_first {
                idle_deadline
            } else {
                hard_deadline
            };

            // Pre-select deadline check — required by Max's review. Under
            // `biased`, a continuously-ready reader arm wins every poll and
            // `sleep_until(next_deadline)` is never reached, silently
            // defeating the hard-deadline guarantee for agents that keep
            // producing output (see `acp.rs:608` for why the hard deadline
            // exists). Check the classified deadline here so a steady-
            // stream agent is still bounded.
            if Instant::now() >= next_deadline {
                if let Some((_, ack_tx)) = pending_steer.take() {
                    // Prompt is timing out — release the withheld event via
                    // PromptCompletedNeutral (no fallback signal: there is
                    // no in-flight turn to signal once we return, and
                    // normal dispatch handles redelivery).
                    let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                }
                if idle_fires_first {
                    tracing::warn!("idle timeout ({idle_timeout:?}) — no agent activity");
                    return Err(AcpError::IdleTimeout(idle_timeout));
                } else {
                    tracing::warn!("hard turn timeout exceeded");
                    return Err(AcpError::HardTimeout);
                }
            }

            // LinesCodec::new_with_max_length enforces MAX_LINE_SIZE at the
            // read level — the buffer never grows beyond the limit.
            let read_result = tokio::select! {
                biased;
                read_result = self.reader.next() => Some(read_result),
                // Steer arm: gated off whenever a steer write is already in
                // flight so we don't stack two writes against the same
                // process. The `async { steer_rx.as_mut()?.recv().await }`
                // wrapper produces `None` when no receiver is installed,
                // which mismatches the `Some(req)` pattern and disables the
                // branch for that iteration (no busy loop). Cancel-safe:
                // `mpsc::Receiver::recv` does not lose messages on drop.
                Some(req) = async {
                    match steer_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => None,
                    }
                }, if pending_steer.is_none() => {
                    // Selected: build steer params at write time using the
                    // lexical `session_id` and the freshest `active_run_id`.
                    //
                    // `active_run_id` is updated by `session/update`
                    // notifications inside this very loop; reading it here
                    // (rather than snapshotting at dispatch) guarantees the
                    // value matches what goose's run-id check will compare
                    // against. If it's `None`, no `session/update` has
                    // arrived yet so we cannot form a valid `expectedRunId`
                    // — ack `ExpectedRunIdMissing` and drop the request
                    // without writing anything. The main loop maps this to
                    // the universal cancel+merge `Steer` fallback.
                    match self.active_run_id.clone() {
                        None => {
                            tracing::warn!(
                                "goose-native steer: no active_run_id at write time \
                                 (no session/update seen yet) — falling back to cancel+merge"
                            );
                            let _ = req.ack_tx.send(crate::pool::SteerAck::Err(
                                crate::pool::SteerError::ExpectedRunIdMissing,
                            ));
                        }
                        Some(run_id) => {
                            let id = self.next_id;
                            self.next_id += 1;
                            let prompt_block_refs: Vec<&str> =
                                req.prompt_blocks.iter().map(String::as_str).collect();
                            let params =
                                build_steer_params(session_id, &run_id, &prompt_block_refs);
                            let msg = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "method": "_goose/unstable/session/steer",
                                "params": params,
                            });
                            tracing::debug!(
                                target: "acp::wire",
                                "→ {}",
                                serde_json::to_string(&msg).unwrap_or_default()
                            );
                            match self.write_ndjson(&msg).await {
                                Ok(()) => {
                                    pending_steer = Some((id, req.ack_tx));
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "goose-native steer write failed: {e} — releasing withheld event"
                                    );
                                    let _ = req.ack_tx.send(crate::pool::SteerAck::Err(
                                        crate::pool::SteerError::Transport(e.to_string()),
                                    ));
                                }
                            }
                        }
                    }
                    // Loop back to the next iteration without consuming a
                    // reader line; we'll wait for either the prompt
                    // response or the steer response next.
                    None
                }
                _ = tokio::time::sleep_until(next_deadline) => {
                    // The pre-select check at the top of the next iteration
                    // would catch this anyway, but firing the deadline arm
                    // here makes the wakeup immediate (no extra reader poll
                    // round-trip when stdout is idle).
                    if let Some((_, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    if idle_fires_first {
                        tracing::warn!("idle timeout ({idle_timeout:?}) — no agent activity");
                        return Err(AcpError::IdleTimeout(idle_timeout));
                    } else {
                        tracing::warn!("hard turn timeout exceeded");
                        return Err(AcpError::HardTimeout);
                    }
                }
            };

            // Steer arm fired (or the select selected nothing read-side this
            // iteration): no reader frame to process, loop to re-evaluate
            // deadlines and arm the next select.
            let read_result = match read_result {
                Some(r) => r,
                None => continue,
            };

            match read_result {
                None => {
                    if let Some((_, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::AgentExited);
                }
                Some(Err(LinesCodecError::MaxLineLengthExceeded)) => {
                    if let Some((_, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::Protocol(
                        "agent stdout line exceeded 10MB limit".into(),
                    ));
                }
                Some(Err(e)) => {
                    if let Some((_, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::Io(std::io::Error::other(e)));
                }
                Some(Ok(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    tracing::debug!(target: "acp::wire", "← {trimmed}");

                    let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                        Ok(v) => v,
                        Err(e) => {
                            self.observe(
                                "acp_parse_error",
                                serde_json::json!({
                                    "line": trimmed,
                                    "error": e.to_string(),
                                }),
                            );
                            tracing::warn!(
                                target: "acp::wire",
                                "failed to parse line as JSON: {e} — skipping"
                            );
                            continue;
                        }
                    };
                    self.observe("acp_read", msg.clone());

                    // Only reset the idle clock on lines that parse as valid JSON.
                    // Malformed lines (skipped above) don't count as real agent activity.
                    idle_deadline = Instant::now() + idle_timeout;

                    // Steer response routing must come BEFORE the prompt
                    // response check: a steer response is a regular
                    // JSON-RPC response (id + result/error, no method),
                    // so the matcher must disambiguate by id. Both checks
                    // share the `no method` guard.
                    if let Some(id) = msg.get("id") {
                        if msg.get("method").is_none() {
                            if let Some((steer_id, _)) = pending_steer.as_ref() {
                                if *id == serde_json::json!(*steer_id) {
                                    // Take the ack_tx out and route the
                                    // response. We do not return — keep
                                    // reading until the prompt response
                                    // arrives.
                                    let (_, ack_tx) = pending_steer.take().expect("just checked");
                                    let ack = if let Some(error) = msg.get("error") {
                                        let code = error
                                            .get("code")
                                            .and_then(|c| c.as_i64())
                                            .unwrap_or(-1);
                                        let message = error.to_string();
                                        crate::pool::SteerAck::Err(
                                            crate::pool::SteerError::AgentError { code, message },
                                        )
                                    } else {
                                        crate::pool::SteerAck::Success
                                    };
                                    let _ = ack_tx.send(ack);
                                    continue;
                                }
                            }
                            if *id == serde_json::json!(expected_id) {
                                if let Some(error) = msg.get("error") {
                                    if let Some((_, ack_tx)) = pending_steer.take() {
                                        let _ = ack_tx
                                            .send(crate::pool::SteerAck::PromptCompletedNeutral);
                                    }
                                    return Err(AcpError::AgentError(error.to_string()));
                                }
                                if let Some((_, ack_tx)) = pending_steer.take() {
                                    let _ =
                                        ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                                }
                                return Ok(msg["result"].clone());
                            }
                        }
                    }

                    // Dispatch notifications and agent-initiated requests.
                    if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                        match method {
                            "session/update" => {
                                if self.handle_session_update(&msg) {
                                    // Belt-and-suspenders — general reset already fired
                                    // above, this is defense-in-depth in case the general
                                    // reset is later narrowed.
                                    tracing::debug!("idle clock reset: tool call started");
                                    idle_deadline = Instant::now() + idle_timeout;
                                }
                            }
                            "_goose/unstable/session/update" => {
                                self.handle_goose_usage_update(&msg);
                            }
                            "session/request_permission" => {
                                self.handle_permission_request(&msg).await?;
                            }
                            other => {
                                // If the unknown message has an id, it's a request expecting a reply.
                                // Silence would cause the agent to hang waiting for a response.
                                // Send a JSON-RPC -32601 "Method not found" error.
                                if msg.get("id").is_some() {
                                    let err_resp = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": msg["id"],
                                        "error": {"code": -32601, "message": format!("Method not found: {other}")}
                                    });
                                    // Surface write failures — a broken pipe means the
                                    // agent process is dead and continuing would hang.
                                    self.write_ndjson(&err_resp).await?;
                                }
                                tracing::debug!(target: "acp::wire", "ignoring unknown method: {other}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Log a `session/update` notification via tracing.
    ///
    /// The discriminator field is `sessionUpdate` (not `type`) per the ACP schema.
    /// Returns `true` if the update indicates a tool call started, signaling that
    /// the idle clock should be explicitly reset (the agent will be silent while
    /// the tool executes).
    ///
    /// Takes `&mut self` (not `&self`) because some updates carry agent state
    /// the client must observe — notably goose's `session_info_update` with
    /// `_meta.goose.activeRunId`, which seeds [`active_run_id`](Self::active_run_id)
    /// so callers can target `_goose/unstable/session/steer` at the correct run.
    fn handle_session_update(&mut self, msg: &serde_json::Value) -> bool {
        let update = &msg["params"]["update"];
        let update_type = update
            .get("sessionUpdate")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        match update_type {
            "agent_message_chunk" => {
                if let Some(text) = update["content"]["text"].as_str() {
                    tracing::info!(target: "acp::stream", "{text}");
                }
                false
            }
            "tool_call" => {
                let title = update
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let kind = update
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                tracing::info!(target: "acp::tool", "tool_call: {title} ({kind})");
                true
            }
            "tool_call_update" => {
                let tool_id = update
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                tracing::info!(target: "acp::tool", "tool_call_update: {tool_id} → {status}");
                false
            }
            "plan" => {
                tracing::info!(target: "acp::plan", "plan update received");
                false
            }
            "agent_thought_chunk" => {
                if let Some(text) = update["content"]["text"].as_str() {
                    tracing::debug!(target: "acp::thought", "{text}");
                }
                false
            }
            "available_commands_update" => {
                // Advertised slash commands (ACP slash-commands extension).
                // Logged for observability; UI surfacing is a follow-up.
                let names: Vec<&str> = update["availableCommands"]
                    .as_array()
                    .map(|cmds| cmds.iter().filter_map(|c| c["name"].as_str()).collect())
                    .unwrap_or_default();
                tracing::info!(
                    target: "acp::update",
                    "available_commands_update: {} commands [{}]",
                    names.len(),
                    names.join(", ")
                );
                false
            }
            "session_info_update" => {
                // Both goose and buzz-agent emit `session_info_update` with
                // `_meta.goose.activeRunId`: the id of the currently-active
                // prompt run, or `null` when the run has cleared. Other agents
                // don't emit this field; for them `active_run_id` stays `None`
                // and steer callers will fall back to cancel+merge.
                //
                // Per the ACP `SessionInfoUpdate` schema, `_meta` is a field
                // on the update object itself — nested inside `update`, not
                // alongside it at the params level. Goose and buzz-agent both
                // emit it at `params.update._meta.goose.activeRunId`.
                let meta = msg["params"]["update"]
                    .get("_meta")
                    .and_then(|m| m.get("goose"));
                if let Some(goose_meta) = meta {
                    match goose_meta.get("activeRunId") {
                        Some(serde_json::Value::String(run_id)) => {
                            tracing::debug!(
                                target: "acp::update",
                                "session_info_update: activeRunId={run_id}"
                            );
                            self.active_run_id = Some(run_id.clone());
                        }
                        Some(serde_json::Value::Null) => {
                            tracing::debug!(
                                target: "acp::update",
                                "session_info_update: activeRunId cleared"
                            );
                            self.active_run_id = None;
                        }
                        // Missing or non-string/null — leave state untouched.
                        _ => {}
                    }
                }
                false
            }
            "keepalive" => false,
            other => {
                tracing::debug!(target: "acp::update", "session/update: {other}");
                false
            }
        }
    }

    /// Parse a `_goose/unstable/session/update` notification and record the
    /// usage snapshot in the per-session tracker.
    ///
    /// Silently ignores malformed or non-`usage_update` variants — the
    /// notification is best-effort observability data, not a protocol
    /// requirement. Failures are logged at debug level.
    fn handle_goose_usage_update(&mut self, msg: &serde_json::Value) {
        use crate::usage::{GooseSessionUpdateNotification, GooseSessionUpdateVariant};
        let params = match msg.get("params") {
            Some(p) => p,
            None => {
                tracing::debug!(
                    target: "acp::usage",
                    "_goose/unstable/session/update: missing params"
                );
                return;
            }
        };
        match serde_json::from_value::<GooseSessionUpdateNotification>(params.clone()) {
            Ok(notif) => {
                if let GooseSessionUpdateVariant::UsageUpdate(payload) = &notif.update {
                    tracing::debug!(
                        target: "acp::usage",
                        session_id = %notif.session_id,
                        input = payload.accumulated_input_tokens,
                        output = payload.accumulated_output_tokens,
                        "goose usage update"
                    );
                    self.goose_usage.record(&notif.session_id, payload);
                }
            }
            Err(e) => {
                tracing::debug!(
                    target: "acp::usage",
                    "_goose/unstable/session/update: deserialization error: {e}"
                );
            }
        }
    }

    /// Auto-approve a `session/request_permission` request from the agent.
    ///
    /// Finds the option with `kind == "allow_once"` and responds with its `optionId`.
    /// If no `allow_once` option exists, falls back to `reject_once`.
    ///
    /// **Critical:** Never hardcode `optionId` — always find it dynamically by `kind`.
    ///
    /// The request `id` is stored as `serde_json::Value` to support both numeric
    /// and string IDs per JSON-RPC 2.0.
    async fn handle_permission_request(&mut self, msg: &serde_json::Value) -> Result<(), AcpError> {
        // Extract id as a Value — JSON-RPC 2.0 allows both numeric and string IDs.
        let id = msg
            .get("id")
            .cloned()
            .ok_or_else(|| AcpError::Protocol("permission request missing id".into()))?;

        // Store pending permission id so cancel_with_cleanup can respond to it.
        self.pending_permission_id = Some(id.clone());
        // Mark as not yet responded — guards against double-response race.
        self.permission_responded = false;

        let options = msg["params"]["options"]
            .as_array()
            .ok_or_else(|| AcpError::Protocol("permission request missing options".into()))?;

        tracing::debug!(
            target: "acp::permission",
            "session/request_permission id={id}, {} options",
            options.len()
        );

        // Find allow_once by kind — NEVER hardcode optionId.
        let allow_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"));

        let response = if let Some(opt) = allow_once {
            let option_id = opt["optionId"]
                .as_str()
                .ok_or_else(|| AcpError::Protocol("allow_once option missing optionId".into()))?;
            tracing::info!(
                target: "acp::permission",
                "auto-approving permission id={id} with allow_once optionId={option_id:?}"
            );
            permission_response_selected(&id, option_id)
        } else {
            // No allow_once — fall back to reject_once.
            tracing::warn!(
                target: "acp::permission",
                "no allow_once option found in permission request id={id}, falling back to reject_once"
            );
            let reject = options
                .iter()
                .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("reject_once"));

            if let Some(opt) = reject {
                let option_id = opt["optionId"].as_str().unwrap_or("reject");
                permission_response_selected(&id, option_id)
            } else {
                return Err(AcpError::Protocol(
                    "no suitable permission option found (neither allow_once nor reject_once)"
                        .into(),
                ));
            }
        };

        // Write the response first, then mark as responded.
        //
        // Previous ordering (flag-before-write) was intended to guard against a
        // double-response if a timeout fires between write and flag-set. However,
        // the deadlock risk is worse: if write_ndjson fails (e.g. WriteTimeout),
        // the flag would be true but no response was actually sent. Then
        // cancel_with_cleanup would see permission_responded=true, skip sending
        // the cancelled outcome, and the agent would hang waiting for a reply
        // that never arrives — a guaranteed deadlock.
        //
        // The correct fix: set the flag AFTER a successful write. The double-
        // response window (between write completion and flag-set) is negligibly
        // small and bounded by a single memory store; the deadlock window was
        // unbounded.
        self.write_ndjson(&response).await?;
        self.permission_responded = true;
        self.pending_permission_id = None;
        Ok(())
    }

    /// Parse `stopReason` from a `session/prompt` result value.
    fn parse_stop_reason(&self, result: &serde_json::Value) -> Result<StopReason, AcpError> {
        let raw = result["stopReason"].as_str().ok_or_else(|| {
            AcpError::Protocol("session/prompt response missing stopReason".into())
        })?;
        StopReason::from_str(raw)
            .ok_or_else(|| AcpError::Protocol(format!("unknown stopReason: {raw:?}")))
    }
}

/// Build `session/prompt` params from one or more text content blocks.
fn build_prompt_params(session_id: &str, prompt_blocks: &[&str]) -> serde_json::Value {
    let blocks: Vec<serde_json::Value> = prompt_blocks
        .iter()
        .map(|text| serde_json::json!({ "type": "text", "text": text }))
        .collect();
    serde_json::json!({
        "sessionId": session_id,
        "prompt": blocks,
    })
}

/// Build `_goose/unstable/session/steer` params from one or more text
/// content blocks plus the freshest `expectedRunId`.
///
/// Wire shape:
/// ```json
/// { "sessionId": "...", "expectedRunId": "...", "prompt": [{"type":"text","text":"..."}, ...] }
/// ```
///
/// Called from the read-loop steer arm at write time so `expectedRunId`
/// matches goose's *current* run (it advances on each `session/update`).
/// See [`crate::pool::SteerRequest`] for why this is the read loop's job
/// and not the main loop's.
fn build_steer_params(
    session_id: &str,
    expected_run_id: &str,
    prompt_blocks: &[&str],
) -> serde_json::Value {
    let blocks: Vec<serde_json::Value> = prompt_blocks
        .iter()
        .map(|text| serde_json::json!({ "type": "text", "text": text }))
        .collect();
    serde_json::json!({
        "sessionId": session_id,
        "expectedRunId": expected_run_id,
        "prompt": blocks,
    })
}

/// Build a JSON-RPC permission response with `outcome: "selected"`.
fn permission_response_selected(id: &serde_json::Value, option_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "selected", "optionId": option_id } }
    })
}

/// Build a JSON-RPC permission response with `outcome: "cancelled"`.
fn permission_response_cancelled(id: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "cancelled" } }
    })
}

/// Full `session/new` response — session ID plus the raw JSON result.
///
/// Callers use the extractor helpers to pull model info from `raw`.
pub struct SessionNewResponse {
    pub session_id: String,
    /// The full `result` value from the JSON-RPC response.
    pub raw: serde_json::Value,
}

/// How to switch to a particular model on a session.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type")]
pub enum ModelSwitchMethod {
    /// Stable: use `session/set_config_option` with these exact values.
    ConfigOption {
        config_id: String,
        option_value: String,
    },
    /// Unstable: use `session/set_model` with this model_id.
    SetModel { model_id: String },
}

/// Extract `configOptions` entries with `category == "model"` from a `session/new` result.
///
/// Returns the raw JSON array entries. Each entry has `configId`, `displayName`,
/// `options: [{ value, displayName }]`, etc.
pub fn extract_model_config_options(result: &serde_json::Value) -> Vec<serde_json::Value> {
    result["configOptions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|opt| opt.get("category").and_then(|c| c.as_str()) == Some("model"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Extract `SessionModelState` (unstable path) from a `session/new` result.
///
/// Returns the `models` object if present: `{ currentModelId, availableModels: [...] }`.
pub fn extract_model_state(result: &serde_json::Value) -> Option<serde_json::Value> {
    result.get("models").cloned()
}

/// Match a desired model ID against a fresh `session/new` response.
///
/// Returns the correct ACP method to call, or `None` if no match.
///
/// **Precedence**: stable `configOptions` first (spec-blessed), then unstable
/// `availableModels`. The fresh `session/new` response is always authoritative.
pub fn resolve_model_switch_method(
    session_new_result: &serde_json::Value,
    desired_model: &str,
) -> Option<ModelSwitchMethod> {
    // 1. Search stable configOptions for a "model"-category entry whose
    //    options contain a value matching desired_model.
    for config_opt in extract_model_config_options(session_new_result) {
        let config_id = match config_opt.get("configId").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => continue,
        };
        if let Some(options) = config_opt.get("options").and_then(|v| v.as_array()) {
            for opt in options {
                if opt.get("value").and_then(|v| v.as_str()) == Some(desired_model) {
                    return Some(ModelSwitchMethod::ConfigOption {
                        config_id: config_id.to_string(),
                        option_value: desired_model.to_string(),
                    });
                }
            }
        }
    }

    // 2. Search unstable availableModels for a matching modelId.
    if let Some(models) = extract_model_state(session_new_result) {
        if let Some(available) = models.get("availableModels").and_then(|v| v.as_array()) {
            for model in available {
                if model.get("modelId").and_then(|v| v.as_str()) == Some(desired_model) {
                    return Some(ModelSwitchMethod::SetModel {
                        model_id: desired_model.to_string(),
                    });
                }
            }
        }
    }

    // 3. No match.
    None
}

/// Whether `desired_model` appears in pre-extracted catalog halves.
///
/// Mirrors [`resolve_model_switch_method`]'s match, but operates on the
/// already-extracted `configOptions` (model category) and `models` state that
/// [`AgentModelCapabilities`](crate::pool::AgentModelCapabilities) caches — the
/// idle-path pre-cancel guard has those halves, not the full `session/new` JSON.
pub fn model_in_catalog(
    config_options: &[serde_json::Value],
    available_models: Option<&serde_json::Value>,
    desired_model: &str,
) -> bool {
    let in_config_options = config_options.iter().any(|config_opt| {
        config_opt
            .get("options")
            .and_then(|v| v.as_array())
            .is_some_and(|options| {
                options
                    .iter()
                    .any(|opt| opt.get("value").and_then(|v| v.as_str()) == Some(desired_model))
            })
    });
    if in_config_options {
        return true;
    }

    available_models
        .and_then(|models| models.get("availableModels"))
        .and_then(|v| v.as_array())
        .is_some_and(|available| {
            available
                .iter()
                .any(|model| model.get("modelId").and_then(|v| v.as_str()) == Some(desired_model))
        })
}

// ─── Drop: kill child process ─────────────────────────────────────────────────

impl Drop for AcpClient {
    fn drop(&mut self) {
        // Best-effort SIGKILL + reap. We cannot `await` in Drop (sync context).
        // Kill the process group when possible so subprocesses don't leak.
        // Callers SHOULD still call `shutdown().await` for guaranteed reaping.
        match self.child.id() {
            Some(pid) if kill_process_group(pid) => {}
            _ => {
                let _ = self.child.start_kill();
            }
        }
        // Non-blocking reap attempt — prevents zombie accumulation in the
        // common case where SIGKILL takes effect before Drop returns.
        let _ = self.child.try_wait();
    }
}

/// Send SIGKILL to an entire process group. Returns `true` if the signal was sent.
///
/// The child is spawned with `process_group(0)`, so its PID equals its PGID.
/// Killing the group ensures subprocesses (MCP servers, tool processes) are
/// cleaned up rather than orphaned to init on repeated crash-recovery cycles.
///
/// Uses `nix::sys::signal::killpg` — a safe wrapper around the POSIX `killpg`
/// syscall — so the crate's `#![deny(unsafe_code)]` policy is preserved.
#[cfg(unix)]
fn kill_process_group(pid: u32) -> bool {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;

    // pid == pgid because the child was spawned with process_group(0).
    killpg(Pid::from_raw(pid as i32), Signal::SIGKILL).is_ok()
}

/// Fallback for non-Unix: process-group kill not available.
/// Returns `false` so the caller falls back to `child.start_kill()`.
#[cfg(not(unix))]
fn kill_process_group(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reason_parses_all_known_values() {
        assert_eq!(StopReason::from_str("end_turn"), Some(StopReason::EndTurn));
        assert_eq!(
            StopReason::from_str("cancelled"),
            Some(StopReason::Cancelled)
        );
        assert_eq!(
            StopReason::from_str("max_tokens"),
            Some(StopReason::MaxTokens)
        );
        assert_eq!(
            StopReason::from_str("max_turn_requests"),
            Some(StopReason::MaxTurnRequests)
        );
        assert_eq!(StopReason::from_str("refusal"), Some(StopReason::Refusal));
    }

    #[test]
    fn stop_reason_returns_none_for_unknown() {
        assert_eq!(StopReason::from_str("unknown_value"), None);
        assert_eq!(StopReason::from_str(""), None);
        assert_eq!(StopReason::from_str("endturn"), None); // no camelCase — still unknown
    }

    #[test]
    fn stop_reason_is_case_insensitive() {
        // Agents may send uppercase or mixed-case variants — all should parse correctly.
        assert_eq!(StopReason::from_str("END_TURN"), Some(StopReason::EndTurn));
        assert_eq!(
            StopReason::from_str("CANCELLED"),
            Some(StopReason::Cancelled)
        );
        assert_eq!(
            StopReason::from_str("Max_Tokens"),
            Some(StopReason::MaxTokens)
        );
        assert_eq!(
            StopReason::from_str("MAX_TURN_REQUESTS"),
            Some(StopReason::MaxTurnRequests)
        );
        assert_eq!(StopReason::from_str("Refusal"), Some(StopReason::Refusal));
    }

    #[test]
    fn find_allow_once_by_kind_not_by_option_id() {
        // optionId values are intentionally non-obvious to prove we don't hardcode them.
        let options: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
            {"optionId": "opt-reject-42",  "name": "Reject",       "kind": "reject_once"},
            {"optionId": "opt-allow-99",   "name": "Allow once",   "kind": "allow_once"},
            {"optionId": "opt-always-7",   "name": "Always allow", "kind": "allow_always"}
        ]"#,
        )
        .unwrap();

        let allow_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"));

        assert!(allow_once.is_some(), "should find allow_once option");
        let opt = allow_once.unwrap();
        // Found by kind, not by hardcoded optionId
        assert_eq!(opt["kind"].as_str(), Some("allow_once"));
        assert_eq!(opt["optionId"].as_str(), Some("opt-allow-99"));
    }

    #[test]
    fn find_allow_once_returns_none_when_absent() {
        let options: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
            {"optionId": "reject-1",      "name": "Reject",        "kind": "reject_once"},
            {"optionId": "reject-always", "name": "Always reject", "kind": "reject_always"}
        ]"#,
        )
        .unwrap();

        let allow_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"));

        assert!(allow_once.is_none());
    }

    #[test]
    fn find_reject_once_fallback_when_no_allow_once() {
        let options: Vec<serde_json::Value> = serde_json::from_str(
            r#"[{"optionId": "rej-x", "name": "Reject", "kind": "reject_once"}]"#,
        )
        .unwrap();

        let allow_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"));
        assert!(allow_once.is_none());

        let reject_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("reject_once"));
        assert!(reject_once.is_some());
        assert_eq!(reject_once.unwrap()["optionId"].as_str(), Some("rej-x"));
    }

    #[test]
    fn request_has_id_field() {
        let id: u64 = 42;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {}
        });
        assert!(msg.get("id").is_some(), "request must have id field");
        assert_eq!(msg["id"].as_u64(), Some(42));
        assert_eq!(msg["jsonrpc"].as_str(), Some("2.0"));
        assert_eq!(msg["method"].as_str(), Some("initialize"));
    }

    #[test]
    fn notification_has_no_id_field() {
        // session/cancel is a notification — must NOT have an id field.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {
                "sessionId": "sess_abc123"
            }
        });
        assert!(
            msg.get("id").is_none(),
            "notification must NOT have id field"
        );
        assert_eq!(msg["jsonrpc"].as_str(), Some("2.0"));
        assert_eq!(msg["method"].as_str(), Some("session/cancel"));
    }

    #[test]
    fn initialize_request_format() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0u64,
            "method": "initialize",
            "params": {
                "protocolVersion": 2,
                "clientCapabilities": {
                    "_meta": {
                        "goose": {
                            "customNotifications": true
                        }
                    }
                },
                "clientInfo": {
                    "name": "buzz-acp",
                    "version": "0.1.0"
                }
            }
        });
        assert_eq!(msg["params"]["protocolVersion"].as_u64(), Some(2));
        assert_eq!(
            msg["params"]["clientInfo"]["name"].as_str(),
            Some("buzz-acp")
        );
        assert!(msg["params"]["clientCapabilities"].is_object());
        assert_eq!(
            msg["params"]["clientCapabilities"]["_meta"]["goose"]["customNotifications"].as_bool(),
            Some(true),
            "goose customNotifications capability must be advertised"
        );
    }

    #[test]
    fn session_new_mcp_server_has_required_fields() {
        // Schema requires name, command, args, env — all present, args/env may be empty.
        let server = McpServer {
            name: "test-mcp".into(),
            command: "/usr/local/bin/test-mcp-server".into(),
            args: vec![],
            env: vec![
                EnvVar {
                    name: "BUZZ_RELAY_URL".into(),
                    value: "ws://localhost:3000".into(),
                },
                EnvVar {
                    name: "BUZZ_PRIVATE_KEY".into(),
                    value: "nsec1abc".into(),
                },
            ],
        };
        let serialized = serde_json::to_value(&server).unwrap();
        assert_eq!(serialized["name"].as_str(), Some("test-mcp"));
        assert_eq!(
            serialized["command"].as_str(),
            Some("/usr/local/bin/test-mcp-server")
        );
        assert!(serialized["args"].is_array());
        assert_eq!(serialized["args"].as_array().unwrap().len(), 0);
        assert!(serialized["env"].is_array());
        assert_eq!(serialized["env"].as_array().unwrap().len(), 2);
        assert_eq!(
            serialized["env"][0]["name"].as_str(),
            Some("BUZZ_RELAY_URL")
        );
    }

    #[test]
    fn session_prompt_request_format() {
        let prompt_text = "[Buzz @mention]\nChannel: test\nFrom: npub1...\nMessage: hello";
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2u64,
            "method": "session/prompt",
            "params": {
                "sessionId": "sess_abc123",
                "prompt": [
                    { "type": "text", "text": prompt_text }
                ]
            }
        });
        assert_eq!(msg["method"].as_str(), Some("session/prompt"));
        let prompt = msg["params"]["prompt"].as_array().unwrap();
        assert_eq!(prompt.len(), 1);
        assert_eq!(prompt[0]["type"].as_str(), Some("text"));
        assert_eq!(prompt[0]["text"].as_str(), Some(prompt_text));
    }

    #[test]
    fn session_prompt_slash_command_two_block_format() {
        // Slash-command pass-through: bare command first, wrapped context second.
        let params = build_prompt_params(
            "sess_abc123",
            &[
                "/goal ship it",
                "[Buzz event: @mention]\nContent: @Eva /goal ship it",
            ],
        );
        let prompt = params["prompt"].as_array().unwrap();
        assert_eq!(prompt.len(), 2);
        assert_eq!(prompt[0]["type"].as_str(), Some("text"));
        assert_eq!(prompt[0]["text"].as_str(), Some("/goal ship it"));
        assert!(prompt[0]["text"].as_str().unwrap().starts_with('/'));
        assert_eq!(prompt[1]["type"].as_str(), Some("text"));
    }

    #[test]
    fn permission_response_selected_format() {
        let id: u64 = 5;
        let option_id = "opt-allow-99";
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "outcome": {
                    "outcome": "selected",
                    "optionId": option_id
                }
            }
        });
        assert_eq!(response["id"].as_u64(), Some(5));
        assert_eq!(
            response["result"]["outcome"]["outcome"].as_str(),
            Some("selected")
        );
        assert_eq!(
            response["result"]["outcome"]["optionId"].as_str(),
            Some("opt-allow-99")
        );
    }

    #[test]
    fn permission_response_cancelled_format() {
        let id: u64 = 5;
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "outcome": {
                    "outcome": "cancelled"
                }
            }
        });
        assert_eq!(
            response["result"]["outcome"]["outcome"].as_str(),
            Some("cancelled")
        );
        // cancelled outcome has no optionId
        assert!(response["result"]["outcome"].get("optionId").is_none());
    }

    #[test]
    fn session_cancel_notification_has_session_id_in_params() {
        let session_id = "sess_xyz789";
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {
                "sessionId": session_id
            }
        });
        // Must have no id (notification)
        assert!(msg.get("id").is_none());
        // Must have sessionId in params
        assert_eq!(msg["params"]["sessionId"].as_str(), Some("sess_xyz789"));
    }

    #[test]
    fn permission_request_with_string_id() {
        // Verify that permission response uses the same ID type as the request.
        // JSON-RPC 2.0 permits string IDs from the agent.
        let string_id = serde_json::json!("perm-req-001");
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": string_id,
            "result": {
                "outcome": { "outcome": "selected", "optionId": "allow-once" }
            }
        });
        assert_eq!(response["id"], "perm-req-001");
        assert!(response["id"].is_string());
    }

    #[test]
    fn id_comparison_works_for_numeric_and_string() {
        // Verify json!(expected_id) comparison logic used in read_until_response.
        let expected_id: u64 = 3;
        let numeric_response_id = serde_json::json!(3u64);
        let string_response_id = serde_json::json!("3");

        // Numeric matches
        assert_eq!(numeric_response_id, serde_json::json!(expected_id));
        // String does NOT match numeric (correct — different types)
        assert_ne!(string_response_id, serde_json::json!(expected_id));
    }

    #[test]
    fn permission_cancelled_response_preserves_id_type() {
        // String ID from agent should be echoed back as string in cancelled response.
        let string_id = serde_json::json!("req-abc");
        let cancelled = serde_json::json!({
            "jsonrpc": "2.0",
            "id": string_id.clone(),
            "result": { "outcome": { "outcome": "cancelled" } }
        });
        assert_eq!(cancelled["id"], string_id);
        assert!(cancelled["id"].is_string());

        // Numeric ID from agent should be echoed back as numeric.
        let numeric_id = serde_json::json!(42u64);
        let cancelled_numeric = serde_json::json!({
            "jsonrpc": "2.0",
            "id": numeric_id.clone(),
            "result": { "outcome": { "outcome": "cancelled" } }
        });
        assert_eq!(cancelled_numeric["id"], numeric_id);
        assert!(cancelled_numeric["id"].is_number());
    }

    #[test]
    fn extract_model_config_options_finds_model_category() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "configOptions": [
                {
                    "configId": "model",
                    "category": "model",
                    "displayName": "Model",
                    "options": [
                        { "value": "claude-sonnet-4-20250514", "displayName": "Claude Sonnet 4" },
                        { "value": "claude-opus-4-20250514", "displayName": "Claude Opus 4" }
                    ]
                },
                {
                    "configId": "theme",
                    "category": "appearance",
                    "displayName": "Theme",
                    "options": [{ "value": "dark", "displayName": "Dark" }]
                }
            ]
        });
        let opts = super::extract_model_config_options(&result);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0]["configId"].as_str(), Some("model"));
    }

    #[test]
    fn extract_model_config_options_empty_when_no_config_options() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::extract_model_config_options(&result).is_empty());
    }

    #[test]
    fn extract_model_config_options_empty_when_no_model_category() {
        let result = serde_json::json!({
            "configOptions": [
                { "configId": "theme", "category": "appearance" }
            ]
        });
        assert!(super::extract_model_config_options(&result).is_empty());
    }

    #[test]
    fn extract_model_state_returns_models_object() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "models": {
                "currentModelId": "gpt-5",
                "availableModels": [
                    { "modelId": "gpt-5", "name": "GPT-5" },
                    { "modelId": "o3-pro", "name": "o3 Pro" }
                ]
            }
        });
        let ms = super::extract_model_state(&result).expect("should have models");
        assert_eq!(ms["currentModelId"].as_str(), Some("gpt-5"));
        assert_eq!(ms["availableModels"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn extract_model_state_none_when_absent() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::extract_model_state(&result).is_none());
    }

    #[test]
    fn resolve_prefers_stable_over_unstable() {
        let result = serde_json::json!({
            "configOptions": [{
                "configId": "model",
                "category": "model",
                "options": [
                    { "value": "claude-sonnet-4-20250514", "displayName": "Sonnet 4" }
                ]
            }],
            "models": {
                "currentModelId": "claude-sonnet-4-20250514",
                "availableModels": [
                    { "modelId": "claude-sonnet-4-20250514", "name": "Sonnet 4" }
                ]
            }
        });
        let method = super::resolve_model_switch_method(&result, "claude-sonnet-4-20250514");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "model".to_string(),
                option_value: "claude-sonnet-4-20250514".to_string(),
            })
        );
    }

    #[test]
    fn resolve_falls_back_to_unstable() {
        let result = serde_json::json!({
            "models": {
                "currentModelId": "gpt-5",
                "availableModels": [
                    { "modelId": "gpt-5", "name": "GPT-5" },
                    { "modelId": "o3-pro", "name": "o3 Pro" }
                ]
            }
        });
        let method = super::resolve_model_switch_method(&result, "o3-pro");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::SetModel {
                model_id: "o3-pro".to_string(),
            })
        );
    }

    #[test]
    fn resolve_returns_none_when_no_match() {
        let result = serde_json::json!({
            "configOptions": [{
                "configId": "model",
                "category": "model",
                "options": [{ "value": "claude-sonnet-4-20250514" }]
            }],
            "models": {
                "availableModels": [{ "modelId": "gpt-5" }]
            }
        });
        assert!(super::resolve_model_switch_method(&result, "nonexistent-model").is_none());
    }

    #[test]
    fn resolve_returns_none_when_no_model_info() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::resolve_model_switch_method(&result, "anything").is_none());
    }

    #[test]
    fn resolve_handles_multiple_config_options() {
        // Agent could have multiple configOptions with category "model"
        // (unlikely but defensive).
        let result = serde_json::json!({
            "configOptions": [
                {
                    "configId": "primary-model",
                    "category": "model",
                    "options": [{ "value": "model-a" }]
                },
                {
                    "configId": "fallback-model",
                    "category": "model",
                    "options": [{ "value": "model-b" }]
                }
            ]
        });
        let method = super::resolve_model_switch_method(&result, "model-b");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "fallback-model".to_string(),
                option_value: "model-b".to_string(),
            })
        );
    }

    // ── model_in_catalog tests ────────────────────────────────────────────

    #[test]
    fn model_in_catalog_true_when_in_config_options() {
        let config_options = vec![serde_json::json!({
            "configId": "model",
            "category": "model",
            "options": [
                { "value": "claude-sonnet-4-20250514" },
                { "value": "claude-opus-4-20250514" }
            ]
        })];
        assert!(super::model_in_catalog(
            &config_options,
            None,
            "claude-opus-4-20250514"
        ));
    }

    #[test]
    fn model_in_catalog_true_when_in_available_models() {
        let available = serde_json::json!({
            "currentModelId": "gpt-5",
            "availableModels": [
                { "modelId": "gpt-5" },
                { "modelId": "o3-pro" }
            ]
        });
        assert!(super::model_in_catalog(&[], Some(&available), "o3-pro"));
    }

    #[test]
    fn model_in_catalog_false_when_absent_from_both_halves() {
        let config_options = vec![serde_json::json!({
            "configId": "model",
            "options": [{ "value": "claude-sonnet-4-20250514" }]
        })];
        let available = serde_json::json!({
            "availableModels": [{ "modelId": "gpt-5" }]
        });
        assert!(!super::model_in_catalog(
            &config_options,
            Some(&available),
            "nonexistent-model"
        ));
    }

    #[test]
    fn model_in_catalog_false_when_both_halves_empty() {
        assert!(!super::model_in_catalog(&[], None, "anything"));
    }

    // ── Error variant display ─────────────────────────────────────────────

    #[test]
    fn idle_timeout_error_includes_duration() {
        let err = AcpError::IdleTimeout(std::time::Duration::from_secs(320));
        let msg = err.to_string();
        assert!(
            msg.contains("320"),
            "IdleTimeout display should include duration: {msg}"
        );
    }

    #[test]
    fn hard_timeout_error_display() {
        let err = AcpError::HardTimeout;
        let msg = err.to_string();
        assert!(
            msg.contains("Hard turn timeout"),
            "HardTimeout display: {msg}"
        );
    }

    async fn spawn_script(script: &str) -> AcpClient {
        AcpClient::spawn("bash", &["-c".into(), script.into()], &[])
            .await
            .expect("failed to spawn test script")
    }

    #[tokio::test]
    async fn idle_timeout_fires_on_silent_process() {
        let mut client = spawn_script("sleep 10").await;
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(100),
                hard_deadline,
            )
            .await;
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout, got {result:?}"
        );
    }

    #[tokio::test]
    async fn hard_timeout_fires_when_deadline_is_immediate() {
        let mut client = spawn_script("while true; do echo 'noise'; sleep 0.01; done").await;
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_secs(60),
                hard_deadline,
            )
            .await;
        assert!(
            matches!(result, Err(AcpError::HardTimeout)),
            "expected HardTimeout, got {result:?}"
        );
    }

    #[tokio::test]
    async fn idle_resets_on_stdout_activity() {
        // Send valid JSON (session/update notifications) to reset the idle timer.
        // Non-JSON lines no longer reset idle (Finding #6 hardening).
        let mut client = spawn_script(
            r#"for i in $(seq 1 10); do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"text":"thinking"}}}}'; sleep 0.05; done; sleep 10"#,
        )
        .await;
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(200),
                hard_deadline,
            )
            .await;
        let elapsed = start.elapsed();
        // 10 messages × 50ms = ~500ms of activity, then idle timeout fires after 200ms more
        assert!(elapsed >= std::time::Duration::from_millis(400));
        assert!(elapsed < std::time::Duration::from_secs(3));
        assert!(matches!(result, Err(AcpError::IdleTimeout(_))));
    }

    #[tokio::test]
    async fn response_returned_when_matching_id_arrives() {
        let mut client =
            spawn_script(r#"echo '{"jsonrpc":"2.0","id":42,"result":{"stopReason":"end_turn"}}'"#)
                .await;
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                42,
                std::time::Duration::from_secs(2),
                hard_deadline,
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["stopReason"].as_str(), Some("end_turn"));
    }

    #[tokio::test]
    async fn agent_exit_detected_as_eof() {
        let mut client = spawn_script("exit 0").await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_secs(2),
                hard_deadline,
            )
            .await;
        assert!(matches!(result, Err(AcpError::AgentExited)));
    }

    /// A message with both `id` and `method` is an agent-initiated request,
    /// not a response. The response matcher must not consume it even if the
    /// id happens to match the expected value.
    #[tokio::test]
    async fn agent_request_with_matching_id_not_consumed_as_response() {
        // The script sends an agent-initiated request (has both id and method)
        // whose id matches what we're waiting for (0), then sends the real
        // response. The request should be dispatched (triggering -32601 since
        // "test/method" is unknown), and the real response should be returned.
        let script = r#"
            echo '{"jsonrpc":"2.0","id":0,"method":"test/method","params":{}}'
            read -t 2 _reply
            echo '{"jsonrpc":"2.0","id":0,"result":{"ok":true}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                0,
                std::time::Duration::from_secs(3),
                hard_deadline,
            )
            .await;
        assert!(result.is_ok(), "expected Ok response, got {result:?}");
        assert_eq!(result.unwrap()["ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn idle_fires_before_hard_when_idle_is_shorter() {
        let mut client = spawn_script("sleep 10").await;
        let idle = std::time::Duration::from_millis(100);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let result = client
            .read_until_response_with_idle_timeout("test", 999, idle, hard_deadline)
            .await;
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "idle should fire before hard when idle << hard, got {result:?}"
        );
    }

    /// Hard-deadline starvation regression (Max's review gate, Eva's required test).
    ///
    /// When the read-loop became a `tokio::select!` with `biased; reader →
    /// steer → sleep_until`, a continuously-ready reader arm could win every
    /// poll and starve the timer arm — silently defeating the hard-deadline
    /// guarantee. The fix is a pre-select deadline check at the top of every
    /// loop iteration; this test pins that behavior.
    ///
    /// Setup: agent emits a **gapless** stream of valid JSON `session/update`
    /// notifications (no `sleep` between lines) so the reader arm is
    /// continuously ready. Each line is valid JSON, so it resets the idle
    /// clock — and we set idle ≫ hard so idle cannot fire first. With
    /// `biased; reader → steer → sleep_until`, the reader arm would win
    /// every poll and `sleep_until` would never be reached. Only the
    /// pre-select deadline check at the top of the loop can stop us.
    ///
    /// Without the pre-select check, this test hangs against the infinite
    /// bash subprocess until the test harness's own outer timeout, and the
    /// returned error would never be `HardTimeout`.
    #[tokio::test]
    async fn hard_deadline_fires_under_continuous_valid_json_stream() {
        // Truly infinite, gapless stream of valid JSON. No `sleep` between
        // echoes — the reader arm is continuously ready, which is the
        // exact starvation scenario the pre-select check guards against.
        // `while :; do echo ...; done` (not a fixed-count `for`) so the
        // subprocess never naturally exits before the hard deadline,
        // regardless of how fast the host drains bash output. Without
        // this, fast hardware drains a bounded loop in < hard_deadline
        // and the reader hits EOF (`AgentExited`) before the timer fires,
        // masking whether the pre-select check actually works.
        let mut client = spawn_script(
            r#"while :; do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"x"}}}}'; done"#,
        )
        .await;
        let hard = std::time::Duration::from_millis(300);
        let hard_deadline = tokio::time::Instant::now() + hard;
        let idle = std::time::Duration::from_secs(60); // idle ≫ hard
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout("test", 999, idle, hard_deadline)
            .await;
        let elapsed = start.elapsed();
        assert!(
            matches!(result, Err(AcpError::HardTimeout)),
            "expected HardTimeout under gapless valid-JSON stream, got {result:?} (elapsed {elapsed:?})"
        );
        // Must fire close to the hard deadline, not late. Without the
        // pre-select check the reader arm starves sleep_until and elapsed
        // tracks the bash subprocess lifetime instead.
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "HardTimeout fired late ({elapsed:?}); reader arm may be starving sleep_until"
        );
    }

    /// Same as `agent_request_with_matching_id_not_consumed_as_response` but
    /// exercises the non-idle `read_until_response` path (via `send_request`).
    #[tokio::test]
    async fn agent_request_not_consumed_via_send_request() {
        // Script: wait for the initialize request, reply, then send an
        // agent-initiated request with id=1 (matching the next send_request id),
        // wait for the -32601 error reply, then send the real response.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 _req
            echo '{"jsonrpc":"2.0","id":1,"method":"test/unknown","params":{}}'
            read -t 2 _err_reply
            echo '{"jsonrpc":"2.0","id":1,"result":{"worked":true}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        // initialize consumes id=0
        let _init = client
            .initialize()
            .await
            .expect("initialize should succeed");
        // send_request uses id=1 — the agent's request with id=1 and method
        // must not be consumed as the response.
        let result = client
            .send_request("test/echo", serde_json::json!({}))
            .await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert_eq!(result.unwrap()["worked"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn keepalive_resets_idle_past_deadline() {
        // Keepalive session/update lines every 50ms against a 100ms idle deadline.
        // The turn should survive well past the 100ms deadline (proves the fix).
        let mut client = spawn_script(
            r#"for i in $(seq 1 20); do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"keepalive"}}}'; sleep 0.05; done; sleep 10"#,
        )
        .await;
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(100),
                hard_deadline,
            )
            .await;
        let elapsed = start.elapsed();
        // 20 keepalives × 50ms = ~1000ms of activity, then idle fires after 100ms more.
        // Must survive well past the 100ms deadline.
        assert!(
            elapsed >= std::time::Duration::from_millis(500),
            "keepalive should reset idle past the deadline; elapsed only {elapsed:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(5));
        assert!(matches!(result, Err(AcpError::IdleTimeout(_))));
    }

    #[tokio::test]
    async fn tool_call_resets_idle_then_silence_times_out() {
        // A tool_call session/update resets the idle timer (belt-and-suspenders path),
        // then silence causes idle timeout. This proves the reset works for tool_call
        // specifically — not just via the general valid-JSON reset at line 839.
        //
        // The script emits a tool_call, waits 80ms (under the 200ms idle), then goes
        // silent. If the tool_call reset didn't fire, idle would fire at 200ms from
        // start. With the reset, idle fires at 80ms + 200ms = ~280ms from start.
        let mut client = spawn_script(
            r#"echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call","title":"long_running","kind":"shell"}}}'; sleep 0.08; sleep 10"#,
        )
        .await;
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(200),
                hard_deadline,
            )
            .await;
        let elapsed = start.elapsed();
        // The tool_call arrives near-instantly and resets idle.
        // Then 80ms of silence, then idle fires at ~280ms from start.
        // Must be > 200ms (proves the reset happened after the tool_call).
        assert!(
            elapsed >= std::time::Duration::from_millis(200),
            "tool_call should reset idle; elapsed only {elapsed:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(2));
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout after silence, got {result:?}"
        );
    }

    #[tokio::test]
    async fn session_new_full_includes_system_prompt_when_some() {
        // Script: respond to initialize, then echo back the session/new request.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], Some("Custom system prompt"))
            .await
            .expect("session_new_full should succeed");

        assert_eq!(resp.session_id, "ses_test");
        let received = &resp.raw["_receivedRequest"];
        assert_eq!(
            received["params"]["systemPrompt"].as_str(),
            Some("Custom system prompt"),
            "systemPrompt should be included in params when Some"
        );
    }

    #[tokio::test]
    async fn session_new_full_omits_system_prompt_when_none() {
        // When system_prompt is None, the field should not appear in params.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], None)
            .await
            .expect("session_new_full should succeed");

        assert_eq!(resp.session_id, "ses_test");
        let received = &resp.raw["_receivedRequest"];
        assert!(
            received["params"]["systemPrompt"].is_null(),
            "systemPrompt should NOT be in params when value is None"
        );
    }

    // ── Goose-native steer scaffold (PR follow-up to #1160) ──────────────

    /// Helper: spawn an inert `cat` subprocess so we have a real AcpClient
    /// to drive `handle_session_update` against. `cat` never writes back,
    /// which is fine — these tests don't read from the agent, they just
    /// feed JSON into the parser.
    async fn spawn_inert_client() -> AcpClient {
        AcpClient::spawn("cat", &[], &[])
            .await
            .expect("spawn cat as inert client")
    }

    /// Build a `session/update` JSON-RPC notification carrying a
    /// `session_info_update` with the given `_meta.goose.activeRunId` value.
    /// Pass `None` to omit the `activeRunId` field entirely.
    ///
    /// `_meta` is nested inside the `update` object (per the ACP
    /// `SessionInfoUpdate` schema), matching what goose and buzz-agent
    /// emit on the wire.
    fn session_info_update_msg(active_run_id: Option<serde_json::Value>) -> serde_json::Value {
        let mut goose = serde_json::Map::new();
        if let Some(v) = active_run_id {
            goose.insert("activeRunId".to_string(), v);
        }
        let mut meta = serde_json::Map::new();
        meta.insert("goose".to_string(), serde_json::Value::Object(goose));
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "test-session",
                "update": {
                    "sessionUpdate": "session_info_update",
                    "_meta": serde_json::Value::Object(meta),
                },
            }
        })
    }

    #[tokio::test]
    async fn active_run_id_sets_on_string() {
        let mut client = spawn_inert_client().await;
        assert!(client.active_run_id().is_none(), "starts as None");

        let msg = session_info_update_msg(Some(serde_json::json!("run-abc-123")));
        let _ = client.handle_session_update(&msg);

        assert_eq!(client.active_run_id(), Some("run-abc-123"));
    }

    #[tokio::test]
    async fn active_run_id_clears_on_null() {
        let mut client = spawn_inert_client().await;
        // Set it first
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-xyz")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-xyz"));

        // Then clear with explicit null
        let clear_msg = session_info_update_msg(Some(serde_json::Value::Null));
        let _ = client.handle_session_update(&clear_msg);
        assert!(
            client.active_run_id().is_none(),
            "explicit null must clear active_run_id"
        );
    }

    #[tokio::test]
    async fn active_run_id_untouched_when_missing() {
        // Field absent entirely — must NOT clear existing state (only an
        // explicit null clears; missing means "no new info this update").
        let mut client = spawn_inert_client().await;
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-stable")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-stable"));

        // session_info_update with no activeRunId field — leave state alone.
        let missing_msg = session_info_update_msg(None);
        let _ = client.handle_session_update(&missing_msg);
        assert_eq!(
            client.active_run_id(),
            Some("run-stable"),
            "missing activeRunId must leave state untouched"
        );
    }

    #[tokio::test]
    async fn active_run_id_untouched_on_wrong_type() {
        // A number or object in activeRunId is malformed — neither set nor clear.
        let mut client = spawn_inert_client().await;
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-stable")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-stable"));

        let wrong_type_msg = session_info_update_msg(Some(serde_json::json!(42)));
        let _ = client.handle_session_update(&wrong_type_msg);
        assert_eq!(
            client.active_run_id(),
            Some("run-stable"),
            "non-string/non-null activeRunId must leave state untouched"
        );
    }

    // ── Goose-native steer arm tests ──────────────────────────────────────
    //
    // These exercise the seam between `install_steer_rx` and the read
    // loop's steer arm, isolated from `AgentPool` / `EventQueue` /
    // dispatch. They prove the locked Option-X contract at the read-loop
    // boundary:
    //   1. With `active_run_id == None`, the steer arm acks
    //      `Err(ExpectedRunIdMissing)` and writes nothing — the main
    //      loop's "Err-before-pending" fallback path is reachable.
    //   2. With `active_run_id` set, the steer arm writes the JSON-RPC
    //      request with the matching `expectedRunId` and routes the
    //      response to the ack oneshot as `Success`.
    //
    // We don't test the full mode-gate fork here — that lives in lib.rs
    // and is covered by goose e2e (Eva's lane).

    /// Steer with no `active_run_id` set acks `ExpectedRunIdMissing`
    /// without writing anything. The read loop continues normally and
    /// eventually hits the idle timeout (which is fine — we just need to
    /// observe the ack).
    #[tokio::test]
    async fn native_steer_with_no_active_run_id_acks_expected_run_id_missing() {
        // Quiet process: never emits anything, so the read loop has only
        // the steer arm and the idle timeout to consider.
        let mut client = spawn_script("sleep 10").await;
        assert!(
            client.active_run_id().is_none(),
            "precondition: active_run_id starts as None"
        );

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        // Fire-and-forget: send a SteerRequest from a separate task so
        // the read loop picks it up via the select! arm.
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["test steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        // Drive the read loop with short idle timeout so the test
        // doesn't hang. The expected_id is intentionally never going to
        // be matched (the script writes nothing); the read loop will
        // exit via IdleTimeout shortly after the steer arm fires.
        let idle = std::time::Duration::from_millis(500);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let read_result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline)
            .await;
        send_task.await.expect("send_task should complete");

        // Read loop exit shape: IdleTimeout (no agent activity).
        assert!(
            matches!(read_result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout once steer was acked + script stayed silent, got {read_result:?}"
        );

        // Ack must be ExpectedRunIdMissing — the steer arm bailed out
        // without writing because active_run_id was None at write time.
        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::ExpectedRunIdMissing) => {}
            other => panic!("expected SteerAck::Err(ExpectedRunIdMissing), got {other:?}"),
        }
    }

    /// Steer with `active_run_id` set writes the JSON-RPC request and
    /// routes the matching response to the ack oneshot as `Success`.
    /// Verifies the wire shape (`sessionId` + `expectedRunId` + `prompt`)
    /// indirectly: the bash script emits a response keyed by the steer
    /// id (0), and `Success` only fires if the read loop matched that
    /// id to its `pending_steer` entry.
    #[tokio::test]
    async fn native_steer_with_active_run_id_routes_response_to_ack() {
        // Script: pause briefly so the test task can install the steer
        // and we can be sure the response doesn't race ahead of the
        // write — then emit the steer response (id=0 because next_id
        // starts at 0 and the steer is the first request the read loop
        // writes), then idle. This is a JSON-RPC success response with
        // a `stopReason` payload (matching the shape goose uses for
        // steer responses in fake_llm.rs).
        let script = "sleep 0.5; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"stopReason\":\"end_turn\"}}'; \
                      sleep 10";
        let mut client = spawn_script(script).await;

        // Set active_run_id via a synthesized session_info_update so the
        // steer arm has a non-None value to read at write time.
        let update = session_info_update_msg(Some(serde_json::json!("run-42")));
        let _ = client.handle_session_update(&update);
        assert_eq!(client.active_run_id(), Some("run-42"));

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["test steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        // Drive the read loop. Expected_id 999 will never be emitted by
        // the script so the read loop exits via idle timeout after the
        // steer response is routed to ack.
        let idle = std::time::Duration::from_secs(2);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let read_result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline)
            .await;
        send_task.await.expect("send_task should complete");

        // Read loop exit: IdleTimeout (no further activity after the
        // routed steer response). AgentExited would also be a valid
        // exit if the bash script terminated early; either is fine —
        // what matters is the ack.
        assert!(
            matches!(
                read_result,
                Err(AcpError::IdleTimeout(_)) | Err(AcpError::AgentExited)
            ),
            "expected IdleTimeout or AgentExited after steer ack, got {read_result:?}"
        );

        // Ack must be Success: the steer response (id=0) was routed to
        // pending_steer.ack_tx.
        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        match ack {
            crate::pool::SteerAck::Success => {}
            other => panic!("expected SteerAck::Success, got {other:?}"),
        }
    }

    // ── Goose usage notification integration ──────────────────────────────

    /// Build a `_goose/unstable/session/update` JSON-RPC notification.
    fn goose_usage_update_msg(
        session_id: &str,
        input: u64,
        output: u64,
        cost: Option<f64>,
    ) -> serde_json::Value {
        let mut update = serde_json::json!({
            "sessionUpdate": "usage_update",
            "used": input + output,
            "contextLimit": 200000u64,
            "accumulatedInputTokens": input,
            "accumulatedOutputTokens": output,
        });
        if let Some(c) = cost {
            update["accumulatedCost"] = serde_json::json!(c);
        }
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "_goose/unstable/session/update",
            "params": {
                "sessionId": session_id,
                "update": update
            }
        })
    }

    #[tokio::test]
    async fn goose_usage_notification_recorded_and_take_returns_usage() {
        let mut client = spawn_inert_client().await;
        assert!(client.take_turn_usage().is_none(), "starts empty");

        // begin_turn before sending the prompt — mirrors the real call flow.
        client.goose_usage.begin_turn("s1");
        let msg = goose_usage_update_msg("s1", 1000, 200, Some(0.01));
        client.handle_goose_usage_update(&msg);

        let usage = client
            .take_turn_usage()
            .expect("usage should be present after notification");
        assert_eq!(usage.session_id, "s1");
        assert_eq!(usage.turn_seq, 1);
        assert!(!usage.delta_reliable, "first turn must be unreliable");
        assert_eq!(usage.cumulative_input_tokens, 1000);
        assert_eq!(usage.cumulative_output_tokens, 200);
        assert_eq!(usage.cumulative_cost_usd, Some(0.01));

        // Second take must be None.
        assert!(
            client.take_turn_usage().is_none(),
            "take after drain is None"
        );
    }

    #[tokio::test]
    async fn goose_usage_second_turn_delta_reliable() {
        let mut client = spawn_inert_client().await;
        // Turn 1.
        client.goose_usage.begin_turn("s2");
        client.handle_goose_usage_update(&goose_usage_update_msg("s2", 1000, 200, None));
        let _ = client.take_turn_usage();
        // Turn 2.
        client.goose_usage.begin_turn("s2");
        client.handle_goose_usage_update(&goose_usage_update_msg("s2", 1800, 450, None));
        let usage = client.take_turn_usage().expect("turn 2 usage");
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(800));
        assert_eq!(usage.turn_output_tokens, Some(250));
    }

    #[tokio::test]
    async fn goose_usage_malformed_notification_does_not_panic() {
        let mut client = spawn_inert_client().await;
        // Missing params entirely.
        let bad = serde_json::json!({"jsonrpc":"2.0","method":"_goose/unstable/session/update"});
        client.handle_goose_usage_update(&bad);
        assert!(client.take_turn_usage().is_none());

        // params present but wrong shape.
        let bad2 = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "_goose/unstable/session/update",
            "params": { "oops": true }
        });
        client.handle_goose_usage_update(&bad2);
        assert!(client.take_turn_usage().is_none());
    }
}
