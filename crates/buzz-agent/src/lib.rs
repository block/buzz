#![forbid(unsafe_code)]
mod agent;
pub mod auth;
mod builtin;
pub mod catalog;
pub mod config;
mod handoff;
mod hints;
mod llm;
mod mcp;
pub mod types;
mod wire;

pub use catalog::{discover_databricks_models, ModelEntry, DATABRICKS_V2_KNOWN_MODELS};
pub use config::Provider;
pub use types::AgentError;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::BufReader;
use tokio::sync::{mpsc, watch, Mutex};

use crate::agent::RunCtx;
use crate::config::{Config, MAX_SYSTEM_PROMPT_BYTES, PROTOCOL_VERSION};
use crate::hints::SkillEntry;
use crate::llm::Llm;
use crate::mcp::McpRegistry;
use crate::types::{ContentBlock, HistoryItem};
use crate::wire::{
    classify, Inbound, InitializeParams, SessionCancelParams, SessionNewParams,
    SessionPromptParams, SessionSteerParams, WireMsg, WireSender, INVALID_PARAMS, METHOD_NOT_FOUND,
    PARSE_ERROR,
};

struct App {
    cfg: Config,
    llm: Arc<Llm>,
    sessions: Mutex<HashMap<String, Session>>,
}

struct Session {
    id: String,
    mcp: Arc<McpRegistry>,
    /// Skills discovered at session creation; used by the built-in `load_skill` tool.
    skills: Vec<SkillEntry>,
    history: Vec<HistoryItem>,
    cancel_tx: watch::Sender<bool>,
    busy: bool,
    /// Run id of the in-flight prompt, set when a prompt starts and cleared
    /// when it ends. `None` means no active run — a steer request targeting
    /// this session is rejected. Steer-capable clients learn this value from
    /// the `params.update._meta.goose.activeRunId` field on `session/update`.
    active_run_id: Option<String>,
    /// Sender for mid-turn steer messages. Created fresh per prompt (like
    /// `cancel_tx`); the running prompt loop holds the matching receiver and
    /// drains queued steers at round boundaries. `None` when no prompt is in
    /// flight.
    steer_tx: Option<mpsc::UnboundedSender<Vec<ContentBlock>>>,
    original_task: Option<String>,
    handoff_count: usize,
    stop_rejections: u32,
    /// Cache-summed input tokens the provider reported for this session's most
    /// recent request, or `None` before the first response (or after a handoff
    /// resets the context). Drives the token-based handoff gate; see
    /// [`RunCtx::should_handoff`].
    last_request_input_tokens: Option<u64>,
    /// History byte size when `last_request_input_tokens` was measured, paired
    /// with it so the gate can account for history appended since.
    last_request_history_bytes: Option<usize>,
    effective_system_prompt: Arc<str>,
}

fn die(msg: String) -> ! {
    tracing::error!("{msg}");
    std::process::exit(2);
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if matches!(args.get(1).map(String::as_str), Some("auth")) {
        return tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(auth_subcommand(&args[2..]));
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main());
    Ok(())
}

/// `buzz-agent auth <provider>` — run the interactive auth flow for a
/// provider and persist the result, then exit. Today this supports Databricks
/// OAuth 2.0 PKCE. Reads `DATABRICKS_HOST` from env; needs a browser on the
/// machine.
async fn auth_subcommand(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let provider = args.first().map(String::as_str);
    match provider {
        Some("databricks" | "databricks_v2" | "databricks-v2") => {
            let host = std::env::var("DATABRICKS_HOST")
                .map_err(|_| "auth databricks: DATABRICKS_HOST required")?;
            let pkce = auth::PkceOAuthConfig {
                discovery_url: format!(
                    "{}/oidc/.well-known/oauth-authorization-server",
                    host.trim_end_matches('/')
                ),
                client_id: "databricks-cli".into(),
                scopes: vec!["all-apis".into(), "offline_access".into()],
                cache_namespace: "databricks".into(),
                cache_dir_override: None,
            };
            let src = auth::PkceOAuthTokenSource::new(pkce)?;
            src.interactive_login().await?;
            eprintln!("Authenticated. Token cached under ~/.config/buzz-agent/oauth/databricks/.");
            Ok(())
        }
        Some(other) => Err(format!("auth: unknown provider {other:?}").into()),
        None => Err("auth: provider required (try: buzz-agent auth databricks)".into()),
    }
}

