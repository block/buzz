use super::KnownAcpRuntime;

pub(super) const AVATAR_URL: &str = "https://github.com/google-antigravity.png";

pub(super) const RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "antigravity",
    label: "Google Antigravity",
    commands: &["agy-acp"],
    aliases: &[],
    avatar_url: AVATAR_URL,
    mcp_command: None,
    mcp_hooks: false,
    underlying_cli: Some("agy"),
    cli_install_commands: &["curl -fsSL https://antigravity.google/cli/install.sh | bash"],
    cli_install_commands_windows: &[
        "powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"irm https://antigravity.google/cli/install.ps1 | iex\"",
    ],
    // Antigravity does not expose ACP natively. The community adapter is
    // currently source-only, so keep installation owner-reviewed instead of
    // executing an unpinned git branch from the desktop installer.
    adapter_install_commands: &[],
    cli_install_instructions_url: "https://antigravity.google/docs/cli-install",
    adapter_install_instructions_url: "https://github.com/hicder/agy-acp",
    cli_install_hint: "Buzz talks to Google Antigravity through the AGY CLI. Run `agy` once after installation to complete Google sign-in.",
    adapter_install_hint: "Buzz talks to AGY through the community `agy-acp` adapter. Build it from source and place `agy-acp` on PATH.",
    // AGY reads the canonical workspace-level `.agents/skills` directory.
    skill_dir: None,
    supports_acp_model_switching: true,
    model_env_var: None,
    provider_env_var: None,
    provider_locked: true,
    default_env: &[],
    config_file_path: Some("~/.gemini/antigravity-cli/settings.json"),
    config_file_format: Some("json"),
    supports_acp_native_config: false,
    thinking_env_var: None,
    max_tokens_env_var: None,
    context_limit_env_var: None,
    required_normalized_fields: &[],
    // AGY currently has no non-interactive auth-status command. Readiness
    // relies on the owner completing first-run sign-in in `agy`.
    login_hint: None,
    auth_probe_args: None,
};
