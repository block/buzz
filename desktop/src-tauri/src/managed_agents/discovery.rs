use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::AppHandle;

use crate::managed_agents::{AcpProviderInfo, CommandAvailabilityInfo};

pub(crate) struct KnownAcpProvider {
    pub id: &'static str,
    pub label: &'static str,
    pub commands: &'static [&'static str],
    pub aliases: &'static [&'static str],
    pub avatar_url: &'static str,
    /// MCP server binary to use instead of the default `sprout-mcp-server`.
    pub mcp_command: Option<&'static str>,
    /// Whether to enable MCP hook tools (`_Stop`, `_PostCompact`) for this agent.
    pub mcp_hooks: bool,
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

const KNOWN_ACP_PROVIDERS: &[KnownAcpProvider] = &[
    KnownAcpProvider {
        id: "goose",
        label: "Goose",
        commands: &["goose"],
        aliases: &[],
        avatar_url: GOOSE_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
    },
    KnownAcpProvider {
        id: "claude",
        label: "Claude Code",
        commands: &["claude-agent-acp", "claude-code-acp"],
        aliases: &["claude-code", "claudecode"],
        avatar_url: CLAUDE_CODE_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
    },
    KnownAcpProvider {
        id: "codex",
        label: "Codex",
        commands: &["codex-acp"],
        aliases: &[],
        avatar_url: CODEX_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
    },
    KnownAcpProvider {
        id: "sprout-agent",
        label: "Sprout Agent",
        commands: &["sprout-agent"],
        aliases: &[],
        avatar_url: SPROUT_AGENT_AVATAR_URL,
        mcp_command: Some("sprout-dev-mcp"),
        mcp_hooks: true,
    },
];

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

pub(crate) fn known_acp_provider(command: &str) -> Option<&'static KnownAcpProvider> {
    let normalized = normalize_command_identity(command);

    KNOWN_ACP_PROVIDERS.iter().find(|provider| {
        normalized == provider.id
            || provider
                .commands
                .iter()
                .any(|command| normalized == normalize_command_identity(command))
            || provider.aliases.iter().any(|alias| normalized == *alias)
    })
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

fn command_search_dirs(app: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut dirs = vec![
        workspace_root_dir().join("target/release"),
        workspace_root_dir().join("target/debug"),
    ];

    if let Ok(current_dir) = std::env::current_dir() {
        dirs.push(current_dir.join("target/release"));
        dirs.push(current_dir.join("target/debug"));
    }

    if app.is_some() {
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                dirs.push(parent.to_path_buf());
            }
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

fn resolve_workspace_command(command: &str, app: Option<&AppHandle>) -> Option<PathBuf> {
    if command_looks_like_path(command) {
        let path = PathBuf::from(command);
        return path.exists().then_some(path);
    }

    let file_name = executable_basename(command);
    command_search_dirs(app)
        .into_iter()
        .map(|dir| dir.join(&file_name))
        .find(|candidate| candidate.exists())
}

/// Resolve a command to an absolute path, caching results for the app lifetime.
/// The cache eliminates redundant login-shell spawns when multiple agents share
/// the same binaries (e.g. `npx`, `uvx`).
pub fn resolve_command(command: &str, app: Option<&AppHandle>) -> Option<PathBuf> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    // Fast path: return cached result without allocating a key.
    if let Ok(guard) = cache.lock() {
        if let Some(result) = guard.get(command) {
            return result.clone();
        }
    }

    // Slow path: resolve and cache.
    let result = resolve_command_uncached(command, app);

    if result.is_some() {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(command.to_string(), result.clone());
        }
    }

    result
}

fn resolve_command_uncached(command: &str, app: Option<&AppHandle>) -> Option<PathBuf> {
    if let Some(path) = resolve_workspace_command(command, app) {
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
    resolve_command(command, None)
}

pub fn command_availability(command: &str, app: Option<&AppHandle>) -> CommandAvailabilityInfo {
    let resolved_path = resolve_command(command, app).map(|path| path.display().to_string());
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

pub fn discover_local_acp_providers() -> Vec<AcpProviderInfo> {
    KNOWN_ACP_PROVIDERS
        .iter()
        .filter_map(|provider| {
            provider
                .commands
                .iter()
                .find_map(|command| find_command(command).map(|path| (*command, path)))
                .map(|(command, binary_path)| AcpProviderInfo {
                    id: provider.id.to_string(),
                    label: provider.label.to_string(),
                    command: command.to_string(),
                    binary_path: binary_path.display().to_string(),
                    default_args: normalize_agent_args(command, Vec::new()),
                    mcp_command: provider.mcp_command.map(str::to_string),
                })
        })
        .collect()
}

pub fn managed_agent_avatar_url(command: &str) -> Option<String> {
    let provider = known_acp_provider(command)?;
    Some(provider.avatar_url.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        find_via_login_shell, managed_agent_avatar_url, normalize_agent_args,
        CLAUDE_CODE_AVATAR_URL, CODEX_AVATAR_URL, GOOSE_AVATAR_URL, SPROUT_AGENT_AVATAR_URL,
    };

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
}
