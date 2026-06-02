#![forbid(unsafe_code)]
mod agent;
pub mod auth;
mod config;
mod handoff;
mod hints;
mod llm;
mod mcp;
mod types;
mod wire;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::BufReader;
use tokio::sync::{mpsc, watch, Mutex};

use crate::agent::RunCtx;
use crate::config::{Config, PROTOCOL_VERSION};
use crate::llm::Llm;
use crate::mcp::McpRegistry;
use crate::types::HistoryItem;
use crate::wire::{
    classify, Inbound, InitializeParams, SessionCancelParams, SessionNewParams,
    SessionPromptParams, WireMsg, WireSender, INVALID_PARAMS, METHOD_NOT_FOUND, PARSE_ERROR,
};

struct App {
    cfg: Config,
    llm: Arc<Llm>,
    sessions: Mutex<HashMap<String, Session>>,
}

struct Session {
    id: String,
    mcp: Arc<McpRegistry>,
    history: Vec<HistoryItem>,
    cancel_tx: watch::Sender<bool>,
    busy: bool,
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

/// `sprout-agent auth <provider>` — run the interactive auth flow for a
/// provider and persist the result, then exit. Today the only provider is
/// `databricks` (OAuth 2.0 PKCE). Reads `DATABRICKS_HOST` from env; needs
/// a browser on the machine.
async fn auth_subcommand(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let provider = args.first().map(String::as_str);
    match provider {
        Some("databricks") => {
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
            eprintln!(
                "Authenticated. Token cached under ~/.config/sprout-agent/oauth/databricks/."
            );
            Ok(())
        }
        Some(other) => Err(format!("auth: unknown provider {other:?}").into()),
        None => Err("auth: provider required (try: sprout-agent auth databricks)".into()),
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
    let _ = p.protocol_version;
    wire::send(
        wire_tx,
        wire::ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": false },
                    "mcpCapabilities": { "http": false, "sse": false },
                },
                "agentInfo": { "name": "sprout-agent", "version": env!("CARGO_PKG_VERSION") },
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
    let effective_system_prompt: Arc<str> = if app.cfg.hints_enabled {
        let hints = hints::build_hints_section(std::path::Path::new(&p.cwd));
        if hints.is_empty() {
            Arc::from(app.cfg.system_prompt.as_str())
        } else {
            Arc::from(format!("{}\n\n{}", app.cfg.system_prompt, hints))
        }
    } else {
        Arc::from(app.cfg.system_prompt.as_str())
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
            history: Vec::new(),
            cancel_tx,
            busy: false,
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
        mut history,
        mut original_task,
        mut handoff_count,
        mut stop_rejections,
        mut last_request_input_tokens,
        mut last_request_history_bytes,
        mut cancel_rx,
        effective_system_prompt,
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
    let mut ctx = RunCtx {
        cfg: &app.cfg,
        session_id: &sid,
        system_prompt: &effective_system_prompt,
        llm: &app.llm,
        mcp: &mcp,
        wire: &wire_tx,
        cancel: &mut cancel_rx,
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
        Vec<HistoryItem>,
        Option<String>,
        usize,
        u32,
        Option<u64>,
        Option<usize>,
        watch::Receiver<bool>,
        Arc<str>,
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
    Ok((
        s.id.clone(),
        s.mcp.clone(),
        std::mem::take(&mut s.history),
        s.original_task.take(),
        s.handoff_count,
        s.stop_rejections,
        s.last_request_input_tokens,
        s.last_request_history_bytes,
        rx,
        Arc::clone(&s.effective_system_prompt),
    ))
}

fn session_token() -> Result<String, String> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| format!("rng: getrandom failed: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}
