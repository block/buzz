use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 2;

/// Reasoning/thinking effort level for providers that support it.
///
/// Set via `BUZZ_AGENT_THINKING_EFFORT` (`none|minimal|low|medium|high|xhigh|max`).
/// When unset the provider's default behaviour is preserved — no thinking
/// config is sent in the request body.
///
/// Provider support (doc-verified, July 2025):
/// - **Anthropic adaptive**: `low|medium|high|xhigh|max` (model-dependent; see `anthropic_thinking_config_generated`).
///   `none`/`minimal` are not Anthropic values — rejected at startup.
/// - **Anthropic manual budget** (claude-3*, opus-4-5): `low|medium|high`; `xhigh`/`max` clamp to high budget.
/// - **OpenAI Responses / Chat Completions**: effort support is model-dependent and normalized at
///   request time; `max` is valid for documented max-supporting families such as GPT-5.6.
/// - **Databricks**: routed by model family (Claude → Anthropic mapping, GPT-5 → Responses, MLflow → Chat).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThinkingEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ThinkingEffort {
    /// Map level to an Anthropic `budget_tokens` value for legacy Claude 3.x / Opus 4.5 models.
    /// `XHigh` and `Max` clamp to the high budget value; the answer-room reserve of 1024 tokens
    /// is applied separately in `anthropic_thinking_config_generated`.
    pub fn anthropic_budget_tokens(self) -> u32 {
        match self {
            ThinkingEffort::Low => 1_024,
            ThinkingEffort::Medium => 8_192,
            ThinkingEffort::High | ThinkingEffort::XHigh | ThinkingEffort::Max => 32_768,
            // None/Minimal are not valid for Anthropic (rejected at startup); treat as zero
            // defensively so a misconfigured call doesn't accidentally enable thinking.
            ThinkingEffort::None | ThinkingEffort::Minimal => 0,
        }
    }

    /// Map level to an OpenAI `reasoning.effort` / `reasoning_effort` string.
    pub fn openai_effort_str(self) -> &'static str {
        match self {
            ThinkingEffort::None => "none",
            ThinkingEffort::Minimal => "minimal",
            ThinkingEffort::Low => "low",
            ThinkingEffort::Medium => "medium",
            ThinkingEffort::High => "high",
            ThinkingEffort::XHigh => "xhigh",
            ThinkingEffort::Max => "max",
        }
    }

    /// Map level to an Anthropic `output_config.effort` string.
    /// Returns the level string if supported, or the highest supported level for the model.
    /// Caller must apply model-level clamping via `clamp_for_anthropic_adaptive`.
    pub fn anthropic_effort_str(self) -> &'static str {
        match self {
            ThinkingEffort::Low => "low",
            ThinkingEffort::Medium => "medium",
            ThinkingEffort::High => "high",
            ThinkingEffort::XHigh => "xhigh",
            ThinkingEffort::Max => "max",
            // None/Minimal are rejected at startup for Anthropic; defensive fallback.
            ThinkingEffort::None | ThinkingEffort::Minimal => "low",
        }
    }
}
/// Resolve the nearest supported effort level for a given OpenAI model.
///
/// When the requested effort is not in the model's supported set, falls back to the
/// nearest supported level using this preference order:
///
/// - `none` ↔ `minimal` are each other's first fallback (the none/minimal split across
///   model families means the "closest analogue" is the other form before jumping to `low`).
/// - Above that pair: upward clamp first, then downward (prefer more thinking over less).
/// - `xhigh` falls back to `high` when not supported (no model skips from `high` to `xhigh`).
/// - `max` passes through for model families whose table includes it; otherwise it resolves to
///   the nearest supported level.
///
/// Logs a `warn!` on every substitution.
fn resolve_openai_effort(
    model: &str,
    requested: ThinkingEffort,
    supported: &[ThinkingEffort],
) -> ThinkingEffort {
    if supported.contains(&requested) {
        return requested;
    }

    // Build a candidate list ordered by preference: the "other" form of none/minimal first,
    // then the levels sorted nearest to requested (ascending distance).
    let candidates: Vec<ThinkingEffort> = {
        // none ↔ minimal are each other's first fallback.
        let peer = match requested {
            ThinkingEffort::None => Some(ThinkingEffort::Minimal),
            ThinkingEffort::Minimal => Some(ThinkingEffort::None),
            _ => None,
        };
        // All supported values sorted by distance (abs diff in ordinal), upward ties win.
        let mut by_dist: Vec<ThinkingEffort> = supported.to_vec();
        by_dist.sort_by_key(|&e| {
            let dist = (e as i32 - requested as i32).unsigned_abs();
            // Prefer upward (e > requested) to break ties between equidistant values.
            let up = if e >= requested { 0u32 } else { 1 };
            (dist, up)
        });
        // Peer first, then by distance.
        let mut result = Vec::new();
        if let Some(p) = peer {
            if supported.contains(&p) {
                result.push(p);
            }
        }
        for e in by_dist {
            if !result.contains(&e) {
                result.push(e);
            }
        }
        result
    };

    let resolved = candidates
        .into_iter()
        .next()
        .expect("supported is non-empty");

    tracing::warn!(
        %model,
        requested = requested.openai_effort_str(),
        resolved = resolved.openai_effort_str(),
        "BUZZ_AGENT_THINKING_EFFORT={} is not supported by this OpenAI model; using nearest supported level",
        requested.openai_effort_str(),
    );
    resolved
}

/// Normalize the effort value for an Anthropic-shaped request body (Messages API).
///
/// Anthropic-shaped bodies (`anthropic_body`) do not have a `none` or `minimal` concept —
/// the thinking block is either present (with a level) or absent. When `none` or `minimal`
/// is configured, we omit the thinking fields entirely and log a warning (omission = provider
/// default; default-on/always-on adaptive models may still think). This handles `DatabricksV2`
/// sessions where the route can switch from GPT to Claude via `session/set_model` after startup.
///
/// Returns `None` to signal "omit thinking fields", or the original effort if it is a valid
/// Anthropic level.
pub fn normalize_effort_for_anthropic_route(effort: ThinkingEffort) -> Option<ThinkingEffort> {
    match effort {
        ThinkingEffort::None | ThinkingEffort::Minimal => {
            tracing::warn!(
                requested = effort.openai_effort_str(),
                "BUZZ_AGENT_THINKING_EFFORT={} is not expressible as an Anthropic thinking level; \
                 omitting thinking fields (provider default; default-on/always-on adaptive models may still think)",
                effort.openai_effort_str()
            );
            None
        }
        other => Some(other),
    }
}

// ---------------------------------------------------------------------------
// Generated-backed production helpers (Phase 2 cutover, old tables retired in Phase 3)
//
// One adapter per request path — resolves the effective provider/model ONCE
// from the generated manifest and applies every manifest-owned axis.
// ---------------------------------------------------------------------------

