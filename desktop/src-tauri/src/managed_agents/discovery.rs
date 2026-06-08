use std::path::{Path, PathBuf};
use std::process::Command;

use crate::managed_agents::{
    AcpAvailabilityStatus, AcpRuntimeCatalogEntry, CommandAvailabilityInfo,
};

pub(crate) struct KnownAcpRuntime {
    pub id: &'static str,
    pub label: &'static str,
    pub commands: &'static [&'static str],
    pub aliases: &'static [&'static str],
    pub avatar_url: &'static str,
    /// MCP server binary for this runtime, or `None` for no MCP server.
    pub mcp_command: Option<&'static str>,
    /// Whether to enable MCP hook tools (`_Stop`, `_PostCompact`) for this agent.
    pub mcp_hooks: bool,
    /// CLI binary that indicates partial install (e.g. `"claude"` when `claude-agent-acp` is missing).
    pub underlying_cli: Option<&'static str>,
    /// Shell commands to install the runtime CLI itself (run sequentially).
    pub cli_install_commands: &'static [&'static str],
    /// Shell commands to install the ACP adapter (run sequentially, after CLI).
    pub adapter_install_commands: &'static [&'static str],
    /// Link to docs/repo for manual instructions.
    pub install_instructions_url: &'static str,
    /// Human-readable hint about installing the CLI binary.
    pub cli_install_hint: &'static str,
    /// Human-readable hint about installing the ACP adapter.
    pub adapter_install_hint: &'static str,
    /// Harness-specific skill discovery directory (e.g. `.goose/skills`).
    /// `Some(dir)` → Sprout creates a symlink at `<nest>/<dir>/sprout-cli`
    /// pointing to the canonical `.agents/skills/sprout-cli`. `None` → this
    /// runtime reads the canonical path directly or has no skill support.
    pub skill_dir: Option<&'static str>,
    pub supports_acp_model_switching: bool,
    pub model_env_var: Option<&'static str>,
    #[allow(dead_code)]
    pub provider_env_var: Option<&'static str>,
    #[allow(dead_code)]
    pub provider_locked: bool,
    pub default_env: &'static [(&'static str, &'static str)],
}

const GOOSE_AVATAR_URL: &str = "https://goose-docs.ai/img/logo_dark.png";
const CLAUDE_CODE_AVATAR_URL: &str = "https://anthropic.gallerycdn.vsassets.io/extensions/anthropic/claude-code/2.1.77/1773707456892/Microsoft.VisualStudio.Services.Icons.Default";
const CODEX_AVATAR_URL: &str = "https://openai.gallerycdn.vsassets.io/extensions/openai/chatgpt/26.5313.41514/1773706730621/Microsoft.VisualStudio.Services.Icons.Default";
const SPROUT_AGENT_AVATAR_URL: &str =
    "https://raw.githubusercontent.com/block/sprout/refs/heads/main/crates/sprout-agent/sprout-agent.png";

fn common_binary_paths() -> &'static [PathBuf] {
    use std::sync::OnceLock;
    static PATHS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    PATHS.get_or_init(|| {
        let mut paths = vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        ];
        if let Some(home) = dirs::home_dir() {
            paths.extend([
                home.join(".local/share/mise/shims"),
                home.join(".local/bin"),
                home.join(".volta/bin"),
                home.join(".asdf/shims"),
            ]);
        }
        paths
    })
}

