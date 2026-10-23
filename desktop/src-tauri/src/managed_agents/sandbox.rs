//! Launch desktop-managed agents inside the nono sandbox.
//!
//! A sandboxed spawn runs `nono wrap --profile <nest>-agent-local -- buzz-acp`.
//! `wrap` applies the sandbox and then `exec`s the harness, so the PID,
//! process group, process name, and environment are those of `buzz-acp` —
//! receipts, ownership checks, the orphan sweep, and the process-group stop
//! path keep working unchanged. Everything the harness starts (agent CLI, MCP
//! servers, shell commands) inherits the sandbox.
//!
//! Static grants live in the profiles (see [`super::sandbox_profile`]); grants
//! that depend on this launch (binary locations, a symlinked REPOS directory,
//! demo config roots) are passed on the command line.
//!
//! A sandboxed spawn fails closed: a missing sidecar, missing profiles, or an
//! invalid user profile refuse the spawn with an actionable message. The only
//! way to run unsandboxed is the explicit `disable_agent_sandbox` setting.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::global_config::GlobalAgentConfig;

/// Env var prefix nono reads as command-line flags.
const NONO_ENV_PREFIX: &str = "NONO_";

/// Tells `buzz-acp` it runs inside nono, so it can stop agents from applying
/// a nested sandbox that macOS refuses (see `outer_sandbox_agent_env` in
/// `buzz-acp`). Reserved in user env.
pub(crate) const OUTER_SANDBOX_ENV: &str = "BUZZ_ACP_OUTER_SANDBOX";

/// App-data identifier `buzz channel-templates` reads templates under. The CLI
/// always uses the production identifier (`crates/buzz-cli/src/commands/
/// channel_templates.rs`), whatever desktop build launched it.
const CLI_TEMPLATES_BUNDLE_IDENTIFIER: &str = "xyz.block.buzz.app";

/// Bound on `nono profile validate`, which only parses local JSON.
#[cfg(unix)]
const VALIDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether a spawn under `global` runs sandboxed. Shared by the spawn path and
/// the prospective spawn snapshot so toggling the setting raises the restart
/// badge.
pub(crate) fn sandbox_requested(global: &GlobalAgentConfig) -> bool {
    cfg!(unix) && !global.disable_agent_sandbox
}

/// A resolved, validated sandbox for one spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSandbox {
    nono: PathBuf,
    profile: String,
}

/// Per-launch filesystem grants passed to `nono wrap`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SandboxGrants {
    /// Read-only directories.
    pub read: Vec<PathBuf>,
    /// Read-write directories.
    pub allow: Vec<PathBuf>,
    /// Read-only files.
    pub read_files: Vec<PathBuf>,
}

/// Resolve the sandbox for a spawn: `Ok(None)` when the user turned it off or
/// the platform has no sandbox, `Err` when it is on but cannot be applied.
pub(crate) fn prepare_agent_sandbox(
    global: &GlobalAgentConfig,
) -> Result<Option<AgentSandbox>, String> {
    if !sandbox_requested(global) {
        return Ok(None);
    }
    prepare_enabled_sandbox().map(Some)
}

#[cfg(unix)]
fn prepare_enabled_sandbox() -> Result<AgentSandbox, String> {
    let nono = bundled_nono().ok_or(
        "the agent sandbox (nono) is missing from this Buzz install; reinstall Buzz, \
         or turn off the agent sandbox in agent settings",
    )?;
    require_nest_workdir()?;
    // Re-run the idempotent install so a failed boot install is retried here
    // rather than leaving every spawn refused.
    let names = super::sandbox_profile::ensure_sandbox_profiles()
        .map_err(|e| format!("cannot install the agent sandbox profiles: {e}"))?;
    validate_profile(&nono, &names.local)?;
    Ok(AgentSandbox {
        nono,
        profile: names.local,
    })
}

#[cfg(not(unix))]
fn prepare_enabled_sandbox() -> Result<AgentSandbox, String> {
    Err("the agent sandbox is not supported on this platform".to_string())
}

