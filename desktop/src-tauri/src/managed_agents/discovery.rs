use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::managed_agents::{
    buzz_managed_command_path, buzz_managed_node_bin_dir, buzz_managed_npm_bin_dir,
    AcpAvailabilityStatus, AcpRuntimeCatalogEntry, AuthStatus, CommandAvailabilityInfo,
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
    /// Windows-specific CLI install commands (e.g. PowerShell installers).
    /// When non-empty on Windows, these are used instead of `cli_install_commands`.
    #[allow(dead_code)] // read only on Windows via cli_install_commands_for_os()
    pub cli_install_commands_windows: &'static [&'static str],
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

const GOOSE_AVATAR_URL: &str = "https://goose-docs.ai/img/logo_dark.png";
const CLAUDE_CODE_AVATAR_URL: &str = "https://anthropic.gallerycdn.vsassets.io/extensions/anthropic/claude-code/2.1.77/1773707456892/Microsoft.VisualStudio.Services.Icons.Default";
const CODEX_AVATAR_URL: &str = "https://openai.gallerycdn.vsassets.io/extensions/openai/chatgpt/26.5313.41514/1773706730621/Microsoft.VisualStudio.Services.Icons.Default";
const BUZZ_AGENT_AVATAR_URL: &str =
    "https://raw.githubusercontent.com/block/buzz/refs/heads/main/crates/buzz-agent/buzz-agent.png";

fn common_binary_paths() -> &'static [PathBuf] {
    static PATHS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    PATHS.get_or_init(|| {
        let mut paths = vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        ];
        if let Some(managed_node_bin) = buzz_managed_node_bin_dir() {
            paths.insert(0, managed_node_bin);
        }
        if let Some(managed_bin) = buzz_managed_npm_bin_dir() {
            paths.insert(0, managed_bin);
        }
        if let Some(home) = dirs::home_dir() {
            paths.extend([
                home.join(".local/share/mise/shims"),
                home.join(".local/bin"),
                home.join(".volta/bin"),
                home.join(".asdf/shims"),
            ]);
        }
        // Windows well-known dirs for npm global shims and standalone installer targets.
        #[cfg(windows)]
        {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                paths.push(PathBuf::from(appdata).join("npm"));
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                paths.push(
                    PathBuf::from(local)
                        .join("Programs")
                        .join("OpenAI")
                        .join("Codex")
                        .join("bin"),
                );
            }
        }
        paths
    })
}

