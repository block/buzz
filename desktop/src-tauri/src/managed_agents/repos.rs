//! Per-workspace `REPOS` directory provisioning.
//!
//! The nest's `REPOS` directory is either a real directory (the default) or a
//! symlink to a user-configured `repos_dir`, letting agents work in existing
//! local checkouts instead of re-cloning. [`ensure_repos_symlink`] reconciles
//! `REPOS` with the configured path; [`validate_repos_dir`] guards the input.
//!
//! The symlink follows the *active* workspace — a human/tooling convention.
//! Agents are made immune to it: every applied workspace's `repos_dir` is
//! also persisted per relay URL ([`persist_workspace_repos_dir`]), and spawn
//! resolves the agent's own workspace entry to a real path
//! ([`resolve_agent_repos_dir`]) so a later workspace switch re-pointing
//! `REPOS` cannot move a running agent into another workspace's checkouts.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use crate::util::{create_symlink, symlink_points_to};

/// Validate a user-supplied `repos_dir`, returning the canonical target path.
///
/// Requires an **existing absolute directory**. Rejects relative paths,
/// `~`-prefixed paths (shell tilde is not expanded by `std::fs` — the FE
/// expands before save, so a `~` reaching here is a bug to surface loudly),
/// non-directories, and a path that is the nest itself or an ancestor of it
/// (symlinking `REPOS` into its own parent would create a cycle). Never
/// creates the target — a typo must not silently mint a stray directory.
pub fn validate_repos_dir(nest_root: &Path, repos_dir: &str) -> Result<PathBuf, String> {
    let trimmed = repos_dir.trim();
    if trimmed.starts_with('~') {
        return Err(format!(
            "repos dir must be an absolute path (got `{trimmed}`); use e.g. /Users/you/Development"
        ));
    }
    let target = Path::new(trimmed);
    if !target.is_absolute() {
        return Err(format!(
            "repos dir must be an absolute path (got `{trimmed}`)"
        ));
    }
    // Resolve symlinks/`..` so the directory check and ancestor check both
    // operate on the real location. Fails loudly on a missing or unreadable
    // path rather than falling back to a real REPOS dir.
    let canonical = target
        .canonicalize()
        .map_err(|e| format!("repos dir `{trimmed}` is not accessible: {e}"))?;
    if !canonical.is_dir() {
        return Err(format!("repos dir `{trimmed}` is not a directory"));
    }
    // Refuse the nest itself or any ancestor of it — pointing REPOS there
    // would nest the symlink inside its own target.
    if let Ok(nest_canonical) = nest_root.canonicalize() {
        if nest_canonical == canonical || nest_canonical.starts_with(&canonical) {
            return Err(format!(
                "repos dir `{trimmed}` is the nest or an ancestor of it; choose a separate directory"
            ));
        }
    }
    Ok(canonical)
}

