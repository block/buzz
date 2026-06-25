use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceError;
use rmcp::ServiceExt;
use serde_json::{Map, Value};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::sync::Mutex as AsyncMutex;

use crate::config::{Config, HookServers};
use crate::types::{clamp, AgentError, McpServerStdio, ToolDef, ToolResult, ToolResultContent};

const SEP: &str = "__";
const MAX_NAME_LEN: usize = 128;
const MAX_QNAME_LEN: usize = 64;
const MAX_TOOLS_PER_SESSION: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 1024;
const MAX_SCHEMA_BYTES: usize = 4096;
const MARKER_FIELD_MAX: usize = 256;
pub const MAX_MCP_SERVERS: usize = 16;
const MAX_HOOK_RESULT_BYTES: usize = 16 * 1024;

/// Byte budgets for a single tool result. `total` bounds everything the
/// result may occupy in history (text + images); `text` bounds the text
/// portion alone, since text is where runaway outputs (build logs, file
/// dumps) live while images are legitimately large and self-limiting.
#[derive(Clone, Copy)]
pub struct ResultBudget {
    pub total: usize,
    pub text: usize,
}

const PASSTHROUGH_ENV: &[&str] = &[
    // Core
    "PATH",
    "HOME",
    "TERM",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    // SSH — required for git clone/push over SSH (git@github.com:...)
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    // Git — operator-configured helpers and transport overrides
    "GIT_ASKPASS",
    "GIT_SSH_COMMAND",
    "GIT_CONFIG_GLOBAL",
    // Buzz identity — dev-mcp writes NOSTR_PRIVATE_KEY to a keyfile then
    // removes it from its own env (children never see it). BUZZ_PRIVATE_KEY
    // and BUZZ_RELAY_URL are kept for the buzz CLI.
    "NOSTR_PRIVATE_KEY",
    "BUZZ_PRIVATE_KEY",
    "BUZZ_RELAY_URL",
];

// Windows has no $TMPDIR/$HOME. TMP/TEMP/USERPROFILE are what
// std::env::temp_dir() consults — without them it falls back to C:\Windows,
// which child processes can't write to (PermissionDenied). USERPROFILE is the
// always-set floor. LOCALAPPDATA/APPDATA carry child-tool config (git, etc.).
#[cfg(windows)]
const PASSTHROUGH_ENV_WINDOWS: &[&str] = &["TMP", "TEMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA"];

type Client = RunningService<RoleClient, ()>;

#[derive(Clone)]
struct ServerSpec {
    name: String,
    command: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: String,
}

enum ClientState {
    Healthy {
        client: Arc<Client>,
        pgid: Option<u32>,
        tools: Arc<Vec<String>>,
    },
    Dead {
        attempts: u32,
        next_retry: Instant,
        reason: String,
        // Preserved from the last Healthy state so tools() filtering stays accurate while dead.
        tools: Arc<Vec<String>>,
    },
}

struct Server {
    name: String,
    spec: ServerSpec,
    client: ArcSwap<ClientState>,
    restart_lock: AsyncMutex<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        if let ClientState::Healthy { pgid: Some(p), .. } = &**self.client.load() {
            killpg(*p, &self.name, "drop");
        }
    }
}

enum RestartCheck {
    Healthy,
    Ready {
        attempt_n: u32,
        prev_tools: Arc<Vec<String>>,
    },
}

fn check_restart_state(server: &Server, max_attempts: u32) -> Result<RestartCheck, AgentError> {
    match &**server.client.load() {
        ClientState::Healthy { .. } => Ok(RestartCheck::Healthy),
        ClientState::Dead { attempts, .. } if *attempts >= max_attempts => {
            Err(AgentError::Mcp(format!(
                "The MCP server '{}' is unavailable (exhausted). Its tools have been removed for this session.",
                server.name
            )))
        }
        ClientState::Dead { next_retry, reason, .. } if Instant::now() < *next_retry => {
            Err(AgentError::Mcp(format!(
                "server '{}' is recovering (last error: {reason}). Try again later or use a different tool.",
                server.name
            )))
        }
        ClientState::Dead { attempts, tools, .. } => Ok(RestartCheck::Ready {
            attempt_n: attempts + 1,
            prev_tools: tools.clone(),
        }),
    }
}

struct Entry {
    server_idx: usize,
    bare: String,
}