const KNOWN_ACP_RUNTIMES: &[KnownAcpRuntime] = &[
    KnownAcpRuntime {
        id: "goose",
        label: "Goose",
        // Prefer the bundled slim ACP sidecar. A full user-installed Goose
        // remains a fallback and can still be selected by explicit path.
        commands: &["goose-acp", "goose"],
        aliases: &[],
        avatar_url: GOOSE_AVATAR_URL,
        mcp_command: None,
        mcp_hooks: false,
        // Both commands are complete ACP agents; goose-acp does not depend on
        // a separate full Goose installation.
        underlying_cli: None,
        cli_install_commands: &["curl -fsSL https://github.com/block-open-source/goose/releases/download/stable/download_cli.sh | CONFIGURE=false bash"],
        cli_install_commands_windows: &[], // goose install script is already Windows-aware
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
        install_instructions_url: "https://github.com/agentclientprotocol/codex-acp",
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
        install_instructions_url: "https://github.com/block/buzz",
        cli_install_hint: "Available from Buzz source builds.",
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

/// Resolve a fresh agent's command from the bundled-Goose catalog entry,
/// falling back to its stable sidecar name if the entry is missing.
pub fn default_agent_command() -> String {
    known_acp_runtime_exact("goose")
        .and_then(|p| p.commands.first().copied())
        .unwrap_or("goose-acp")
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
    personas: &[crate::managed_agents::types::AgentDefinition],
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
    personas: &[crate::managed_agents::types::AgentDefinition],
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
        "goose-acp" | "codex" | "codex-acp" | "claude-agent-acp" | "claude-code-acp"
        | "claude-code" | "claudecode" | "buzz-agent" => Some(Vec::new()),
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

fn profile_target_dirs(root: &Path) -> [PathBuf; 2] {
    if cfg!(debug_assertions) {
        // `just dev` builds fresh debug sidecars; never prefer stale release output.
        [root.join("target/debug"), root.join("target/release")]
    } else {
        [root.join("target/release"), root.join("target/debug")]
    }
}

fn command_search_dirs() -> Vec<PathBuf> {
    let workspace_root = workspace_root_dir();
    let mut dirs = profile_target_dirs(&workspace_root).to_vec();
    dirs.extend(profile_target_dirs(&workspace_root.join("goose-acp")));
    if let Ok(current_dir) = std::env::current_dir() {
        dirs.extend(profile_target_dirs(&current_dir));
    }

    dirs.extend(
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf)),
    );
    dirs.into_iter().fold(Vec::new(), |mut unique, dir| {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
        unique
    })
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
    if let Some(managed) = resolve_buzz_managed_command(command) {
        return Some(managed);
    }

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
    // Also invalidate the adapter-availability cache so a freshly-installed
    // adapter is reflected the next time the summary builder checks the badge.
    clear_adapter_availability_cache();
}

// ── Adapter availability cache (Phase-2 badge fallback) ─────────────────────
//
// `build_managed_agent_summary` needs to compare the spawn-time adapter
// availability against the *current* availability without triggering a live
// `probe_codex_acp_major_version` subprocess on every poll cycle.  This cache
// stores the last availability status of the codex-acp binary at its resolved
// path.  It is warmed by `discover_acp_runtimes` (which already probes), so
// the badge path reads warm data, and is invalidated by `clear_resolve_cache`
// (called on every Doctor install and every `discover_acp_providers` call).

fn adapter_availability_cache() -> &'static std::sync::Mutex<Option<AcpAvailabilityStatus>> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Option<AcpAvailabilityStatus>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn clear_adapter_availability_cache() {
    if let Ok(mut guard) = adapter_availability_cache().lock() {
        *guard = None;
    }
}

/// Cache the current codex-acp adapter availability status.
///
/// Called by `discover_acp_runtimes` after it probes the codex adapter so the
/// badge path has a warm value without re-probing.
pub(crate) fn cache_adapter_availability(status: AcpAvailabilityStatus) {
    if let Ok(mut guard) = adapter_availability_cache().lock() {
        *guard = Some(status);
    }
}

/// Return the most recently cached codex-acp adapter availability, or
/// `None` if no discovery has run yet.
///
/// This is a **read from cache only** — it never spawns a subprocess.  The
/// value is populated by `discover_acp_runtimes` and invalidated by
/// `clear_resolve_cache`.  When the cache is cold, returning `None` defers
/// the drift check until discovery has produced a real value, preventing
/// a fabricated `AdapterMissing` stamp from triggering a false restart badge
/// on a newly restarted process.
pub(crate) fn adapter_availability_cached() -> Option<AcpAvailabilityStatus> {
    adapter_availability_cache()
        .lock()
        .ok()
        .and_then(|g| g.clone())
}

/// Pure predicate: does the stamped adapter availability differ from the
/// current cached availability?
///
/// Returns `false` whenever either side is `None` (unknown) — "no data" is
/// not evidence of drift.  This is extracted for unit testing without global
/// state and used by `build_managed_agent_summary`.
pub(crate) fn availability_drift(
    stamped: Option<&AcpAvailabilityStatus>,
    current: Option<AcpAvailabilityStatus>,
) -> bool {
    match (stamped, current) {
        (Some(s), Some(c)) => *s != c,
        _ => false,
    }
}

/// Return all candidate basenames for `command` on the current platform.
///
/// Always includes `executable_basename(command)` (appends `.exe` on Windows).
/// On Windows also includes `.cmd` and `.bat` variants so npm-generated shims
/// (e.g. `codex-acp.cmd` in `%APPDATA%\npm`) are discoverable.
fn command_basenames(command: &str) -> Vec<String> {
    let candidates = vec![executable_basename(command)];
    #[cfg(windows)]
    {
        let mut candidates = candidates;
        if !command.contains('.') {
            candidates.push(format!("{command}.cmd"));
            candidates.push(format!("{command}.bat"));
        }
        return candidates;
    }
    #[allow(unreachable_code)]
    candidates
}

