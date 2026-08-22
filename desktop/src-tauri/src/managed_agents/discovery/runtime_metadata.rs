/// Static capabilities and installation metadata for a known ACP runtime.
pub(crate) struct KnownAcpRuntime {
    pub id: &'static str,
    pub label: &'static str,
    pub commands: &'static [&'static str],
    pub aliases: &'static [&'static str],
    pub avatar_url: &'static str,
    /// Legacy MCP server binary field. Vestigial — all agents now use the bundled CLI
    /// directly. Will be removed when runtime discovery is simplified.
    pub mcp_command: Option<&'static str>,
    /// Whether to enable MCP hook tools (`_Stop`, `_PostCompact`) for this agent.
    pub mcp_hooks: bool,
    /// CLI binary that indicates partial install (e.g. `"claude"` when `claude-agent-acp` is missing).
    pub underlying_cli: Option<&'static str>,
    /// Shell commands to install the runtime CLI itself (run sequentially).
    pub cli_install_commands: &'static [&'static str],
    /// Windows-specific CLI install commands (e.g. PowerShell installers).
    /// When non-empty on Windows, these are used instead of `cli_install_commands`.
    #[allow(dead_code)] // read only on Windows via cli_install_commands_for_os()
    pub cli_install_commands_windows: &'static [&'static str],
    /// Shell commands to install the ACP adapter (run sequentially, after CLI).
    pub adapter_install_commands: &'static [&'static str],
    /// Official CLI installation documentation.
    pub cli_install_instructions_url: &'static str,
    /// ACP adapter installation documentation.
    pub adapter_install_instructions_url: &'static str,
    /// Human-readable hint about installing the CLI binary.
    pub cli_install_hint: &'static str,
    /// Human-readable hint about installing the ACP adapter.
    pub adapter_install_hint: &'static str,
    /// Harness-specific skill discovery directory (e.g. `.goose/skills`).
    /// `Some(dir)` → Buzz creates a symlink at `<nest>/<dir>/buzz-cli`
    /// pointing to the canonical `.agents/skills/buzz-cli`. `None` → this
    /// runtime reads the canonical path directly or has no skill support.
    pub skill_dir: Option<&'static str>,
    /// Whether this runtime handles model switching via ACP protocol natively.
    /// Currently unused — env var injection runs unconditionally regardless of
    /// this value. Retained as scaffolding for when ACP model switching matures.
    #[allow(dead_code)]
    pub supports_acp_model_switching: bool,
    pub model_env_var: Option<&'static str>,
    pub provider_env_var: Option<&'static str>,
    pub provider_locked: bool,
    pub default_env: &'static [(&'static str, &'static str)],
    pub config_file_path: Option<&'static str>,
    #[allow(dead_code)] // reserved for format-based dispatch when readers are unified
    pub config_file_format: Option<&'static str>,
    pub supports_acp_native_config: bool, // tier 1a: config/read+write
    pub thinking_env_var: Option<&'static str>,
    /// Env var for normalizing `max_output_tokens`. `None` when the harness
    /// does not have a first-class env var for this field (config-file only).
    pub max_tokens_env_var: Option<&'static str>,
    /// Env var for normalizing `context_limit`. `None` when not applicable.
    pub context_limit_env_var: Option<&'static str>,
    /// Env var for normalizing `max_rounds`. `None` when not applicable.
    pub max_rounds_env_var: Option<&'static str>,
    /// Normalized field keys that must be set for this harness to function.
    /// Used by the config bridge to mark fields as required in the UI.
    /// Keys match the camelCase names used in `NormalizedConfig` (e.g. "model", "provider").
    pub required_normalized_fields: &'static [&'static str],
    /// Human-readable hint shown in Doctor when the runtime is available but not
    /// authenticated. `None` for runtimes that have no login step (goose, buzz-agent).
    pub login_hint: Option<&'static str>,
    /// CLI args for probing authentication status. `args[0]` is the binary name;
    /// the remainder are the subcommand. `None` for runtimes with no login step.
    pub auth_probe_args: Option<&'static [&'static str]>,
}

impl KnownAcpRuntime {
    /// Return the CLI install commands for the current platform.
    ///
    /// On Windows, returns `cli_install_commands_windows` when non-empty,
    /// falling back to the default `cli_install_commands`. On other platforms
    /// always returns `cli_install_commands`.
    pub fn cli_install_commands_for_os(&self) -> &[&str] {
        #[cfg(windows)]
        {
            if !self.cli_install_commands_windows.is_empty() {
                return self.cli_install_commands_windows;
            }
        }
        self.cli_install_commands
    }
}

const OPENCODE_AVATAR_URL: &str =
    "https://raw.githubusercontent.com/block/buzz/refs/heads/main/desktop/public/harness-logos/opencode.svg";