const KNOWN_ACP_RUNTIMES: &[KnownAcpRuntime] = &[
    KnownAcpRuntime {
        id: "goose",
        label: "Goose",
        commands: &["goose"],
        aliases: &[],
        avatar_url: GOOSE_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli: Some("goose"),
        cli_install_commands: &["curl -fsSL https://github.com/block-open-source/goose/releases/download/stable/download_cli.sh | CONFIGURE=false bash"],
        adapter_install_commands: &[],
        install_instructions_url: "https://block.github.io/goose/",
        cli_install_hint: "Install Goose via the official install script.",
        adapter_install_hint: "",
        skill_dir: Some(".goose/skills"),
        supports_acp_model_switching: false,
        model_env_var: Some("GOOSE_MODEL"),
        provider_env_var: Some("GOOSE_PROVIDER"),
        provider_locked: false,
        default_env: &[("GOOSE_MODE", "auto")],
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
        adapter_install_commands: &["npm install -g @agentclientprotocol/claude-agent-acp"],
        install_instructions_url: "https://github.com/agentclientprotocol/claude-agent-acp",
        cli_install_hint: "Install the Claude Code CLI via the official install script.",
        adapter_install_hint: "Install the Claude Code ACP adapter via npm.",
        skill_dir: Some(".claude/skills"),
        supports_acp_model_switching: false,
        model_env_var: None,
        provider_env_var: None,
        provider_locked: true,
        default_env: &[],
    },
    KnownAcpRuntime {
        id: "codex",
        label: "Codex",
        commands: &["codex-acp"],
        aliases: &[],
        avatar_url: CODEX_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli: Some("codex"),
        cli_install_commands: &["curl -fsSL https://chatgpt.com/codex/install.sh | sh"],
        adapter_install_commands: &["npm install -g @zed-industries/codex-acp"],
        install_instructions_url: "https://github.com/zed-industries/codex-acp",
        cli_install_hint: "Install the Codex CLI via the official install script.",
        adapter_install_hint: "Install the Codex ACP adapter via npm.",
        skill_dir: Some(".codex/skills"),
        supports_acp_model_switching: false,
        model_env_var: None,
        provider_env_var: None,
        provider_locked: true,
        default_env: &[],
    },
    KnownAcpRuntime {
        id: "sprout-agent",
        label: "Sprout Agent",
        commands: &["sprout-agent"],
        aliases: &[],
        avatar_url: SPROUT_AGENT_AVATAR_URL,
        mcp_command: Some("sprout-dev-mcp"),
        mcp_hooks: true,
        underlying_cli: None,
        cli_install_commands: &[],
        adapter_install_commands: &[],
        install_instructions_url: "https://github.com/block/sprout",
        cli_install_hint: "Ships with the Sprout desktop app.",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: true,
        model_env_var: None,
        provider_env_var: Some("SPROUT_AGENT_PROVIDER"),
        provider_locked: false,
        default_env: &[],
    },
];

/// Skill discovery directories declared by known runtimes.
pub(crate) fn known_skill_dirs() -> impl Iterator<Item = &'static str> {
    KNOWN_ACP_RUNTIMES.iter().filter_map(|p| p.skill_dir)
}

fn workspace_root_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn command_looks_like_path(command: &str) -> bool {
    let path = Path::new(command);
    path.is_absolute() || path.components().count() > 1
}

fn executable_basename(command: &str) -> String {
    let suffix = std::env::consts::EXE_SUFFIX;
    if suffix.is_empty() || command.ends_with(suffix) {
        command.to_string()
    } else {
        format!("{command}{suffix}")
    }
}

fn normalize_command_identity(command: &str) -> String {
    let normalized = command.trim().replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    let lower = basename
        .chars()
        .map(|character| match character {
            ' ' | '_' => '-',
            _ => character.to_ascii_lowercase(),
        })
        .collect::<String>();
    let lower = lower.strip_suffix(".exe").unwrap_or(&lower).to_string();

    if let Some(suffix) = std::env::consts::EXE_SUFFIX.strip_prefix('.') {
        return lower
            .strip_suffix(&format!(".{suffix}"))
            .unwrap_or(&lower)
            .to_string();
    }

    if !std::env::consts::EXE_SUFFIX.is_empty() {
        return lower
            .strip_suffix(std::env::consts::EXE_SUFFIX)
            .unwrap_or(&lower)
            .to_string();
    }

    lower
}

pub(crate) fn known_acp_runtime(command: &str) -> Option<&'static KnownAcpRuntime> {
    let normalized = normalize_command_identity(command);

    KNOWN_ACP_RUNTIMES.iter().find(|runtime| {
        normalized == runtime.id
            || runtime
                .commands
                .iter()
                .any(|command| normalized == normalize_command_identity(command))
            || runtime.aliases.iter().any(|alias| normalized == *alias)
    })
}

pub(crate) fn known_acp_runtime_exact(id: &str) -> Option<&'static KnownAcpRuntime> {
    KNOWN_ACP_RUNTIMES.iter().find(|p| p.id == id)
}

fn default_agent_args(command: &str) -> Option<Vec<String>> {
    match normalize_command_identity(command).as_str() {
        "goose" => Some(vec!["acp".to_string()]),
        "codex" | "codex-acp" | "claude-agent-acp" | "claude-code-acp" | "claude-code"
        | "claudecode" | "sprout-agent" => Some(Vec::new()),
        _ => None,
    }
}