fn resolve_buzz_managed_command(command: &str) -> Option<PathBuf> {
    let basenames = command_basenames(command);
    basenames
        .iter()
        .find_map(|basename| buzz_managed_command_path(command, basename))
}

fn resolve_command_uncached(command: &str) -> Option<PathBuf> {
    if let Some(path) = resolve_workspace_command(command) {
        return Some(path);
    }

    let basenames = command_basenames(command);

    if command_looks_like_path(command) {
        let path = PathBuf::from(command);
        return path.exists().then_some(path);
    }

    if let Some(managed) = resolve_buzz_managed_command(command) {
        return Some(managed);
    }

    for candidate in path_candidates_from_env(command) {
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }

    // On Windows, also scan PATH for .cmd/.bat shims (npm globals).
    #[cfg(windows)]
    {
        for basename in command_basenames(command).iter().skip(1) {
            for candidate in path_candidates_from_env_raw(basename) {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    if let Some(path) = find_via_login_shell(command) {
        return Some(path);
    }
    for dir in common_binary_paths() {
        for basename in &basenames {
            let candidate = dir.join(basename);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    // Check nvm's default Node.js bin directory — nvm initializes via
    // ~/.zshrc (interactive) which is not loaded by a login shell, so
    // `node`, `npm`, and npm-global shims installed there are otherwise
    // invisible.
    if let Some(home) = dirs::home_dir() {
        if let Some(nvm_bin) = find_nvm_default_bin(&home) {
            for basename in &basenames {
                let candidate = nvm_bin.join(basename);
                if is_executable_file(&candidate) {
                    return Some(candidate);
                }
            }
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

/// Like `path_candidates_from_env` but joins `basename` as-is (no `.exe` suffix).
/// Used for `.cmd`/`.bat` shim resolution on Windows.
#[cfg(windows)]
fn path_candidates_from_env_raw(basename: &str) -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(basename))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Collect login shell candidates for the current platform.
///
/// On Unix: `/bin/zsh`, `/bin/bash` (the historical defaults).
/// On Windows: Git Bash via `resolve_bash_path` — skips `BUZZ_SHELL` because
/// login-shell callers use bash-only `-l -c` syntax.
fn login_shell_candidates() -> Vec<PathBuf> {
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/bin/zsh"), PathBuf::from("/bin/bash")]
    }
    #[cfg(windows)]
    {
        super::git_bash::resolve_bash_path().into_iter().collect()
    }
}

/// Run a command in a login shell (tries zsh then bash on Unix, Git Bash on Windows).
/// Returns trimmed stdout if the command succeeds with non-empty output.
fn run_in_login_shell(args: &[&str]) -> Option<String> {
    for shell in login_shell_candidates() {
        let Ok(output) = Command::new(&shell).args(args).output() else {
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

/// Three-state backing store for the login-shell PATH cache.
#[derive(Clone)]
enum LoginShellPath {
    /// Cache has never been populated; the next call will spawn a login shell.
    Uninit,
    /// A login shell was invoked; the inner value is the PATH it returned
    /// (`None` when the shell produced no output).
    Probed(Option<String>),
}

fn path_cache() -> &'static std::sync::Mutex<LoginShellPath> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<LoginShellPath>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(LoginShellPath::Uninit))
}

fn fetch_login_shell_path_inner() -> Option<String> {
    // On Windows, Git Bash's `echo $PATH` returns POSIX colon-delimited paths
    // (`/mingw64/bin:/c/Users/...`) which poison native Windows children that
    // split on `;`. login_shell_path() feeds agent_models, runtime, and
    // cli_probe — all native processes. Return None so they inherit the real
    // Windows PATH instead.
    #[cfg(windows)]
    {
        return None;
    }

    #[cfg(not(windows))]
    {
        let stdout = run_in_login_shell(&["-l", "-c", "echo $PATH"])?;
        let last_line = stdout.lines().rfind(|l| !l.trim().is_empty())?;
        Some(last_line.trim().to_string())
    }
}

/// Return the user's full PATH from a login shell.
///
/// The result is cached after the first call. Call [`refresh_login_shell_path`]
/// to invalidate the cache so the next call re-fetches — e.g. after the user
/// installs Node.js mid-session and clicks Retry.
///
/// The lock is never held while the login shell spawns: we check for a cached
/// value, release the lock, run the shell, then re-lock to write. Two concurrent
/// callers may both run the shell (last-writer-wins is fine — both produce the
/// same result), but neither blocks a concurrent agent spawn on the Mutex.
pub fn login_shell_path() -> Option<String> {
    // Fast path: return cached result without spawning a shell.
    {
        let guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let LoginShellPath::Probed(ref result) = *guard {
            return result.clone();
        }
    }

    // Slow path: spawn shell outside any lock.
    let result = fetch_login_shell_path_inner();

    // Write back; last-writer-wins is safe here.
    {
        let mut guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
        *guard = LoginShellPath::Probed(result.clone());
    }

    result
}

/// Invalidate the login-shell PATH cache so the next [`login_shell_path`] call
/// re-fetches from a fresh login shell.
///
/// Called before every install/retry operation and on Doctor Re-run so a
/// newly-installed tool becomes visible without restarting the app.
pub(crate) fn refresh_login_shell_path() {
    let mut guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
    *guard = LoginShellPath::Uninit;
}

#[cfg(test)]
fn is_login_shell_path_uninit() -> bool {
    matches!(
        *path_cache().lock().unwrap_or_else(|e| e.into_inner()),
        LoginShellPath::Uninit
    )
}

/// Return `true` when `tag` is a safe nvm alias/version tag that can be joined
/// onto a `PathBuf` without escaping the nvm root.
///
/// nvm uses tags like `v22.1.0` or `lts/hydrogen`. We allow ASCII alphanumeric
/// plus `. - / _` and require that no path component is `..` and that the tag
/// does not start with `/` (which would replace the base in `PathBuf::join`).
fn is_safe_nvm_tag(tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    // An absolute path in the alias file would let PathBuf::join silently
    // replace the nvm root with an attacker-controlled path.
    if tag.starts_with('/') {
        return false;
    }
    // Reject any .. component to prevent upward traversal.
    for component in tag.split('/') {
        if component == ".." {
            return false;
        }
    }
    // Allow only the characters nvm uses in real tag names.
    tag.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '/' | '_'))
}

/// Locate the `bin` directory for nvm's default Node.js version.
///
/// Reads `~/.nvm/alias/default`; resolves at most one alias hop to handle
/// nvm alias chains; falls back to the highest-semver directory under
/// `~/.nvm/versions/node/`. Returns the `bin` subdirectory only when it exists.
///
/// Cheap: at most two file reads or one `read_dir`. Never cached — computed
/// fresh per call so a mid-session `nvm install` is visible at the next spawn.
pub fn find_nvm_default_bin(home: &Path) -> Option<PathBuf> {
    let nvm_root = home.join(".nvm");
    let versions_root = nvm_root.join("versions").join("node");

    // 1. Try alias/default, with at most one hop.
    let default_alias = nvm_root.join("alias").join("default");
    if let Ok(content) = std::fs::read_to_string(&default_alias) {
        let tag = content.trim().to_string();
        if is_safe_nvm_tag(&tag) {
            let candidate = versions_root.join(&tag).join("bin");
            if candidate.is_dir() {
                return Some(candidate);
            }
            // One alias hop: ~/.nvm/alias/<tag>
            let hop_file = nvm_root.join("alias").join(&tag);
            if let Ok(hop_content) = std::fs::read_to_string(&hop_file) {
                let hop_tag = hop_content.trim().to_string();
                if is_safe_nvm_tag(&hop_tag) {
                    let hop_candidate = versions_root.join(&hop_tag).join("bin");
                    if hop_candidate.is_dir() {
                        return Some(hop_candidate);
                    }
                }
            }
        }
    }

    // 2. Fall back to highest-semver directory under ~/.nvm/versions/node/.
    let entries = std::fs::read_dir(&versions_root).ok()?;
    let best = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy().into_owned();
            parse_semver_tag(&s).map(|v| (v, s))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b));

    let (_, tag) = best?;
    let bin = versions_root.join(&tag).join("bin");
    bin.is_dir().then_some(bin)
}

/// Parse a `vMAJ.MIN.PATCH` (or `vMAJ.MIN.PATCH-extra`) tag into a numeric
/// triple for semver comparison.
fn parse_semver_tag(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v')?;
    let mut parts = s.splitn(3, '.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch_str = parts.next()?;
    let patch = patch_str.split('-').next()?.parse::<u64>().ok()?;
    Some((major, minor, patch))
}

pub(crate) fn find_command(command: &str) -> Option<PathBuf> {
    resolve_command(command)
}

/// Returns true when the runtime has at least one adapter install step that
/// is an npm global install. Used to determine whether Node.js is required.
fn runtime_needs_npm(runtime: &KnownAcpRuntime) -> bool {
    runtime
        .adapter_install_commands
        .iter()
        .any(|cmd| is_npm_global_install(cmd))
}

/// Returns `true` when `cmd` is an npm global install/uninstall invocation.
///
/// Buzz rewrites these catalog commands to an app-private npm prefix before
/// execution; the global shape remains in the catalog so existing install plans
/// and Doctor's Node.js-required detection stay simple.
pub(crate) fn is_npm_global_install(cmd: &str) -> bool {
    let t = cmd.trim_start();
    t.starts_with("npm install -g ")
        || t.starts_with("npm i -g ")
        || t.starts_with("npm uninstall -g ")
}

/// Run a CLI auth probe with a 10-second process-level timeout.
///
/// Spawns the probe CLI as a child process. Stdout and stderr are drained on
/// background threads to prevent pipe-buffer deadlock. On timeout the child is
/// killed and `Unknown` is returned; no orphaned threads or processes are left
/// behind. Returns `Unknown` on timeout.
fn probe_auth_status(binary_path: &Path, probe_args: &[&str]) -> AuthStatus {
    use crate::managed_agents::readiness::cli_probe;

    let augmented_path = cli_probe::augmented_path();

    let mut command = std::process::Command::new(binary_path);
    command.args(&probe_args[1..]);
    if let Some(ref path) = augmented_path {
        command.env("PATH", path);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(_) => return AuthStatus::Unknown,
    };

    // Drain stdout/stderr on background threads to prevent pipe-buffer deadlock.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    // Save PID for kill-on-timeout before moving child into the wait thread.
    let child_pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let _ = tx.send(child.wait());
    });

    // 10-second timeout for auth probes.
    let deadline = Instant::now() + Duration::from_secs(10);
    let exit_status = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            #[cfg(unix)]
            unsafe {
                libc::kill(child_pid as i32, libc::SIGTERM);
            }
            #[cfg(not(unix))]
            let _ = child_pid;
            drop(rx);
            let _ = wait_thread.join();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return AuthStatus::Unknown;
        }
        match rx.recv_timeout(Duration::from_millis(100).min(remaining)) {
            Ok(Ok(status)) => break status,
            Ok(Err(_)) => {
                let _ = wait_thread.join();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return AuthStatus::Unknown;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return AuthStatus::Unknown;
            }
        }
    };

    let _ = wait_thread.join();
    let _ = stdout_thread.join();
    let stderr_bytes = stderr_thread.join().unwrap_or_default();

    match cli_probe::classify_probe_output(&stderr_bytes, exit_status.success()) {
        cli_probe::ProbeOutcome::LoggedIn => AuthStatus::LoggedIn,
        cli_probe::ProbeOutcome::LoggedOut => AuthStatus::LoggedOut,
        cli_probe::ProbeOutcome::ConfigInvalid { stderr_excerpt } => AuthStatus::ConfigInvalid {
            diagnostic: stderr_excerpt,
        },
    }
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

/// Probe the major version of a `codex-acp` binary by running `--version`.
///
/// The 1.x adapter (`@agentclientprotocol/codex-acp`) outputs
/// `@agentclientprotocol/codex-acp <major>.<minor>.<patch>` on stdout and exits 0.
/// The old 0.16.x adapter (`@zed-industries/codex-acp`) is a Rust binary that does
/// not recognise `--version` and exits non-zero.
///
/// Returns the major version on success, `None` on any failure (non-zero exit,
/// unparseable output, timeout, or missing binary).
///
/// The probe is bounded by a 5-second deadline. The child is polled with
/// [`std::process::Child::try_wait`] (the repo's standard deadline pattern) and
/// killed if it does not exit in time.
///
/// Stdout is redirected to a temporary file rather than a pipe, so forked
/// descendants cannot hold EOF open. Reads from a regular file return EOF at its
/// current write position regardless of inherited file descriptors, cross-platform.
pub(crate) fn probe_codex_acp_major_version(binary_path: &Path) -> Option<u64> {
    probe_codex_acp_major_version_with_path(
        binary_path,
        crate::managed_agents::readiness::cli_probe::augmented_path().as_deref(),
    )
}
pub(crate) fn probe_codex_acp_major_version_with_path(
    binary_path: &Path,
    augmented_path: Option<&str>,
) -> Option<u64> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    use std::time::{Duration, Instant};
    const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

    // A regular file returns EOF at its current size even when a descendant
    // inherits its descriptor, bounding the post-exit read cross-platform.
    let mut tmp = tempfile::tempfile().ok()?;

    let mut command = Command::new(binary_path);
    command.arg("--version");
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    let mut child = command
        .stdout(tmp.try_clone().ok()?)
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Poll until the deadline rather than blocking on stdout EOF.
    let deadline = Instant::now() + VERSION_PROBE_TIMEOUT;
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };

    if !exit_status.success() {
        return None;
    }

    // Read at most 4 KiB from the regular file without blocking.
    tmp.seek(SeekFrom::Start(0)).ok()?;
    let mut buf = Vec::with_capacity(128);
    let _ = (&mut tmp as &mut dyn std::io::Read)
        .take(4096)
        .read_to_end(&mut buf);

    let stdout = String::from_utf8_lossy(&buf);
    // Output format: "<package-name> <major>.<minor>.<patch>"
    let version_str = stdout.split_whitespace().last()?;
    let major_str = version_str.split('.').next()?;
    major_str.parse::<u64>().ok()
}