async fn async_main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let cfg = Config::from_env().unwrap_or_else(|e| die(e));
    let llm = Arc::new(Llm::new(&cfg).unwrap_or_else(|e| die(e.to_string())));
    let max_line = cfg.max_line_bytes;
    let app = Arc::new(App {
        cfg,
        llm,
        sessions: Mutex::new(HashMap::new()),
    });
    let (wire_tx, wire_rx) = mpsc::channel::<WireMsg>(64);
    let writer = tokio::spawn(wire::writer_task(wire_rx));
    if let Err(e) = read_loop(
        BufReader::new(tokio::io::stdin()),
        app.clone(),
        wire_tx,
        max_line,
    )
    .await
    {
        tracing::error!("io: reader: {e}");
    }
    for session in app.sessions.lock().await.values() {
        let _ = session.cancel_tx.send(true);
    }
    let _ = writer.await;
}

async fn read_loop<R: tokio::io::AsyncBufRead + Unpin>(
    mut stdin: R,
    app: Arc<App>,
    wire_tx: WireSender,
    max_line: usize,
) -> std::io::Result<()> {
    while let Some(line) = wire::read_bounded_line(&mut stdin, max_line).await? {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(msg) => dispatch(&app, msg, &wire_tx).await,
            Err(e) => {
                wire::send(
                    &wire_tx,
                    wire::err(Value::Null, PARSE_ERROR, &format!("jsonrpc: parse: {e}")),
                )
                .await;
            }
        }
    }
    Ok(())
}

async fn dispatch(app: &Arc<App>, msg: Value, wire_tx: &WireSender) {
    match classify(&msg) {
        Inbound::Request { id, method, params } => {
            handle_request(app, id, method, params, wire_tx).await
        }
        Inbound::Notification { method, params } => handle_notification(app, &method, params).await,
        Inbound::Ignored => {}
        Inbound::Invalid { id, code, message } => {
            wire::send(wire_tx, wire::err(id, code, &message)).await
        }
    }
}

async fn handle_request(
    app: &Arc<App>,
    id: Value,
    method: String,
    params: Value,
    wire_tx: &WireSender,
) {
    match method.as_str() {
        "initialize" => initialize(id, params, wire_tx).await,
        "session/new" => {
            let app = app.clone();
            let wire_tx = wire_tx.clone();
            tokio::spawn(async move { session_new(&app, id, params, &wire_tx).await });
        }
        "session/prompt" => spawn_prompt(app.clone(), id, params, wire_tx.clone()),
        "session/cancel" => {
            cancel_session(app, params).await;
            wire::send(wire_tx, wire::ok(id, Value::Null)).await;
        }
        // goose-compatible non-standard extension: inject user input into the
        // currently active prompt without starting a new one. Mirrors goose's
        // `_goose/unstable/session/steer` wire contract so a single client-side
        // delivery path serves both agents.
        "_goose/unstable/session/steer" => {
            steer_session(app, id, params, wire_tx).await;
        }
        _ => {
            wire::send(
                wire_tx,
                wire::err(
                    id,
                    METHOD_NOT_FOUND,
                    &format!("jsonrpc: method not found: {method}"),
                ),
            )
            .await
        }
    }
}

async fn handle_notification(app: &Arc<App>, method: &str, params: Value) {
    if method == "session/cancel" {
        cancel_session(app, params).await;
    }
}

