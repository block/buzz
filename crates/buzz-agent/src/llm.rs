use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use reqwest::Client;
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::auth::{PkceOAuthConfig, PkceOAuthTokenSource, StaticTokenSource, TokenSource};
use crate::config::{
    is_openai_host, normalize_effort_for_anthropic_route, normalize_effort_for_openai_route,
    Config, OpenAiApi, Provider, ThinkingEffort,
};
use crate::types::{
    AgentError, HistoryItem, LlmResponse, ProviderStop, ToolCall, ToolDef, ToolResultContent,
};

/// Databricks OAuth client_id — the public Databricks-published CLI client.
/// PKCE-only, no secret. Same identifier goose uses, so a user's browser
/// consent for `databricks-cli` covers buzz-agent too.
const DATABRICKS_CLIENT_ID: &str = "databricks-cli";
const DATABRICKS_OAUTH_SCOPES: &[&str] = &["all-apis", "offline_access"];

const MAX_LLM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LLM_ERROR_BODY_BYTES: usize = 4 * 1024;
const STALL_NOTICE_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(300);
const MESH_VIRTUAL_MODEL_ID: &str = "mesh";
const MESH_AUTO_MODEL_ID: &str = "auto";
const MESH_AUTO_CATALOG_TTL: std::time::Duration = std::time::Duration::from_secs(5);
const MESH_AUTO_CATALOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const MESH_AUTO_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);
const MESH_AUTO_ENABLE_OBSERVATIONS: u8 = 2;
const MESH_MOA_UNAVAILABLE_MESSAGE: &str = "MoA requires ≥2 models available in the mesh";

/// Parser for an OpenAI-family JSON response. Per-endpoint pair lives
/// alongside its `_body` serializer.
type OpenAiParse = fn(Value) -> Result<LlmResponse, AgentError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshCatalogObservation {
    Available,
    Unavailable,
    Unknown,
}

fn mesh_catalog_supports_collective(catalog: &Value) -> Option<bool> {
    let models = catalog.get("data").and_then(Value::as_array)?;
    let mut has_virtual_mesh = false;
    let mut physical_models = BTreeSet::new();
    for id in models
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if id == MESH_VIRTUAL_MODEL_ID {
            has_virtual_mesh = true;
        } else if id != MESH_AUTO_MODEL_ID {
            physical_models.insert(id.replace("@main", ""));
        }
    }
    Some(has_virtual_mesh && physical_models.len() >= 2)
}

fn looks_like_unstructured_tool_call(text: &str) -> bool {
    let text = text.trim_start().to_ascii_lowercase();
    text.starts_with("<|tool_call")
        || text.starts_with("<tool_call")
        || text.starts_with("[tool_call]")
}

#[derive(Debug, Default)]
struct MeshAutoState {
    last_checked: Option<Instant>,
    consecutive_available: u8,
    collective_enabled: bool,
    cooldown_until: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabricksV2Route {
    OpenAiResponses,
    AnthropicMessages,
    MlflowChatCompletions,
}

pub struct Llm {
    http: Client,
    /// One-shot sticky flag: set when a Chat Completions request comes
    /// back with a "use /v1/responses" provider error while `cfg.openai_api
    /// == Auto`. Subsequent OpenAI calls then go straight to Responses
    /// for the lifetime of the process.
    auto_upgraded: AtomicBool,
    /// Hysteretic view of whether mesh-llm currently advertises its virtual
    /// Mixture-of-Agents model. A TTL, confirmation count, and failure cooldown
    /// let long-running agents adapt without bouncing as peers briefly flap.
    mesh_auto_state: Mutex<MeshAutoState>,
    /// Bearer-token source for OpenAI-family requests. Static for OpenAI
    /// (the `OPENAI_COMPAT_API_KEY` env var) and Databricks-with-token
    /// (the `DATABRICKS_TOKEN` env var); a refreshable PKCE engine for
    /// Databricks otherwise. Anthropic doesn't use this — it always
    /// reads `cfg.api_key` directly because the API expects `x-api-key`.
    auth: Arc<dyn TokenSource>,
}

impl Llm {
    pub fn new(cfg: &Config) -> Result<Self, AgentError> {
        let http = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(cfg.llm_timeout)
            .build()
            .map_err(|e| AgentError::Llm(format!("http: {e}")))?;
        let auth = build_token_source(cfg)?;
        Ok(Self {
            http,
            auto_upgraded: AtomicBool::new(false),
            mesh_auto_state: Mutex::new(MeshAutoState::default()),
            auth,
        })
    }

    pub async fn complete(
        &self,
        cfg: &Config,
        system_prompt: &str,
        history: &[HistoryItem],
        tools: &[ToolDef],
        effective_model: &str,
    ) -> Result<LlmResponse, AgentError> {
        let effort = cfg.thinking_effort;
        let result = match cfg.provider {
            Provider::Anthropic => self
                .post_anthropic(
                    cfg,
                    &anthropic_body(cfg, system_prompt, history, tools, effective_model, effort),
                )
                .await
                .and_then(parse_anthropic),
            Provider::OpenRouter => {
                let mut body =
                    openai_body(cfg, system_prompt, history, tools, effective_model, None);
                apply_openrouter_mutations(
                    &mut body,
                    cfg.thinking_effort,
                    effective_model,
                    cfg.prompt_caching,
                );
                self.post_openrouter(cfg, &body)
                    .await
                    .and_then(parse_openai_with_reasoning_details)
            }
            Provider::OpenAi | Provider::Databricks => {
                self.openai_request(
                    cfg,
                    effective_model,
                    !tools.is_empty(),
                    |use_responses, request_model| {
                        // Normalize effort for model-specific availability. Startup no longer rejects
                        // `max` for pure OpenAI/Databricks; this per-model table is the single authority
                        // — it keeps `max` for gpt-5.6, clamps `max`→`xhigh` for other OpenAI-shaped
                        // models, and still applies corrections like none→minimal on the gpt-5 base.
                        let e =
                            effort.map(|ef| normalize_effort_for_openai_route(ef, request_model));
                        if use_responses {
                            (
                                responses_body(
                                    cfg,
                                    system_prompt,
                                    history,
                                    tools,
                                    request_model,
                                    e,
                                ),
                                parse_responses as OpenAiParse,
                            )
                        } else {
                            (
                                openai_body(cfg, system_prompt, history, tools, request_model, e),
                                parse_openai as OpenAiParse,
                            )
                        }
                    },
                )
                .await
            }
            Provider::DatabricksV2 => {
                self.databricks_v2_request(cfg, effective_model, |route| match route {
                    DatabricksV2Route::OpenAiResponses => {
                        // OpenAI Responses path: normalize effort against the per-model table.
                        let e =
                            effort.map(|ef| normalize_effort_for_openai_route(ef, effective_model));
                        (
                            responses_body(cfg, system_prompt, history, tools, effective_model, e),
                            parse_responses as OpenAiParse,
                        )
                    }
                    DatabricksV2Route::AnthropicMessages => {
                        // Anthropic Messages path: normalize effort (none|minimal → omit).
                        let e = effort.and_then(normalize_effort_for_anthropic_route);
                        (
                            anthropic_body(cfg, system_prompt, history, tools, effective_model, e),
                            parse_anthropic as OpenAiParse,
                        )
                    }
                    DatabricksV2Route::MlflowChatCompletions => {
                        // MLflow Chat path (OpenAI-shaped): normalize effort against the per-model table.
                        let e =
                            effort.map(|ef| normalize_effort_for_openai_route(ef, effective_model));
                        (
                            openai_body(cfg, system_prompt, history, tools, effective_model, e),
                            parse_openai as OpenAiParse,
                        )
                    }
                })
                .await
            }
        };
        // Stamp the effective model into Llm errors so log lines carry
        // `llm: (model-name) 404 Not Found: …` instead of the bare status.
        // The `llm: ` prefix comes from `Display for AgentError::Llm`; the
        // map_err here prepends `(model-name) ` to the inner string only.
        // This is the single place all provider paths converge, so the mapping
        // is centralized and never needs to be repeated in each provider arm.
        // Every arm above returns its `Result` into this mapper rather than
        // using `?` — an early return would silently skip the stamp, which is
        // exactly what the Anthropic and OpenRouter arms used to do.
        result.map_err(|e| match e {
            AgentError::Llm(s) => AgentError::Llm(format!("({effective_model}) {s}")),
            AgentError::LlmModelNotFound(s) => {
                AgentError::LlmModelNotFound(format!("({effective_model}) {s}"))
            }
            // Stamped like the others: this is the error most likely to be read
            // during an incident, so it must name the model whose window was
            // exceeded. Without an explicit arm it would fall through `other`
            // and be the only unstamped provider error.
            AgentError::LlmContextExceeded(s) => {
                AgentError::LlmContextExceeded(format!("({effective_model}) {s}"))
            }
            other => other,
        })
    }

    pub async fn summarize(
        &self,
        cfg: &Config,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: u32,
        effective_model: &str,
    ) -> Result<String, AgentError> {
        match cfg.provider {
            Provider::Anthropic => {
                let body = json!({
                    "model": effective_model,
                    "max_tokens": max_output_tokens,
                    "system": system_prompt,
                    "messages": [{
                        "role": "user",
                        "content": [{ "type": "text", "text": user_prompt }],
                    }],
                });
                Ok(parse_anthropic(self.post_anthropic(cfg, &body).await?)?.text)
            }
            Provider::OpenRouter => {
                let body = openrouter_summary_body(
                    effective_model,
                    system_prompt,
                    user_prompt,
                    max_output_tokens,
                );
                let v = self.post_openrouter(cfg, &body).await?;
                Ok(parse_openai(v)?.text)
            }
            Provider::OpenAi | Provider::Databricks => {
                let r = self
                    .openai_request(
                        cfg,
                        effective_model,
                        false,
                        |use_responses, request_model| {
                            if use_responses {
                                (
                                    json!({
                                        "model": request_model,
                                        "max_output_tokens": max_output_tokens,
                                        "instructions": system_prompt,
                                        "input": user_prompt,
                                    }),
                                    parse_responses as OpenAiParse,
                                )
                            } else {
                                (
                                    json!({
                                        "model": request_model,
                                        "stream": false,
                                        "max_completion_tokens": max_output_tokens,
                                        "messages": [
                                            { "role": "system", "content": system_prompt },
                                            { "role": "user", "content": user_prompt },
                                        ],
                                    }),
                                    parse_openai as OpenAiParse,
                                )
                            }
                        },
                    )
                    .await?;
                Ok(r.text)
            }
            Provider::DatabricksV2 => {
                let r = self
                    .databricks_v2_request(cfg, effective_model, |route| match route {
                        DatabricksV2Route::OpenAiResponses => (
                            json!({
                                "model": effective_model,
                                "max_output_tokens": max_output_tokens,
                                "instructions": system_prompt,
                                "input": user_prompt,
                            }),
                            parse_responses as OpenAiParse,
                        ),
                        DatabricksV2Route::AnthropicMessages => (
                            json!({
                                "model": effective_model,
                                "max_tokens": max_output_tokens,
                                "system": system_prompt,
                                "messages": [{
                                    "role": "user",
                                    "content": [{ "type": "text", "text": user_prompt }],
                                }],
                            }),
                            parse_anthropic as OpenAiParse,
                        ),
                        DatabricksV2Route::MlflowChatCompletions => (
                            json!({
                                "model": effective_model,
                                "stream": false,
                                "max_completion_tokens": max_output_tokens,
                                "messages": [
                                    { "role": "system", "content": system_prompt },
                                    { "role": "user", "content": user_prompt },
                                ],
                            }),
                            parse_openai as OpenAiParse,
                        ),
                    })
                    .await?;
                Ok(r.text)
            }
        }
    }

    async fn post_anthropic(&self, cfg: &Config, body: &Value) -> Result<Value, AgentError> {
        let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
        post(&self.http, &url, body, false, |r| {
            r.header("x-api-key", &cfg.api_key)
                .header("anthropic-version", &cfg.anthropic_api_version)
        })
        .await
        .map_err(PostError::into_agent)
    }

    /// OpenAI dispatch with Buzz's relay-mesh `auto` policy layered over the
    /// normal endpoint selection. When enabled, a live virtual `mesh` model is
    /// preferred; if the mesh contracts between discovery and inference, retry
    /// the same request once through the router's ordinary `auto` model.
    async fn openai_request<F>(
        &self,
        cfg: &Config,
        effective_model: &str,
        tools_supplied: bool,
        mut build: F,
    ) -> Result<LlmResponse, AgentError>
    where
        F: FnMut(bool, &str) -> (Value, OpenAiParse) + Send,
    {
        let request_model = self.resolve_openai_model(cfg, effective_model).await;
        let adaptive_mesh =
            effective_model == MESH_AUTO_MODEL_ID && request_model == MESH_VIRTUAL_MODEL_ID;

        let first = self
            .openai_request_for_model(cfg, &request_model, &mut build)
            .await;
        match first {
            Err(PostError::MeshFallback(detail)) if adaptive_mesh => {
                self.cool_down_collective().await;
                tracing::warn!(
                    configured_model = effective_model,
                    attempted_model = MESH_VIRTUAL_MODEL_ID,
                    fallback_model = MESH_AUTO_MODEL_ID,
                    provider_message = detail,
                    "relay-mesh auto: collective request failed; retrying once with auto"
                );
                self.openai_request_for_model(cfg, MESH_AUTO_MODEL_ID, &mut build)
                    .await
                    .map_err(PostError::into_agent)
            }
            Ok(response)
                if adaptive_mesh
                    && tools_supplied
                    && response.tool_calls.is_empty()
                    && looks_like_unstructured_tool_call(&response.text) =>
            {
                self.cool_down_collective().await;
                tracing::warn!(
                    configured_model = effective_model,
                    attempted_model = MESH_VIRTUAL_MODEL_ID,
                    fallback_model = MESH_AUTO_MODEL_ID,
                    "relay-mesh auto: collective response emitted unstructured tool markup; retrying once with auto"
                );
                self.openai_request_for_model(cfg, MESH_AUTO_MODEL_ID, &mut build)
                    .await
                    .map_err(PostError::into_agent)
            }
            Ok(response) => Ok(response),
            Err(error) => Err(error.into_agent()),
        }
    }

    async fn cool_down_collective(&self) {
        let now = Instant::now();
        let mut state = self.mesh_auto_state.lock().await;
        state.last_checked = Some(now);
        state.consecutive_available = 0;
        state.collective_enabled = false;
        state.cooldown_until = Some(now + MESH_AUTO_COOLDOWN);
    }

    /// Resolve the model for one OpenAI-family request. Explicit model choices
    /// are never changed. Relay-mesh `auto` dynamically follows the short-lived
    /// `/models` catalog so long-running agents can adopt or leave MoA without
    /// being restarted.
    async fn resolve_openai_model(&self, cfg: &Config, effective_model: &str) -> String {
        if cfg.provider != Provider::OpenAi
            || !cfg.prefer_mesh_for_auto
            || effective_model != MESH_AUTO_MODEL_ID
        {
            return effective_model.to_string();
        }

        let mut state = self.mesh_auto_state.lock().await;
        let now = Instant::now();
        if let Some(cooldown_until) = state.cooldown_until {
            if now < cooldown_until {
                return MESH_AUTO_MODEL_ID.to_string();
            }
            state.cooldown_until = None;
        }
        if state
            .last_checked
            .is_some_and(|checked_at| checked_at.elapsed() < MESH_AUTO_CATALOG_TTL)
        {
            return if state.collective_enabled {
                MESH_VIRTUAL_MODEL_ID.to_string()
            } else {
                MESH_AUTO_MODEL_ID.to_string()
            };
        }

        let observation = self.observe_mesh_virtual_model(cfg).await;
        let checked_at = Instant::now();
        state.last_checked = Some(checked_at);
        match observation {
            MeshCatalogObservation::Available => {
                state.consecutive_available = state.consecutive_available.saturating_add(1);
                if state.consecutive_available >= MESH_AUTO_ENABLE_OBSERVATIONS {
                    state.collective_enabled = true;
                }
            }
            MeshCatalogObservation::Unavailable => {
                if state.collective_enabled {
                    state.cooldown_until = Some(checked_at + MESH_AUTO_COOLDOWN);
                }
                state.consecutive_available = 0;
                state.collective_enabled = false;
            }
            // A failed catalog check is not evidence that a previously stable
            // mesh disappeared. Preserve the last confirmed state; inference
            // itself remains authoritative and has the narrow 503 fallback.
            MeshCatalogObservation::Unknown => {}
        }
        let request_model = if state.collective_enabled {
            MESH_VIRTUAL_MODEL_ID
        } else {
            MESH_AUTO_MODEL_ID
        };
        tracing::debug!(
            configured_model = effective_model,
            request_model,
            "relay-mesh auto: resolved request model from live catalog"
        );
        request_model.to_string()
    }

    async fn observe_mesh_virtual_model(&self, cfg: &Config) -> MeshCatalogObservation {
        let url = format!("{}/models", cfg.base_url.trim_end_matches('/'));
        let bearer = match self.auth.bearer().await {
            Ok(bearer) => bearer,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "relay-mesh auto: catalog auth unavailable; preserving last confirmed route"
                );
                return MeshCatalogObservation::Unknown;
            }
        };
        let response = match self
            .http
            .get(&url)
            .bearer_auth(bearer)
            .timeout(MESH_AUTO_CATALOG_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "relay-mesh auto: catalog probe failed; preserving last confirmed route"
                );
                return MeshCatalogObservation::Unknown;
            }
        };
        if !response.status().is_success() {
            tracing::debug!(
                status = %response.status(),
                "relay-mesh auto: catalog probe was not successful; preserving last confirmed route"
            );
            return MeshCatalogObservation::Unknown;
        }
        match response.json::<Value>().await {
            Ok(catalog) => {
                let Some(available) = mesh_catalog_supports_collective(&catalog) else {
                    tracing::debug!(
                        "relay-mesh auto: catalog response has no data array; preserving last confirmed route"
                    );
                    return MeshCatalogObservation::Unknown;
                };
                if available {
                    MeshCatalogObservation::Available
                } else {
                    MeshCatalogObservation::Unavailable
                }
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    "relay-mesh auto: invalid catalog response; preserving last confirmed route"
                );
                MeshCatalogObservation::Unknown
            }
        }
    }

    /// Dispatch one OpenAI request for an already-resolved model. Endpoint
    /// selection remains unchanged: pinned > sticky-upgraded > host default,
    /// with the existing one-shot Chat-to-Responses upgrade.
    async fn openai_request_for_model<F>(
        &self,
        cfg: &Config,
        request_model: &str,
        build: &mut F,
    ) -> Result<LlmResponse, PostError>
    where
        F: FnMut(bool, &str) -> (Value, OpenAiParse) + Send,
    {
        let use_responses = self.auto_upgraded.load(Ordering::Relaxed)
            || matches!(cfg.openai_api, OpenAiApi::Responses)
            || matches!(cfg.openai_api, OpenAiApi::Auto) && is_openai_host(&cfg.base_url);

        if use_responses {
            let (body, parse) = build(true, request_model);
            return parse(
                self.post_openai(cfg, "/responses", &body, request_model)
                    .await?,
            )
            .map_err(PostError::from);
        }
        let (body, parse) = build(false, request_model);
        match self
            .post_openai(cfg, "/chat/completions", &body, request_model)
            .await
        {
            Ok(value) => parse(value).map_err(PostError::from),
            Err(PostError::Agent(error))
                if cfg.openai_api == OpenAiApi::Auto && self.try_upgrade(&error) =>
            {
                let (body, parse) = build(true, request_model);
                parse(
                    self.post_openai(cfg, "/responses", &body, request_model)
                        .await?,
                )
                .map_err(PostError::from)
            }
            Err(error) => Err(error),
        }
    }

    async fn databricks_v2_request<F>(
        &self,
        cfg: &Config,
        effective_model: &str,
        build: F,
    ) -> Result<LlmResponse, AgentError>
    where
        F: FnOnce(DatabricksV2Route) -> (Value, OpenAiParse) + Send,
    {
        let route = databricks_v2_route_for_model(effective_model);
        let (body, parse) = build(route);
        parse(
            self.post_openai(cfg, databricks_v2_path(route), &body, effective_model)
                .await
                .map_err(PostError::into_agent)?,
        )
    }

    /// POST to an OpenAI-family endpoint. For OpenAI-compat this is just
    /// `{base_url}{path}` with the body untouched. For Databricks the URL
    /// becomes `{base_url}/serving-endpoints/{model}/invocations` and the
    /// `model` field is stripped from the body (Databricks rejects it —
    /// the endpoint path already names the model).
    async fn post_openai(
        &self,
        cfg: &Config,
        path: &str,
        body: &Value,
        effective_model: &str,
    ) -> Result<Value, PostError> {
        let (url, body_owned);
        let body_ref: &Value = match cfg.provider {
            Provider::Databricks => {
                url = format!(
                    "{}/serving-endpoints/{}/invocations",
                    cfg.base_url.trim_end_matches('/'),
                    effective_model
                );
                body_owned = strip_model(body);
                &body_owned
            }
            _ => {
                url = format!("{}{}", cfg.base_url.trim_end_matches('/'), path);
                body
            }
        };

        // A 401 or 403 can mean the local expiry clock disagreed with the
        // server (skew, revocation, a node that never saw the token). On the
        // first such rejection, force a refresh keyed off the rejected bearer
        // and retry once. The guard is local to this call so an earlier turn's
        // rejection can never suppress a later turn's legitimate retry. Both
        // statuses map to `LlmAuth` in `post`: a 403 is indistinguishable from
        // an expired-token 403 here, so we refresh once and let it propagate.
        let mut bearer = self.auth.bearer().await.map_err(PostError::from)?;
        let mut refreshed = false;
        loop {
            match post(
                &self.http,
                &url,
                body_ref,
                effective_model == MESH_VIRTUAL_MODEL_ID,
                |r| r.bearer_auth(&bearer),
            )
            .await
            {
                Err(PostError::Agent(AgentError::LlmAuth(_))) if !refreshed => {
                    refreshed = true;
                    bearer = self
                        .auth
                        .refresh_now(&bearer)
                        .await
                        .map_err(PostError::from)?;
                }
                result => return result,
            }
        }
    }

    async fn post_openrouter(&self, cfg: &Config, body: &Value) -> Result<Value, AgentError> {
        let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
        let mut bearer = self.auth.bearer().await?;
        let mut refreshed = false;
        loop {
            match openrouter_post(&self.http, &url, body, &bearer).await {
                Err(AgentError::LlmAuth(_)) if !refreshed => {
                    refreshed = true;
                    let new_bearer = self.auth.refresh_now(&bearer).await?;
                    // A static key refreshes to itself — a byte-identical retry
                    // would be a guaranteed duplicate request against a key the
                    // server just rejected. Fail terminal immediately; the retry
                    // is only meaningful when the source can actually mint a
                    // distinct token (e.g., a PKCE OAuth source).
                    if new_bearer == bearer {
                        return Err(AgentError::LlmAuth(
                            "401: static key rejected — update key in agent settings".into(),
                        ));
                    }
                    bearer = new_bearer;
                }
                result => return result,
            }
        }
    }

    /// If `err` names `/v1/responses` / "use the Responses API", latch a
    /// sticky upgrade so subsequent OpenAI calls hit Responses. Logged once.
    fn try_upgrade(&self, err: &AgentError) -> bool {
        let body = match err {
            AgentError::Llm(s) => s.as_str(),
            _ => return false, // auth/transport aren't "use the other endpoint" signals
        };
        if !is_responses_required_error(body) {
            return false;
        }
        if !self.auto_upgraded.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                provider_message = body,
                "openai: provider asked for the Responses API; \
                 routing subsequent OpenAI calls to /v1/responses for this process"
            );
        }
        true
    }
}

