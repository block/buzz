//! Shared harness environment contract for workload substrates.
//!
//! Both the process substrate and the Docker substrate launch the same sprig
//! ACP harness (`buzz-acp`) and hand it the same environment contract the
//! desktop launcher uses. This module owns that mapping so the two substrates
//! cannot drift: given a workload spec, the one-time launch key, the effective
//! relay URL, and the resolved runtime launch details, it produces the exact
//! `BUZZ_*` variable set a body needs. How those variables reach the body —
//! `Command::env` for a child process, an env-file for `docker run` — stays
//! substrate-local.

use buzz_core::execution::{AgentWorkloadContext, WorkloadSpec};
use zeroize::Zeroizing;

/// LLM provider credentials and endpoints forwarded from the node operator's
/// own environment into every workload body.
///
/// This is a deliberate allowlist, not blanket inheritance: provider
/// credentials are node-operator environment — never part of the workload
/// spec ("keep secrets out of configuration"). The names mirror what the
/// bundled runtimes actually read: `buzz-agent`
/// (crates/buzz-agent/src/config.rs) and Goose read the
/// Anthropic/OpenAI-compatible/OpenRouter/Databricks families; Goose and
/// Codex also accept `OPENAI_API_KEY` directly; Claude Code accepts
/// `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`) for headless
/// subscription auth.
pub(crate) const PROVIDER_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_BASE_URL",
    "OPENAI_API_KEY",
    "OPENAI_COMPAT_API_KEY",
    "OPENAI_COMPAT_MODEL",
    "OPENAI_COMPAT_BASE_URL",
    "OPENAI_COMPAT_API",
    "OPENROUTER_API_KEY",
    "OPENROUTER_MODEL",
    "OPENROUTER_BASE_URL",
    "DATABRICKS_HOST",
    "DATABRICKS_TOKEN",
    "DATABRICKS_MODEL",
];

/// Static launch details for one known runtime identifier.
///
/// This is the shared runtime catalog both substrates resolve against,
/// mirroring the desktop runtime catalog
/// (`desktop/src-tauri/src/managed_agents/discovery.rs`): Goose, Claude Code
/// (via the `claude-agent-acp` adapter), Codex (via the `codex-acp` adapter),
/// and the bundled `buzz-agent`. Command names are substrate-relative — the
/// process substrate resolves them to host executable paths, the Docker
/// substrate uses them as-is because the runtime's agent-body image variant
/// (`Dockerfile.agent`, `RUNTIME` build arg) bakes them onto the image's
/// `PATH`. Unknown runtime identifiers are attempted verbatim as a command
/// name so custom harness setups keep working.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KnownRuntime {
    /// Inner agent command the harness runs (`BUZZ_ACP_AGENT_COMMAND`).
    pub command: &'static str,
    /// Developer MCP command, when the runtime uses one.
    pub mcp: Option<&'static str>,
    /// Runtime-specific defaults, e.g. Goose's non-interactive mode.
    pub default_env: &'static [(&'static str, &'static str)],
    /// Env var the runtime reads its model from, when it has one.
    pub model_env: Option<&'static str>,
    /// Env var the runtime reads its provider from, when it is not locked.
    pub provider_env: Option<&'static str>,
    /// Whether to point the Claude adapter at a `claude` CLI through
    /// `CLAUDE_CODE_EXECUTABLE`. Substrate-appropriate: the process substrate
    /// resolves a host path, the Docker substrate the in-image install path.
    pub wants_claude_cli: bool,
    /// Agent-body image variant carrying this runtime, when it is not part
    /// of the slim image (`Dockerfile.agent` builds one image per runtime,
    /// selected by its `RUNTIME` build arg; `just agent-image <variant>`
    /// tags it `buzz-agent:<variant>`). `None` means the slim image already
    /// carries the runtime (the sprig personalities) or the runtime is
    /// unknown and runs whatever image the operator configured.
    pub image_variant: Option<&'static str>,
}

/// Catalog entry shape for a runtime with no known launch details.
pub(crate) const UNKNOWN_RUNTIME: KnownRuntime = KnownRuntime {
    command: "",
    mcp: None,
    default_env: &[],
    model_env: None,
    provider_env: None,
    wants_claude_cli: false,
    image_variant: None,
};

/// Look up a normalized (trimmed, lowercased) runtime identifier in the
/// shared catalog. `None` means the identifier should be attempted verbatim
/// as a command name.
pub(crate) fn known_runtime(normalized: &str) -> Option<KnownRuntime> {
    match normalized {
        "goose" => Some(KnownRuntime {
            command: "goose",
            model_env: Some("GOOSE_MODEL"),
            provider_env: Some("GOOSE_PROVIDER"),
            default_env: &[("GOOSE_MODE", "auto")],
            image_variant: Some("goose"),
            ..UNKNOWN_RUNTIME
        }),
        // Claude Code is provider-locked: no provider env is derived.
        "claude" | "claude-code" | "claudecode" | "claude-agent-acp" | "claude-code-acp" => {
            Some(KnownRuntime {
                command: "claude-agent-acp",
                wants_claude_cli: true,
                image_variant: Some("claude"),
                ..UNKNOWN_RUNTIME
            })
        }
        "codex" | "codex-acp" => Some(KnownRuntime {
            command: "codex-acp",
            mcp: Some("buzz-dev-mcp"),
            image_variant: Some("codex"),
            ..UNKNOWN_RUNTIME
        }),
        "buzz-agent" => Some(KnownRuntime {
            command: "buzz-agent",
            mcp: Some("buzz-dev-mcp"),
            model_env: Some("BUZZ_AGENT_MODEL"),
            provider_env: Some("BUZZ_AGENT_PROVIDER"),
            ..UNKNOWN_RUNTIME
        }),
        _ => None,
    }
}

