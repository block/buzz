/// Native model-catalog protocol owned by a known runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeModelDiscovery {
    /// LM Studio's native `GET /api/v1/models` contract.
    LmStudioV1,
}

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
    /// Link to docs/repo for manual instructions.
    pub install_instructions_url: &'static str,
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
    /// Exact provider value forced when `provider_locked` is true.
    pub locked_provider_id: Option<&'static str>,
    /// Human-readable label for the catalog-owned locked provider.
    pub locked_provider_label: Option<&'static str>,
    /// Native model discovery protocol, when discovery must not spawn ACP.
    pub native_model_discovery: Option<NativeModelDiscovery>,
    /// Trusted runtime-policy environment keys. These are projected to the
    /// frontend but remain enforced in Rust after every user environment layer.
    pub base_url_env_var: Option<&'static str>,
    pub classification_env_var: Option<&'static str>,
    pub integrations_env_var: Option<&'static str>,
    /// Optional OS-Keychain entry name for an API bearer token.
    pub keychain_token_key: Option<&'static str>,
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

/// Canonical capabilities for Buzz's bundled LM Studio-native ACP runtime.
pub(super) const LM_STUDIO_RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "buzz-lmstudio-agent",
    label: "Buzz LM Studio Agent",
    commands: &["buzz-lmstudio-agent"],
    aliases: &[],
    avatar_url: super::BUZZ_AGENT_AVATAR_URL,
    mcp_command: None,
    mcp_hooks: false,
    underlying_cli: None,
    cli_install_commands: &[],
    cli_install_commands_windows: &[],
    adapter_install_commands: &[],
    install_instructions_url: "https://lmstudio.ai/docs/developer/rest",
    cli_install_hint: "Ships with Buzz; LM Studio must be installed separately.",
    adapter_install_hint: "",
    skill_dir: None,
    supports_acp_model_switching: true,
    model_env_var: Some("LM_STUDIO_MODEL"),
    provider_env_var: Some("BUZZ_AGENT_PROVIDER"),
    provider_locked: true,
    locked_provider_id: Some("lmstudio-native"),
    locked_provider_label: Some("LM Studio native"),
    native_model_discovery: Some(NativeModelDiscovery::LmStudioV1),
    base_url_env_var: Some("LM_STUDIO_BASE_URL"),
    classification_env_var: Some("BUZZ_AGENT_CLASSIFICATION"),
    integrations_env_var: Some("LM_STUDIO_MCP_INTEGRATIONS"),
    keychain_token_key: Some("lm-studio-api-token"),
    default_env: &[],
    config_file_path: None,
    config_file_format: None,
    supports_acp_native_config: false,
    thinking_env_var: Some("LM_STUDIO_REASONING"),
    max_tokens_env_var: Some("BUZZ_AGENT_MAX_OUTPUT_TOKENS"),
    context_limit_env_var: Some("BUZZ_AGENT_MAX_CONTEXT_TOKENS"),
    required_normalized_fields: &["model"],
    login_hint: None,
    auth_probe_args: None,
};