/// Normalize the effort value for any OpenAI-shaped request (pure OpenAI or legacy Databricks).
///
/// Resolves the generated capability record for the actual provider/model and
/// applies `resolve_openai_effort` over its `supported_efforts`.  Uses the same
/// clamping/peer-fallback semantics as the old hand-table lookup but is driven by
/// the generated manifest, so exact-record F1 corrections (e.g. `databricks-gpt-5-5`
/// → `[low,medium,high]`) are enforced in production.
///
/// This is the single production authority for `Provider::OpenAi` and
/// `Provider::Databricks` effort normalization.
pub fn normalize_effort_for_provider(
    provider: &str,
    raw_model: &str,
    effort: ThinkingEffort,
) -> ThinkingEffort {
    use crate::generated_model_capabilities::resolve_model_capabilities;
    let cap = resolve_model_capabilities(provider, raw_model);
    resolve_openai_effort(raw_model, effort, cap.supported_efforts.as_ref())
}

/// Normalize the effort value for a DatabricksV2 request.
///
/// Reads `normalization_policy` from the generated capability record for this
/// raw model ID (provider = "databricks_v2") and applies it:
/// - `OpenAiStandard`      → per-family table lookup (GPT-5.x, etc.)
/// - `OpenAiClampMaxToXHigh` → clamp max → xhigh, pass others unchanged
/// - `None`                → pass effort through unchanged (Anthropic path)
///
/// This is the production authority for DatabricksV2 effort normalization.
/// `normalize_effort_for_provider` is the authority for pure OpenAI and legacy
/// Databricks effort normalization.
pub fn normalize_effort_for_databricks_v2(
    effort: ThinkingEffort,
    raw_model: &str,
) -> ThinkingEffort {
    use crate::generated_model_capabilities::{resolve_model_capabilities, NormalizationPolicy};
    let cap = resolve_model_capabilities("databricks_v2", raw_model);
    match cap.normalization_policy {
        NormalizationPolicy::OpenAiStandard => {
            // Resolve against the generated `supported_efforts` — this is the axis that
            // carries exact-record corrections (e.g. databricks-gpt-5-5 → [low,medium,high]).
            // Uses the same clamping/peer-fallback semantics as the old hand-table lookup.
            resolve_openai_effort(raw_model, effort, cap.supported_efforts.as_ref())
        }
        NormalizationPolicy::OpenAiClampMaxToXHigh => {
            // Only `max` is out-of-range; all other values pass through if supported.
            // Resolve against supported_efforts so that unsupported values are clamped
            // consistently (not just `max`).
            if effort == ThinkingEffort::Max {
                tracing::warn!(
                    requested = "max",
                    resolved = "xhigh",
                    model = raw_model,
                    "BUZZ_AGENT_THINKING_EFFORT=max not confirmed for this DatabricksV2 model; clamping to xhigh"
                );
                ThinkingEffort::XHigh
            } else {
                resolve_openai_effort(raw_model, effort, cap.supported_efforts.as_ref())
            }
        }
        NormalizationPolicy::None => effort,
    }
}

/// Build the Anthropic thinking/effort request fields for any manifest-owned provider/model.
///
/// Resolves `thinking_mode` and `supported_efforts` from the generated capability record
/// for the effective provider/model and applies them:
/// - `ManualBudget` → `thinking:{type:"enabled", budget_tokens}` shape
/// - `Adaptive`     → `thinking:{type:"adaptive"} + output_config:{effort}` shape,
///   with effort clamped down to the highest supported level
/// - `OmitFields` / `None` / `NotApplicable` → omit both fields
///
/// This is the single production authority for all providers' Anthropic thinking.
pub fn anthropic_thinking_config_generated(
    provider: &str,
    raw_model: &str,
    effort: ThinkingEffort,
    max_output_tokens: u32,
) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    use crate::generated_model_capabilities::{resolve_model_capabilities, ThinkingMode};
    use serde_json::json;

    let cap = resolve_model_capabilities(provider, raw_model);
    match cap.thinking_mode {
        ThinkingMode::ManualBudget => {
            // Manual-budget shape (claude-3*, claude-opus-4-5): budget_tokens clamped
            // to fit within max_output_tokens.
            const MIN_ANSWER_TOKENS: u32 = 1024;
            let level_budget = effort.anthropic_budget_tokens();
            let headroom = max_output_tokens.saturating_sub(MIN_ANSWER_TOKENS);
            let budget = level_budget.min(headroom);
            if budget < MIN_ANSWER_TOKENS {
                tracing::warn!(
                    max_output_tokens,
                    level_budget,
                    headroom,
                    model = raw_model,
                    "BUZZ_AGENT_THINKING_EFFORT: max_output_tokens too small to fit thinking budget + answer headroom; omitting thinking fields"
                );
                return (None, None);
            }
            (
                Some(json!({ "type": "enabled", "budget_tokens": budget })),
                None,
            )
        }
        ThinkingMode::Adaptive => {
            // Adaptive shape: clamp effort downward to the highest supported level
            // using the generated supported_efforts (the manifest-owned authority).
            let clamped = cap
                .supported_efforts
                .iter()
                .rev()
                .find(|&&e| e <= effort)
                .copied()
                .unwrap_or(effort); // effort is below the lowest supported; pass through (rare)
            if clamped != effort {
                tracing::warn!(
                    model = raw_model,
                    requested = effort.openai_effort_str(),
                    clamped = clamped.openai_effort_str(),
                    "BUZZ_AGENT_THINKING_EFFORT is not available for this model; clamping to highest supported level"
                );
            }
            (
                Some(json!({ "type": "adaptive" })),
                Some(json!({ "effort": clamped.anthropic_effort_str() })),
            )
        }
        ThinkingMode::OmitFields | ThinkingMode::None | ThinkingMode::NotApplicable => {
            // Unknown Anthropic model, non-Anthropic-routed, or not applicable:
            // omit thinking fields rather than guess.
            (None, None)
        }
    }
}
/// Parse `BUZZ_AGENT_THINKING_EFFORT`. Pure (env-free) for testability.
pub fn parse_thinking_effort(raw: Option<&str>) -> Result<Option<ThinkingEffort>, String> {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") => Ok(None),
        Some("none") => Ok(Some(ThinkingEffort::None)),
        Some("minimal") => Ok(Some(ThinkingEffort::Minimal)),
        Some("low") => Ok(Some(ThinkingEffort::Low)),
        Some("medium") => Ok(Some(ThinkingEffort::Medium)),
        Some("high") => Ok(Some(ThinkingEffort::High)),
        Some("xhigh") => Ok(Some(ThinkingEffort::XHigh)),
        Some("max") => Ok(Some(ThinkingEffort::Max)),
        Some(other) => Err(format!(
            "config: BUZZ_AGENT_THINKING_EFFORT={other} not supported (use none|minimal|low|medium|high|xhigh|max)"
        )),
    }
}

