//! Buzz Nest — persistent agent workspace at `~/.buzz`.
//!
//! Creates a shared knowledge directory on first launch so every
//! Buzz-spawned agent starts with orientation (AGENTS.md) and a
//! place to accumulate research, plans, and logs across sessions.
//!
//! Static template content in AGENTS.md (above the managed-section markers)
//! and SKILL.md is refreshed when the embedded template version changes.

use super::{load_managed_agents, load_personas, AgentDefinition, ManagedAgentRecord};
#[cfg(test)]
use super::{BackendKind, RespondTo};
use crate::app_state::AppState;
use crate::relay::relay_ws_url_with_override;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use crate::managed_agents::discovery::known_skill_dirs;
#[cfg(unix)]
use crate::util::create_symlink;

/// Subdirectories created inside the nest.
/// `REPOS` is intentionally absent: it is provisioned by
/// [`super::repos::ensure_repos_symlink`], which makes it either a real directory (default)
/// or a symlink to a user-configured `repos_dir`. Creating it here
/// unconditionally would race a future symlink re-point.
const NEST_DIRS: &[&str] = &[
    "GUIDES",
    "RESEARCH",
    "PLANS",
    "WORK_LOGS",
    "OUTBOX",
    ".scratch",
];

/// Default AGENTS.md content written on first init.
/// Fully static — no runtime interpolation, no secrets, no user paths.
pub(crate) const AGENTS_MD: &str = include_str!("nest_agents.md");

/// Default SKILL.md content for the buzz-cli skill.
/// Written to ~/.buzz/.agents/skills/buzz-cli/SKILL.md on first init.
const BUZZ_CLI_SKILL_MD: &str = include_str!("nest_skill.md");

/// Template content version for AGENTS.md static content (above managed markers).
/// Bump this when changing `nest_agents.md` to trigger refresh on existing installs.
/// Version 1 is implicitly "before this mechanism existed" (no version file).
const NEST_AGENTS_VERSION: u32 = 4;

/// Template content version for SKILL.md.
/// Bump this when changing `nest_skill.md` to trigger refresh on existing installs.
const NEST_SKILL_VERSION: u32 = 5;

const BEGIN_MARKER: &str = "<!-- BEGIN BUZZ MANAGED";
const END_MARKER: &str = "<!-- END BUZZ MANAGED -->";

/// Canonical skill directory path relative to the nest root.
const CANONICAL_SKILL_DIR: &str = ".agents/skills/buzz-cli";

/// Nest directory name for production builds.
const NEST_DIR_PROD: &str = ".buzz";

/// Nest directory name for dev builds. Dev builds (those whose Tauri app-data
/// directory name starts with `"xyz.block.buzz.app.dev"`) use a separate nest
/// so that the DMG and dev-build instances don't clobber each other's
/// `.repos-dir` dotfile and `REPOS` symlink.
const NEST_DIR_DEV: &str = ".buzz-dev";

/// Process-lifetime nest directory. Initialized once at startup via
/// [`init_nest_dir`] before any call to [`nest_dir`].
///
/// `None` inside the `OnceLock` means "home dir was unresolvable at init time".
/// The outer `None` from `OnceLock::get` means "not initialized yet" —
/// [`nest_dir`] falls back to the prod path in that case, ensuring test code
/// that never calls [`init_nest_dir`] still works.
static NEST_DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Initialize the process-lifetime nest directory.
///
/// Must be called once at app startup (before any call to [`nest_dir`] that
/// may result in a filesystem operation). Subsequent calls are no-ops — the
/// `OnceLock` is set exactly once.
///
/// `is_dev` should be `true` when the running binary is a dev build — i.e.
/// when the Tauri app-data directory name starts with `"xyz.block.buzz.app.dev"`.
/// Pass `false` for production (signed DMG) builds.
pub fn init_nest_dir(is_dev: bool) {
    let suffix = if is_dev { NEST_DIR_DEV } else { NEST_DIR_PROD };
    let path = dirs::home_dir().map(|h| h.join(suffix));
    // set() is a no-op when already initialized, which is correct: only the
    // first call (at boot, before any filesystem work) should win.
    let _ = NEST_DIR.set(path);
}

