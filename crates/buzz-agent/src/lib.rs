#![forbid(unsafe_code)]
//! buzz-agent's ACP server, with Goose as the agent loop.
//!
//! This is the layer `buzz-acp` actually talks to, and it is deliberately
//! unchanged in contract from `crates/buzz-agent/src/lib.rs` (960 lines):
//! the same six JSON-RPC methods, the same `agentInfo.name = "buzz-agent"`,
//! the same `activeRunId` steering handshake, the same `usage_update`
//! ordering. Only the loop underneath is Goose's.
//!
//! Not standard ACP, and preserved here on purpose (each of these is
//! something `buzz-acp` or the desktop UI depends on):
//!
//! * `_goose/unstable/session/steer` with `expectedRunId` optimistic
//!   concurrency, and `activeRunId` advertised via
//!   `params.update._meta.goose.activeRunId` — note the `_meta` nests *inside*
//!   `update` (`buzz-acp/src/acp.rs:1607-1613`). Get the depth wrong and the
//!   harness silently falls back to cancel+re-prompt forever.
//! * `usage_update` on the `_goose/unstable/session/update` channel, emitted
//!   *before* the `session/prompt` response, suppressed when no tokens were
//!   seen (`buzz-agent/src/lib.rs:701-712`).
//! * `keepalive` — see `agent.rs`.

pub mod agent;
pub mod builtin;
pub mod builtin_client;
pub mod config;
pub mod hints;
pub mod hooks;
pub mod mesh;
pub mod types;
pub mod wire;

// Databricks model discovery and the Windows shell-env contract moved to
// `buzz-model-catalog` when this crate took on goose: the desktop cannot link
// goose (native `sqlite3` collision with its own rusqlite), and it only ever
// needed those two things. See that crate's lib.rs for the full reasoning.
pub use types::AgentError;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use goose::agents::{Agent, AgentConfig, ExtensionConfig, GoosePlatform};
use goose::config::PermissionManager;
use goose::session::session_manager::{SessionManager, SessionType};

use config::{Config, MAX_PROMPT_BYTES, MAX_SYSTEM_PROMPT_BYTES, PROTOCOL_VERSION};
use types::McpServerStdio;
use wire::{
    classify, goose_session_update, Inbound, InitializeParams, SessionCancelParams,
    SessionNewParams, SessionPromptParams, SessionSetModelParams, SessionSteerParams, WireSender,
    INVALID_PARAMS, METHOD_NOT_FOUND,
};

/// One live ACP session, wrapping a Goose `Agent`.
struct Session {
    agent: Arc<Agent>,
    /// Goose's own session id (the SQLite-backed conversation).
    goose_session_id: String,
    busy: bool,
    /// Set for the duration of a turn; advertised to steer-capable clients.
    active_run_id: Option<String>,
    cancel: Option<CancellationToken>,
    /// Model id currently in force for this session (`session/set_model`).
    /// Reported back on `usage_update`.
    model_override: Option<String>,
    /// Set by `session/set_model`, consumed by the next `session/prompt`.
    /// Applying it at prompt time rather than immediately matches buzz-agent:
    /// the override takes effect "from the next prompt" and never mutates a
    /// turn already in flight (`buzz-agent/src/lib.rs:494-502`).
    pending_model: Option<String>,
    /// Name of the MCP extension carrying `_Stop`/`_PostCompact`, if any.
    /// See [`crate::hooks`] for why we dispatch these ourselves.
    hook_extension: Option<String>,
    accumulated_input_tokens: u64,
    accumulated_output_tokens: u64,
    /// Subset of `accumulated_input_tokens`, published as
    /// `accumulatedCachedInputTokens` for buzz-acp pricing (#3463).
    accumulated_cached_input_tokens: u64,
    /// Session-cumulative provider total. `None` once any turn fails to report
    /// one -- buzz-acp must not read a missing total as zero (#3593).
    accumulated_total_tokens: Option<u64>,
}

pub struct App {
    cfg: Config,
    sessions: Mutex<HashMap<String, Session>>,
}