async fn initialize(id: Value, params: Value, wire_tx: &WireSender) {
    let p: InitializeParams = match decode(params, "initialize") {
        Ok(p) => p,
        Err(m) => return reject(wire_tx, id, INVALID_PARAMS, &m).await,
    };
    // Honest negotiation: respond with the minimum of what the client
    // requested and what we support.
    // NOTE: gating `[Base]` injection on `protocol_version < 2` is a deliberate
    // temporary measure — we are squatting on ACP v2 ahead of the upstream ACP
    // RFD. Revisit when that RFD merges; otherwise a genuine upstream-v2 agent
    // would silently lose `[Base]`.
    let negotiated_version = p.protocol_version.min(PROTOCOL_VERSION);
    wire::send(
        wire_tx,
        wire::ok(
            id,
            json!({
                "protocolVersion": negotiated_version,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": false },
                    "mcpCapabilities": { "http": false, "sse": false },
                },
                "agentInfo": { "name": "buzz-agent", "version": env!("CARGO_PKG_VERSION") },
            }),
        ),
    )
    .await;
}

async fn session_new(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: SessionNewParams = match decode(params, "session/new") {
        Ok(p) => p,
        Err(m) => return reject(wire_tx, id, INVALID_PARAMS, &m).await,
    };
    if p.cwd.is_empty() || !Path::new(&p.cwd).is_absolute() {
        return reject(
            wire_tx,
            id,
            INVALID_PARAMS,
            "session/new: cwd must be an absolute path",
        )
        .await;
    }
    // Check cap without holding lock across MCP spawn (which may be slow).
    {
        let sessions = app.sessions.lock().await;
        if sessions.len() >= app.cfg.max_sessions {
            return reject(
                wire_tx,
                id,
                INVALID_PARAMS,
                "session/new: max sessions reached",
            )
            .await;
        }
    }
    let (hints_text, skills) = if app.cfg.hints_enabled {
        hints::build_hints_section(std::path::Path::new(&p.cwd))
    } else {
        (String::new(), Vec::new())
    };
    let effective_system_prompt: Arc<str> = {
        // When the harness provides a systemPrompt (base_prompt + persona), use
        // it as the primary content and suppress the default. The default is only
        // a fallback for legacy harnesses that don't send systemPrompt.
        let base = match p.system_prompt.as_deref() {
            Some(client_prompt) if !client_prompt.trim().is_empty() => client_prompt.to_owned(),
            _ => app.cfg.system_prompt.clone(),
        };
        let prompt = if hints_text.is_empty() {
            base
        } else {
            format!("{base}\n\n{hints_text}")
        };
        // Reject combined prompts exceeding 512KB.
        if prompt.len() > MAX_SYSTEM_PROMPT_BYTES {
            return reject(
                wire_tx,
                id,
                INVALID_PARAMS,
                &format!(
                    "session/new: combined system prompt exceeds {}KB limit ({} bytes)",
                    MAX_SYSTEM_PROMPT_BYTES / 1024,
                    prompt.len()
                ),
            )
            .await;
        }
        Arc::from(prompt)
    };
    let mcp = match McpRegistry::spawn_all(&app.cfg, &p.mcp_servers, &p.cwd).await {
        Ok(m) => Arc::new(m),
        Err(e) => return reject(wire_tx, id, e.json_rpc_code(), &e.to_string()).await,
    };
    let session_id = match session_token() {
        Ok(t) => format!("ses_{t}"),
        Err(e) => return reject(wire_tx, id, -32000, &e).await,
    };
    let (cancel_tx, _) = watch::channel(false);
    let mut sessions = app.sessions.lock().await;
    // Re-check cap (another session may have been created while we spawned MCP).
    if sessions.len() >= app.cfg.max_sessions {
        return reject(
            wire_tx,
            id,
            INVALID_PARAMS,
            "session/new: max sessions reached",
        )
        .await;
    }
    sessions.insert(
        session_id.clone(),
        Session {
            id: session_id.clone(),
            mcp,
            skills,
            history: Vec::new(),
            cancel_tx,
            busy: false,
            active_run_id: None,
            steer_tx: None,
            original_task: None,
            handoff_count: 0,
            stop_rejections: 0,
            last_request_input_tokens: None,
            last_request_history_bytes: None,
            effective_system_prompt,
        },
    );
    drop(sessions);
    wire::send(wire_tx, wire::ok(id, json!({ "sessionId": session_id }))).await;
}