pub const MAX_PROMPT_BYTES: usize = 1024 * 1024;
pub const MAX_SYSTEM_PROMPT_BYTES: usize = 512 * 1024;
/// Total per-result byte ceiling (text + images). Sized for image-bearing
/// results — view_image can legitimately return multi-MiB base64 payloads.
/// Text is governed by the much smaller `BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES`.
pub const MAX_TOOL_RESULT_BYTES: usize = 8 * 1024 * 1024;
/// Default cap on the *text* portion of a single tool result. Oversized text
/// is middle-elided before it enters history; without this, one fat `cat`
/// burns the context window and forces a lossy handoff. 50 KiB matches the
/// shell-output caps in sprout-dev-mcp, goose, and pi; codex defaults to
/// 10 KB. Tunable via `BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES`.
pub const DEFAULT_TOOL_RESULT_TEXT_BYTES: usize = 50 * 1024;
pub const MAX_TOOL_CALLS_PER_TURN: usize = 64;

pub const HANDOFF_MAX_OUTPUT_TOKENS: u32 = 8192;

pub const HANDOFF_ORIGINAL_TASK_MAX_BYTES: usize = 16 * 1024;

pub const HANDOFF_MAX_TOOL_NAMES: usize = 20;

const DEFAULT_SYSTEM_PROMPT: &str =
    "You are buzz-agent. Use the provided tools to act. Tool calls are your only output.";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Provider {
    Anthropic,
    OpenAi,
    /// Databricks model serving. Routes to `{base_url}/serving-endpoints/{model}/invocations`
    /// with a dynamically-acquired bearer (OAuth 2.0 PKCE, or static `DATABRICKS_TOKEN`).
    /// Wire format is OpenAI-chat-compatible — reuses the same body builder and parser.
    Databricks,
    /// Databricks AI Gateway v2. Routes by model family through the gateway's
    /// OpenAI Responses, Anthropic Messages, or MLflow Chat Completions paths.
    DatabricksV2,
    /// OpenRouter multi-provider gateway. Routes to `{base_url}/chat/completions` with bearer auth. Wire format is OpenAI-chat-compatible.
    OpenRouter,
}