/// Ensure `nest_root/REPOS` matches the configured `repos_dir`.
///
/// - **`repos_dir` = `None`/empty** → ensure `REPOS` is a real in-nest
///   directory (the default). A pre-existing symlink (from a prior
///   `repos_dir`) is removed first so clearing the field genuinely reverts;
///   removing a symlink never touches its target. Idempotent otherwise.
/// - **`repos_dir` set, `REPOS` absent** → create a symlink to the target.
/// - **`repos_dir` set, `REPOS` is a symlink** (any target) → replace it
///   (`remove_file` + re-symlink). Removing a symlink never touches the
///   target's contents, so this is data-safe.
/// - **`repos_dir` set, `REPOS` is an empty real dir** → remove it and
///   symlink. Converting an empty dir loses nothing.
/// - **`repos_dir` set, `REPOS` is a NON-EMPTY real dir** → refuse and warn.
///   Never `remove_dir_all` — that would destroy repos the agent cloned
///   in-nest. The user must clear or relocate them first.
///
/// Validation (`validate_repos_dir`) runs before any filesystem mutation, so
/// an invalid path returns `Err` with `REPOS` left exactly as it was.
#[cfg(unix)]
pub fn ensure_repos_symlink(nest_root: &Path, repos_dir: Option<&str>) -> Result<(), String> {
    let repos_path = nest_root.join("REPOS");

    let Some(target) = repos_dir
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|raw| validate_repos_dir(nest_root, raw))
        .transpose()?
    else {
        // No repos_dir: REPOS must be a real in-nest directory. If it is
        // currently a symlink (from a prior repos_dir), remove the link first
        // — create_dir_all follows a symlink and would leave the stale link in
        // place. remove_file never touches the link's target.
        if let Ok(meta) = repos_path.symlink_metadata() {
            if meta.file_type().is_symlink() {
                fs::remove_file(&repos_path)
                    .map_err(|e| format!("remove symlink {}: {e}", repos_path.display()))?;
            }
        }
        fs::create_dir_all(&repos_path)
            .map_err(|e| format!("create {}: {e}", repos_path.display()))?;
        return Ok(());
    };

    match repos_path.symlink_metadata() {
        // Existing symlink → replace it if it points elsewhere. Re-pointing a
        // symlink is data-safe; remove_file never follows the link.
        Ok(meta) if meta.file_type().is_symlink() => {
            if symlink_points_to(&repos_path, &target) {
                return Ok(()); // already correct
            }
            fs::remove_file(&repos_path)
                .map_err(|e| format!("remove symlink {}: {e}", repos_path.display()))?;
            symlink_repos(&target, &repos_path)
        }
        // Existing real directory → convert only if empty; otherwise refuse.
        Ok(meta) if meta.is_dir() => {
            let empty = fs::read_dir(&repos_path)
                .map_err(|e| format!("read {}: {e}", repos_path.display()))?
                .next()
                .is_none();
            if !empty {
                return Err(format!(
                    "{} holds repositories; move or delete them before pointing repos dir elsewhere",
                    repos_path.display()
                ));
            }
            fs::remove_dir(&repos_path)
                .map_err(|e| format!("remove {}: {e}", repos_path.display()))?;
            symlink_repos(&target, &repos_path)
        }
        // Exists but is neither symlink nor dir (e.g. a file) → refuse.
        Ok(_) => Err(format!(
            "{} exists and is not a directory; cannot point repos dir there",
            repos_path.display()
        )),
        // Absent → create the symlink.
        Err(e) if e.kind() == io::ErrorKind::NotFound => symlink_repos(&target, &repos_path),
        Err(e) => Err(format!("stat {}: {e}", repos_path.display())),
    }
}

#[cfg(unix)]
fn symlink_repos(target: &Path, link: &Path) -> Result<(), String> {
    create_symlink(target, link)
        .map_err(|e| format!("symlink {} → {}: {e}", link.display(), target.display()))
}

#[cfg(not(unix))]
pub fn ensure_repos_symlink(nest_root: &Path, _repos_dir: Option<&str>) -> Result<(), String> {
    let repos_path = nest_root.join("REPOS");
    fs::create_dir_all(&repos_path).map_err(|e| format!("create {}: {e}", repos_path.display()))
}

/// Provision `REPOS` at nest setup, before any configured `repos_dir` is known.
///
/// Leaves an existing symlink untouched — `apply_workspace` is the sole
/// authority over a configured symlink. Clearing it here with `None` would
/// destroy a symlink restored from a prior session; async-restored agents
/// would then write into the fresh real dir, and the later FE re-point would
/// refuse the now-non-empty REPOS — silently breaking `repos_dir` on restart.
/// Otherwise (absent, or a real dir) lands the default real-dir fallback.
pub fn ensure_repos_setup_default(nest_root: &Path) -> Result<(), String> {
    let repos_path = nest_root.join("REPOS");
    let is_symlink = repos_path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink {
        return Ok(());
    }
    ensure_repos_symlink(nest_root, None)
}