pub struct McpRegistry {
    by_qname: HashMap<String, Entry>,
    defs: Vec<ToolDef>,
    servers: Vec<Arc<Server>>,
    max_attempts: u32,
    backoff_base: Duration,
    backoff_max: Duration,
    init_timeout: Duration,
    /// Consecutive hook timeout count per server. Kill on second consecutive.
    hook_timeouts: std::sync::Mutex<HashMap<String, u32>>,
}

impl McpRegistry {
    pub async fn spawn_all(
        cfg: &Config,
        servers: &[McpServerStdio],
        cwd: &str,
    ) -> Result<Self, AgentError> {
        if servers.len() > MAX_MCP_SERVERS {
            return Err(AgentError::Mcp(format!(
                "too many MCP servers: {} > {MAX_MCP_SERVERS}",
                servers.len()
            )));
        }
        let mut reg = Self {
            by_qname: HashMap::new(),
            defs: Vec::new(),
            servers: Vec::new(),

            max_attempts: cfg.mcp_max_restart_attempts.max(1),
            backoff_base: Duration::from_millis(cfg.mcp_restart_base_ms.max(1)),
            backoff_max: Duration::from_millis(cfg.mcp_restart_max_ms.max(1)),
            init_timeout: cfg.mcp_init_timeout,
            hook_timeouts: std::sync::Mutex::new(HashMap::new()),
        };

        let mut seen_names = HashSet::new();
        for s in servers {
            if !valid_name(&s.name) || s.name.contains("__") {
                return Err(AgentError::Mcp(format!("invalid server name: {}", s.name)));
            }
            if !seen_names.insert(s.name.clone()) {
                return Err(AgentError::Mcp(format!(
                    "duplicate server name: {}",
                    s.name
                )));
            }
            let spec = ServerSpec {
                name: s.name.clone(),
                command: s.command.clone(),
                args: s.args.clone(),
                env: s
                    .env
                    .iter()
                    .map(|e| (e.name.clone(), e.value.clone()))
                    .collect(),
                cwd: cwd.to_owned(),
            };
            let (client, pgid, tool_names, raw_tools) = spawn_one(&spec, reg.init_timeout).await?;
            let server_idx = reg.servers.len();
            let server = Arc::new(Server {
                name: spec.name.clone(),
                spec,
                client: ArcSwap::from_pointee(ClientState::Healthy {
                    client: Arc::new(client),
                    pgid,
                    tools: Arc::new(tool_names),
                }),
                restart_lock: AsyncMutex::new(()),
            });
            reg.servers.push(server);

            for t in raw_tools {
                if reg.defs.len() >= MAX_TOOLS_PER_SESSION {
                    return Err(AgentError::Mcp(format!(
                        "too many tools (>{MAX_TOOLS_PER_SESSION})"
                    )));
                }
                let bare = t.name.to_string();
                if !valid_name(&bare) || bare.contains("__") {
                    return Err(AgentError::Mcp(format!("invalid tool name: {bare}")));
                }
                let qname = format!("{}{SEP}{}", s.name, bare);
                if qname.len() > MAX_QNAME_LEN {
                    return Err(AgentError::Mcp(format!(
                        "qualified tool name too long: {} ({} > {MAX_QNAME_LEN})",
                        qname,
                        qname.len()
                    )));
                }
                if reg.by_qname.contains_key(&qname) {
                    return Err(AgentError::Mcp(format!("duplicate tool: {qname}")));
                }
                reg.defs.push(ToolDef {
                    name: qname.clone(),
                    description: clamp(
                        t.description.as_deref().unwrap_or("").to_owned(),
                        MAX_DESCRIPTION_BYTES,
                    ),
                    input_schema: cap_schema(&qname, Value::Object((*t.input_schema).clone())),
                });
                reg.by_qname.insert(qname, Entry { server_idx, bare });
            }
        }
        Ok(reg)
    }

    pub fn server_of(&self, qname: &str) -> Option<&str> {
        self.by_qname
            .get(qname)
            .map(|e| self.servers[e.server_idx].name.as_str())
    }

    pub fn has(&self, qname: &str) -> bool {
        self.by_qname.contains_key(qname)
    }

