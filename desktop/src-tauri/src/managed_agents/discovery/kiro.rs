use super::KnownAcpRuntime;
use crate::managed_agents::KIRO_AVATAR_URL;

pub(super) const RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "kiro",
    label: "Kiro CLI",
    commands: &["kiro-cli"],
    aliases: &[],
    avatar_url: KIRO_AVATAR_URL,
    mcp_command: Some("buzz-dev-mcp"),
    mcp_hooks: false,
    underlying_cli: Some("kiro-cli"),
    cli_install_commands: &["curl -fsSL https://cli.kiro.dev/install | bash"],
    cli_install_commands_windows: &[
        "powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"irm 'https://cli.kiro.dev/install.ps1' | iex\"",
    ],
    adapter_install_commands: &[],
    cli_install_instructions_url: "https://kiro.dev/docs/cli/installation/",
    adapter_install_instructions_url: "",
    cli_install_hint: "Buzz requires Kiro CLI; the desktop app alone is not enough.",
    adapter_install_hint: "",
    skill_dir: None,
    supports_acp_model_switching: true,
    model_env_var: None,
    provider_env_var: None,
    // Kiro owns provider selection through its model catalog. Keep this false
    // because the backend's `provider_locked` display is Claude-specific.
    provider_locked: false,
    default_env: &[],
    config_file_path: None,
    config_file_format: None,
    supports_acp_native_config: false,
    thinking_env_var: None,
    max_tokens_env_var: None,
    context_limit_env_var: None,
    required_normalized_fields: &[],
    login_hint: Some("Run `kiro-cli login` to authenticate."),
    // Verified locally: this exits 0 for an authenticated session and is a
    // read-only identity probe.
    auth_probe_args: Some(&["kiro-cli", "whoami", "--format", "json"]),
    login_command_args: Some(&["kiro-cli", "login"]),
};