/// The nono sidecar next to the app executable. Never resolved from `PATH`,
/// so the pinned version is what runs. Dev builds without a fetched sidecar
/// carry a zero-byte, non-executable stub, which is treated as missing.
#[cfg(unix)]
fn bundled_nono() -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::current_exe().ok()?.parent()?.join("nono");
    let metadata = path.metadata().ok()?;
    (metadata.is_file() && metadata.len() > 0 && metadata.permissions().mode() & 0o111 != 0)
        .then_some(path)
}

/// The profile's workdir grant covers the spawn's current directory, which
/// must be the nest. `default_agent_workdir` falls back to `$HOME` when the
/// nest is missing, and sharing all of `$HOME` read-write would defeat the
/// sandbox.
#[cfg(unix)]
fn require_nest_workdir() -> Result<(), String> {
    let workdir = super::default_agent_workdir();
    let nest = super::nest_dir();
    match (workdir, nest) {
        (Some(workdir), Some(nest)) if workdir == nest => Ok(()),
        _ => Err(
            "the agent sandbox needs the Buzz nest directory, which could not be \
                  created; check the desktop log"
                .to_string(),
        ),
    }
}

#[cfg(unix)]
fn validate_profile(nono: &Path, profile: &str) -> Result<(), String> {
    let mut command = Command::new(nono);
    command.args(["profile", "validate", profile]);
    strip_nono_env(&mut command);
    let output = super::discovery::output_with_timeout(command, VALIDATE_TIMEOUT)
        .ok_or_else(|| format!("validating the agent sandbox profile `{profile}` timed out"))?;
    if output.status.success() {
        return Ok(());
    }
    let report = strip_ansi_escapes::strip_str(String::from_utf8_lossy(&output.stdout));
    let errors: Vec<&str> = report
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("[err]"))
        .collect();
    let detail = if errors.is_empty() {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    } else {
        errors.join("; ")
    };
    let file = super::sandbox_profile::nono_profiles_dir()
        .map(|dir| dir.join(format!("{profile}.json")).display().to_string())
        .unwrap_or_else(|| format!("{profile}.json"));
    Err(format!(
        "the agent sandbox profile `{profile}` is invalid ({detail}); fix {file}, \
         or delete it to restore the default"
    ))
}

/// Create the spawn command: `nono` when sandboxed, the harness otherwise.
/// A sandboxed command must be finished with [`finish_sandboxed_command`].
pub(crate) fn agent_command(sandbox: Option<&AgentSandbox>, acp_command: &Path) -> Command {
    match sandbox {
        Some(sandbox) => {
            let mut command = Command::new(&sandbox.nono);
            command.env(OUTER_SANDBOX_ENV, "nono");
            command
        }
        None => {
            let mut command = Command::new(acp_command);
            command.env_remove(OUTER_SANDBOX_ENV);
            command
        }
    }
}

/// Append the `nono wrap` arguments, after all env has been set. Must be the
/// last change to the command: everything after `--` is the harness argv.
pub(crate) fn finish_sandboxed_command(
    command: &mut Command,
    sandbox: &AgentSandbox,
    grants: &SandboxGrants,
    acp_command: &Path,
) {
    strip_nono_env(command);
    command.args(wrap_args(sandbox, grants, acp_command));
}

fn wrap_args(sandbox: &AgentSandbox, grants: &SandboxGrants, acp_command: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "wrap".into(),
        "--silent".into(),
        // Grant the nest workdir (at the profile's `workdir.access`) without
        // nono's interactive prompt, which fails without a terminal.
        "--allow-cwd".into(),
        "--profile".into(),
        sandbox.profile.clone().into(),
    ];
    for dir in &grants.read {
        args.push("--read".into());
        args.push(dir.into());
    }
    for dir in &grants.allow {
        args.push("--allow".into());
        args.push(dir.into());
    }
    for file in &grants.read_files {
        args.push("--read-file".into());
        args.push(file.into());
    }
    args.push("--".into());
    args.push(acp_command.into());
    args
}

/// Remove every `NONO_*` variable — inherited from the desktop's own
/// environment or set anywhere on the command — so nothing but the arguments
/// built here configures the sandbox.
fn strip_nono_env(command: &mut Command) {
    let set: Vec<OsString> = command
        .get_envs()
        .filter(|(_, value)| value.is_some())
        .map(|(key, _)| key.to_os_string())
        .collect();
    let inherited = std::env::vars_os().map(|(key, _)| key);
    for key in set.into_iter().chain(inherited) {
        if is_nono_env_key(&key) {
            command.env_remove(&key);
        }
    }
}