fn anthropic_body(
    cfg: &Config,
    system_prompt: &str,
    history: &[HistoryItem],
    tools: &[ToolDef],
    effective_model: &str,
    effort: Option<ThinkingEffort>,
) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    let mut pending: Vec<Value> = Vec::new();
    let flush = |out: &mut Vec<Value>, p: &mut Vec<Value>| {
        if !p.is_empty() {
            out.push(json!({ "role": "user", "content": std::mem::take(p) }));
        }
    };
    for item in history {
        match item {
            HistoryItem::User(text) => {
                flush(&mut messages, &mut pending);
                messages.push(json!({ "role": "user",
                    "content": [{ "type": "text", "text": text }] }));
            }
            HistoryItem::Assistant {
                text,
                tool_calls,
                reasoning_details: _,
            } => {
                flush(&mut messages, &mut pending);
                let mut content: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                for c in tool_calls {
                    content.push(json!({ "type": "tool_use", "id": c.provider_id,
                        "name": c.name, "input": c.arguments }));
                }
                if content.is_empty() {
                    // Empty assistant turn (no text, no tool calls) — skip it.
                    // Anthropic rejects empty text blocks, and a placeholder
                    // just defers the problem. No tool_use = no pairing
                    // constraint, so omitting is safe.
                    continue;
                }
                messages.push(json!({ "role": "assistant", "content": content }));
            }
            HistoryItem::ToolResult(r) => pending.push(json!({
                "type": "tool_result", "tool_use_id": r.provider_id,
                "content": anthropic_tool_result_content(&r.content), "is_error": r.is_error })),
        }
    }
    flush(&mut messages, &mut pending);
    // Rolling cache breakpoint: mark the tail of the (append-only) conversation
    // so the next turn re-reads this whole prefix from cache instead of paying
    // full input price for it. See `stamp_rolling_cache_breakpoint`.
    if cfg.prompt_caching {
        stamp_rolling_cache_breakpoint(&mut messages);
    }
    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
        "name": t.name, "description": t.description, "input_schema": t.input_schema })
        })
        .collect();
    // Static prefix breakpoint: caching the `system` block caches the whole
    // prefix up to and including it — and the prefix order is
    // `tools -> system -> messages`, so this single marker caches tools +
    // system together. Requires the structured (array) form of `system`; skip
    // it for an empty prompt since Anthropic rejects empty text blocks.
    let system_value = if cfg.prompt_caching && !system_prompt.is_empty() {
        json!([{ "type": "text", "text": system_prompt,
            "cache_control": { "type": "ephemeral" } }])
    } else {
        json!(system_prompt)
    };
    let mut body = json!({ "model": effective_model, "max_tokens": cfg.max_output_tokens,
        "system": system_value, "messages": messages });
    if let Some(e) = effort {
        let (thinking, output_config) =
            crate::config::anthropic_thinking_config(effective_model, e, cfg.max_output_tokens);
        if let Some(t) = thinking {
            body["thinking"] = t;
        }
        if let Some(oc) = output_config {
            body["output_config"] = oc;
        }
    }
    if !tools_json.is_empty() {
        body["tools"] = Value::Array(tools_json);
    }
    body
}

/// Attach ephemeral `cache_control` markers to the tail of the conversation so
/// the next turn re-reads the whole prior prefix from cache (~0.1x input price)
/// rather than re-billing it as fresh input. Anthropic caches the prefix up to
/// and including each marked block.
///
/// We mark the last content block of the last *two* messages, not just the
/// final one. Each Anthropic breakpoint walks back at most 20 content blocks to
/// find a prior cache entry, and one agentic turn can append ~17 blocks at the
/// default `max_parallel_tools` (1 assistant text + N `tool_use` +
/// N `tool_result`). With only a tail marker, consecutive breakpoints sit a
/// full turn apart, which slips past the 20-block window as soon as parallelism
/// rises or a turn carries extra blocks — and the miss is silent. Marking the
/// last two messages halves the gap (to ~N+1 blocks), keeping a live cache
/// entry comfortably within reach. Uses 2 of the 4 allowed breakpoints; the
/// static `system` marker is the third.
///
/// A no-op for messages whose content is empty or whose tail block is not a
/// JSON object.
fn stamp_rolling_cache_breakpoint(messages: &mut [Value]) {
    let n = messages.len();
    // The two most-recently-appended messages (the current turn's tool results
    // and the assistant turn before them). `checked_sub` + `flatten` skips the
    // second index when there is only one message.
    for idx in [n.checked_sub(1), n.checked_sub(2)].into_iter().flatten() {
        if let Some(block) = messages[idx]
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .and_then(|c| c.last_mut())
            .and_then(Value::as_object_mut)
        {
            block.insert("cache_control".into(), json!({ "type": "ephemeral" }));
        }
    }
}

fn anthropic_tool_result_content(content: &[ToolResultContent]) -> Vec<Value> {
    content
        .iter()
        .map(|c| match c {
            ToolResultContent::Text(text) => json!({ "type": "text", "text": text }),
            ToolResultContent::Image { data, mime_type } => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": mime_type, "data": data },
            }),
        })
        .collect()
}

fn openai_body(
    cfg: &Config,
    system_prompt: &str,
    history: &[HistoryItem],
    tools: &[ToolDef],
    effective_model: &str,
    effort: Option<ThinkingEffort>,
) -> Value {
    let mut messages: Vec<Value> = vec![json!({ "role": "system", "content": system_prompt })];
    // Images returned from tool calls ride on a trailing `role:"user"`
    // message because OpenAI Chat's `role:"tool"` content is text-only. We
    // batch them across a run of adjacent ToolResult items so that all
    // `role:"tool"` messages stay contiguous — splitting them with a user
    // turn breaks OpenAI-Chat-compatible frontends that translate back to
    // Anthropic `tool_result` (notably Databricks model serving), since
    // Anthropic requires every `tool_use` in one assistant turn to be
    // answered by a single immediately-following user message.
    let mut pending_images: Vec<Value> = Vec::new();
    let flush_images = |messages: &mut Vec<Value>, pending: &mut Vec<Value>| {
        if !pending.is_empty() {
            messages.push(json!({ "role": "user", "content": std::mem::take(pending) }));
        }
    };
    for item in history {
        match item {
            HistoryItem::User(text) => {
                flush_images(&mut messages, &mut pending_images);
                messages.push(json!({ "role": "user", "content": text }));
            }
            HistoryItem::Assistant {
                text,
                tool_calls,
                reasoning_details,
            } => {
                flush_images(&mut messages, &mut pending_images);
                let mut msg = serde_json::Map::new();
                msg.insert("role".into(), json!("assistant"));
                msg.insert("content".into(), json!(text.as_str()));
                if let Some(details) = reasoning_details {
                    msg.insert("reasoning_details".into(), details.clone());
                }
                if !tool_calls.is_empty() {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|c| {
                            let mut call = serde_json::Map::new();
                            call.insert("id".into(), json!(c.provider_id));
                            call.insert("type".into(), json!("function"));
                            // Provider-owned fields go back beside `function`,
                            // which is where the provider put them. Position is
                            // load-bearing, not cosmetic: Gemini rejects a
                            // `thoughtSignature` nested inside `function{}` with
                            // the same 400 it gives for one that is missing.
                            for (k, v) in &c.provider_extra {
                                call.insert(k.clone(), v.clone());
                            }
                            call.insert(
                                "function".into(),
                                json!({ "name": c.name,
                                    "arguments": serde_json::to_string(&c.arguments)
                                        .unwrap_or_else(|_| "{}".into()) }),
                            );
                            Value::Object(call)
                        })
                        .collect();
                    msg.insert("tool_calls".into(), Value::Array(calls));
                }
                messages.push(Value::Object(msg));
            }
            HistoryItem::ToolResult(r) => {
                messages.push(json!({
                    "role": "tool", "tool_call_id": r.provider_id,
                    "content": openai_tool_text_content(&r.content) }));
                pending_images.extend(openai_image_user_content(&r.content));
            }
        }
    }
    flush_images(&mut messages, &mut pending_images);
    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
        "type": "function",
        "function": { "name": t.name, "description": t.description,
            "parameters": t.input_schema } })
        })
        .collect();
    let mut body = json!({ "model": effective_model, "stream": false,
        "max_completion_tokens": cfg.max_output_tokens, "messages": messages });
    if let Some(e) = effort {
        body["reasoning_effort"] = json!(e.openai_effort_str());
    }
    if !tools_json.is_empty() {
        body["tools"] = Value::Array(tools_json);
        body["tool_choice"] = json!("auto");
    }
    body
}

fn openai_tool_text_content(content: &[ToolResultContent]) -> String {
    let mut parts = Vec::new();
    for c in content {
        match c {
            ToolResultContent::Text(text) => parts.push(text.clone()),
            ToolResultContent::Image { data, mime_type } => parts.push(format!(
                "This tool result included an image ({mime_type}, {} base64 bytes) that is provided in the next user message.",
                data.len()
            )),
        }
    }
    parts.join("\n")
}

fn openai_image_user_content(content: &[ToolResultContent]) -> Vec<Value> {
    content
        .iter()
        .filter_map(|c| match c {
            ToolResultContent::Image { data, mime_type } => Some(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{mime_type};base64,{data}") },
            })),
            ToolResultContent::Text(_) => None,
        })
        .collect()
}

// Spec: https://platform.openai.com/docs/api-reference/responses
//
// Replay invariant: each assistant `function_call` input item **must**
// precede its matching `function_call_output`, or the API rejects with
// "No tool call found for call_id ...". `HistoryItem` ordering already
// guarantees this.

fn responses_body(
    cfg: &Config,
    system_prompt: &str,
    history: &[HistoryItem],
    tools: &[ToolDef],
    effective_model: &str,
    effort: Option<ThinkingEffort>,
) -> Value {
    let mut input: Vec<Value> = Vec::with_capacity(history.len());
    for item in history {
        match item {
            HistoryItem::User(text) => input.push(json!({
                "role": "user",
                "content": [{ "type": "input_text", "text": text }],
            })),
            HistoryItem::Assistant {
                text,
                tool_calls,
                reasoning_details: _,
            } => {
                if !text.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }],
                    }));
                }
                for c in tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": c.provider_id,
                        "name": c.name,
                        "arguments": serde_json::to_string(&c.arguments)
                            .unwrap_or_else(|_| "{}".into()),
                    }));
                }
            }
            HistoryItem::ToolResult(r) => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": r.provider_id,
                    "output": openai_tool_text_content(&r.content),
                }));
                // Responses takes images as `input_image` parts on a user message.
                let images: Vec<Value> = r
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ToolResultContent::Image { data, mime_type } => Some(json!({
                            "type": "input_image",
                            "image_url": format!("data:{mime_type};base64,{data}"),
                        })),
                        ToolResultContent::Text(_) => None,
                    })
                    .collect();
                if !images.is_empty() {
                    input.push(json!({ "role": "user", "content": images }));
                }
            }
        }
    }

    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.input_schema,
            })
        })
        .collect();

    let mut body = json!({
        "model": effective_model,
        "instructions": system_prompt,
        "max_output_tokens": cfg.max_output_tokens,
        "input": input,
    });
    if let Some(e) = effort {
        body["reasoning"] = json!({ "effort": e.openai_effort_str() });
    }
    if !tools_json.is_empty() {
        body["tools"] = Value::Array(tools_json);
        body["tool_choice"] = json!("auto");
    }
    body
}

/// Narrow matcher for "the input exceeded the model's context window" provider
/// errors — the ground-truth signal that history must shrink. Only consulted
/// alongside an HTTP 400 (see the two `!status.is_success()` classification
/// sites), never on its own: the phrases below are specific, but pairing them
/// with the status keeps an unrelated 4xx that happens to quote one of them
/// from triggering a recovery.
///
/// Deliberately tight. A generic 400 must stay `AgentError::Llm` so it remains
/// terminal — misclassifying one as recoverable would spend the whole recovery
/// budget on an error that shrinking history cannot fix, replacing a clear
/// failure with a slow one.
fn is_context_length_error(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    // OpenAI/Databricks machine-readable code; the most reliable marker.
    b.contains("context_length_exceeded")
        // Prose forms: OpenAI's classic phrasing and the Databricks gateway's
        // "context window of this model" variant seen in both bug reports.
        || b.contains("maximum context length")
        || b.contains("context window")
        // Anthropic: "prompt is too long: N tokens > M maximum".
        || b.contains("prompt is too long")
}

/// Narrow matcher for "you should be on the Responses API" provider errors,
/// the signal we use to auto-upgrade. Triggers on the literal path
/// `/v1/responses` (Databricks GPT-5.5 phrasing) or the prose
/// "use the Responses API" / "Responses API instead".
fn is_responses_required_error(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    b.contains("/v1/responses")
        || b.contains("responses api instead")
        || b.contains("use the responses api")
}

/// OpenAI-family code names that appear as their own segment in a Databricks v2
/// endpoint name (the GPT-5 launch aliases). The `gpt` family itself is matched
/// separately by segment prefix so `gpt`, `gpt5`, and the `gpt` of a split
/// `gpt-5` all qualify.
const DATABRICKS_V2_OPENAI_CODE_NAMES: &[&str] = &["sol", "luna", "terra"];

/// Anthropic (Claude) family and release code names that appear as their own
/// segment in a Databricks v2 endpoint name — the `claude` prefix, the family
/// names (`opus`, `sonnet`, `haiku`), and the release code names (`mythos`,
/// `fable`). Getting a Claude model onto the Anthropic Messages route is what
/// lets it carry a `cache_control` breakpoint; an endpoint that matches none of
/// these falls through to the MLflow (OpenAI-wire) path, where Anthropic prompt
/// caching is structurally impossible and the discount is silently lost.
const DATABRICKS_V2_CLAUDE_NAMES: &[&str] =
    &["claude", "opus", "sonnet", "haiku", "mythos", "fable"];

/// Split a Databricks v2 endpoint name into its lowercase alphanumeric segments,
/// breaking on any non-alphanumeric delimiter (`-`, `_`, `.`, `/`, …). E.g.
/// `Databricks-Claude-Opus-5` -> `["databricks", "claude", "opus", "5"]`.
fn model_name_segments(model: &str) -> Vec<String> {
    model
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn databricks_v2_route_for_model(model: &str) -> DatabricksV2Route {
    // The v2 catalog exposes no family field, so the wire format is inferred
    // from the endpoint name. Discovery deliberately keeps arbitrary custom
    // aliases, so we match whole name *segments* rather than raw substrings: a
    // substring test would misroute unrelated names — `consolidated-llama`
    // (`sol`), `terraform-coder` (`terra`), `corpus-reranker`/`octopus-model`
    // (`opus`) — onto a wire whose request shape their backend can't parse,
    // turning a caching optimization into a hard request/parse failure. Segment
    // matching still accepts real prefixed names like `goose-opus-5`.
    let segments = model_name_segments(model);
    let has_named_segment =
        |names: &[&str]| segments.iter().any(|seg| names.contains(&seg.as_str()));
    // `gpt` family: any segment beginning with `gpt` — covers `gpt`, `gpt5`, and
    // the `gpt` segment of a split `gpt-5`, without matching mid-word.
    let is_gpt_family = segments.iter().any(|seg| seg.starts_with("gpt"));
    // OpenAI is checked before Claude so a name carrying both markers resolves
    // to the OpenAI wire (preserving the prior `gpt-5`-first precedence).
    if is_gpt_family || has_named_segment(DATABRICKS_V2_OPENAI_CODE_NAMES) {
        DatabricksV2Route::OpenAiResponses
    } else if has_named_segment(DATABRICKS_V2_CLAUDE_NAMES) {
        DatabricksV2Route::AnthropicMessages
    } else {
        DatabricksV2Route::MlflowChatCompletions
    }
}

fn databricks_v2_path(route: DatabricksV2Route) -> &'static str {
    match route {
        DatabricksV2Route::OpenAiResponses => "/ai-gateway/openai/v1/responses",
        DatabricksV2Route::AnthropicMessages => "/ai-gateway/anthropic/v1/messages",
        DatabricksV2Route::MlflowChatCompletions => "/ai-gateway/mlflow/v1/chat/completions",
    }
}

fn parse_responses(v: Value) -> Result<LlmResponse, AgentError> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    let mut saw_function_call = false;

    for item in v
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                for p in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    // Responses emits "output_text"; accept "text" forward-compat.
                    if matches!(
                        p.get("type").and_then(Value::as_str),
                        Some("output_text" | "text")
                    ) {
                        if let Some(t) = p.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        }
                    }
                }
            }
            Some("function_call") => {
                saw_function_call = true;
                let raw = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let args: Value = serde_json::from_str(raw).map_err(|e| {
                    AgentError::Llm(format!("function_call.arguments not valid JSON: {e}"))
                })?;
                // No passthrough on this route: `responses_body` replays a
                // function call as `{call_id, name, arguments}` and the Responses
                // API asks for nothing else, so an empty map keeps the request
                // byte-identical to before.
                tool_calls.push(make_tool_call(
                    str_field(item, "call_id"),
                    str_field(item, "name"),
                    args,
                    Default::default(),
                )?);
            }
            Some("reasoning") => {
                // Reasoning summary items from the Responses API. Each item has a
                // `summary` array of `{"type": "summary_text", "text": "..."}` objects.
                for s in item
                    .get("summary")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if matches!(
                        s.get("type").and_then(Value::as_str),
                        Some("summary_text" | "text")
                    ) {
                        if let Some(t) = s.get("text").and_then(Value::as_str) {
                            if !reasoning.is_empty() {
                                reasoning.push('\n');
                            }
                            reasoning.push_str(t);
                        }
                    }
                }
            }
            // Unknown types ignored for forward-compat.
            _ => {}
        }
    }

    let stop = match v.get("status").and_then(Value::as_str) {
        Some("incomplete") => {
            let reason = v
                .get("incomplete_details")
                .and_then(|d| d.get("reason"))
                .and_then(Value::as_str);
            if reason == Some("max_output_tokens") {
                ProviderStop::MaxTokens
            } else {
                ProviderStop::Other
            }
        }
        Some("completed") if saw_function_call => ProviderStop::ToolUse,
        Some("completed") => ProviderStop::EndTurn,
        _ => ProviderStop::Other,
    };
    let input_tokens = sum_usage(&v, &["input_tokens"]);
    let output_tokens = sum_usage(&v, &["output_tokens"]);
    // The Responses API nests the cache split under `input_tokens_details`.
    let cached_input_tokens = usage_first(
        &v,
        &["cache_read_input_tokens"],
        &[("input_tokens_details", "cached_tokens")],
    );
    // Responses API reports a genuine provider total. Read it directly —
    // never derived, so it stays None when the provider omits it.
    let total_tokens = sum_usage(&v, &["total_tokens"]);
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
        input_tokens,
        cached_input_tokens,
        output_tokens,
        total_tokens,
        reasoning,
        reasoning_details: None,
    })
}

fn map_stop(s: Option<&str>) -> ProviderStop {
    match s {
        Some("end_turn" | "stop") => ProviderStop::EndTurn,
        Some("tool_use" | "tool_calls") => ProviderStop::ToolUse,
        Some("max_tokens" | "length") => ProviderStop::MaxTokens,
        Some("refusal" | "content_filter") => ProviderStop::Refusal,
        _ => ProviderStop::Other,
    }
}

/// Sum a set of `usage` token fields, returning `None` only when the `usage`
/// object is absent or carries none of the requested fields. A field that is
/// present is added; a field that is missing contributes 0. This keeps the
/// result an inclusive total (so cached tokens are never silently dropped)
/// while still distinguishing "no usage reported" from "usage was zero".
fn sum_usage(v: &Value, fields: &[&str]) -> Option<u64> {
    let usage = v.get("usage")?;
    let mut total: u64 = 0;
    let mut saw_any = false;
    for f in fields {
        if let Some(n) = usage.get(*f).and_then(Value::as_u64) {
            total = total.saturating_add(n);
            saw_any = true;
        }
    }
    saw_any.then_some(total)
}

/// Input-token total for Anthropic / Databricks (Anthropic-style) responses.
/// `input_tokens` alone EXCLUDES cached tokens, so we sum it with the two
/// cache fields to get the inclusive total the context budget must gate on.
fn anthropic_input_tokens(v: &Value) -> Option<u64> {
    sum_usage(
        v,
        &[
            "input_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ],
    )
}

/// Input-token total for OpenAI Chat Completions and Databricks MLflow-route
/// responses. `prompt_tokens` is already the inclusive input total on both, so
/// it is read alone and never summed with the cache fields.
///
/// Vanilla OpenAI nests the cache split under `prompt_tokens_details` and
/// `prompt_tokens` includes it. The Databricks MLflow route reports the split
/// with the flat Anthropic spelling (`cache_read_input_tokens`) *alongside* an
/// already-inclusive `prompt_tokens` — so summing double-counts. Verified on
/// `databricks-glm-5-2` (2026-07-28): `prompt_tokens 13320`,
/// `cache_read_input_tokens 13312`, `completion_tokens 30`, `total_tokens
/// 13350`; since `prompt_tokens + completion_tokens == total_tokens`, the 13312
/// cached tokens are contained in the 13320, not additional to it. Summing gave
/// 26632 — nearly double — inflating both the context-budget gate and cost.
///
/// This differs from Anthropic's native route (see [`anthropic_input_tokens`]),
/// where `input_tokens` genuinely EXCLUDES the cache fields and must be summed.
/// The two never collide here: the router sends `claude*` models to the
/// Anthropic route, so `parse_openai` only ever sees inclusive `prompt_tokens`.
fn openai_chat_input_tokens(v: &Value) -> Option<u64> {
    sum_usage(v, &["prompt_tokens"])
}

/// First present value among `usage.<field>` and `usage.<nested>.<leaf>` pairs.
///
/// Cache counts are the one usage figure providers do not agree on the shape of.
/// Anthropic puts `cache_read_input_tokens` flat on `usage`; OpenAI nests the
/// same quantity one level down, under `prompt_tokens_details` on
/// `/chat/completions` and `input_tokens_details` on `/responses`. [`sum_usage`]
/// only reads flat keys, which is why the OpenAI split was invisible for so
/// long: `prompt_tokens` is already inclusive, so the *total* was right and
/// nothing looked broken while the discount silently went unclaimed.
///
/// Returns the first candidate that resolves, not a sum — these are alternative
/// spellings of one number, so adding them would double-count on Databricks,
/// which reports both shapes.
fn usage_first(v: &Value, flat: &[&str], nested: &[(&str, &str)]) -> Option<u64> {
    let usage = v.get("usage")?;
    for f in flat {
        if let Some(n) = usage.get(*f).and_then(Value::as_u64) {
            return Some(n);
        }
    }
    for (outer, leaf) in nested {
        if let Some(n) = usage
            .get(*outer)
            .and_then(|o| o.get(*leaf))
            .and_then(Value::as_u64)
        {
            return Some(n);
        }
    }
    None
}

/// Cache-read tokens for an OpenAI Chat Completions response.
///
/// `prompt_tokens_details.cached_tokens` is where vanilla OpenAI reports it.
/// The flat Anthropic spelling is checked first for Databricks, which routes
/// Anthropic models through an OpenAI-shaped envelope.
fn openai_chat_cached_tokens(v: &Value) -> Option<u64> {
    usage_first(
        v,
        &["cache_read_input_tokens"],
        &[("prompt_tokens_details", "cached_tokens")],
    )
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// Append `part` to `buf` on its own line, ignoring empties.
fn push_part(buf: &mut String, part: &str) {
    if part.is_empty() {
        return;
    }
    if !buf.is_empty() {
        buf.push('\n');
    }
    buf.push_str(part);
}

/// Split an OpenAI-shaped `message.content` into `(text, reasoning)`.
///
/// Standard OpenAI sends a string. Several models on the Databricks MLflow route
/// — Gemini, Qwen35, gpt-oss — send an array of typed blocks instead, and
/// `as_str()` yields nothing for an array, so their entire answer was being
/// discarded: no error, no warning, just a turn that looked like the model had
/// said nothing. `parse_anthropic` already walks a block array; this gives
/// `parse_openai` the same tolerance.
fn openai_content_parts(content: Option<&Value>) -> (String, String) {
    let mut text = String::new();
    let mut reasoning = String::new();
    match content {
        Some(Value::String(s)) => text.push_str(s),
        Some(Value::Array(blocks)) => {
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => push_part(&mut text, &str_field(b, "text")),
                    Some("reasoning") => match b.get("summary").and_then(Value::as_array) {
                        // Gemini nests the prose one level down under `summary`.
                        Some(summary) => {
                            for s in summary {
                                push_part(&mut reasoning, &str_field(s, "text"));
                            }
                        }
                        None => push_part(&mut reasoning, &str_field(b, "text")),
                    },
                    // An untyped block carrying text is still the model talking;
                    // treating it as text loses nothing and keeps one more
                    // provider out of the silent-empty-answer failure mode.
                    _ => push_part(&mut text, &str_field(b, "text")),
                }
            }
        }
        _ => {}
    }
    (text, reasoning)
}

/// Make `provider_id` unique across one assistant turn's tool calls.
///
/// Gemini returns the function name as the id, so two parallel calls to the same
/// function arrive sharing one id — and that id is what pairs a `role:"tool"`
/// result back to its call, leaving two results indistinguishable. Rewriting is
/// safe because both halves of that pairing are re-emitted from this same value;
/// the provider never sees its original id again.
fn dedupe_provider_ids(calls: &mut [ToolCall]) {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for c in calls.iter_mut() {
        if seen.contains(&c.provider_id) {
            let mut n = 2;
            while seen.contains(&format!("{}-{n}", c.provider_id)) {
                n += 1;
            }
            c.provider_id = format!("{}-{n}", c.provider_id);
        }
        seen.insert(c.provider_id.clone());
    }
}

