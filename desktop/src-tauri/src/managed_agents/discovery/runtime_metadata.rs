/// Canonicalization contract for a harness's thinking-effort env var.
///
/// The single value authority shared by UI choices, the spawn/deploy launch
/// projection, and the reader. All effort candidates (native env, legacy env,
/// ACP tier, file tier) are normalized through `normalize_str` before any
/// validity, precedence, override, or B-equality check.
///
/// Source for Goose: `crates/goose-provider-types/src/thinking.rs`
///   • `FromStr` (aliases, case-insensitive): `off|disabled|none`, `low`,
///     `medium|med`, `high`, `max|xhigh`
///   • `Display` (canonical): `off`, `low`, `medium`, `high`, `max`
///   • Live ACP emits Display values via `response_builder.rs:326-337`.
pub(crate) struct EffortNormalization {
    /// Canonical values in UI display order (drive choices, persistence, ACP comparison).
    pub canonical: &'static [&'static str],
    /// `(alias, canonical)` pairs, case-insensitive. Only aliases that differ
    /// from their canonical form are listed.
    pub aliases: &'static [(&'static str, &'static str)],
}

/// Goose thinking-effort canonicalization contract.
///
/// Source: `crates/goose-provider-types/src/thinking.rs` at Goose `2db0e31fe`.
/// Canonical Display values: `off`, `low`, `medium`, `high`, `max`.
/// Aliases (case-insensitive): `none|disabled→off`, `med→medium`, `xhigh→max`.
/// `minimal` (Buzz-only) is invalid — skipped as absent at every tier.
pub(crate) static GOOSE_EFFORT_NORMALIZATION: EffortNormalization = EffortNormalization {
    canonical: &["off", "low", "medium", "high", "max"],
    aliases: &[
        ("none", "off"),
        ("disabled", "off"),
        ("med", "medium"),
        ("xhigh", "max"),
    ],
};

impl EffortNormalization {
    /// Normalize `raw` to canonical form. `None` → invalid for this harness;
    /// the caller must treat it as absent (skip-as-absent policy).
    pub fn normalize_str(&self, raw: &str) -> Option<String> {
        let lower = raw.to_lowercase();
        if self.canonical.contains(&lower.as_str()) {
            return Some(lower);
        }
        for &(alias, canon) in self.aliases {
            if lower == alias {
                return Some(canon.to_string());
            }
        }
        None
    }
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
    /// Canonicalization contract for `thinking_env_var` on this harness.
    ///
    /// `Some(contract)` — harness uses a finite, static effort vocabulary.
    /// All candidates (native env, legacy env, ACP tier, file tier) are
    /// normalized through this contract before validity checks, precedence
    /// resolution, override tracking, and B-equality comparison.
    ///
    /// `None` — harness accepts any provider/model-specific value via its own
    /// catalog (buzz-agent); see `getProviderEffortConfig()` in TS for that
    /// path. Contract-less does NOT mean keyless: buzz-agent still has a native
    /// `thinking_env_var`, and Claude/Codex route the canonical through
    /// `BUZZ_ACP_EFFORT_LEVEL` for ACP startup even with `thinking_env_var: None`.
    ///
    /// The single canonical authority shared by UI choices, the launch
    /// projection, and the reader. No value-authority logic may live outside
    /// this struct for harnesses that declare one.
    pub effort_normalization: Option<&'static EffortNormalization>,
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
}