/// Returns the nest root path (`~/.buzz` for prod, `~/.buzz-dev` for dev),
/// or `None` if the home directory cannot be resolved.
///
/// If [`init_nest_dir`] has not been called yet (e.g. in unit tests), falls
/// back to the production path `~/.buzz`.
pub fn nest_dir() -> Option<PathBuf> {
    match NEST_DIR.get() {
        Some(path) => path.clone(),
        // Not yet initialized — fall back to prod path. Covers test code.
        None => dirs::home_dir().map(|h| h.join(NEST_DIR_PROD)),
    }
}

/// Returns `true` iff `path` ends with the dev-nest directory name (`.buzz-dev`).
///
/// Pure function — no globals — so it can be unit-tested without touching the
/// process-lifetime [`NEST_DIR`] `OnceLock`.
fn path_is_dev_nest(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n == NEST_DIR_DEV)
        .unwrap_or(false)
}

/// Returns `true` when the running binary is using the dev nest (`~/.buzz-dev`).
///
/// This is `true` for all dev builds — `just staging` and `just dev` — because
/// [`init_nest_dir`] is called with `is_dev = true` when the Tauri app-data
/// directory starts with `"xyz.block.buzz.app.dev"`.
///
/// Returns `false` when:
/// - The nest is the production nest (`~/.buzz`, signed DMG).
/// - [`init_nest_dir`] has not been called yet (unit tests, home dir
///   unresolvable) — the fallback path is always the prod nest.
pub fn nest_is_dev() -> bool {
    nest_dir().map(|p| path_is_dev_nest(&p)).unwrap_or(false)
}

/// Creates the Buzz nest at `~/.buzz` if it doesn't already exist.
///
/// Delegates to [`ensure_nest_at`] with the resolved nest directory.
/// Returns an error string if the home directory cannot be resolved.
pub fn ensure_nest() -> Result<(), String> {
    let root = nest_dir().ok_or("cannot resolve home directory for nest")?;
    ensure_nest_at(&root)
}

