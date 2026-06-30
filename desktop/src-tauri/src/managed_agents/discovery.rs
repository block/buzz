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
        required_normalized_fields: &[],
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
        provider_locked: false,
        default_env: &[],
        config_file_path: Some("~/.codex/config.toml"),
        config_file_format: Some("toml"),
        supports_acp_native_config: false,
        thinking_env_var: None,
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
        thinking_env_var: None,
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

/// Resolve the agent command (harness) for a spawn/deploy/summary. The linked
/// persona wins so persona harness edits propagate on the next spawn. An
/// explicit per-instance override (`agent_command_override`) takes precedence.
///
/// Resolution order:
///   1. explicit override (non-empty) — a deliberate per-instance pin;
///   2. the linked persona's `runtime` id mapped to its primary command;
///   3. the record's stored `agent_command` snapshot for legacy records whose
///      persona has no runtime field;
///   4. `default_agent_command()` — no persona/runtime/snapshot, or persona deleted.
pub fn effective_agent_command(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::PersonaRecord],
    agent_command_override: Option<&str>,
    record_agent_command: Option<&str>,
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
        .or_else(|| {
            record_agent_command
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(default_agent_command)
}

/// Decide whether a user-picked harness command is an explicit per-instance
/// pin or merely the persona's own runtime restated. Returns the override to
/// persist: `Some(picked)` when it diverges from the persona, `None` when it
/// inherits.
///
/// Comparison is by RUNTIME IDENTITY, not raw string: a persona on the `claude`
/// runtime resolves to `claude-agent-acp`, but a client with only the
/// `claude-code-acp` adapter installed sends that command instead. Both map to
/// the same `claude` runtime, so neither is a real divergence — string equality
/// would wrongly bake a pin. An unknown/custom command (no matching runtime)
/// only inherits when it exactly equals the persona command.
pub fn divergent_agent_command_override(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::PersonaRecord],
    picked_command: Option<&str>,
) -> Option<String> {
    let picked = picked_command
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let persona_command = effective_agent_command(persona_id, personas, None, None);
    let same_runtime = match (
        known_acp_runtime(picked),
        known_acp_runtime(&persona_command),
    ) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        _ => picked == persona_command,
    };
    if same_runtime {
        None
    } else {
        Some(picked.to_string())
    }
}

