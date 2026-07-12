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
    /// Legacy MCP server binary field. Vestigial — all agents now use the bundled CLI.
    /// directly. Will be removed when runtime discovery is simplified.
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
    /// Env var for normalizing `max_output_tokens`. `None` when the harness
    /// does not have a first-class env var for this field (config-file only).
    pub max_tokens_env_var: Option<&'static str>,
    /// Env var for normalizing `context_limit`. `None` when not applicable.
    pub context_limit_env_var: Option<&'static str>,
    /// Normalized field keys that must be set for this harness to function.
    /// Used by the config bridge to mark fields as required in the UI.
    /// Keys match the camelCase names used in `NormalizedConfig` (e.g. "model", "provider").
    pub required_normalized_fields: &'static [&'static str],
}

const GOOSE_AVATAR_URL: &str = "https://goose-docs.ai/img/logo_dark.png";
const CLAUDE_CODE_AVATAR_URL: &str = "https://anthropic.gallerycdn.vsassets.io/extensions/anthropic/claude-code/2.1.77/1773707456892/Microsoft.VisualStudio.Services.Icons.Default";
const CODEX_AVATAR_URL: &str = "https://openai.gallerycdn.vsassets.io/extensions/openai/chatgpt/26.5313.41514/1773706730621/Microsoft.VisualStudio.Services.Icons.Default";
const BUZZ_AGENT_AVATAR_URL: &str =
    "https://raw.githubusercontent.com/block/buzz/refs/heads/main/crates/buzz-agent/buzz-agent.png";

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
        config_file_path: Some("~/.config/goose/config.yaml"),
        config_file_format: Some("yaml"),
        supports_acp_native_config: true,
        thinking_env_var: Some("GOOSE_THINKING_EFFORT"),
        max_tokens_env_var: Some("GOOSE_MAX_TOKENS"),
        context_limit_env_var: Some("GOOSE_CONTEXT_LIMIT"),
        required_normalized_fields: &["model", "provider"],
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
        config_file_path: Some("~/.claude/settings.json"),
        config_file_format: Some("json"),
        supports_acp_native_config: false,
        thinking_env_var: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        required_normalized_fields: &[],
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
        adapter_install_commands: &["npm install -g @zed-industries/codex-acp"],
        install_instructions_url: "https://github.com/zed-industries/codex-acp",
        cli_install_hint: "Install the Codex CLI via the official install script.",
        adapter_install_hint: "Install the Codex ACP adapter via npm.",
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
        adapter_install_commands: &[],
        install_instructions_url: "https://github.com/block/buzz",
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

/// The agent command a freshly-created agent defaults to when the create
/// request supplies none. Resolves the bundled `buzz-agent` from the catalog —
/// the same shape `mesh_llm::preset` uses — so the default can't drift from the
/// provider definition. Falls back to the id if the catalog entry is missing.
///
/// The previous default was the bare global `goose`, which is not on PATH on a
/// stock Windows install: every worker failed with `program not found`. The
/// bundled `buzz-agent` ships with the app and resolves on every platform.
pub fn default_agent_command() -> String {
    known_acp_runtime_exact("buzz-agent")
        .and_then(|p| p.commands.first().copied())
        .unwrap_or("buzz-agent")
        .to_string()
}

/// Record-first harness resolution (unified agent model, Phase 1A).
///
/// Resolution order:
///   1. explicit override (non-empty) — a deliberate per-instance pin;
///   2. the record's own `runtime` id mapped to its primary command —
///      records materialize their runtime at create/migration time;
///   3. legacy fallback: the linked persona's `runtime` (records created
///      before the unified model carry `persona_id` but no `runtime`);
///   4. `default_agent_command()`.
pub fn record_agent_command(
    record: &crate::managed_agents::types::ManagedAgentRecord,
    personas: &[crate::managed_agents::types::PersonaRecord],
) -> String {
    if let Some(pin) = record
        .agent_command_override
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return pin.to_string();
    }

    if let Some(command) = record
        .runtime
        .as_deref()
        .and_then(known_acp_runtime_exact)
        .and_then(|r| r.commands.first().copied())
    {
        return command.to_string();
    }

    effective_agent_command(record.persona_id.as_deref(), personas, None)
}

/// Resolve the agent command (harness) for a spawn/deploy/summary. The linked
/// persona wins so persona harness edits propagate on the next spawn. An
/// explicit per-instance override (`agent_command_override`) takes precedence.
///
/// Resolution order:
///   1. explicit override (non-empty) — a deliberate per-instance pin;
///   2. the linked persona's `runtime` id mapped to its primary command;
///   3. `default_agent_command()` — no persona/runtime, or persona deleted.
pub fn effective_agent_command(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::PersonaRecord],
    agent_command_override: Option<&str>,
) -> String {
    if let Some(pin) = agent_command_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return pin.to_string();
    }

    persona_id
        .and_then(|pid| personas.iter().find(|p| p.id == pid))
        .and_then(|persona| persona.runtime.as_deref())
        .and_then(known_acp_runtime_exact)
        .and_then(|r| r.commands.first().copied())
        .map(str::to_string)
        .unwrap_or_else(default_agent_command)
}

mod overrides;
pub use overrides::{apply_agent_command_update, create_time_agent_command_override};

fn default_agent_args(command: &str) -> Option<Vec<String>> {
    match normalize_command_identity(command).as_str() {
        "goose" => Some(vec!["acp".to_string()]),
        "codex" | "codex-acp" | "claude-agent-acp" | "claude-code-acp" | "claude-code"
        | "claudecode" | "buzz-agent" => Some(Vec::new()),
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

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn resolve_workspace_command(command: &str) -> Option<PathBuf> {
    if command_looks_like_path(command) {
        let path = PathBuf::from(command);
        return is_executable_file(&path).then_some(path);
    }

    let file_name = executable_basename(command);
    command_search_dirs()
        .into_iter()
        .map(|dir| dir.join(&file_name))
        .find(|candidate| is_executable_file(candidate))
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
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }

    if let Some(path) = find_via_login_shell(command) {
        return Some(path);
    }
    for dir in common_binary_paths() {
        let candidate = dir.join(executable_basename(command));
        if is_executable_file(&candidate) {
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
    (path.is_absolute() && is_executable_file(&path)).then_some(path)
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

pub(crate) fn find_command(command: &str) -> Option<PathBuf> {
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

pub(crate) fn classify_runtime(
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
mod tests;
