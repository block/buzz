use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 1;

pub const MAX_PROMPT_BYTES: usize = 1024 * 1024;
pub const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;
pub const MAX_TOOL_CALLS_PER_TURN: usize = 64;

/// Leaves headroom for the summary call.
pub const HANDOFF_THRESHOLD: f64 = 0.75;

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
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let provider = match req("SPROUT_AGENT_PROVIDER")?.to_ascii_lowercase().as_str() {
            "anthropic" => Provider::Anthropic,
            "openai" | "openai-compat" => Provider::OpenAi,
            o => return Err(format!("config: SPROUT_AGENT_PROVIDER={o} not supported")),
        };
        let (api_key, model, base_url) = match provider {
            Provider::Anthropic => (
                req("ANTHROPIC_API_KEY")?,
                req("ANTHROPIC_MODEL")?,
                env_or("ANTHROPIC_BASE_URL", "https://api.anthropic.com"),
            ),
            Provider::OpenAi => (
                req("OPENAI_COMPAT_API_KEY")?,
                req("OPENAI_COMPAT_MODEL")?,
                env_or("OPENAI_COMPAT_BASE_URL", "https://api.openai.com/v1"),
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
            max_history_bytes: parse_env("SPROUT_AGENT_MAX_HISTORY_BYTES", 1024 * 1024)?,
            max_handoffs: parse_env("SPROUT_AGENT_MAX_HANDOFFS", 5)?,
            max_parallel_tools: parse_env("SPROUT_AGENT_MAX_PARALLEL_TOOLS", 8usize)?,
            hook_timeout: Duration::from_millis(parse_env(
                "SPROUT_AGENT_HOOK_TIMEOUT_MS",
                2500u64,
            )?),
            stop_max_rejections: parse_env("SPROUT_AGENT_STOP_MAX_REJECTIONS", 3u32)?,
            hook_servers: parse_hook_servers_env("MCP_HOOK_SERVERS"),
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
}
