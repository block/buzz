//! Environment configuration, translated into Goose's native vocabulary.
//!
//! This replaces `crates/buzz-agent/src/config.rs` (2,709 lines). Most of that
//! file existed to *implement* provider configuration: the `Provider` enum,
//! per-provider base URLs, model-name normalization, Databricks host parsing,
//! OpenAI auto-upgrade rules, and the resolution order between them.
//!
//! Goose already owns all of that (`goose::config`, `goose::providers`). So
//! what remains here is a translation table: read the `BUZZ_AGENT_*` variables
//! `buzz-acp` injects (see `desktop/src-tauri/src/managed_agents/runtime.rs`),
//! and set the `GOOSE_*` / provider variables Goose reads.
//!
//! The mapping is applied to the *process* environment before the Goose
//! `Config` singleton is first touched, because `Config::global()` reads env at
//! initialization.

use goose_provider_types::goose_mode::GooseMode;

/// ACP protocol version. Buzz squats on v2 ahead of the upstream RFD; see the
/// note in `lib.rs::initialize`.
pub const PROTOCOL_VERSION: u32 = 2;

/// Hard caps preserved from buzz-agent's wire contract. These are protocol
/// limits (rejections are `invalid_params`), not loop tuning, so they stay.
pub const MAX_PROMPT_BYTES: usize = 1024 * 1024;
pub const MAX_SYSTEM_PROMPT_BYTES: usize = 512 * 1024;
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum model-requested tool calls dispatched from one provider round.
/// Preserved from the pre-Goose loop to bound fan-out and resource use.
pub const MAX_TOOL_CALLS_PER_TURN: usize = 64;

#[derive(Debug, Clone)]
pub struct Config {
    /// Model id, if pinned by the harness. `None` lets Goose resolve its own
    /// default from `GOOSE_MODEL` / its config file.
    pub model: Option<String>,
    /// Max provider round-trips per turn. Maps to `SessionConfig.max_turns`.
    pub max_rounds: Option<u32>,
    /// Concurrent sessions this process will hold.
    pub max_sessions: usize,
    /// Process-wide cap on simultaneously-outstanding `session/request_permission`
    /// asks. Bounds the [`PermissionBroker`](crate::permission::PermissionBroker)
    /// correlation map independently of `max_sessions` (unbounded by default).
    /// Default 32. Set via `BUZZ_AGENT_MAX_PENDING_PERMISSIONS`; a value of 0 is
    /// treated as 1 by the broker.
    pub max_pending_permissions: usize,
    /// Single absolute deadline for a permission ask — shared by broker
    /// admission and the response wait, so a saturated call cannot live for two
    /// full timeout windows. Default 330s, chosen to outlast the client's 300s
    /// auto-deny so the answer (or auto-deny) lands first. Set via
    /// `BUZZ_AGENT_PERMISSION_TIMEOUT_SECS`.
    pub permission_timeout: std::time::Duration,
    /// Default system prompt, used only when `session/new` omits one.
    pub system_prompt: Option<String>,
    /// Tool-call approval policy.
    ///
    /// `GooseMode::default()` is **`Auto`** (`goose_mode.rs:23-25`) — every
    /// tool call is approved without asking. That matches what buzz ships
    /// today (`buzz-acp/src/acp.rs:1671-1712` auto-approves every permission
    /// request, and the desktop catalog sets `GOOSE_MODE=auto` for the
    /// external goose runtime, `discovery.rs:89`), so it stays the default
    /// here to avoid changing behaviour.
    ///
    /// It is now a knob rather than a hardcode: `BUZZ_AGENT_APPROVAL=approve`
    /// makes goose ask before every tool call, `smart_approve` only for
    /// sensitive ones, `chat` disables tools entirely. Nothing in buzz drives
    /// this yet — wiring it to a real human affordance is the point of the
    /// isolation work, and this is the seam it will use.
    pub goose_mode: GooseMode,
    /// Remind the model to publish when a turn is about to end with no
    /// recognised attempt to post to Buzz. Default off; Desktop opts
    /// shared-compute agents in via `BUZZ_AGENT_REQUIRE_REPLY=1`.
    pub require_reply: bool,