    /// True if `qname` resolves to a hidden hook tool (bare name starts
    /// with `_`). Used to reject hook calls coming from the LLM path —
    /// hooks are only callable via `call_hooks`.
    pub fn is_hook(&self, qname: &str) -> bool {
        self.by_qname
            .get(qname)
            .map(|e| e.bare.starts_with('_'))
            .unwrap_or(false)
    }

    pub fn tools(&self) -> Vec<ToolDef> {
        self.defs
            .iter()
            .filter(|d| {
                let entry = match self.by_qname.get(&d.name) {
                    Some(e) => e,
                    None => return false,
                };
                // Bare names starting with `_` are hooks — invisible to the LLM.
                if entry.bare.starts_with('_') {
                    return false;
                }
                let server = &self.servers[entry.server_idx];
                match &**server.client.load() {
                    ClientState::Healthy { tools, .. } => tools.iter().any(|t| t == &entry.bare),
                    ClientState::Dead {
                        attempts, tools, ..
                    } => *attempts < self.max_attempts && tools.iter().any(|t| t == &entry.bare),
                }
            })
            .cloned()
            .collect()
    }

    /// Call every tool whose bare name equals `hook_name` across all
    /// allowlisted servers in parallel, bounded by `timeout`. Returns
    /// `(server_name, text)` pairs in **config order** (deterministic),
    /// dropping empty/whitespace-only responses, errors and timeouts.
    /// Hooks are fail-open and must never block the agent.
    pub async fn call_hooks(
        self: &Arc<Self>,
        hook_name: &str,
        input: &Value,
        timeout: Duration,
        allowed: &HookServers,
    ) -> Vec<(String, String)> {
        if allowed.is_disabled() {
            return Vec::new();
        }
        // Walk servers in registration order so the result is deterministic
        // regardless of HashMap iteration order or task completion order.
        let mut targets: Vec<(usize, String, String)> = Vec::new();
        for (idx, server) in self.servers.iter().enumerate() {
            if !allowed.allows(&server.name) {
                continue;
            }
            let qname = format!("{}{SEP}{}", server.name, hook_name);
            if self.by_qname.contains_key(&qname) {
                targets.push((idx, server.name.clone(), qname));
            }
        }
        if targets.is_empty() {
            return Vec::new();
        }
        let mut set = tokio::task::JoinSet::new();
        for (idx, server_name, qname) in targets {
            let reg = Arc::clone(self);
            let args = input.clone();
            set.spawn(async move {
                // Hooks are intentionally non-cancellable: they are
                // already bounded by their own timeout and are fail-open.
                // Session cancel should not interrupt hook evaluation.
                let (_dummy_tx, mut dummy_cancel) = watch::channel(false);
                let res = tokio::time::timeout(
                    timeout,
                    reg.call(
                        &qname,
                        "hook",
                        &args,
                        ResultBudget {
                            total: MAX_HOOK_RESULT_BYTES,
                            text: MAX_HOOK_RESULT_BYTES,
                        },
                        &mut dummy_cancel,
                    ),
                )
                .await;
                drop(_dummy_tx);
                (idx, server_name, res)
            });
        }
        let mut indexed: Vec<(usize, String, String)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            // fail-open: drop join errors, timeouts, call errors,
            // empty/whitespace-only text. On timeout, also kill the server
            // process group so a wedged hook can't poison the next regular
            // tool call. The registry's lazy restart handles the rest.
            match joined {
                Ok((idx, server_name, Ok(Ok(r)))) => {
                    // Success — reset consecutive timeout counter.
                    if let Ok(mut counts) = self.hook_timeouts.lock() {
                        counts.remove(&server_name);
                    }
                    if !r.is_error && !r.text().trim().is_empty() {
                        indexed.push((idx, server_name, r.text()));
                    }
                }
                Ok((_idx, server_name, Err(_elapsed))) => {
                    // Kill only on second consecutive timeout.
                    let count = {
                        let mut counts =
                            self.hook_timeouts.lock().unwrap_or_else(|e| e.into_inner());
                        let c = counts.entry(server_name.clone()).or_insert(0);
                        *c += 1;
                        *c
                    };
                    if count >= 2 {
                        tracing::warn!(
                            "hook: killing server '{}' after {} consecutive timeouts",
                            server_name,
                            count
                        );
                        self.kill_server(&server_name, "hook timeout (consecutive)");
                        if let Ok(mut counts) = self.hook_timeouts.lock() {
                            counts.remove(&server_name);
                        }
                    } else {
                        tracing::warn!("hook: server '{}' timed out ({}/2)", server_name, count);
                    }
                }
                _ => {}
            }
        }
        indexed.sort_by_key(|(idx, _, _)| *idx);
        indexed
            .into_iter()
            .map(|(_, name, text)| (name, text))
            .collect()
    }

    /// Kill the server's process group and mark it dead. Idempotent:
    /// if the server is already Dead (or unknown), this is a no-op.
    /// Counts as one attempt toward the restart budget so that a
    /// pathological server (starts fine, deadlocks on every call)
    /// eventually exhausts.
    pub fn kill_server(&self, name: &str, reason: &str) {
        let server = match self.servers.iter().find(|s| s.name == name) {
            Some(s) => s,
            None => return,
        };
        let current = server.client.load_full();
        let (pgid, tools) = match &*current {
            ClientState::Dead { .. } => return,
            ClientState::Healthy { pgid, tools, .. } => (*pgid, tools.clone()),
        };
        let dead = Arc::new(ClientState::Dead {
            attempts: 1,
            next_retry: Instant::now() + backoff(1, self.backoff_base, self.backoff_max),
            reason: reason.to_owned(),
            tools,
        });
        // CAS so we don't clobber a concurrent restart that already
        // transitioned the state. If the swap fails, the kill below is
        // still safe — the pgid we read belonged to a process we observed
        // as Healthy, and killpg on an already-reaped pgid is a no-op.
        let prev = server.client.compare_and_swap(&current, dead);
        if Arc::ptr_eq(&prev, &current) {
            if let Some(p) = pgid {
                killpg(p, &server.name, "kill_server");
            }
            tracing::error!(
                "MCP server '{}' killed and marked dead (reason={reason})",
                server.name
            );
        }
    }

    fn kill_and_mark_dead_if_current(
        &self,
        server: &Server,
        failed_client: &Arc<Client>,
        reason: &str,
    ) {
        let current = server.client.load_full();
        match &*current {
            ClientState::Healthy {
                client,
                pgid,
                tools,
            } if Arc::ptr_eq(client, failed_client) => {
                if let Some(p) = *pgid {
                    killpg(p, &server.name, "call_failed");
                }
                let dead = Arc::new(ClientState::Dead {
                    attempts: 1,
                    next_retry: Instant::now() + backoff(1, self.backoff_base, self.backoff_max),
                    reason: reason.to_owned(),
                    tools: tools.clone(),
                });
                let _ = server.client.compare_and_swap(&current, dead);
                tracing::error!(
                    "MCP server '{}' killed and marked dead (reason={reason})",
                    server.name
                );
            }
            _ => {}
        }
    }

    pub async fn call(
        &self,
        qname: &str,
        provider_id: &str,
        arguments: &Value,
        budget: ResultBudget,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<ToolResult, AgentError> {
        let entry = self
            .by_qname
            .get(qname)
            .ok_or_else(|| AgentError::Mcp(format!("unknown tool {qname}")))?;
        let server = self.servers[entry.server_idx].clone();

        let state = server.client.load();
        if let ClientState::Healthy { client, tools, .. } = &**state {
            if !tools.iter().any(|t| t == &entry.bare) {
                return Err(AgentError::Mcp(format!(
                    "tool '{qname}': no longer available; the MCP server restarted with a different tool set."
                )));
            }
            let client = client.clone();
            drop(state);
            return self
                .do_call(
                    &server,
                    &client,
                    &entry.bare,
                    qname,
                    provider_id,
                    arguments,
                    budget,
                    cancel,
                )
                .await;
        }
        drop(state);

        self.maybe_restart(&server).await?;
        let state = server.client.load();
        let client = match &**state {
            ClientState::Healthy { client, tools, .. } => {
                if !tools.iter().any(|t| t == &entry.bare) {
                    return Err(AgentError::Mcp(format!(
                        "tool '{qname}': no longer available; the MCP server restarted with a different tool set."
                    )));
                }
                client.clone()
            }
            ClientState::Dead { reason, .. } => {
                return Err(AgentError::Mcp(format!(
                    "tool '{qname}': server '{}' restart failed: {reason}",
                    server.name
                )));
            }
        };
        drop(state);
        self.do_call(
            &server,
            &client,
            &entry.bare,
            qname,
            provider_id,
            arguments,
            budget,
            cancel,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn do_call(
        &self,
        server: &Server,
        client: &Arc<Client>,
        bare: &str,
        qname: &str,
        provider_id: &str,
        arguments: &Value,
        budget: ResultBudget,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<ToolResult, AgentError> {
        let arg_obj = match arguments {
            Value::Object(m) => Some(m.clone()),
            Value::Null => None,
            _ => {
                return Err(AgentError::Mcp(format!(
                    "tool {qname} arguments must be a JSON object"
                )))
            }
        };
        let mut params = CallToolRequestParams::default();
        params.name = bare.to_owned().into();
        params.arguments = arg_obj;

        use rmcp::model::{CallToolRequest, ClientRequest, ServerResult};
        use rmcp::service::PeerRequestOptions;

        let req = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let mut handle = client
            .peer()
            .send_cancellable_request(req, PeerRequestOptions::no_options())
            .await
            .map_err(|e| AgentError::Mcp(format!("call {qname}: {e}")))?;

        // Early cancel check — watch::changed() only fires on NEW writes.
        if *cancel.borrow() {
            fire_and_forget_cancel(handle, qname);
            return Err(AgentError::Cancelled);
        }

        // Poll the inner oneshot directly so we can still own `handle` in
        // the cancel branch (await_response would move it).
        let raw: Result<ServerResult, ServiceError> = tokio::select! {
            biased;
            _ = cancel.changed() => {
                fire_and_forget_cancel(handle, qname);
                return Err(AgentError::Cancelled);
            }
            r = &mut handle.rx => match r {
                Ok(inner) => inner,
                Err(_) => Err(ServiceError::TransportClosed),
            },
        };

        let res = match raw {
            Ok(ServerResult::CallToolResult(r)) => r,
            Ok(_) => {
                return Err(AgentError::Mcp(format!(
                    "call {qname}: unexpected response type"
                )))
            }
            Err(e) => {
                if is_transport_error(&e) {
                    self.kill_and_mark_dead_if_current(
                        server,
                        client,
                        &format!("call failed: {e}"),
                    );
                    return Err(AgentError::Mcp(format!("call {qname}: {e}")));
                }
                // Application-level JSON-RPC error (e.g. -32602 invalid params).
                // Server is healthy — it correctly rejected bad input. Return to LLM.
                return Ok(ToolResult {
                    provider_id: provider_id.to_owned(),
                    content: vec![ToolResultContent::Text(clamp(
                        format!("Tool call rejected: {e}"),
                        budget.text,
                    ))],
                    is_error: true,
                });
            }
        };
        let content = tool_result_content(&res.content, budget.total, budget.text);
        Ok(ToolResult {
            provider_id: provider_id.to_owned(),
            content,
            is_error: res.is_error.unwrap_or(false),
        })
    }

    async fn maybe_restart(&self, server: &Server) -> Result<(), AgentError> {
        match check_restart_state(server, self.max_attempts)? {
            RestartCheck::Healthy => return Ok(()),
            RestartCheck::Ready { .. } => {}
        }

        let _guard = server.restart_lock.lock().await;

        let (attempt_n, prev_tools) = match check_restart_state(server, self.max_attempts)? {
            RestartCheck::Healthy => return Ok(()),
            RestartCheck::Ready {
                attempt_n,
                prev_tools,
            } => (attempt_n, prev_tools),
        };

        let started = Instant::now();
        tracing::info!(
            "MCP server '{}' restarting (attempt {attempt_n}/{})",
            server.name,
            self.max_attempts
        );
        match spawn_one(&server.spec, self.init_timeout).await {
            Ok((client, pgid, tool_names, _raw_tools)) => {
                server.client.store(Arc::new(ClientState::Healthy {
                    client: Arc::new(client),
                    pgid,
                    tools: Arc::new(tool_names),
                }));

                tracing::info!(
                    "MCP server '{}' restarted in {}ms (attempt {attempt_n})",
                    server.name,
                    started.elapsed().as_millis()
                );
                Ok(())
            }
            Err(e) => {
                let reason = format!("restart failed: {e}");
                let permanent = attempt_n >= self.max_attempts;
                let next_retry = if permanent {
                    Instant::now() + Duration::from_secs(86_400)
                } else {
                    Instant::now() + backoff(attempt_n, self.backoff_base, self.backoff_max)
                };
                server.client.store(Arc::new(ClientState::Dead {
                    attempts: attempt_n,
                    next_retry,
                    reason: reason.clone(),
                    tools: prev_tools,
                }));

                tracing::error!(
                    "MCP server '{}' restart failed (attempt {attempt_n}/{}, permanent={permanent}): {reason}",
                    server.name, self.max_attempts
                );
                Err(AgentError::Mcp(reason))
            }
        }
    }
}

async fn spawn_one(
    spec: &ServerSpec,
    timeout: Duration,
) -> Result<(Client, Option<u32>, Vec<String>, Vec<rmcp::model::Tool>), AgentError> {
    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args);
    cmd.env_clear();
    for k in PASSTHROUGH_ENV {
        if let Ok(v) = std::env::var(k) {
            cmd.env(k, v);
        }
    }
    #[cfg(windows)]
    for k in PASSTHROUGH_ENV_WINDOWS {
        if let Ok(v) = std::env::var(k) {
            cmd.env(k, v);
        }
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.current_dir(&spec.cwd);
    cmd.stderr(std::process::Stdio::inherit());

    #[cfg(unix)]
    cmd.process_group(0);

    let transport = TokioChildProcess::new(cmd)
        .map_err(|e| AgentError::Mcp(format!("spawn {}: {e}", spec.name)))?;
    let pgid = transport.id();

    struct PgidGuard {
        pgid: Option<u32>,
        name: String,
    }
    impl Drop for PgidGuard {
        fn drop(&mut self) {
            if let Some(p) = self.pgid.take() {
                killpg(p, &self.name, "spawn_dropped");
            }
        }
    }
    let mut guard = PgidGuard {
        pgid,
        name: spec.name.clone(),
    };

    let client: Client = match tokio::time::timeout(timeout, ().serve(transport)).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            return Err(AgentError::Mcp(format!("init {}: {e}", spec.name)));
        }
        Err(_) => {
            return Err(AgentError::Mcp(timeout_msg("init", &spec.name, timeout)));
        }
    };

    let tools = match tokio::time::timeout(timeout, client.peer().list_all_tools()).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            return Err(AgentError::Mcp(format!("list_tools {}: {e}", spec.name)));
        }
        Err(_) => {
            return Err(AgentError::Mcp(timeout_msg(
                "list_tools",
                &spec.name,
                timeout,
            )));
        }
    };
    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    guard.pgid = None;
    Ok((client, pgid, names, tools))
}