/// Decide what `repos_dir` `apply_workspace` should persist and symlink for a
/// candidate value, running the single source-of-truth [`validate_repos_dir`].
///
/// - **`None`/empty candidate** → `Ok(None)`: no override; `REPOS` reverts to a
///   real in-nest directory.
/// - **valid candidate** → `Ok(Some(raw_trimmed))`: persist the *raw* trimmed
///   string (not the canonical path — persisting canonical would drift the
///   symlink target on `..`/symlinked-ancestor paths).
/// - **invalid candidate** → `Err(reason)`: the caller must persist `None`
///   (clearing the override so the next boot resolves clean and agent restore
///   proceeds) and surface `reason` to the user. Returning `Err` rather than
///   silently `Ok(None)` lets the caller emit the human-readable cause.
///
/// One validate call drives both the persisted value and the error: a bad path
/// is never persisted, so it can never silently skip agent restore on a later
/// boot. Pure (no FS mutation, no emit) so the persist decision is unit-tested.
pub fn effective_repos_dir(
    nest_root: &Path,
    candidate: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(trimmed) = candidate.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    validate_repos_dir(nest_root, trimmed).map(|_| Some(trimmed.to_string()))
}

/// Filename of the dotfile persisting the active workspace's `repos_dir`.
const REPOS_DIR_FILE: &str = ".repos-dir";

/// Read the persisted `repos_dir` from `nest_root/.repos-dir`.
///
/// Returns the trimmed value, or `None` when the file is absent, unreadable,
/// or empty. This is the backend's sole knowledge of `repos_dir` at boot —
/// the frontend persists it via [`write_persisted_repos_dir`] on every
/// `apply_workspace`, so [`resolve_repos_at_boot`] can resolve the `REPOS`
/// symlink before any agent is restored.
fn read_persisted_repos_dir(nest_root: &Path) -> Option<String> {
    fs::read_to_string(nest_root.join(REPOS_DIR_FILE))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Persist the active workspace's `repos_dir` to `nest_root/.repos-dir`.
///
/// Writes the trimmed value (one line). A `None`/empty value clears the
/// override by removing the file, so a later boot reverts `REPOS` to a real
/// in-nest directory. Removing an absent file is not an error. Mirrors the
/// `.nest-agents-version` dotfile pattern.
pub fn write_persisted_repos_dir(nest_root: &Path, repos_dir: Option<&str>) -> Result<(), String> {
    let path = nest_root.join(REPOS_DIR_FILE);
    match repos_dir.map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => fs::write(&path, format!("{value}\n"))
            .map_err(|e| format!("write {}: {e}", path.display())),
        None => match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("remove {}: {e}", path.display())),
        },
    }
}

// ── Per-relay repos_dir map ───────────────────────────────────────────────

/// Filename of the dotfile persisting every workspace's `repos_dir`, keyed by
/// normalized relay URL. Distinct from [`REPOS_DIR_FILE`], which holds only
/// the *active* workspace's value for the boot-time symlink resolve.
const REPOS_DIR_MAP_FILE: &str = ".repos-dirs.json";

/// Read the per-relay `repos_dir` map from `nest_root/.repos-dirs.json`.
///
/// Keys are normalized relay URLs ([`crate::relay::normalize_relay_url`]);
/// values are raw trimmed `repos_dir` strings, exactly as persisted by
/// [`persist_workspace_repos_dir`]. An absent, unreadable, or malformed file
/// yields an empty map — the map is an isolation layer over spawn, not a boot
/// dependency, so it degrades to the pre-map default rather than failing.
/// Keys are re-normalized and empty entries dropped on read, as defense
/// against a hand-edited file.
fn read_repos_dir_map(nest_root: &Path) -> BTreeMap<String, String> {
    let Ok(raw) = fs::read_to_string(nest_root.join(REPOS_DIR_MAP_FILE)) else {
        return BTreeMap::new();
    };
    let Ok(parsed) = serde_json::from_str::<BTreeMap<String, String>>(&raw) else {
        eprintln!("buzz-desktop: {REPOS_DIR_MAP_FILE} is malformed; ignoring");
        return BTreeMap::new();
    };
    parsed
        .into_iter()
        .filter_map(|(relay, dir)| {
            let key = crate::relay::normalize_relay_url(&relay);
            let value = dir.trim().to_string();
            (!key.is_empty() && !value.is_empty()).then_some((key, value))
        })
        .collect()
}

