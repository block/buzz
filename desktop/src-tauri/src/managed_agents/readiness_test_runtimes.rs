//! `KnownAcpRuntime` fixtures for the `readiness.rs` unit tests, split out of
//! that file to respect the repo's file-size ratchet.
//!
//! Declared as `mod runtime_fixtures` from `readiness.rs`, so `use super::*`
//! reaches the types that module already imports.

use super::*;

/// Construct a minimal `KnownAcpRuntime` stub for testing cli_login_requirements.
/// `commands` are the adapter binaries; `underlying_cli` is the CLI name.
pub(super) fn make_cli_runtime(
    commands: &'static [&'static str],
    underlying_cli: Option<&'static str>,
) -> KnownAcpRuntime {
    KnownAcpRuntime {
        id: "test-cli-runtime",
        label: "Test CLI",
        commands,
        aliases: &[],
        avatar_url: "",
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli,
        cli_install_commands: &[],
        cli_install_commands_windows: &[],
        adapter_install_commands: &[],
        cli_install_instructions_url: "",
        adapter_install_instructions_url: "",
        cli_install_hint: "",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: false,
        config_file_path: None,
        config_file_format: None,
        model_env_var: None,
        provider_env_var: None,
        provider_locked: false,
        default_env: &[],
        supports_acp_native_config: false,
        thinking_env_var: None,
        thinking_config_json_env_var: None,
        thinking_config_json_key: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        max_rounds_env_var: None,
        required_normalized_fields: &[],
        login_hint: None,
        auth_probe_args: None,
    }
}

/// Build a minimal `KnownAcpRuntime` for testing the codex version gate.
/// `adapter_commands` are the exact strings passed to `find_command` — use
/// `&["codex-acp"]` when the binary is on PATH, or `&[<absolute_path>]`
/// when resolving via absolute path.  `underlying_cli` is a portable
/// stand-in so the adapter is not misclassified as `CliMissing`.
pub(super) fn make_codex_runtime(
    adapter_commands: &'static [&'static str],
    underlying_cli: Option<&'static str>,
) -> KnownAcpRuntime {
    KnownAcpRuntime {
        id: "codex",
        label: "Codex",
        commands: adapter_commands,
        aliases: &[],
        avatar_url: "",
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli,
        cli_install_commands: &[],
        cli_install_commands_windows: &[],
        adapter_install_commands: &[],
        cli_install_instructions_url: "",
        adapter_install_instructions_url: "",
        cli_install_hint: "",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: false,
        config_file_path: None,
        config_file_format: None,
        model_env_var: None,
        provider_env_var: None,
        provider_locked: false,
        default_env: &[],
        supports_acp_native_config: false,
        thinking_env_var: None,
        thinking_config_json_env_var: None,
        thinking_config_json_key: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        max_rounds_env_var: None,
        required_normalized_fields: &[],
        login_hint: None,
        auth_probe_args: None,
    }
}