/// Classifies a resolved codex-acp binary path as [`AcpAvailabilityStatus::Available`]
/// or [`AcpAvailabilityStatus::AdapterOutdated`].
///
/// The 0.16.x adapter (`@zed-industries/codex-acp`) does not recognise `--version`
/// and exits non-zero — that probe failure yields `AdapterOutdated`. The 1.x adapter
/// (`@agentclientprotocol/codex-acp`) prints its version and exits 0; major ≥ 1
/// yields `Available`.
///
/// Used by `discover_acp_runtimes`, `cli_login_requirements`, and
/// `install_acp_runtime_blocking` so the version-gate logic is not duplicated.
pub(crate) fn codex_adapter_availability(path: &Path) -> AcpAvailabilityStatus {
    match probe_codex_acp_major_version(path) {
        Some(major) if major >= 1 => AcpAvailabilityStatus::Available,
        _ => AcpAvailabilityStatus::AdapterOutdated,
    }
}

/// Returns `true` when the codex-acp binary at `path` is outdated (major version < 1)
/// or cannot be probed. Thin wrapper around [`codex_adapter_availability`].
pub(crate) fn codex_adapter_is_outdated(path: &Path) -> bool {
    codex_adapter_availability(path) == AcpAvailabilityStatus::AdapterOutdated
}