    // ---- Databricks model-discovery fields -------------------------------
    // Goose owns provider auth for the agent loop, but the desktop model
    // picker calls `discover_databricks_models` directly as a library
    // (`desktop/src-tauri/src/commands/agent_models.rs:791`). That path is
    // still ours, so these three fields survive the swap purely to feed it.
    /// Provider family, for model discovery only.
    pub provider: Provider,
    /// Static bearer. Empty means "try the PKCE cache, no browser".
    pub api_key: String,
    /// Provider host, e.g. `DATABRICKS_HOST`.
    pub base_url: String,
}

/// Provider families the desktop model picker can discover models for.
///
/// Retained verbatim from the pre-goose `config.rs` because
/// `desktop/src-tauri` matches on it by name
/// (`commands/agent_models.rs:755-761`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Provider {
    Anthropic,
    #[default]
    OpenAi,
    /// Databricks model serving (`api/2.0/serving-endpoints`).
    Databricks,
    /// Databricks AI Gateway v2.
    DatabricksV2,
}

/// Map `BUZZ_AGENT_APPROVAL` onto goose's tool-approval policy.
///
/// Unknown values fall back to the current shipped behaviour (`Auto`) rather
/// than failing the process: a typo in an env var must not take an agent off
/// the air, and silently tightening would be just as surprising as silently
/// loosening.
fn parse_approval(raw: Option<&str>) -> GooseMode {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("approve") => GooseMode::Approve,
        Some("smart_approve") | Some("smart-approve") => GooseMode::SmartApprove,
        Some("chat") => GooseMode::Chat,
        Some("auto") | None => GooseMode::Auto,
        Some(other) => {
            tracing::warn!(value = other, "unknown BUZZ_AGENT_APPROVAL; using auto");
            GooseMode::Auto
        }
    }
}

/// Translate a Buzz provider id into the name goose's registry knows.
///
/// Unknown names pass through so provider construction can return the
/// authoritative supported-provider error.
fn goose_provider_name(provider: &str) -> &str {
    match provider {
        // Buzz's OpenAI-wire-compatible providers; goose calls them `openai`.
        "openai-compat" | "openai_compat" | "relay-mesh" | "relay_mesh" => "openai",
        // The desktop persists this hyphenated (`agent_models.rs:757`) but
        // goose registers `databricks_v2`
        // (`goose-providers/src/databricks_v2.rs`). Without this alias an
        // existing Databricks v2 agent fails to start.
        "databricks-v2" => "databricks_v2",
        other => other,
    }
}

/// goose's per-request timeout variable for a Buzz provider id, if it has one.
///
/// goose reads the timeout per provider rather than globally, so the name
/// depends on which provider the agent is configured for. `relay-mesh` and the
/// other OpenAI-wire providers all run through goose's `openai` provider and so
/// read `OPENAI_TIMEOUT` (`goose/src/providers/openai_def.rs:118`).
///
/// Returns `None` for providers whose goose implementation has no timeout knob
/// (databricks among them) — there the buzz value cannot be honoured, and
/// inventing a variable goose never reads would be worse than not setting one.
fn provider_timeout_env_key(provider: &str) -> Option<&'static str> {
    match goose_provider_name(provider) {
        "openai" => Some("OPENAI_TIMEOUT"),
        "anthropic" => Some("ANTHROPIC_TIMEOUT"),
        "ollama" => Some("OLLAMA_TIMEOUT"),
        "litellm" => Some("LITELLM_TIMEOUT"),
        _ => None,
    }
}

fn env_str(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    env_str(key).and_then(|s| s.parse().ok())
}

/// Derive a billing identity only when the configured provider endpoint is an
/// official allowlisted origin. Custom/gateway endpoints deliberately return
/// `None`, even when they speak a compatible wire protocol.
pub fn pricing_identity(provider: &str, model: &str) -> Option<crate::types::PricingIdentity> {
    let base_url = match goose_provider_name(provider) {
        "anthropic" => {
            env_str("ANTHROPIC_HOST").unwrap_or_else(|| "https://api.anthropic.com".to_string())
        }
        "openai" => env_str("OPENAI_BASE_URL")
            .or_else(|| env_str("OPENAI_HOST"))
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        "openrouter" => "https://openrouter.ai/api/v1".to_string(),
        _ => return None,
    };
    pricing_authority(&base_url).map(|authority| crate::types::PricingIdentity {
        authority: authority.to_string(),
        model: model.to_string(),
        cache_class: None,
    })
}