/// Build a Goose agent for one ACP session.
///
/// Note `Agent::with_config` loads **zero** extensions — a tool-free agent is
/// the default, not something we switch off (goose `agent.rs:362-420`). Tools
/// arrive only via the `mcpServers` the harness declares in `session/new`,
/// which is how `buzz-dev-mcp` (shell/read_file/str_replace/todo + the
/// `_Stop`/`_PostCompact` hooks + the `buzz`/`rg`/`tree` PATH shim) is wired.
async fn build_agent(
    cfg: &Config,
    cwd: &str,
    system_prompt: Option<&str>,
    mcp_servers: &[McpServerStdio],
) -> Result<(Arc<Agent>, String, Option<String>), AgentError> {
    let session_manager = Arc::new(SessionManager::instance());
    let permission_manager = PermissionManager::instance();

    let agent = Agent::with_config(AgentConfig::new(
        session_manager.clone(),
        permission_manager,
        None,
        cfg.goose_mode,
        // Session naming is a Goose UX affordance; the harness names sessions.
        true,
        GoosePlatform::GooseCli,
    ));

    let session = session_manager
        .create_session(
            std::path::PathBuf::from(cwd),
            "buzz-agent".to_string(),
            SessionType::Acp,
            cfg.goose_mode,
        )
        .await
        .map_err(|e| AgentError::Llm(format!("session create: {e}")))?;

    // Provider. `Agent::with_config` starts with no provider set, so this is
    // mandatory before the first `reply()` — otherwise the turn fails with
    // "Provider not set".
    //
    // Provider/model names come from the `GOOSE_*` variables `Config::from_env`
    // projected out of the `BUZZ_AGENT_*` ones; goose's registry owns base-url
    // and credential resolution from there.
    let provider_name = std::env::var("GOOSE_PROVIDER").unwrap_or_else(|_| "openai".to_string());
    let model_name = cfg
        .model
        .clone()
        .or_else(|| std::env::var("GOOSE_MODEL").ok())
        .ok_or_else(|| AgentError::Llm("no model configured".into()))?;

    let provider = build_provider(&provider_name).await?;
    let model_config =
        goose::model_config::model_config_from_user_config(&provider_name, &model_name)
            .map_err(|e| AgentError::LlmModelNotFound(e.to_string()))?;

    agent
        .update_provider(provider, model_config, &session.id)
        .await
        .map_err(|e| map_provider_error(&e.to_string()))?;

    // Persona. This is the seam that does NOT work over plain ACP: goose's own
    // ACP server never reads `systemPrompt` (rg finds zero hits in
    // goose/crates/goose/src), and both PRs that would have wired it —
    // buzz#1290 and goose#9971 — are closed unmerged. Driving the library
    // directly is what makes Fizz (and `[Base]`) actually arrive.
    if let Some(sp) = system_prompt.or(cfg.system_prompt.as_deref()) {
        if !sp.trim().is_empty() {
            agent.override_system_prompt(sp.to_string()).await;
        }
    }

    for server in mcp_servers {
        let envs: HashMap<String, String> = server
            .env
            .iter()
            .map(|e| (e.name.clone(), e.value.clone()))
            .collect();

        let ext = ExtensionConfig::Stdio {
            name: server.name.clone(),
            description: String::new(),
            cmd: server.command.clone(),
            args: server.args.clone(),
            envs: goose::agents::extension::Envs::new(envs),
            env_keys: Vec::new(),
            timeout: Some(300),
            cwd: Some(cwd.to_string()),
            bundled: None,
            available_tools: Vec::new(),
        };

        if let Err(e) = agent.add_extension(ext, &session.id).await {
            // Match buzz-agent: a failed MCP server is a hard session error,
            // not a silently tool-less agent.
            return Err(AgentError::Mcp(format!("{}: {e}", server.name)));
        }
    }

    // AGENTS.md hints + the skill index. `system_prompt_extras` are appended
    // whether the base prompt came from the override or goose's template
    // (`prompt_manager.rs:170-198`), so this survives `override_system_prompt`.
    let (hints, skills) = crate::hints::build_hints_section(std::path::Path::new(cwd));
    if !hints.trim().is_empty() {
        agent
            .extend_system_prompt("buzz_hints".to_string(), hints)
            .await;
    }

    // `load_skill` runs in-process, registered the way Maple does it: a
    // platform extension backed by an `McpClientTrait`, NOT
    // `ExtensionConfig::Frontend`. Goose advertises frontend tools but refuses
    // to dispatch them -- it blocks on an uncorrelated result channel until the
    // embedder calls `handle_tool_result`, with no timeout and no cancellation,
    // so one missed result wedges the session. A platform client goes through
    // goose's ordinary tool path instead. See `crate::builtin_client`.
    if !skills.is_empty() {
        agent
            .extension_manager
            .add_client(
                crate::builtin_client::BUILTIN_EXTENSION_NAME.to_string(),
                crate::builtin_client::builtin_extension_config(),
                std::sync::Arc::new(crate::builtin_client::BuiltinClient::new(skills)),
                None,
                None,
            )
            .await;
    }

    let agent = Arc::new(agent);

    // Find the extension carrying `_Stop`, by asking rather than assuming the
    // name — `buzz-acp` derives it from the MCP binary's file stem
    // (`buzz-acp/src/lib.rs:4145-4149`), so it is not a fixed string.
    //
    // KNOWN DEVIATION: buzz-agent hid `_`-prefixed tools from the model
    // (`agent.rs:328-336`) while still calling them itself. Goose's
    // `available_tools` allowlist gates advertising *and* dispatch through the
    // same cache (`extension_manager.rs:1421`, `:1698`), so hiding them would
    // also make them undispatchable — which would break the veto. They stay
    // visible; `hook_tool_guidance()` tells the model to leave them alone.
    let mut hook_extension = None;
    for tool in agent.list_tools(&session.id, None).await {
        if tool.name.ends_with("___Stop") {
            hook_extension = tool
                .name
                .strip_suffix("___Stop")
                .map(|prefix| prefix.to_string());
            break;
        }
    }

    if let Some(ext) = &hook_extension {
        agent
            .extend_system_prompt("buzz_hook_tools".to_string(), hook_tool_guidance())
            .await;
        tracing::info!(extension = %ext, "lifecycle hooks available");
    }

    Ok((agent, session.id, hook_extension))
}