fn decode<T: serde::de::DeserializeOwned>(params: Value, stage: &str) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("{stage}: {e}"))
}

async fn reject(wire_tx: &WireSender, id: Value, code: i32, message: &str) {
    wire::send(wire_tx, wire::err(id, code, message)).await;
}

async fn cancel_session(app: &Arc<App>, params: Value) {
    if let Ok(p) = serde_json::from_value::<SessionCancelParams>(params) {
        if let Some(s) = app.sessions.lock().await.get(&p.session_id) {
            let _ = s.cancel_tx.send(true);
        }
    }
}

/// Handle `_goose/unstable/session/steer`: queue user input into the in-flight
/// prompt. Validation mirrors goose's `on_steer_session`:
///   - empty prompt → `invalid_params`
///   - no active run (no prompt in flight) → `invalid_params`
///   - `expectedRunId` mismatch → `invalid_params` (caller is steering a turn
///     that already ended or rotated; it must fall back to cancel+merge)
///
/// On success the message is queued for pickup at the next round boundary and
/// we reply `{ runId, messageId }`, then emit a `queuedSteer` session/update so
/// the client can correlate the accepted steer with its eventual pickup.
async fn steer_session(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: SessionSteerParams = match decode(params, "_goose/unstable/session/steer") {
        Ok(p) => p,
        Err(m) => return reject(wire_tx, id, INVALID_PARAMS, &m).await,
    };
    if p.prompt.is_empty() {
        return reject(
            wire_tx,
            id,
            INVALID_PARAMS,
            "steer: prompt must not be empty",
        )
        .await;
    }
    if p.expected_run_id.is_empty() {
        return reject(
            wire_tx,
            id,
            INVALID_PARAMS,
            "steer: expectedRunId must not be empty",
        )
        .await;
    }
    let message_id = format!("steer_{}", session_token().unwrap_or_else(|_| "x".into()));
    let run_id = {
        let sessions = app.sessions.lock().await;
        let Some(s) = sessions.get(&p.session_id) else {
            return reject(wire_tx, id, INVALID_PARAMS, "steer: unknown session").await;
        };
        let Some(active) = s.active_run_id.as_deref() else {
            return reject(wire_tx, id, INVALID_PARAMS, "steer: no active run to steer").await;
        };
        if active != p.expected_run_id {
            return reject(
                wire_tx,
                id,
                INVALID_PARAMS,
                &format!(
                    "steer: expected active run id `{}` but found `{active}`",
                    p.expected_run_id
                ),
            )
            .await;
        }
        // A live run always has a steer_tx; if the channel is gone the run is
        // tearing down — treat as no active run rather than queue into the void.
        match &s.steer_tx {
            Some(tx) if tx.send(p.prompt).is_ok() => active.to_owned(),
            _ => return reject(wire_tx, id, INVALID_PARAMS, "steer: no active run to steer").await,
        }
    };
    wire::send(
        wire_tx,
        wire::ok(id, json!({ "runId": run_id, "messageId": message_id })),
    )
    .await;
    // Best-effort correlation hint for the client; mirrors goose's
    // `send_queued_steer_update`. Not load-bearing for delivery.
    wire::send(
        wire_tx,
        wire::session_update_with_goose_meta(
            &p.session_id,
            json!({ "sessionUpdate": "session_info_update" }),
            json!({ "queuedSteer": { "messageId": message_id, "runId": run_id } }),
        ),
    )
    .await;
}

fn spawn_prompt(app: Arc<App>, id: Value, params: Value, wire_tx: WireSender) {
    tokio::spawn(async move { run_prompt(app, id, params, wire_tx).await });
}

