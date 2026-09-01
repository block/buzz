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
pub mod config;
pub mod hooks;
pub mod loop_drive;
pub mod mcp;
pub mod model;
pub mod ops;
pub mod permission;
pub mod prompt;
pub mod provider;
pub mod skills;
pub mod steer;
pub mod tools;
pub mod turn_state;
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

use config::{Config, MAX_PROMPT_BYTES, MAX_SYSTEM_PROMPT_BYTES, PROTOCOL_VERSION};
use types::McpServerStdio;
use wire::{
    classify, goose_session_update, Inbound, InitializeParams, SessionCancelParams,
    SessionNewParams, SessionPromptParams, SessionSetModelParams, SessionSteerParams, WireSender,
    INVALID_PARAMS, METHOD_NOT_FOUND,
};

/// One live ACP session, wrapping a Goose `Agent`.
struct Session {
    mcp: Arc<crate::mcp::McpRegistry>,
    /// Session id, shared with goose so tool dispatch and provider request
    /// attribution agree on a name. No database sits behind it: the
    /// conversation lives in `history` and in the turn's own
    /// [`crate::turn_state::TurnState`].
    goose_session_id: String,
    /// The conversation so far, across turns.
    ///
    /// buzz-agent owns this because a Buzz agent's durable record is the
    /// relay, not a local store. Held here so a second `session/prompt` sees
    /// what the first one said.
    history: Vec<goose_provider_types::conversation::message::Message>,
    /// Working directory for the session, as given at `session/new`.
    working_dir: std::path::PathBuf,
    /// Provider + model config in force. Held by buzz rather than pushed into
    /// goose's `Agent`, because `update_provider` writes to the session store.
    model: crate::model::SessionModel,
    busy: bool,
    /// Set for the duration of a turn; advertised to steer-capable clients.
    active_run_id: Option<String>,
    cancel: Option<CancellationToken>,
    /// Set by `session/set_model`, consumed by the next `session/prompt`.
    /// Applying it at prompt time rather than immediately matches buzz-agent:
    /// the override takes effect "from the next prompt" and never mutates a
    /// turn already in flight (`buzz-agent/src/lib.rs:494-502`).
    pending_model: Option<String>,
    /// Name of the MCP extension carrying `_Stop`/`_PostCompact`, if any.
    /// See [`crate::hooks`] for why we dispatch these ourselves.
    hook_extension: Option<String>,
    /// This session's system prompt. Owned here rather than inside goose's
    /// `Agent`, because buzz-agent builds the prompt itself each round.
    prompt: crate::prompt::SessionPrompt,
    /// Mid-turn steer queue. goose's own is `pub(crate)` to `Agent::reply`,
    /// which buzz-agent no longer calls. See [`crate::steer`].
    steers: crate::steer::SteerQueue,
    accumulated_input_tokens: u64,
    accumulated_output_tokens: u64,
    /// Subset of `accumulated_input_tokens`, published as cache-read tokens.
    accumulated_cached_input_tokens: crate::types::CacheTotalState,
    /// Cache-written subset, billed independently from cache reads.
    accumulated_cache_write_tokens: crate::types::CacheTotalState,
    /// Session-cumulative provider total with sticky unknown semantics.
    accumulated_total_tokens: crate::types::TurnTotalState,
    /// Proven publisher identity across every usage-bearing turn. `Some(None)`
    /// is sticky once any turn is untrusted or disagrees.
    accumulated_pricing_identity: Option<Option<crate::types::PricingIdentity>>,
}

pub struct App {
    cfg: Config,
    sessions: Mutex<HashMap<String, Session>>,
    /// ACP protocol version negotiated at `initialize`, stored for the whole
    /// connection lifetime. The `session/request_permission` wire shape derives
    /// from this value — never from a later mutable session field — so a strict
    /// client always receives exactly the shape it negotiated. Defaults to
    /// [`PROTOCOL_VERSION`] before `initialize`; no prompt (and thus no
    /// permission ask) can run before then.
    negotiated_version: std::sync::atomic::AtomicU32,
    /// Owns the entire `session/request_permission` correlation lifecycle:
    /// process-wide admission, id allocation, response delivery, and abort-safe
    /// cleanup. See [`permission::PermissionBroker`].
    permissions: Arc<permission::PermissionBroker>,
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
) -> Result<
    (
        Arc<crate::mcp::McpRegistry>,
        String,
        crate::model::SessionModel,
        Option<String>,
        crate::prompt::SessionPrompt,
    ),
    AgentError,