/// Keep the model's hands off the lifecycle hooks.
///
/// They are ordinary MCP tools on the wire, so a generic harness advertises
/// them. buzz-agent solved this by hiding them; we cannot (see `build_agent`),
/// so we ask instead.
fn hook_tool_guidance() -> String {
    "Tools whose names begin with an underscore (`_Stop`, `_PostCompact`) are \
     lifecycle hooks invoked automatically by the runtime. Never call them \
     yourself. Use the `todo` tool to manage your task list."
        .to_string()
}

pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    let (wire_tx, wire_rx) = tokio::sync::mpsc::channel(256);
    // Keep the join handle: on EOF we must await it so queued frames -- including
    // the final response of a turn that finished on the same tick -- actually
    // reach stdout before the runtime is dropped.
    let writer = tokio::spawn(wire::writer_task(wire_rx));

    let app = Arc::new(App {
        cfg,
        sessions: Mutex::new(HashMap::new()),
    });

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
    loop {
        match wire::read_bounded_line(&mut stdin, config::MAX_LINE_BYTES).await {
            Ok(None) => break,
            Ok(Some(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(msg) => dispatch(&app, msg, &wire_tx).await,
                    Err(e) => {
                        wire::send(
                            &wire_tx,
                            wire::err(
                                Value::Null,
                                wire::PARSE_ERROR,
                                &format!("jsonrpc: parse: {e}"),
                            ),
                        )
                        .await;
                    }
                }
            }
            Err(e) => {
                tracing::error!("io: {e}");
                break;
            }
        }
    }

    // Orderly shutdown. Both halves matter and both were missing:
    //
    // 1. Cancel every in-flight turn. Dropping a CancellationToken does NOT
    //    cancel it, so goose would never send `notifications/cancelled` to its
    //    MCP children and they would outlive us as orphans -- exactly the
    //    failure `agent::run_turn` goes to lengths to avoid on session/cancel.
    // 2. Await the writer. `wire_tx` is a bounded mpsc drained by a detached
    //    task; returning here would discard anything still queued.
    {
        let sessions = app.sessions.lock().await;
        for session in sessions.values() {
            if let Some(token) = &session.cancel {
                token.cancel();
            }
        }
    }
    drop(wire_tx);
    let _ = writer.await;
    Ok(())
}