fn parse_anthropic(v: Value) -> Result<LlmResponse, AgentError> {
    let stop = map_stop(v.get("stop_reason").and_then(Value::as_str));
    let mut tool_calls = Vec::new();
    let mut text = String::new();
    let mut reasoning = String::new();
    if let Some(blocks) = v.get("content").and_then(Value::as_array) {
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                Some("thinking") => {
                    // Anthropic extended thinking block: `{"type": "thinking", "thinking": "..."}`
                    if let Some(t) = b.get("thinking").and_then(Value::as_str) {
                        if !reasoning.is_empty() {
                            reasoning.push('\n');
                        }
                        reasoning.push_str(t);
                    }
                }
                // Anthropic's replay shape is fully modelled, so nothing to keep.
                Some("tool_use") => tool_calls.push(make_tool_call(
                    str_field(b, "id"),
                    str_field(b, "name"),
                    b.get("input").cloned().unwrap_or(Value::Null),
                    Default::default(),
                )?),
                _ => {}
            }
        }
    }
    let input_tokens = anthropic_input_tokens(&v);
    let output_tokens = sum_usage(&v, &["output_tokens"]);
    // Anthropic reports the cache split flat on `usage`. Note this is already
    // part of `input_tokens` above, which sums it in deliberately.
    let cached_input_tokens = usage_first(&v, &["cache_read_input_tokens"], &[]);
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
        input_tokens,
        cached_input_tokens,
        output_tokens,
        // Anthropic reports only category counts; NIP-AM forbids deriving a
        // total from them. Always None for this provider.
        total_tokens: None,
        reasoning,
        reasoning_details: None,
    })
}

fn parse_openai(v: Value) -> Result<LlmResponse, AgentError> {
    // A5: error-inside-200 check — choice-level `finish_reason == "error"`
    if let Some(choice) = v
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        if choice.get("finish_reason").and_then(Value::as_str) == Some("error") {
            let err = choice.get("error").cloned().unwrap_or(Value::Null);
            // OpenRouter's `error.code` is numeric; other OpenAI-compat hosts
            // may send a string. Accept either rather than discarding the
            // typed code as "unknown".
            let code = err
                .get("code")
                .and_then(|c| {
                    c.as_str()
                        .map(str::to_string)
                        .or_else(|| c.as_i64().map(|n| n.to_string()))
                })
                .unwrap_or_else(|| "unknown".into());
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("provider error in 200 response");
            let error_type = err
                .get("metadata")
                .and_then(|m| m.get("error_type"))
                .and_then(Value::as_str);
            return Err(AgentError::Llm(match error_type {
                Some(et) => format!("provider error ({code}, {et}): {message}"),
                None => format!("provider error ({code}): {message}"),
            }));
        }
    }
    let choice = v
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| AgentError::Llm("response missing choices".into()))?;
    let stop = map_stop(choice.get("finish_reason").and_then(Value::as_str));
    let msg = choice
        .get("message")
        .ok_or_else(|| AgentError::Llm("missing message".into()))?;
    let (text, block_reasoning) = openai_content_parts(msg.get("content"));
    // DeepSeek and vLLM-style OpenAI-compat hosts expose reasoning tokens on the
    // message object. Prefer `reasoning_content` (DeepSeek's field name); fall
    // back to `reasoning` (some other providers), and last to reasoning blocks
    // found inside `content`. All three are absent for standard OpenAI
    // responses, which leaves this empty without any special-casing.
    let reasoning = {
        let rc = str_field(msg, "reasoning_content");
        let rc = if rc.is_empty() {
            str_field(msg, "reasoning")
        } else {
            rc
        };
        if rc.is_empty() {
            block_reasoning
        } else {
            rc
        }
    };
    let mut tool_calls = Vec::new();
    if let Some(arr) = msg.get("tool_calls").and_then(Value::as_array) {
        for tc in arr {
            let f = tc
                .get("function")
                .ok_or_else(|| AgentError::Llm("tool_call missing function".into()))?;
            let raw = f.get("arguments").and_then(Value::as_str).unwrap_or("{}");
            let args: Value = serde_json::from_str(raw)
                .map_err(|e| AgentError::Llm(format!("tool_call.arguments not valid JSON: {e}")))?;
            // Everything on the wire object we do not model, kept for replay.
            let extra = tc
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter(|(k, _)| !matches!(k.as_str(), "id" | "type" | "function"))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            tool_calls.push(make_tool_call(
                str_field(tc, "id"),
                str_field(f, "name"),
                args,
                extra,
            )?);
        }
    }
    dedupe_provider_ids(&mut tool_calls);
    let input_tokens = openai_chat_input_tokens(&v);
    let output_tokens = sum_usage(&v, &["completion_tokens"]);
    let cached_input_tokens = openai_chat_cached_tokens(&v);
    // OpenAI Chat Completions reports a genuine provider total. Read it
    // directly — never derived, so it stays None when the provider omits it.
    let total_tokens = sum_usage(&v, &["total_tokens"]);
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
        input_tokens,
        cached_input_tokens,
        output_tokens,
        total_tokens,
        reasoning,
        reasoning_details: None,
    })
}

fn parse_openai_with_reasoning_details(v: Value) -> Result<LlmResponse, AgentError> {
    let reasoning_details = v
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("reasoning_details"))
        .filter(|rd| rd.is_array())
        .cloned();
    let mut response = parse_openai(v)?;
    response.reasoning_details = reasoning_details;
    Ok(response)
}

fn make_tool_call(
    id: String,
    name: String,
    args: Value,
    provider_extra: Map<String, Value>,
) -> Result<ToolCall, AgentError> {
    if id.is_empty() || name.is_empty() {
        return Err(AgentError::Llm("tool_call missing id or name".into()));
    }
    let arguments = match args {
        Value::Object(_) => args,
        Value::Null => Value::Object(Default::default()),
        _ => {
            return Err(AgentError::Llm(
                "tool_call arguments must be a JSON object".into(),
            ))
        }
    };
    Ok(ToolCall {
        provider_id: id,
        name,
        arguments,
        provider_extra,
    })
}

async fn read_error_body(mut resp: reqwest::Response) -> String {
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < MAX_LLM_ERROR_BODY_BYTES {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let take = chunk.len().min(MAX_LLM_ERROR_BODY_BYTES - buf.len());
                buf.extend_from_slice(&chunk[..take]);
                if take < chunk.len() {
                    break;
                }
            }
            _ => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

const MAX_RETRIES: u32 = 3;
const BASE_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_MS: u64 = 8_000;

async fn backoff_with_jitter(attempt: u32) {
    let base = BASE_BACKOFF_MS
        .saturating_mul(1u64 << attempt)
        .min(MAX_BACKOFF_MS);
    let mut buf = [0u8; 8];
    let jitter_range = base / 2;
    let delay = if jitter_range > 0 && getrandom::fill(&mut buf).is_ok() {
        let r = u64::from_le_bytes(buf) % jitter_range;
        base - jitter_range + r
    } else {
        base
    };
    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
}

/// Transport-layer errors safe to retry for non-streaming LLM POSTs.
///
/// Covers timeouts, connect failures, and the broader request-class errors
/// reqwest reports for pre-response failures: TLS handshake aborts, sockets
/// dropped or reset mid-send, h2 GOAWAY/RST_STREAM, hyper protocol errors.
/// Body-serialization happens before the retry loop, so `is_request()` here
/// is always a network failure, never a malformed request we'd just resend.
fn is_retryable_transport_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request()
}

fn is_unsupported_image_input_error(body: &str) -> bool {
    body.to_ascii_lowercase()
        .contains("no endpoints found that support image input")
}

/// Build the terminal `AgentError::Llm` for a `post()` exit that has given up
/// retrying — persistent retryable status, transport failure, or a body-read
/// break. `detail` carries the specific cause (status/body, or the transport
/// error text); `elapsed` and `attempts` are the cumulative cost of every
/// attempt made on this call, including the retries that failed before this
/// one. When cumulative time crosses `STALL_NOTICE_THRESHOLD`, this also logs
/// a `tracing::warn!` so a slow-building outage is visible in logs even
/// before an operator reads the returned error text.
fn terminal_llm_error(elapsed: std::time::Duration, attempts: u32, detail: &str) -> AgentError {
    if elapsed >= STALL_NOTICE_THRESHOLD {
        tracing::warn!(
            cumulative_stall = ?elapsed,
            attempts,
            "llm: cumulative stall {elapsed:?} across {attempts} attempts ({detail})"
        );
    }
    AgentError::Llm(format!(
        "{detail} (cumulative {elapsed:?}, {attempts} attempt{})",
        if attempts == 1 { "" } else { "s" },
    ))
}

/// Internal HTTP failure that preserves a mesh-specific MoA failure as a
/// typed signal. This covers both pre-inference eligibility rejection and a
/// structured `moa_failure` from workers/reducers. It is consumed inside
/// `openai_request`: an adaptive `auto` call falls back once, while an explicit
/// `mesh` call is surfaced as the ordinary LLM error callers already
/// understand.
#[derive(Debug)]
enum PostError {
    Agent(AgentError),
    MeshFallback(String),
}

impl PostError {
    fn into_agent(self) -> AgentError {
        match self {
            Self::Agent(error) => error,
            Self::MeshFallback(detail) => AgentError::Llm(detail),
        }
    }
}

impl From<AgentError> for PostError {
    fn from(error: AgentError) -> Self {
        Self::Agent(error)
    }
}

fn is_mesh_moa_unavailable_body(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .as_deref()
        == Some(MESH_MOA_UNAVAILABLE_MESSAGE)
}

fn is_mesh_moa_failure_body(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/type")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .as_deref()
        == Some("moa_failure")
}