/// Send `notifications/cancelled` to the MCP server, fire-and-forget.
/// Per MCP spec, cancellation notifications are best-effort; we never
/// block the agent on slow server stdio.
fn fire_and_forget_cancel(
    handle: rmcp::service::RequestHandle<rmcp::service::RoleClient>,
    qname: &str,
) {
    let qname_owned = qname.to_owned();
    tokio::spawn(async move {
        if let Err(e) = handle.cancel(Some("session cancelled".into())).await {
            tracing::debug!("cancel notification failed for {qname_owned}: {e}");
        }
    });
}

/// Returns `true` for errors indicating the MCP server process is dead or
/// unreachable. Returns `false` for application-level JSON-RPC errors where
/// the server is healthy but rejected the request (e.g. invalid params).
fn is_transport_error(e: &ServiceError) -> bool {
    matches!(
        e,
        ServiceError::TransportSend(_)
            | ServiceError::TransportClosed
            | ServiceError::Timeout { .. }
            | ServiceError::UnexpectedResponse
    )
}

fn backoff(attempt: u32, base: Duration, max: Duration) -> Duration {
    let shift = attempt.saturating_sub(1).min(20);
    let scaled = base.saturating_mul(1u32 << shift);
    let capped = scaled.min(max);
    let ms = capped.as_millis() as u64;
    let jitter_pct = jitter_percent();
    let jittered = (ms as i64) + ((ms as i64) * jitter_pct / 100);
    Duration::from_millis(jittered.max(0) as u64)
}