async fn dispatch(app: &Arc<App>, msg: Value, wire_tx: &WireSender) {
    match classify(&msg) {
        Inbound::Request { id, method, params } => {
            handle_request(app, id, method, params, wire_tx).await
        }
        Inbound::Notification { method, params } => {
            if method == "session/cancel" {
                cancel_session(app, params).await;
            }
        }
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
            // Spawned so a slow MCP init can't block the read loop — otherwise
            // `session/cancel` can't be received.
            tokio::spawn(async move { session_new(&app, id, params, &wire_tx).await });
        }
        "session/prompt" => {
            let app = app.clone();
            let wire_tx = wire_tx.clone();
            tokio::spawn(async move { session_prompt(&app, id, params, &wire_tx).await });
        }
        "session/set_model" => set_model(app, id, params, wire_tx).await,
        "session/cancel" => {
            cancel_session(app, params).await;
            wire::send(wire_tx, wire::ok(id, Value::Null)).await;
        }
        "_goose/unstable/session/steer" => steer(app, id, params, wire_tx).await,
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

async fn initialize(id: Value, params: Value, wire_tx: &WireSender) {
    let p: InitializeParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, &format!("initialize: {e}")),
            )
            .await
        }
    };
    // Honest negotiation: min(client, ours). Buzz squats on v2 ahead of the
    // upstream ACP RFD (#1237); see buzz-agent/src/lib.rs:279-283.
    let negotiated = p.protocol_version.min(PROTOCOL_VERSION);
    wire::send(
        wire_tx,
        wire::ok(
            id,
            json!({
                "protocolVersion": negotiated,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": false },
                    "mcpCapabilities": { "http": false, "sse": false },
                },
                // Identity is unchanged on purpose: this is still buzz-agent.
                // The `harness` field of the encrypted kind-44200 turn metric
                // derives from this (`buzz-acp/src/pool.rs:3350`), so changing
                // it would blank any dashboard filtering on it.
                "agentInfo": { "name": "buzz-agent", "version": env!("CARGO_PKG_VERSION") },
            }),
        ),
    )
    .await;
}

async fn session_new(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: SessionNewParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, &format!("session/new: {e}")),
            )
            .await
        }
    };

    if !std::path::Path::new(&p.cwd).is_absolute() {
        return wire::send(
            wire_tx,
            wire::err(id, INVALID_PARAMS, "session/new: cwd must be absolute"),
        )
        .await;
    }

    if let Some(sp) = &p.system_prompt {
        if sp.len() > MAX_SYSTEM_PROMPT_BYTES {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, "session/new: systemPrompt too large"),
            )
            .await;
        }
    }

    // Cheap early reject. The authoritative check is re-done under the insert
    // guard below -- `session/new` is dispatched on its own task, so N
    // concurrent calls would otherwise all pass this and all insert.
    if app.sessions.lock().await.len() >= app.cfg.max_sessions {
        return wire::send(
            wire_tx,
            wire::err(id, INVALID_PARAMS, "session/new: max sessions reached"),
        )
        .await;
    }

    let (agent, goose_session_id, hook_extension) =
        match build_agent(&app.cfg, &p.cwd, p.system_prompt.as_deref(), &p.mcp_servers).await {
            Ok(v) => v,
            Err(e) => {
                return wire::send(wire_tx, wire::err(id, e.json_rpc_code(), &e.to_string())).await
            }
        };

    let sid = format!("ses_{}", goose_session_id);

    // Keep a handle for catalog discovery below; the Session takes ownership.
    let session_agent = agent.clone();
    let current_model = app
        .cfg
        .model
        .clone()
        .or_else(|| std::env::var("GOOSE_MODEL").ok())
        .unwrap_or_default();

    {
        let mut sessions = app.sessions.lock().await;
        // Re-check under the guard we insert with: build_agent above spawns MCP
        // children and does a provider round-trip, so other session/new tasks
        // can land in that window.
        if sessions.len() >= app.cfg.max_sessions {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, "session/new: max sessions reached"),
            )
            .await;
        }
        sessions.insert(
            sid.clone(),
            Session {
                agent,
                goose_session_id,
                busy: false,
                active_run_id: None,
                cancel: None,
                model_override: None,
                pending_model: None,
                hook_extension,
                accumulated_input_tokens: 0,
                accumulated_output_tokens: 0,
                accumulated_cached_input_tokens: 0,
                accumulated_total_tokens: Some(0),
            },
        );
    }

    // Advertise the model catalog. `buzz-acp` reads `models.availableModels`
    // off this response (`acp.rs:1866`, `:1900`) to drive the desktop
    // ModelPicker and to resolve `session/set_model` targets
    // (`resolve_model_switch_method`, `acp.rs:1876`). Omitting it degrades the
    // picker to "current model only" — which is what buzz-agent's `catalog.rs`
    // existed to prevent.
    //
    // Goose builds the same structure internally (`build_model_state`,
    // acp/response_builder.rs:130) but it is `pub(super)`, so an embedder
    // cannot call it. The underlying data is public, though:
    // `Provider::fetch_supported_models` (goose-provider-types/base.rs:425).
    let mut result = json!({ "sessionId": sid });
    if let Some(models) = discover_models(&session_agent, &current_model).await {
        result["models"] = models;
    }

    wire::send(wire_tx, wire::ok(id, result)).await;
}