fn is_nono_env_key(key: &std::ffi::OsStr) -> bool {
    key.to_str()
        .is_some_and(|key| key.to_ascii_uppercase().starts_with(NONO_ENV_PREFIX))
}

/// Inputs for the per-launch grants, already resolved by the spawn.
pub(crate) struct GrantInputs<'a> {
    /// Resolved binaries the harness runs: `buzz-acp`, the agent CLI, MCP.
    pub binaries: &'a [&'a Path],
    /// The augmented `PATH` given to the child.
    pub path_env: Option<&'a std::ffi::OsStr>,
    pub home: Option<&'a Path>,
    pub nest: Option<&'a Path>,
    /// Demo-build agent config root, read-write when present.
    pub demo_config_home: Option<&'a Path>,
    /// Buzz-owned data directories agents read ([`buzz_data_reads`]).
    pub data_reads: &'a [PathBuf],
    /// The agent's log, inherited as stdout and stderr. Node `fstat`s its
    /// stdio at startup and aborts when the sandbox denies that.
    pub log_file: Option<&'a Path>,
}

/// Compute the per-launch grants. Only existing directories are granted, and
/// never `$HOME` or one of its ancestors, which would expose everything the
/// base profile deliberately leaves out.
pub(crate) fn spawn_grants(inputs: &GrantInputs<'_>) -> SandboxGrants {
    let home = inputs.home.and_then(canonical_dir);
    let mut grants = SandboxGrants::default();

    let mut read = Vec::new();
    // Binary directories: sidecars live in the app bundle, but dev builds run
    // `buzz-acp` and friends from the repo's `target/` directory.
    read.extend(
        inputs
            .binaries
            .iter()
            .filter(|binary| binary.is_absolute())
            .filter_map(|binary| binary.parent().map(Path::to_path_buf)),
    );
    // User-level PATH entries (login-shell tool dirs) so agents can run the
    // tools the user can. System dirs are already readable via the profile.
    if let (Some(path_env), Some(home)) = (inputs.path_env, home.as_deref()) {
        read.extend(
            std::env::split_paths(path_env)
                .filter_map(|dir| canonical_dir(&dir))
                .filter(|dir| dir.starts_with(home)),
        );
    }
    read.extend(inputs.data_reads.iter().cloned());

    let mut allow = Vec::new();
    // A user-configured repos directory is a symlink out of the nest.
    if let Some(nest) = inputs.nest {
        if let Some(repos) = canonical_dir(&nest.join("REPOS")) {
            let nest = canonical_dir(nest);
            if nest.is_none_or(|nest| !repos.starts_with(nest)) {
                allow.push(repos);
            }
        }
    }
    allow.extend(inputs.demo_config_home.map(Path::to_path_buf));

    for (source, target) in [(read, &mut grants.read), (allow, &mut grants.allow)] {
        for dir in source.iter().filter_map(|dir| canonical_dir(dir)) {
            let exposes_home = home.as_deref().is_some_and(|home| home.starts_with(&dir));
            if !exposes_home && !target.contains(&dir) {
                target.push(dir);
            }
        }
    }
    grants.read_files.extend(
        inputs
            .log_file
            .and_then(|file| std::fs::canonicalize(file).ok()),
    );
    grants
}

/// Buzz-owned data directories agents read: the templates
/// `buzz channel-templates` uses, and the Node runtime and npm tools Buzz
/// installs (`claude-agent-acp`, `codex-acp`). An npm tool's `bin/` entry links
/// into the prefix's `lib/node_modules`, so the whole prefix is granted. The
/// rest of the desktop's app-data directory (agent keys) stays out of reach.
pub(crate) fn buzz_data_reads() -> Vec<PathBuf> {
    [
        cli_templates_dir(),
        super::buzz_managed_npm_prefix(),
        super::buzz_managed_node_root(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The directory `buzz channel-templates` reads templates from.
fn cli_templates_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join(CLI_TEMPLATES_BUNDLE_IDENTIFIER).join("templates"))
}

fn canonical_dir(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path)
        .ok()
        .filter(|path| path.is_dir())
}

#[cfg(test)]
mod tests;