fn jitter_percent() -> i64 {
    let mut buf = [0u8; 1];
    let _ = getrandom::fill(&mut buf);
    ((buf[0] as i64) % 41) - 20
}

fn timeout_msg(stage: &str, name: &str, t: Duration) -> String {
    format!("{stage} {name}: timeout after {}s", t.as_secs())
}

fn cap_schema(qname: &str, schema: Value) -> Value {
    let size = serde_json::to_vec(&schema).map(|b| b.len()).unwrap_or(0);
    if size <= MAX_SCHEMA_BYTES {
        return schema;
    }
    tracing::warn!(
        "tool {qname} schema is {size} bytes (>{MAX_SCHEMA_BYTES}); replacing with empty object"
    );
    Value::Object(Map::new())
}

#[cfg(unix)]
fn killpg(pgid: u32, name: &str, stage: &str) {
    use nix::sys::signal::{killpg as nix_killpg, Signal};
    use nix::unistd::Pid;
    let result = nix_killpg(Pid::from_raw(pgid as i32), Signal::SIGKILL);
    tracing::info!(
        "killpg MCP {name} ({stage}) pgid={pgid} ok={}",
        result.is_ok()
    );
}
#[cfg(not(unix))]
fn killpg(_pgid: u32, name: &str, stage: &str) {
    tracing::info!("relying on Drop to kill MCP {name} ({stage})");
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_NAME_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub(crate) fn truncate_at_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

/// Byte allowance reserved for the elision marker inside [`truncate_middle`].
/// The marker is ~80 bytes; the slack keeps the arithmetic safely one-sided.
const ELISION_MARKER_ALLOWANCE: usize = 128;

/// Truncate `s` to at most `max` bytes by eliding the *middle*, keeping the
/// head and tail. Tool output puts its conclusion at the end (test summaries,
/// error trailers) and its identity at the start; head-only truncation loses
/// the part the model needs most. The marker reports how much was elided so
/// the model knows to re-run a narrower command rather than trust the gap.
pub(crate) fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let keep = max.saturating_sub(ELISION_MARKER_ALLOWANCE);
    if keep == 0 {
        // Budget too small for head + marker + tail; degrade to a head cut.
        return truncate_at_boundary(s, max).to_owned();
    }
    let head = truncate_at_boundary(s, keep.div_ceil(2));
    let mut tail_start = s.len() - keep / 2;
    while tail_start < s.len() && !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let tail = &s[tail_start..];
    let elided = s.len() - head.len() - tail.len();
    format!(
        "{head}\n[... {elided} of {} bytes elided from tool result ...]\n{tail}",
        s.len()
    )
}

