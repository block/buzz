//! Static catalog of ACP runtimes Buzz knows how to launch.
//!
//! Split out of `discovery.rs`: the catalog is pure data and grows with
//! every new runtime, so it lives on its own to keep the discovery logic
//! readable and within the module size budget.

use super::KnownAcpRuntime;

pub(crate) const GOOSE_AVATAR_URL: &str = "https://goose-docs.ai/img/logo_dark.png";
pub(crate) const CLAUDE_CODE_AVATAR_URL: &str = "https://anthropic.gallerycdn.vsassets.io/extensions/anthropic/claude-code/2.1.77/1773707456892/Microsoft.VisualStudio.Services.Icons.Default";
pub(crate) const CODEX_AVATAR_URL: &str = "https://openai.gallerycdn.vsassets.io/extensions/openai/chatgpt/26.5313.41514/1773706730621/Microsoft.VisualStudio.Services.Icons.Default";
pub(crate) const NANO_CORE_AVATAR_URL: &str = "https://github.com/0-CYBERDYNE-SYSTEMS-0.png";
pub(crate) const BUZZ_AGENT_AVATAR_URL: &str =
    "https://raw.githubusercontent.com/block/buzz/refs/heads/main/crates/buzz-agent/buzz-agent.png";

pub(crate) const KNOWN_ACP_RUNTIMES: &[KnownAcpRuntime] = &[
    KnownAcpRuntime {
        id: "goose",
        label: "Goose",
        commands: &["goose"],
        aliases: &[],
        avatar_url: GOOSE_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli: Some("goose"),
        cli_install_commands: &["curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh | CONFIGURE=false bash"],
        // Goose's stable release currently publishes only the Unix installer;
        // its official Windows instructions intentionally point at this main-branch script.
        cli_install_commands_windows: &["powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"$env:CONFIGURE='false'; irm https://raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1 | iex\""],
        adapter_install_commands: &[],
        cli_install_instructions_url: "https://goose-docs.ai/docs/getting-started/installation/",
        adapter_install_instructions_url: "",
        cli_install_hint: "Buzz talks to Goose through the Goose CLI.",
        adapter_install_hint: "",
        skill_dir: Some(".goose/skills"),
        supports_acp_model_switching: false,
        model_env_var: Some("GOOSE_MODEL"),
        provider_env_var: Some("GOOSE_PROVIDER"),
        provider_locked: false,
        default_env: &[("GOOSE_MODE", "auto")],
        config_file_path: Some("~/.config/goose/config.yaml"),
        config_file_format: Some("yaml"),
        supports_acp_native_config: true,
        thinking_env_var: Some("GOOSE_THINKING_EFFORT"),
        max_tokens_env_var: Some("GOOSE_MAX_TOKENS"),
        context_limit_env_var: Some("GOOSE_CONTEXT_LIMIT"),
        required_normalized_fields: &["model", "provider"],
        login_hint: None,
        auth_probe_args: None,
    },
    KnownAcpRuntime {
        id: "claude",
        label: "Claude Code",
        commands: &["claude-agent-acp", "claude-code-acp"],
        aliases: &["claude-code", "claudecode"],
        avatar_url: CLAUDE_CODE_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli: Some("claude"),
        cli_install_commands: &["curl -fsSL https://claude.ai/install.sh | bash"],
        cli_install_commands_windows: &["powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"irm https://claude.ai/install.ps1 | iex\""],
        adapter_install_commands: &["npm install -g @agentclientprotocol/claude-agent-acp"],
        cli_install_instructions_url: "https://code.claude.com/docs/en/getting-started",
        adapter_install_instructions_url: "https://github.com/agentclientprotocol/claude-agent-acp",
        cli_install_hint: "Buzz talks to Claude Code through the Claude Code CLI.",
        adapter_install_hint: "Buzz talks to the Claude Code CLI through an ACP adapter. Install it with: npm install -g @agentclientprotocol/claude-agent-acp.",
        skill_dir: Some(".claude/skills"),
        supports_acp_model_switching: false,
        model_env_var: None,
        provider_env_var: None,
        provider_locked: true,
        default_env: &[],
        config_file_path: Some("~/.claude/settings.json"),
        config_file_format: Some("json"),
        supports_acp_native_config: false,
        thinking_env_var: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        required_normalized_fields: &[],
        login_hint: Some("Run the Claude CLI to complete authentication."),
        auth_probe_args: Some(&["claude", "auth", "status"]),
    },
    KnownAcpRuntime {
        id: "codex",
        label: "Codex",
        commands: &["codex-acp"],
        aliases: &[],
        avatar_url: CODEX_AVATAR_URL,
        mcp_command: Some("buzz-dev-mcp"),
        mcp_hooks: false,
        underlying_cli: Some("codex"),
        cli_install_commands: &["curl -fsSL https://chatgpt.com/codex/install.sh | sh"],
        cli_install_commands_windows: &["powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"irm https://chatgpt.com/codex/install.ps1 | iex\""],
        adapter_install_commands: &["npm install -g @agentclientprotocol/codex-acp"],
        cli_install_instructions_url: "https://developers.openai.com/codex/cli/",
        adapter_install_instructions_url: "https://github.com/agentclientprotocol/codex-acp",
        cli_install_hint: "Buzz talks to Codex through the Codex CLI.",
        adapter_install_hint: "Buzz talks to the Codex CLI through an ACP adapter. Install it with: npm install -g @agentclientprotocol/codex-acp.",
        skill_dir: Some(".codex/skills"),
        supports_acp_model_switching: false,
        model_env_var: None,
        provider_env_var: None,
        provider_locked: false,
        default_env: &[],
        config_file_path: Some("~/.codex/config.toml"),
        config_file_format: Some("toml"),
        supports_acp_native_config: false,
        thinking_env_var: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        required_normalized_fields: &[],
        login_hint: Some("Run `codex login` to authenticate."),
        // Verified: `codex login status` exits 0 when logged in, non-zero otherwise.
        auth_probe_args: Some(&["codex", "login", "status"]),
    },
    KnownAcpRuntime {
        id: "buzz-agent",
        label: "Buzz Agent",
        commands: &["buzz-agent"],
        aliases: &[],
        avatar_url: BUZZ_AGENT_AVATAR_URL,
        mcp_command: Some("buzz-dev-mcp"),
        mcp_hooks: true,
        underlying_cli: None,
        cli_install_commands: &[],
        cli_install_commands_windows: &[],
        adapter_install_commands: &[],
        cli_install_instructions_url: "https://github.com/block/buzz",
        adapter_install_instructions_url: "https://github.com/block/buzz",
        cli_install_hint: "Ships with the Buzz desktop app.",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: true,
        model_env_var: Some("BUZZ_AGENT_MODEL"),
        provider_env_var: Some("BUZZ_AGENT_PROVIDER"),
        provider_locked: false,
        default_env: &[],
        config_file_path: None,
        config_file_format: None,
        supports_acp_native_config: false,
        thinking_env_var: Some("BUZZ_AGENT_THINKING_EFFORT"),
        max_tokens_env_var: Some("BUZZ_AGENT_MAX_OUTPUT_TOKENS"),
        context_limit_env_var: Some("BUZZ_AGENT_MAX_CONTEXT_TOKENS"),
        required_normalized_fields: &["model", "provider"],
        login_hint: None,
        auth_probe_args: None,
    },
    // nano-core executes each turn inside a per-run Docker container, so it is
    // registered as a conversational / ops runtime rather than a workspace
    // agent. Its ACP preview declares `loadSession: false`, offers no
    // `authMethods`, and advertises neither `modes` nor `models` in
    // `session/new` — so the permission-mode and model-switch calls are skipped
    // by the existing response-shape gates rather than failing.
    //
    // `mcp_command` MUST stay `None`: nano-core *rejects* a non-empty
    // `mcpServers` in `session/new` with `invalidParams` instead of ignoring
    // it, so handing it the dev MCP server would break session creation
    // outright. `skill_dir` is `None` because the container boundary means a
    // `buzz-cli` symlink in the host workspace is not reachable from the agent.
    //
    // `session/new` mints an independent id per call and validates the `cwd`
    // Buzz hands it, so concurrent channels are safe; only a second concurrent
    // prompt on a single session is refused, which matches the harness's
    // one-turn-per-session model.
    KnownAcpRuntime {
        id: "nano-core",
        label: "nano-core",
        commands: &["nano-core"],
        // No aliases needed: command identities normalize `_` → `-`, so
        // `nano_core` already resolves to this entry's id.
        aliases: &[],
        // NOTE: the `["acp"]` default from `default_agent_args` is the launch
        // *shape*, not a complete command line. `bin/nano-core.js` finds its
        // repo by walking up from the working directory, and Buzz launches
        // agents from the nest (`~/.buzz`), so the walk never reaches the
        // checkout and the CLI exits before the ACP handshake. Operators must
        // override this agent's args with `--repo <abs-path> acp`; the flag is
        // position-independent and has no env-var fallback, so it cannot be
        // defaulted here — the path differs per machine.
        avatar_url: NANO_CORE_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli: None,
        // Published as a private package — installed from a local checkout, so
        // there is no automated install command to run.
        cli_install_commands: &[],
        cli_install_commands_windows: &[],
        adapter_install_commands: &[],
        cli_install_instructions_url: "https://github.com/0-CYBERDYNE-SYSTEMS-0/nano-core#readme",
        adapter_install_instructions_url: "https://github.com/0-CYBERDYNE-SYSTEMS-0/nano-core#readme",
        cli_install_hint:
            "Buzz requires the nano-core CLI on PATH — build a local checkout with `npm install && npm run build && npm link`. Then set this agent's arguments to `--repo,/absolute/path/to/nano-core,acp`: the CLI locates its repo by walking up from the working directory, and Buzz launches agents from the nest, so without `--repo` it exits before the ACP handshake.",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: false,
        // nano-core runs Pi inside the agent container and reads both the model
        // and the provider from the Pi env vars. Profile defaults from
        // `PROFILE.json` are softer than the environment, so these win.
        model_env_var: Some("PI_MODEL"),
        provider_env_var: Some("PI_PROVIDER"),
        provider_locked: false,
        default_env: &[],
        // Config lives in a per-profile `PROFILE.json` selected by
        // `NANO_CORE_PROFILE`, so there is no single stable path to surface.
        config_file_path: None,
        config_file_format: None,
        supports_acp_native_config: false,
        thinking_env_var: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        required_normalized_fields: &[],
        login_hint: Some(
            "nano-core authenticates through its own profile. It also needs Docker running and an onboarded profile (`nano-core onboard`) — without those, prompts fail at turn time rather than at connect.",
        ),
        auth_probe_args: None,
    },
];

#[cfg(test)]
mod tests {
    use super::super::{known_acp_runtime_exact, managed_agent_avatar_url, normalize_agent_args};
    use super::NANO_CORE_AVATAR_URL;

    #[test]
    fn resolves_nano_core_as_conversational_runtime() {
        let runtime = known_acp_runtime_exact("nano-core").expect("nano-core runtime");
        assert_eq!(runtime.label, "nano-core");
        assert_eq!(runtime.commands, &["nano-core"]);
        // `nano-core acp` is the ACP entrypoint itself, not an adapter over a
        // separate vendor CLI.
        assert_eq!(runtime.underlying_cli, None);
        // nano-core rejects a non-empty `mcpServers` in `session/new` with
        // `invalidParams` rather than ignoring it, so Buzz must never hand it an
        // MCP server — `build_mcp_servers` only emits one when `mcp_command` is
        // set, so this assertion is what keeps session creation working.
        assert_eq!(runtime.mcp_command, None);
        assert!(!runtime.mcp_hooks);
        // Each turn runs in a per-run Docker container, so a `buzz-cli` symlink in
        // the host workspace would not be reachable from the agent.
        assert_eq!(runtime.skill_dir, None);
        // Private package — installed from a local checkout, so nothing to run.
        assert!(runtime.cli_install_commands.is_empty());
        assert!(runtime.adapter_install_commands.is_empty());
        // Pi runs inside the container and takes both values from the environment,
        // which outranks the profile's own defaults.
        assert_eq!(runtime.model_env_var, Some("PI_MODEL"));
        assert_eq!(runtime.provider_env_var, Some("PI_PROVIDER"));
        // The ACP preview implements no `session/set_config_option`.
        assert!(!runtime.supports_acp_native_config);
        assert_eq!(
            managed_agent_avatar_url("nano-core"),
            Some(NANO_CORE_AVATAR_URL.to_string())
        );
        // Command identities normalize `_` → `-`, so the underscore spelling
        // resolves without needing an explicit alias.
        assert_eq!(
            managed_agent_avatar_url("nano_core"),
            Some(NANO_CORE_AVATAR_URL.to_string())
        );
    }

    #[test]
    fn nano_core_launches_with_the_acp_subcommand() {
        // Unlike the zero-arg native entrypoints, nano-core reaches ACP through a
        // subcommand — `acp` must be supplied, and must survive when a caller
        // passes it explicitly.
        assert_eq!(normalize_agent_args("nano-core", Vec::new()), vec!["acp"]);
        assert_eq!(
            normalize_agent_args("nano-core", vec!["acp".into()]),
            vec!["acp"]
        );
        assert_eq!(normalize_agent_args("nano_core", Vec::new()), vec!["acp"]);
        // Operators must point the CLI at its checkout, because Buzz launches from
        // the nest and nano-core resolves its repo by walking up from the working
        // directory. That override has to survive normalization intact.
        assert_eq!(
            normalize_agent_args(
                "nano-core",
                vec!["--repo".into(), "/opt/nano-core".into(), "acp".into()]
            ),
            vec!["--repo", "/opt/nano-core", "acp"]
        );
    }
}