fn pricing_authority(base_url: &str) -> Option<&'static str> {
    let parsed = url::Url::parse(base_url).ok()?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.port().is_some_and(|port| port != 443)
    {
        return None;
    }
    let path = parsed.path().trim_end_matches('/');
    match (parsed.host_str()?.to_ascii_lowercase().as_str(), path) {
        ("api.anthropic.com", "") => Some("api.anthropic.com"),
        ("api.openai.com", "/v1") => Some("api.openai.com"),
        ("openrouter.ai", "/api/v1") => Some("openrouter.ai"),
        _ => None,
    }
}

impl Config {
    /// Read `BUZZ_AGENT_*` from the environment and, as a side effect, project
    /// the provider-shaped ones onto the `GOOSE_*` names Goose reads.
    pub fn from_env() -> Self {
        Self::project_goose_env();

        let system_prompt = env_str("BUZZ_AGENT_SYSTEM_PROMPT").or_else(|| {
            env_str("BUZZ_AGENT_SYSTEM_PROMPT_FILE")
                .and_then(|p| std::fs::read_to_string(p).ok())
                .filter(|s| !s.trim().is_empty())
        });

        Self {
            model: env_str("BUZZ_AGENT_MODEL"),
            max_rounds: env_parse::<u32>("BUZZ_AGENT_MAX_ROUNDS").filter(|n| *n > 0),
            // `usize::MAX`, not a small number: main defaulted to unlimited
            // and an agent that never set this must not start refusing
            // sessions after some arbitrary count.
            max_sessions: env_parse("BUZZ_AGENT_MAX_SESSIONS").unwrap_or(usize::MAX),
            max_pending_permissions: env_parse("BUZZ_AGENT_MAX_PENDING_PERMISSIONS")
                .filter(|n| *n > 0)
                .unwrap_or(32),
            permission_timeout: std::time::Duration::from_secs(
                env_parse("BUZZ_AGENT_PERMISSION_TIMEOUT_SECS")
                    .filter(|n| *n > 0)
                    .unwrap_or(330),
            ),
            system_prompt,
            goose_mode: parse_approval(env_str("BUZZ_AGENT_APPROVAL").as_deref()),
            require_reply: env_str("BUZZ_AGENT_REQUIRE_REPLY").is_some_and(|v| v != "0"),
            // Discovery-only; the agent loop resolves providers through goose.
            provider: Provider::default(),
            api_key: String::new(),
            base_url: String::new(),
        }
    }

    /// Minimal config for Databricks model discovery.
    ///
    /// Signature preserved from the pre-goose crate — `desktop/src-tauri`
    /// calls this directly (`commands/agent_models.rs:785`).
    pub fn for_discovery(provider: Provider, api_key: String, base_url: String) -> Self {
        Self {
            model: None,
            max_rounds: None,
            max_sessions: 1,
            max_pending_permissions: 32,
            permission_timeout: std::time::Duration::from_secs(330),
            system_prompt: None,
            goose_mode: GooseMode::default(),
            require_reply: false,
            provider,
            api_key,
            base_url,
        }
    }

