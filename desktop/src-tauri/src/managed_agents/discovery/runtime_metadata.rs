use crate::managed_agents::HarnessSource;

/// How a rich runtime exposes authentication/setup.
///
/// Absence of a non-interactive CLI probe is not evidence that authentication
/// is irrelevant: ACP runtimes such as OMPK can advertise account/setup
/// methods while having no truthful login-status command.
///
/// `AcpMethods` is used only after hermetic initialize acceptance proves the
/// runtime's advertised ACP metadata contract (owner-permission bridge ack).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeAuthentication {
    NotApplicable,
    CliProbe {
        /// CLI argv used only for a non-interactive authentication probe.
        /// `args[0]` is the executable and the remainder is its subcommand.
        args: &'static [&'static str],
        login_hint: &'static str,
    },
    AcpMethods {
        login_hint: &'static str,
    },
}

impl RuntimeAuthentication {
    pub(crate) fn probe_args(self) -> Option<&'static [&'static str]> {
        match self {
            Self::CliProbe { args, .. } => Some(args),
            Self::NotApplicable | Self::AcpMethods { .. } => None,
        }
    }

    pub(crate) fn login_hint(self) -> Option<&'static str> {
        match self {
            Self::NotApplicable => None,
            Self::CliProbe { login_hint, .. } | Self::AcpMethods { login_hint } => Some(login_hint),
        }
    }

    pub(crate) fn can_connect_account(self) -> bool {
        !matches!(self, Self::NotApplicable)
    }
}

/// Rust-owned policy used to determine whether an installed runtime can
/// actually service a managed ACP session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeReadinessPolicy {
    AvailabilityOnly,
    Configuration,
    Authentication,
    AcpModelCatalog,
}

/// Static capabilities and installation metadata for a known ACP runtime.
#[derive(Debug)]
pub(crate) struct KnownAcpRuntime {
    pub id: &'static str,
    pub label: &'static str,
    pub commands: &'static [&'static str],
    pub aliases: &'static [&'static str],
    pub avatar_url: &'static str,
    /// Trust/editability source is independent from capability richness.
    pub source: HarnessSource,
    /// Canonical default argv for this runtime. Empty persisted instance args
    /// resolve to this live catalog value.
    pub default_args: &'static [&'static str],
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
    /// `Some(dir)` → Pkzz creates a symlink at `<nest>/<dir>/buzz-cli`
    /// pointing to the canonical `.agents/skills/buzz-cli`. `None` → this
    /// runtime reads the canonical path directly or has no skill support.
    #[cfg_attr(not(unix), allow(dead_code))]
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
    pub authentication: RuntimeAuthentication,
    pub readiness_policy: RuntimeReadinessPolicy,
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

#[cfg(test)]
mod tests {
    use super::super::known_acp_runtime_exact;
    use super::{RuntimeAuthentication, RuntimeReadinessPolicy};
    use crate::managed_agents::HarnessSource;

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
    fn frozen_authentication_and_readiness_policies_are_truthful() {
        for id in ["goose", "buzz-agent"] {
            let runtime = known_acp_runtime_exact(id).unwrap();
            assert_eq!(runtime.authentication, RuntimeAuthentication::NotApplicable);
            assert_eq!(
                runtime.readiness_policy,
                RuntimeReadinessPolicy::AvailabilityOnly
            );
        }

        let remote = known_acp_runtime_exact("remote-agent-computer").unwrap();
        assert_eq!(remote.authentication, RuntimeAuthentication::NotApplicable);
        assert_eq!(
            remote.readiness_policy,
            RuntimeReadinessPolicy::Configuration
        );

        let claude = known_acp_runtime_exact("claude").unwrap();
        assert_eq!(
            claude.authentication,
            RuntimeAuthentication::CliProbe {
                args: &["claude", "auth", "status"],
                login_hint: "Run the Claude CLI to complete authentication.",
            }
        );
        assert_eq!(
            claude.readiness_policy,
            RuntimeReadinessPolicy::Authentication
        );

        let codex = known_acp_runtime_exact("codex").unwrap();
        assert_eq!(
            codex.authentication,
            RuntimeAuthentication::CliProbe {
                args: &["codex", "login", "status"],
                login_hint: "Run `codex login` to authenticate.",
            }
        );
        assert_eq!(
            codex.readiness_policy,
            RuntimeReadinessPolicy::Authentication
        );
    }