/// Persist the per-relay `repos_dir` map, removing the file when empty
/// (mirrors [`write_persisted_repos_dir`]'s clear-by-removal).
///
/// The write goes through a temp file + rename so a crash mid-write cannot
/// leave truncated JSON behind: [`read_repos_dir_map`] degrades a malformed
/// file to an *empty* map, which would silently send every workspace's spawn
/// back to the shared `REPOS` fallback — exactly the cross-workspace hazard
/// the map exists to prevent.
fn write_repos_dir_map(nest_root: &Path, map: &BTreeMap<String, String>) -> Result<(), String> {
    let path = nest_root.join(REPOS_DIR_MAP_FILE);
    if map.is_empty() {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("remove {}: {e}", path.display())),
        };
    }
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| format!("serialize {REPOS_DIR_MAP_FILE}: {e}"))?;
    let tmp = nest_root.join(format!("{REPOS_DIR_MAP_FILE}.tmp"));
    fs::write(&tmp, format!("{json}\n")).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))
}

/// Upsert (or clear, on `None`/empty) the applied workspace's `repos_dir` in
/// the per-relay map.
///
/// Written on every `apply_workspace`, alongside the single-value
/// [`REPOS_DIR_FILE`]: that dotfile drives the active-workspace `REPOS`
/// symlink at boot, while this map is what makes agents immune to the symlink
/// — spawn resolves the agent's own workspace entry via
/// [`resolve_agent_repos_dir`], keyed by the agent's pinned relay. Values are
/// stored raw (trimmed, not canonicalized) for the same reason
/// [`effective_repos_dir`] persists raw.
pub fn persist_workspace_repos_dir(
    nest_root: &Path,
    relay_url: &str,
    repos_dir: Option<&str>,
) -> Result<(), String> {
    let key = crate::relay::normalize_relay_url(relay_url);
    if key.is_empty() {
        return Err("cannot persist a repos dir for an empty relay URL".to_string());
    }
    let mut map = read_repos_dir_map(nest_root);
    match repos_dir.map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => {
            if map.get(&key).map(String::as_str) == Some(value) {
                return Ok(());
            }
            map.insert(key, value.to_string());
        }
        None => {
            if map.remove(&key).is_none() {
                return Ok(());
            }
        }
    }
    write_repos_dir_map(nest_root, &map)
}

/// Move a workspace's per-relay `repos_dir` entry when the community's relay
/// URL is edited — the map analogue of `rebind_agent_relay_urls` for agent
/// records. Without this, agents rebound to the new URL would resolve no map
/// entry and silently fall back to the shared `REPOS` default. Returns
/// whether an entry moved; moving onto an existing entry overwrites it (the
/// edit just happened, so the edited community's value is freshest).
pub fn rebind_workspace_repos_dir(
    nest_root: &Path,
    old_relay_url: &str,
    new_relay_url: &str,
) -> Result<bool, String> {
    let old_key = crate::relay::normalize_relay_url(old_relay_url);
    let new_key = crate::relay::normalize_relay_url(new_relay_url);
    if new_key.is_empty() || old_key == new_key {
        return Ok(false);
    }
    let mut map = read_repos_dir_map(nest_root);
    let Some(value) = map.remove(&old_key) else {
        return Ok(false);
    };
    map.insert(new_key, value);
    write_repos_dir_map(nest_root, &map)?;
    Ok(true)
}

/// The raw persisted `repos_dir` for the workspace identified by `relay_url`,
/// or `None` when that workspace has no override (or has not been applied
/// since the per-relay map was introduced). Raw and un-canonicalized so it is
/// a stable spawn-config-hash input — filesystem state changes alone must not
/// flip the restart badge; [`resolve_agent_repos_dir`] does the real-path
/// resolution separately at spawn.
pub fn workspace_repos_dir_for_relay(nest_root: &Path, relay_url: &str) -> Option<String> {
    read_repos_dir_map(nest_root).remove(&crate::relay::normalize_relay_url(relay_url))
}