/// Build the `{currentModelId, availableModels}` object for `session/new`.
///
/// Mirrors goose's own `build_model_state`, including its rule that the
/// current model is prepended when the provider's list omits it — otherwise
/// `buzz-acp` cannot resolve a switch back to it.
///
/// Returns `None` when the provider cannot enumerate models (many can't;
/// `fetch_supported_models` defaults to an empty list). A missing catalog is
/// degraded UX, never a session failure — buzz-agent's Databricks discovery
/// made the same choice (`catalog.rs:52-80`).
async fn discover_models(agent: &Arc<Agent>, current_model: &str) -> Option<Value> {
    let provider = agent.provider().await.ok()?;
    let ids = provider.fetch_supported_models().await.ok()?;

    let mut available: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "modelId": id, "name": id }))
        .collect();

    if !ids.iter().any(|id| id == current_model) {
        available.insert(
            0,
            json!({ "modelId": current_model, "name": current_model }),
        );
    }

    if available.is_empty() {
        return None;
    }

    Some(json!({
        "currentModelId": current_model,
        "availableModels": available,
    }))
}

async fn session_prompt(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: SessionPromptParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, &format!("session/prompt: {e}")),
            )
            .await
        }
    };

    let prompt_bytes: usize = p
        .prompt
        .iter()
        .map(|b| match b {
            types::ContentBlock::Text { text } => text.len(),
            types::ContentBlock::ResourceLink { uri } => uri.len(),
            types::ContentBlock::Unsupported => 0,
        })
        .sum();
    if prompt_bytes > MAX_PROMPT_BYTES {
        return wire::send(
            wire_tx,
            wire::err(id, INVALID_PARAMS, "session/prompt: prompt too large"),
        )
        .await;
    }

    let run_id = format!("run_{}", uuid_like());
    let cancel = CancellationToken::new();

    // Single-flight per session, and capture the agent handle.
    let (agent, goose_session_id, pending_model, hook_extension) = {
        let mut sessions = app.sessions.lock().await;
        let Some(s) = sessions.get_mut(&p.session_id) else {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, "session/prompt: unknown session"),
            )
            .await;
        };
        if s.busy {
            return wire::send(
                wire_tx,
                wire::err(
                    id,
                    INVALID_PARAMS,
                    "session/prompt: prompt already in flight",
                ),
            )
            .await;
        }
        s.busy = true;
        s.active_run_id = Some(run_id.clone());
        s.cancel = Some(cancel.clone());
        // Take the pending override so a `session/set_model` applies exactly
        // once, from the next prompt onward (buzz-agent `lib.rs:494-502`).
        (
            s.agent.clone(),
            s.goose_session_id.clone(),
            s.pending_model.take(),
            s.hook_extension.clone(),
        )
    };

    // Apply a pending `session/set_model` before the turn starts.
    if let Some(model_id) = pending_model {
        if let Err(e) = apply_model(&agent, &goose_session_id, &model_id).await {
            {
                let mut sessions = app.sessions.lock().await;
                if let Some(s) = sessions.get_mut(&p.session_id) {
                    s.busy = false;
                    s.active_run_id = None;
                    s.cancel = None;
                }
            }
            return wire::send(wire_tx, wire::err(id, e.json_rpc_code(), &e.to_string())).await;
        }
    }

    // Advertise the run id so steer-capable clients can target this turn.
    // `_meta` nests INSIDE `update` — see the module docs.
    wire::send(
        wire_tx,
        wire::session_update_with_goose_meta(
            &p.session_id,
            json!({ "sessionUpdate": "session_info_update" }),
            json!({ "activeRunId": run_id }),
        ),
    )
    .await;

    let (result, tokens) = agent::run_turn(
        agent,
        &goose_session_id,
        p.prompt,
        app.cfg.max_rounds,
        wire_tx,
        cancel,
        hook_extension.as_deref(),
    )
    .await;

    // Clear run state so a late steer can't queue into a finished turn.
    let accumulated = {
        let mut sessions = app.sessions.lock().await;
        sessions.get_mut(&p.session_id).map(|s| {
            s.busy = false;
            s.active_run_id = None;
            s.cancel = None;
            s.accumulated_input_tokens = s
                .accumulated_input_tokens
                .saturating_add(tokens.input.unwrap_or(0));
            s.accumulated_output_tokens = s
                .accumulated_output_tokens
                .saturating_add(tokens.output.unwrap_or(0));
            s.accumulated_cached_input_tokens = s
                .accumulated_cached_input_tokens
                .saturating_add(tokens.cached_input.unwrap_or(0));
            // Sticky-unknown: one turn without a provider total poisons the
            // session cumulative, so we omit the field rather than under-report.
            s.accumulated_total_tokens = match (s.accumulated_total_tokens, tokens.total) {
                (Some(acc), Some(t)) => Some(acc.saturating_add(t)),
                _ => None,
            };
            (
                s.accumulated_input_tokens,
                s.accumulated_output_tokens,
                s.accumulated_cached_input_tokens,
                s.accumulated_total_tokens,
            )
        })
    };

    wire::send(
        wire_tx,
        wire::session_update_with_goose_meta(
            &p.session_id,
            json!({ "sessionUpdate": "session_info_update" }),
            json!({ "activeRunId": Value::Null }),
        ),
    )
    .await;

    // ORDERING IS LOAD-BEARING: emit usage BEFORE the prompt response.
    // buzz-acp's UsageTracker processes this while the turn is still in
    // flight; the response triggers take_turn_usage(). Reversing these
    // produces zero kind-44200 events, silently.
    if tokens.observed() {
        if let Some((acc_in, acc_out, acc_cached, acc_total)) = accumulated {
            let model = {
                let sessions = app.sessions.lock().await;
                sessions
                    .get(&p.session_id)
                    .and_then(|s| s.model_override.clone())
                    .or_else(|| app.cfg.model.clone())
                    // A session starts fine with only GOOSE_MODEL set
                    // (build_agent falls back to it), so omitting it here
                    // emitted `"model": ""` and blanked kind-44200 attribution.
                    .or_else(|| std::env::var("GOOSE_MODEL").ok())
                    .unwrap_or_default()
            };
            wire::send(
                wire_tx,
                goose_session_update(&p.session_id, {
                    let mut u = json!({
                        "sessionUpdate": "usage_update",
                        "used": acc_in.saturating_add(acc_out),
                        "contextLimit": 0u64,
                        "accumulatedInputTokens": acc_in,
                        "accumulatedOutputTokens": acc_out,
                        // A subset of accumulatedInputTokens, not an
                        // addition to it (#3463).
                        "accumulatedCachedInputTokens": acc_cached,
                        "model": model,
                    });
                    // Only when exactly known; absent means "unknown",
                    // which buzz-acp distinguishes from zero (#3593).
                    if let Some(total) = acc_total {
                        u["accumulatedTotalTokens"] = json!(total);
                    }
                    u
                }),
            )
            .await;
        }
    }

    match result {
        Ok(stop) => {
            wire::send(
                wire_tx,
                wire::ok(id, json!({ "stopReason": stop.as_wire() })),
            )
            .await
        }
        Err(e) => wire::send(wire_tx, wire::err(id, e.json_rpc_code(), &e.to_string())).await,
    }
}

