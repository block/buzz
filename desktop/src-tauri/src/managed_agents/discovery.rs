use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use crate::managed_agents::{
    buzz_managed_command_path, buzz_managed_node_bin_dir, buzz_managed_npm_bin_dir,
    AcpAvailabilityStatus, AcpRuntimeCatalogEntry, AuthStatus, CommandAvailabilityInfo,
    HarnessSource,
};
mod auth_status_cache;
mod bounded_command;
mod login_shell;
mod presets;
mod runtime_metadata;
#[macro_use]
mod windows_install;
mod catalog;
pub(crate) use catalog::KNOWN_ACP_RUNTIMES;
pub use login_shell::{find_nvm_default_bin, login_shell_path};
pub(crate) use login_shell::{find_via_login_shell, refresh_login_shell_path};
#[cfg(test)]
pub(crate) use login_shell::{
    is_login_shell_path_uninit, is_safe_nvm_tag, login_shell_candidates, parse_semver_tag,
};
pub(crate) use presets::{
    canonical_harness_command, command_for_runtime_id, preset_harness_definitions,
    preset_harness_ids,
};
use presets::{preset_catalog_entry, PRESET_HARNESSES};
pub(crate) use runtime_metadata::EffortNormalization;
pub(crate) use runtime_metadata::KnownAcpRuntime;
#[cfg(test)]
pub(crate) use runtime_metadata::GOOSE_EFFORT_NORMALIZATION;

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
                home.join(".bun/bin"),
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
            // Goose's legacy Windows installer (superseded by #2680) unpacked
            // to %USERPROFILE%\goose\goose.exe, which is on no standard PATH —
            // without this probe those installs stay permanently undiscovered.
            if let Some(profile) = std::env::var_os("USERPROFILE") {
                paths.push(PathBuf::from(profile).join("goose"));
            }
        }
        paths
    })
}

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

pub(crate) fn normalize_command_identity(command: &str) -> String {
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
/// request supplies none. Resolves the bundled `buzz-agent` from the catalog so
/// the default cannot drift from the provider definition. Falls back to the id
/// if the catalog entry is missing. (Previous default was bare `goose`, which
/// is not on PATH on a stock Windows install; buzz-agent ships with the app.)
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
///   2. the record's own `runtime` id mapped to its primary command via the
///      authoritative three-tier lookup (static builtins → static preset list
///      → loaded registry) — preset harnesses (e.g. openclaw) resolve
///      correctly even with a cold registry;
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

    if let Some(id) = record.runtime.as_deref() {
        // Three-tier lookup: static builtins → static presets → loaded registry.
        // Using the shared resolver ensures preset harnesses (e.g. openclaw)
        // resolve correctly even without a warm registry.
        if let Some(cmd) = presets::command_for_runtime_id(id) {
            return cmd;
        }
    }

    effective_agent_command(record.persona_id.as_deref(), personas, None)
}

/// Resolve the agent command (harness) for a spawn/deploy/summary. The linked
/// persona wins so persona harness edits propagate on the next spawn. An
/// explicit per-instance override (`agent_command_override`) takes precedence.
///
/// Resolution order:
///   1. explicit override (non-empty) — a deliberate per-instance pin;
///   2. the linked persona's `runtime` id mapped to its primary command via
///      the authoritative three-tier lookup (static builtins → static preset
///      list → loaded registry);
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

    let runtime_id = persona_id
        .and_then(|pid| personas.iter().find(|p| p.id == pid))
        .and_then(|persona| persona.runtime.as_deref());

    if let Some(id) = runtime_id {
        // Three-tier lookup: static builtins → static presets → loaded registry.
        if let Some(cmd) = presets::command_for_runtime_id(id) {
            return cmd;
        }
    }

    default_agent_command()
}

mod overrides;
pub use overrides::remove_record_effort_aliases;
pub use overrides::{
    apply_agent_command_update, apply_env_vars_then_effort_transition,
    create_time_agent_command_override,
};

/// Prefix of the typed dangling-harness error produced by
/// `try_record_agent_command` / `resolve_effective_harness_descriptor`.
/// Internal Rust contract: surfaces must convert it via [`user_facing_harness_error`] or
/// [`dangling_harness_id`] — never show it raw.
pub(crate) const DANGLING_HARNESS_PREFIX: &str = "DANGLING_HARNESS_ID:";

/// Extract the missing harness id from a `DANGLING_HARNESS_ID:<id>` error.
/// Returns `None` for any other error string.
pub(crate) fn dangling_harness_id(error: &str) -> Option<&str> {
    error.strip_prefix(DANGLING_HARNESS_PREFIX)
}