/// Which OpenAI-family HTTP API to call. Set via `OPENAI_COMPAT_API`
/// (`auto|chat|responses`); ignored when `provider = Anthropic`. `Auto`
/// picks Responses for `*.openai.com`, Chat Completions otherwise, and
/// permits a one-shot chat→responses upgrade on a "use /v1/responses"
/// provider error.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenAiApi {
    Chat,
    Responses,
    Auto,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: Provider,
    pub system_prompt: String,
    pub max_rounds: u32,
    pub max_output_tokens: u32,
    pub llm_timeout: Duration,
    pub tool_timeout: Duration,
    pub mcp_init_timeout: Duration,
    pub mcp_max_restart_attempts: u32,
    pub mcp_restart_base_ms: u64,
    pub mcp_restart_max_ms: u64,
    pub max_sessions: usize,
    pub max_line_bytes: usize,
    pub max_history_bytes: usize,
    /// Per-tool-result cap on text content. Oversized text is middle-elided
    /// (head + tail kept) before entering history. Images are exempt — they
    /// are bounded by [`MAX_TOOL_RESULT_BYTES`] and accounted separately.
    /// Set via `BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES`.
    pub max_tool_result_text_bytes: usize,
    /// Provider context window in tokens used to gate handoff. The handoff
    /// fires when the previous request's (cache-summed) input tokens cross the
    /// handoff threshold for this budget, before the next request can exceed
    /// the window and 400. Default 200_000 — matching Claude 4.x windows;
    /// operators lower/raise it for other models. Set via
    /// `BUZZ_AGENT_MAX_CONTEXT_TOKENS`.
    pub max_context_tokens: u64,
    pub max_handoffs: usize,
    pub max_parallel_tools: usize,
    pub hook_timeout: Duration,
    /// Maximum `_Stop` rejections per prompt. Default 3. Set to 0 to
    /// disable `_Stop` hooks entirely (agent always honors end_turn).
    pub stop_max_rejections: u32,
    /// Remind the model to publish when a turn is about to end without any
    /// recognized attempt to post to Buzz. Default off; opt in per agent with
    /// `BUZZ_AGENT_REQUIRE_REPLY=1`.
    ///
    /// Advisory only: at most `MAX_REPLY_NAGS` reminders (see `agent.rs`),
    /// then the turn ends regardless. Bounded by the same
    /// `stop_max_rejections` budget as `_Stop` hooks, which is the outer cap on
    /// all end-turn objections — at the default 3 both reminders fit; at 1 only
    /// one does; at 0 the guard is off with the hooks.
    pub require_reply: bool,
    /// Hook server allowlist. See [`HookServers`] for variant semantics.
    /// Default (env unset/empty) is `None` — hooks are off unless the
    /// operator explicitly opts in.
    pub hook_servers: HookServers,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub anthropic_api_version: String,
    /// OpenAI endpoint selection. See [`OpenAiApi`].
    pub openai_api: OpenAiApi,
    /// Prefer mesh-llm's virtual `mesh` model when the configured/effective
    /// OpenAI model is `auto` and the live model catalog advertises it.
    /// Set by Buzz's relay-mesh provider via
    /// `BUZZ_AGENT_PREFER_MESH_FOR_AUTO=1`; other providers keep their
    /// existing `auto` semantics.
    pub prefer_mesh_for_auto: bool,
    pub hints_enabled: bool,
    /// Thinking/reasoning effort level. `None` = use provider default (no
    /// thinking config sent). Set via `BUZZ_AGENT_THINKING_EFFORT`.
    pub thinking_effort: Option<ThinkingEffort>,
    /// Emit Anthropic `cache_control` breakpoints on the stable prefix
    /// (tools + system prompt) and the rolling conversation tail. Default on;
    /// disable with `BUZZ_AGENT_PROMPT_CACHING=0`. Consulted on every route that
    /// speaks the Anthropic caching dialect: first-party Anthropic, the
    /// DatabricksV2 Claude route, and OpenRouter's `anthropic/*` models. The
    /// Databricks gateway does not auto-cache, so without this the surfaced
    /// `cache_read_input_tokens` is structurally always 0.
    pub prompt_caching: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let databricks_host = env("DATABRICKS_HOST");
        let databricks_model = env("DATABRICKS_MODEL");
        let provider = resolve_provider(
            env("BUZZ_AGENT_PROVIDER").as_deref(),
            env("ANTHROPIC_API_KEY").as_deref(),
            env("OPENAI_COMPAT_API_KEY").as_deref(),
            env("OPENROUTER_API_KEY").as_deref(),
        )?;

        // Universal model override — takes priority over provider-specific model
        // env vars (ANTHROPIC_MODEL, OPENAI_COMPAT_MODEL, DATABRICKS_MODEL) when
        // present. Set by the desktop from the persona/record to express explicit
        // user intent; provider-specific vars serve as defaults for CLI/standalone use.
        let buzz_agent_model = env("BUZZ_AGENT_MODEL");

        // OPENAI_COMPAT_API is only read when provider=openai, so a stray
        // bad value can't break an Anthropic-only deployment.
        //
        // Databricks borrows api_key as the *optional* `DATABRICKS_TOKEN` escape
        // hatch — empty means "use OAuth PKCE." Legacy Databricks encodes the
        // model in the URL path; Databricks v2 keeps it in the request body.
        let (api_key, model, base_url, openai_api) = match provider {
            Provider::Anthropic => (
                req("ANTHROPIC_API_KEY")?,
                resolve_model(
                    buzz_agent_model.as_deref(),
                    env("ANTHROPIC_MODEL").as_deref(),
                )
                .ok_or_else(|| "config: ANTHROPIC_MODEL required".to_string())?,
                env_or("ANTHROPIC_BASE_URL", "https://api.anthropic.com"),
                OpenAiApi::Auto, // unused for Anthropic
            ),
            Provider::OpenAi => (
                req("OPENAI_COMPAT_API_KEY")?,
                resolve_model(
                    buzz_agent_model.as_deref(),
                    env("OPENAI_COMPAT_MODEL").as_deref(),
                )
                .ok_or_else(|| "config: OPENAI_COMPAT_MODEL required".to_string())?,
                env_or("OPENAI_COMPAT_BASE_URL", "https://api.openai.com/v1"),
                parse_openai_api(env("OPENAI_COMPAT_API").as_deref())?,
            ),
            Provider::Databricks | Provider::DatabricksV2 => (
                env("DATABRICKS_TOKEN").unwrap_or_default(),
                resolve_model(buzz_agent_model.as_deref(), databricks_model.as_deref())
                    .ok_or_else(|| "config: DATABRICKS_MODEL required".to_string())?,
                databricks_host.ok_or_else(|| "config: DATABRICKS_HOST required".to_string())?,
                OpenAiApi::Chat, // only read by OpenAI/legacy Databricks dispatch
            ),
            Provider::OpenRouter => (
                req("OPENROUTER_API_KEY")?,
                resolve_model(
                    buzz_agent_model.as_deref(),
                    env("OPENROUTER_MODEL").as_deref(),
                )
                .ok_or_else(|| "config: OPENROUTER_MODEL required".to_string())?,
                env_or("OPENROUTER_BASE_URL", "https://openrouter.ai/api/v1"),
                OpenAiApi::Chat, // OpenRouter uses Chat Completions only
            ),
        };
        let system_prompt = match (env("BUZZ_AGENT_SYSTEM_PROMPT"), env("BUZZ_AGENT_SYSTEM_PROMPT_FILE")) {
            (Some(_), Some(_)) => return Err(
                "config: BUZZ_AGENT_SYSTEM_PROMPT and BUZZ_AGENT_SYSTEM_PROMPT_FILE are mutually exclusive".into()),
            (Some(s), _) => s,
            (_, Some(p)) => std::fs::read_to_string(&p).map_err(|e| format!("config: read {p}: {e}"))?,
            _ => DEFAULT_SYSTEM_PROMPT.to_owned(),
        };
        let cfg = Config {
            provider,
            system_prompt,
            api_key,
            model,
            base_url,
            anthropic_api_version: env_or("ANTHROPIC_API_VERSION", "2023-06-01"),
            openai_api,
            prefer_mesh_for_auto: parse_env("BUZZ_AGENT_PREFER_MESH_FOR_AUTO", 0u8)? != 0,
            max_rounds: parse_env("BUZZ_AGENT_MAX_ROUNDS", 0)?,
            max_output_tokens: parse_env("BUZZ_AGENT_MAX_OUTPUT_TOKENS", 32_768)?,
            llm_timeout: Duration::from_secs(parse_env("BUZZ_AGENT_LLM_TIMEOUT_SECS", 240)?),
            tool_timeout: Duration::from_secs(parse_env("BUZZ_AGENT_TOOL_TIMEOUT_SECS", 660)?),
            mcp_init_timeout: Duration::from_secs(parse_env(
                "BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS",
                30,
            )?),
            mcp_max_restart_attempts: parse_env("BUZZ_AGENT_MCP_RESTART_MAX_ATTEMPTS", 3u32)?,
            mcp_restart_base_ms: parse_env("BUZZ_AGENT_MCP_RESTART_BASE_MS", 500u64)?,
            mcp_restart_max_ms: parse_env("BUZZ_AGENT_MCP_RESTART_MAX_MS", 30_000u64)?,
            max_sessions: parse_env("BUZZ_AGENT_MAX_SESSIONS", usize::MAX)?,
            max_line_bytes: parse_env("BUZZ_AGENT_MAX_LINE_BYTES", 4 * 1024 * 1024)?,
            max_history_bytes: parse_env("BUZZ_AGENT_MAX_HISTORY_BYTES", 16 * 1024 * 1024)?,
            max_tool_result_text_bytes: parse_env(
                "BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES",
                DEFAULT_TOOL_RESULT_TEXT_BYTES,
            )?,
            max_context_tokens: parse_env("BUZZ_AGENT_MAX_CONTEXT_TOKENS", 200_000u64)?,
            max_handoffs: parse_env("BUZZ_AGENT_MAX_HANDOFFS", 10)?,
            max_parallel_tools: parse_env("BUZZ_AGENT_MAX_PARALLEL_TOOLS", 8usize)?,
            hook_timeout: Duration::from_millis(parse_env("BUZZ_AGENT_HOOK_TIMEOUT_MS", 2500u64)?),
            stop_max_rejections: parse_env("BUZZ_AGENT_STOP_MAX_REJECTIONS", 3u32)?,
            require_reply: parse_env("BUZZ_AGENT_REQUIRE_REPLY", 0u8)? != 0,
            hook_servers: parse_hook_servers_env("MCP_HOOK_SERVERS"),
            hints_enabled: parse_env("BUZZ_AGENT_NO_HINTS", 0u8)? == 0,
            thinking_effort: parse_thinking_effort(env("BUZZ_AGENT_THINKING_EFFORT").as_deref())?,
            prompt_caching: parse_env("BUZZ_AGENT_PROMPT_CACHING", 1u8)? != 0,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Construct a minimal `Config` for model-catalog discovery.
    ///
    /// Only the fields used by [`build_token_source`](crate::llm::build_token_source)
    /// and the catalog HTTP helpers are meaningful; all others are set to
    /// inert defaults. Never call `from_env` for discovery — it requires
    /// `DATABRICKS_MODEL` and other fields that are irrelevant here.
    pub fn for_discovery(provider: Provider, api_key: String, base_url: String) -> Self {
        Self {
            provider,
            api_key,
            base_url,
            model: String::new(),
            system_prompt: String::new(),
            anthropic_api_version: "2023-06-01".into(),
            openai_api: OpenAiApi::Chat,
            prefer_mesh_for_auto: false,
            max_rounds: 0,
            max_output_tokens: 1,
            llm_timeout: Duration::from_secs(30),
            tool_timeout: Duration::from_secs(30),
            mcp_init_timeout: Duration::from_secs(30),
            mcp_max_restart_attempts: 0,
            mcp_restart_base_ms: 0,
            mcp_restart_max_ms: 0,
            max_sessions: 1,
            max_line_bytes: 4 * 1024 * 1024,
            max_history_bytes: 16 * 1024 * 1024,
            max_tool_result_text_bytes: 50 * 1024,
            max_context_tokens: 200_001,
            max_handoffs: 0,
            max_parallel_tools: 1,
            hook_timeout: Duration::from_secs(1),
            stop_max_rejections: 0,
            require_reply: false,
            hook_servers: HookServers::None,
            hints_enabled: false,
            thinking_effort: None,
            prompt_caching: false,
        }
    }

    fn validate(&self) -> Result<(), String> {
        const MIN_HISTORY_BYTES: usize = 4096;
        const MIN_LINE_BYTES: usize = 1024;
        const MIN_TOOL_RESULT_TEXT_BYTES: usize = 1024;
        const MIN_TIMEOUT: Duration = Duration::from_secs(1);

        if self.max_output_tokens < 1 {
            return Err("config: BUZZ_AGENT_MAX_OUTPUT_TOKENS must be >= 1".into());
        }
        if self.max_context_tokens <= u64::from(self.max_output_tokens) {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_CONTEXT_TOKENS ({}) must be > BUZZ_AGENT_MAX_OUTPUT_TOKENS ({}) — the context window must leave room for the response",
                self.max_context_tokens, self.max_output_tokens
            ));
        }
        if self.max_history_bytes < MIN_HISTORY_BYTES {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_HISTORY_BYTES must be >= {MIN_HISTORY_BYTES}"
            ));
        }
        if self.max_history_bytes < MAX_PROMPT_BYTES {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_HISTORY_BYTES ({}) must be >= MAX_PROMPT_BYTES ({MAX_PROMPT_BYTES})",
                self.max_history_bytes
            ));
        }
        if self.max_line_bytes < MIN_LINE_BYTES {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_LINE_BYTES must be >= {MIN_LINE_BYTES}"
            ));
        }
        if self.max_tool_result_text_bytes < MIN_TOOL_RESULT_TEXT_BYTES
            || self.max_tool_result_text_bytes > MAX_TOOL_RESULT_BYTES
        {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES must be in {MIN_TOOL_RESULT_TEXT_BYTES}..={MAX_TOOL_RESULT_BYTES}"
            ));
        }
        if self.llm_timeout < MIN_TIMEOUT {
            return Err("config: BUZZ_AGENT_LLM_TIMEOUT_SECS must be >= 1".into());
        }
        if self.tool_timeout < MIN_TIMEOUT {
            return Err("config: BUZZ_AGENT_TOOL_TIMEOUT_SECS must be >= 1".into());
        }
        if self.mcp_init_timeout < MIN_TIMEOUT {
            return Err("config: BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS must be >= 1".into());
        }
        if self.max_parallel_tools < 1 {
            return Err("config: BUZZ_AGENT_MAX_PARALLEL_TOOLS must be >= 1".into());
        }
        if self.mcp_max_restart_attempts < 1 {
            return Err("config: BUZZ_AGENT_MCP_RESTART_MAX_ATTEMPTS must be >= 1".into());
        }
        if self.mcp_restart_base_ms < 1 {
            return Err("config: BUZZ_AGENT_MCP_RESTART_BASE_MS must be >= 1".into());
        }
        if self.mcp_restart_max_ms < self.mcp_restart_base_ms {
            return Err(
                "config: BUZZ_AGENT_MCP_RESTART_MAX_MS must be >= BUZZ_AGENT_MCP_RESTART_BASE_MS"
                    .into(),
            );
        }
        // Provider-level effort validation (fail-fast, clear error).
        // `none`/`minimal` are not Anthropic values — rejected at startup.
        //
        // OpenAI, Databricks, and DatabricksV2 defer effort validation to request-time routing:
        // availability is model-dependent, and `session/set_model` can change the effective model
        // after startup. `normalize_effort_for_provider` / `normalize_effort_for_anthropic_route`
        // apply route-aware normalization in `llm.rs` when building each request.
        if let Some(effort) = self.thinking_effort {
            let is_pure_anthropic = matches!(self.provider, Provider::Anthropic);
            if is_pure_anthropic && matches!(effort, ThinkingEffort::None | ThinkingEffort::Minimal)
            {
                return Err(format!(
                    "config: BUZZ_AGENT_THINKING_EFFORT={} is not valid for Anthropic providers \
                     (allowed: low|medium|high|xhigh|max)",
                    effort.openai_effort_str()
                ));
            }
        }
        Ok(())
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn env_or(k: &str, d: &str) -> String {
    env(k).unwrap_or_else(|| d.into())
}