/// Creates a Buzz nest at the given `root` path.
///
/// - Creates the root directory and all subdirectories.
/// - Writes `AGENTS.md` only if it doesn't already exist.
/// - Writes `.agents/skills/buzz-cli/SKILL.md` only if it doesn't already exist.
/// - Creates harness-specific symlinks pointing to the canonical
///   `.agents/skills/buzz-cli` directory for each known provider.
/// - Sets 700 permissions on the root, all subdirectories, and the skill
///   directory tree (Unix).
///
/// Idempotent: safe to call on every launch. Static template content in
/// AGENTS.md (above the managed-section markers) and SKILL.md is refreshed
/// when the embedded template version changes. The managed section in AGENTS.md
/// and any user content below it are preserved.
///
/// Rejects symlinks at the root path to prevent redirect attacks.
///
/// Errors are returned as strings for Tauri compatibility; callers
/// should log and continue rather than aborting app startup.
pub fn ensure_nest_at(root: &Path) -> Result<(), String> {
    // Reject symlinks — we want a real directory, not a redirect.
    // Platform-independent: symlink_metadata works on all OS.
    if root
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!(
            "{} is a symlink; refusing to use as nest root",
            root.display()
        ));
    }

    // Create root and all subdirectories. create_dir_all is idempotent —
    // it succeeds silently if the directory already exists.
    fs::create_dir_all(root).map_err(|e| format!("create {}: {e}", root.display()))?;

    for dir in NEST_DIRS {
        let path = root.join(dir);
        fs::create_dir_all(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
    }

    // REPOS is provisioned separately from NEST_DIRS: it may be a symlink to a
    // user-configured repos_dir (applied later via apply_workspace), so setup
    // must not clobber an existing configured symlink. See repos.rs.
    super::repos::ensure_repos_setup_default(root)?;

    // Write AGENTS.md only if it doesn't already exist.
    // Uses create_new (O_CREAT|O_EXCL) to atomically check-and-create,
    // closing the TOCTOU gap that exists() + write() would leave open.
    // Also guarantees we never clobber a user-edited file.
    let agents_md = root.join("AGENTS.md");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&agents_md)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(AGENTS_MD.as_bytes())
                .map_err(|e| format!("write {}: {e}", agents_md.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // File already exists — leave it alone (idempotent).
        }
        Err(e) => {
            return Err(format!("create {}: {e}", agents_md.display()));
        }
    }

    // Write buzz-cli skill to the harness-agnostic .agents path.
    // The first-init write uses the new canonical path; migration from
    // the old .claude path is handled in refresh_skill_md_if_stale.
    let agents_skill_dir = root.join(CANONICAL_SKILL_DIR);
    fs::create_dir_all(&agents_skill_dir)
        .map_err(|e| format!("create {}: {e}", agents_skill_dir.display()))?;

    let skill_md = agents_skill_dir.join("SKILL.md");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&skill_md)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(BUZZ_CLI_SKILL_MD.as_bytes())
                .map_err(|e| format!("write {}: {e}", skill_md.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(format!("create {}: {e}", skill_md.display()));
        }
    }

    // Create harness-specific symlinks for all known providers.
    // Migration of the old .claude/skills/buzz-cli real dir is handled in
    // refresh_skill_md_if_stale; ensure_skill_symlinks skips paths that already exist.
    ensure_skill_symlinks(root)?;

    // Refresh static content if the embedded template version is newer.
    refresh_agents_md_if_stale(root)?;
    refresh_skill_md_if_stale(root)?;

    // Set owner-only permissions on root and all subdirectories.
    // Skip any path that is a symlink — chmod would affect the target.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o700);
        fs::set_permissions(root, perms.clone())
            .map_err(|e| format!("set permissions on {}: {e}", root.display()))?;
        for dir in NEST_DIRS {
            let path = root.join(dir);
            let is_symlink = path
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_symlink {
                fs::set_permissions(&path, perms.clone())
                    .map_err(|e| format!("set permissions on {}: {e}", path.display()))?;
            }
        }
        // REPOS is provisioned outside NEST_DIRS (it may be a symlink). Only
        // chmod it when it is a real directory — chmod on a symlink would
        // affect the user's external repos_dir target.
        let repos_path = root.join("REPOS");
        let repos_is_symlink = repos_path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if !repos_is_symlink {
            fs::set_permissions(&repos_path, perms.clone())
                .map_err(|e| format!("set permissions on {}: {e}", repos_path.display()))?;
        }
        // Skill directory trees inside root get 700.
        // Build the list from canonical path + all known provider skill dirs.
        let mut skill_perm_dirs = Vec::new();
        {
            let mut accumulated = std::path::PathBuf::new();
            for component in std::path::Path::new(CANONICAL_SKILL_DIR).components() {
                accumulated.push(component);
                skill_perm_dirs.push(root.join(&accumulated));
            }
        }
        for skill_dir in known_skill_dirs() {
            // Ensure every ancestor dir gets 700, not just the leaf.
            let mut accumulated = std::path::PathBuf::new();
            for component in std::path::Path::new(skill_dir).components() {
                accumulated.push(component);
                skill_perm_dirs.push(root.join(&accumulated));
            }
        }
        for dir in skill_perm_dirs {
            let is_symlink = dir
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_symlink {
                fs::set_permissions(&dir, perms.clone())
                    .map_err(|e| format!("set permissions on {}: {e}", dir.display()))?;
            }
        }
    }

    Ok(())
}