pub fn normalize_agent_args(command: &str, agent_args: Vec<String>) -> Vec<String> {
    let normalized = agent_args
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();

    let Some(default_args) = default_agent_args(command) else {
        return normalized;
    };

    if normalized.is_empty() {
        return default_args;
    }

    if normalized.len() == 1 && normalized[0].eq_ignore_ascii_case("acp") && default_args.is_empty()
    {
        return default_args;
    }

    normalized
}

fn command_search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        workspace_root_dir().join("target/release"),
        workspace_root_dir().join("target/debug"),
    ];

    if let Ok(current_dir) = std::env::current_dir() {
        dirs.push(current_dir.join("target/release"));
        dirs.push(current_dir.join("target/debug"));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            dirs.push(parent.to_path_buf());
        }
    }

    let mut unique = Vec::new();
    for dir in dirs {
        if unique.iter().any(|candidate: &PathBuf| candidate == &dir) {
            continue;
        }
        unique.push(dir);
    }

    unique
}

fn resolve_workspace_command(command: &str) -> Option<PathBuf> {
    if command_looks_like_path(command) {
        let path = PathBuf::from(command);
        return path.exists().then_some(path);
    }

    let file_name = executable_basename(command);
    command_search_dirs()
        .into_iter()
        .map(|dir| dir.join(&file_name))
        .find(|candidate| candidate.exists())
}

fn resolve_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, Option<PathBuf>>>
{
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve a command to an absolute path, caching results for the app lifetime.
/// The cache eliminates redundant login-shell spawns when multiple agents share
/// the same binaries (e.g. `npx`, `uvx`).
pub fn resolve_command(command: &str) -> Option<PathBuf> {
    let cache = resolve_cache();

    // Fast path: return cached result without allocating a key.
    if let Ok(guard) = cache.lock() {
        if let Some(result) = guard.get(command) {
            return result.clone();
        }
    }

    // Slow path: resolve and cache.
    let result = resolve_command_uncached(command);

    if result.is_some() {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(command.to_string(), result.clone());
        }
    }

    result
}

/// Clear the resolve_command cache so that newly-installed binaries are detected.
pub fn clear_resolve_cache() {
    let mut guard = resolve_cache().lock().unwrap_or_else(|e| e.into_inner());
    guard.clear();
}