fn req(k: &str) -> Result<String, String> {
    env(k).ok_or_else(|| format!("config: {k} required"))
}

/// Returns the first present value. `explicit_override` (BUZZ_AGENT_MODEL,
/// set by the desktop from the persona/record) wins over `provider_default`
/// (provider-specific env var that may be inherited from the shell).
/// Returns `None` when both are absent so the caller can supply a
/// provider-specific error message.
fn resolve_model(
    explicit_override: Option<&str>,
    provider_default: Option<&str>,
) -> Option<String> {
    explicit_override.or(provider_default).map(str::to_owned)
}

fn present_nonempty(v: Option<&str>) -> bool {
    v.map(str::trim).is_some_and(|s| !s.is_empty())
}

fn resolve_provider(
    requested: Option<&str>,
    anthropic_key: Option<&str>,
    openai_key: Option<&str>,
    openrouter_key: Option<&str>,
) -> Result<Provider, String> {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => {
            let normalized = raw.to_ascii_lowercase();
            match normalized.as_str() {
                "anthropic" if present_nonempty(anthropic_key) => Ok(Provider::Anthropic),
                "anthropic" => Err(
                    "config: ANTHROPIC_API_KEY required".into(),
                ),
                "openai" | "openai-compat" if present_nonempty(openai_key) => Ok(Provider::OpenAi),
                "openai" | "openai-compat" => Err(
                    "config: OPENAI_COMPAT_API_KEY required".into(),
                ),
                "databricks" => Ok(Provider::Databricks),
                "databricks_v2" | "databricks-v2" => Ok(Provider::DatabricksV2),
                "openrouter" if present_nonempty(openrouter_key) => Ok(Provider::OpenRouter),
                "openrouter" => Err("config: OPENROUTER_API_KEY required".into()),
                _ => Err(format!(
                    "config: BUZZ_AGENT_PROVIDER={raw} not supported"
                )),
            }
        }
        None => Err(
            "config: BUZZ_AGENT_PROVIDER is required — set it to your provider (e.g. anthropic, openai, databricks)".into(),
        ),
    }
}