async fn set_model(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: SessionSetModelParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, &format!("session/set_model: {e}")),
            )
            .await
        }
    };
    if p.model_id.trim().is_empty() {
        return wire::send(
            wire_tx,
            wire::err(id, INVALID_PARAMS, "session/set_model: empty modelId"),
        )
        .await;
    }
    let mut sessions = app.sessions.lock().await;
    match sessions.get_mut(&p.session_id) {
        Some(s) => {
            s.model_override = Some(p.model_id.clone());
            s.pending_model = Some(p.model_id);
            drop(sessions);
            wire::send(wire_tx, wire::ok(id, Value::Null)).await;
        }
        None => {
            drop(sessions);
            wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, "session/set_model: unknown session"),
            )
            .await
        }
    }
}

/// Non-cancelling mid-turn steering.
///
/// Goose's own `steer()` has exactly buzz-agent's semantics: the message is
/// queued and drained at the *round boundary* (goose `agent.rs:1951-1974`),
/// the turn is not restarted, and a pending steer even prevents the turn from
/// ending (`agent.rs:2876-2878`).
async fn steer(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: SessionSteerParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, &format!("steer: {e}")),
            )
            .await
        }
    };

    // Reject an empty steer before touching the session map, matching main.
    // buzz-acp maps a successful steer to SteerAck::Ok and considers the user's
    // message delivered, so acknowledging a no-op would swallow it silently and
    // suppress the cancel+merge fallback (`buzz-acp/src/pool.rs:329-366`).
    let text = agent::prompt_to_text(&p.prompt);
    if text.trim().is_empty() {
        return wire::send(
            wire_tx,
            wire::err(id, INVALID_PARAMS, "steer: prompt must not be empty"),
        )
        .await;
    }

    let (agent, goose_session_id) = {
        let sessions = app.sessions.lock().await;
        let Some(s) = sessions.get(&p.session_id) else {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, "steer: unknown session"),
            )
            .await;
        };
        // Optimistic concurrency: the harness distinguishes "no active run"
        // from "run id mismatch" to decide whether to fire its cancel+merge
        // fallback (`buzz-acp/src/pool.rs:329-366`).
        let Some(active) = s.active_run_id.as_deref() else {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, "steer: no active run"),
            )
            .await;
        };
        if active != p.expected_run_id {
            return wire::send(
                wire_tx,
                wire::err(id, INVALID_PARAMS, "steer: run id mismatch"),
            )
            .await;
        }
        (s.agent.clone(), s.goose_session_id.clone())
    };

    let message_id = format!("steer_{}", uuid_like());
    agent
        .steer(
            &goose_session_id,
            goose_provider_types::conversation::message::Message::user().with_text(text),
        )
        .await;

    wire::send(
        wire_tx,
        wire::ok(
            id,
            json!({ "runId": p.expected_run_id, "messageId": message_id }),
        ),
    )
    .await;
}