/// Resolved launch details for one runtime, as the harness needs them.
///
/// The process substrate resolves these to host executable paths; the Docker
/// substrate uses the command names baked into the agent image. Either way
/// the harness contract is identical.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeLaunch<'a> {
    /// Inner agent command the harness runs (`BUZZ_ACP_AGENT_COMMAND`).
    pub agent_command: &'a str,
    /// Developer MCP command, when the runtime uses one.
    pub mcp_command: Option<&'a str>,
    /// Runtime-specific defaults, e.g. Goose's non-interactive mode.
    pub default_env: &'a [(&'static str, &'static str)],
    /// Env var the runtime reads its model from, when it has one.
    pub model_env: Option<&'a str>,
    /// Env var the runtime reads its provider from, when it is not locked.
    pub provider_env: Option<&'a str>,
}

fn push(env: &mut Vec<(String, Zeroizing<String>)>, name: &str, value: impl Into<String>) {
    env.push((name.to_string(), Zeroizing::new(value.into())));
}

/// Build the harness environment for one workload body.
///
/// Mirrors the desktop launcher contract
/// (`desktop/src-tauri/src/managed_agents/runtime.rs`). The returned values
/// include the one-time launch key (`BUZZ_PRIVATE_KEY`), so every entry is
/// zeroized on drop and the whole set must never be persisted or logged.
pub(crate) fn harness_environment(
    spec: &WorkloadSpec,
    agent: &AgentWorkloadContext,
    launch_key: &str,
    relay_url: &str,
    launch: &RuntimeLaunch<'_>,
) -> Vec<(String, Zeroizing<String>)> {
    let mut env: Vec<(String, Zeroizing<String>)> = Vec::new();

    // ── Identity and relay: the one-time key handoff. ───────────────────────
    push(&mut env, "BUZZ_PRIVATE_KEY", launch_key);
    push(&mut env, "BUZZ_RELAY_URL", relay_url);
    if let Some(auth_tag) = &agent.auth_tag {
        push(&mut env, "BUZZ_AUTH_TAG", auth_tag.as_str());
    }

    // ── Harness contract. ───────────────────────────────────────────────────
    if let Some(prompt) = &agent.system_prompt {
        push(&mut env, "BUZZ_ACP_SYSTEM_PROMPT", prompt.as_str());
    }
    push(&mut env, "BUZZ_ACP_AGENT_COMMAND", launch.agent_command);
    push(
        &mut env,
        "BUZZ_ACP_AGENT_ARGS",
        agent.runtime_settings.agent_args.join(","),
    );
    push(
        &mut env,
        "BUZZ_ACP_MCP_COMMAND",
        launch.mcp_command.unwrap_or(""),
    );
    // Timeouts are set only when the owner overrode them; otherwise the
    // harness default is the single source of truth ("bound the instance's
    // lifetime" — the body reaps itself, the node never does).
    if let Some(idle) = agent.runtime_settings.idle_timeout_seconds {
        push(&mut env, "BUZZ_ACP_IDLE_TIMEOUT", idle.to_string());
    }
    if let Some(max_turn) = agent.runtime_settings.max_turn_duration_seconds {
        push(&mut env, "BUZZ_ACP_MAX_TURN_DURATION", max_turn.to_string());
    }
    push(
        &mut env,
        "BUZZ_ACP_AGENTS",
        agent.runtime_settings.parallelism.to_string(),
    );
    push(&mut env, "BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "steer");
    push(&mut env, "BUZZ_ACP_DEDUP", "queue");
    push(&mut env, "BUZZ_ACP_RELAY_OBSERVER", "true");
    push(
        &mut env,
        "BUZZ_ACP_SESSION_TITLE",
        spec.display_name.as_str(),
    );
    if let Some(model) = &spec.model {
        push(&mut env, "BUZZ_ACP_MODEL", model.as_str());
    }
    for (name, value) in launch.default_env {
        push(&mut env, name, *value);
    }
    if let (Some(model_env), Some(model)) = (launch.model_env, spec.model.as_deref()) {
        push(&mut env, model_env, model);
    }
    if let (Some(provider_env), Some(provider)) = (launch.provider_env, spec.provider.as_deref()) {
        push(&mut env, provider_env, provider);
    }

    // ── Inbound author gate. ────────────────────────────────────────────────
    if let Some(mode) = &agent.response_mode {
        push(&mut env, "BUZZ_ACP_RESPOND_TO", mode.as_str());
    }
    if !agent.response_allowlist.is_empty() {
        push(
            &mut env,
            "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
            agent.response_allowlist.join(","),
        );
    }

    env
}