/// Parse `OPENAI_COMPAT_API`. Pure (env-free) for testability; the
/// caller hands in the raw value.
fn parse_openai_api(raw: Option<&str>) -> Result<OpenAiApi, String> {
    match raw.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "chat" | "chat-completions" | "chat_completions" => Ok(OpenAiApi::Chat),
        "responses" => Ok(OpenAiApi::Responses),
        "auto" | "" => Ok(OpenAiApi::Auto),
        other => Err(format!(
            "config: OPENAI_COMPAT_API={other} not supported (use auto|chat|responses)"
        )),
    }
}

/// `true` when `base_url` is an official OpenAI host. Hosts on
/// `*.openai.com` get Responses under `Auto`; everything else (vLLM,
/// Ollama, OpenRouter, Block Gateway, …) gets Chat Completions.
/// Lookalike-safe: `api.openai.com.evil.example` returns `false`.
pub fn is_openai_host(base_url: &str) -> bool {
    let rest = match base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))
    {
        Some(r) => r,
        None => return false,
    };
    let host = &rest[..rest.find(['/', ':']).unwrap_or(rest.len())];
    host == "api.openai.com" || host.ends_with(".openai.com")
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    env(key)
        .map(|v| v.parse().map_err(|e| format!("config: {key}: {e}")))
        .unwrap_or(Ok(default))
}

/// Hook-server allowlist parsed from a comma-separated env var.
///   - unset / empty / whitespace-only → `None` (no hooks enabled)
///   - `*`                              → `All` (every server eligible)
///   - `a,b,c`                          → `Only(["a","b","c"])`
#[derive(Debug, Clone)]
pub enum HookServers {
    None,
    All,
    Only(Vec<String>),
}

impl HookServers {
    /// Returns true iff `name` may receive hook calls.
    pub fn allows(&self, name: &str) -> bool {
        match self {
            HookServers::None => false,
            HookServers::All => true,
            HookServers::Only(v) => v.iter().any(|s| s == name),
        }
    }

    /// True if no hooks should ever fire — used to short-circuit dispatch.
    pub fn is_disabled(&self) -> bool {
        matches!(self, HookServers::None)
    }
}

fn parse_hook_servers_env(key: &str) -> HookServers {
    parse_hook_servers(env(key).as_deref())
}