fn resolve_command_uncached(command: &str) -> Option<PathBuf> {
    if let Some(path) = resolve_workspace_command(command) {
        return Some(path);
    }

    if command_looks_like_path(command) {
        let path = PathBuf::from(command);
        return path.exists().then_some(path);
    }

    for candidate in path_candidates_from_env(command) {
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Some(path) = find_via_login_shell(command) {
        return Some(path);
    }
    for dir in common_binary_paths() {
        let candidate = dir.join(executable_basename(command));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn path_candidates_from_env(command: &str) -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(executable_basename(command)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Run a command in a login shell (tries zsh then bash).
/// Returns trimmed stdout if the command succeeds with non-empty output.
fn run_in_login_shell(args: &[&str]) -> Option<String> {
    for shell in ["/bin/zsh", "/bin/bash"] {
        let Ok(output) = Command::new(shell).args(args).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() {
            return Some(stdout);
        }
    }
    None
}

fn find_via_login_shell(command: &str) -> Option<PathBuf> {
    let stdout = run_in_login_shell(&["-l", "-c", r#"command -v -- "$1""#, "_", command])?;
    let resolved = stdout.lines().rfind(|line| !line.trim().is_empty())?;
    let path = PathBuf::from(resolved.trim());
    (path.is_absolute() && path.exists()).then_some(path)
}

/// Return the user's full PATH from a login shell.
/// Cached via OnceLock so we only spawn one shell per app lifetime.
pub fn login_shell_path() -> Option<String> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let stdout = run_in_login_shell(&["-l", "-c", "echo $PATH"])?;
            let last_line = stdout.lines().rfind(|l| !l.trim().is_empty())?;
            Some(last_line.trim().to_string())
        })
        .clone()
}

fn find_command(command: &str) -> Option<PathBuf> {
    resolve_command(command)
}

pub fn command_availability(command: &str) -> CommandAvailabilityInfo {
    let resolved_path = resolve_command(command).map(|path| path.display().to_string());
    CommandAvailabilityInfo {
        command: command.to_string(),
        available: resolved_path.is_some(),
        resolved_path,
    }
}

pub fn missing_command_message(command: &str, role: &str) -> String {
    if command_looks_like_path(command) {
        return format!("{role} `{command}` does not exist.");
    }

    format!(
        "{role} `{command}` was not found. Build the workspace binaries (`cargo build --release --workspace`) or add `target/release` to PATH as described in TESTING.md."
    )
}

fn classify_runtime(
    adapter_result: Option<(&str, PathBuf)>,
    underlying_cli: Option<&str>,
    underlying_cli_found: bool,
) -> (AcpAvailabilityStatus, Option<String>, Option<String>) {
    if let Some((cmd, path)) = adapter_result {
        if underlying_cli.is_some() && !underlying_cli_found {
            (
                AcpAvailabilityStatus::CliMissing,
                Some(cmd.to_string()),
                Some(path.display().to_string()),
            )
        } else {
            (
                AcpAvailabilityStatus::Available,
                Some(cmd.to_string()),
                Some(path.display().to_string()),
            )
        }
    } else if underlying_cli.is_some() && underlying_cli_found {
        (AcpAvailabilityStatus::AdapterMissing, None, None)
    } else {
        (AcpAvailabilityStatus::NotInstalled, None, None)
    }
}

pub fn discover_acp_runtimes() -> Vec<AcpRuntimeCatalogEntry> {
    KNOWN_ACP_RUNTIMES
        .iter()
        .map(|runtime| {
            // Try to find the ACP adapter binary.
            let adapter_result = runtime
                .commands
                .iter()
                .find_map(|command| find_command(command).map(|path| (*command, path)));

            let underlying_cli_found = runtime
                .underlying_cli
                .map(|cli| find_command(cli).is_some())
                .unwrap_or(false);
            let (availability, command, binary_path) =
                classify_runtime(adapter_result, runtime.underlying_cli, underlying_cli_found);

            let underlying_cli_path = runtime
                .underlying_cli
                .and_then(find_command)
                .map(|p| p.display().to_string());

            let default_args = command
                .as_deref()
                .map(|cmd| normalize_agent_args(cmd, Vec::new()))
                .unwrap_or_default();

            let can_auto_install = !runtime.cli_install_commands.is_empty()
                || !runtime.adapter_install_commands.is_empty();

            let cli_hint = runtime.cli_install_hint;
            let adapter_hint = runtime.adapter_install_hint;
            let install_hint = match availability {
                AcpAvailabilityStatus::Available => cli_hint.to_string(),
                AcpAvailabilityStatus::CliMissing => cli_hint.to_string(),
                AcpAvailabilityStatus::AdapterMissing => adapter_hint.to_string(),
                AcpAvailabilityStatus::NotInstalled => {
                    if !cli_hint.is_empty() && !adapter_hint.is_empty() {
                        format!("{cli_hint} {adapter_hint}")
                    } else if !cli_hint.is_empty() {
                        cli_hint.to_string()
                    } else {
                        adapter_hint.to_string()
                    }
                }
            };

            AcpRuntimeCatalogEntry {
                id: runtime.id.to_string(),
                label: runtime.label.to_string(),
                avatar_url: runtime.avatar_url.to_string(),
                availability,
                command,
                binary_path,
                default_args,
                mcp_command: runtime.mcp_command.map(str::to_string),
                install_hint,
                install_instructions_url: runtime.install_instructions_url.to_string(),
                can_auto_install,
                underlying_cli_path,
            }
        })
        .collect()
}

pub fn managed_agent_avatar_url(command: &str) -> Option<String> {
    let runtime = known_acp_runtime(command)?;
    Some(runtime.avatar_url.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        classify_runtime, find_via_login_shell, managed_agent_avatar_url, normalize_agent_args,
        CLAUDE_CODE_AVATAR_URL, CODEX_AVATAR_URL, GOOSE_AVATAR_URL, SPROUT_AGENT_AVATAR_URL,
    };
    use crate::managed_agents::AcpAvailabilityStatus;

    #[test]
    fn resolves_known_avatar_for_bare_command() {
        let avatar_url = managed_agent_avatar_url("goose").expect("goose avatar should resolve");

        assert_eq!(avatar_url, GOOSE_AVATAR_URL);
    }

    #[test]
    fn resolves_known_avatar_for_command_paths_and_aliases() {
        assert_eq!(
            managed_agent_avatar_url("/usr/local/bin/codex-acp"),
            Some(CODEX_AVATAR_URL.to_string())
        );
        assert_eq!(
            managed_agent_avatar_url("Claude Code"),
            Some(CLAUDE_CODE_AVATAR_URL.to_string())
        );
        assert_eq!(
            managed_agent_avatar_url(r"C:\Tools\claude-agent-acp.exe"),
            Some(CLAUDE_CODE_AVATAR_URL.to_string())
        );
        assert_eq!(
            managed_agent_avatar_url("/usr/local/bin/claude-code-acp"),
            Some(CLAUDE_CODE_AVATAR_URL.to_string())
        );
    }

    #[test]
    fn returns_none_for_unknown_commands() {
        assert!(managed_agent_avatar_url("custom-agent").is_none());
    }

    #[test]
    fn normalizes_claude_and_codex_args_to_empty() {
        assert_eq!(
            normalize_agent_args("claude-agent-acp", vec!["acp".into()]),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("claude-code-acp", vec!["acp".into()]),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("codex-acp", vec!["acp".into()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn resolves_sprout_agent_avatar() {
        assert_eq!(
            managed_agent_avatar_url("sprout-agent"),
            Some(SPROUT_AGENT_AVATAR_URL.to_string())
        );
        assert_eq!(
            managed_agent_avatar_url("/usr/local/bin/sprout-agent"),
            Some(SPROUT_AGENT_AVATAR_URL.to_string())
        );
    }

    #[test]
    fn normalizes_sprout_agent_args_to_empty() {
        assert_eq!(
            normalize_agent_args("sprout-agent", Vec::new()),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("sprout-agent", vec!["acp".into()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn login_shell_lookup_treats_command_as_data() {
        let marker =
            std::env::temp_dir().join(format!("sprout-discovery-marker-{}", uuid::Uuid::new_v4()));
        let payload = format!("doesnotexist; touch {} #", marker.display());

        let resolved = find_via_login_shell(&payload);

        assert!(
            resolved.is_none(),
            "payload should not resolve to a command"
        );
        assert!(
            !marker.exists(),
            "shell lookup must not execute injected commands"
        );
    }

    #[test]
    fn classifies_available_when_adapter_found() {
        let (status, cmd, path) = classify_runtime(
            Some(("goose", PathBuf::from("/usr/local/bin/goose"))),
            None,
            false,
        );
        assert_eq!(status, AcpAvailabilityStatus::Available);
        assert_eq!(cmd.as_deref(), Some("goose"));
        assert_eq!(path.as_deref(), Some("/usr/local/bin/goose"));
    }

    #[test]
    fn classifies_adapter_missing_when_cli_present() {
        let (status, cmd, path) = classify_runtime(None, Some("claude"), true);
        assert_eq!(status, AcpAvailabilityStatus::AdapterMissing);
        assert!(cmd.is_none());
        assert!(path.is_none());
    }

    #[test]
    fn classifies_not_installed_when_nothing_found() {
        let (status, cmd, path) = classify_runtime(None, Some("claude"), false);
        assert_eq!(status, AcpAvailabilityStatus::NotInstalled);
        assert!(cmd.is_none());
        assert!(path.is_none());
    }

    #[test]
    fn classifies_not_installed_when_no_underlying_cli() {
        let (status, cmd, path) = classify_runtime(None, None, false);
        assert_eq!(status, AcpAvailabilityStatus::NotInstalled);
        assert!(cmd.is_none());
        assert!(path.is_none());
    }

    #[test]
    fn classifies_cli_missing_when_adapter_found_but_cli_absent() {
        let (status, cmd, path) = classify_runtime(
            Some(("codex-acp", PathBuf::from("/opt/homebrew/bin/codex-acp"))),
            Some("codex"),
            false,
        );
        assert_eq!(status, AcpAvailabilityStatus::CliMissing);
        assert_eq!(cmd.as_deref(), Some("codex-acp"));
        assert_eq!(path.as_deref(), Some("/opt/homebrew/bin/codex-acp"));
    }
}