/// Assemble tool-result content under two budgets: `max_bytes` bounds the
/// whole result (text + images), `max_text_bytes` bounds the text portion
/// alone. Images are large by nature and pass through whole or get elided
/// with a marker; text is middle-elided so the head (what ran) and tail
/// (how it ended) both survive. Every elision leaves an inline marker.
fn tool_result_content(
    blocks: &[rmcp::model::Content],
    max_bytes: usize,
    max_text_bytes: usize,
) -> Vec<ToolResultContent> {
    use rmcp::model::RawContent;
    let mut out = Vec::new();
    let mut text = String::new();
    let mut used = 0usize; // total bytes emitted (text + images)
    let mut text_used = 0usize; // text bytes emitted
    let short = |s: &str| truncate_at_boundary(s, MARKER_FIELD_MAX).to_owned();

    // Flush accumulated text, middle-eliding to whatever budget remains.
    let flush_text = |out: &mut Vec<ToolResultContent>,
                      text: &mut String,
                      used: &mut usize,
                      text_used: &mut usize| {
        if text.is_empty() {
            return;
        }
        let budget = max_text_bytes
            .saturating_sub(*text_used)
            .min(max_bytes.saturating_sub(*used));
        let kept = truncate_middle(&std::mem::take(text), budget);
        *used = used.saturating_add(kept.len());
        *text_used = text_used.saturating_add(kept.len());
        if !kept.is_empty() {
            out.push(ToolResultContent::Text(kept));
        }
    };

    let append = |text: &mut String, s: &str| {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(s);
    };

    for c in blocks {
        match &c.raw {
            RawContent::Text(t) => append(&mut text, &t.text),
            RawContent::Image(i) => {
                flush_text(&mut out, &mut text, &mut used, &mut text_used);
                let image_bytes = i.data.len().saturating_add(i.mime_type.len());
                if used.saturating_add(image_bytes) <= max_bytes {
                    used = used.saturating_add(image_bytes);
                    out.push(ToolResultContent::Image {
                        data: i.data.clone(),
                        mime_type: i.mime_type.clone(),
                    });
                } else {
                    append(
                        &mut text,
                        &format!(
                            "[image elided: {}, {} base64 bytes exceeds remaining tool-result budget]",
                            short(&i.mime_type),
                            i.data.len()
                        ),
                    );
                }
            }
            RawContent::Audio(a) => append(
                &mut text,
                &format!(
                    "[audio elided: {}, {} bytes]",
                    short(&a.mime_type),
                    a.data.len()
                ),
            ),
            RawContent::ResourceLink(r) => {
                append(&mut text, &format!("[resource: {}]", short(&r.uri)));
            }
            RawContent::Resource(_) => append(&mut text, "[resource elided]"),
        }
    }
    flush_text(&mut out, &mut text, &mut used, &mut text_used);
    out
}