    /// Translate Buzz's provider configuration into Goose's environment.
    ///
    /// Mirrors `goose_env.rs` from PR #1526.
    fn project_goose_env() {
        // Provider and model **override** any ambient `GOOSE_*`, they do not
        // defer to it.
        //
        // `BUZZ_AGENT_PROVIDER` / `BUZZ_AGENT_MODEL` are not ambient config:
        // the desktop derives them from the agent record's structured
        // provider/model fields at spawn time, and deliberately refuses to
        // persist `GOOSE_PROVIDER` / `GOOSE_MODEL` in an agent's own env so
        // they cannot shadow those fields
        // (`managed_agents/env_vars.rs:DERIVED_PROVIDER_MODEL_ENV_KEYS`).
        //
        // But the agent subprocess still inherits the desktop's environment,
        // and the desktop inherits the user's login shell. Anyone with goose
        // installed exports `GOOSE_PROVIDER`. With `set_if_absent` that
        // inherited value won, so an agent configured for OpenAI sent its
        // OpenAI model to Anthropic and got `404 model: gpt-…` on every turn
        // — the agent looked broken while its settings looked correct.
        // Observed in live testing, not hypothetical.
        if let Some(provider) = env_str("BUZZ_AGENT_PROVIDER") {
            std::env::set_var("GOOSE_PROVIDER", goose_provider_name(&provider));
        }

        if let Some(model) = env_str("BUZZ_AGENT_MODEL") {
            std::env::set_var("GOOSE_MODEL", &model);
        }

        // Credentials override too, and for the same reason as the provider:
        // the desktop hands the agent its configured key as
        // `OPENAI_COMPAT_API_KEY`, while `OPENAI_API_KEY` is very commonly
        // exported in a developer's login shell and inherited here. Deferring
        // to the inherited one would bill and authenticate the agent as
        // whoever that key belongs to rather than as configured — the same
        // silent-wrong-target failure as the provider, with a credential
        // instead of a hostname.
        if let Some(key) = env_str("OPENAI_COMPAT_API_KEY") {
            std::env::set_var("OPENAI_API_KEY", &key);
        }
        if let Some(base) = env_str("OPENAI_COMPAT_BASE_URL") {
            std::env::set_var("OPENAI_BASE_URL", &base);
        }

        if let Some(effort) = env_str("BUZZ_AGENT_THINKING_EFFORT") {
            set_if_absent("GOOSE_THINKING_EFFORT", &effort);
        }
        if let Some(max_tokens) = env_str("BUZZ_AGENT_MAX_OUTPUT_TOKENS") {
            set_if_absent("GOOSE_MAX_TOKENS", &max_tokens);
        }
        if let Some(ctx) = env_str("BUZZ_AGENT_MAX_CONTEXT_TOKENS") {
            set_if_absent("GOOSE_CONTEXT_LIMIT", &ctx);
        }

        // ---- Knobs whose implementation moved to goose ------------------
        //
        // goose owning the *mechanism* must not mean the `BUZZ_AGENT_*` name
        // stops working. Anyone who set one of these against buzz-agent on
        // main gets the same effect here, because we translate it onto goose's
        // equivalent. Silently ignoring a knob someone deliberately set is the
        // worst outcome: no error, just different behaviour.
        //
        // `set_if_absent`, unlike the provider/model pair above: these have no
        // structured field behind them, so an explicit `GOOSE_*` in the
        // environment is a deliberate override and should win.

        // Per-tool-call timeout. buzz's default was 660s; goose's is 300s
        // (`DEFAULT_EXTENSION_TIMEOUT`), so leaving it unset would silently
        // shorten every long tool call. We therefore project buzz's default
        // too, not just an explicit value.
        set_if_absent(
            "GOOSE_DEFAULT_EXTENSION_TIMEOUT",
            &env_str("BUZZ_AGENT_TOOL_TIMEOUT_SECS")
                .unwrap_or_else(|| DEFAULT_TOOL_TIMEOUT_SECS.to_string()),
        );

        // Tool-result truncation threshold. Same reasoning: buzz truncated at
        // 50 KB, goose at 200 KB, so the default has to be carried across or a
        // tool returning 100 KB reaches the model on this branch when it did
        // not on main.
        set_if_absent(
            "GOOSE_MAX_TOOL_RESPONSE_SIZE",
            &env_str("BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES")
                .unwrap_or_else(|| DEFAULT_TOOL_RESULT_TEXT_BYTES.to_string()),
        );

        // Per-LLM-call timeout. goose has no single provider-agnostic name for
        // this — each provider reads its own (`OPENAI_TIMEOUT`,
        // `ANTHROPIC_TIMEOUT`, ...) — so this projects onto the one belonging
        // to the provider actually in use.
        //
        // This is load-bearing for shared compute, not hygiene. The desktop
        // seeds `BUZZ_AGENT_LLM_TIMEOUT_SECS=660` for mesh agents
        // (`managed_agents/relay_mesh.rs`, PR #6115) because MeshLLM's own
        // backend budget is 600 s and a cold prefill of a large prompt can
        // legitimately take ~500 s. Without this projection that seed reaches
        // nothing on this branch: goose's default is 600 s, just under the
        // server's, so the client would abort a request the mesh is still
        // serving — the exact failure #6115 diagnosed and fixed.
        if let Some(timeout) = env_str("BUZZ_AGENT_LLM_TIMEOUT_SECS") {
            if let Some(key) =
                provider_timeout_env_key(&env_str("BUZZ_AGENT_PROVIDER").unwrap_or_default())
            {
                set_if_absent(key, &timeout);
            }
        }

        // `BUZZ_AGENT_NO_HINTS=1` suppressed AGENTS.md/.goosehints loading.
        // goose has no boolean for this, but it takes the *filename list* —
        // an empty list finds nothing, which is the same outcome.
        if env_str("BUZZ_AGENT_NO_HINTS").is_some_and(|v| v != "0") {
            set_if_absent("CONTEXT_FILE_NAMES", "[]");
        }
    }
}

