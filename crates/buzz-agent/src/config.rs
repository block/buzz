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

#[derive(Debug, Clone)]
pub struct Config {
    /// Model id, if pinned by the harness. `None` lets Goose resolve its own
    /// default from `GOOSE_MODEL` / its config file.
    pub model: Option<String>,
    /// Max provider round-trips per turn. Maps to `SessionConfig.max_turns`.
    pub max_rounds: Option<u32>,
    /// Concurrent sessions this process will hold.
    pub max_sessions: usize,
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
/// Unknown names pass through untouched — goose owns the registry, so
/// gatekeeping here would break every provider it gains that we don't list.
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

fn env_str(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    env_str(key).and_then(|s| s.parse().ok())
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
            max_sessions: env_parse("BUZZ_AGENT_MAX_SESSIONS").unwrap_or(8),
            system_prompt,
            goose_mode: parse_approval(env_str("BUZZ_AGENT_APPROVAL").as_deref()),
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
            system_prompt: None,
            goose_mode: GooseMode::default(),
            provider,
            api_key,
            base_url,
        }
    }

    /// Translate Buzz's provider configuration into Goose's environment.
    ///
    /// Mirrors `goose_env.rs` from PR #1526.
    fn project_goose_env() {
        // Provider: Buzz's `openai-compat` and `relay-mesh` are both
        // OpenAI-wire-compatible, and Goose knows them as plain `openai`.
        if let Some(provider) = env_str("BUZZ_AGENT_PROVIDER") {
            set_if_absent("GOOSE_PROVIDER", goose_provider_name(&provider));
        }

        if let Some(model) = env_str("BUZZ_AGENT_MODEL") {
            set_if_absent("GOOSE_MODEL", &model);
        }

        // Key/base-url aliasing: native Goose names win if already present.
        if let Some(key) = env_str("OPENAI_COMPAT_API_KEY") {
            set_if_absent("OPENAI_API_KEY", &key);
        }
        if let Some(base) = env_str("OPENAI_COMPAT_BASE_URL") {
            set_if_absent("OPENAI_BASE_URL", &base);
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

        // `BUZZ_AGENT_PREFER_MESH_FOR_AUTO` is consumed in `build_provider`
        // (lib.rs), not here: it selects a provider *wrapper* rather than an
        // env var goose reads. See `mesh.rs`.
    }
}

fn set_if_absent(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env is process-global; these tests set disjoint keys and assert only on
    // the pure mapping helpers where possible.

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
        // Goose owns the registry; we must not gatekeep names we don't know.
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