/// Decide the `agent_command_override` to persist at AGENT CREATE time.
///
/// A persona-backed create receives its harness command from
/// `resolvePersonaRuntime` (frontend), which produces a divergent command in two
/// distinct cases that the backend MUST tell apart:
///
/// - DELIBERATE OVERRIDE (`harness_override` true): the user explicitly picked a
///   runtime command in UI that exposes a runtime selector. This is a real pin
///   and is preserved when it differs from the command inheritance would spawn,
///   including installed aliases such as `claude-code-acp`.
/// - MISSING-RUNTIME FALLBACK (`harness_override` false): the persona's runtime
///   isn't installed locally, so `resolvePersonaRuntime` substitutes a fallback
///   default. This is NOT a pin — baking it would freeze the agent on the fallback
///   harness even after the persona's runtime is installed and the persona is
///   re-edited, the exact bug this resolver chain exists to prevent. Stores `None`
///   so the persona stays authoritative.
///
/// `isOverridden` from `resolvePersonaRuntime` cannot distinguish these — it is
/// `true` for BOTH — so the caller must thread the explicit user-intent bit.
///
/// Persona-less creates (`persona_id` is `None`, e.g. the standalone
/// CreateAgentDialog) have no persona to inherit, so the picked command is always a
/// real pin and is preserved via `divergent_agent_command_override` regardless of
/// `harness_override`.
pub fn create_time_agent_command_override(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::PersonaRecord],
    picked_command: Option<&str>,
    harness_override: bool,
) -> Option<String> {
    if persona_id.is_some() && !harness_override {
        return None;
    }

    if persona_id.is_some() && harness_override {
        let picked = picked_command
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let inherited_command = effective_agent_command(persona_id, personas, None, None);
        return (picked != inherited_command).then(|| picked.to_string());
    }

    divergent_agent_command_override(persona_id, personas, picked_command)
}

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
        classify_runtime, create_time_agent_command_override, default_agent_command,
        divergent_agent_command_override, effective_agent_command, find_via_login_shell,
        managed_agent_avatar_url, normalize_agent_args, BUZZ_AGENT_AVATAR_URL,
        CLAUDE_CODE_AVATAR_URL, CODEX_AVATAR_URL, GOOSE_AVATAR_URL,
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
    fn default_agent_command_resolves_bundled_buzz_agent() {
        // The create-path default must be the bundled buzz-agent, never the
        // bare `goose` that isn't on PATH on a stock Windows install.
        assert_eq!(default_agent_command(), "buzz-agent");
        // And buzz-agent takes no `acp` arg — confirm no arg leakage from the default.
        assert_eq!(
            normalize_agent_args(&default_agent_command(), vec!["acp".into()]),
            Vec::<String>::new()
        );
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
    fn resolves_buzz_agent_avatar() {
        assert_eq!(
            managed_agent_avatar_url("buzz-agent"),
            Some(BUZZ_AGENT_AVATAR_URL.to_string())
        );
        assert_eq!(
            managed_agent_avatar_url("/usr/local/bin/buzz-agent"),
            Some(BUZZ_AGENT_AVATAR_URL.to_string())
        );
    }

    #[test]
    fn normalizes_buzz_agent_args_to_empty() {
        assert_eq!(
            normalize_agent_args("buzz-agent", Vec::new()),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("buzz-agent", vec!["acp".into()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn login_shell_lookup_treats_command_as_data() {
        let marker =
            std::env::temp_dir().join(format!("buzz-discovery-marker-{}", uuid::Uuid::new_v4()));
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

    #[cfg(unix)]
    #[test]
    fn explicit_path_resolution_ignores_non_executable_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("buzz-discovery-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let bin = dir.join("buzz-acp");
        std::fs::write(&bin, "").expect("write placeholder");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644))
            .expect("chmod placeholder");

        assert!(
            super::resolve_workspace_command(bin.to_str().expect("utf8 path")).is_none(),
            "non-executable placeholder must not resolve"
        );

        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod executable");
        assert_eq!(
            super::resolve_workspace_command(bin.to_str().expect("utf8 path")),
            Some(bin.clone())
        );

        let _ = std::fs::remove_dir_all(dir);
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

    fn persona_with_runtime(
        id: &str,
        runtime: Option<&str>,
    ) -> crate::managed_agents::PersonaRecord {
        crate::managed_agents::PersonaRecord {
            id: id.to_string(),
            display_name: id.to_string(),
            avatar_url: None,
            system_prompt: String::new(),
            runtime: runtime.map(str::to_string),
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: std::collections::BTreeMap::new(),
            created_at: "2026-06-09T00:00:00Z".to_string(),
            updated_at: "2026-06-09T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn effective_agent_command_explicit_override_wins() {
        // An explicit pin beats the persona's runtime.
        let personas = vec![persona_with_runtime("p1", Some("claude"))];
        assert_eq!(
            effective_agent_command(Some("p1"), &personas, Some("codex-acp"), Some("goose")),
            "codex-acp"
        );
    }

    #[test]
    fn effective_agent_command_inherits_persona_runtime() {
        // No override → persona runtime id maps to its primary command.
        let personas = vec![persona_with_runtime("p1", Some("claude"))];
        assert_eq!(
            effective_agent_command(Some("p1"), &personas, None, Some("goose")),
            "claude-agent-acp"
        );
    }

    #[test]
    fn effective_agent_command_empty_override_is_inherit() {
        // A blank/whitespace override is treated as "inherit", not a pin.
        let personas = vec![persona_with_runtime("p1", Some("goose"))];
        assert_eq!(
            effective_agent_command(Some("p1"), &personas, Some("   "), Some("buzz-agent")),
            "goose"
        );
    }

    #[test]
    fn effective_agent_command_falls_back_to_record_snapshot() {
        // Legacy records may predate persona runtime fields. Preserve their
        // stored harness instead of silently changing them to the new default.
        let personas = vec![persona_with_runtime("p1", None)];
        assert_eq!(
            effective_agent_command(Some("p1"), &personas, None, Some("goose")),
            "goose"
        );
    }

    #[test]
    fn effective_agent_command_falls_back_to_default() {
        // No override, no persona runtime, no record snapshot, and a deleted
        // persona all fall back to the bundled default.
        let personas = vec![persona_with_runtime("p1", None)];
        assert_eq!(
            effective_agent_command(Some("p1"), &personas, None, None),
            default_agent_command()
        );
        assert_eq!(
            effective_agent_command(Some("gone"), &personas, None, None),
            default_agent_command()
        );
        assert_eq!(
            effective_agent_command(None, &personas, None, None),
            default_agent_command()
        );
    }

    #[test]
    fn divergent_override_none_when_picked_matches_persona_runtime() {
        // The persona-backed create/edit flow sends the persona's resolved
        // command. It must be treated as "inherit" (None), not a pin.
        let personas = vec![persona_with_runtime("p1", Some("goose"))];
        assert_eq!(
            divergent_agent_command_override(Some("p1"), &personas, Some("goose")),
            None
        );
    }

    #[test]
    fn divergent_override_none_for_alternate_command_of_same_runtime() {
        // A client with only `claude-code-acp` installed sends that command for
        // a `claude` persona whose primary command is `claude-agent-acp`. Both
        // map to the `claude` runtime, so it inherits — string equality would
        // wrongly bake a pin (CRITICAL-3).
        let personas = vec![persona_with_runtime("p1", Some("claude"))];
        assert_eq!(
            divergent_agent_command_override(Some("p1"), &personas, Some("claude-code-acp")),
            None
        );
    }

    #[test]
    fn divergent_override_some_when_picked_is_different_runtime() {
        // A deliberate pin to a different runtime is preserved.
        let personas = vec![persona_with_runtime("p1", Some("goose"))];
        assert_eq!(
            divergent_agent_command_override(Some("p1"), &personas, Some("codex-acp")),
            Some("codex-acp".to_string())
        );
    }

    #[test]
    fn divergent_override_none_for_empty_or_absent_pick() {
        // The "Inherit from persona" sentinel (empty) and a name-only edit
        // (absent) both clear the pin.
        let personas = vec![persona_with_runtime("p1", Some("goose"))];
        assert_eq!(
            divergent_agent_command_override(Some("p1"), &personas, Some("   ")),
            None
        );
        assert_eq!(
            divergent_agent_command_override(Some("p1"), &personas, None),
            None
        );
    }

    #[test]
    fn create_time_override_none_when_persona_runtime_not_installed() {
        // CRITICAL-3 (Case 3): a `claude`-persona agent created on a machine
        // where the claude adapter isn't installed. `resolvePersonaRuntime`
        // falls back to the default (`buzz-agent`) and sends THAT command with
        // `harness_override` false (the user did not pick it). At create this
        // is a fallback, not a deliberate pin — it must store `None` so the
        // agent inherits the persona's runtime once it's installed and the
        // persona is re-edited. Baking `Some("buzz-agent")` here is the exact
        // bug this resolver chain exists to kill.
        let personas = vec![persona_with_runtime("p1", Some("claude"))];
        assert_eq!(
            create_time_agent_command_override(Some("p1"), &personas, Some("buzz-agent"), false),
            None
        );
    }

    #[test]
    fn create_time_override_some_when_user_deliberately_overrides_installed_runtime() {
        // Case 2 + deliberate override: the persona's `claude` runtime IS
        // available, but the user explicitly picked `codex` in a deploy dialog's
        // runtime selector ("overriding persona preferences"), so the frontend
        // sends `codex-acp` with `harness_override` true. This is a real pin and
        // MUST be preserved — returning `None` would silently swallow the
        // deliberate override and inherit `claude` on spawn.
        let personas = vec![persona_with_runtime("p1", Some("claude"))];
        assert_eq!(
            create_time_agent_command_override(Some("p1"), &personas, Some("codex-acp"), true),
            Some("codex-acp".to_string())
        );
    }

    #[test]
    fn create_time_override_none_when_persona_runtime_installed() {
        // Case 2: the persona's runtime is available, so `resolvePersonaRuntime`
        // sends the persona's own command with no override. Inherits — no pin.
        let personas = vec![persona_with_runtime("p1", Some("goose"))];
        assert_eq!(
            create_time_agent_command_override(Some("p1"), &personas, Some("goose"), false),
            None
        );
    }

    #[test]
    fn create_time_override_preserves_selected_runtime_alias() {
        // A `claude` persona inherits the primary command `claude-agent-acp`,
        // but discovery may select an installed alias such as `claude-code-acp`.
        // When UI marks that create-time selection as explicit, preserve the
        // alias so the first spawn uses a command known to be installed.
        let personas = vec![persona_with_runtime("p1", Some("claude"))];
        assert_eq!(
            create_time_agent_command_override(
                Some("p1"),
                &personas,
                Some("claude-code-acp"),
                true
            ),
            Some("claude-code-acp".to_string())
        );
    }

    #[test]
    fn create_time_override_inherits_exact_persona_command() {
        let personas = vec![persona_with_runtime("p1", Some("claude"))];
        assert_eq!(
            create_time_agent_command_override(
                Some("p1"),
                &personas,
                Some("claude-agent-acp"),
                true
            ),
            None
        );
    }

    #[test]
    fn create_time_override_preserves_pin_for_persona_less_create() {
        // The standalone CreateAgentDialog creates persona-LESS agents. With no
        // persona to inherit, the picked command IS the agent's harness and must
        // be preserved as a real pin (divergence from the bundled default),
        // regardless of the override flag.
        let personas = vec![persona_with_runtime("p1", Some("goose"))];
        assert_eq!(
            create_time_agent_command_override(None, &personas, Some("codex-acp"), false),
            Some("codex-acp".to_string())
        );
    }
}