/// Create harness-specific skill symlinks for each known provider.
/// Idempotent: skips any path where `symlink_metadata` succeeds — real
/// directories, valid symlinks, and dangling symlinks are all left alone.
#[cfg(unix)]
fn ensure_skill_symlinks(root: &Path) -> Result<(), String> {
    for skill_dir in known_skill_dirs() {
        let parent = root.join(skill_dir);
        fs::create_dir_all(&parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        let link = parent.join("buzz-cli");
        if link.symlink_metadata().is_ok() {
            continue; // symlink or real path exists — skip
        }
        let depth = std::path::Path::new(skill_dir).components().count();
        let prefix = "../".repeat(depth);
        let target = format!("{prefix}{CANONICAL_SKILL_DIR}");
        create_symlink(std::path::Path::new(&target), &link)
            .map_err(|e| format!("symlink {} → {}: {e}", link.display(), target))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_skill_symlinks(_root: &Path) -> Result<(), String> {
    Ok(())
}

/// Returns the `~/.local/bin` link name for the bundled CLI.
///
/// Dev builds (`is_dev = true`) use `"buzz-dev"` so that a running DMG and a
/// concurrent dev build each own a separate link and never clobber each other —
/// the same isolation that separates `~/.buzz` (prod) from `~/.buzz-dev` (dev).
pub fn cli_link_name(is_dev: bool) -> &'static str {
    if is_dev {
        "buzz-dev"
    } else {
        "buzz"
    }
}

/// Ensures `~/.local/bin/buzz` (prod) or `~/.local/bin/buzz-dev` (dev) is a
/// symlink to the bundled CLI binary.
///
/// The link name is split by `is_dev` so that an installed DMG and a
/// concurrently running dev build each maintain their own symlink and never
/// overwrite each other's target — the same isolation that separates the
/// `~/.buzz` and `~/.buzz-dev` nests (see [`NEST_DIR_DEV`]).
///
/// On every boot: replaces any existing symlink unconditionally (the `buzz` /
/// `buzz-dev` name is our namespace), creates a new one if absent, and leaves
/// regular files alone to avoid clobbering a user-compiled binary.
///
/// Non-fatal: callers should ignore errors — the symlink is a convenience
/// for human Terminal use; agents find the CLI via PATH augmentation.
#[cfg(unix)]
pub fn ensure_cli_symlink(exe_parent: &Path, is_dev: bool) -> Result<(), String> {
    let buzz_bin = exe_parent.join("buzz");
    if !buzz_bin.exists() {
        return Ok(()); // CLI not bundled (e.g., dev builds without sidecars).
    }

    let local_bin = dirs::home_dir()
        .ok_or("cannot resolve home directory")?
        .join(".local")
        .join("bin");
    fs::create_dir_all(&local_bin).map_err(|e| format!("create {}: {e}", local_bin.display()))?;

    let link = local_bin.join(cli_link_name(is_dev));
    match link.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => {
            let _ = fs::remove_file(&link);
            create_symlink(&buzz_bin, &link)
                .map_err(|e| format!("symlink {}: {e}", link.display()))?;
        }
        Ok(_) => {
            // Regular file or directory — don't clobber.
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            create_symlink(&buzz_bin, &link)
                .map_err(|e| format!("symlink {}: {e}", link.display()))?;
        }
        Err(e) => {
            return Err(format!("stat {}: {e}", link.display()));
        }
    }

    Ok(())
}

/// No-op on non-Unix platforms — symlink management is macOS/Linux only.
#[cfg(not(unix))]
pub fn ensure_cli_symlink(_exe_parent: &Path, _is_dev: bool) -> Result<(), String> {
    Ok(())
}

/// Read a version number from a file. Returns 0 if the file doesn't exist or can't be parsed.
fn read_version_file(path: &Path) -> u32 {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Refresh AGENTS.md static content if the template version has changed.
///
/// Preserves everything from the `<!-- BEGIN BUZZ MANAGED` marker onward
/// (the dynamic section managed by `upsert_managed_section`). Replaces
/// only the static template content above the marker.
fn refresh_agents_md_if_stale(root: &Path) -> Result<(), String> {
    let version_path = root.join(".nest-agents-version");
    if read_version_file(&version_path) >= NEST_AGENTS_VERSION {
        return Ok(());
    }

    let agents_md = root.join("AGENTS.md");
    let current =
        fs::read_to_string(&agents_md).map_err(|e| format!("read {}: {e}", agents_md.display()))?;

    let new_content = match find_marker_at_line_start(&current, BEGIN_MARKER) {
        Some(pos) => {
            // Find the start of the marker line (could be preceded by blank lines).
            let marker_line_start = current[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
            // Template content up to (but not including) the managed section,
            // then the existing managed section from the marker onward.
            let template_static = match AGENTS_MD.find(BEGIN_MARKER) {
                Some(tmpl_marker_pos) => {
                    let tmpl_line_start = AGENTS_MD[..tmpl_marker_pos]
                        .rfind('\n')
                        .map(|p| p + 1)
                        .unwrap_or(0);
                    &AGENTS_MD[..tmpl_line_start]
                }
                None => AGENTS_MD,
            };
            format!("{}{}", template_static, &current[marker_line_start..])
        }
        None => {
            // No managed section found — write full template.
            AGENTS_MD.to_string()
        }
    };

    // Atomic write via temp file.
    let parent = agents_md.parent().ok_or("AGENTS.md has no parent dir")?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("tempfile in {}: {e}", parent.display()))?;
    {
        use std::io::Write;
        tmp.write_all(new_content.as_bytes())
            .map_err(|e| format!("write tempfile: {e}"))?;
    }
    tmp.persist(&agents_md)
        .map_err(|e| format!("persist {}: {e}", agents_md.display()))?;

    fs::write(&version_path, format!("{NEST_AGENTS_VERSION}\n"))
        .map_err(|e| format!("write {}: {e}", version_path.display()))?;

    Ok(())
}

/// Refresh SKILL.md if the template version has changed.
///
/// SKILL.md has no user-editable sections — it is fully overwritten on version bump.
fn refresh_skill_md_if_stale(root: &Path) -> Result<(), String> {
    let agents_skill_dir = root.join(".agents/skills/buzz-cli");
    let version_path = agents_skill_dir.join(".skill-version");
    if read_version_file(&version_path) >= NEST_SKILL_VERSION {
        return Ok(());
    }

    // Migration: if .claude/skills/buzz-cli exists as a real directory
    // (pre-migration install), copy user's SKILL.md to the new location
    // then remove the old directory so we can replace it with a symlink.
    let old_skill_dir = root.join(".claude/skills/buzz-cli");
    let old_is_real_dir = old_skill_dir
        .symlink_metadata()
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false);

    let skill_content = if old_is_real_dir {
        // Preserve user-edited content during migration.
        fs::read_to_string(old_skill_dir.join("SKILL.md"))
            .unwrap_or_else(|_| BUZZ_CLI_SKILL_MD.to_string())
    } else {
        BUZZ_CLI_SKILL_MD.to_string()
    };

    // Ensure the canonical .agents skill directory exists.
    fs::create_dir_all(&agents_skill_dir)
        .map_err(|e| format!("create {}: {e}", agents_skill_dir.display()))?;

    // Atomic write via temp file.
    let skill_md = agents_skill_dir.join("SKILL.md");
    let mut tmp = tempfile::NamedTempFile::new_in(&agents_skill_dir)
        .map_err(|e| format!("tempfile in {}: {e}", agents_skill_dir.display()))?;
    {
        use std::io::Write;
        tmp.write_all(skill_content.as_bytes())
            .map_err(|e| format!("write tempfile: {e}"))?;
    }
    tmp.persist(&skill_md)
        .map_err(|e| format!("persist {}: {e}", skill_md.display()))?;

    // Replace old real directory with a symlink.
    if old_is_real_dir {
        fs::remove_dir_all(&old_skill_dir)
            .map_err(|e| format!("remove {}: {e}", old_skill_dir.display()))?;
    }

    // Create/replace the .claude/skills/buzz-cli symlink.
    #[cfg(unix)]
    {
        let claude_skills_dir = root.join(".claude/skills");
        fs::create_dir_all(&claude_skills_dir)
            .map_err(|e| format!("create {}: {e}", claude_skills_dir.display()))?;
        let symlink_path = root.join(".claude/skills/buzz-cli");
        // Remove any stale symlink before (re)creating.
        let symlink_exists = symlink_path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if symlink_exists {
            fs::remove_file(&symlink_path)
                .map_err(|e| format!("remove symlink {}: {e}", symlink_path.display()))?;
        }
        create_symlink(
            std::path::Path::new("../../.agents/skills/buzz-cli"),
            &symlink_path,
        )
        .map_err(|e| format!("symlink {}: {e}", symlink_path.display()))?;
    }

    fs::write(&version_path, format!("{NEST_SKILL_VERSION}\n"))
        .map_err(|e| format!("write {}: {e}", version_path.display()))?;

    Ok(())
}

fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

pub fn render_dynamic_section(
    personas: &[AgentDefinition],
    agents: &[ManagedAgentRecord],
    relay_url: &str,
) -> String {
    let active_agents = if agents.is_empty() {
        "## Active Agents\n\n*(No agents deployed yet. Add agents in the Buzz desktop app.)*"
            .to_string()
    } else {
        let mut table =
            "## Active Agents\n\n| Name | Persona | How to address |\n|------|---------|----------------|"
                .to_string();
        for agent in agents {
            let role = agent
                .persona_id
                .as_deref()
                .and_then(|pid| personas.iter().find(|p| p.id == pid))
                .map(|p| p.display_name.as_str())
                .unwrap_or("—");
            let name = escape_md_cell(&agent.name);
            let role_escaped = escape_md_cell(role);
            table.push_str(&format!("\n| {name} | {role_escaped} | @{name} |"));
        }
        table
    };

    let relay_url = relay_url.replace(['\n', '\r'], "");
    format!("{active_agents}\n\n## Workspace\n- Relay: {relay_url}")
}

/// Find a marker that appears at the start of a line (position 0 or preceded by `\n`).
fn find_marker_at_line_start(content: &str, marker: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(marker) {
        let abs_pos = search_from + pos;
        if abs_pos == 0 || content.as_bytes()[abs_pos - 1] == b'\n' {
            return Some(abs_pos);
        }
        search_from = abs_pos + 1;
    }
    None
}

/// Find the first valid ordered BEGIN/END marker pair, both at line starts.
/// Returns `(begin_line_start, after_end)` byte offsets for slicing.
fn find_managed_markers(content: &str) -> Option<(usize, usize)> {
    let begin_pos = find_marker_at_line_start(content, BEGIN_MARKER)?;
    let begin_line_start = content[..begin_pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let end_pos =
        find_marker_at_line_start(&content[begin_pos..], END_MARKER).map(|p| p + begin_pos)?;
    let end_of_end = end_pos + END_MARKER.len();
    let after_end = if content[end_of_end..].starts_with('\n') {
        end_of_end + 1
    } else {
        end_of_end
    };
    Some((begin_line_start, after_end))
}

/// Remove an orphan BEGIN marker line (one with no matching END after it).
fn strip_orphan_begin_marker(content: &str) -> String {
    if let Some(pos) = find_marker_at_line_start(content, BEGIN_MARKER) {
        let line_start = content[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let line_end = content[pos..]
            .find('\n')
            .map(|p| pos + p + 1)
            .unwrap_or(content.len());
        format!(
            "{}{}",
            &content[..line_start],
            content[line_end..]
                .strip_prefix('\n')
                .unwrap_or(&content[line_end..])
        )
    } else {
        content.to_string()
    }
}

pub fn upsert_managed_section(file_path: &Path, new_section_content: &str) -> io::Result<()> {
    let current = fs::read_to_string(file_path)?;

    let replacement = format!(
        "{BEGIN_MARKER} — regenerated automatically, do not edit below -->\n{new_section_content}\n{END_MARKER}\n"
    );

    let new_content = match find_managed_markers(&current) {
        Some((begin_line_start, after_end)) => {
            format!(
                "{}{}{}",
                &current[..begin_line_start],
                replacement,
                &current[after_end..]
            )
        }
        None => {
            let cleaned = strip_orphan_begin_marker(&current);
            format!("{}\n\n{}", cleaned.trim_end_matches('\n'), replacement)
        }
    };

    // Skip write when content is unchanged — avoids bumping mtime on every launch.
    if new_content == current {
        return Ok(());
    }

    let parent = file_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "file path has no parent directory",
        )
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        use std::io::Write;
        tmp.write_all(new_content.as_bytes())?;
    }
    tmp.persist(file_path).map_err(|e| e.error)?;

    Ok(())
}

pub fn regenerate_nest_context(app: &AppHandle) -> Result<(), String> {
    let nest = nest_dir().ok_or("cannot resolve home directory for nest")?;
    let agents_md = nest.join("AGENTS.md");

    if !agents_md.exists() {
        return Ok(());
    }

    let personas = load_personas(app)?;
    let agents = load_managed_agents(app)?;
    let state = app.state::<AppState>();
    let relay_url = relay_ws_url_with_override(&state);
    let content = render_dynamic_section(&personas, &agents, &relay_url);
    upsert_managed_section(&agents_md, &content)
        .map_err(|e| format!("regenerate nest context: {e}"))?;

    Ok(())
}

/// Convenience wrapper: regenerates nest context, logging a warning on failure.
///
/// All call sites treat regeneration as fire-and-forget — agents run fine with
/// a stale AGENTS.md, so we warn and continue rather than propagating the error.
pub fn try_regenerate_nest(app: &AppHandle) {
    if let Err(error) = regenerate_nest_context(app) {
        eprintln!("buzz-desktop: nest context regeneration failed: {error}");
    }
}

#[cfg(test)]
mod tests;
