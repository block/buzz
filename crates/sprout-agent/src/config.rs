use std::{collections::HashMap, path::PathBuf, time::Duration};

pub const PROTOCOL_VERSION: u32 = 1;

pub const MAX_PROMPT_BYTES: usize = 1024 * 1024;
pub const MAX_TOOL_RESULT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOOL_CALLS_PER_TURN: usize = 64;

pub const HANDOFF_MAX_OUTPUT_TOKENS: u32 = 8192;

pub const HANDOFF_TAIL_ITEMS: usize = 5;

pub const HANDOFF_ORIGINAL_TASK_MAX_BYTES: usize = 16 * 1024;

pub const HANDOFF_PROMPT_MAX_BYTES: usize = 32 * 1024;

pub const HANDOFF_MAX_TOOL_NAMES: usize = 20;

const DEFAULT_SYSTEM_PROMPT: &str =
    "You are sprout-agent. Use the provided tools to act. Tool calls are your only output.";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Provider {
    Anthropic,
    OpenAi,
    /// Databricks model serving. Routes to `{base_url}/serving-endpoints/{model}/invocations`
    /// with a dynamically-acquired bearer (OAuth 2.0 PKCE, or static `DATABRICKS_TOKEN`).
    /// Wire format is OpenAI-chat-compatible — reuses the same body builder and parser.
    Databricks,
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
    /// Provider context window in tokens used to gate handoff. The handoff
    /// fires when the previous request's (cache-summed) input tokens cross the
    /// handoff threshold for this budget, before the next request can exceed
    /// the window and 400. Default 200_000 — matching Claude 4.x windows;
    /// operators lower/raise it for other models. Set via
    /// `SPROUT_AGENT_MAX_CONTEXT_TOKENS`.
    pub max_context_tokens: u64,
    pub max_handoffs: usize,
    pub max_parallel_tools: usize,
    pub hook_timeout: Duration,
    /// Maximum `_Stop` rejections per session. Default 3. Set to 0 to
    /// disable `_Stop` hooks entirely (agent always honors end_turn).
    pub stop_max_rejections: u32,
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
    pub hints_enabled: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let goose_databricks = GooseDatabricksConfig::load_default();
        let databricks_host = env("DATABRICKS_HOST").or_else(|| goose_databricks.host.clone());
        let databricks_model = env("DATABRICKS_MODEL").or_else(|| goose_databricks.model.clone());
        let provider = resolve_provider(
            env("SPROUT_AGENT_PROVIDER").as_deref(),
            env("ANTHROPIC_API_KEY").as_deref(),
            env("OPENAI_COMPAT_API_KEY").as_deref(),
            databricks_host.as_deref(),
            databricks_model.as_deref(),
        )?;
        // OPENAI_COMPAT_API is only read when provider=openai, so a stray
        // bad value can't break an Anthropic-only deployment.
        //
        // Databricks borrows api_key as the *optional* `DATABRICKS_TOKEN` escape
        // hatch — empty means "use OAuth PKCE." The model lives in the URL path,
        // not the request body (see `EndpointStrategy::DatabricksServing`).
        let (api_key, model, base_url, openai_api) = match provider {
            Provider::Anthropic => (
                req("ANTHROPIC_API_KEY")?,
                req("ANTHROPIC_MODEL")?,
                env_or("ANTHROPIC_BASE_URL", "https://api.anthropic.com"),
                OpenAiApi::Auto, // unused for Anthropic
            ),
            Provider::OpenAi => (
                req("OPENAI_COMPAT_API_KEY")?,
                req("OPENAI_COMPAT_MODEL")?,
                env_or("OPENAI_COMPAT_BASE_URL", "https://api.openai.com/v1"),
                parse_openai_api(env("OPENAI_COMPAT_API").as_deref())?,
            ),
            Provider::Databricks => (
                env("DATABRICKS_TOKEN").unwrap_or_default(),
                databricks_model.ok_or_else(|| {
                    "config: DATABRICKS_MODEL required (or set GOOSE_MODEL in goose config with GOOSE_PROVIDER=databricks)".to_string()
                })?,
                databricks_host.ok_or_else(|| {
                    "config: DATABRICKS_HOST required (or set DATABRICKS_HOST in goose config)".to_string()
                })?,
                OpenAiApi::Chat, // Databricks invocations is chat-shaped
            ),
        };
        let system_prompt = match (env("SPROUT_AGENT_SYSTEM_PROMPT"), env("SPROUT_AGENT_SYSTEM_PROMPT_FILE")) {
            (Some(_), Some(_)) => return Err(
                "config: SPROUT_AGENT_SYSTEM_PROMPT and SPROUT_AGENT_SYSTEM_PROMPT_FILE are mutually exclusive".into()),
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
            max_rounds: parse_env("SPROUT_AGENT_MAX_ROUNDS", 0)?,
            max_output_tokens: parse_env("SPROUT_AGENT_MAX_OUTPUT_TOKENS", 32_768)?,
            llm_timeout: Duration::from_secs(parse_env("SPROUT_AGENT_LLM_TIMEOUT_SECS", 120)?),
            tool_timeout: Duration::from_secs(parse_env("SPROUT_AGENT_TOOL_TIMEOUT_SECS", 660)?),
            mcp_init_timeout: Duration::from_secs(parse_env(
                "SPROUT_AGENT_MCP_INIT_TIMEOUT_SECS",
                30,
            )?),
            mcp_max_restart_attempts: parse_env("SPROUT_AGENT_MCP_RESTART_MAX_ATTEMPTS", 3u32)?,
            mcp_restart_base_ms: parse_env("SPROUT_AGENT_MCP_RESTART_BASE_MS", 500u64)?,
            mcp_restart_max_ms: parse_env("SPROUT_AGENT_MCP_RESTART_MAX_MS", 30_000u64)?,
            max_sessions: parse_env("SPROUT_AGENT_MAX_SESSIONS", usize::MAX)?,
            max_line_bytes: parse_env("SPROUT_AGENT_MAX_LINE_BYTES", 4 * 1024 * 1024)?,
            max_history_bytes: parse_env("SPROUT_AGENT_MAX_HISTORY_BYTES", 16 * 1024 * 1024)?,
            max_context_tokens: parse_env("SPROUT_AGENT_MAX_CONTEXT_TOKENS", 200_000u64)?,
            max_handoffs: parse_env("SPROUT_AGENT_MAX_HANDOFFS", 10)?,
            max_parallel_tools: parse_env("SPROUT_AGENT_MAX_PARALLEL_TOOLS", 8usize)?,
            hook_timeout: Duration::from_millis(parse_env(
                "SPROUT_AGENT_HOOK_TIMEOUT_MS",
                2500u64,
            )?),
            stop_max_rejections: parse_env("SPROUT_AGENT_STOP_MAX_REJECTIONS", 3u32)?,
            hook_servers: parse_hook_servers_env("MCP_HOOK_SERVERS"),
            hints_enabled: parse_env("SPROUT_AGENT_NO_HINTS", 0u8)? == 0,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), String> {
        const MIN_HISTORY_BYTES: usize = 4096;
        const MIN_LINE_BYTES: usize = 1024;
        const MIN_TIMEOUT: Duration = Duration::from_secs(1);

        if self.max_output_tokens < 1 {
            return Err("config: SPROUT_AGENT_MAX_OUTPUT_TOKENS must be >= 1".into());
        }
        if self.max_context_tokens <= u64::from(self.max_output_tokens) {
            return Err(format!(
                "config: SPROUT_AGENT_MAX_CONTEXT_TOKENS ({}) must be > SPROUT_AGENT_MAX_OUTPUT_TOKENS ({}) — the context window must leave room for the response",
                self.max_context_tokens, self.max_output_tokens
            ));
        }
        if self.max_history_bytes < MIN_HISTORY_BYTES {
            return Err(format!(
                "config: SPROUT_AGENT_MAX_HISTORY_BYTES must be >= {MIN_HISTORY_BYTES}"
            ));
        }
        if self.max_history_bytes < MAX_PROMPT_BYTES {
            return Err(format!(
                "config: SPROUT_AGENT_MAX_HISTORY_BYTES ({}) must be >= MAX_PROMPT_BYTES ({MAX_PROMPT_BYTES})",
                self.max_history_bytes
            ));
        }
        if self.max_line_bytes < MIN_LINE_BYTES {
            return Err(format!(
                "config: SPROUT_AGENT_MAX_LINE_BYTES must be >= {MIN_LINE_BYTES}"
            ));
        }
        if self.llm_timeout < MIN_TIMEOUT {
            return Err("config: SPROUT_AGENT_LLM_TIMEOUT_SECS must be >= 1".into());
        }
        if self.tool_timeout < MIN_TIMEOUT {
            return Err("config: SPROUT_AGENT_TOOL_TIMEOUT_SECS must be >= 1".into());
        }
        if self.mcp_init_timeout < MIN_TIMEOUT {
            return Err("config: SPROUT_AGENT_MCP_INIT_TIMEOUT_SECS must be >= 1".into());
        }
        if self.max_parallel_tools < 1 {
            return Err("config: SPROUT_AGENT_MAX_PARALLEL_TOOLS must be >= 1".into());
        }
        if self.mcp_max_restart_attempts < 1 {
            return Err("config: SPROUT_AGENT_MCP_RESTART_MAX_ATTEMPTS must be >= 1".into());
        }
        if self.mcp_restart_base_ms < 1 {
            return Err("config: SPROUT_AGENT_MCP_RESTART_BASE_MS must be >= 1".into());
        }
        if self.mcp_restart_max_ms < self.mcp_restart_base_ms {
            return Err(
                "config: SPROUT_AGENT_MCP_RESTART_MAX_MS must be >= SPROUT_AGENT_MCP_RESTART_BASE_MS".into(),
            );
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

#[derive(Default)]
struct GooseDatabricksConfig {
    host: Option<String>,
    model: Option<String>,
}

impl GooseDatabricksConfig {
    fn load_default() -> Self {
        goose_config_path()
            .and_then(|p| Self::load_from_path(&p))
            .unwrap_or_default()
    }

    fn load_from_path(path: &std::path::Path) -> Option<Self> {
        let raw = std::fs::read_to_string(path).ok()?;
        let map: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(&raw).ok()?;
        Some(Self::from_map(&map))
    }

    fn from_map(map: &HashMap<String, serde_yaml::Value>) -> Self {
        let host = yaml_string(map, "DATABRICKS_HOST");
        let explicit_model = yaml_string(map, "DATABRICKS_MODEL");
        let goose_provider = yaml_string(map, "GOOSE_PROVIDER");
        let goose_model = yaml_string(map, "GOOSE_MODEL");
        let goose_mode = yaml_string(map, "GOOSE_MODE");
        let model = explicit_model.or_else(|| {
            if goose_provider
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("databricks"))
            {
                goose_model.or(goose_mode)
            } else {
                None
            }
        });
        Self { host, model }
    }
}

fn yaml_string(map: &HashMap<String, serde_yaml::Value>, key: &str) -> Option<String> {
    map.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn goose_config_path() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("GOOSE_PATH_ROOT") {
        return Some(PathBuf::from(root).join("config").join("config.yaml"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("goose")
            .join("config.yaml"),
    )
}

fn present_nonempty(v: Option<&str>) -> bool {
    v.map(str::trim).is_some_and(|s| !s.is_empty())
}

fn databricks_available(host: Option<&str>, model: Option<&str>) -> bool {
    present_nonempty(host) && present_nonempty(model)
}

fn resolve_provider(
    requested: Option<&str>,
    anthropic_key: Option<&str>,
    openai_key: Option<&str>,
    databricks_host: Option<&str>,
    databricks_model: Option<&str>,
) -> Result<Provider, String> {
    let databricks_ready = databricks_available(databricks_host, databricks_model);
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => {
            let normalized = raw.to_ascii_lowercase();
            match normalized.as_str() {
                "anthropic" if present_nonempty(anthropic_key) => Ok(Provider::Anthropic),
                "anthropic" if databricks_ready => {
                    tracing::warn!(
                        requested = raw,
                        "API key missing for requested provider; falling back to Databricks OAuth"
                    );
                    Ok(Provider::Databricks)
                }
                "anthropic" => Err(
                    "config: ANTHROPIC_API_KEY required (or set DATABRICKS_HOST and DATABRICKS_MODEL for Databricks OAuth fallback)".into(),
                ),
                "openai" | "openai-compat" if present_nonempty(openai_key) => Ok(Provider::OpenAi),
                "openai" | "openai-compat" if databricks_ready => {
                    tracing::warn!(
                        requested = raw,
                        "API key missing for requested provider; falling back to Databricks OAuth"
                    );
                    Ok(Provider::Databricks)
                }
                "openai" | "openai-compat" => Err(
                    "config: OPENAI_COMPAT_API_KEY required (or set DATABRICKS_HOST and DATABRICKS_MODEL for Databricks OAuth fallback)".into(),
                ),
                "databricks" => Ok(Provider::Databricks),
                _ => Err(format!(
                    "config: SPROUT_AGENT_PROVIDER={raw} not supported"
                )),
            }
        }
        None if databricks_ready => Ok(Provider::Databricks),
        None => Err(
            "config: SPROUT_AGENT_PROVIDER required (or set DATABRICKS_HOST and DATABRICKS_MODEL for Databricks OAuth fallback)".into(),
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
    fn goose_databricks_config_reads_host_and_model() {
        let map = HashMap::from([
            (
                "DATABRICKS_HOST".to_string(),
                serde_yaml::Value::String("https://dbc.example".into()),
            ),
            (
                "GOOSE_PROVIDER".to_string(),
                serde_yaml::Value::String("databricks".into()),
            ),
            (
                "GOOSE_MODEL".to_string(),
                serde_yaml::Value::String("goose-claude-4-6-sonnet".into()),
            ),
        ]);
        let cfg = GooseDatabricksConfig::from_map(&map);
        assert_eq!(cfg.host.as_deref(), Some("https://dbc.example"));
        assert_eq!(cfg.model.as_deref(), Some("goose-claude-4-6-sonnet"));
    }

    #[test]
    fn goose_databricks_config_prefers_explicit_databricks_model() {
        let map = HashMap::from([
            (
                "DATABRICKS_HOST".to_string(),
                serde_yaml::Value::String("https://dbc.example".into()),
            ),
            (
                "DATABRICKS_MODEL".to_string(),
                serde_yaml::Value::String("explicit-db-model".into()),
            ),
            (
                "GOOSE_PROVIDER".to_string(),
                serde_yaml::Value::String("databricks".into()),
            ),
            (
                "GOOSE_MODEL".to_string(),
                serde_yaml::Value::String("goose-model".into()),
            ),
        ]);
        let cfg = GooseDatabricksConfig::from_map(&map);
        assert_eq!(cfg.model.as_deref(), Some("explicit-db-model"));
    }

    #[test]
    fn goose_databricks_config_ignores_goose_model_for_other_provider() {
        let map = HashMap::from([
            (
                "DATABRICKS_HOST".to_string(),
                serde_yaml::Value::String("https://dbc.example".into()),
            ),
            (
                "GOOSE_PROVIDER".to_string(),
                serde_yaml::Value::String("anthropic".into()),
            ),
            (
                "GOOSE_MODEL".to_string(),
                serde_yaml::Value::String("claude".into()),
            ),
        ]);
        let cfg = GooseDatabricksConfig::from_map(&map);
        assert_eq!(cfg.host.as_deref(), Some("https://dbc.example"));
        assert!(cfg.model.is_none());
    }

    #[test]
    fn resolve_provider_keeps_requested_provider_when_token_present() {
        assert_eq!(
            resolve_provider(
                Some("anthropic"),
                Some("sk-ant"),
                None,
                Some("https://dbc.example"),
                Some("db-model")
            )
            .unwrap(),
            Provider::Anthropic
        );
        assert_eq!(
            resolve_provider(
                Some("openai"),
                None,
                Some("sk-openai"),
                Some("https://dbc.example"),
                Some("db-model")
            )
            .unwrap(),
            Provider::OpenAi
        );
    }

    #[test]
    fn resolve_provider_falls_back_to_databricks_when_requested_token_missing() {
        assert_eq!(
            resolve_provider(
                Some("anthropic"),
                None,
                None,
                Some("https://dbc.example"),
                Some("goose-claude-4-6-sonnet")
            )
            .unwrap(),
            Provider::Databricks
        );
        assert_eq!(
            resolve_provider(
                Some("openai-compat"),
                None,
                Some("   "),
                Some("https://dbc.example"),
                Some("goose-claude-4-6-sonnet")
            )
            .unwrap(),
            Provider::Databricks
        );
    }

    #[test]
    fn resolve_provider_can_auto_select_databricks_without_explicit_provider() {
        assert_eq!(
            resolve_provider(
                None,
                None,
                None,
                Some("https://dbc.example"),
                Some("goose-claude-4-6-sonnet")
            )
            .unwrap(),
            Provider::Databricks
        );
    }

    #[test]
    fn resolve_provider_requires_databricks_host_and_model_for_fallback() {
        let err = resolve_provider(
            Some("openai"),
            None,
            None,
            Some("https://dbc.example"),
            None,
        )
        .unwrap_err();
        assert!(err.contains("OPENAI_COMPAT_API_KEY required"));
        let err =
            resolve_provider(None, None, None, Some("https://dbc.example"), None).unwrap_err();
        assert!(err.contains("SPROUT_AGENT_PROVIDER required"));
    }

    #[test]
    fn resolve_provider_unsupported_error_preserves_user_casing() {
        let err = resolve_provider(Some("OpenAIish"), None, None, None, None).unwrap_err();
        assert!(err.contains("SPROUT_AGENT_PROVIDER=OpenAIish"));
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
}