    #[test]
    fn remote_agent_computer_uses_the_bundled_authenticated_adapter() {
        use super::super::catalog::runtime_setup_fields;

        let runtime = known_acp_runtime_exact("remote-agent-computer")
            .expect("remote agent computer must be rich metadata");
        assert_eq!(runtime.source, HarnessSource::Builtin);
        assert_eq!(runtime.commands, &["buzz-manual-agent-acp"]);
        assert_eq!(runtime.aliases, &["manual-agent"]);
        assert!(runtime.provider_locked);
        assert_eq!(runtime.model_env_var, Some("MANUAL_AGENT_MODEL"));
        assert_eq!(
            runtime.thinking_env_var,
            Some("MANUAL_AGENT_REASONING_EFFORT")
        );
        assert_eq!(
            runtime.readiness_policy,
            RuntimeReadinessPolicy::Configuration
        );
        let setup_fields = runtime_setup_fields(runtime.id);
        assert_eq!(setup_fields.len(), 2);
        assert_eq!(setup_fields[0].env_key, "MANUAL_AGENT_BASE_URL");
        assert_eq!(
            setup_fields[0].validation,
            crate::managed_agents::RuntimeSetupFieldValidation::TailnetHttpsOrigin
        );
        assert_eq!(setup_fields[1].env_key, "MANUAL_AGENT_TOKEN");
        assert!(setup_fields[1].kind == crate::managed_agents::RuntimeSetupFieldKind::Secret);
        assert_eq!(
            setup_fields[1].validation,
            crate::managed_agents::RuntimeSetupFieldValidation::AppPassword
        );
        let serialized = serde_json::to_string(&setup_fields).expect("setup metadata serializes");
        assert!(!serialized.contains("test-app-password"));
    }

    #[test]
    fn ompk_rich_metadata_contains_only_verified_capabilities() {
        let ompk = known_acp_runtime_exact("ompk").expect("OMPK must be rich metadata");
        assert_eq!(ompk.id, "ompk");
        assert_eq!(ompk.commands, &["ompk"]);
        assert!(ompk.aliases.is_empty());
        assert_eq!(ompk.source, HarnessSource::Preset);
        assert_eq!(ompk.default_args, &["acp"]);
        assert!(matches!(
            ompk.authentication,
            RuntimeAuthentication::AcpMethods { .. }
        ));
        assert_eq!(
            ompk.readiness_policy,
            RuntimeReadinessPolicy::AcpModelCatalog
        );
        assert!(ompk.supports_acp_model_switching);
        assert!(ompk.model_env_var.is_none());
        assert!(ompk.provider_env_var.is_none());
        assert!(ompk.thinking_env_var.is_none());
        assert!(ompk.max_tokens_env_var.is_none());
        assert!(ompk.context_limit_env_var.is_none());
        assert!(ompk.max_rounds_env_var.is_none());
        assert!(ompk.config_file_path.is_none());
        assert!(ompk.config_file_format.is_none());
        assert!(!ompk.supports_acp_native_config);
        assert!(ompk.mcp_command.is_none());
        assert!(!ompk.mcp_hooks);
        assert!(ompk.default_env.is_empty());
        assert!(ompk.underlying_cli.is_none());
        assert!(ompk.cli_install_commands.is_empty());
        assert!(ompk.cli_install_commands_windows.is_empty());
        assert!(ompk.adapter_install_commands.is_empty());
        assert!(ompk.required_normalized_fields.is_empty());
    }
}