/// Intermediate struct built before the (potentially slow) auth probe phase.
struct PartialEntry {
    runtime: &'static KnownAcpRuntime,
    entry: AcpRuntimeCatalogEntry,
}

pub fn discover_acp_runtimes() -> Vec<AcpRuntimeCatalogEntry> {
    // Phase 1: build all entries (fast — no probes yet).
    let mut partials: Vec<PartialEntry> = KNOWN_ACP_RUNTIMES
        .iter()
        .map(|runtime| {
            let adapter_result = runtime
                .commands
                .iter()
                .find_map(|command| find_command(command).map(|path| (*command, path)));

            let underlying_cli_found = runtime
                .underlying_cli
                .map(|cli| find_command(cli).is_some())
                .unwrap_or(false);
            let (mut availability, command, binary_path) =
                classify_runtime(adapter_result, runtime.underlying_cli, underlying_cli_found);

            // For codex-acp: when the adapter resolves as Available, probe the
            // version. An adapter with major version < 1 is treated as outdated —
            // the CODEX_CONFIG spawn contract requires 1.x.
            if runtime.id == "codex"
                && availability == AcpAvailabilityStatus::Available
                && command.as_deref() == Some("codex-acp")
            {
                if let Some(path_str) = &binary_path {
                    availability = codex_adapter_availability(&PathBuf::from(path_str));
                }
            }

            // Warm the adapter-availability cache for the badge fallback.
            // The cache is scoped to the codex runtime; other runtimes leave it
            // unchanged. Invalidated by `clear_resolve_cache`.
            if runtime.id == "codex" {
                cache_adapter_availability(availability.clone());
            }

            let underlying_cli_path = runtime
                .underlying_cli
                .and_then(find_command)
                .map(|p| p.display().to_string());

            let default_args = command
                .as_deref()
                .map(|cmd| normalize_agent_args(cmd, Vec::new()))
                .unwrap_or_default();

            let can_auto_install = !runtime.cli_install_commands_for_os().is_empty()
                || !runtime.adapter_install_commands.is_empty();

            let cli_hint = runtime.cli_install_hint;
            let adapter_hint = runtime.adapter_install_hint;
            let install_hint = match availability {
                AcpAvailabilityStatus::Available => cli_hint.to_string(),
                AcpAvailabilityStatus::CliMissing => cli_hint.to_string(),
                AcpAvailabilityStatus::AdapterMissing => adapter_hint.to_string(),
                AcpAvailabilityStatus::AdapterOutdated => adapter_hint.to_string(),
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

            // node_required now means Buzz cannot provide npm for this platform.
            // On supported desktop platforms, Buzz downloads a private Node/npm
            // runtime into app data before running npm-backed adapter installs.
            let node_required = matches!(
                availability,
                AcpAvailabilityStatus::AdapterMissing | AcpAvailabilityStatus::NotInstalled
            ) && runtime_needs_npm(runtime)
                && buzz_managed_node_bin_dir().is_none()
                && resolve_command("npm").is_none()
                && resolve_command("node").is_none();

            PartialEntry {
                runtime,
                entry: AcpRuntimeCatalogEntry {
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
                    node_required,
                    // Filled in by the probe phase below.
                    auth_status: AuthStatus::Unknown,
                    login_hint: None,
                },
            }
        })
        .collect();

    // Phase 2: run auth probes in parallel for entries that need them.
    // Spawn one thread per probeable entry; total cost = max(probe latency).
    let probe_handles: Vec<(usize, std::thread::JoinHandle<AuthStatus>)> = partials
        .iter()
        .enumerate()
        .filter_map(|(idx, partial)| {
            if partial.entry.availability != AcpAvailabilityStatus::Available {
                return None;
            }
            let probe_args = partial.runtime.auth_probe_args?;
            // Need the resolved binary path for the CLI (e.g. the actual `claude` binary).
            let binary_path = resolve_command(probe_args[0])?;
            let probe_args_owned: Vec<String> = probe_args.iter().map(|s| s.to_string()).collect();

            let handle = std::thread::spawn(move || {
                let refs: Vec<&str> = probe_args_owned.iter().map(String::as_str).collect();
                probe_auth_status(&binary_path, &refs)
            });
            Some((idx, handle))
        })
        .collect();

    // Collect probe results and patch entries.
    for (idx, handle) in probe_handles {
        let status = handle.join().unwrap_or(AuthStatus::Unknown);
        let partial = &mut partials[idx];
        partial.entry.login_hint =
            if matches!(status, AuthStatus::LoggedIn | AuthStatus::NotApplicable) {
                None
            } else {
                partial.runtime.login_hint.map(str::to_string)
            };
        partial.entry.auth_status = status;
    }

    // Fill NotApplicable / Unknown for non-probed entries.
    for partial in &mut partials {
        if partial.entry.auth_status == AuthStatus::Unknown {
            partial.entry.auth_status = if partial.entry.availability
                == AcpAvailabilityStatus::Available
                && partial.runtime.auth_probe_args.is_none()
            {
                AuthStatus::NotApplicable
            } else {
                AuthStatus::Unknown
            };
        }
    }

    partials.into_iter().map(|p| p.entry).collect()
}

pub fn managed_agent_avatar_url(command: &str) -> Option<String> {
    let runtime = known_acp_runtime(command)?;
    Some(runtime.avatar_url.to_string())
}

#[cfg(test)]
mod tests;
