/// Static capabilities and installation metadata for a known ACP runtime.
pub(crate) struct KnownAcpRuntime {
    pub id: &'static str,
    pub label: &'static str,
    /// Compact product label used by runtime pickers.
    pub display_label: &'static str,
    /// Stable catalog ordering before the label tie-breaker.
    pub sort_priority: u16,
    /// Whether first-run onboarding should offer this runtime.
    pub onboarding_visible: bool,
    pub commands: &'static [&'static str],
    pub aliases: &'static [&'static str],
    /// Arguments used when the runtime is launched without an explicit argv.
    pub default_args: &'static [&'static str],
    /// Runtime-specific worker default used when neither the request nor a
    /// linked persona specifies parallelism. `None` preserves Buzz's global
    /// default.
    pub default_parallelism: Option<u32>,
    /// Whether a desktop-requested lazy harness should defer spawning the ACP
    /// subprocess until accepted work is queued. Runtimes with expensive or
    /// failure-prone first handshakes can opt out while preserving the lazy
    /// relay socket.
    pub defer_agent_start_until_work: bool,
    /// Runtime-specific idle timeout used only when the agent record, process
    /// environment, and merged user environment do not supply an override.
    /// `None` preserves the harness default.
    pub default_idle_timeout_secs: Option<u64>,
    /// App-local runtime mark used by catalog-driven frontend surfaces.
    pub icon_url: &'static str,
    /// Presentation scale for the runtime mark. Kept with the catalog entry so
    /// React does not need a harness-ID lookup table.
    pub icon_scale: f32,
    pub avatar_url: &'static str,
    /// Catalog-default avatar URLs superseded by `avatar_url`. These are not
    /// user-selected images and may be replaced during read-time migration.
    pub superseded_avatar_urls: &'static [&'static str],
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
    /// Env var injection still handles initial model selection separately.
    pub supports_acp_model_switching: bool,
    /// Whether Buzz should pass its resolved model through the generic
    /// `BUZZ_ACP_MODEL` harness setting at process launch.
    ///
    /// This is intentionally separate from `supports_acp_model_switching` and
    /// `model_env_var`: existing adapters may consume the generic bootstrap
    /// model without exposing Buzz-side model controls. Native runtimes whose
    /// official ACP server owns model selection set this to `false`.
    pub accepts_harness_model: bool,
    pub model_env_var: Option<&'static str>,
    pub provider_env_var: Option<&'static str>,
    pub provider_locked: bool,
    /// Environment defaults applied only when neither the parent process nor
    /// saved agent configuration supplies a value.
    pub default_env: &'static [(&'static str, &'static str)],
    /// Environment values enforced at process launch after inherited and
    /// user-configured values have been merged.
    pub enforced_env: &'static [(&'static str, &'static str)],
    /// Environment variables removed from runtime subprocesses. This prevents
    /// ambient process state from overriding catalog-declared identity policy.
    pub scrub_env_vars: &'static [&'static str],
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
    /// CLI argv for an interactive login launched in a visible terminal.
    /// `None` when authentication is adapter-owned or not applicable.
    pub auth_login_args: Option<&'static [&'static str]>,
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

    #[test]
    fn onboarding_visibility_and_order_are_catalog_owned() {
        let mut visible = super::super::KNOWN_ACP_RUNTIMES
            .iter()
            .filter(|runtime| runtime.onboarding_visible)
            .collect::<Vec<_>>();
        visible.sort_by_key(|runtime| runtime.sort_priority);

        assert_eq!(
            visible
                .into_iter()
                .map(|runtime| runtime.id)
                .collect::<Vec<_>>(),
            ["devin", "claude", "codex", "goose", "buzz-agent"]
        );
    }

    #[test]
    fn vendor_metadata_distinguishes_cli_and_adapter_guidance() {
        let goose = known_acp_runtime_exact("goose").unwrap();
        assert_eq!(
            goose.cli_install_instructions_url,
            "https://goose-docs.ai/docs/getting-started/installation/"
        );
        assert!(goose.adapter_install_instructions_url.is_empty());
        assert!(goose.cli_install_hint.contains("desktop app alone"));
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
        assert!(claude.cli_install_hint.contains("desktop app alone"));

        let codex = known_acp_runtime_exact("codex").unwrap();
        assert_eq!(
            codex.cli_install_instructions_url,
            "https://developers.openai.com/codex/cli/"
        );
        assert!(codex.adapter_install_instructions_url.contains("codex-acp"));
        assert!(codex.cli_install_hint.contains("desktop app alone"));

        let devin = known_acp_runtime_exact("devin").unwrap();
        assert_eq!(devin.commands, &["devin"]);
        assert_eq!(devin.default_args, &["acp"]);
        assert_eq!(devin.default_parallelism, Some(1));
        assert!(!devin.defer_agent_start_until_work);
        assert_eq!(devin.default_idle_timeout_secs, Some(120));
        assert_eq!(devin.display_label, "Devin");
        assert_eq!(devin.sort_priority, 20);
        assert!(devin.onboarding_visible);
        assert_eq!(devin.icon_url, "/runtime-icons/devin.svg");
        assert_eq!(devin.icon_scale, 1.1);
        assert_eq!(devin.underlying_cli, Some("devin"));
        assert_eq!(devin.skill_dir, Some(".devin/skills"));
        assert_eq!(
            devin.auth_probe_args,
            Some(&["devin", "auth", "status"][..])
        );
        assert_eq!(devin.auth_login_args, Some(&["devin", "auth", "login"][..]));
        assert!(devin.default_env.is_empty());
        assert_eq!(
            devin.enforced_env,
            &[
                ("BUZZ_ACP_PERMISSION_MODE", "default"),
                ("BUZZ_ACP_AUTO_APPROVE_PERMISSIONS", "false"),
                ("BUZZ_ACP_INTERACTIVE_PERMISSIONS", "true"),
                ("BUZZ_ACP_SELF_PUBLISH_COMPLETION_GRACE", "30"),
            ]
        );
        assert_eq!(devin.scrub_env_vars, &["WINDSURF_API_KEY", "ACP_BACKEND"]);
        assert_eq!(
            devin.cli_install_instructions_url,
            "https://docs.devin.ai/cli"
        );
        assert_eq!(
            devin.cli_install_commands,
            &["curl -fsSL https://cli.devin.ai/install.sh | bash"]
        );
        assert_eq!(
            devin.cli_install_commands_windows,
            &[
                "powershell.exe -NoProfile -Command \"irm https://static.devin.ai/cli/setup.ps1 | iex\""
            ]
        );
        assert!(devin.adapter_install_commands.is_empty());
        assert!(devin.cli_install_hint.contains("desktop app alone"));
    }
}