async fn post<F>(
    http: &Client,
    url: &str,
    body: &Value,
    detect_mesh_fallback: bool,
    apply: F,
) -> Result<Value, PostError>
where
    F: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
{
    let body_bytes = serde_json::to_vec(body)
        .map_err(|e| PostError::Agent(AgentError::Llm(format!("serialize: {e}"))))?;
    let call_start = std::time::Instant::now();
    for attempt in 0..MAX_RETRIES {
        let resp = match apply(
            http.post(url)
                .header("content-type", "application/json")
                .body(body_bytes.clone()),
        )
        .send()
        .await
        {
            Ok(r) => r,
            Err(e) => {
                if attempt + 1 < MAX_RETRIES && is_retryable_transport_error(&e) {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = MAX_RETRIES,
                        error = %e,
                        "llm: transport error, retrying"
                    );
                    backoff_with_jitter(attempt).await;
                    continue;
                }
                return Err(PostError::Agent(terminal_llm_error(
                    call_start.elapsed(),
                    attempt + 1,
                    &format!("transport: {e}"),
                )));
            }
        };
        let status = resp.status();
        // Both 401 and 403 are treated as refreshable: a 403 can mean an
        // expired or revoked token, not just a pure authorization verdict, and
        // the two are indistinguishable at the HTTP-status layer. The caller's
        // retry loop keys off `LlmAuth` and refreshes once; the per-call guard
        // bounds a pure-authz 403 to one wasted refresh before it propagates.
        // Not a stall path: auth failures are not surfaced through
        // terminal_llm_error, they resolve on the next call after refresh.
        if status == 401 || status == 403 {
            return Err(PostError::Agent(AgentError::LlmAuth(
                read_error_body(resp).await,
            )));
        }
        if status.is_server_error() || status == 429 || status.as_u16() == 499 {
            let body = read_error_body(resp).await;
            let mesh_fallback = detect_mesh_fallback
                && (status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                    && is_mesh_moa_unavailable_body(&body)
                    || status.is_server_error() && is_mesh_moa_failure_body(&body));
            if mesh_fallback {
                return Err(PostError::MeshFallback(format!("{status}: {body}")));
            }
            if attempt + 1 < MAX_RETRIES {
                tracing::warn!(
                    attempt = attempt + 1,
                    max_attempts = MAX_RETRIES,
                    %status,
                    "llm: retryable status, retrying"
                );
                backoff_with_jitter(attempt).await;
                continue;
            }
            return Err(PostError::Agent(terminal_llm_error(
                call_start.elapsed(),
                attempt + 1,
                &format!("exhausted retries: {status}: {body}"),
            )));
        }
        // Not a stall path: the model is misconfigured, not the transport or
        // upstream capacity — no retry was attempted, so cumulative duration
        // would be misleading.
        if status == 404 {
            let error_body = read_error_body(resp).await;
            if is_unsupported_image_input_error(&error_body) {
                return Err(PostError::Agent(AgentError::UnsupportedImageInput(
                    error_body,
                )));
            }
            return Err(PostError::Agent(AgentError::LlmModelNotFound(format!(
                "{status}: {error_body}"
            ))));
        }
        if !status.is_success() {
            let body = read_error_body(resp).await;
            // Context-window overflow is a recovery signal, not a terminal
            // error: classify it here, where status and body are still separate
            // values. Callers must never re-derive this from the formatted
            // string — `Llm::complete` stamps the model name onto it before the
            // agent loop ever sees it.
            if status == 400 && is_context_length_error(&body) {
                return Err(PostError::Agent(AgentError::LlmContextExceeded(format!(
                    "{status}: {body}"
                ))));
            }
            return Err(PostError::Agent(AgentError::Llm(format!(
                "{status}: {body}"
            ))));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_LLM_RESPONSE_BYTES {
                return Err(PostError::Agent(AgentError::Llm(format!(
                    "response too large: {len} > {MAX_LLM_RESPONSE_BYTES}"
                ))));
            }
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp;
        loop {
            match stream.chunk().await {
                Ok(Some(chunk)) => {
                    if buf.len() + chunk.len() > MAX_LLM_RESPONSE_BYTES {
                        return Err(PostError::Agent(AgentError::Llm(format!(
                            "response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                        ))));
                    }
                    buf.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(PostError::Agent(terminal_llm_error(
                        call_start.elapsed(),
                        attempt + 1,
                        &format!("body read: {e}"),
                    )));
                }
            }
        }
        return serde_json::from_slice(&buf)
            .map_err(|e| PostError::Agent(AgentError::Llm(format!("json: {e}"))));
    }
    unreachable!("loop always returns on its final iteration (attempt + 1 == MAX_RETRIES)");
}

pub(crate) fn databricks_pkce_config(host: &str) -> PkceOAuthConfig {
    PkceOAuthConfig {
        discovery_url: format!(
            "{}/oidc/.well-known/oauth-authorization-server",
            host.trim_end_matches('/')
        ),
        client_id: DATABRICKS_CLIENT_ID.into(),
        scopes: DATABRICKS_OAUTH_SCOPES
            .iter()
            .map(|scope| (*scope).into())
            .collect(),
        cache_namespace: "databricks".into(),
        cache_dir_override: None,
    }
}

/// Build the `TokenSource` for the configured provider.
///
/// - `Provider::Anthropic`: a static source seeded from `cfg.api_key`. It's
///   never read for Anthropic requests (those go through `post_anthropic` with
///   `x-api-key`), but Llm holds one to keep the field non-`Option`.
/// - `Provider::OpenAi`: a static source over `OPENAI_COMPAT_API_KEY`.
/// - `Provider::Databricks`: if `DATABRICKS_TOKEN` is set, a static source.
///   Otherwise a `PkceOAuthTokenSource` pointed at the workspace's OIDC
///   discovery URL. First request without a cached token triggers a browser
///   flow; subsequent requests use the cache + refresh transparently.
pub(crate) fn build_token_source(cfg: &Config) -> Result<Arc<dyn TokenSource>, AgentError> {
    match cfg.provider {
        Provider::Anthropic | Provider::OpenAi | Provider::OpenRouter => {
            Ok(Arc::new(StaticTokenSource::new(cfg.api_key.clone())))
        }
        Provider::Databricks | Provider::DatabricksV2 => {
            if !cfg.api_key.is_empty() {
                return Ok(Arc::new(StaticTokenSource::new(cfg.api_key.clone())));
            }
            Ok(PkceOAuthTokenSource::new(databricks_pkce_config(
                &cfg.base_url,
            ))?)
        }
    }
}

/// Build the request body for `Llm::summarize` on `Provider::OpenRouter`.
/// Extracted so tests can assert on the actual wire shape instead of a
/// hand-rolled literal — summaries never carry `reasoning` (see
/// `apply_openrouter_mutations`, which the summary path never calls).
/// It spells the token limit `max_tokens` directly for the same reason: the
/// mutation that renames it is never applied here.
fn openrouter_summary_body(
    effective_model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_output_tokens: u32,
) -> Value {
    json!({
        "model": effective_model,
        "stream": false,
        "max_tokens": max_output_tokens,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt },
        ],
    })
}

/// Return a clone of `body` with any top-level `"model"` field removed.
/// Used for Databricks model-serving, which encodes the model in the URL
/// path and rejects the field in the body.
fn strip_model(body: &Value) -> Value {
    match body {
        Value::Object(map) => {
            let mut m = map.clone();
            m.remove("model");
            Value::Object(m)
        }
        other => other.clone(),
    }
}

#[derive(Debug)]
enum OpenRouterErrorClass {
    Retryable(Option<std::time::Duration>),
    Unknown,
}

/// Ceiling applied to the server-supplied `Retry-After` header before we
/// sleep on it. OpenRouter can advertise waits up to an hour, but
/// `openrouter_post`'s per-attempt sleep happens *outside*
/// `Client::timeout` (`cfg.llm_timeout`, default 240s) — an unclamped hint
/// could keep a single turn alive for up to two full-duration sleeps across
/// `MAX_RETRIES`. Clamping (never rejecting) keeps us honoring the server's
/// backoff signal while bounding worst-case turn latency to a value smaller
/// than the connect/response timeout.
const RETRY_AFTER_CAP_SECS: u64 = 60;

fn parse_retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<std::time::Duration> {
    let val = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: u64 = val.trim().parse().ok()?;
    (secs > 0).then(|| std::time::Duration::from_secs(secs.min(RETRY_AFTER_CAP_SECS)))
}

fn classify_openrouter_error(
    status: u16,
    body: &str,
    header_retry_after: Option<std::time::Duration>,
) -> OpenRouterErrorClass {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let error_type = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("metadata"))
        .and_then(|m| m.get("error_type"))
        .and_then(Value::as_str);
    // OpenRouter's documented retry hint is the HTTP `Retry-After` header
    // (see https://openrouter.ai/docs/api_reference/errors-and-debugging);
    // no current doc specifies a body-level retry field, so we don't parse one.
    let retry_after = header_retry_after;

    match (status, error_type) {
        (429, _) => OpenRouterErrorClass::Retryable(retry_after),
        (502, _) => OpenRouterErrorClass::Retryable(None),
        (503, Some("provider_overloaded")) => OpenRouterErrorClass::Retryable(retry_after),
        _ => OpenRouterErrorClass::Unknown,
    }
}

/// The one place the parameter-routing failure is worded. OpenRouter reports it
/// two ways — a 404 `No endpoints found ...` (what a `require_parameters`-style
/// or unsupported-parameter body actually returns) and an untyped 503 — and both
/// mean the same thing to the user: the model id is fine, the request shape is
/// not serveable by any endpoint behind it.
fn openrouter_parameter_routing_error(error_body: &str) -> AgentError {
    AgentError::Llm(format!(
        "no OpenRouter endpoint supports the requested parameters — \
         check model, effort, and tool requirements: {error_body}"
    ))
}

async fn openrouter_post(
    http: &Client,
    url: &str,
    body: &Value,
    bearer: &str,
) -> Result<Value, AgentError> {
    let body_bytes =
        serde_json::to_vec(body).map_err(|e| AgentError::Llm(format!("serialize: {e}")))?;
    let call_start = std::time::Instant::now();
    for attempt in 0..MAX_RETRIES {
        let resp = match http
            .post(url)
            .header("content-type", "application/json")
            .header("HTTP-Referer", "https://github.com/block/buzz")
            .header("X-OpenRouter-Title", "Buzz")
            .bearer_auth(bearer)
            .body(body_bytes.clone())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if attempt + 1 < MAX_RETRIES && is_retryable_transport_error(&e) {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = MAX_RETRIES,
                        error = %e,
                        "llm: openrouter transport error, retrying"
                    );
                    backoff_with_jitter(attempt).await;
                    continue;
                }
                return Err(terminal_llm_error(
                    call_start.elapsed(),
                    attempt + 1,
                    &format!("transport: {e}"),
                ));
            }
        };
        let status = resp.status();
        // Unlike the generic `post` path, OpenRouter's static-key auth makes
        // 401 and 403 distinguishable: 401 is an invalid/expired key (worth
        // one refresh-and-retry), while OpenRouter documents 403 as a
        // guardrail/moderation/permission rejection the same key will always
        // reproduce. Refreshing a static key returns the identical key, so
        // classifying 403 as `LlmAuth` would just waste a duplicate request
        // and surface Desktop's unrelated "access denied" copy.
        if status == 401 {
            return Err(AgentError::LlmAuth(read_error_body(resp).await));
        }
        if status == 403 {
            return Err(AgentError::Llm(format!(
                "{status}: {}",
                read_error_body(resp).await
            )));
        }
        if status == 402 {
            return Err(AgentError::Llm(
                "OpenRouter credits exhausted — check https://openrouter.ai/credits".into(),
            ));
        }
        if status == 404 {
            // OpenRouter overloads 404: a genuinely unknown/unavailable model id
            // and a valid model whose parameter set no endpoint can serve both
            // land here. Discriminate on the full parameter-routing phrase rather
            // than a prefix — "No endpoints found for <model>"-style bodies are
            // about the model, and reporting a parameter problem as
            // `LlmModelNotFound` (or vice versa) sends the user to the wrong fix.
            let error_body = read_error_body(resp).await;
            if is_unsupported_image_input_error(&error_body) {
                return Err(AgentError::UnsupportedImageInput(error_body));
            }
            if error_body.contains("No endpoints found that can handle the requested parameters") {
                return Err(openrouter_parameter_routing_error(&error_body));
            }
            return Err(AgentError::LlmModelNotFound(format!(
                "{status}: {error_body}"
            )));
        }
        // A6: status+error_type retry matrix
        // 499 (Client Closed Request) is included: OpenRouter may emit it when a
        // turn times out mid-stream. Will added 499 to the shared `post()` path in
        // #2175 for the same reason; OpenRouter must match to avoid silently opting
        // out of that stall-surfacing recovery.
        if status.is_server_error() || status == 429 || status.as_u16() == 499 {
            let header_retry_after = parse_retry_after_header(resp.headers());
            let error_body = read_error_body(resp).await;
            let should_retry = if attempt + 1 < MAX_RETRIES {
                match classify_openrouter_error(status.as_u16(), &error_body, header_retry_after) {
                    OpenRouterErrorClass::Retryable(delay) => {
                        if let Some(d) = delay {
                            tokio::time::sleep(d).await;
                        } else {
                            backoff_with_jitter(attempt).await;
                        }
                        true
                    }
                    OpenRouterErrorClass::Unknown => {
                        backoff_with_jitter(attempt).await;
                        true
                    }
                }
            } else {
                false
            };
            if should_retry {
                continue;
            }
            // Terminal: classify for the user
            return if status == 429 {
                Err(terminal_llm_error(
                    call_start.elapsed(),
                    attempt + 1,
                    &format!("rate limited: {error_body}"),
                ))
            } else {
                let parsed: Option<Value> = serde_json::from_str(&error_body).ok();
                let has_error_type = parsed
                    .as_ref()
                    .and_then(|v| v.get("error"))
                    .and_then(|e| e.get("metadata"))
                    .and_then(|m| m.get("error_type"))
                    .and_then(Value::as_str)
                    .is_some();
                Err(if !has_error_type && status.as_u16() == 503 {
                    openrouter_parameter_routing_error(&error_body)
                } else {
                    terminal_llm_error(
                        call_start.elapsed(),
                        attempt + 1,
                        &format!("exhausted retries: {status}: {error_body}"),
                    )
                })
            };
        }
        if !status.is_success() {
            let body = read_error_body(resp).await;
            // Same recovery classification as the shared `post()` terminal:
            // `openrouter_post` is a separate implementation with its own retry
            // loop and status ladder, so it needs its own arm or OpenRouter
            // agents keep the permanent context-400 stuck loop.
            if status == 400 && is_context_length_error(&body) {
                return Err(AgentError::LlmContextExceeded(format!("{status}: {body}")));
            }
            return Err(AgentError::Llm(format!("{status}: {body}")));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_LLM_RESPONSE_BYTES {
                return Err(AgentError::Llm(format!(
                    "response too large: {len} > {MAX_LLM_RESPONSE_BYTES}"
                )));
            }
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp;
        loop {
            match stream.chunk().await {
                Ok(Some(chunk)) => {
                    if buf.len() + chunk.len() > MAX_LLM_RESPONSE_BYTES {
                        return Err(AgentError::Llm(format!(
                            "response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                        )));
                    }
                    buf.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(terminal_llm_error(
                        call_start.elapsed(),
                        attempt + 1,
                        &format!("body read: {e}"),
                    ))
                }
            }
        }
        return serde_json::from_slice(&buf).map_err(|e| AgentError::Llm(format!("json: {e}")));
    }
    Err(terminal_llm_error(
        call_start.elapsed(),
        MAX_RETRIES,
        "exhausted retries",
    ))
}

fn apply_openrouter_mutations(
    body: &mut Value,
    effort: Option<ThinkingEffort>,
    effective_model: &str,
    prompt_caching: bool,
) {
    if let Some(obj) = body.as_object_mut() {
        // OpenRouter's Chat Completions API spells the output cap `max_tokens`;
        // `max_completion_tokens` is OpenAI-native and only 53 of 274
        // tools-capable OpenRouter models advertise it. Sending the OpenAI
        // spelling risks the cap being dropped on the floor by the endpoint we
        // route to, so translate it here rather than at the shared `openai_body`
        // (which OpenAI and Databricks also use, where the OpenAI spelling is
        // correct).
        if let Some(max_tokens) = obj.remove("max_completion_tokens") {
            obj.insert("max_tokens".into(), max_tokens);
        }

        // A2/A3: Add OpenRouter reasoning object when effort is configured.
        // Deliberately NOT paired with `provider.require_parameters`: that filter
        // routes only to endpoints advertising every parameter in the body, and
        // 83 of 274 tools-capable models do not advertise `reasoning`, so it turns
        // an opt-in effort setting into a hard 404 ("No endpoints found that can
        // handle the requested parameters") on a model id that is perfectly
        // valid. OpenRouter already best-effort routes on `tools`/`max_tokens`
        // without the filter, so we accept a request served without reasoning
        // over a request that cannot be served at all.
        if let Some(e) = effort {
            obj.insert(
                "reasoning".into(),
                json!({ "effort": e.openai_effort_str() }),
            );
        }

        // A7: Anthropic cache_control injection for anthropic/* models. Gated on
        // `prompt_caching` (`BUZZ_AGENT_PROMPT_CACHING`) for the same reason as
        // the native Anthropic route: these are Anthropic-dialect breakpoints on
        // an Anthropic model, so the documented kill switch must reach them too.
        if prompt_caching && effective_model.starts_with("anthropic/") {
            apply_anthropic_cache_control(obj);
        }
    }
}

fn apply_anthropic_cache_control(body: &mut serde_json::Map<String, Value>) {
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        // Cache the system message
        if let Some(system_msg) = messages
            .iter_mut()
            .find(|m| m.get("role").and_then(Value::as_str) == Some("system"))
        {
            if let Some(content) = system_msg.get("content").and_then(Value::as_str) {
                let content_str = content.to_string();
                if let Some(obj) = system_msg.as_object_mut() {
                    obj.insert(
                        "content".into(),
                        json!([{
                            "type": "text",
                            "text": content_str,
                            "cache_control": { "type": "ephemeral" }
                        }]),
                    );
                }
            }
        }

        // Cache last 2 user messages (skip image-only ones — A7 mixed-content regression)
        let mut user_count = 0;
        for msg in messages.iter_mut().rev() {
            if msg.get("role").and_then(Value::as_str) != Some("user") {
                continue;
            }
            // Only cache string content (plain text user messages), not array content
            // (image batches from tool results). This prevents corrupting image-only
            // user messages by converting them to text cache breakpoints.
            if let Some(content) = msg.get("content").and_then(Value::as_str) {
                let content_str = content.to_string();
                if let Some(obj) = msg.as_object_mut() {
                    obj.insert(
                        "content".into(),
                        json!([{
                            "type": "text",
                            "text": content_str,
                            "cache_control": { "type": "ephemeral" }
                        }]),
                    );
                }
                user_count += 1;
            }
            if user_count >= 2 {
                break;
            }
        }
    }
    // Cache the last tool definition
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        if let Some(last_tool) = tools.last_mut() {
            if let Some(function) = last_tool.get_mut("function").and_then(Value::as_object_mut) {
                function.insert("cache_control".into(), json!({ "type": "ephemeral" }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, HookServers, OpenAiApi, Provider};
    use crate::types::{HistoryItem, ToolCall, ToolResult, ToolResultContent};
    use std::collections::VecDeque;
    use std::time::Duration;
    use tokio::sync::Mutex;
    use tracing_subscriber::layer::SubscriberExt;

    fn cfg(provider: Provider) -> Config {
        Config {
            provider,
            system_prompt: "system".into(),
            max_rounds: 10,
            max_output_tokens: 1024,
            llm_timeout: Duration::from_secs(10),
            tool_timeout: Duration::from_secs(10),
            mcp_init_timeout: Duration::from_secs(10),
            mcp_max_restart_attempts: 1,
            mcp_restart_base_ms: 1,
            mcp_restart_max_ms: 1,
            max_sessions: 1,
            max_line_bytes: 1024 * 1024,
            max_history_bytes: 16 * 1024 * 1024,
            max_tool_result_text_bytes: 50 * 1024,
            max_context_tokens: 200_000,
            max_handoffs: 1,
            max_parallel_tools: 1,
            hook_timeout: Duration::from_secs(1),
            stop_max_rejections: 0,
            require_reply: false,
            hook_servers: HookServers::None,
            api_key: "key".into(),
            model: "model".into(),
            base_url: "http://example.invalid".into(),
            anthropic_api_version: "2023-06-01".into(),
            openai_api: OpenAiApi::Chat,
            prefer_mesh_for_auto: false,
            hints_enabled: true,
            thinking_effort: None,
            prompt_caching: true,
        }
    }

    #[derive(Debug, Clone)]
    struct CapturedHttpRequest {
        method: String,
        path: String,
        body: Option<Value>,
    }

    #[derive(Debug)]
    struct StubHttpResponse {
        status: u16,
        body: Value,
    }

    impl StubHttpResponse {
        fn ok(body: Value) -> Self {
            Self { status: 200, body }
        }

        fn error(status: u16, message: &str) -> Self {
            Self {
                status,
                body: json!({
                    "error": {
                        "message": message,
                        "type": "server_error",
                        "code": "service_unavailable",
                    }
                }),
            }
        }

        fn moa_failure(status: u16, code: &str, message: &str) -> Self {
            Self {
                status,
                body: json!({
                    "choices": [{
                        "finish_reason": "error",
                        "message": { "content": message, "role": "assistant" },
                    }],
                    "error": {
                        "message": message,
                        "type": "moa_failure",
                        "code": code,
                    },
                    "model": "mesh",
                }),
            }
        }
    }

    async fn spawn_sequence_stub(
        responses: Vec<StubHttpResponse>,
    ) -> (String, Arc<Mutex<Vec<CapturedHttpRequest>>>) {
        use std::collections::VecDeque;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_for_server = captured.clone();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));

        tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(connection) => connection,
                    Err(_) => return,
                };
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 4096];
                let header_end = loop {
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                    }
                };
                let header_text = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
                let content_length = header_text
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + content_length {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                    }
                }
                let mut request_line = header_text.lines().next().unwrap_or_default().split(' ');
                let method = request_line.next().unwrap_or_default().to_string();
                let path = request_line.next().unwrap_or_default().to_string();
                let body = if content_length == 0 {
                    None
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + content_length]).ok()
                };
                captured_for_server
                    .lock()
                    .await
                    .push(CapturedHttpRequest { method, path, body });

                let response = responses.lock().await.pop_front().unwrap_or_else(|| {
                    StubHttpResponse::error(500, "stub response sequence exhausted")
                });
                let status_text = match response.status {
                    200 => "OK",
                    400 => "Bad Request",
                    413 => "Payload Too Large",
                    500 => "Internal Server Error",
                    502 => "Bad Gateway",
                    503 => "Service Unavailable",
                    other => panic!("unsupported stub status: {other}"),
                };
                let body = response.body.to_string();
                let wire = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status,
                    status_text,
                    body.len(),
                    body,
                );
                let _ = socket.write_all(wire.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (base_url, captured)
    }

    fn model_catalog(ids: &[&str]) -> Value {
        json!({
            "object": "list",
            "data": ids
                .iter()
                .map(|id| json!({ "id": id, "object": "model" }))
                .collect::<Vec<_>>(),
        })
    }

    fn chat_response(text: &str) -> Value {
        json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "content": text },
            }]
        })
    }

    async fn complete_model(
        llm: &Llm,
        cfg: &Config,
        model: &str,
    ) -> Result<LlmResponse, AgentError> {
        llm.complete(
            cfg,
            "system",
            &[HistoryItem::User("hello".into())],
            &[],
            model,
        )
        .await
    }

    async fn complete_model_with_tool(
        llm: &Llm,
        cfg: &Config,
        model: &str,
    ) -> Result<LlmResponse, AgentError> {
        llm.complete(
            cfg,
            "system",
            &[HistoryItem::User("add two numbers".into())],
            &[ToolDef {
                name: "add_numbers".into(),
                description: "Add two numbers".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "a": {"type": "number"},
                        "b": {"type": "number"}
                    },
                    "required": ["a", "b"]
                }),
            }],
            model,
        )
        .await
    }

    async fn expire_mesh_catalog_check(llm: &Llm) {
        llm.mesh_auto_state.lock().await.last_checked =
            Some(Instant::now() - MESH_AUTO_CATALOG_TTL);
    }

    async fn expire_mesh_auto_cooldown(llm: &Llm) {
        let mut state = llm.mesh_auto_state.lock().await;
        state.last_checked = Some(Instant::now() - MESH_AUTO_CATALOG_TTL);
        state.cooldown_until = Some(Instant::now() - std::time::Duration::from_secs(1));
    }

    fn posted_models(requests: &[CapturedHttpRequest]) -> Vec<&str> {
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .filter_map(|request| request.body.as_ref()?.get("model")?.as_str())
            .collect()
    }

    #[tokio::test]
    async fn mesh_auto_requires_two_stable_catalog_observations() {
        let (base_url, captured) = spawn_sequence_stub(vec![
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("direct")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("collective")),
        ])
        .await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        assert_eq!(
            complete_model(&llm, &config, "auto").await.unwrap().text,
            "direct"
        );
        expire_mesh_catalog_check(&llm).await;
        assert_eq!(
            complete_model(&llm, &config, "auto").await.unwrap().text,
            "collective"
        );

        let requests = captured.lock().await;
        assert_eq!(posted_models(&requests), vec!["auto", "mesh"]);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/v1/models")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn mesh_auto_does_not_enable_while_second_model_flaps() {
        let (base_url, captured) = spawn_sequence_stub(vec![
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("one")),
            StubHttpResponse::ok(model_catalog(&["model-a"])),
            StubHttpResponse::ok(chat_response("two")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("three")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("four")),
        ])
        .await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        for _ in 0..4 {
            complete_model(&llm, &config, "auto").await.unwrap();
            expire_mesh_catalog_check(&llm).await;
        }

        let requests = captured.lock().await;
        assert_eq!(
            posted_models(&requests),
            vec!["auto", "auto", "auto", "mesh"]
        );
    }

    #[test]
    fn collective_catalog_requires_two_distinct_physical_models() {
        assert_eq!(mesh_catalog_supports_collective(&json!({})), None);
        assert_eq!(
            mesh_catalog_supports_collective(&model_catalog(&[])),
            Some(false)
        );
        assert_eq!(
            mesh_catalog_supports_collective(&model_catalog(&["model-a", "mesh"])),
            Some(false)
        );
        assert_eq!(
            mesh_catalog_supports_collective(&model_catalog(&[
                "org/model@main:Q4",
                "org/model:Q4",
                "mesh"
            ])),
            Some(false),
            "two spellings of one model are not collective capacity"
        );
        assert_eq!(
            mesh_catalog_supports_collective(&model_catalog(&["model-a", "model-b", "mesh"])),
            Some(true)
        );
    }

    #[tokio::test]
    async fn mesh_auto_tracks_models_appearing_disappearing_and_reappearing() {
        let (base_url, captured) = spawn_sequence_stub(vec![
            StubHttpResponse::ok(model_catalog(&[])),
            StubHttpResponse::ok(chat_response("zero")),
            StubHttpResponse::ok(model_catalog(&["model-a", "mesh"])),
            StubHttpResponse::ok(chat_response("one")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("two-first-observation")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("two-stable")),
            StubHttpResponse::ok(model_catalog(&["model-a", "mesh"])),
            StubHttpResponse::ok(chat_response("contracted")),
            StubHttpResponse::ok(model_catalog(&[])),
            StubHttpResponse::ok(chat_response("empty-again")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("rejoin-first-observation")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("rejoined-stable")),
        ])
        .await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        for call in 0..8 {
            if call == 5 {
                expire_mesh_auto_cooldown(&llm).await;
            } else if call > 0 {
                expire_mesh_catalog_check(&llm).await;
            }
            complete_model(&llm, &config, "auto").await.unwrap();
        }

        let requests = captured.lock().await;
        assert_eq!(
            posted_models(&requests),
            vec!["auto", "auto", "auto", "mesh", "auto", "auto", "auto", "mesh"]
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/v1/models")
                .count(),
            8
        );
    }

    #[tokio::test]
    async fn mesh_auto_falls_back_once_and_cools_down_when_mesh_contracts() {
        let (base_url, captured) = spawn_sequence_stub(vec![
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("warmup")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::error(503, MESH_MOA_UNAVAILABLE_MESSAGE),
            StubHttpResponse::ok(chat_response("fallback")),
            StubHttpResponse::ok(chat_response("cooldown")),
        ])
        .await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        complete_model(&llm, &config, "auto").await.unwrap();
        expire_mesh_catalog_check(&llm).await;
        assert_eq!(
            complete_model(&llm, &config, "auto").await.unwrap().text,
            "fallback"
        );
        assert_eq!(
            complete_model(&llm, &config, "auto").await.unwrap().text,
            "cooldown"
        );

        let requests = captured.lock().await;
        assert_eq!(
            posted_models(&requests),
            vec!["auto", "mesh", "auto", "auto"]
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/v1/models")
                .count(),
            2,
            "cooldown request must not re-probe the catalog"
        );
    }

    #[tokio::test]
    async fn mesh_auto_falls_back_once_when_moa_reducer_fails() {
        let (base_url, captured) = spawn_sequence_stub(vec![
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("warmup")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::moa_failure(
                502,
                "all_reducers_failed",
                "Reducer failed: unsupported chat template",
            ),
            StubHttpResponse::ok(chat_response("fallback")),
            StubHttpResponse::ok(chat_response("cooldown")),
        ])
        .await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        complete_model(&llm, &config, "auto").await.unwrap();
        expire_mesh_catalog_check(&llm).await;
        assert_eq!(
            complete_model(&llm, &config, "auto").await.unwrap().text,
            "fallback"
        );
        assert_eq!(
            complete_model(&llm, &config, "auto").await.unwrap().text,
            "cooldown"
        );

        let requests = captured.lock().await;
        assert_eq!(
            posted_models(&requests),
            vec!["auto", "mesh", "auto", "auto"]
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/v1/models")
                .count(),
            2,
            "mesh-specific failure must enter cooldown without re-probing"
        );
    }

    #[tokio::test]
    async fn mesh_auto_retries_unstructured_tool_markup_through_auto() {
        let (base_url, captured) = spawn_sequence_stub(vec![
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response("warmup")),
            StubHttpResponse::ok(model_catalog(&["model-a", "model-b", "mesh"])),
            StubHttpResponse::ok(chat_response(
                "<|tool_call>call:add_numbers{a:17,b:25}<tool_call|>",
            )),
            StubHttpResponse::ok(chat_response("safe fallback")),
            StubHttpResponse::ok(chat_response("cooldown")),
        ])
        .await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        complete_model(&llm, &config, "auto").await.unwrap();
        expire_mesh_catalog_check(&llm).await;
        assert_eq!(
            complete_model_with_tool(&llm, &config, "auto")
                .await
                .unwrap()
                .text,
            "safe fallback"
        );
        assert_eq!(
            complete_model_with_tool(&llm, &config, "auto")
                .await
                .unwrap()
                .text,
            "cooldown"
        );

        let requests = captured.lock().await;
        assert_eq!(
            posted_models(&requests),
            vec!["auto", "mesh", "auto", "auto"]
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/v1/models")
                .count(),
            2,
            "pseudo tool markup must enter cooldown without another catalog probe"
        );
    }

    #[tokio::test]
    async fn mesh_auto_catalog_failure_fails_open_to_auto() {
        let (base_url, captured) = spawn_sequence_stub(vec![
            StubHttpResponse::error(500, "catalog unavailable"),
            StubHttpResponse::ok(chat_response("direct")),
        ])
        .await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        complete_model(&llm, &config, "auto").await.unwrap();
        let requests = captured.lock().await;
        assert_eq!(posted_models(&requests), vec!["auto"]);
    }

    #[tokio::test]
    async fn generic_openai_auto_does_not_probe_or_select_mesh() {
        let (base_url, captured) =
            spawn_sequence_stub(vec![StubHttpResponse::ok(chat_response("provider auto"))]).await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = false;
        let llm = Llm::new(&config).unwrap();

        complete_model(&llm, &config, "auto").await.unwrap();
        let requests = captured.lock().await;
        assert_eq!(posted_models(&requests), vec!["auto"]);
        assert!(requests.iter().all(|request| request.path != "/v1/models"));
    }

    #[tokio::test]
    async fn explicit_models_are_never_rewritten_or_fallback_retried() {
        let (base_url, captured) = spawn_sequence_stub(vec![StubHttpResponse::error(
            503,
            MESH_MOA_UNAVAILABLE_MESSAGE,
        )])
        .await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        let error = complete_model(&llm, &config, "mesh").await.unwrap_err();
        assert!(error.to_string().contains(MESH_MOA_UNAVAILABLE_MESSAGE));
        let requests = captured.lock().await;
        assert_eq!(posted_models(&requests), vec!["mesh"]);
        assert!(requests.iter().all(|request| request.path != "/v1/models"));
    }

    #[tokio::test]
    async fn explicit_real_model_bypasses_mesh_auto_policy() {
        let (base_url, captured) =
            spawn_sequence_stub(vec![StubHttpResponse::ok(chat_response("explicit"))]).await;
        let mut config = cfg(Provider::OpenAi);
        config.base_url = base_url;
        config.prefer_mesh_for_auto = true;
        let llm = Llm::new(&config).unwrap();

        let response = complete_model(&llm, &config, "model-a").await.unwrap();
        assert_eq!(response.text, "explicit");
        let requests = captured.lock().await;
        assert_eq!(posted_models(&requests), vec!["model-a"]);
        assert!(requests.iter().all(|request| request.path != "/v1/models"));
    }

    #[test]
    fn mesh_unavailable_classifier_requires_exact_openai_error_message() {
        assert!(is_mesh_moa_unavailable_body(
            &StubHttpResponse::error(503, MESH_MOA_UNAVAILABLE_MESSAGE)
                .body
                .to_string()
        ));
        assert!(!is_mesh_moa_unavailable_body(
            &StubHttpResponse::error(503, "some other outage")
                .body
                .to_string()
        ));
    }

    #[test]
    fn mesh_failure_classifier_requires_structured_moa_failure_type() {
        assert!(is_mesh_moa_failure_body(
            &StubHttpResponse::moa_failure(
                502,
                "all_reducers_failed",
                "Reducer failed: unsupported chat template",
            )
            .body
            .to_string()
        ));
        assert!(!is_mesh_moa_failure_body(
            &StubHttpResponse::error(502, "some other bad gateway")
                .body
                .to_string()
        ));
    }

    fn image_history() -> Vec<HistoryItem> {
        vec![
            HistoryItem::User("describe the image".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    provider_id: "toolu_1".into(),
                    name: "dev__view_image".into(),
                    arguments: serde_json::json!({"source":"x.png"}),
                    provider_extra: Default::default(),
                }],
                reasoning_details: None,
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "toolu_1".into(),
                content: vec![
                    ToolResultContent::Text("10×10, 70 B (image/png from x.png)".into()),
                    ToolResultContent::Image {
                        data: "aW1n".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
            }),
        ]
    }

    #[test]
    fn anthropic_tool_result_preserves_image_block() {
        let body = anthropic_body(
            &cfg(Provider::Anthropic),
            "system",
            &image_history(),
            &[],
            "model",
            None,
        );
        let content = &body["messages"][2]["content"][0]["content"];
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "aW1n");
    }

    fn cfg_responses() -> Config {
        let mut c = cfg(Provider::OpenAi);
        c.openai_api = OpenAiApi::Responses;
        c
    }

    fn tool_call_history() -> Vec<HistoryItem> {
        vec![
            HistoryItem::User("call the tool".into()),
            HistoryItem::Assistant {
                text: "ok, calling".into(),
                tool_calls: vec![ToolCall {
                    provider_id: "call_abc".into(),
                    name: "dev__shell".into(),
                    arguments: serde_json::json!({"command": "ls"}),
                    provider_extra: Default::default(),
                }],
                reasoning_details: None,
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "call_abc".into(),
                content: vec![ToolResultContent::Text("file.txt".into())],
                is_error: false,
            }),
        ]
    }

    #[test]
    fn responses_body_top_level_shape() {
        let tools = vec![ToolDef {
            name: "dev__shell".into(),
            description: "run a shell command".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
            }),
        }];
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &tools,
            "model",
            None,
        );
        assert_eq!(body["model"], "model");
        assert_eq!(body["instructions"], "system");
        assert_eq!(body["max_output_tokens"], 1024);
        assert!(
            body.get("messages").is_none(),
            "must use `input`, not `messages`"
        );
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_completion_tokens").is_none());

        // Tools are flat — top-level type/name/description/parameters.
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["name"], "dev__shell");
        assert!(
            tool.get("function").is_none(),
            "Responses tool schema is flat"
        );
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn responses_body_replay_emits_function_call_before_output() {
        // Replay requirement from the live API: the assistant's prior
        // function_call item *must* appear in `input[]` before its matching
        // function_call_output, otherwise the API rejects with
        // "No tool call found for call_id ...".
        let body = responses_body(
            &cfg_responses(),
            "system",
            &tool_call_history(),
            &[],
            "model",
            None,
        );
        let input = body["input"].as_array().unwrap();

        // [0] user, [1] assistant text, [2] function_call, [3] function_call_output
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "call the tool");

        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[1]["content"][0]["text"], "ok, calling");

        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_abc");
        assert_eq!(input[2]["name"], "dev__shell");
        // Arguments are a JSON-encoded string per spec.
        assert_eq!(input[2]["arguments"], "{\"command\":\"ls\"}");

        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_abc");
        assert_eq!(input[3]["output"], "file.txt");
    }

    #[test]
    fn responses_body_skips_empty_assistant_text() {
        // Mirrors the Chat Completions behavior (#559/#560): empty assistant
        // turns are skipped so we don't emit an empty `output_text` block,
        // but the tool_call(s) on that assistant turn still go through.
        let history = vec![
            HistoryItem::User("u".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    provider_id: "call_x".into(),
                    name: "t".into(),
                    arguments: serde_json::json!({}),
                    provider_extra: Default::default(),
                }],
                reasoning_details: None,
            },
        ];
        let body = responses_body(&cfg_responses(), "system", &history, &[], "model", None);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
    }

    #[test]
    fn responses_body_image_tool_result_attaches_input_image() {
        let body = responses_body(
            &cfg_responses(),
            "system",
            &image_history(),
            &[],
            "model",
            None,
        );
        let input = body["input"].as_array().unwrap();
        // function_call_output carries the text part; image rides on a
        // trailing user message as `input_image`.
        let fco = input
            .iter()
            .find(|i| i["type"] == "function_call_output")
            .unwrap();
        assert_eq!(fco["call_id"], "toolu_1");
        let img_msg = input.iter().rev().find(|i| i["role"] == "user").unwrap();
        assert_eq!(img_msg["content"][0]["type"], "input_image");
        assert_eq!(
            img_msg["content"][0]["image_url"],
            "data:image/png;base64,aW1n"
        );
    }

    #[test]
    fn parse_responses_completed_with_text_is_end_turn() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hello"}],
            }],
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.text, "hello");
        assert!(r.tool_calls.is_empty());
        assert_eq!(r.stop, ProviderStop::EndTurn);
    }

    #[test]
    fn parse_responses_completed_with_function_call_is_tool_use() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": []},
                {
                    "type": "function_call",
                    "call_id": "call_z",
                    "name": "dev__shell",
                    "arguments": "{\"command\":\"ls\"}",
                },
            ],
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.text, "");
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].provider_id, "call_z");
        assert_eq!(r.tool_calls[0].name, "dev__shell");
        assert_eq!(
            r.tool_calls[0].arguments,
            serde_json::json!({"command": "ls"})
        );
        assert_eq!(r.stop, ProviderStop::ToolUse);
    }

    #[test]
    fn parse_responses_incomplete_max_output_tokens() {
        let v = serde_json::json!({
            "status": "incomplete",
            "incomplete_details": {"reason": "max_output_tokens"},
            "output": [],
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.stop, ProviderStop::MaxTokens);
    }

    #[test]
    fn is_responses_required_error_matrix() {
        for (body, want) in [
            // Databricks GPT-5.5 (the actual case we observed).
            ("Function tools with reasoning_effort are not supported for gpt-5.5 in /v1/chat/completions. Please use /v1/responses instead.", true),
            // Forward-compat: OpenAI saying the same thing in prose.
            ("This model requires the Responses API. Please use the Responses API instead.", true),
            // Negatives — must NOT trigger on unrelated 4xx.
            ("{\"error\":\"invalid_api_key\"}", false),
            ("max_tokens is not supported with this model", false),
            ("", false),
        ] {
            assert_eq!(is_responses_required_error(body), want, "body={body:?}");
        }
    }

    #[test]
    fn databricks_v2_routes_by_model_family() {
        use DatabricksV2Route::{AnthropicMessages, MlflowChatCompletions, OpenAiResponses};
        for (model, route, path) in [
            // OpenAI-shaped: the gpt family plus the GPT-5 code names.
            (
                "databricks-gpt-5-5",
                OpenAiResponses,
                "/ai-gateway/openai/v1/responses",
            ),
            ("gpt-4o", OpenAiResponses, "/ai-gateway/openai/v1/responses"),
            // The intentional dashless `gpt5` spelling still routes to OpenAI.
            ("gpt5", OpenAiResponses, "/ai-gateway/openai/v1/responses"),
            (
                "databricks-gpt-5-6-luna",
                OpenAiResponses,
                "/ai-gateway/openai/v1/responses",
            ),
            (
                "databricks-gpt-5-6-sol",
                OpenAiResponses,
                "/ai-gateway/openai/v1/responses",
            ),
            (
                "databricks-terra",
                OpenAiResponses,
                "/ai-gateway/openai/v1/responses",
            ),
            // Anthropic-shaped: the claude prefix, the family names, and the
            // release code names — each must reach the cache-capable route even
            // when the endpoint name omits the literal "claude".
            (
                "databricks-claude-opus-4-7",
                AnthropicMessages,
                "/ai-gateway/anthropic/v1/messages",
            ),
            (
                "goose-opus-5",
                AnthropicMessages,
                "/ai-gateway/anthropic/v1/messages",
            ),
            (
                "databricks-sonnet-5",
                AnthropicMessages,
                "/ai-gateway/anthropic/v1/messages",
            ),
            (
                "databricks-haiku-4-5",
                AnthropicMessages,
                "/ai-gateway/anthropic/v1/messages",
            ),
            (
                "databricks-mythos-5",
                AnthropicMessages,
                "/ai-gateway/anthropic/v1/messages",
            ),
            (
                "databricks-fable-5",
                AnthropicMessages,
                "/ai-gateway/anthropic/v1/messages",
            ),
            // Case-insensitive.
            (
                "Databricks-Claude-Opus-5",
                AnthropicMessages,
                "/ai-gateway/anthropic/v1/messages",
            ),
            // Unrecognised names still fall through to the MLflow chat route.
            (
                "custom-tool-model",
                MlflowChatCompletions,
                "/ai-gateway/mlflow/v1/chat/completions",
            ),
            (
                "databricks-gemini-3-pro",
                MlflowChatCompletions,
                "/ai-gateway/mlflow/v1/chat/completions",
            ),
            // Collision guard: short code names must match only as whole
            // segments, never as substrings of an unrelated custom alias.
            // Each of these embeds a marker (`sol`, `terra`, `opus`) mid-word
            // and must stay on the MLflow fallback, not adopt a wire its
            // backend can't parse.
            (
                "consolidated-llama",
                MlflowChatCompletions,
                "/ai-gateway/mlflow/v1/chat/completions",
            ),
            (
                "terraform-coder",
                MlflowChatCompletions,
                "/ai-gateway/mlflow/v1/chat/completions",
            ),
            (
                "corpus-reranker",
                MlflowChatCompletions,
                "/ai-gateway/mlflow/v1/chat/completions",
            ),
            (
                "octopus-model",
                MlflowChatCompletions,
                "/ai-gateway/mlflow/v1/chat/completions",
            ),
        ] {
            let got = databricks_v2_route_for_model(model);
            assert_eq!(got, route, "model={model}");
            assert_eq!(databricks_v2_path(got), path, "model={model}");
        }
    }

    #[test]
    fn parse_responses_rejects_malformed_function_arguments() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "function_call",
                "call_id": "call_z",
                "name": "t",
                "arguments": "not json {",
            }],
        });
        assert!(matches!(parse_responses(v), Err(AgentError::Llm(_))));
    }

    #[test]
    fn openai_tool_result_adds_followup_image_user_message() {
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &image_history(),
            &[],
            "model",
            None,
        );
        assert_eq!(body["messages"][3]["role"], "tool");
        assert!(body["messages"][3]["content"]
            .as_str()
            .unwrap()
            .contains("provided in the next user message"));
        assert_eq!(body["messages"][4]["role"], "user");
        assert_eq!(body["messages"][4]["content"][0]["type"], "image_url");
        assert_eq!(
            body["messages"][4]["content"][0]["image_url"]["url"],
            "data:image/png;base64,aW1n"
        );
    }

    /// Regression for Databricks model serving (and any OpenAI-Chat frontend
    /// that translates to Anthropic on the way to the model). Parallel tool
    /// calls where one or more return images previously produced an
    /// interleaved sequence:
    ///   role:"tool"  (A)
    ///   role:"user"  (image A)
    ///   role:"tool"  (B)
    ///   role:"user"  (image B)
    /// The intervening user message split the run of tool results, so the
    /// translator could no longer fold them into a single Anthropic
    /// `tool_result`-bearing user message — Anthropic then rejected the
    /// request with "tool_use ids were found without tool_result blocks
    /// immediately after". Fix: every `role:"tool"` for a run of adjacent
    /// ToolResults emits contiguously, then a single trailing user message
    /// carries all of the images from the batch.
    #[test]
    fn openai_parallel_image_tool_results_stay_contiguous() {
        let history = vec![
            HistoryItem::User("describe both images".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![
                    ToolCall {
                        provider_id: "toolu_a".into(),
                        name: "dev__view_image".into(),
                        arguments: serde_json::json!({"source": "a.png"}),
                        provider_extra: Default::default(),
                    },
                    ToolCall {
                        provider_id: "toolu_b".into(),
                        name: "dev__view_image".into(),
                        arguments: serde_json::json!({"source": "b.png"}),
                        provider_extra: Default::default(),
                    },
                ],
                reasoning_details: None,
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "toolu_a".into(),
                content: vec![
                    ToolResultContent::Text("10×10, 70 B (image/png from a.png)".into()),
                    ToolResultContent::Image {
                        data: "aaa".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
            }),
            HistoryItem::ToolResult(ToolResult {
                provider_id: "toolu_b".into(),
                content: vec![
                    ToolResultContent::Text("10×10, 70 B (image/png from b.png)".into()),
                    ToolResultContent::Image {
                        data: "bbb".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
            }),
        ];
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &history,
            &[],
            "model",
            None,
        );
        let messages = body["messages"].as_array().unwrap();
        // [0] system, [1] user, [2] assistant(tool_calls), [3] tool A, [4] tool B, [5] user(images)
        assert_eq!(messages.len(), 6, "messages: {messages:#?}");
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "toolu_a");
        assert_eq!(
            messages[4]["role"], "tool",
            "tool results must stay adjacent; intervening user message breaks Databricks/Anthropic pairing"
        );
        assert_eq!(messages[4]["tool_call_id"], "toolu_b");
        assert_eq!(messages[5]["role"], "user");
        let imgs = messages[5]["content"].as_array().unwrap();
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0]["image_url"]["url"], "data:image/png;base64,aaa");
        assert_eq!(imgs[1]["image_url"]["url"], "data:image/png;base64,bbb");
    }

    // ---- prompt caching (cache_control) body-shape tests ----

    #[test]
    fn anthropic_body_stamps_cache_control_when_enabled() {
        let body = anthropic_body(
            &cfg(Provider::DatabricksV2),
            "sys",
            &[
                HistoryItem::User("hello".into()),
                HistoryItem::Assistant {
                    text: "hi".into(),
                    tool_calls: vec![],
                    reasoning_details: None,
                },
                HistoryItem::User("more".into()),
            ],
            &[],
            "databricks-claude-opus-5",
            None,
        );
        // Static prefix: system promoted to a structured block carrying the marker.
        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], "sys");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        // Leapfrog: the last block of the last TWO messages is marked; earlier
        // ones are not. Three distinct user/assistant/user turns → messages[1]
        // and messages[2] marked, messages[0] clean.
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        let tail_block = |m: &Value| m["content"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(tail_block(&msgs[2])["cache_control"]["type"], "ephemeral");
        assert_eq!(tail_block(&msgs[1])["cache_control"]["type"], "ephemeral");
        assert!(
            tail_block(&msgs[0]).get("cache_control").is_none(),
            "only the last two messages carry a breakpoint"
        );
    }

    #[test]
    fn anthropic_body_single_message_stamps_only_one_breakpoint() {
        // With a single message there is no second turn to leapfrog to; the
        // checked_sub(2) index is skipped rather than panicking.
        let body = anthropic_body(
            &cfg(Provider::DatabricksV2),
            "sys",
            &[HistoryItem::User("hello".into())],
            &[],
            "databricks-claude-opus-5",
            None,
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0]["content"].as_array().unwrap().last().unwrap()["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn anthropic_body_no_cache_control_when_disabled() {
        let mut c = cfg(Provider::DatabricksV2);
        c.prompt_caching = false;
        let body = anthropic_body(
            &c,
            "sys",
            &[HistoryItem::User("hello".into())],
            &[],
            "databricks-claude-opus-5",
            None,
        );
        // system stays a bare string; no marker anywhere.
        assert_eq!(body["system"], "sys");
        let last_block = &body["messages"][0]["content"][0];
        assert!(last_block.get("cache_control").is_none());
    }

    #[test]
    fn anthropic_body_empty_system_stays_string_even_when_caching() {
        // An empty system prompt must not become an empty text block —
        // Anthropic rejects those. Caching is still applied to the tail.
        let body = anthropic_body(
            &cfg(Provider::DatabricksV2),
            "",
            &[HistoryItem::User("hello".into())],
            &[],
            "databricks-claude-opus-5",
            None,
        );
        assert_eq!(body["system"], "");
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    // ---- ThinkingEffort body-shape tests ----

    #[test]
    fn anthropic_body_omits_thinking_when_effort_none() {
        let body = anthropic_body(
            &cfg(Provider::Anthropic),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            None,
        );
        assert!(
            body.get("thinking").is_none(),
            "thinking must be absent when effort is None"
        );
    }

    #[test]
    fn anthropic_body_emits_thinking_when_effort_high() {
        // claude-3.x model → manual budget_tokens shape.
        // Use max_output_tokens = 4096 so budget fits: headroom = 4096 - 1024 = 3072.
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 4096;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-3-7-sonnet-20250219",
            Some(ThinkingEffort::High),
        );
        assert_eq!(body["thinking"]["type"], "enabled");
        // budget_tokens = min(32768, 4096-1024) = 3072
        assert_eq!(body["thinking"]["budget_tokens"], 3072);
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn anthropic_body_omits_thinking_when_max_output_too_small() {
        // max_output_tokens = 2047: headroom = 2047 - 1024 = 1023 < 1024 → omit thinking.
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 2047;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-3-7-sonnet-20250219",
            Some(ThinkingEffort::High),
        );
        assert!(
            body.get("thinking").is_none(),
            "thinking must be omitted when max_output_tokens leaves < 1024 for budget"
        );
    }

    #[test]
    fn anthropic_body_emits_thinking_at_boundary_2048() {
        // max_output_tokens = 2048: headroom = 2048 - 1024 = 1024 ≥ 1024 → emit.
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 2048;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-3-7-sonnet-20250219",
            Some(ThinkingEffort::High),
        );
        let t = body
            .get("thinking")
            .expect("thinking must be present at boundary 2048");
        assert_eq!(t["budget_tokens"], 1024); // min(32768, 2048-1024)
    }

    #[test]
    fn anthropic_body_emits_thinking_high_uncapped_when_budget_fits() {
        // When max_output_tokens is large enough, budget_tokens is not capped.
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 65_536;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-3-7-sonnet-20250219",
            Some(ThinkingEffort::High),
        );
        assert_eq!(body["thinking"]["budget_tokens"], 32_768);
    }

    #[test]
    fn anthropic_body_emits_thinking_low_budget() {
        // Low budget (1024 tokens) exactly fits when max_output_tokens = 2048.
        // headroom = 2048 - 1024 = 1024; min(1024, 1024) = 1024 ≥ 1024 → emit.
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 2048;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-3-7-sonnet-20250219",
            Some(ThinkingEffort::Low),
        );
        // Low budget (1024) fits exactly at the boundary — emitted without capping.
        assert_eq!(body["thinking"]["budget_tokens"], 1024);
    }

    #[test]
    fn anthropic_body_emits_adaptive_thinking_for_opus_4() {
        // Adaptive Claude (claude-opus-4-6/4.7/4.8) → thinking:{type:"adaptive"} + output_config.effort.
        // Note: Opus 4.5 is NOT adaptive — it uses manual budget.
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 32_768;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-opus-4-7",
            Some(ThinkingEffort::High),
        );
        assert_eq!(
            body["thinking"]["type"], "adaptive",
            "thinking must be {{type:adaptive}} for claude-opus-4-7"
        );
        assert_eq!(body["output_config"]["effort"], "high");
    }

    #[test]
    fn anthropic_body_emits_manual_budget_for_opus_4_5() {
        // Opus 4.5 uses manual budget (effort page: "uses manual thinking").
        // max_output_tokens = 32768; headroom = 32768 - 1024 = 31744; min(32768, 31744) = 31744.
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 32_768;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-opus-4-5",
            Some(ThinkingEffort::High),
        );
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 31_744); // min(32768, 32768-1024)
        assert!(
            body.get("output_config").is_none(),
            "output_config must be absent for claude-opus-4-5 (manual budget)"
        );
    }

    #[test]
    fn anthropic_body_omits_both_fields_for_unrecognized_model() {
        // Non-Anthropic models (gpt-5, llama, etc.) → omit both fields rather than guess.
        let body = anthropic_body(
            &cfg(Provider::Anthropic),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "gpt-4o",
            Some(ThinkingEffort::High),
        );
        assert!(body.get("thinking").is_none(), "thinking must be absent");
        assert!(
            body.get("output_config").is_none(),
            "output_config must be absent"
        );
    }

    #[test]
    fn openai_body_omits_reasoning_effort_when_none() {
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            None,
        );
        assert!(
            body.get("reasoning_effort").is_none(),
            "reasoning_effort must be absent when effort is None"
        );
    }

    #[test]
    fn openai_body_emits_reasoning_effort_medium() {
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            Some(ThinkingEffort::Medium),
        );
        assert_eq!(body["reasoning_effort"], "medium");
    }

    #[test]
    fn responses_body_omits_reasoning_when_effort_none() {
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            None,
        );
        assert!(
            body.get("reasoning").is_none(),
            "reasoning must be absent when effort is None"
        );
    }

    #[test]
    fn responses_body_emits_reasoning_effort_low() {
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            Some(ThinkingEffort::Low),
        );
        assert_eq!(body["reasoning"]["effort"], "low");
    }

    #[test]
    fn effective_model_overrides_cfg_model_in_anthropic_body() {
        let body = anthropic_body(
            &cfg(Provider::Anthropic),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "override-model",
            None,
        );
        assert_eq!(body["model"], "override-model");
    }

    #[test]
    fn effective_model_overrides_cfg_model_in_openai_body() {
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "override-model",
            None,
        );
        assert_eq!(body["model"], "override-model");
    }

    #[test]
    fn anthropic_body_opus_4_8_xhigh_emits_xhigh_effort() {
        // Body-shape regression: xhigh on Opus 4.8 must emit output_config.effort="xhigh".
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 32_768;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-opus-4-8",
            Some(ThinkingEffort::XHigh),
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "xhigh");
    }

    #[test]
    fn anthropic_body_opus_4_8_max_emits_max_effort() {
        // Body-shape regression: max on Opus 4.8 must emit output_config.effort="max".
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 32_768;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-opus-4-8",
            Some(ThinkingEffort::Max),
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "max");
    }

    #[test]
    fn openai_body_emits_xhigh_effort() {
        // xhigh is a valid OpenAI effort value — must pass through.
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            Some(ThinkingEffort::XHigh),
        );
        assert_eq!(body["reasoning_effort"], "xhigh");
    }

    #[test]
    fn openai_body_emits_none_effort() {
        // none is a valid OpenAI effort value.
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            Some(ThinkingEffort::None),
        );
        assert_eq!(body["reasoning_effort"], "none");
    }

    #[test]
    fn responses_body_emits_xhigh_effort() {
        // xhigh is a valid Responses API effort value.
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            Some(ThinkingEffort::XHigh),
        );
        assert_eq!(body["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn responses_body_emits_minimal_effort() {
        // minimal is a valid Responses API effort value.
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            Some(ThinkingEffort::Minimal),
        );
        assert_eq!(body["reasoning"]["effort"], "minimal");
    }

    // ---- DatabricksV2 route-aware effort normalization (body-level assertions) ----
    //
    // The DBv2 `complete()` dispatch applies `normalize_effort_for_openai_route` /
    // `normalize_effort_for_anthropic_route` before calling body builders. These tests
    // verify the body shape that results from the already-normalized effort values — i.e.,
    // they confirm the body builders correctly serialize the values the dispatch passes them.

    #[test]
    fn dbv2_openai_route_max_effort_clamped_to_xhigh_in_responses_body() {
        // DBv2 GPT-5.5 route: max → clamped to xhigh by normalize_effort_for_openai_route
        // before reaching responses_body. gpt-5.5 supports xhigh so the final value is xhigh.
        let clamped =
            crate::config::normalize_effort_for_openai_route(ThinkingEffort::Max, "gpt-5.5");
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "gpt-5.5",
            Some(clamped),
        );
        assert_eq!(
            body["reasoning"]["effort"], "xhigh",
            "DBv2 GPT-5.5 route: max must be clamped to xhigh before responses_body"
        );
    }

    #[test]
    fn dbv2_openai_route_max_effort_passes_through_for_gpt5_6() {
        let normalized =
            crate::config::normalize_effort_for_openai_route(ThinkingEffort::Max, "gpt-5.6-sol");
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "gpt-5.6-sol",
            Some(normalized),
        );
        assert_eq!(
            body["reasoning"]["effort"], "max",
            "DBv2 GPT-5.6 route must serialize max to the Responses API"
        );
    }

    #[test]
    fn dbv2_mlflow_route_max_effort_clamped_to_xhigh_in_openai_body() {
        // DBv2 MLflow route (unknown model): max → clamped to xhigh by normalize_effort_for_openai_route.
        // Unknown models pass through after the max→xhigh clamp.
        let clamped =
            crate::config::normalize_effort_for_openai_route(ThinkingEffort::Max, "llama-4");
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "llama-4",
            Some(clamped),
        );
        assert_eq!(
            body["reasoning_effort"], "xhigh",
            "DBv2 MLflow route: max must be clamped to xhigh before openai_body"
        );
    }

    #[test]
    fn dbv2_openai_route_none_minimal_pass_through_in_responses_body() {
        // Verify that supported values pass through for the respective model families.
        // gpt-5.5 supports none (but not minimal); gpt-5 base supports minimal (but not none).
        let none_normalized =
            crate::config::normalize_effort_for_openai_route(ThinkingEffort::None, "gpt-5.5");
        assert_eq!(
            none_normalized,
            ThinkingEffort::None,
            "OpenAI normalizer must not touch none for gpt-5.5"
        );
        let minimal_normalized =
            crate::config::normalize_effort_for_openai_route(ThinkingEffort::Minimal, "gpt-5");
        assert_eq!(
            minimal_normalized,
            ThinkingEffort::Minimal,
            "OpenAI normalizer must not touch minimal for gpt-5 base"
        );
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "gpt-5.5",
            Some(none_normalized),
        );
        assert_eq!(
            body["reasoning"]["effort"], "none",
            "DBv2 GPT-5.5 route: none must be emitted as-is"
        );
    }

    #[test]
    fn dbv2_claude_route_none_effort_omits_thinking_fields() {
        // DBv2 Claude route: none → normalize_effort_for_anthropic_route returns None → omit.
        let normalized = crate::config::normalize_effort_for_anthropic_route(ThinkingEffort::None);
        assert_eq!(
            normalized, None,
            "Anthropic normalizer must return None for ThinkingEffort::None"
        );
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 32_768;
        let body = anthropic_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "claude-opus-4-8",
            normalized, // None → omit thinking fields
        );
        assert!(
            body.get("thinking").is_none(),
            "DBv2 Claude route: none effort must omit thinking fields"
        );
        assert!(
            body.get("output_config").is_none(),
            "DBv2 Claude route: none effort must omit output_config"
        );
    }

    #[test]
    fn dbv2_route_switch_max_body_level_simulation() {
        // Body-level simulation of a session/set_model switch from a Claude model to a GPT-5
        // model when thinking_effort=max. Calls body builders and normalizers directly (not
        // through the ACP session/set_model path or DatabricksV2 dispatch) to verify the
        // correct output shape for each side of the route switch.
        // Before the switch: Claude route → max passes through as Anthropic "max".
        // After the switch: GPT-5 route → max clamped to xhigh.
        let mut c = cfg(Provider::Anthropic);
        c.max_output_tokens = 32_768;

        // Before switch: claude-opus-4-8 with effort=max → adaptive shape, effort="max"
        let (thinking_before, oc_before) = crate::config::anthropic_thinking_config(
            "claude-opus-4-8",
            ThinkingEffort::Max,
            32_768,
        );
        assert_eq!(thinking_before.unwrap()["type"], "adaptive");
        assert_eq!(oc_before.unwrap()["effort"], "max");

        // After switch to GPT-5.5 route: normalize max → xhigh for responses_body
        // (gpt-5.5 supports xhigh, so the clamp result is xhigh, not further reduced)
        let clamped =
            crate::config::normalize_effort_for_openai_route(ThinkingEffort::Max, "gpt-5.5");
        assert_eq!(clamped, ThinkingEffort::XHigh);
        let body_after = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "gpt-5.5",
            Some(clamped),
        );
        assert_eq!(
            body_after["reasoning"]["effort"], "xhigh",
            "After set_model to GPT-5.5: max must be clamped to xhigh"
        );
    }

    /// Regression: a connection that is accepted and then dropped before any
    /// HTTP response bytes are written surfaces as a reqwest request-class
    /// error (not `is_connect()`, not `is_timeout()`). The retry predicate
    /// must recognize it; otherwise transient TLS/h2/proxy hiccups bubble
    /// out of the agent as `transport: error sending request ...`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_retries_on_dropped_connection_before_response() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/x", listener.local_addr().unwrap());
        let accepts = Arc::new(AtomicU32::new(0));
        let accepts_srv = accepts.clone();

        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let n = accepts_srv.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    // First attempt: read the request, then drop the socket
                    // without writing a response. reqwest surfaces this as
                    // a request-class error (is_request() == true).
                    let mut tmp = [0u8; 4096];
                    let _ = sock.read(&mut tmp).await;
                    drop(sock);
                    continue;
                }
                // Subsequent attempts: serve a tiny JSON body.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    }
                }
                let body = "{\"ok\":true}";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let out = post(&client, &url, &serde_json::json!({}), false, |b| b)
            .await
            .expect("post should succeed after retry");
        assert_eq!(out, serde_json::json!({ "ok": true }));
        assert!(
            accepts.load(Ordering::SeqCst) >= 2,
            "server should have seen at least 2 connection attempts, saw {}",
            accepts.load(Ordering::SeqCst)
        );
    }

    /// A 499 response is retried and the call succeeds on the second attempt.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_retries_499_and_succeeds() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/x", listener.local_addr().unwrap());
        let accepts = Arc::new(AtomicU32::new(0));
        let accepts_srv = accepts.clone();

        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let n = accepts_srv.fetch_add(1, Ordering::SeqCst);
                // Read the full request headers before responding.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    }
                }
                if n == 0 {
                    // First attempt: respond with 499.
                    let resp = "HTTP/1.1 499 Client Closed Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                    continue;
                }
                // Subsequent attempts: 200 OK with a tiny JSON body.
                let body = "{\"ok\":true}";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let out = post(&client, &url, &serde_json::json!({}), false, |b| b)
            .await
            .expect("post should succeed after 499 retry");
        assert_eq!(out, serde_json::json!({ "ok": true }));
        assert!(
            accepts.load(Ordering::SeqCst) >= 2,
            "server must see at least 2 attempts (got {})",
            accepts.load(Ordering::SeqCst)
        );
    }

    /// When all MAX_RETRIES attempts return 499 the error includes "exhausted retries".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_exhausts_retries_on_persistent_499() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/x", listener.local_addr().unwrap());
        let accepts = Arc::new(AtomicU32::new(0));
        let accepts_srv = accepts.clone();

        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                accepts_srv.fetch_add(1, Ordering::SeqCst);
                // Read the full request headers before responding.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    }
                }
                // Always 499.
                let resp = "HTTP/1.1 499 Client Closed Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let err = post(&client, &url, &serde_json::json!({}), false, |b| b)
            .await
            .unwrap_err();
        match &err {
            PostError::Agent(AgentError::Llm(msg)) => {
                assert!(
                    msg.contains("exhausted retries") && msg.contains("499"),
                    "expected 'exhausted retries' + '499' in error, got: {msg}"
                );
                assert!(
                    msg.contains("cumulative") && msg.contains("3 attempts"),
                    "expected cumulative duration + exact attempt count, got: {msg}"
                );
            }
            other => panic!("expected PostError::Agent(AgentError::Llm), got: {other:?}"),
        }
        assert_eq!(
            accepts.load(Ordering::SeqCst),
            MAX_RETRIES,
            "server must see exactly MAX_RETRIES attempts — 499 must be retried"
        );
    }

    /// `terminal_llm_error` below `STALL_NOTICE_THRESHOLD` carries the detail
    /// and attempt count but no stall-specific text — this is the common case
    /// (a handful of quick retries), not an outage.
    #[test]
    fn terminal_llm_error_below_threshold_carries_detail_no_stall_claim() {
        let err = terminal_llm_error(Duration::from_secs(2), 3, "exhausted retries: 499: body");
        match err {
            AgentError::Llm(msg) => {
                assert!(msg.contains("cumulative 2s"), "must carry duration: {msg}");
                assert!(
                    msg.contains("3 attempts"),
                    "must carry attempt count: {msg}"
                );
                assert!(
                    msg.contains("exhausted retries") && msg.contains("499"),
                    "must preserve detail: {msg}"
                );
            }
            other => panic!("expected Llm, got: {other:?}"),
        }
    }

    /// Above `STALL_NOTICE_THRESHOLD`, the error still carries the same
    /// detail/duration/count — the threshold only gates the extra
    /// `tracing::warn!` (not separately assertable here), not the error text.
    /// Singular "1 attempt" (not "1 attempts") confirms the pluralization.
    #[test]
    fn terminal_llm_error_above_threshold_uses_singular_attempt() {
        let err = terminal_llm_error(Duration::from_secs(301), 1, "transport: connection reset");
        match err {
            AgentError::Llm(msg) => {
                assert!(
                    msg.contains("cumulative 301s"),
                    "must carry duration: {msg}"
                );
                assert!(msg.contains("1 attempt"), "must carry count: {msg}");
                assert!(
                    !msg.contains("1 attempts"),
                    "singular for count of 1: {msg}"
                );
            }
            other => panic!("expected Llm, got: {other:?}"),
        }
    }

    /// Captures `tracing::warn!` events emitted during a scoped subscriber
    /// so a test can assert on the stall-notice threshold without a real
    /// sleep or a global logger.
    struct StallWarnCapture {
        count: Arc<std::sync::atomic::AtomicUsize>,
    }

    struct StallWarnVisitor {
        saw_cumulative_stall: bool,
        saw_attempts: bool,
    }

    impl tracing::field::Visit for StallWarnVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {
            match field.name() {
                "cumulative_stall" => self.saw_cumulative_stall = true,
                "attempts" => self.saw_attempts = true,
                _ => {}
            }
        }

        fn record_u64(&mut self, field: &tracing::field::Field, _value: u64) {
            if field.name() == "attempts" {
                self.saw_attempts = true;
            }
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for StallWarnCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if *event.metadata().level() != tracing::Level::WARN {
                return;
            }
            let mut visitor = StallWarnVisitor {
                saw_cumulative_stall: false,
                saw_attempts: false,
            };
            event.record(&mut visitor);
            if visitor.saw_cumulative_stall && visitor.saw_attempts {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// Runs `f` under a scoped subscriber that only counts `WARN` events
    /// carrying both `cumulative_stall` and `attempts` fields — the shape
    /// `terminal_llm_error`'s stall notice emits. Returns that count.
    fn count_stall_warnings(f: impl FnOnce()) -> usize {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(StallWarnCapture {
            count: count.clone(),
        });
        tracing::subscriber::with_default(subscriber, f);
        count.load(Ordering::SeqCst)
    }

    /// Below `STALL_NOTICE_THRESHOLD`, `terminal_llm_error` must not emit the
    /// stall warning at all — otherwise every ordinary retry-exhaustion would
    /// falsely read as an outage. This is mutation-sensitive: deleting the
    /// threshold branch, weakening `>=` to `>` at the boundary, or an
    /// always-warn mutation are all caught by pairing this with the 300s case
    /// below.
    #[test]
    fn terminal_llm_error_below_threshold_emits_no_stall_warning() {
        let warnings = count_stall_warnings(|| {
            let _ = terminal_llm_error(Duration::from_secs(299), 3, "exhausted retries: 499");
        });
        assert_eq!(warnings, 0, "no stall warning below STALL_NOTICE_THRESHOLD");
    }

    /// At `STALL_NOTICE_THRESHOLD`, `terminal_llm_error` must emit exactly
    /// one stall warning carrying the duration and attempt count — the
    /// never-warn mutation is caught here; combined with the 299s case above,
    /// a deleted branch or an always-warn mutation are caught by whichever
    /// assertion it violates.
    #[test]
    fn terminal_llm_error_at_threshold_emits_one_stall_warning() {
        let warnings = count_stall_warnings(|| {
            let _ = terminal_llm_error(Duration::from_secs(300), 5, "transport: connection reset");
        });
        assert_eq!(
            warnings, 1,
            "exactly one stall warning with duration+attempts fields at threshold"
        );
    }

    // ---- usage / input-token extraction -------------------------------------

    #[test]
    fn parse_anthropic_sums_input_and_cache_tokens() {
        // input_tokens alone excludes cached tokens; the inclusive total must
        // sum all three so a cache-heavy turn can't undercount the budget.
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}],
            "usage": {
                "input_tokens": 100,
                "cache_read_input_tokens": 900,
                "cache_creation_input_tokens": 50,
                "output_tokens": 7
            }
        });
        let r = parse_anthropic(v).unwrap();
        assert_eq!(r.input_tokens, Some(1050));
    }

    #[test]
    fn parse_anthropic_input_tokens_only() {
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}],
            "usage": {"input_tokens": 42, "output_tokens": 3}
        });
        assert_eq!(parse_anthropic(v).unwrap().input_tokens, Some(42));
    }

    #[test]
    fn parse_anthropic_missing_usage_is_none() {
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}]
        });
        assert_eq!(parse_anthropic(v).unwrap().input_tokens, None);
    }

    /// A Gemini reply as the Databricks MLflow route actually returns it:
    /// block-array `content`, and a `thoughtSignature` beside `function`.
    fn gemini_choice() -> Value {
        json!({"choices": [{"finish_reason": "tool_calls", "message": {
            "role": "assistant",
            "content": [
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "weighing it"}]},
                {"type": "text", "text": "391"}
            ],
            "tool_calls": [{
                "id": "get_weather", "type": "function", "thoughtSignature": "SIG-A",
                "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}
            }]
        }}]})
    }

    #[test]
    fn parse_openai_reads_text_out_of_a_block_array() {
        // Before this, `as_str()` on the array yielded "" and the model's answer
        // was discarded with no error at all.
        let r = parse_openai(gemini_choice()).unwrap();
        assert_eq!(r.text, "391");
        assert_eq!(r.reasoning, "weighing it");
    }

    #[test]
    fn parse_openai_still_reads_a_plain_string_content() {
        let v = json!({"choices": [{"finish_reason": "stop", "message": {
            "role": "assistant", "content": "plain"}}]});
        let r = parse_openai(v).unwrap();
        assert_eq!(r.text, "plain");
        assert_eq!(r.reasoning, "");
    }

    #[test]
    fn parse_openai_keeps_unmodelled_tool_call_fields() {
        let r = parse_openai(gemini_choice()).unwrap();
        let extra = &r.tool_calls[0].provider_extra;
        assert_eq!(extra.get("thoughtSignature"), Some(&json!("SIG-A")));
        // `id`/`type`/`function` are modelled, so they must not be duplicated
        // into the passthrough — they would be re-emitted twice.
        assert!(!extra.contains_key("id"));
        assert!(!extra.contains_key("type"));
        assert!(!extra.contains_key("function"));
    }

    #[test]
    fn openai_body_replays_the_signature_beside_function_not_inside_it() {
        // Position is what the gateway checks: nested inside `function{}` it is
        // rejected with the same 400 as a missing signature.
        let r = parse_openai(gemini_choice()).unwrap();
        let history = vec![HistoryItem::Assistant {
            text: r.text.clone(),
            tool_calls: r.tool_calls.clone(),
            reasoning_details: None,
        }];
        let body = openai_body(
            &cfg(Provider::DatabricksV2),
            "sys",
            &history,
            &[],
            "databricks-gemini-3-6-flash",
            None,
        );
        let call = &body["messages"][1]["tool_calls"][0];
        assert_eq!(call["thoughtSignature"], json!("SIG-A"));
        assert!(call["function"].get("thoughtSignature").is_none());
        assert_eq!(call["function"]["name"], json!("get_weather"));
    }

    #[test]
    fn parse_openai_makes_duplicate_tool_call_ids_unique() {
        // Gemini returns the function name as the id, so parallel calls to one
        // function collide and their results become indistinguishable.
        let v = json!({"choices": [{"finish_reason": "tool_calls", "message": {
        "role": "assistant", "content": "",
        "tool_calls": [
            {"id": "get_weather", "type": "function",
             "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}},
            {"id": "get_weather", "type": "function",
             "function": {"name": "get_weather", "arguments": "{\"city\":\"Rome\"}"}}
        ]}}]});
        let r = parse_openai(v).unwrap();
        assert_eq!(r.tool_calls[0].provider_id, "get_weather");
        assert_eq!(r.tool_calls[1].provider_id, "get_weather-2");
    }

    #[test]
    fn parse_openai_uses_prompt_tokens() {
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}],
            "usage": {"prompt_tokens": 123, "completion_tokens": 4, "total_tokens": 127}
        });
        assert_eq!(parse_openai(v).unwrap().input_tokens, Some(123));
    }

    #[test]
    fn parse_openai_databricks_prompt_tokens_already_inclusive() {
        // Databricks' MLflow route uses the OpenAI chat wire format
        // (prompt_tokens) but ALSO reports the flat Anthropic-style
        // cache_read_input_tokens. prompt_tokens is already inclusive of that
        // slice, so the total is prompt_tokens alone — summing double-counts.
        // Values are the live databricks-glm-5-2 response (2026-07-28), where
        // prompt_tokens + completion_tokens == total_tokens proves inclusivity.
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}],
            "usage": {
                "prompt_tokens": 13320,
                "completion_tokens": 30,
                "total_tokens": 13350,
                "cache_read_input_tokens": 13312,
                "prompt_tokens_details": {"cached_tokens": 13312}
            }
        });
        let r = parse_openai(v).unwrap();
        assert_eq!(
            r.input_tokens,
            Some(13320),
            "prompt_tokens is the inclusive total"
        );
        assert_eq!(r.cached_input_tokens, Some(13312));
        assert!(
            r.cached_input_tokens.unwrap() <= r.input_tokens.unwrap(),
            "the cached slice is a subset of the input total"
        );
    }

    #[test]
    fn parse_openai_missing_usage_is_none() {
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}]
        });
        assert_eq!(parse_openai(v).unwrap().input_tokens, None);
    }

    // ── total_tokens parsing ───────────────────────────────────────────────

    #[test]
    fn parse_openai_chat_total_tokens_present_is_read() {
        // Chat Completions: `usage.total_tokens` is a genuine provider total.
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "ok"}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 25, "total_tokens": 125}
        });
        let r = parse_openai(v).unwrap();
        assert_eq!(r.total_tokens, Some(125));
        assert_eq!(r.input_tokens, Some(100));
        assert_eq!(r.output_tokens, Some(25));
    }

    #[test]
    fn parse_openai_chat_total_tokens_absent_is_none() {
        // Chat Completions without `total_tokens` → None, not a derived sum.
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "ok"}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 25}
        });
        let r = parse_openai(v).unwrap();
        assert_eq!(r.total_tokens, None);
    }

    #[test]
    fn parse_responses_total_tokens_present_is_read() {
        // Responses API: `usage.total_tokens` is a genuine provider total.
        let v = serde_json::json!({
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}],
            "status": "completed",
            "usage": {"input_tokens": 80, "output_tokens": 20, "total_tokens": 100}
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.total_tokens, Some(100));
        assert_eq!(r.input_tokens, Some(80));
        assert_eq!(r.output_tokens, Some(20));
    }

    #[test]
    fn parse_responses_total_tokens_absent_is_none() {
        // Responses API without `total_tokens` → None.
        let v = serde_json::json!({
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}],
            "status": "completed",
            "usage": {"input_tokens": 80, "output_tokens": 20}
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.total_tokens, None);
    }

    #[test]
    fn parse_anthropic_total_tokens_always_none() {
        // Anthropic reports only category counts; NIP-AM forbids deriving a total.
        // total_tokens must always be None regardless of what the response contains —
        // including if a future Anthropic API version unexpectedly adds total_tokens.
        let v = serde_json::json!({
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 100,
                "cache_read_input_tokens": 50,
                "cache_creation_input_tokens": 0,
                "output_tokens": 30,
                // Unexpected field: parse_anthropic must ignore this and return None.
                "total_tokens": 180
            }
        });
        let r = parse_anthropic(v).unwrap();
        assert!(
            r.total_tokens.is_none(),
            "Anthropic must never supply a total_tokens value"
        );
        // Verify other fields still parse correctly.
        assert_eq!(r.input_tokens, Some(150)); // inclusive sum with cache
        assert_eq!(r.output_tokens, Some(30));
    }

    #[test]
    fn parse_openai_reads_nested_cached_tokens() {
        // The shape vanilla OpenAI actually returns, captured from a live
        // /chat/completions probe on gpt-5.6-luna: `prompt_tokens` is already
        // inclusive and the cache split is nested one level down. Reading only
        // flat keys left the discount unclaimed while the total looked correct,
        // which is why this went unnoticed.
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "OK"}}],
            "usage": {
                "prompt_tokens": 5229,
                "completion_tokens": 4,
                "total_tokens": 5233,
                "prompt_tokens_details": {"audio_tokens": 0, "cached_tokens": 5226,
                                          "cache_write_tokens": 0}
            }
        });
        let r = parse_openai(v).unwrap();
        assert_eq!(r.input_tokens, Some(5229), "total must stay inclusive");
        assert_eq!(r.cached_input_tokens, Some(5226));
    }

    #[test]
    fn parse_openai_cache_write_round_reports_zero_cached() {
        // First request of a cold prefix: the provider writes the cache and
        // serves nothing from it. `Some(0)` not `None` — the split was reported,
        // it was simply zero, and a consumer must be able to tell the two apart.
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "OK"}}],
            "usage": {
                "prompt_tokens": 5229,
                "completion_tokens": 4,
                "prompt_tokens_details": {"cached_tokens": 0, "cache_write_tokens": 5226}
            }
        });
        assert_eq!(parse_openai(v).unwrap().cached_input_tokens, Some(0));
    }

    #[test]
    fn parse_openai_no_cache_detail_is_none() {
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}],
            "usage": {"prompt_tokens": 123, "completion_tokens": 4}
        });
        assert_eq!(parse_openai(v).unwrap().cached_input_tokens, None);
    }

    #[test]
    fn parse_openai_prefers_flat_anthropic_spelling_over_nested() {
        // Databricks reports both shapes for the same quantity. Take one, never
        // the sum, or the cached slice double-counts. cache_read (800) is a
        // subset of the inclusive prompt_tokens (1000), as it must be.
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 4,
                "cache_read_input_tokens": 800,
                "prompt_tokens_details": {"cached_tokens": 800}
            }
        });
        let r = parse_openai(v).unwrap();
        assert_eq!(r.input_tokens, Some(1000));
        assert_eq!(r.cached_input_tokens, Some(800));
        assert!(r.cached_input_tokens.unwrap() <= r.input_tokens.unwrap());
    }

    #[test]
    fn parse_anthropic_reports_cache_read_as_cached() {
        // Anthropic's `input_tokens` EXCLUDES cached, so the inclusive total is
        // a sum -- but the cached slice must still be a subset of that total.
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 7,
                "cache_read_input_tokens": 900,
                "cache_creation_input_tokens": 50
            }
        });
        let r = parse_anthropic(v).unwrap();
        assert_eq!(r.input_tokens, Some(1050));
        assert_eq!(r.cached_input_tokens, Some(900));
        assert!(r.cached_input_tokens.unwrap() <= r.input_tokens.unwrap());
    }

    #[test]
    fn parse_responses_reads_nested_cached_tokens() {
        // The Responses API nests the same figure under a different key than
        // /chat/completions does.
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi"}]
            }],
            "usage": {
                "input_tokens": 4000,
                "output_tokens": 9,
                "input_tokens_details": {"cached_tokens": 3584}
            }
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.input_tokens, Some(4000));
        assert_eq!(r.cached_input_tokens, Some(3584));
    }

    #[test]
    fn parse_responses_uses_input_tokens() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi"}]
            }],
            "usage": {"input_tokens": 321, "output_tokens": 9, "total_tokens": 330}
        });
        assert_eq!(parse_responses(v).unwrap().input_tokens, Some(321));
    }

    #[test]
    fn parse_responses_missing_usage_is_none() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi"}]
            }]
        });
        assert_eq!(parse_responses(v).unwrap().input_tokens, None);
    }

    #[test]
    fn sum_usage_empty_object_is_none() {
        // A `usage` object present but carrying none of the requested fields
        // is "no usable reading" -> None, not Some(0).
        let v = serde_json::json!({"usage": {"output_tokens": 5}});
        assert_eq!(sum_usage(&v, &["input_tokens", "prompt_tokens"]), None);
    }

    /// A token source whose `bearer()` always hands back the same stale
    /// token and whose `refresh_now()` mints a distinct fresh one, counting
    /// each refresh. Lets a test assert exactly how many forced refreshes a
    /// `post_openai` call provoked.
    struct CountingAuth {
        refreshes: std::sync::atomic::AtomicU32,
    }

    #[async_trait::async_trait]
    impl TokenSource for CountingAuth {
        async fn bearer(&self) -> Result<String, AgentError> {
            Ok("stale".into())
        }
        async fn refresh_now(&self, _rejected: &str) -> Result<String, AgentError> {
            self.refreshes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("fresh".into())
        }
    }

    /// Stub that answers `reject_status` to any request carrying `Bearer
    /// stale` and 200 to `Bearer fresh`. When `always_reject` is set it rejects
    /// unconditionally, simulating a token the refresh can never satisfy.
    /// Counts requests so a test can assert "one retry, not a loop".
    async fn spawn_auth_stub(
        always_reject: std::sync::Arc<std::sync::atomic::AtomicBool>,
        reject_status: u16,
    ) -> String {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let reject_line = match reject_status {
            401 => "401 Unauthorized",
            403 => "403 Forbidden",
            other => panic!("unsupported reject_status {other}"),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let always_reject = always_reject.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(k) => buf.extend_from_slice(&tmp[..k]),
                        }
                    }
                    let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                    let stale = head.contains("authorization: bearer stale");
                    let resp = if always_reject.load(Ordering::SeqCst) || stale {
                        format!(
                            "HTTP/1.1 {reject_line}\r\nContent-Length: 11\r\n\
                             Connection: close\r\n\r\ntoken stale"
                        )
                    } else {
                        let body = "{\"ok\":true}";
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body,
                        )
                    };
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        url
    }

    fn llm_with(auth: Arc<dyn TokenSource>) -> Llm {
        Llm {
            http: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            auto_upgraded: std::sync::atomic::AtomicBool::new(false),
            mesh_auto_state: Mutex::new(MeshAutoState::default()),
            auth,
        }
    }

    /// A single 401 forces exactly one refresh, the retry with the fresh
    /// token succeeds, and a *later* call gets its own refresh — proving the
    /// one-shot guard is per-call, not stored on the source.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openai_refreshes_once_per_call_on_401() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        let always_401 = Arc::new(AtomicBool::new(false));
        let base = spawn_auth_stub(always_401, 401).await;
        let auth = Arc::new(CountingAuth {
            refreshes: AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base;

        let out = llm
            .post_openai(&c, "/v1/x", &json!({}), "model")
            .await
            .expect("retry with fresh token should succeed");
        assert_eq!(out, json!({ "ok": true }));
        assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1, "one refresh");

        // Second call's 401 must trigger its own refresh — the guard cannot
        // be a stored flag that an earlier turn already tripped.
        let out2 = llm
            .post_openai(&c, "/v1/x", &json!({}), "model")
            .await
            .unwrap();
        assert_eq!(out2, json!({ "ok": true }));
        assert_eq!(
            auth.refreshes.load(Ordering::SeqCst),
            2,
            "later call gets its own retry"
        );
    }

    /// A persistent 401 (even the refreshed token is rejected) propagates as
    /// `LlmAuth` after exactly one refresh — no infinite loop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openai_persistent_401_propagates_after_one_retry() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        let always_401 = Arc::new(AtomicBool::new(true));
        let base = spawn_auth_stub(always_401, 401).await;
        let auth = Arc::new(CountingAuth {
            refreshes: AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base;

        let err = llm
            .post_openai(&c, "/v1/x", &json!({}), "model")
            .await
            .unwrap_err();
        assert!(
            matches!(err, PostError::Agent(AgentError::LlmAuth(_))),
            "got {err:?}"
        );
        assert_eq!(
            auth.refreshes.load(Ordering::SeqCst),
            1,
            "exactly one refresh, then propagate"
        );
    }

    /// A 403 is treated as refreshable: a persistent 403 forces exactly one
    /// refresh-and-retry, then propagates as `LlmAuth`. Proves a revoked-token
    /// 403 takes the same recovery path as a 401, bounded by the per-call guard.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openai_persistent_403_propagates_after_one_retry() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        let always_403 = Arc::new(AtomicBool::new(true));
        let base = spawn_auth_stub(always_403, 403).await;
        let auth = Arc::new(CountingAuth {
            refreshes: AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base;

        let err = llm
            .post_openai(&c, "/v1/x", &json!({}), "model")
            .await
            .unwrap_err();
        assert!(
            matches!(err, PostError::Agent(AgentError::LlmAuth(_))),
            "got {err:?}"
        );
        assert_eq!(
            auth.refreshes.load(Ordering::SeqCst),
            1,
            "403 refreshes exactly once, then propagates"
        );
    }

    /// A recoverable 403 (stale token 403s, fresh token 200s) forces exactly
    /// one refresh and the retry succeeds — proving a 403 enters the refresh
    /// path and a refreshed token clears it, the stale-token-403 recovery case.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openai_refreshes_once_on_403() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        let always_403 = Arc::new(AtomicBool::new(false));
        let base = spawn_auth_stub(always_403, 403).await;
        let auth = Arc::new(CountingAuth {
            refreshes: AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base;

        let out = llm
            .post_openai(&c, "/v1/x", &json!({}), "model")
            .await
            .expect("retry with fresh token should clear the 403");
        assert_eq!(out, json!({ "ok": true }));
        assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1, "one refresh");
    }

    /// The default `refresh_now()` on a static source returns the static
    /// token unchanged — a key that can't refresh still answers harmlessly.
    #[tokio::test]
    async fn static_token_source_refresh_now_returns_static_token() {
        let src = StaticTokenSource::new("static-key");
        assert_eq!(src.refresh_now("rejected").await.unwrap(), "static-key");
    }

    // ── Output-token parsing tests ──────────────────────────────────────────

    /// `parse_anthropic` extracts `output_tokens` from the usage object.
    #[test]
    fn parse_anthropic_output_tokens() {
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}],
            "usage": {"input_tokens": 42, "output_tokens": 7}
        });
        assert_eq!(parse_anthropic(v).unwrap().output_tokens, Some(7));
    }

    /// `parse_anthropic` returns `None` for `output_tokens` when usage is absent.
    #[test]
    fn parse_anthropic_output_tokens_missing_usage_is_none() {
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}]
        });
        assert_eq!(parse_anthropic(v).unwrap().output_tokens, None);
    }

    /// `parse_openai` maps `completion_tokens` to `output_tokens`.
    #[test]
    fn parse_openai_output_tokens_from_completion_tokens() {
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}],
            "usage": {"prompt_tokens": 123, "completion_tokens": 4, "total_tokens": 127}
        });
        assert_eq!(parse_openai(v).unwrap().output_tokens, Some(4));
    }

    /// `parse_openai` returns `None` for `output_tokens` when usage is absent.
    #[test]
    fn parse_openai_output_tokens_missing_usage_is_none() {
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}]
        });
        assert_eq!(parse_openai(v).unwrap().output_tokens, None);
    }

    /// `parse_responses` extracts `output_tokens` from the usage object.
    #[test]
    fn parse_responses_output_tokens() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi"}]
            }],
            "usage": {"input_tokens": 321, "output_tokens": 9, "total_tokens": 330}
        });
        assert_eq!(parse_responses(v).unwrap().output_tokens, Some(9));
    }

    /// `parse_responses` returns `None` for `output_tokens` when usage is absent.
    #[test]
    fn parse_responses_output_tokens_missing_usage_is_none() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi"}]
            }]
        });
        assert_eq!(parse_responses(v).unwrap().output_tokens, None);
    }

    // ---- A3: OpenRouter body-shape tests ----

    fn tools_vec() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "dev__shell".into(),
            description: "run a shell command".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
            }),
        }]
    }

    #[test]
    fn openrouter_body_tools_with_effort() {
        let mut c = cfg(Provider::OpenRouter);
        c.thinking_effort = Some(ThinkingEffort::High);
        let mut body = openai_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &tools_vec(),
            "anthropic/claude-opus-4-7",
            None,
        );
        apply_openrouter_mutations(
            &mut body,
            c.thinking_effort,
            "anthropic/claude-opus-4-7",
            true,
        );
        assert_eq!(body["reasoning"]["effort"], "high");
        // `openai_body` is always called with `effort=None` on the OpenRouter
        // path (line 151): OpenRouter uses its own `reasoning` object, not the
        // OpenAI-style `reasoning_effort` field. Verify the field is structurally
        // absent — not merely removed by a no-op cleanup.
        assert!(
            body.get("reasoning_effort").is_none(),
            "reasoning_effort must never appear on the OpenRouter body path: \
             openai_body is called with effort=None, so the field is never emitted"
        );
        assert!(
            body.get("provider").is_none(),
            "provider.require_parameters hard-404s models that do not advertise \
             every parameter we send"
        );
        assert!(!body["tools"].as_array().unwrap().is_empty());
        assert_eq!(
            body["max_tokens"], 1024,
            "token limit must carry openai_body's value under OpenRouter's spelling"
        );
        assert!(
            body.get("max_completion_tokens").is_none(),
            "max_completion_tokens is the OpenAI-native spelling; OpenRouter reads max_tokens"
        );
    }

    #[test]
    fn openrouter_body_tools_no_effort() {
        let c = cfg(Provider::OpenRouter);
        let mut body = openai_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &tools_vec(),
            "anthropic/claude-opus-4-7",
            None,
        );
        apply_openrouter_mutations(&mut body, None, "anthropic/claude-opus-4-7", true);
        assert!(
            body.get("reasoning").is_none(),
            "reasoning must be absent when effort is None"
        );
        assert!(
            body.get("reasoning_effort").is_none(),
            "reasoning_effort must never appear on the OpenRouter body path"
        );
        assert!(
            body.get("provider").is_none(),
            "tools alone must not add a provider routing filter"
        );
    }

    #[test]
    fn openrouter_body_empty_tools_with_effort() {
        let mut c = cfg(Provider::OpenRouter);
        c.thinking_effort = Some(ThinkingEffort::Medium);
        let mut body = openai_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "anthropic/claude-opus-4-7",
            None,
        );
        apply_openrouter_mutations(
            &mut body,
            c.thinking_effort,
            "anthropic/claude-opus-4-7",
            true,
        );
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert!(
            body.get("provider").is_none(),
            "83 of 274 tools-capable models do not advertise `reasoning`; filtering on \
             it would 404 them instead of answering without reasoning"
        );
    }

    #[test]
    fn openrouter_body_empty_tools_no_effort() {
        let c = cfg(Provider::OpenRouter);
        let mut body = openai_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "anthropic/claude-opus-4-7",
            None,
        );
        apply_openrouter_mutations(&mut body, None, "anthropic/claude-opus-4-7", true);
        assert!(body.get("reasoning").is_none());
        assert!(
            body.get("provider").is_none(),
            "no body shape adds a provider routing filter"
        );
    }

    /// `BUZZ_AGENT_PROMPT_CACHING=0` must reach the OpenRouter `anthropic/*`
    /// route too, not just the native Anthropic Messages routes. The switch and
    /// this route landed in separate changes, so nothing but this test stops the
    /// gate from being dropped and the kill switch silently becoming a no-op on
    /// a route that emits Anthropic-dialect breakpoints.
    #[test]
    fn openrouter_body_caching_disabled_emits_no_cache_control() {
        let c = cfg(Provider::OpenRouter);
        let mut body = openai_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &tools_vec(),
            "anthropic/claude-opus-4-7",
            None,
        );
        apply_openrouter_mutations(&mut body, None, "anthropic/claude-opus-4-7", false);
        assert!(
            !body.to_string().contains("cache_control"),
            "BUZZ_AGENT_PROMPT_CACHING=0 must suppress every breakpoint: {body}"
        );
        // The switch is scoped to caching — the routing-contract mutations that
        // make the request serveable at all must still be applied.
        assert_eq!(
            body.get("max_tokens").and_then(Value::as_u64),
            Some(u64::from(c.max_output_tokens)),
            "the max_tokens rename is not part of the caching gate"
        );
    }

    /// The paired positive case: with caching on, the same body does carry
    /// breakpoints. Without this twin, a mutation that hard-disabled caching
    /// outright would still leave the test above green.
    #[test]
    fn openrouter_body_caching_enabled_emits_cache_control() {
        let c = cfg(Provider::OpenRouter);
        let mut body = openai_body(
            &c,
            "system",
            &[HistoryItem::User("hi".into())],
            &tools_vec(),
            "anthropic/claude-opus-4-7",
            None,
        );
        apply_openrouter_mutations(&mut body, None, "anthropic/claude-opus-4-7", true);
        assert!(
            body.to_string().contains("cache_control"),
            "caching on must still emit breakpoints: {body}"
        );
    }

    /// The rename moves an existing value; it never invents a token limit for a
    /// body that did not carry one.
    #[test]
    fn openrouter_body_without_token_limit_gains_none() {
        let mut body = json!({ "model": "vendor/model", "messages": [] });
        apply_openrouter_mutations(&mut body, None, "vendor/model", true);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn openrouter_summary_carries_neither_reasoning_nor_provider() {
        let body = openrouter_summary_body(
            "anthropic/claude-opus-4-7",
            "summarize",
            "text to summarize",
            1024,
        );
        assert_eq!(body["model"], "anthropic/claude-opus-4-7");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "text to summarize");
        assert_eq!(body["max_tokens"], 1024);
        assert!(
            body.get("max_completion_tokens").is_none(),
            "summary body must use OpenRouter's token-limit spelling"
        );
        assert!(
            body.get("reasoning").is_none(),
            "summary body must not carry reasoning"
        );
        assert!(
            body.get("provider").is_none(),
            "summary body must not carry provider"
        );
    }

    /// The token-limit rename belongs to `apply_openrouter_mutations`, not to
    /// `openai_body` — which OpenAI and Databricks also use, and where
    /// `max_completion_tokens` is the correct spelling.
    #[test]
    fn openai_body_keeps_max_completion_tokens_when_unmutated() {
        let body = openai_body(
            &cfg(Provider::OpenAi),
            "system",
            &[HistoryItem::User("hi".into())],
            &[],
            "model",
            None,
        );
        assert_eq!(body["max_completion_tokens"], 1024);
        assert!(body.get("max_tokens").is_none());
    }

    // ---- A5: error-inside-200 ----

    #[test]
    fn parse_openai_error_inside_200_returns_error() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "error",
                "error": {
                    "code": 503,
                    "message": "No endpoints found that support tool use"
                }
            }]
        });
        let err = parse_openai(v).unwrap_err();
        match &err {
            AgentError::Llm(s) => {
                assert!(s.contains("provider error (503)"), "got: {s}");
                assert!(s.contains("No endpoints found"), "got: {s}");
            }
            _ => panic!("expected AgentError::Llm, got: {err:?}"),
        }
    }

    #[test]
    fn parse_openai_error_inside_200_accepts_string_code() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "error",
                "error": {
                    "code": "insufficient_quota",
                    "message": "quota exceeded"
                }
            }]
        });
        let err = parse_openai(v).unwrap_err();
        match &err {
            AgentError::Llm(s) => assert!(
                s.contains("provider error (insufficient_quota)"),
                "got: {s}"
            ),
            _ => panic!("expected AgentError::Llm, got: {err:?}"),
        }
    }

    #[test]
    fn parse_openai_error_inside_200_surfaces_error_type() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "error",
                "error": {
                    "code": 429,
                    "message": "Rate limit exceeded",
                    "metadata": { "error_type": "rate_limit_exceeded" }
                }
            }]
        });
        let err = parse_openai(v).unwrap_err();
        match &err {
            AgentError::Llm(s) => {
                assert!(s.contains("429"), "got: {s}");
                assert!(s.contains("rate_limit_exceeded"), "got: {s}");
            }
            _ => panic!("expected AgentError::Llm, got: {err:?}"),
        }
    }

    #[test]
    fn parse_openai_normal_stop_not_affected_by_error_check() {
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hello"}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let r = parse_openai(v).unwrap();
        assert_eq!(r.text, "hello");
        assert_eq!(r.stop, ProviderStop::EndTurn);
    }

    #[test]
    fn parse_openai_tool_calls_not_affected_by_error_check() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "test", "arguments": "{}"}
                    }]
                }
            }]
        });
        let r = parse_openai(v).unwrap();
        assert_eq!(r.stop, ProviderStop::ToolUse);
        assert_eq!(r.tool_calls.len(), 1);
    }

    // ---- A6: OpenRouter retry classification ----

    #[test]
    fn classify_429_rate_limit_body_retry_after_is_ignored() {
        // M1: no documented body-level retry field, so a `retry_after` key
        // inside `error.metadata` is inert — only the HTTP header (passed
        // separately) can produce a delay.
        let body =
            r#"{"error":{"metadata":{"error_type":"rate_limit_exceeded","retry_after":2.5}}}"#;
        match classify_openrouter_error(429, body, None) {
            OpenRouterErrorClass::Retryable(None) => {}
            other => panic!("expected Retryable(None), got: {other:?}"),
        }
    }

    #[test]
    fn classify_429_rate_limit_without_retry_after() {
        let body = r#"{"error":{"metadata":{"error_type":"rate_limit_exceeded"}}}"#;
        match classify_openrouter_error(429, body, None) {
            OpenRouterErrorClass::Retryable(None) => {}
            other => panic!("expected Retryable(None), got: {other:?}"),
        }
    }

    #[test]
    fn classify_429_prefers_http_header_over_body() {
        // Body-level retry hints are no longer parsed (M1: undocumented field);
        // the HTTP header is the only source, and it's honored when present.
        let body = r#"{"error":{"metadata":{"error_type":"rate_limit_exceeded"}}}"#;
        let header = Some(Duration::from_secs(3));
        match classify_openrouter_error(429, body, header) {
            OpenRouterErrorClass::Retryable(Some(d)) => {
                assert_eq!(d, Duration::from_secs(3), "HTTP header must be honored");
            }
            other => panic!("expected Retryable with header delay, got: {other:?}"),
        }
    }

    #[test]
    fn classify_502_provider_unavailable() {
        let body = r#"{"error":{"metadata":{"error_type":"provider_unavailable"}}}"#;
        match classify_openrouter_error(502, body, None) {
            OpenRouterErrorClass::Retryable(None) => {}
            other => panic!("expected Retryable(None) for 502, got: {other:?}"),
        }
    }

    #[test]
    fn classify_503_provider_overloaded_with_retry_after() {
        // No documented body-level retry field (M1); the header is the only
        // source `classify_openrouter_error` consults.
        let body = r#"{"error":{"metadata":{"error_type":"provider_overloaded"}}}"#;
        let header = Some(Duration::from_secs(5));
        match classify_openrouter_error(503, body, header) {
            OpenRouterErrorClass::Retryable(Some(d)) => {
                assert_eq!(d, Duration::from_secs(5));
            }
            other => panic!("expected Retryable with delay, got: {other:?}"),
        }
    }

    #[test]
    fn classify_503_untyped_is_unknown() {
        let body = r#"{"error":{"message":"No endpoints found"}}"#;
        match classify_openrouter_error(503, body, None) {
            OpenRouterErrorClass::Unknown => {}
            other => panic!("expected Unknown for untyped 503, got: {other:?}"),
        }
    }

    #[test]
    fn classify_500_untyped_is_unknown() {
        match classify_openrouter_error(500, r#"{"error":{"message":"internal"}}"#, None) {
            OpenRouterErrorClass::Unknown => {}
            other => panic!("expected Unknown for 500, got: {other:?}"),
        }
    }

    #[test]
    fn parse_retry_after_header_valid() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "5".parse().unwrap());
        assert_eq!(
            parse_retry_after_header(&headers),
            Some(Duration::from_secs(5))
        );
    }

    #[test]
    fn parse_retry_after_header_zero_rejected() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "0".parse().unwrap());
        assert_eq!(parse_retry_after_header(&headers), None);
    }

    #[test]
    fn parse_retry_after_header_over_cap_clamped() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "3601".parse().unwrap());
        assert_eq!(
            parse_retry_after_header(&headers),
            Some(Duration::from_secs(RETRY_AFTER_CAP_SECS)),
            "over-cap hints clamp to the ceiling rather than being dropped"
        );
    }

    #[test]
    fn parse_retry_after_header_at_cap_unclamped() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            RETRY_AFTER_CAP_SECS.to_string().parse().unwrap(),
        );
        assert_eq!(
            parse_retry_after_header(&headers),
            Some(Duration::from_secs(RETRY_AFTER_CAP_SECS))
        );
    }

    #[test]
    fn parse_retry_after_header_missing() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(parse_retry_after_header(&headers), None);
    }

    #[test]
    fn parse_retry_after_header_non_numeric_ignored() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(parse_retry_after_header(&headers), None);
    }

    // ---- A7: Anthropic cache_control with mixed content ----

    #[test]
    fn anthropic_cache_control_mixed_text_tool_image_history() {
        let history = vec![
            HistoryItem::User("first question".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    provider_id: "toolu_1".into(),
                    name: "dev__view_image".into(),
                    arguments: serde_json::json!({"source": "x.png"}),
                    provider_extra: Default::default(),
                }],
                reasoning_details: None,
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "toolu_1".into(),
                content: vec![
                    ToolResultContent::Text("10×10 image".into()),
                    ToolResultContent::Image {
                        data: "aW1n".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
            }),
            HistoryItem::User("second question about the image".into()),
            HistoryItem::User("third question".into()),
        ];
        let mut body = openai_body(
            &cfg(Provider::OpenRouter),
            "system",
            &history,
            &tools_vec(),
            "anthropic/claude-opus-4-7",
            None,
        );
        apply_openrouter_mutations(&mut body, None, "anthropic/claude-opus-4-7", true);
        let messages = body["messages"].as_array().unwrap();

        // System message should have cache_control
        let system = &messages[0];
        assert_eq!(system["content"][0]["cache_control"]["type"], "ephemeral");

        // Image-batch user messages (containing image_url blocks) must NOT have cache_control
        let image_user_msgs: Vec<_> = messages
            .iter()
            .filter(|m| {
                m.get("role").and_then(Value::as_str) == Some("user")
                    && m.get("content")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .any(|b| b.get("type").and_then(Value::as_str) == Some("image_url"))
                        })
                        .unwrap_or(false)
            })
            .collect();
        assert!(
            !image_user_msgs.is_empty(),
            "should have image user messages"
        );
        for img_msg in &image_user_msgs {
            let content = img_msg["content"].as_array().unwrap();
            for block in content {
                assert!(
                    block.get("cache_control").is_none(),
                    "image-only user message must not receive cache_control"
                );
            }
        }

        // Exactly 2 text user messages should have cache_control (skipping image-only ones)
        let cached_text_count = messages
            .iter()
            .filter(|m| {
                m.get("role").and_then(Value::as_str) == Some("user")
                    && m.get("content")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter().any(|b| {
                                b.get("type").and_then(Value::as_str) == Some("text")
                                    && b.get("cache_control").is_some()
                            })
                        })
                        .unwrap_or(false)
            })
            .count();
        assert_eq!(
            cached_text_count, 2,
            "exactly 2 text user messages should have cache_control"
        );

        // Last tool def should have cache_control
        let tools = body["tools"].as_array().unwrap();
        let last_tool = tools.last().unwrap();
        assert_eq!(last_tool["function"]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn anthropic_cache_control_image_only_user_does_not_consume_slot() {
        // An image-only user message between two text user messages must not
        // consume a cache breakpoint slot — both text messages should get cached.
        let history = vec![
            HistoryItem::User("text message one".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    provider_id: "toolu_1".into(),
                    name: "dev__view_image".into(),
                    arguments: serde_json::json!({"source": "x.png"}),
                    provider_extra: Default::default(),
                }],
                reasoning_details: None,
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "toolu_1".into(),
                content: vec![ToolResultContent::Image {
                    data: "aW1n".into(),
                    mime_type: "image/png".into(),
                }],
                is_error: false,
            }),
            HistoryItem::User("text message two".into()),
            HistoryItem::User("text message three".into()),
        ];
        let mut body = openai_body(
            &cfg(Provider::OpenRouter),
            "system",
            &history,
            &[],
            "anthropic/claude-opus-4-7",
            None,
        );
        apply_openrouter_mutations(&mut body, None, "anthropic/claude-opus-4-7", true);
        let messages = body["messages"].as_array().unwrap();

        // Count text user messages that got cache_control
        let cached_text_count = messages
            .iter()
            .filter(|m| {
                m.get("role").and_then(Value::as_str) == Some("user")
                    && m.get("content")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().any(|b| b.get("cache_control").is_some()))
                        .unwrap_or(false)
            })
            .count();
        assert_eq!(
            cached_text_count, 2,
            "image-only user messages must not consume a cache breakpoint slot"
        );
    }

    // ---- A9: reasoning_details round-trip ----

    #[test]
    fn parse_openai_with_reasoning_details_captures_array() {
        let details = serde_json::json!([
            {"type": "thinking", "content": "Let me consider..."},
            {"type": "thinking", "content": "The answer is 42."}
        ]);
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "",
                    "reasoning_details": details,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "test", "arguments": "{}"}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let r = parse_openai_with_reasoning_details(v).unwrap();
        assert_eq!(r.reasoning_details, Some(details));
    }

    #[test]
    fn parse_openai_with_reasoning_details_none_when_absent() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": "hello"}
            }]
        });
        let r = parse_openai_with_reasoning_details(v).unwrap();
        assert!(
            r.reasoning_details.is_none(),
            "reasoning_details must be None when not in response"
        );
    }

    /// M2: a malformed `reasoning_details` shape (null or a bare object,
    /// rather than the documented array) must be omitted at the parse
    /// boundary, not stored and later replayed into the next request.
    #[test]
    fn parse_openai_with_reasoning_details_omits_non_array_shapes() {
        let null_shape = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": "hello", "reasoning_details": null}
            }]
        });
        let r = parse_openai_with_reasoning_details(null_shape).unwrap();
        assert!(
            r.reasoning_details.is_none(),
            "null reasoning_details must be omitted, not stored as Some(Null)"
        );

        let object_shape = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "hello",
                    "reasoning_details": {"type": "thinking", "content": "not an array"}
                }
            }]
        });
        let r = parse_openai_with_reasoning_details(object_shape).unwrap();
        assert!(
            r.reasoning_details.is_none(),
            "a bare-object reasoning_details must be omitted, not stored as Some(object)"
        );
    }

    #[test]
    fn parse_openai_plain_never_captures_reasoning_details() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "hello",
                    "reasoning_details": [{"type": "thinking", "content": "hmm"}]
                }
            }]
        });
        let r = parse_openai(v).unwrap();
        assert!(
            r.reasoning_details.is_none(),
            "plain parse_openai must never capture reasoning_details (OpenAI/Databricks regression)"
        );
    }

    #[test]
    fn reasoning_details_two_request_round_trip() {
        let details = serde_json::json!([
            {"type": "thinking", "content": "Step 1: analyze the request."},
            {"type": "thinking", "content": "Step 2: call the tool."}
        ]);
        // Request 1: model returns a tool call with reasoning_details
        let response1 = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "",
                    "reasoning_details": details,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {"name": "dev__shell", "arguments": "{\"command\":\"ls\"}"}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 50, "completion_tokens": 20}
        });
        let r1 = parse_openai_with_reasoning_details(response1).unwrap();
        assert_eq!(r1.reasoning_details, Some(details.clone()));

        // Build history as the agent would: assistant turn with reasoning_details,
        // followed by a tool result.
        let history = vec![
            HistoryItem::User("run ls".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: r1.tool_calls,
                reasoning_details: r1.reasoning_details,
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "call_abc".into(),
                content: vec![ToolResultContent::Text("file.txt".into())],
                is_error: false,
            }),
        ];

        // Request 2: build the body for the continuation
        let body = openai_body(
            &cfg(Provider::OpenRouter),
            "system",
            &history,
            &[],
            "anthropic/claude-opus-4-7",
            None,
        );
        let messages = body["messages"].as_array().unwrap();

        // The assistant message must carry the identical reasoning_details array
        let assistant_msg = messages
            .iter()
            .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
            .expect("assistant message must exist");
        assert_eq!(
            assistant_msg["reasoning_details"], details,
            "reasoning_details must be replayed byte-for-byte on the assistant message"
        );

        // The assistant message must appear BEFORE the tool result
        let assistant_idx = messages
            .iter()
            .position(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
            .unwrap();
        let tool_idx = messages
            .iter()
            .position(|m| m.get("role").and_then(Value::as_str) == Some("tool"))
            .unwrap();
        assert!(
            assistant_idx < tool_idx,
            "assistant with reasoning_details must precede tool result"
        );
    }

    #[test]
    fn reasoning_details_none_emits_no_field_in_body() {
        let history = vec![
            HistoryItem::User("hello".into()),
            HistoryItem::Assistant {
                text: "hi back".into(),
                tool_calls: Vec::new(),
                reasoning_details: None,
            },
        ];
        let body = openai_body(
            &cfg(Provider::OpenRouter),
            "system",
            &history,
            &[],
            "anthropic/claude-opus-4-7",
            None,
        );
        let messages = body["messages"].as_array().unwrap();
        let assistant_msg = messages
            .iter()
            .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
            .expect("assistant message must exist");
        assert!(
            assistant_msg.get("reasoning_details").is_none(),
            "assistant with None reasoning_details must not emit the field"
        );
    }

    #[test]
    fn reasoning_details_charged_to_estimated_bytes() {
        let details = serde_json::json!([
            {"type": "thinking", "content": "A long chain of reasoning tokens here."}
        ]);
        let with = HistoryItem::Assistant {
            text: "text".into(),
            tool_calls: Vec::new(),
            reasoning_details: Some(details.clone()),
        };
        let without = HistoryItem::Assistant {
            text: "text".into(),
            tool_calls: Vec::new(),
            reasoning_details: None,
        };
        assert!(
            with.estimated_bytes() > without.estimated_bytes(),
            "reasoning_details must contribute to estimated_bytes"
        );
        assert!(
            with.context_pressure_bytes() > without.context_pressure_bytes(),
            "reasoning_details must contribute to context_pressure_bytes"
        );
        let details_size = serde_json::to_vec(&details).unwrap().len();
        assert_eq!(
            with.estimated_bytes() - without.estimated_bytes(),
            details_size,
            "reasoning_details contribution must equal its serialized size"
        );
    }

    #[test]
    fn reasoning_details_not_replayed_in_anthropic_body() {
        let history = vec![
            HistoryItem::User("hi".into()),
            HistoryItem::Assistant {
                text: "ok".into(),
                tool_calls: Vec::new(),
                reasoning_details: Some(
                    serde_json::json!([{"type": "thinking", "content": "hmm"}]),
                ),
            },
        ];
        let body = anthropic_body(
            &cfg(Provider::Anthropic),
            "system",
            &history,
            &[],
            "claude-opus-4-7",
            None,
        );
        let messages = body["messages"].as_array().unwrap();
        let assistant = messages
            .iter()
            .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
            .unwrap();
        assert!(
            assistant.get("reasoning_details").is_none(),
            "anthropic_body must not replay reasoning_details"
        );
    }

    #[test]
    fn reasoning_details_not_replayed_in_responses_body() {
        let history = vec![
            HistoryItem::User("hi".into()),
            HistoryItem::Assistant {
                text: "ok".into(),
                tool_calls: Vec::new(),
                reasoning_details: Some(
                    serde_json::json!([{"type": "thinking", "content": "hmm"}]),
                ),
            },
        ];
        let body = responses_body(&cfg_responses(), "system", &history, &[], "model", None);
        let body_str = serde_json::to_string(&body).unwrap();
        assert!(
            !body_str.contains("reasoning_details"),
            "responses_body must not replay reasoning_details"
        );
    }

    // ---- T4: openrouter_post transport-level regressions ----
    //
    // These stub an HTTP server directly and drive `openrouter_post` (not the
    // classifier in isolation), proving the retry/attempt-accounting and
    // header behavior the classifier-only tests above cannot see.

    /// One canned response: status, body, and any extra headers (e.g.
    /// `Retry-After`) to send back for a single request.
    struct CannedResponse {
        status: u16,
        body: String,
        extra_headers: Vec<(String, String)>,
    }

    impl CannedResponse {
        fn new(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.into(),
                extra_headers: Vec::new(),
            }
        }

        fn with_header(mut self, name: &str, value: &str) -> Self {
            self.extra_headers.push((name.into(), value.into()));
            self
        }
    }

    fn status_line(status: u16) -> &'static str {
        match status {
            200 => "200 OK",
            400 => "400 Bad Request",
            413 => "413 Payload Too Large",
            401 => "401 Unauthorized",
            402 => "402 Payment Required",
            403 => "403 Forbidden",
            404 => "404 Not Found",
            429 => "429 Too Many Requests",
            499 => "499 Client Closed Request",
            500 => "500 Internal Server Error",
            502 => "502 Bad Gateway",
            503 => "503 Service Unavailable",
            _ => panic!("unsupported status {status} in test stub"),
        }
    }

    /// Spawns a stub HTTP server that pops one `CannedResponse` per request
    /// (repeating the last one once the queue is exhausted, so an
    /// over-budget attempt count is visible rather than hanging), and
    /// captures each request's raw header block for header-attribution
    /// assertions. Returns (url, captured_header_blocks, attempt_counter).
    async fn spawn_openrouter_stub(
        responses: Vec<CannedResponse>,
    ) -> (
        String,
        Arc<Mutex<Vec<String>>>,
        Arc<std::sync::atomic::AtomicU32>,
    ) {
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let attempts = Arc::new(AtomicU32::new(0));
        let captured_clone = captured.clone();
        let attempts_clone = attempts.clone();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let queue = queue.clone();
                let captured = captured_clone.clone();
                let attempts = attempts_clone.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                        if buf.len() > 1_000_000 {
                            return;
                        }
                    }
                    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                    let header_str = String::from_utf8_lossy(&buf[..header_end]).into_owned();
                    // Drain any remaining body per Content-Length so `Connection:
                    // close` doesn't race the client's write.
                    let content_length: usize = header_str
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    let mut body_len = buf.len() - header_end;
                    while body_len < content_length {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => body_len += n,
                        }
                    }
                    captured.lock().await.push(header_str);
                    attempts.fetch_add(1, Ordering::SeqCst);

                    let mut q = queue.lock().await;
                    let canned = if q.len() > 1 {
                        q.pop_front().unwrap()
                    } else {
                        // Repeat the final canned response so a test bug that
                        // over-retries produces a visible extra attempt
                        // instead of a hung connection.
                        let last = q.front().unwrap();
                        CannedResponse {
                            status: last.status,
                            body: last.body.clone(),
                            extra_headers: last.extra_headers.clone(),
                        }
                    };
                    drop(q);

                    let mut resp = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
                        status_line(canned.status),
                        canned.body.len()
                    );
                    for (name, value) in &canned.extra_headers {
                        resp.push_str(&format!("{name}: {value}\r\n"));
                    }
                    resp.push_str("Connection: close\r\n\r\n");
                    resp.push_str(&canned.body);
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (url, captured, attempts)
    }

    /// Wren's rider: assert on the error emerging from `complete()` for the
    /// OpenRouter path, not from `openrouter_post`. The bug was the `?` in the
    /// provider arm, which is invisible from below — a low-level test can see
    /// the classification but not whether the arm returns it into the
    /// convergence mapper. The regression test has to cross the layer that had
    /// the bug.
    ///
    /// Two claims here: the variant is `LlmContextExceeded` (so the agent loop
    /// can recover), and the message carries the `(model)` stamp (so the arm
    /// reaches the mapper at all). Measured before the fix: variant was correct
    /// but UNSTAMPED, which is exactly the bypass Wren named.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_context_400_is_typed_and_stamped_through_complete() {
        let (url, _captured, _attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            400,
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens","code":"context_length_exceeded"}}"#,
        )])
        .await;
        let mut c = cfg(Provider::OpenRouter);
        c.base_url = url;
        let llm = Llm::new(&c).unwrap();
        let err = complete_model(&llm, &c, "or-model-xyz").await.unwrap_err();
        assert!(
            matches!(err, AgentError::LlmContextExceeded(_)),
            "OpenRouter context-window 400 must classify as LlmContextExceeded, got: {err:?}"
        );
        let text = err.to_string();
        assert!(
            text.contains("or-model-xyz"),
            "OpenRouter arm must return into the convergence mapper so the model stamp is \
             applied; got: {text}"
        );
    }

    /// Same two claims on the Anthropic arm — the other `?` Wren named, and the
    /// other terminal's provider phrasing ("prompt is too long").
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_context_400_is_typed_and_stamped_through_complete() {
        let (base_url, _captured) = spawn_sequence_stub(vec![StubHttpResponse {
            status: 400,
            body: json!({"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 300000 tokens > 200000 maximum"}}),
        }])
        .await;
        let mut c = cfg(Provider::Anthropic);
        c.base_url = base_url;
        let llm = Llm::new(&c).unwrap();
        let err = complete_model(&llm, &c, "claude-probe-model")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentError::LlmContextExceeded(_)),
            "Anthropic context-window 400 must classify as LlmContextExceeded, got: {err:?}"
        );
        let text = err.to_string();
        assert!(
            text.contains("claude-probe-model"),
            "Anthropic arm must return into the convergence mapper so the model stamp is \
             applied; got: {text}"
        );
    }

    /// Negative arm for the OpenRouter terminal: an ordinary 400 must stay
    /// `AgentError::Llm`. Paired with the positive above, this is what proves
    /// the matcher — not the status alone — is doing the classification. The
    /// body deliberately quotes "tokens" and "model", the words a loose matcher
    /// would key on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_ordinary_400_stays_plain_llm_error() {
        let (url, _captured, _attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            400,
            r#"{"error":{"message":"Invalid value for 'max_tokens': must be an integer for this model","code":"invalid_value"}}"#,
        )])
        .await;
        let mut c = cfg(Provider::OpenRouter);
        c.base_url = url;
        let llm = Llm::new(&c).unwrap();
        let err = complete_model(&llm, &c, "or-model-xyz").await.unwrap_err();
        assert!(
            matches!(err, AgentError::Llm(_)),
            "an ordinary 400 must stay a terminal AgentError::Llm, got: {err:?}"
        );
    }

    /// Negative arm for the shared `post()` terminal (OpenAI/Databricks), the
    /// second of the two `!status.is_success()` sites. Same body as the
    /// OpenRouter negative so the two terminals are compared on equal input.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_ordinary_400_stays_plain_llm_error() {
        let (base_url, _captured) = spawn_sequence_stub(vec![StubHttpResponse {
            status: 400,
            body: json!({"error":{"message":"Invalid value for 'max_tokens': must be an integer for this model","code":"invalid_value"}}),
        }])
        .await;
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base_url;
        let llm = Llm::new(&c).unwrap();
        let err = complete_model(&llm, &c, "gpt-probe-model")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentError::Llm(_)),
            "an ordinary 400 must stay a terminal AgentError::Llm, got: {err:?}"
        );
    }

    /// Positive arm for the shared `post()` terminal: OpenAI's machine-readable
    /// `context_length_exceeded` code classifies as recoverable.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_context_400_is_typed_through_complete() {
        let (base_url, _captured) = spawn_sequence_stub(vec![StubHttpResponse {
            status: 400,
            body: json!({"error":{"message":"This model's maximum context length is 8192 tokens.","type":"invalid_request_error","code":"context_length_exceeded"}}),
        }])
        .await;
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base_url;
        let llm = Llm::new(&c).unwrap();
        let err = complete_model(&llm, &c, "gpt-probe-model")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentError::LlmContextExceeded(_)),
            "OpenAI context-window 400 must classify as LlmContextExceeded, got: {err:?}"
        );
        assert!(
            err.to_string().contains("gpt-probe-model"),
            "expected the convergence mapper's model stamp, got: {err}"
        );
    }

    /// A context-window 400 must NOT trip the Responses-API auto-upgrade. True
    /// by construction — `try_upgrade` matches only `AgentError::Llm` and the
    /// typed variant can never reach it — but asserted because the guarantee
    /// lives in a pattern match one refactor away from widening, and a silent
    /// sticky upgrade would reroute every later OpenAI call for the process.
    ///
    /// `openai_api = Auto` is load-bearing in BOTH arms: `try_upgrade` is only
    /// consulted under `Auto` (`llm.rs:587`), so with the test helper's default
    /// `Chat` the upgrade path is disabled outright and the negative below would
    /// pass without observing anything. The control caught exactly that.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn context_400_does_not_trip_responses_upgrade() {
        let (base_url, _captured) = spawn_sequence_stub(vec![StubHttpResponse {
            status: 400,
            body: json!({"error":{"message":"This model's maximum context length is 8192 tokens.","code":"context_length_exceeded"}}),
        }])
        .await;
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base_url;
        c.openai_api = OpenAiApi::Auto;
        let llm = Llm::new(&c).unwrap();
        let err = complete_model(&llm, &c, "gpt-probe-model")
            .await
            .unwrap_err();
        assert!(matches!(err, AgentError::LlmContextExceeded(_)));
        assert!(
            !llm.auto_upgraded.load(Ordering::Relaxed),
            "a context-window 400 must not latch the Responses-API upgrade"
        );
        // Positive control: the same helper DOES latch on a genuine
        // "use the Responses API" error, so the negative above is a real
        // observation and not a probe that can never fire.
        let (base_url2, _c2) = spawn_sequence_stub(vec![StubHttpResponse {
            status: 400,
            body: json!({"error":{"message":"This model is only supported in /v1/responses"}}),
        }])
        .await;
        let mut c2 = cfg(Provider::OpenAi);
        c2.base_url = base_url2;
        c2.openai_api = OpenAiApi::Auto;
        let llm2 = Llm::new(&c2).unwrap();
        let _ = complete_model(&llm2, &c2, "gpt-probe-model").await;
        assert!(
            llm2.auto_upgraded.load(Ordering::Relaxed),
            "control: a genuine Responses-API error must latch the upgrade"
        );
    }

    /// The `status == 400` conjunct is load-bearing, not belt-and-braces: the
    /// recovery ladder is only a correct response to an INPUT-SIZE rejection.
    /// A 403 whose body happens to quote context-window prose (a guardrail
    /// echoing the request, say) is a permission failure — shrinking history
    /// cannot fix it, so classifying it as recoverable would burn the whole
    /// recovery budget on three doomed summarize round-trips and turn a clear
    /// immediate error into a slow one.
    ///
    /// 413 (Payload Too Large) is the right probe status, and picking it took a
    /// measurement: my first attempt used 403, which BOTH ladders intercept
    /// earlier (shared `post()` maps 401/403 to `LlmAuth`; `openrouter_post()`
    /// has its own 403 arm), so those probes never reached the classification
    /// site at all and the mutant with the conjunct deleted survived them. 413
    /// is intercepted by neither ladder, so it reaches the same
    /// `!status.is_success()` terminal the 400 does — and it is the most
    /// plausible real-world carrier of size prose on a non-400. One arm per
    /// terminal site.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_413_with_context_prose_is_not_recoverable() {
        let (base_url, _captured) = spawn_sequence_stub(vec![StubHttpResponse {
            status: 413,
            body: json!({"error":{"message":"payload too large: this model's maximum context length is 8192 tokens"}}),
        }])
        .await;
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base_url;
        let llm = Llm::new(&c).unwrap();
        let err = complete_model(&llm, &c, "gpt-probe-model")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentError::Llm(_)),
            "only a 400 may classify as a context overflow; a 413 must stay terminal, got: \
             {err:?}"
        );
    }

    /// Same claim at the OpenRouter terminal, which has its own status ladder.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_413_with_context_prose_is_not_recoverable() {
        let (url, _captured, _attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            413,
            r#"{"error":{"message":"payload too large: this model's maximum context length is 8192 tokens"}}"#,
        )])
        .await;
        let mut c = cfg(Provider::OpenRouter);
        c.base_url = url;
        let llm = Llm::new(&c).unwrap();
        let err = complete_model(&llm, &c, "or-model-xyz").await.unwrap_err();
        assert!(
            matches!(err, AgentError::Llm(_)),
            "only a 400 may classify as a context overflow; a 413 must stay terminal, got: \
             {err:?}"
        );
    }

    /// A 403 (guardrail/moderation/permission rejection, per OpenRouter docs)
    /// must NOT be classified as `LlmAuth`: refreshing a static key returns
    /// the identical key, so retrying would just waste a duplicate request.
    /// Exactly one attempt, plain `AgentError::Llm` with the body preserved.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_403_single_attempt_not_auth_error() {
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            403,
            r#"{"error":{"message":"model flagged by moderation"}}"#,
        )])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let err = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AgentError::Llm(s) if s.contains("403") && s.contains("model flagged by moderation")),
            "403 must surface as AgentError::Llm with status+body, not LlmAuth: got {err:?}"
        );
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "403 must not be retried (a refreshed static key is identical)"
        );
    }

    /// A 402 short-circuits on the first attempt: no retry, one request,
    /// the actionable credits-exhausted message.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_402_single_attempt_short_circuit() {
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            402,
            r#"{"error":{"message":"payment required"}}"#,
        )])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let err = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AgentError::Llm(s) if s.contains("credits exhausted")),
            "got {err:?}"
        );
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "402 must not be retried"
        );
    }

    /// A 404 whose body is OpenRouter's parameter-routing rejection is NOT a
    /// missing model: the id is valid and no endpoint behind it can serve the
    /// request shape. It must surface the actionable routing message rather than
    /// `LlmModelNotFound`, which sends the user hunting a model-name typo.
    /// Body text is the one OpenRouter actually returned in the live probe run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_404_no_endpoints_found_is_parameter_routing_error() {
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            404,
            r#"{"error":{"message":"No endpoints found that can handle the requested parameters. To learn more about provider routing, visit: https://openrouter.ai/docs/guides/routing/provider-selection","code":404}}"#,
        )])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let err = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AgentError::Llm(s) if s.contains("no OpenRouter endpoint supports")),
            "parameter-routing 404 must not be reported as a missing model: got {err:?}"
        );
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "404 must not be retried"
        );
    }

    /// A provider's explicit image-capability rejection is a recoverable typed
    /// error, not a missing model. The agent loop uses this signal to remove the
    /// image from history before retrying the next LLM round.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_404_unsupported_image_is_typed_and_not_retried() {
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            404,
            r#"{"error":{"message":"No endpoints found that support image input"}}"#,
        )])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let err = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AgentError::UnsupportedImageInput(s) if s.contains("support image input")),
            "image rejection must reach the history-recovery path: got {err:?}"
        );
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a deterministic capability rejection must not be retried"
        );
    }

    /// Every other 404 still maps to `LlmModelNotFound`, including one that
    /// shares the `No endpoints found` prefix but is about the model rather than
    /// the parameters — the discriminator is narrow enough that a genuinely
    /// unavailable model keeps its own error kind (Desktop renders
    /// model-not-found differently from a generic LLM failure).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_404_unknown_model_stays_model_not_found() {
        let (url, _captured, _attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            404,
            r#"{"error":{"message":"No endpoints found for vendor/nonexistent-model.","code":404}}"#,
        )])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let err = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AgentError::LlmModelNotFound(s) if s.contains("404") && s.contains("vendor/nonexistent-model")),
            "a model-level 404 must stay LlmModelNotFound: got {err:?}"
        );
    }

    /// A 429 with `Retry-After: 1` sleeps for that duration before the retry
    /// succeeds — proving the header value is actually honored, not just
    /// classified.
    #[tokio::test(flavor = "current_thread")]
    async fn openrouter_post_429_honors_retry_after_header() {
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![
            CannedResponse::new(429, r#"{"error":{"message":"rate limited"}}"#)
                .with_header("Retry-After", "1"),
            CannedResponse::new(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#),
        ])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let before = std::time::Instant::now();
        let out = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .expect("second attempt succeeds");
        assert_eq!(out["choices"][0]["message"]["content"], "ok");
        assert!(
            before.elapsed() >= Duration::from_secs(1),
            "must sleep at least the Retry-After hint"
        );
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// A `Retry-After` far beyond `RETRY_AFTER_CAP_SECS` must not stall the
    /// retry loop for anywhere near its advertised duration — proving the
    /// cap is enforced end-to-end in `openrouter_post`'s actual sleep, not
    /// merely in the isolated `parse_retry_after_header` unit tests above.
    /// Runs on a paused clock so a real 999999s wait would hang the test
    /// instead of silently passing.
    #[tokio::test(start_paused = true)]
    async fn openrouter_post_429_retry_sleep_capped_despite_huge_retry_after() {
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![
            CannedResponse::new(429, r#"{"error":{"message":"rate limited"}}"#)
                .with_header("Retry-After", "999999"),
            CannedResponse::new(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#),
        ])
        .await;
        let http = Client::builder().build().unwrap();
        let before = tokio::time::Instant::now();
        let out = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .expect("second attempt succeeds");
        assert_eq!(out["choices"][0]["message"]["content"], "ok");
        assert!(
            before.elapsed() <= Duration::from_secs(RETRY_AFTER_CAP_SECS + 5),
            "retry sleep must be clamped to RETRY_AFTER_CAP_SECS ({RETRY_AFTER_CAP_SECS}s), \
             not the header's 999999s: elapsed {:?}",
            before.elapsed()
        );
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// An untyped 503 (no `error.metadata.error_type`) exhausts all
    /// `MAX_RETRIES` attempts, then returns the actionable routing message —
    /// proving attempt accounting terminates rather than retrying forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_untyped_503_exhausts_retries_into_actionable_message() {
        let canned = CannedResponse::new(503, r#"{"error":{"message":"no capacity"}}"#);
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![canned]).await;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let err = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AgentError::Llm(s) if s.contains("no OpenRouter endpoint supports")),
            "got {err:?}"
        );
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            MAX_RETRIES,
            "must exhaust exactly MAX_RETRIES attempts, no more"
        );
    }

    /// Attribution headers (`HTTP-Referer`, `X-OpenRouter-Title`) are on the
    /// actual wire request, not merely asserted against a body fixture.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_sends_attribution_headers() {
        let (url, captured, _attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            200,
            r#"{"choices":[{"message":{"content":"ok"}}]}"#,
        )])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .expect("200 succeeds");
        let headers = captured.lock().await;
        let header_str = headers
            .first()
            .expect("one request captured")
            .to_lowercase();
        assert!(
            header_str.contains("http-referer: https://github.com/block/buzz"),
            "got: {header_str}"
        );
        assert!(
            header_str.contains("x-openrouter-title: buzz"),
            "got: {header_str}"
        );
    }

    /// A 499 response is retried and the call succeeds on the second attempt,
    /// mirroring the shared `post()` path (#2175).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_retries_499_then_succeeds() {
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![
            CannedResponse::new(499, ""),
            CannedResponse::new(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#),
        ])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let out = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .expect("retry after 499 should succeed");
        assert_eq!(out["choices"][0]["message"]["content"], "ok");
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "exactly one 499 retry"
        );
    }

    /// A 502 without `provider_unavailable` still retries (the classifier's
    /// unconditional-retry branch), then succeeds on attempt 2 — proving the
    /// generic 502 path isn't accidentally routed to `Unknown`/terminal.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_502_retries_then_succeeds() {
        let (url, _captured, attempts) = spawn_openrouter_stub(vec![
            CannedResponse::new(502, r#"{"error":{"message":"bad gateway"}}"#),
            CannedResponse::new(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#),
        ])
        .await;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let out = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .expect("retry succeeds");
        assert_eq!(out["choices"][0]["message"]["content"], "ok");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// A 200 response whose body is truncated mid-stream (connection closed
    /// before the full Content-Length is delivered) must surface the error
    /// through `terminal_llm_error`, not a bare `AgentError::Llm("read: …")` —
    /// the caller needs cumulative duration and attempt-count context to diagnose
    /// an upstream that silently drops connections.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openrouter_post_truncated_200_body_wraps_in_terminal_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // Spawn a stub that sends a 200 with Content-Length > actual body,
        // then closes the connection — reqwest sees a truncated stream.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                // Read until end-of-headers, ignore body
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                // Claim 1 MB of body, send only 10 bytes, then close.
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                          Content-Length: 1048576\r\nConnection: close\r\n\r\n\
                          {truncated",
                    )
                    .await;
                let _ = sock.shutdown().await;
            }
        });

        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let err = openrouter_post(&http, &format!("{url}/x"), &json!({}), "key")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AgentError::Llm(s) if s.contains("body read")),
            "truncated body must surface as AgentError::Llm with 'body read': got {err:?}"
        );
        assert!(
            matches!(&err, AgentError::Llm(s) if s.contains("cumulative")),
            "body-read error must include terminal_llm_error's cumulative context: got {err:?}"
        );
    }

    /// A `TokenSource` whose `refresh_now` always returns the identical token —
    /// models a static API key whose bytes never change on refresh.
    struct StaticAuth {
        token: String,
    }

    #[async_trait::async_trait]
    impl TokenSource for StaticAuth {
        async fn bearer(&self) -> Result<String, AgentError> {
            Ok(self.token.clone())
        }
        async fn refresh_now(&self, _rejected: &str) -> Result<String, AgentError> {
            Ok(self.token.clone()) // static: same bytes every time
        }
    }

    /// A `TokenSource` that returns a stale token from `bearer()` and a
    /// distinct fresh token from `refresh_now()`, modelling a PKCE OAuth source.
    struct MintingAuth {
        stale: String,
        fresh: String,
        refreshes: std::sync::atomic::AtomicU32,
    }

    #[async_trait::async_trait]
    impl TokenSource for MintingAuth {
        async fn bearer(&self) -> Result<String, AgentError> {
            Ok(self.stale.clone())
        }
        async fn refresh_now(&self, _rejected: &str) -> Result<String, AgentError> {
            self.refreshes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.fresh.clone())
        }
    }

    /// A static key 401: `refresh_now` returns the same bytes — the second
    /// wire request would be byte-identical, so the retry must be skipped.
    /// Exactly one wire request reaches the server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openrouter_static_key_401_single_attempt_no_retry() {
        use std::sync::atomic::Ordering;

        let (url, _captured, attempts) = spawn_openrouter_stub(vec![CannedResponse::new(
            401,
            r#"{"error":{"message":"invalid api key"}}"#,
        )])
        .await;
        let auth = Arc::new(StaticAuth {
            token: "static-key".into(),
        });
        let llm = llm_with(auth);
        let mut c = cfg(Provider::OpenRouter);
        c.base_url = url;

        let err = llm.post_openrouter(&c, &json!({})).await.unwrap_err();
        assert!(
            matches!(&err, AgentError::LlmAuth(s) if s.contains("static key rejected")),
            "static 401 must surface as LlmAuth with 'static key rejected': got {err:?}"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "exactly one wire request — no duplicate retry for a static key"
        );
    }

    /// A minting-source 401: `refresh_now` produces a distinct fresh token, so
    /// the retry is legitimate. The stub accepts the fresh token's second
    /// request with 200 and exactly two wire requests reach the server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openrouter_minting_source_401_retries_with_fresh_token() {
        use std::sync::atomic::Ordering;

        // Stub: always 401 for bearer "stale", 200 for anything else.
        // We repurpose `spawn_auth_stub` here: it rejects `Bearer stale`,
        // accepts `Bearer fresh`.
        let always_401 = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let base = spawn_auth_stub(always_401, 401).await;

        let auth = Arc::new(MintingAuth {
            stale: "stale".into(),
            fresh: "fresh".into(),
            refreshes: std::sync::atomic::AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenRouter);
        c.base_url = base;

        let result = llm.post_openrouter(&c, &json!({})).await;
        // `spawn_auth_stub` returns `{"ok":true}` on success.
        assert!(
            result.is_ok(),
            "minting-source retry should succeed: {result:?}"
        );
        assert_eq!(
            auth.refreshes.load(Ordering::SeqCst),
            1,
            "exactly one refresh"
        );
    }
}