/// Convert a harness-resolution error to a user-facing sentence. Dangling
/// harness ids become an actionable message; other errors pass through.
pub(crate) fn user_facing_harness_error(error: &str) -> String {
    match dangling_harness_id(error) {
        Some(id) => format!(
            "harness \"{id}\" was deleted — pick a new harness for this agent or restore the harness definition"
        ),
        None => error.to_string(),
    }
}

/// Summary-row display for a dangling harness id: shows the *missing* id so the agent list
/// tells the same story as spawn rather than silently falling back to the default command.
pub(crate) fn dangling_harness_display(id: &str) -> String {
    format!("harness (deleted): {id}")
}

/// Spawn-time variant of `record_agent_command` that returns a typed error when
/// a record's `runtime` id or persona's `runtime` id is set but unresolvable
/// (definition deleted after agent was created). Returns `Err("DANGLING_HARNESS_ID:<id>")`.
/// When there is no runtime id at all, falls through to `default_agent_command()` intentionally.
pub fn try_record_agent_command(
    record: &crate::managed_agents::types::ManagedAgentRecord,
    personas: &[crate::managed_agents::types::AgentDefinition],
) -> Result<String, String> {
    // Explicit pin always wins — if the user set a raw override, honour it.
    if let Some(pin) = record
        .agent_command_override
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Ok(pin.to_string());
    }

    // Record-level runtime id: if set but unresolvable → typed error.
    if let Some(id) = record.runtime.as_deref() {
        if let Some(cmd) = presets::command_for_runtime_id(id) {
            return Ok(cmd);
        }
        return Err(format!("DANGLING_HARNESS_ID:{id}"));
    }

    // Persona-level runtime id.
    if let Some(persona_id) = record.persona_id.as_deref() {
        if let Some(persona) = personas.iter().find(|p| p.id == persona_id) {
            if let Some(id) = persona.runtime.as_deref() {
                if let Some(cmd) = presets::command_for_runtime_id(id) {
                    return Ok(cmd);
                }
                return Err(format!("DANGLING_HARNESS_ID:{id}"));
            }
        }
    }

    // No runtime id set — legacy agent; use the safe default.
    Ok(default_agent_command())
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

fn profile_target_dirs(root: &Path) -> [PathBuf; 2] {
    if cfg!(debug_assertions) {
        // `just dev` builds fresh debug sidecars; never prefer stale release output.
        [root.join("target/debug"), root.join("target/release")]
    } else {
        [root.join("target/release"), root.join("target/debug")]
    }
}

fn command_search_dirs() -> Vec<PathBuf> {
    let mut dirs = profile_target_dirs(&workspace_root_dir()).to_vec();
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

    // Slow path: resolve and cache. Negative results are cached too: an absent
    // command must not re-run `resolve_command_uncached` (which spawns a login
    // shell via `find_via_login_shell`) on every cheap discovery — that spawn
    // on the channel-switch/composer hot path is exactly what this cache exists
    // to prevent. `clear_resolve_cache` (run by every forced discovery) is the
    // invalidation seam, so a newly-installed binary is still found on refresh.
    let result = resolve_command_uncached(command);

    if let Ok(mut guard) = cache.lock() {
        guard.insert(command.to_string(), result.clone());
    }

    result
}

/// Cache-only command resolution for the cheap discovery path.
///
/// Consults the Buzz-managed shim dir (a filesystem stat, never a spawn) and
/// the resolve cache; on a miss it reports the command absent rather than
/// resolving live via `resolve_command_uncached` → `find_via_login_shell`,
/// which spawns a login shell on the channel-switch / composer hot path — the
/// freeze the cheap path exists to avoid. `resolve_command` (the forced path)
/// is the sole prober and cache populator.
pub fn resolve_command_cached(command: &str) -> Option<PathBuf> {
    if let Some(managed) = resolve_buzz_managed_command(command) {
        return Some(managed);
    }
    // Bundled sidecars (e.g. `buzz-agent`) ship next to the app executable, so
    // `resolve_workspace_command` finds them with a filesystem stat and no
    // login-shell spawn — the same class of work the managed-shim check above
    // already performs. Without this the cheap path could never see the sidecar
    // until a forced discovery warmed the resolve cache, so `buzz-agent` (which
    // cannot legitimately be missing) reported "not installed" at every cold
    // launch across the create/edit and agent-defaults surfaces.
    if let Some(workspace) = resolve_workspace_command(command) {
        return Some(workspace);
    }
    resolve_cache()
        .lock()
        .ok()
        .and_then(|guard| guard.get(command).cloned())
        .flatten()
}