async fn cancel_session(app: &Arc<App>, params: Value) {
    let Ok(p) = serde_json::from_value::<SessionCancelParams>(params) else {
        return;
    };
    let sessions = app.sessions.lock().await;
    if let Some(s) = sessions.get(&p.session_id) {
        if let Some(c) = &s.cancel {
            // Goose plumbs this token down into MCP tool calls and sends
            // `notifications/cancelled` per in-flight request
            // (goose `mcp_client.rs:687-690`) — a cooperative drain rather
            // than a hard abort, matching buzz-agent's contract.
            c.cancel();
        }
    }
}

/// Swap the model for an existing session.
///
/// Rebuilds the provider (base-url/credential resolution lives in goose's
/// registry) and installs it with the new `ModelConfig`. `SharedProvider` is an
/// `Arc<Mutex<Option<..>>>` precisely so this is hot-swappable
/// (goose `agents/types.rs:11-12`).
/// Construct the goose provider, wrapping it in Buzz's relay-mesh `auto`
/// policy when the desktop asked for it.
///
/// `BUZZ_AGENT_PREFER_MESH_FOR_AUTO=1` is set on every relay-mesh agent
/// (`desktop/src-tauri/src/managed_agents/relay_mesh.rs:41-44`). It means "when
/// the configured model is `auto`, dynamically use mesh-llm's virtual
/// Mixture-of-Agents model whenever the live catalog can support it". Goose
/// resolves a model once per session and has no hook for that, so
/// [`mesh::MeshAutoProvider`] re-resolves per request instead — see that module
/// for why this is only possible with goose-as-a-library.
async fn build_provider(
    provider_name: &str,
) -> Result<Arc<dyn goose::providers::base::Provider>, AgentError> {
    let provider = goose::providers::create(provider_name, Vec::new())
        .await
        .map_err(|e| map_provider_error(&e.to_string()))?;

    let prefer_mesh = std::env::var("BUZZ_AGENT_PREFER_MESH_FOR_AUTO")
        .is_ok_and(|v| !v.trim().is_empty() && v != "0");
    if !prefer_mesh {
        return Ok(provider);
    }

    // The policy needs to poll the router's own `/models`, so it needs the
    // base URL and key. Without them there is nothing to probe, and silently
    // pinning `auto` would look like MoA is broken.
    let Some(base_url) = env_first(&["OPENAI_BASE_URL", "OPENAI_COMPAT_BASE_URL"]) else {
        tracing::warn!(
            "BUZZ_AGENT_PREFER_MESH_FOR_AUTO is set but no OpenAI base URL is \
             configured; relay-mesh auto policy disabled"
        );
        return Ok(provider);
    };
    let api_key = env_first(&["OPENAI_API_KEY", "OPENAI_COMPAT_API_KEY"]).unwrap_or_default();

    tracing::info!(%base_url, "relay-mesh auto policy enabled");
    Ok(Arc::new(mesh::MeshAutoProvider::new(
        provider, base_url, api_key,
    )))
}