/// Per-tool-call timeout, in seconds. buzz-agent's long-standing default,
/// preserved because goose's is shorter (300s).
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 660;

/// Tool-result text truncation threshold, in bytes. buzz-agent's default,
/// preserved because goose's is larger (200 KB).
const DEFAULT_TOOL_RESULT_TEXT_BYTES: usize = 50 * 1024;

fn set_if_absent(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Process env is global; tests that mutate it must not interleave.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // Env is process-global; these tests set disjoint keys and assert only on
    // the pure mapping helpers where possible.

    /// The mesh client budget must outlast the mesh server's own.
    ///
    /// The desktop seeds `BUZZ_AGENT_LLM_TIMEOUT_SECS=660` for mesh agents
    /// (PR #6115) against MeshLLM's 600 s backend budget. goose reads the
    /// timeout per provider, so without the projection under test the seed
    /// reaches nothing and goose's own 600 s default applies — under the
    /// server's, which is what made Buzz abandon prefills the mesh was still
    /// serving. Asserts the invariant (`> 600`) rather than the literal.
    #[test]
    fn mesh_llm_timeout_outlasts_the_mesh_backend_budget() {
        let _guard = env_lock();
        for key in [
            "BUZZ_AGENT_LLM_TIMEOUT_SECS",
            "BUZZ_AGENT_PROVIDER",
            "OPENAI_TIMEOUT",
            "GOOSE_PROVIDER",
        ] {
            std::env::remove_var(key);
        }

        std::env::set_var("BUZZ_AGENT_PROVIDER", "relay-mesh");
        std::env::set_var("BUZZ_AGENT_LLM_TIMEOUT_SECS", "660");
        Config::project_goose_env();

        let seated: u64 = std::env::var("OPENAI_TIMEOUT")
            .expect("mesh agents must carry a timeout onto goose's openai provider")
            .parse()
            .expect("timeout must be numeric");
        assert!(
            seated > 600,
            "client budget {seated}s must outlast MeshLLM's 600s backend budget"
        );

        for key in [
            "BUZZ_AGENT_LLM_TIMEOUT_SECS",
            "BUZZ_AGENT_PROVIDER",
            "OPENAI_TIMEOUT",
            "GOOSE_PROVIDER",
        ] {
            std::env::remove_var(key);
        }
    }

    /// A provider goose gives no timeout knob must not get a bogus one.
    #[test]
    fn provider_without_a_timeout_knob_projects_nothing() {
        assert_eq!(
            provider_timeout_env_key("relay-mesh"),
            Some("OPENAI_TIMEOUT")
        );
        assert_eq!(
            provider_timeout_env_key("openai-compat"),
            Some("OPENAI_TIMEOUT")
        );
        assert_eq!(
            provider_timeout_env_key("anthropic"),
            Some("ANTHROPIC_TIMEOUT")
        );
        // goose's databricks providers read no timeout variable; inventing one
        // would be a silent no-op dressed up as support.
        assert_eq!(provider_timeout_env_key("databricks-v2"), None);
    }

    /// Every `BUZZ_AGENT_*` knob whose implementation moved to goose must
    /// still reach goose under its goose name. A knob that silently stops
    /// working is the regression this test exists to prevent.
    ///
    /// Serialised by the shared env lock: `project_goose_env` mutates process
    /// globals.
    #[test]
    fn knobs_that_moved_to_goose_still_reach_it() {
        let _guard = env_lock();
        let keys = [
            "BUZZ_AGENT_TOOL_TIMEOUT_SECS",
            "BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES",
            "BUZZ_AGENT_NO_HINTS",
            "GOOSE_DEFAULT_EXTENSION_TIMEOUT",
            "GOOSE_MAX_TOOL_RESPONSE_SIZE",
            "CONTEXT_FILE_NAMES",
        ];
        for key in keys {
            std::env::remove_var(key);
        }

        std::env::set_var("BUZZ_AGENT_TOOL_TIMEOUT_SECS", "1200");
        std::env::set_var("BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES", "12345");
        std::env::set_var("BUZZ_AGENT_NO_HINTS", "1");
        Config::project_goose_env();

        assert_eq!(
            std::env::var("GOOSE_DEFAULT_EXTENSION_TIMEOUT").unwrap(),
            "1200"
        );
        assert_eq!(
            std::env::var("GOOSE_MAX_TOOL_RESPONSE_SIZE").unwrap(),
            "12345"
        );
        assert_eq!(std::env::var("CONTEXT_FILE_NAMES").unwrap(), "[]");

        for key in keys {
            std::env::remove_var(key);
        }
    }

    /// buzz's defaults differ from goose's, so leaving them unset would change
    /// behaviour for everyone who never touched these knobs: tool calls would
    /// time out at 300s instead of 660s, and tool output would truncate at
    /// 200 KB instead of 50 KB.
    #[test]
    fn buzz_defaults_are_projected_not_left_to_goose() {
        let _guard = env_lock();
        for key in [
            "BUZZ_AGENT_TOOL_TIMEOUT_SECS",
            "BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES",
            "GOOSE_DEFAULT_EXTENSION_TIMEOUT",
            "GOOSE_MAX_TOOL_RESPONSE_SIZE",
        ] {
            std::env::remove_var(key);
        }

        Config::project_goose_env();

        assert_eq!(
            std::env::var("GOOSE_DEFAULT_EXTENSION_TIMEOUT").unwrap(),
            "660",
            "buzz's tool timeout, not goose's 300s"
        );
        assert_eq!(
            std::env::var("GOOSE_MAX_TOOL_RESPONSE_SIZE").unwrap(),
            (50 * 1024).to_string(),
            "buzz's truncation threshold, not goose's 200 KB"
        );

        for key in [
            "GOOSE_DEFAULT_EXTENSION_TIMEOUT",
            "GOOSE_MAX_TOOL_RESPONSE_SIZE",
        ] {
            std::env::remove_var(key);
        }
    }

    /// main defaulted to unlimited concurrent sessions. A small default would
    /// make a busy agent start refusing sessions it used to accept.
    #[test]
    fn max_sessions_defaults_to_unlimited() {
        let _guard = env_lock();
        std::env::remove_var("BUZZ_AGENT_MAX_SESSIONS");
        assert_eq!(Config::from_env().max_sessions, usize::MAX);
    }

    /// Desktop enables this for shared-compute agents; it must be read.
    #[test]
    fn require_reply_is_read_from_the_environment() {
        let _guard = env_lock();
        std::env::remove_var("BUZZ_AGENT_REQUIRE_REPLY");
        assert!(!Config::from_env().require_reply);

        std::env::set_var("BUZZ_AGENT_REQUIRE_REPLY", "1");
        assert!(Config::from_env().require_reply);

        // An explicit 0 opts out, which relay_mesh.rs goes out of its way to
        // preserve through the spawn path.
        std::env::set_var("BUZZ_AGENT_REQUIRE_REPLY", "0");
        assert!(!Config::from_env().require_reply);
        std::env::remove_var("BUZZ_AGENT_REQUIRE_REPLY");
    }

    #[test]
    fn set_if_absent_does_not_clobber() {
        std::env::set_var("BUZZ_TEST_EXISTING", "native");
        set_if_absent("BUZZ_TEST_EXISTING", "translated");
        assert_eq!(std::env::var("BUZZ_TEST_EXISTING").unwrap(), "native");
        std::env::remove_var("BUZZ_TEST_EXISTING");
    }

    #[test]
    fn set_if_absent_fills_missing() {
        std::env::remove_var("BUZZ_TEST_MISSING");
        set_if_absent("BUZZ_TEST_MISSING", "translated");
        assert_eq!(std::env::var("BUZZ_TEST_MISSING").unwrap(), "translated");
        std::env::remove_var("BUZZ_TEST_MISSING");
    }

    /// The regression that made a live OpenAI agent 404 against Anthropic.
    ///
    /// The desktop refuses to persist `GOOSE_PROVIDER` in an agent's env
    /// precisely so it cannot shadow the agent's configured provider — but the
    /// subprocess still inherits it from the user's login shell if goose is
    /// installed. Deferring to the inherited value silently reroutes the
    /// agent's traffic to the wrong provider while its settings still read
    /// correctly.
    #[test]
    fn the_agents_configured_provider_beats_an_inherited_goose_provider() {
        let _guard = env_lock();
        // One test: `project_goose_env` mutates process-global env, so
        // splitting these would race the rest of the suite.
        let restore: Vec<(&str, Option<String>)> = [
            "BUZZ_AGENT_PROVIDER",
            "BUZZ_AGENT_MODEL",
            "GOOSE_PROVIDER",
            "GOOSE_MODEL",
        ]
        .iter()
        .map(|key| (*key, std::env::var(key).ok()))
        .collect();

        std::env::set_var("GOOSE_PROVIDER", "anthropic");
        std::env::set_var("GOOSE_MODEL", "claude-opus-5");
        std::env::set_var("BUZZ_AGENT_PROVIDER", "openai");
        std::env::set_var("BUZZ_AGENT_MODEL", "gpt-5.6-sol");

        Config::project_goose_env();

        assert_eq!(std::env::var("GOOSE_PROVIDER").unwrap(), "openai");
        assert_eq!(std::env::var("GOOSE_MODEL").unwrap(), "gpt-5.6-sol");

        // With no agent-configured provider there is nothing to override with,
        // so an ambient value is still the best answer available.
        std::env::remove_var("BUZZ_AGENT_PROVIDER");
        std::env::set_var("GOOSE_PROVIDER", "anthropic");
        Config::project_goose_env();
        assert_eq!(std::env::var("GOOSE_PROVIDER").unwrap(), "anthropic");

        for (key, value) in restore {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[test]
    fn databricks_v2_hyphen_is_aliased_for_goose() {
        // The desktop persists "databricks-v2" (agent_models.rs:757) but goose
        // registers "databricks_v2". Without the alias an existing v2 agent
        // fails to start.
        assert_eq!(goose_provider_name("databricks-v2"), "databricks_v2");
        assert_eq!(goose_provider_name("databricks_v2"), "databricks_v2");
        assert_eq!(goose_provider_name("databricks"), "databricks");
    }

    #[test]
    fn openai_wire_compatible_providers_map_to_openai() {
        for alias in ["openai-compat", "openai_compat", "relay-mesh", "relay_mesh"] {
            assert_eq!(goose_provider_name(alias), "openai", "alias {alias}");
        }
    }

    #[test]
    fn unknown_providers_pass_through_untouched() {
        // Keep translation separate from the finite provider factory.
        assert_eq!(goose_provider_name("anthropic"), "anthropic");
        assert_eq!(
            goose_provider_name("some_future_provider"),
            "some_future_provider"
        );
    }

    #[test]
    fn approval_defaults_to_auto() {
        // Matches what buzz ships today; changing this silently would alter
        // the security posture of every existing agent.
        assert_eq!(parse_approval(None), GooseMode::Auto);
        assert_eq!(parse_approval(Some("auto")), GooseMode::Auto);
    }

    #[test]
    fn approval_parses_the_stricter_modes() {
        assert_eq!(parse_approval(Some("approve")), GooseMode::Approve);
        assert_eq!(parse_approval(Some(" APPROVE ")), GooseMode::Approve);
        assert_eq!(
            parse_approval(Some("smart_approve")),
            GooseMode::SmartApprove
        );
        assert_eq!(
            parse_approval(Some("smart-approve")),
            GooseMode::SmartApprove
        );
        assert_eq!(parse_approval(Some("chat")), GooseMode::Chat);
    }

    #[test]
    fn approval_falls_back_to_auto_on_garbage() {
        // A typo must not take an agent off the air, and must not silently
        // tighten either — both would be surprising.
        assert_eq!(parse_approval(Some("yolo")), GooseMode::Auto);
    }

    #[test]
    fn stop_reason_wire_strings_are_stable() {
        use crate::types::StopReason;
        // buzz-acp parses these; drift breaks turn completion.
        assert_eq!(StopReason::EndTurn.as_wire(), "end_turn");
        assert_eq!(StopReason::Cancelled.as_wire(), "cancelled");
        assert_eq!(StopReason::MaxTurnRequests.as_wire(), "max_turn_requests");
    }
}