/// Clear the resolve_command cache so that newly-installed binaries are detected.
pub fn clear_resolve_cache() {
    let mut guard = resolve_cache().lock().unwrap_or_else(|e| e.into_inner());
    guard.clear();
    // Also invalidate the adapter-availability cache so a freshly-installed
    // adapter is reflected the next time the summary builder checks the badge.
    clear_adapter_availability_cache();
    // And the auth-status cache so a forced re-discovery re-probes rather than
    // reusing stale login state.
    auth_status_cache::clear();
}

// ── Adapter availability cache (Phase-2 badge fallback) ─────────────────────
//
// `build_managed_agent_summary` needs to compare the spawn-time adapter
// availability against the *current* availability without triggering a live
// `probe_codex_acp_version` subprocess on every poll cycle.  This cache
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

/// Test-only counter for login-shell spawn attempts (see submodule).
#[cfg(test)]
#[path = "discovery/login_shell_spawn_probe.rs"]
pub(crate) mod login_shell_spawn_probe;

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
/// On timeout or spawn failure the child is killed and `Unknown` is returned;
/// no orphaned threads or processes are left behind (see
/// [`bounded_command::output_with_timeout`]).
fn probe_auth_status(binary_path: &Path, probe_args: &[&str]) -> AuthStatus {
    use crate::managed_agents::readiness::cli_probe;

    let augmented_path = cli_probe::augmented_path();

    let mut command = std::process::Command::new(binary_path);
    command.args(&probe_args[1..]);
    if let Some(ref path) = augmented_path {
        command.env("PATH", path);
    }
    // Window suppression is owned by `output_with_timeout`'s spawn
    // (`BOUNDED_CREATION_FLAGS` carries `CREATE_NO_WINDOW`); a
    // `configure_no_window` call here would be clobbered by that later
    // `creation_flags` set, so it is deliberately omitted.

    let Some(output) = bounded_command::output_with_timeout(command, Duration::from_secs(10))
    else {
        return AuthStatus::Unknown;
    };

    match cli_probe::classify_probe_output(&output.stderr, output.status.success()) {
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
        "{role} `{command}` was not found. Make sure it is installed and on your PATH. Antivirus software can quarantine bundled binaries — if that happened, restore the file or reinstall Buzz. (Source builds: see TESTING.md.)"
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

/// The oldest `codex-acp` version supported by Buzz managed agents.
///
/// Older 1.x adapters are detected successfully, but can still bundle a Codex runtime
/// that does not reliably give `buzz` CLI subprocesses outbound relay access.
///
/// Bump policy: raise this only when a newer adapter fixes a defect that breaks managed
/// agents, and only to a version already published on npm — every user below the floor is
/// offered a reinstall on their next discovery pass.
pub(crate) const MIN_CODEX_ACP_VERSION: (u64, u64, u64) = (1, 1, 7);

/// Probe the full version of a `codex-acp` binary by running `--version`.
///
/// The 1.x adapter (`@agentclientprotocol/codex-acp`) outputs
/// `@agentclientprotocol/codex-acp <major>.<minor>.<patch>` on stdout and exits 0.
/// The old 0.16.x adapter (`@zed-industries/codex-acp`) is a Rust binary that does
/// not recognise `--version` and exits non-zero.
///
/// Returns the `(major, minor, patch)` triple on success, `None` on any failure
/// (non-zero exit, unparseable output, timeout, or missing binary).
///
/// The parse is deliberately strict: exactly three numeric dot-separated components.
/// Partial versions (`1.2`) and prerelease tags (`1.2.0-rc1`) return `None` and so
/// classify as [`AcpAvailabilityStatus::AdapterOutdated`] — failing closed offers a
/// reinstall rather than running an adapter whose version cannot be compared.
///
/// The probe is bounded by a 5-second deadline. The child is polled with
/// [`std::process::Child::try_wait`] (the repo's standard deadline pattern) and
/// killed if it does not exit in time.
///
/// Stdout is redirected to a temporary file rather than a pipe, so forked
/// descendants cannot hold EOF open. Reads from a regular file return EOF at its
/// current write position regardless of inherited file descriptors, cross-platform.
pub(crate) fn probe_codex_acp_version(binary_path: &Path) -> Option<(u64, u64, u64)> {
    probe_codex_acp_version_with_path(
        binary_path,
        crate::managed_agents::readiness::cli_probe::augmented_path().as_deref(),
    )
}
pub(crate) fn probe_codex_acp_version_with_path(
    binary_path: &Path,
    augmented_path: Option<&str>,
) -> Option<(u64, u64, u64)> {
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
    crate::util::configure_no_window(&mut command);
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
    let mut components = version_str.split('.');
    let major = components.next()?.parse::<u64>().ok()?;
    let minor = components.next()?.parse::<u64>().ok()?;
    let patch = components.next()?.parse::<u64>().ok()?;
    if components.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Classifies a resolved codex-acp binary path as [`AcpAvailabilityStatus::Available`]
/// or [`AcpAvailabilityStatus::AdapterOutdated`].
///
/// The 0.16.x adapter (`@zed-industries/codex-acp`) does not recognise `--version`
/// and exits non-zero — that probe failure yields `AdapterOutdated`. An adapter is
/// available only when its version is at least [`MIN_CODEX_ACP_VERSION`].
///
/// Used by `discover_acp_runtimes`, `cli_login_requirements`, and
/// `install_acp_runtime_blocking` so the version-gate logic is not duplicated.
pub(crate) fn codex_adapter_availability(path: &Path) -> AcpAvailabilityStatus {
    match probe_codex_acp_version(path) {
        Some(version) if version >= MIN_CODEX_ACP_VERSION => AcpAvailabilityStatus::Available,
        _ => AcpAvailabilityStatus::AdapterOutdated,
    }
}

/// Returns `true` when the codex-acp binary at `path` is below
/// [`MIN_CODEX_ACP_VERSION`] or cannot be probed using `augmented_path`. Thin wrapper
/// around [`codex_adapter_is_outdated_with_path`].
#[cfg(test)]
pub(crate) fn codex_adapter_is_outdated(path: &Path) -> bool {
    codex_adapter_is_outdated_with_path(
        path,
        crate::managed_agents::readiness::cli_probe::augmented_path().as_deref(),
    )
}

/// Returns `true` when the codex-acp binary at `path` is below
/// [`MIN_CODEX_ACP_VERSION`] or cannot be probed with the supplied PATH.
pub(crate) fn codex_adapter_is_outdated_with_path(
    path: &Path,
    augmented_path: Option<&str>,
) -> bool {
    !matches!(
        probe_codex_acp_version_with_path(path, augmented_path),
        Some(version) if version >= MIN_CODEX_ACP_VERSION
    )
}

/// Intermediate struct built before the (potentially slow) auth probe phase.
struct PartialEntry {
    runtime: &'static KnownAcpRuntime,
    entry: AcpRuntimeCatalogEntry,
}

fn discover_acp_runtime_phase1(runtime: &'static KnownAcpRuntime, force: bool) -> PartialEntry {
    // Cheap path is cache-only (no login-shell spawn); forced path resolves live.
    let resolve = if force {
        resolve_command
    } else {
        resolve_command_cached
    };
    let adapter_result = runtime
        .commands
        .iter()
        .find_map(|command| resolve(command).map(|path| (*command, path)));

    let underlying_cli_found = runtime
        .underlying_cli
        .map(|cli| resolve(cli).is_some())
        .unwrap_or(false);
    let (mut availability, command, binary_path) =
        classify_runtime(adapter_result, runtime.underlying_cli, underlying_cli_found);

    // For codex-acp: when the adapter resolves as Available, determine its full
    // version. A forced discovery probes the binary (spawns a subprocess); the
    // cheap default path reuses the last cached availability so it stays
    // process-free. An adapter below MIN_CODEX_ACP_VERSION is treated as outdated.
    if runtime.id == "codex"
        && availability == AcpAvailabilityStatus::Available
        && command.as_deref() == Some("codex-acp")
    {
        if force {
            if let Some(path_str) = &binary_path {
                availability = codex_adapter_availability(&PathBuf::from(path_str));
            }
        } else if let Some(cached) = adapter_availability_cached() {
            availability = cached;
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
        .and_then(resolve)
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
    let install_instructions_url = match availability {
        AcpAvailabilityStatus::AdapterMissing | AcpAvailabilityStatus::AdapterOutdated => {
            runtime.adapter_install_instructions_url
        }
        AcpAvailabilityStatus::Available
        | AcpAvailabilityStatus::CliMissing
        | AcpAvailabilityStatus::NotInstalled => runtime.cli_install_instructions_url,
    };

    // node_required now means Buzz cannot provide npm for this platform.
    // On supported desktop platforms, Buzz downloads a private Node/npm
    // runtime into app data before running npm-backed adapter installs.
    let node_required = matches!(
        availability,
        AcpAvailabilityStatus::AdapterMissing | AcpAvailabilityStatus::NotInstalled
    ) && runtime_needs_npm(runtime)
        && buzz_managed_node_bin_dir().is_none()
        && resolve("npm").is_none()
        && resolve("node").is_none();

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
            model_env_var: runtime.model_env_var.map(str::to_string),
            provider_env_var: runtime.provider_env_var.map(str::to_string),
            thinking_env_var: runtime.thinking_env_var.map(str::to_string),
            effort_canonical_values: runtime
                .effort_normalization
                .map(|norm| norm.canonical.iter().map(|s| s.to_string()).collect()),
            max_tokens_env_var: runtime.max_tokens_env_var.map(str::to_string),
            context_limit_env_var: runtime.context_limit_env_var.map(str::to_string),
            max_rounds_env_var: runtime.max_rounds_env_var.map(str::to_string),
            install_hint,
            install_instructions_url: install_instructions_url.to_string(),
            can_auto_install,
            requires_external_cli: runtime.underlying_cli.is_some(),
            underlying_cli_path,
            node_required,
            // Filled in by the auth-probe phase in full catalog discovery.
            auth_status: AuthStatus::Unknown,
            login_hint: None,
            source: HarnessSource::Builtin,
            definition_env: Default::default(),
            max_parallelism: super::parallelism::harness_max_parallelism(runtime.id),
        },
    }
}

/// Discover one runtime's filesystem availability without running any auth probes.
///
/// Post-install verification only needs to know whether the requested runtime
/// resolves, so it should not pay the cost of authenticating every catalog entry.
pub(crate) fn discover_acp_runtime_availability(runtime_id: &str) -> Option<AcpAvailabilityStatus> {
    known_acp_runtime_exact(runtime_id)
        // Post-install verification wants fresh filesystem/version state, so
        // probe rather than trust the cheap-path cache.
        .map(|runtime| discover_acp_runtime_phase1(runtime, true))
        .map(|partial| partial.entry.availability)
}

/// Discover all ACP runtimes, optionally merging user-defined custom harnesses
/// from `custom_harnesses_dir`.
///
/// This is the primary entry point used by the Tauri command layer. It:
/// 1. Builds entries for all compiled-in (`Builtin`) runtimes.
/// 2. Runs auth probes in parallel.
/// 3. Inserts static `Preset` entries (PATH-probed, `source: Preset`).
/// 4. If `custom_harnesses_dir` is `Some`, loads `*.json` files from that
///    directory and appends `Custom` entries — no auth probe, command resolved
///    via PATH, availability is `Available` or `NotInstalled`.
///
/// The custom dir is re-scanned on every call (goose `refresh_custom_providers`
/// pattern) — no caching, no restart needed to pick up new files.
///
/// After building the catalog, updates the loaded-harness registry so spawn
/// and readiness paths can resolve preset/custom harness commands without
/// re-running discovery.
pub fn discover_acp_runtimes_from(
    custom_harnesses_dir: Option<&Path>,
    force: bool,
) -> Vec<AcpRuntimeCatalogEntry> {
    // Cheap path is cache-only (no login-shell spawn); forced path resolves live.
    let resolve = if force {
        resolve_command
    } else {
        resolve_command_cached
    };

    // Phase 1: build all builtin entries (fast — no probes yet).
    let mut partials: Vec<PartialEntry> = KNOWN_ACP_RUNTIMES
        .iter()
        .map(|runtime| discover_acp_runtime_phase1(runtime, force))
        .collect();

    // Phase 2: resolve each available runtime's auth status (forced discovery
    // spawns parallel CLI probes and warms the cache; the cheap path reuses it).
    auth_status_cache::resolve_auth_statuses(&mut partials, force);

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

    let mut entries: Vec<AcpRuntimeCatalogEntry> = partials.into_iter().map(|p| p.entry).collect();

    // Track all ids seen so far (builtins) to prevent preset/custom collisions.
    let mut seen_ids: std::collections::HashSet<String> =
        entries.iter().map(|e| e.id.clone()).collect();

    // Phase 2.5: insert static preset entries (PATH-probed, not editable/deletable).
    for def in PRESET_HARNESSES {
        if seen_ids.contains(def.id) {
            // Builtin or earlier preset shadowed this id — skip silently.
            continue;
        }
        seen_ids.insert(def.id.to_string());

        entries.push(preset_catalog_entry(def, resolve));
    }

    // Phase 3: load and append custom harness definitions.
    if let Some(dir) = custom_harnesses_dir {
        // The loader applies collision + duplicate filtering at the boundary,
        // so anything it returns is safe to surface in the catalog. Builtin
        // shadowing is impossible here (check_id_collision covers builtins and
        // presets); `seen_ids` guards only same-run duplicates.
        for def in crate::managed_agents::custom_harnesses::load_custom_harnesses(dir) {
            if !seen_ids.insert(def.id.clone()) {
                tracing::warn!("custom_harnesses: skipping duplicate id {:?}", def.id);
                continue;
            }

            // Availability: command resolves → Available, else NotInstalled.
            let (availability, command, binary_path) = match resolve(&def.command) {
                Some(path) => (
                    AcpAvailabilityStatus::Available,
                    Some(def.command.clone()),
                    Some(path.display().to_string()),
                ),
                None => (AcpAvailabilityStatus::NotInstalled, None, None),
            };

            let default_args = normalize_agent_args(&def.command, def.args.clone());

            entries.push(AcpRuntimeCatalogEntry {
                id: def.id.clone(),
                label: def.label.clone(),
                // F1 security fix: never copy user-supplied avatar URL into the catalog.
                // All icons are bundled assets; customs fall back to TerminalSquare in the UI.
                avatar_url: String::new(),
                availability,
                command,
                binary_path,
                default_args,
                // Custom harnesses are plain ACP — no MCP sidecar, no env-var
                // model switching, no thinking knobs.
                mcp_command: None,
                model_env_var: None,
                provider_env_var: None,
                thinking_env_var: None,
                effort_canonical_values: None,
                max_tokens_env_var: None,
                context_limit_env_var: None,
                max_rounds_env_var: None,
                install_hint: def.install_hint.clone(),
                install_instructions_url: def.install_instructions_url.clone(),
                // Security line: custom definitions carry no install scripts.
                can_auto_install: false,
                requires_external_cli: false,
                underlying_cli_path: None,
                node_required: false,
                // No auth probe for custom harnesses.
                auth_status: AuthStatus::NotApplicable,
                login_hint: None,
                source: HarnessSource::Custom,
                definition_env: def.env.clone(), // preserve for edit round-trip
                max_parallelism: super::parallelism::harness_max_parallelism(&def.command),
            });
        }
    }

    // Publish the loaded-harness registry from a FRESH directory read under the
    // persist mutex — never from the snapshot taken before the auth probes ran.
    // A save/delete landing during Phase 2 already re-warmed the registry; a
    // stale-snapshot publish here would clobber it (the just-saved harness
    // would become unresolvable at spawn until the next discovery).
    //
    // This exact line is pinned by `discovery_publish_path_survives_mid_flight_save`
    // / `..._drops_mid_flight_delete` (discovery tests), which land a save/delete
    // through the pre-publish test hook below and red if this reverts to
    // publishing a stale snapshot.
    #[cfg(test)]
    pre_publish_test_hook::run();
    crate::managed_agents::custom_harnesses::warm_harness_registry_locked(custom_harnesses_dir);

    entries
}

/// Test-only seam: a callback invoked between discovery's directory scan and
/// its registry publish, so tests can land a `save_and_warm`/`delete_and_warm`
/// in exactly the window the stale-snapshot bug lived in — through the REAL
/// `discover_acp_runtimes_from` call path, not a hand-called seam.
#[cfg(test)]
pub(crate) mod pre_publish_test_hook {
    use std::sync::{Mutex, OnceLock};

    type Hook = Box<dyn Fn() + Send>;

    fn cell() -> &'static Mutex<Option<Hook>> {
        static CELL: OnceLock<Mutex<Option<Hook>>> = OnceLock::new();
        CELL.get_or_init(|| Mutex::new(None))
    }

    /// Install (or clear, with `None`) the hook. Callers must serialize via
    /// `registry_test_lock` — the hook is process-global.
    pub(crate) fn set(hook: Option<Hook>) {
        *cell().lock().unwrap_or_else(|e| e.into_inner()) = hook;
    }

    pub(crate) fn run() {
        if let Some(hook) = cell().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            hook();
        }
    }
}

pub fn managed_agent_avatar_url(command: &str) -> Option<String> {
    let runtime = known_acp_runtime(command)?;
    Some(runtime.avatar_url.to_string())
}

#[cfg(test)]
mod tests;