/// Resolve the repos directory a spawning agent should be handed
/// (`BUZZ_REPOS_DIR`), from the per-relay map entry for the agent's effective
/// relay.
///
/// - **Entry present** → the validated, canonical **real path** of that
///   workspace's `repos_dir`. Real, so the agent stays in its own workspace's
///   checkouts even while the `REPOS` symlink follows a different active
///   workspace. An unresolvable entry (unmounted volume, deleted dir) is an
///   `Err` — the spawn must fail CLOSED, mirroring [`resolve_repos_at_boot`]:
///   a refused start is recoverable, work landing in another workspace's
///   checkout is not.
/// - **No entry** → the nest's `REPOS` path (the shared in-nest default).
///   Deliberately NOT canonicalized: workspaces without an override share the
///   in-nest `REPOS` by design, and baking in a symlink target that happens
///   to be active now would be worse than the convention path.
pub fn resolve_agent_repos_dir(nest_root: &Path, relay_url: &str) -> Result<PathBuf, String> {
    match workspace_repos_dir_for_relay(nest_root, relay_url) {
        Some(dir) => validate_repos_dir(nest_root, &dir).map_err(|reason| {
            format!(
                "repos dir `{dir}` configured for this agent's workspace ({relay_url}) cannot be \
                 resolved: {reason}; refusing to start the agent so it cannot work in another \
                 workspace's repos directory"
            )
        }),
        None => Ok(nest_root.join("REPOS")),
    }
}

/// Decide whether launch-time agent restore is safe given the boot symlink
/// outcome. Fails CLOSED: when a `repos_dir` was configured but its symlink
/// could not be resolved, restoring agents would let one clone into the empty
/// real `REPOS` that `ensure_nest` provisioned — the wrong location — and once
/// it is non-empty [`ensure_repos_symlink`] refuses forever, permanently
/// re-triggering the race. Skipping restore is recoverable on the next boot
/// once the external target is reachable; a misplaced clone is not.
///
/// The no-`repos_dir` case (`persisted_present == false`) is always safe: the
/// real in-nest `REPOS` default is exactly where clones belong.
fn should_restore_agents(persisted_present: bool, symlink_result: &Result<(), String>) -> bool {
    !(persisted_present && symlink_result.is_err())
}

/// Resolve the `REPOS` symlink at boot from the persisted `repos_dir` and
/// report whether agent restore is safe to proceed.
///
/// Runs in the synchronous setup hook, after `ensure_nest` and before the
/// async agent restore is spawned, so `REPOS` is the user's configured symlink
/// before any agent can clone. Logs on failure (no toast path exists
/// pre-mount) and returns the fail-closed [`should_restore_agents`] decision.
pub fn resolve_repos_at_boot(nest_root: &Path) -> bool {
    let persisted = read_persisted_repos_dir(nest_root);
    let symlink_result = ensure_repos_symlink(nest_root, persisted.as_deref());
    if let Err(error) = &symlink_result {
        eprintln!("buzz-desktop: repos dir setup failed at boot: {error}");
    }
    let restore = should_restore_agents(persisted.is_some(), &symlink_result);
    // Log the resolved outcome on success so a healthy boot is observable (the
    // Err/skip branches already log; previously success was silent).
    if symlink_result.is_ok() {
        match persisted.as_deref() {
            Some(dir) => eprintln!(
                "buzz-desktop: repos dir resolved at boot — REPOS symlinked to configured `{dir}`"
            ),
            None => eprintln!(
                "buzz-desktop: repos dir resolved at boot — no configured override, REPOS is the default real dir"
            ),
        }
    }
    if !restore {
        eprintln!(
            "buzz-desktop: skipping agent restore — configured repos_dir `{}` could not be resolved at boot; will retry on next launch",
            persisted.as_deref().unwrap_or_default()
        );
    }
    restore
}

#[cfg(test)]
mod tests;