/// Compiled-in opencode runtime (tier-1). Native ACP: the CLI is the agent, so
/// `underlying_cli` doubles as the Phase-1 auto-install marker. Two deliberate
/// `None`s, verified against opencode source: `auth_probe_args` (`opencode
/// auth list` exits 0 even with zero credentials — no faithful probe) and
/// `model_env_var` (opencode core reads no model env var; pinning is
/// config-side via `~/.config/opencode/opencode.json`).
pub(crate) const OPENCODE_RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "opencode",
    label: "OpenCode",
    commands: &["opencode"],
    aliases: &[],
    avatar_url: OPENCODE_AVATAR_URL,
    mcp_command: None,
    mcp_hooks: false,
    underlying_cli: Some("opencode"),
    // The vendor installer is one cross-platform bash script (Git Bash on
    // Windows); opencode publishes no PowerShell installer to route through
    // the Defender-safe two-step form, so both OSes use the pipe.
    cli_install_commands: &["curl -fsSL https://opencode.ai/install | bash"],
    cli_install_commands_windows: &[],
    adapter_install_commands: &[],
    cli_install_instructions_url: "https://opencode.ai/docs",
    adapter_install_instructions_url: "",
    cli_install_hint: "Buzz talks to OpenCode through the OpenCode CLI's ACP mode (opencode acp).",
    adapter_install_hint: "",
    skill_dir: Some(".opencode/skills"),
    supports_acp_model_switching: false,
    model_env_var: None,
    provider_env_var: None,
    provider_locked: false,
    default_env: &[],
    config_file_path: Some("~/.config/opencode/opencode.json"),
    config_file_format: Some("json"),
    supports_acp_native_config: false,
    thinking_env_var: None,
    max_tokens_env_var: None,
    context_limit_env_var: None,
    max_rounds_env_var: None,
    required_normalized_fields: &[],
    login_hint: None,
    auth_probe_args: None,
};

/// Resolve a `BUZZ_DEFAULT_RUNTIME` value to a harness command via the
/// authoritative three-tier lookup (builtins → presets → loaded registry).
/// Empty or unresolvable values yield `None` so callers keep the bundled
/// default; unknown ids log a warning rather than pinning agents to a
/// dangling command.
pub(super) fn resolve_default_runtime(id: &str) -> Option<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return None;
    }
    match super::presets::command_for_runtime_id(trimmed) {
        Some(cmd) => Some(cmd),
        None => {
            tracing::warn!(
                runtime = trimmed,
                "BUZZ_DEFAULT_RUNTIME does not resolve to a known harness — using bundled buzz-agent"
            );
            None
        }
    }
}

/// `BUZZ_DEFAULT_RUNTIME` engine override for newly-created agents, read live
/// from the process environment. Documented in crates/buzz-acp/README.md.
pub(super) fn default_runtime_override() -> Option<String> {
    resolve_default_runtime(&std::env::var("BUZZ_DEFAULT_RUNTIME").ok()?)
}

/// The bundled default engine: buzz-agent ships with the app, so it is safe
/// on a stock install where no third-party CLI is on PATH.
pub(super) fn bundled_default_agent_command() -> String {
    super::known_acp_runtime_exact("buzz-agent")
        .and_then(|p| p.commands.first().copied())
        .unwrap_or("buzz-agent")
        .to_string()
}

/// opencode runtime tests live in a sibling file (file-size ratchet keeps
/// discovery.rs/tests.rs at their merge-base sizes; this module has headroom).
#[cfg(test)]
#[path = "tests/opencode.rs"]
mod opencode_tests;

#[cfg(test)]
mod tests {
    use super::super::known_acp_runtime_exact;

    #[test]
    fn vendor_metadata_distinguishes_cli_and_adapter_guidance() {
        let goose = known_acp_runtime_exact("goose").unwrap();
        assert_eq!(
            goose.cli_install_instructions_url,
            "https://goose-docs.ai/docs/getting-started/installation/"
        );
        assert!(goose.adapter_install_instructions_url.is_empty());
        assert!(goose.cli_install_hint.contains("Goose CLI"));
        assert!(goose
            .cli_install_commands_windows
            .iter()
            .any(|command| command.contains("raw.githubusercontent.com/aaif-goose/goose/main")));
        assert!(goose
            .cli_install_commands_windows
            .iter()
            .any(|command| command.contains("$env:CONFIGURE='false'")));

        let claude = known_acp_runtime_exact("claude").unwrap();
        assert_eq!(
            claude.cli_install_instructions_url,
            "https://code.claude.com/docs/en/getting-started"
        );
        assert!(claude
            .adapter_install_instructions_url
            .contains("claude-agent-acp"));
        assert!(claude.cli_install_hint.contains("Claude Code CLI"));

        let codex = known_acp_runtime_exact("codex").unwrap();
        assert_eq!(
            codex.cli_install_instructions_url,
            "https://developers.openai.com/codex/cli/"
        );
        assert!(codex.adapter_install_instructions_url.contains("codex-acp"));
        assert!(codex.cli_install_hint.contains("Codex CLI"));
    }

    #[test]
    fn opencode_metadata_pins_native_acp_shape() {
        let opencode = known_acp_runtime_exact("opencode").unwrap();

        // Native ACP: the CLI is the agent — no adapter tier, and the CLI
        // itself doubles as the underlying-CLI marker so Phase-1 install runs.
        assert_eq!(opencode.commands, &["opencode"]);
        assert_eq!(opencode.underlying_cli, Some("opencode"));
        assert!(opencode.adapter_install_commands.is_empty());
        assert!(opencode
            .cli_install_commands
            .iter()
            .any(|cmd| cmd.contains("https://opencode.ai/install")));

        // Skills and config follow opencode's documented project layout.
        assert_eq!(opencode.skill_dir, Some(".opencode/skills"));
        assert_eq!(
            opencode.config_file_path,
            Some("~/.config/opencode/opencode.json")
        );

        // No faithful exit-code auth probe exists (`opencode auth list` exits
        // 0 with zero credentials) and no native model env var is read by
        // opencode core — both stay None until those become probeable.
        assert_eq!(opencode.auth_probe_args, None);
        assert_eq!(opencode.model_env_var, None);
        assert_eq!(opencode.provider_env_var, None);
    }
}