fn env_first(keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.trim().is_empty())
}

async fn apply_model(
    agent: &Arc<Agent>,
    session_id: &str,
    model_id: &str,
) -> Result<(), AgentError> {
    let provider_name = std::env::var("GOOSE_PROVIDER").unwrap_or_else(|_| "openai".to_string());
    let provider = build_provider(&provider_name).await?;
    let model_config = goose::model_config::model_config_from_user_config(&provider_name, model_id)
        .map_err(|e| AgentError::LlmModelNotFound(e.to_string()))?;
    agent
        .update_provider(provider, model_config, session_id)
        .await
        .map_err(|e| map_provider_error(&e.to_string()))?;
    Ok(())
}

/// Preserve buzz-agent's error taxonomy across provider construction, so the
/// harness's JSON-RPC code mapping (-32001 auth, -32002 model-not-found) keeps
/// its meaning.
fn map_provider_error(msg: &str) -> AgentError {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("auth") || lower.contains("401") || lower.contains("api key") {
        AgentError::LlmAuth(msg.to_string())
    } else if lower.contains("model") && lower.contains("not found") {
        AgentError::LlmModelNotFound(msg.to_string())
    } else {
        AgentError::Llm(msg.to_string())
    }
}

/// Short random id. Not a UUID; just needs to be unguessable enough that a
/// stale steer can't collide with a live run.
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // `buzz-agent auth <provider>` — interactive login, then exit.
    //
    // Preserved from the pre-goose crate. Goose owns provider auth for the
    // agent loop, but nothing in goose performs an *interactive* Databricks
    // PKCE login, and `buzz-model-catalog/src/auth.rs:417` still tells users
    // to run this exact command when the token cache is empty.
    let args: Vec<String> = std::env::args().collect();
    if matches!(args.get(1).map(String::as_str), Some("auth")) {
        return tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(auth_subcommand(&args[2..]));
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cfg = Config::from_env();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(cfg))?;
    Ok(())
}

/// `buzz-agent auth <provider>` — run a provider's interactive auth flow and
/// persist the result. Needs a browser. Reads `DATABRICKS_HOST` from env.
///
/// The cached token is what lets both the agent loop and the desktop model
/// picker work without a static `DATABRICKS_TOKEN`.
async fn auth_subcommand(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use buzz_model_catalog::auth::{PkceOAuthConfig, PkceOAuthTokenSource};

    match args.first().map(String::as_str) {
        Some("databricks" | "databricks_v2" | "databricks-v2") => {
            let host = std::env::var("DATABRICKS_HOST")
                .map_err(|_| "auth databricks: DATABRICKS_HOST required")?;
            let pkce = PkceOAuthConfig {
                discovery_url: format!(
                    "{}/oidc/.well-known/oauth-authorization-server",
                    host.trim_end_matches('/')
                ),
                client_id: "databricks-cli".into(),
                scopes: vec!["all-apis".into(), "offline_access".into()],
                cache_namespace: "databricks".into(),
                cache_dir_override: None,
            };
            PkceOAuthTokenSource::new(pkce)?.interactive_login().await?;
            eprintln!("Authenticated. Token cached under ~/.config/buzz-agent/oauth/databricks/.");
            Ok(())
        }
        Some(other) => Err(format!("auth: unknown provider {other:?}").into()),
        None => Err("auth: provider required (try: buzz-agent auth databricks)".into()),
    }
}