async fn run_prompt(app: Arc<App>, id: Value, params: Value, wire_tx: WireSender) {
    let p: SessionPromptParams = match decode(params, "session/prompt") {
        Ok(p) => p,
        Err(m) => return reject(&wire_tx, id, INVALID_PARAMS, &m).await,
    };
    let (
        sid,
        mcp,
        skills,
        mut history,
        mut original_task,
        mut handoff_count,
        mut stop_rejections,
        mut last_request_input_tokens,
        mut last_request_history_bytes,
        mut cancel_rx,
        effective_system_prompt,
        run_id,
        mut steer_rx,
    ) = match acquire_session(&app, &p.session_id).await {
        Ok(v) => v,
        Err(reason) => {
            return reject(
                &wire_tx,
                id,
                INVALID_PARAMS,
                &format!("session/prompt: {reason}"),
            )
            .await
        }
    };
    // Advertise the active run id so steer-capable clients can target this turn
    // via `expectedRunId`. Mirrors goose's `send_active_run_update`.
    wire::send(
        &wire_tx,
        wire::session_update_with_goose_meta(
            &sid,
            json!({ "sessionUpdate": "session_info_update" }),
            json!({ "activeRunId": run_id }),
        ),
    )
    .await;
    let mut ctx = RunCtx {
        cfg: &app.cfg,
        session_id: &sid,
        system_prompt: &effective_system_prompt,
        llm: &app.llm,
        mcp: &mcp,
        skills: &skills,
        wire: &wire_tx,
        cancel: &mut cancel_rx,
        steer: &mut steer_rx,
        history: &mut history,
        original_task: &mut original_task,
        handoff_count: &mut handoff_count,
        stop_rejections: &mut stop_rejections,
        last_request_input_tokens: &mut last_request_input_tokens,
        last_request_history_bytes: &mut last_request_history_bytes,
    };
    let result = ctx.run(p.prompt).await;
    if let Some(s) = app.sessions.lock().await.get_mut(&sid) {
        s.busy = false;
        // Clear run state so a late steer can't queue into a finished turn.
        s.active_run_id = None;
        s.steer_tx = None;
        s.history = history;
        s.original_task = original_task;
        s.handoff_count = handoff_count;
        s.stop_rejections = stop_rejections;
        s.last_request_input_tokens = last_request_input_tokens;
        s.last_request_history_bytes = last_request_history_bytes;
    }
    match result {
        Ok(stop) => {
            wire::send(
                &wire_tx,
                wire::ok(id, json!({ "stopReason": stop.as_wire() })),
            )
            .await
        }
        Err(e) => wire::send(&wire_tx, wire::err(id, e.json_rpc_code(), &e.to_string())).await,
    }
}

async fn acquire_session(
    app: &Arc<App>,
    session_id: &str,
) -> Result<
    (
        String,
        Arc<McpRegistry>,
        Vec<SkillEntry>,
        Vec<HistoryItem>,
        Option<String>,
        usize,
        u32,
        Option<u64>,
        Option<usize>,
        watch::Receiver<bool>,
        Arc<str>,
        String,
        mpsc::UnboundedReceiver<Vec<ContentBlock>>,
    ),
    &'static str,
> {
    let mut sessions = app.sessions.lock().await;
    let s = sessions.get_mut(session_id).ok_or("unknown session")?;
    if s.busy {
        return Err("prompt already in flight");
    }
    s.busy = true;
    let (tx, rx) = watch::channel(false);
    s.cancel_tx = tx;
    // Skills are read-only after session creation; clone the Vec so RunCtx
    // can hold a reference without holding the sessions lock.
    let skills = s.skills.clone();
    // Fresh run id + steer channel for this turn. The run id lets steer-capable
    // clients target *this* turn (rejecting steers aimed at a turn that already
    // ended); the channel carries mid-turn injections to the run loop.
    let run_id = format!("run_{}", session_token().unwrap_or_else(|_| "x".into()));
    s.active_run_id = Some(run_id.clone());
    let (steer_tx, steer_rx) = mpsc::unbounded_channel();
    s.steer_tx = Some(steer_tx);
    Ok((
        s.id.clone(),
        s.mcp.clone(),
        skills,
        std::mem::take(&mut s.history),
        s.original_task.take(),
        s.handoff_count,
        s.stop_rejections,
        s.last_request_input_tokens,
        s.last_request_history_bytes,
        rx,
        Arc::clone(&s.effective_system_prompt),
        run_id,
        steer_rx,
    ))
}

fn session_token() -> Result<String, String> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| format!("rng: getrandom failed: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}