#[cfg(test)]
mod content_tests {
    use super::*;
    use rmcp::model::Content;

    #[cfg(windows)]
    #[test]
    fn windows_passthrough_includes_temp_dir_vars() {
        // std::env::temp_dir() consults these in order; without them it falls
        // back to C:\Windows, which children can't write to.
        for var in ["TMP", "TEMP", "USERPROFILE"] {
            assert!(
                PASSTHROUGH_ENV_WINDOWS.contains(&var),
                "{var} must pass through or temp_dir() falls back to C:\\Windows"
            );
        }
    }

    #[test]
    fn tool_result_content_preserves_images() {
        let blocks = vec![
            Content::text("header"),
            Content::image("aW1n", "image/png"),
            Content::text("tail"),
        ];
        let out = tool_result_content(&blocks, 1024, 1024);
        assert_eq!(out.len(), 3);
        assert!(matches!(&out[0], ToolResultContent::Text(t) if t == "header"));
        assert!(matches!(
            &out[1],
            ToolResultContent::Image { data, mime_type }
                if data == "aW1n" && mime_type == "image/png"
        ));
        assert!(matches!(&out[2], ToolResultContent::Text(t) if t == "tail"));
    }

    #[test]
    fn tool_result_content_elides_images_over_budget() {
        let blocks = vec![Content::image("a".repeat(300), "image/png")];
        let out = tool_result_content(&blocks, 256, 256);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], ToolResultContent::Text(t) if t.contains("image elided")));
    }

    #[test]
    fn oversized_text_is_middle_elided() {
        let mut body = String::new();
        for i in 0..5000 {
            body.push_str(&format!("line {i}\n"));
        }
        let blocks = vec![Content::text(body.clone())];
        let out = tool_result_content(&blocks, 1024 * 1024, 4096);
        assert_eq!(out.len(), 1);
        let ToolResultContent::Text(t) = &out[0] else {
            panic!("expected text");
        };
        assert!(t.len() <= 4096, "text exceeds budget: {}", t.len());
        assert!(t.starts_with("line 0\n"), "head lost");
        assert!(t.ends_with("line 4999\n"), "tail lost");
        assert!(
            t.contains("bytes elided from tool result"),
            "missing elision marker"
        );
    }

    #[test]
    fn text_within_budget_is_untouched() {
        let blocks = vec![Content::text("short output")];
        let out = tool_result_content(&blocks, 1024 * 1024, 4096);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], ToolResultContent::Text(t) if t == "short output"));
    }

    #[test]
    fn image_passes_whole_even_when_text_budget_is_small() {
        let big_text = "x".repeat(10_000);
        let img = "a".repeat(100_000);
        let blocks = vec![
            Content::text(big_text),
            Content::image(img.clone(), "image/png"),
        ];
        let out = tool_result_content(&blocks, 8 * 1024 * 1024, 4096);
        assert_eq!(out.len(), 2);
        assert!(matches!(&out[0], ToolResultContent::Text(t) if t.len() <= 4096));
        assert!(matches!(
            &out[1],
            ToolResultContent::Image { data, .. } if data == &img
        ));
    }

    #[test]
    fn truncate_middle_respects_max_and_boundaries() {
        let s = "é".repeat(60_000); // 2-byte chars stress boundary handling
        for max in [200usize, 1024, 50 * 1024] {
            let out = super::truncate_middle(&s, max);
            assert!(out.len() <= max, "max={max} got {}", out.len());
            assert!(std::str::from_utf8(out.as_bytes()).is_ok());
        }
        assert_eq!(super::truncate_middle("ok", 1024), "ok");
    }
}