> {
    let provider_name = std::env::var("GOOSE_PROVIDER").unwrap_or_else(|_| "openai".to_string());
    let model_name = cfg
        .model
        .clone()
        .or_else(|| std::env::var("GOOSE_MODEL").ok())
        .ok_or_else(|| AgentError::Llm("no model configured".into()))?;
    let provider = build_provider(&provider_name).await?;
    let model_config =
        goose::model_config::model_config_from_user_config(&provider_name, &model_name)
            .map_err(|error| AgentError::LlmModelNotFound(error.to_string()))?;
    let model = crate::model::SessionModel::new(provider, model_config, model_name);

    let prompt = crate::prompt::SessionPrompt::new(cfg.goose_mode);
    if let Some(system_prompt) = system_prompt.or(cfg.system_prompt.as_deref()) {
        if !system_prompt.trim().is_empty() {
            prompt.set_override(system_prompt.to_string()).await;
        }
    }

    let mcp = Arc::new(crate::mcp::McpRegistry::spawn_all(cfg, mcp_servers, cwd).await?);
    let skill_index = crate::skills::skill_index(mcp.skills());
    if !skill_index.is_empty() {
        prompt.add_extra("skills", skill_index).await;
    }
    let hook_extension = mcp.hook_extension("_Stop");
    if let Some(extension) = &hook_extension {
        prompt
            .add_extra("buzz_hook_tools", hook_tool_guidance())
            .await;
        tracing::info!(extension, "lifecycle hooks available");
    }
    Ok((mcp, uuid_like(), model, hook_extension, prompt))
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

    let permissions = Arc::new(permission::PermissionBroker::new(
        cfg.max_pending_permissions,
        cfg.permission_timeout,
    ));
    let app = Arc::new(App {
        cfg,
        sessions: Mutex::new(HashMap::new()),
        negotiated_version: std::sync::atomic::AtomicU32::new(PROTOCOL_VERSION),
        permissions,
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
        // Client's answer to a `session/request_permission` we issued. The
        // broker matches it to a live correlation id (waking that waiter) or
        // ignores an unknown/late id.
        Inbound::Response { id, result } => app.permissions.deliver(&id, result),
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
        "initialize" => initialize(app, id, params, wire_tx).await,
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

async fn initialize(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
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
    // Store it for the connection lifetime: the `session/request_permission`
    // wire shape derives from this value, never from a later mutable session
    // field, so a strict client always receives exactly the shape it
    // negotiated at `initialize`.
    app.negotiated_version
        .store(negotiated, std::sync::atomic::Ordering::Relaxed);
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

    let (mcp, goose_session_id, model, hook_extension, prompt) =
        match build_agent(&app.cfg, &p.cwd, p.system_prompt.as_deref(), &p.mcp_servers).await {
            Ok(v) => v,
            Err(e) => {
                return wire::send(wire_tx, wire::err(id, e.json_rpc_code(), &e.to_string())).await
            }
        };

    let sid = format!("ses_{}", goose_session_id);

    // Keep a handle for catalog discovery below; the Session takes ownership.
    let session_model = model.clone();
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
                mcp,
                goose_session_id,
                history: Vec::new(),
                working_dir: std::path::PathBuf::from(&p.cwd),
                model,
                busy: false,
                active_run_id: None,
                cancel: None,
                pending_model: None,
                hook_extension,
                prompt,
                steers: crate::steer::SteerQueue::new(),
                accumulated_input_tokens: 0,
                accumulated_output_tokens: 0,
                accumulated_cached_input_tokens: crate::types::CacheTotalState::Unseen,
                accumulated_cache_write_tokens: crate::types::CacheTotalState::Unseen,
                accumulated_total_tokens: crate::types::TurnTotalState::Unseen,
                accumulated_pricing_identity: None,
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
    if let Some(models) = discover_models(&session_model, &current_model).await {
        result["models"] = models;
    }

    wire::send(wire_tx, wire::ok(id, result)).await;
}

/// Human label for a model id, from the capability manifest when it knows the id.
///
/// The manifest's label registry is Databricks-scoped today, so this is a no-op
/// for every other provider — which is the correct behaviour either way: an
/// unknown id must reach the picker as itself, never blanked or guessed.
fn curated_model_label(id: &str) -> String {
    buzz_model_catalog::model_capabilities::databricks_registry_label(id)
        .unwrap_or(id)
        .to_string()
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
///
/// `name` is curated through the capability manifest (#5597): the provider APIs
/// return no display-name field, so a raw id like `databricks-gpt-5-5` would
/// otherwise reach the picker verbatim instead of `GPT-5.5`. Ids the manifest
/// does not know pass through unchanged, which is also the non-Databricks case.
async fn discover_models(model: &crate::model::SessionModel, current_model: &str) -> Option<Value> {
    let provider = model.provider().await;
    let ids = provider.fetch_supported_models().await.ok()?;

    let mut available: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "modelId": id, "name": curated_model_label(id) }))
        .collect();

    if !ids.iter().any(|id| id == current_model) {
        available.insert(
            0,
            json!({ "modelId": current_model, "name": curated_model_label(current_model) }),
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
    let (
        mcp,
        goose_session_id,
        pending_model,
        hook_extension,
        prompt,
        steers,
        history,
        working_dir,
        model,
    ) = {
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
            s.mcp.clone(),
            s.goose_session_id.clone(),
            s.pending_model.take(),
            s.hook_extension.clone(),
            s.prompt.clone(),
            s.steers.clone(),
            s.history.clone(),
            s.working_dir.clone(),
            s.model.clone(),
        )
    };

    // Apply a pending `session/set_model` before the turn starts.
    if let Some(model_id) = pending_model {
        if let Err(e) = apply_model(&model, &model_id).await {
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

    // Snapshot the exact model identity this turn will use after applying any
    // pending switch. A later `session/set_model` may queue the following turn,
    // but must not relabel usage from this one.
    let (_provider, _config, turn_model_id) = model.snapshot().await;

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

    let (result, tokens, conversation) = loop_drive::run_turn(
        loop_drive::TurnContext {
            mcp: &mcp,
            session_id: &goose_session_id,
            wire_tx,
            cancel: &cancel,
            hook_extension: hook_extension.as_deref(),
            require_reply: app.cfg.require_reply,
            max_rounds: app.cfg.max_rounds,
            prompt: &prompt,
            steers: &steers,
            working_dir,
            history: &history,
            model: &model,
            permissions: &app.permissions,
            protocol_version: app
                .negotiated_version
                .load(std::sync::atomic::Ordering::Relaxed),
        },
        goose_provider_types::conversation::message::Message::user()
            .with_text(agent::prompt_to_text(&p.prompt)),
    )
    .await;

    // Clear run state so a late steer can't queue into a finished turn.
    let accumulated = {
        let mut sessions = app.sessions.lock().await;
        sessions.get_mut(&p.session_id).map(|s| {
            // Carry the turn's conversation forward. Without this the next
            // prompt starts from an empty history and the agent forgets what
            // it just said.
            s.history = conversation;
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
                .merge_session(tokens.cached_input);
            s.accumulated_cache_write_tokens = s
                .accumulated_cache_write_tokens
                .merge_session(tokens.cache_write);
            s.accumulated_total_tokens = s.accumulated_total_tokens.merge_session(tokens.total);
            s.accumulated_pricing_identity = match (
                s.accumulated_pricing_identity.take(),
                tokens.pricing_identity.clone(),
            ) {
                (None, identity) => identity,
                (Some(Some(current)), Some(Some(turn))) if current == turn => Some(Some(current)),
                (Some(_), Some(_)) => Some(None),
                (current, None) => current,
            };
            (
                s.accumulated_input_tokens,
                s.accumulated_output_tokens,
                s.accumulated_cached_input_tokens,
                s.accumulated_cache_write_tokens,
                s.accumulated_total_tokens,
                s.accumulated_pricing_identity.clone(),
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
        if let Some((acc_in, acc_out, acc_cached, acc_written, acc_total, acc_identity)) =
            accumulated
        {
            let update = wire::usage_update_payload(
                Some(acc_in),
                Some(acc_out),
                acc_cached.exact_value(),
                acc_written.exact_value(),
                acc_total,
                &turn_model_id,
                acc_identity.as_ref().and_then(|identity| identity.as_ref()),
            );
            wire::send(wire_tx, goose_session_update(&p.session_id, update)).await;
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

    let steers = {
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
        s.steers.clone()
    };

    let message_id = format!("steer_{}", uuid_like());
    // `with_steer()` marks the message so downstream consumers can tell a
    // steer apart from an ordinary user turn -- goose's own steer path sets
    // it (`agent.rs:540`), and its ACP surface republishes it as
    // `_meta.goose.steer`. buzz-acp does not read it today, but an unmarked
    // steer is indistinguishable from the user having simply sent another
    // message, which is exactly the distinction the flag exists to keep.
    steers
        .push(
            goose_provider_types::conversation::message::Message::user()
                .with_text(text)
                .with_steer(),
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
/// Construct the goose provider. See [`crate::provider`].
async fn build_provider(
    provider_name: &str,
) -> Result<Arc<dyn goose_providers::base::Provider>, AgentError> {
    crate::provider::build(provider_name).await
}

async fn apply_model(model: &crate::model::SessionModel, model_id: &str) -> Result<(), AgentError> {
    let provider_name = std::env::var("GOOSE_PROVIDER").unwrap_or_else(|_| "openai".to_string());
    let provider = build_provider(&provider_name).await?;
    let model_config = goose::model_config::model_config_from_user_config(&provider_name, model_id)
        .map_err(|e| AgentError::LlmModelNotFound(e.to_string()))?;
    // Swapped on buzz's own handle. `Agent::update_provider` would do the same
    // thing plus a write to goose's session row; see [`crate::model`].
    model
        .set(provider, model_config, model_id.to_string())
        .await;
    Ok(())
}

/// Preserve buzz-agent's error taxonomy across provider construction, so the
/// harness's JSON-RPC code mapping (-32001 auth, -32002 model-not-found) keeps
/// its meaning.
pub(crate) fn map_provider_error(msg: &str) -> AgentError {
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
    match args.first().map(String::as_str) {
        Some("databricks" | "databricks_v2" | "databricks-v2") => {
            let host = std::env::var("DATABRICKS_HOST")
                .map_err(|_| "auth databricks: DATABRICKS_HOST required")?;
            buzz_model_catalog::authenticate_databricks(&host).await?;
            eprintln!("Authenticated. Token cached under ~/.config/buzz-agent/oauth/databricks/.");
            Ok(())
        }
        Some(other) => Err(format!("auth: unknown provider {other:?}").into()),
        None => Err("auth: provider required (try: buzz-agent auth databricks)".into()),
    }
}