/// Pure parser exposed for unit tests. `None` (env unset) and `Some("")`
/// (env set but empty) both yield `HookServers::None`.
fn parse_hook_servers(raw: Option<&str>) -> HookServers {
    let raw = match raw {
        Some(v) => v,
        None => return HookServers::None,
    };
    let names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        return HookServers::None;
    }
    // `*` is the wildcard — only honored when it's the sole entry. A mixed
    // value like "*,foo" falls through to `Only(["*","foo"])`; "*" is not a
    // legal MCP server name (it can't pass `valid_name`), so it never matches
    // an actual server. This avoids silently widening scope on typos.
    if names.len() == 1 && names[0] == "*" {
        return HookServers::All;
    }
    HookServers::Only(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_servers_unset_is_none() {
        assert!(matches!(parse_hook_servers(None), HookServers::None));
    }

    #[test]
    fn hook_servers_empty_string_is_none() {
        assert!(matches!(parse_hook_servers(Some("")), HookServers::None));
    }

    #[test]
    fn hook_servers_whitespace_only_is_none() {
        assert!(matches!(
            parse_hook_servers(Some("   ,, ,")),
            HookServers::None
        ));
    }

    #[test]
    fn hook_servers_star_is_all() {
        assert!(matches!(parse_hook_servers(Some("*")), HookServers::All));
    }

    #[test]
    fn hook_servers_star_with_whitespace_is_all() {
        assert!(matches!(
            parse_hook_servers(Some("  *  ")),
            HookServers::All
        ));
    }

    #[test]
    fn hook_servers_named_list() {
        match parse_hook_servers(Some("foo,bar")) {
            HookServers::Only(v) => assert_eq!(v, vec!["foo".to_owned(), "bar".to_owned()]),
            other => panic!("expected Only, got {other:?}"),
        }
    }

    #[test]
    fn hook_servers_trims_entries() {
        match parse_hook_servers(Some(" foo , bar , ")) {
            HookServers::Only(v) => assert_eq!(v, vec!["foo".to_owned(), "bar".to_owned()]),
            other => panic!("expected Only, got {other:?}"),
        }
    }

    #[test]
    fn hook_servers_star_mixed_is_literal() {
        // `*,foo` is NOT a wildcard — it's a literal Only(["*","foo"]).
        // No real server can be named `*`, so this never matches anything.
        match parse_hook_servers(Some("*,foo")) {
            HookServers::Only(v) => assert_eq!(v, vec!["*".to_owned(), "foo".to_owned()]),
            other => panic!("expected Only, got {other:?}"),
        }
    }

    #[test]
    fn hook_servers_allows_matches_named_only() {
        let hs = parse_hook_servers(Some("foo,bar"));
        assert!(hs.allows("foo"));
        assert!(hs.allows("bar"));
        assert!(!hs.allows("baz"));
    }

    #[test]
    fn hook_servers_allows_matches_all() {
        assert!(parse_hook_servers(Some("*")).allows("anything"));
    }

    #[test]
    fn hook_servers_allows_blocks_when_none() {
        assert!(!parse_hook_servers(None).allows("foo"));
    }

    #[test]
    fn hook_servers_star_mixed_does_not_match_real_server() {
        let hs = parse_hook_servers(Some("*,foo"));
        // The literal "*" entry exists in Only, but no real server can
        // be named "*" (rejected by the MCP server name validator).
        assert!(hs.allows("foo"));
        assert!(!hs.allows("bar"));
        // Allowed strictly only as a literal match — defense-in-depth
        // expectation for callers.
        assert!(hs.allows("*"));
    }

    #[test]
    fn parse_openai_api_values() {
        use OpenAiApi::*;
        for (raw, want) in [
            (None, Ok(Auto)),
            (Some("auto"), Ok(Auto)),
            (Some("  AUTO  "), Ok(Auto)),
            (Some(""), Ok(Auto)),
            (Some("chat"), Ok(Chat)),
            (Some("chat-completions"), Ok(Chat)),
            (Some("Responses"), Ok(Responses)),
        ] {
            assert_eq!(parse_openai_api(raw), want, "raw={raw:?}");
        }
        let err = parse_openai_api(Some("nope")).unwrap_err();
        assert!(err.contains("OPENAI_COMPAT_API=nope"), "{err}");
    }

    #[test]
    fn resolve_provider_keeps_requested_provider_when_token_present() {
        assert_eq!(
            resolve_provider(Some("anthropic"), Some("sk-ant"), None, None).unwrap(),
            Provider::Anthropic
        );
        assert_eq!(
            resolve_provider(Some("openai"), None, Some("sk-openai"), None).unwrap(),
            Provider::OpenAi
        );
    }

    #[test]
    fn resolve_provider_errors_when_requested_provider_key_missing() {
        // No fallback — missing key returns an error regardless of Databricks availability.
        let err = resolve_provider(Some("anthropic"), None, None, None).unwrap_err();
        assert!(err.contains("ANTHROPIC_API_KEY required"), "{err}");

        let err = resolve_provider(Some("openai-compat"), None, Some("   "), None).unwrap_err();
        assert!(err.contains("OPENAI_COMPAT_API_KEY required"), "{err}");
    }

    #[test]
    fn resolve_provider_errors_when_provider_env_absent() {
        // No implicit inference — absent BUZZ_AGENT_PROVIDER is an error.
        let err = resolve_provider(None, None, None, None).unwrap_err();
        assert!(err.contains("BUZZ_AGENT_PROVIDER is required"), "{err}");
    }

    #[test]
    fn resolve_provider_requires_databricks_host_and_model_for_fallback() {
        // Renamed: verify the explicit databricks provider path works correctly.
        // When BUZZ_AGENT_PROVIDER=databricks, resolve_provider succeeds regardless
        // of DATABRICKS_HOST/MODEL (those are validated later in from_env()).
        assert_eq!(
            resolve_provider(Some("databricks"), None, None, None).unwrap(),
            Provider::Databricks
        );
        // Missing key for other providers still errors — no Databricks fallback.
        let err = resolve_provider(Some("openai"), None, None, None).unwrap_err();
        assert!(err.contains("OPENAI_COMPAT_API_KEY required"), "{err}");
        let err = resolve_provider(None, None, None, None).unwrap_err();
        assert!(err.contains("BUZZ_AGENT_PROVIDER is required"), "{err}");
    }

    #[test]
    fn resolve_provider_unsupported_error_preserves_user_casing() {
        let err = resolve_provider(Some("OpenAIish"), None, None, None).unwrap_err();
        assert!(err.contains("BUZZ_AGENT_PROVIDER=OpenAIish"));
    }

    #[test]
    fn is_openai_host_matrix() {
        // Lookalike-safe: `api.openai.com.evil.example` and malformed URLs
        // are treated as non-OpenAI (which falls back to Chat Completions).
        for (url, want) in [
            ("https://api.openai.com/v1", true),
            ("https://api.openai.com", true),
            ("http://eu.api.openai.com/v1", true),
            ("http://localhost:11434/v1", false),
            ("https://openrouter.ai/api/v1", false),
            ("https://gateway.block.example/v1", false),
            ("https://api.openai.com.evil.example/v1", false),
            ("not a url", false),
        ] {
            assert_eq!(is_openai_host(url), want, "url={url}");
        }
    }

    #[test]
    fn resolve_model_prefers_explicit_override() {
        let result = resolve_model(Some("override-model"), Some("provider-model"));
        assert_eq!(result.as_deref(), Some("override-model"));
    }

    #[test]
    fn resolve_model_falls_back_to_provider_default() {
        let result = resolve_model(None, Some("provider-model"));
        assert_eq!(result.as_deref(), Some("provider-model"));
    }

    #[test]
    fn resolve_model_returns_none_when_both_absent() {
        let result = resolve_model(None, None);
        assert!(result.is_none());
    }

    #[test]
    fn parse_thinking_effort_round_trips_all_values() {
        for (raw, expected) in [
            ("none", ThinkingEffort::None),
            ("minimal", ThinkingEffort::Minimal),
            ("low", ThinkingEffort::Low),
            ("medium", ThinkingEffort::Medium),
            ("high", ThinkingEffort::High),
            ("xhigh", ThinkingEffort::XHigh),
            ("max", ThinkingEffort::Max),
        ] {
            assert_eq!(
                parse_thinking_effort(Some(raw)).unwrap(),
                Some(expected),
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn parse_thinking_effort_none_and_empty_yield_none() {
        assert_eq!(parse_thinking_effort(None).unwrap(), None);
        assert_eq!(parse_thinking_effort(Some("")).unwrap(), None);
        assert_eq!(parse_thinking_effort(Some("   ")).unwrap(), None);
    }

    #[test]
    fn parse_thinking_effort_is_case_insensitive() {
        assert_eq!(
            parse_thinking_effort(Some("HIGH")).unwrap(),
            Some(ThinkingEffort::High)
        );
        assert_eq!(
            parse_thinking_effort(Some("  Medium  ")).unwrap(),
            Some(ThinkingEffort::Medium)
        );
    }

    #[test]
    fn parse_thinking_effort_rejects_unknown_value() {
        let err = parse_thinking_effort(Some("extreme")).unwrap_err();
        assert!(err.contains("BUZZ_AGENT_THINKING_EFFORT=extreme"), "{err}");
        assert!(
            err.contains("none|minimal|low|medium|high|xhigh|max"),
            "{err}"
        );
    }

    #[test]
    fn thinking_effort_anthropic_budget_tokens_mapping() {
        assert_eq!(ThinkingEffort::Low.anthropic_budget_tokens(), 1_024);
        assert_eq!(ThinkingEffort::Medium.anthropic_budget_tokens(), 8_192);
        assert_eq!(ThinkingEffort::High.anthropic_budget_tokens(), 32_768);
        // XHigh and Max clamp to the high budget value for manual-budget models.
        assert_eq!(ThinkingEffort::XHigh.anthropic_budget_tokens(), 32_768);
        assert_eq!(ThinkingEffort::Max.anthropic_budget_tokens(), 32_768);
        // None/Minimal are rejected at startup for Anthropic; defensive zero.
        assert_eq!(ThinkingEffort::None.anthropic_budget_tokens(), 0);
        assert_eq!(ThinkingEffort::Minimal.anthropic_budget_tokens(), 0);
    }

    #[test]
    fn thinking_effort_openai_effort_str_mapping() {
        assert_eq!(ThinkingEffort::None.openai_effort_str(), "none");
        assert_eq!(ThinkingEffort::Minimal.openai_effort_str(), "minimal");
        assert_eq!(ThinkingEffort::Low.openai_effort_str(), "low");
        assert_eq!(ThinkingEffort::Medium.openai_effort_str(), "medium");
        assert_eq!(ThinkingEffort::High.openai_effort_str(), "high");
        assert_eq!(ThinkingEffort::XHigh.openai_effort_str(), "xhigh");
        assert_eq!(ThinkingEffort::Max.openai_effort_str(), "max");
    }

    #[test]
    fn thinking_effort_anthropic_effort_str_mapping() {
        assert_eq!(ThinkingEffort::Low.anthropic_effort_str(), "low");
        assert_eq!(ThinkingEffort::Medium.anthropic_effort_str(), "medium");
        assert_eq!(ThinkingEffort::High.anthropic_effort_str(), "high");
        assert_eq!(ThinkingEffort::XHigh.anthropic_effort_str(), "xhigh");
        assert_eq!(ThinkingEffort::Max.anthropic_effort_str(), "max");
        // Defensive fallback for invalid Anthropic values (caught at startup validation).
        assert_eq!(ThinkingEffort::None.anthropic_effort_str(), "low");
        assert_eq!(ThinkingEffort::Minimal.anthropic_effort_str(), "low");
    }

    #[test]
    fn thinking_effort_ord_ordering() {
        // PartialOrd/Ord must reflect the ordered hierarchy.
        assert!(ThinkingEffort::None < ThinkingEffort::Minimal);
        assert!(ThinkingEffort::Minimal < ThinkingEffort::Low);
        assert!(ThinkingEffort::Low < ThinkingEffort::Medium);
        assert!(ThinkingEffort::Medium < ThinkingEffort::High);
        assert!(ThinkingEffort::High < ThinkingEffort::XHigh);
        assert!(ThinkingEffort::XHigh < ThinkingEffort::Max);
    }
    // ---- normalize_effort_for_databricks_v2 regression tests (F1 corrections) ----
    // These pin the exact behavior Paul's pre-review probes checked. The key invariant:
    // normalize_effort_for_databricks_v2 must resolve against the generated supported_efforts
    // (which carries exact-record F1 corrections), NOT the old hand table.

    #[test]
    fn normalize_effort_for_databricks_v2_gpt_5_5_xhigh_clamps_to_high() {
        // F1 correction: databricks-gpt-5-5 generated supported_efforts = [low, medium, high].
        // XHigh is outside the supported set → nearest supported is High.
        assert_eq!(
            normalize_effort_for_databricks_v2(ThinkingEffort::XHigh, "databricks-gpt-5-5"),
            ThinkingEffort::High,
            "databricks-gpt-5-5 XHigh must clamp to High (F1 correction: supported=[low,medium,high])"
        );
    }

    #[test]
    fn normalize_effort_for_databricks_v2_gpt_5_5_none_clamps_to_low() {
        // F1 correction: databricks-gpt-5-5 supported_efforts = [low, medium, high].
        // None is outside the set → nearest supported is Low.
        assert_eq!(
            normalize_effort_for_databricks_v2(ThinkingEffort::None, "databricks-gpt-5-5"),
            ThinkingEffort::Low,
            "databricks-gpt-5-5 None must clamp to Low (F1 correction: supported=[low,medium,high])"
        );
    }

    #[test]
    fn normalize_effort_for_databricks_v2_gpt_5_5_in_range_passes_through() {
        // Values within the corrected set must pass through unchanged.
        for effort in [
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
        ] {
            assert_eq!(
                normalize_effort_for_databricks_v2(effort, "databricks-gpt-5-5"),
                effort,
                "databricks-gpt-5-5 {effort:?} is in supported set, must pass through"
            );
        }
    }

    #[test]
    fn normalize_effort_for_databricks_v2_gpt_5_6_sol_max_passes_through() {
        // databricks-gpt-5-6-sol F1 adoption: [low, medium, high, max] — max is supported.
        assert_eq!(
            normalize_effort_for_databricks_v2(ThinkingEffort::Max, "databricks-gpt-5-6-sol"),
            ThinkingEffort::Max,
            "databricks-gpt-5-6-sol Max must pass through (F1: supported includes max)"
        );
    }

    #[test]
    fn resolve_provider_openrouter_with_key() {
        assert_eq!(
            resolve_provider(Some("openrouter"), None, None, Some("sk-or-123")).unwrap(),
            Provider::OpenRouter
        );
    }

    #[test]
    fn resolve_provider_openrouter_missing_key() {
        let err = resolve_provider(Some("openrouter"), None, None, None).unwrap_err();
        assert!(err.contains("OPENROUTER_API_KEY"));
    }

    // ---- Config::validate() — Anthropic effort gate ----
    // Pinned behavioral coverage for the pure-Anthropic none|minimal rejection
    // at config.rs:672-681. This gate is reached from Config::from_env() at :558.
    // A regression (e.g. predicate accidentally inverted) makes these tests fail
    // while the cargo-test suite remains otherwise green.

    /// Minimal Config that passes all validate() invariants (Anthropic provider).
    /// Tests set only the fields under scrutiny before calling validate().
    fn cfg_anthropic_for_validate() -> Config {
        Config {
            provider: Provider::Anthropic,
            system_prompt: String::new(),
            api_key: "sk-ant-key".into(),
            model: "claude-opus-4-8".into(),
            base_url: "https://api.anthropic.com".into(),
            anthropic_api_version: "2023-06-01".into(),
            openai_api: OpenAiApi::Auto,
            prefer_mesh_for_auto: false,
            max_rounds: 10,
            max_output_tokens: 32_768,
            llm_timeout: Duration::from_secs(10),
            tool_timeout: Duration::from_secs(10),
            mcp_init_timeout: Duration::from_secs(10),
            mcp_max_restart_attempts: 1,
            mcp_restart_base_ms: 100,
            mcp_restart_max_ms: 1_000,
            max_sessions: 1,
            max_line_bytes: 4 * 1024 * 1024,
            max_history_bytes: 16 * 1024 * 1024,
            max_tool_result_text_bytes: 50 * 1024,
            max_context_tokens: 200_000,
            max_handoffs: 1,
            max_parallel_tools: 1,
            hook_timeout: Duration::from_secs(1),
            stop_max_rejections: 0,
            require_reply: false,
            hook_servers: HookServers::None,
            hints_enabled: false,
            thinking_effort: None,
            prompt_caching: false,
        }
    }

    #[test]
    fn validate_rejects_none_and_minimal_for_pure_anthropic() {
        for effort in [ThinkingEffort::None, ThinkingEffort::Minimal] {
            let mut cfg = cfg_anthropic_for_validate();
            cfg.thinking_effort = Some(effort);
            let err = cfg.validate().unwrap_err();
            assert!(
                err.contains("not valid for Anthropic providers"),
                "effort={effort:?}: expected rejection, got {err:?}"
            );
        }
    }

    #[test]
    fn validate_accepts_valid_efforts_for_pure_anthropic() {
        for effort in [
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ] {
            let mut cfg = cfg_anthropic_for_validate();
            cfg.thinking_effort = Some(effort);
            assert!(
                cfg.validate().is_ok(),
                "effort={effort:?}: expected Ok, got error"
            );
        }
    }

    // ---- normalize_effort_for_anthropic_route() matrix ----
    // Pinned coverage for the none|minimal → None mapping at config.rs:151-164.
    // Used by the DBv2 Anthropic arm (llm.rs:216). A regression that lets
    // none/minimal reach the wire as an Anthropic effort level fails these tests.

    #[test]
    fn normalize_effort_for_anthropic_route_omits_none_and_minimal() {
        assert_eq!(
            normalize_effort_for_anthropic_route(ThinkingEffort::None),
            None
        );
        assert_eq!(
            normalize_effort_for_anthropic_route(ThinkingEffort::Minimal),
            None
        );
    }

    #[test]
    fn normalize_effort_for_anthropic_route_passes_through_valid_levels() {
        for effort in [
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ] {
            assert_eq!(
                normalize_effort_for_anthropic_route(effort),
                Some(effort),
                "effort={effort:?}: expected Some({effort:?}), got None"
            );
        }
    }
}
