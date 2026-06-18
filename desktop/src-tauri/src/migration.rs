//! Worktree data sync and on-launch reconciliation for the Buzz desktop app.
//!
//! **Worktree sync** (`sync_shared_agent_data`): Per-launch symlink creation
//! from the current worktree data directory to the canonical dev data
//! directory (`xyz.block.buzz.app.dev`). Only runs when
//! `BUZZ_SHARE_IDENTITY=1` and `BUZZ_PRIVATE_KEY` is set. All dev
//! instances share the same physical files — edits in any worktree are
//! immediately visible to all others.
//!
//! **Command reconciliation** (`reconcile_legacy_command_names`): Per-launch
//! fix-up of persisted built-in command names from the Sprout→Buzz rename.
//!
//! **Provider reconciliation** (`reconcile_provider_mcp_commands`): Per-launch
//! fix-up of `mcp_command` values in `managed-agents.json` against the
//! discovery table. Ensures known providers always have their canonical
//! `mcp_command`; unknown/custom agents are left untouched.

use std::path::{Path, PathBuf};
use tauri::Manager;

const CANONICAL_DEV_IDENTIFIER: &str = "xyz.block.buzz.app.dev";
const LEGACY_CANONICAL_DEV_IDENTIFIER: &str = "xyz.block.sprout.app.dev";
const LEGACY_RELEASE_IDENTIFIER: &str = "xyz.block.sprout.app";

/// JSON files symlinked from worktree data directories to the canonical
/// dev data directory. Only data files — never `agent-pids/` or `logs/`.
/// `identity.key` is deliberately excluded because worktree instances
/// receive their identity via the `BUZZ_PRIVATE_KEY` env var.
const SHARED_AGENT_FILES: &[&str] = &[
    "agents/managed-agents.json",
    "agents/personas.json",
    "agents/teams.json",
];

/// Directories symlinked from worktree data directories to the canonical
/// dev data directory. Each entry becomes a single directory symlink.
const SHARED_AGENT_DIRS: &[&str] = &["agents/teams"];

/// Create a symlink at `dst` pointing to `src`.
///
/// Worktree sync is a dev-only feature (`BUZZ_SHARE_IDENTITY=1`); on Windows
/// this is a no-op so the rest of `sync_shared_agent_data` keeps compiling and
/// running harmlessly.
#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(not(unix))]
fn symlink(_src: &Path, _dst: &Path) -> std::io::Result<()> {
    Ok(())
}

fn canonical_dev_data_dir(current: &Path) -> Option<PathBuf> {
    current.parent().map(|p| p.join(CANONICAL_DEV_IDENTIFIER))
}

fn legacy_app_data_dir(current: &Path) -> Option<PathBuf> {
    let name = current.file_name()?.to_str()?;
    let legacy_name = if name.starts_with(CANONICAL_DEV_IDENTIFIER) {
        name.replacen(CANONICAL_DEV_IDENTIFIER, LEGACY_CANONICAL_DEV_IDENTIFIER, 1)
    } else if name.starts_with("xyz.block.buzz.app") {
        name.replacen("xyz.block.buzz.app", LEGACY_RELEASE_IDENTIFIER, 1)
    } else {
        return None;
    };
    current.parent().map(|parent| parent.join(legacy_name))
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&src_path)?;
        if metadata.file_type().is_symlink() {
            #[cfg(unix)]
            {
                let target = std::fs::read_link(&src_path)?;
                if dst_path.exists() || dst_path.is_symlink() {
                    let _ = std::fs::remove_file(&dst_path);
                }
                std::os::unix::fs::symlink(target, &dst_path)?;
            }
            #[cfg(not(unix))]
            {
                continue;
            }
        } else if metadata.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else if metadata.is_file() {
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if !dst_path.exists() {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
    }
    Ok(())
}

/// Copy one-time app state from the legacy app identifier directory to
/// the current Buzz identifier directory. The Tauri identifier controls the app
/// data path, so without this copy a product rename would look like a fresh
/// install and users would lose their persisted identity and agent settings.
pub fn migrate_legacy_app_data_dir(app: &tauri::AppHandle) {
    let current_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("buzz-desktop: app-data-migration: cannot resolve app data dir: {e}");
            return;
        }
    };
    let Some(legacy_dir) = legacy_app_data_dir(&current_dir) else {
        return;
    };
    if !legacy_dir.exists() {
        return;
    }
    match copy_dir_all(&legacy_dir, &current_dir) {
        Ok(()) => eprintln!(
            "buzz-desktop: app-data-migration: copied legacy data from {} to {}",
            legacy_dir.display(),
            current_dir.display()
        ),
        Err(error) => eprintln!(
            "buzz-desktop: app-data-migration: failed to copy {} to {}: {error}",
            legacy_dir.display(),
            current_dir.display()
        ),
    }
}

/// Read a JSON array of objects from `path`, apply `f` to each object,
/// and write back if any mutation returned `true`.
fn patch_json_records(
    path: &Path,
    mut f: impl FnMut(&mut serde_json::Map<String, serde_json::Value>) -> bool,
) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut records) = serde_json::from_str::<Vec<serde_json::Value>>(&content) else {
        eprintln!(
            "buzz-desktop: patch-json-records: failed to parse {}",
            path.display()
        );
        return;
    };
    let mut changed = false;
    for record in &mut records {
        if let Some(obj) = record.as_object_mut() {
            changed |= f(obj);
        }
    }
    if changed {
        if let Ok(bytes) = serde_json::to_vec_pretty(&records) {
            if let Err(e) = crate::managed_agents::atomic_write_json(path, &bytes) {
                eprintln!("buzz-desktop: patch-json-records: {e}");
            }
        }
    }
}

/// Create symlinks for shared agent data files from the current (worktree)
/// data directory to the canonical dev data directory.
///
/// Guards:
/// - `BUZZ_SHARE_IDENTITY` must be `"1"`
/// - `BUZZ_PRIVATE_KEY` must parse as valid `nostr::Keys`
/// - The canonical dir must differ from the current dir (skip if we ARE canonical)
/// - The canonical dir must exist
pub fn sync_shared_agent_data(app: &tauri::AppHandle) {
    // Guard: only runs when sharing identity with a worktree.
    let is_shared = std::env::var("BUZZ_SHARE_IDENTITY")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !is_shared {
        return;
    }

    // Guard: BUZZ_PRIVATE_KEY must be a valid nostr key.
    let has_valid_key = std::env::var("BUZZ_PRIVATE_KEY")
        .ok()
        .and_then(|k| k.parse::<nostr::Keys>().ok())
        .is_some();
    if !has_valid_key {
        eprintln!("buzz-desktop: shared-agent-sync: BUZZ_PRIVATE_KEY missing or invalid, skipping");
        return;
    }

    let current_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("buzz-desktop: shared-agent-sync: cannot resolve app data dir: {e}");
            return;
        }
    };

    let canonical_dir = match canonical_dev_data_dir(&current_dir) {
        Some(dir) => dir,
        None => {
            eprintln!("buzz-desktop: shared-agent-sync: cannot compute canonical dir (no parent)");
            return;
        }
    };

    // Guard: skip if we ARE the canonical instance.
    // Use canonicalize to handle case-insensitive FS and symlinks.
    let current_canonical =
        std::fs::canonicalize(&current_dir).unwrap_or_else(|_| current_dir.clone());
    let source_canonical =
        std::fs::canonicalize(&canonical_dir).unwrap_or_else(|_| canonical_dir.clone());
    if current_canonical == source_canonical {
        return;
    }

    // Guard: skip if canonical dir doesn't exist.
    if !canonical_dir.exists() {
        eprintln!(
            "buzz-desktop: shared-agent-sync: canonical dir does not exist: {}",
            canonical_dir.display()
        );
        return;
    }

    // Seed-up: if canonical is missing a shared file but a sibling instance
    // holds real (non-symlink) content, migrate it up to canonical before the
    // symlink loop runs. Mirrors the SHARED_AGENT_DIRS migration below, applied
    // to individual files. Without this, a fresh write in a worktree is never
    // promoted to canonical and gets clobbered by the symlink step.
    for rel in SHARED_AGENT_FILES {
        let canonical_file = canonical_dir.join(rel);
        if canonical_file.exists() {
            continue;
        }
        let Some(parent) = canonical_dir.parent() else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let sibling = entry.path();
            if sibling == canonical_dir {
                continue;
            }
            let sibling_file = sibling.join(rel);
            if sibling_file.is_file() && !sibling_file.is_symlink() {
                if let Some(file_parent) = canonical_file.parent() {
                    if let Err(e) = std::fs::create_dir_all(file_parent) {
                        eprintln!(
                            "buzz-desktop: shared-agent-sync: failed to create {}: {e}",
                            file_parent.display()
                        );
                        break;
                    }
                }
                let _ = std::fs::rename(&sibling_file, &canonical_file);
                eprintln!(
                    "buzz-desktop: shared-agent-sync: seeded {rel} from {}",
                    sibling.display()
                );
                break;
            }
        }
    }

    let mut synced = 0u32;
    for rel in SHARED_AGENT_FILES {
        let src = canonical_dir.join(rel);
        let dst = current_dir.join(rel);

        if !src.exists() {
            continue;
        }

        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "buzz-desktop: shared-agent-sync: failed to create {}: {e}",
                    parent.display()
                );
                continue;
            }
        }

        // Already a correct symlink — nothing to do.
        if dst.is_symlink() {
            if let Ok(target) = std::fs::read_link(&dst) {
                if target == src {
                    continue;
                }
            }
        }

        // Remove whatever's at dst (regular file, wrong symlink, broken symlink).
        if dst.exists() || dst.is_symlink() {
            let _ = std::fs::remove_file(&dst);
        }

        match symlink(&src, &dst) {
            Ok(_) => synced += 1,
            Err(e) => {
                eprintln!("buzz-desktop: shared-agent-sync: failed to symlink {rel}: {e}");
            }
        }
    }

    // Ensure shared directories exist in canonical before symlinking.
    // Packs may have been installed in a sibling instance (e.g., `.main`)
    // before shared-dir syncing existed — migrate them to canonical.
    for rel in SHARED_AGENT_DIRS {
        let canonical_target = canonical_dir.join(rel);
        if !canonical_target.exists() {
            if let Err(e) = std::fs::create_dir_all(&canonical_target) {
                eprintln!(
                    "buzz-desktop: shared-agent-sync: failed to create {}: {e}",
                    canonical_target.display()
                );
            }
            // Migrate from whichever sibling has real (non-symlink) content.
            if let Some(parent) = canonical_dir.parent() {
                if let Ok(entries) = std::fs::read_dir(parent) {
                    for entry in entries.flatten() {
                        let sibling = entry.path();
                        if sibling == canonical_dir {
                            continue;
                        }
                        let sibling_dir = sibling.join(rel);
                        if sibling_dir.is_dir() && !sibling_dir.is_symlink() {
                            if let Ok(children) = std::fs::read_dir(&sibling_dir) {
                                for child in children.flatten() {
                                    let dest = canonical_target.join(child.file_name());
                                    if !dest.exists() {
                                        let _ = std::fs::rename(child.path(), &dest);
                                    }
                                }
                            }
                            // Replace the sibling's dir with a symlink to canonical.
                            let _ = std::fs::remove_dir_all(&sibling_dir);
                            let _ = symlink(&canonical_target, &sibling_dir);
                            eprintln!(
                                "buzz-desktop: shared-agent-sync: migrated {rel} from {}",
                                sibling.display()
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    for rel in SHARED_AGENT_DIRS {
        let src = canonical_dir.join(rel);
        let dst = current_dir.join(rel);

        if !src.exists() {
            continue;
        }

        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "buzz-desktop: shared-agent-sync: failed to create {}: {e}",
                    parent.display()
                );
                continue;
            }
        }

        if dst.is_symlink() {
            if let Ok(target) = std::fs::read_link(&dst) {
                if target == src {
                    continue;
                }
            }
        }

        if dst.is_symlink() {
            let _ = std::fs::remove_file(&dst);
        } else if dst.exists() {
            let _ = std::fs::remove_dir_all(&dst);
        }

        match symlink(&src, &dst) {
            Ok(_) => synced += 1,
            Err(e) => {
                eprintln!("buzz-desktop: shared-agent-sync: failed to symlink {rel}: {e}");
            }
        }
    }

    if synced > 0 {
        eprintln!(
            "buzz-desktop: shared-agent-sync: {synced} item(s) linked to {}",
            canonical_dir.display()
        );
    }
}

fn reconcile_team_dirs_in_file(path: &Path, target_dir: &Path) {
    // Build per-component so the persisted value uses native separators on
    // every platform, matching fresh writes (agents.rs builds the same path as
    // base.join("teams").join(id)). A single join("agents/teams") would embed a
    // literal '/' on Windows, persisting a mixed-separator path into the store.
    let target_teams = target_dir.join("agents").join("teams");
    patch_json_records(path, |obj| {
        // Handle both old field name and new field name
        let field_name = if obj.contains_key("persona_team_dir") {
            "persona_team_dir"
        } else if obj.contains_key("persona_pack_path") {
            "persona_pack_path"
        } else {
            return false;
        };
        let team_path = match obj.get(field_name).and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return false,
        };
        let team_path = Path::new(team_path);
        // Extract the team ID from the path (component after "teams" or "packs")
        let mut found_dir = false;
        let mut team_id: Option<&std::ffi::OsStr> = None;
        for component in team_path.components() {
            if found_dir {
                team_id = Some(component.as_os_str());
                break;
            }
            if component.as_os_str() == "teams" || component.as_os_str() == "packs" {
                found_dir = true;
            }
        }
        let Some(id) = team_id else {
            return false;
        };
        let expected = target_teams.join(id);
        if team_path == expected {
            // Value already correct — still normalize the legacy field name so
            // stores converge on `persona_team_dir` (runtime reads either via
            // serde alias).
            if field_name == "persona_pack_path" {
                if let Some(val) = obj.remove("persona_pack_path") {
                    obj.insert("persona_team_dir".to_string(), val);
                    return true;
                }
            }
            return false;
        }
        // Rewriting to a path that does not exist on disk makes things worse
        // than leaving a stale-but-working path in place. fs::metadata follows
        // symlinks, so a valid symlinked install passes; a dangling symlink
        // fails with NotFound.
        if let Err(e) = std::fs::metadata(&expected) {
            eprintln!(
                "buzz-desktop: team-dir-reconcile: {:?}: {:?} expected at {:?} — {e}, leaving as-is",
                obj.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                team_path,
                expected,
            );
            return false;
        }
        let Some(expected_str) = expected.to_str() else {
            eprintln!(
                "buzz-desktop: team-dir-reconcile: {:?}: expected path {:?} is not valid UTF-8, leaving as-is",
                obj.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                expected,
            );
            return false;
        };
        eprintln!(
            "buzz-desktop: team-dir-reconcile: {:?}: {:?} → {:?}",
            obj.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
            team_path,
            expected,
        );
        // Always write the new field name
        obj.remove("persona_pack_path");
        obj.insert(
            "persona_team_dir".to_string(),
            serde_json::Value::String(expected_str.to_owned()),
        );
        true
    });
}

/// Select the data directory to reconcile against.
///
/// Dev instances — identified by the data-dir name starting with
/// `CANONICAL_DEV_IDENTIFIER` (covers the canonical dir itself and any
/// worktree variant like `xyz.block.buzz.app.dev.mybranch`) — share
/// `agents/managed-agents.json` and `agents/teams` via symlinks to the
/// canonical dev dir, so they should normalize against that canonical dir.
///
/// Release builds must reconcile their own data dir — keying off the canonical
/// dev dir's mere existence would leave release records permanently stale on
/// developer machines, where that dir is always present.
fn reconcile_target_dir(current_dir: &Path) -> PathBuf {
    let is_dev_instance = current_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(CANONICAL_DEV_IDENTIFIER));
    if is_dev_instance {
        match canonical_dev_data_dir(current_dir) {
            Some(dir) if dir.exists() => dir,
            _ => current_dir.to_path_buf(),
        }
    } else {
        current_dir.to_path_buf()
    }
}

/// Reconcile `persona_team_dir` (and legacy `persona_pack_path`) values in
/// managed-agents.json to point to the correct `agents/teams/` prefix.
///
/// Fixes two classes of stale paths:
/// - Worktree dev instances whose records point at a sibling data dir rather
///   than the canonical dev dir (dev instances share managed-agents.json and
///   agents/teams via symlinks, so they all reconcile against the canonical dir).
/// - Legacy paths left by historical renames: `agents/packs/` → `agents/teams/`
///   (the packs→teams consolidation) and bundle-id `xyz.block.sprout.app` →
///   `xyz.block.buzz.app` (the sprout→buzz rename, which moved the app data dir).
///
/// Release builds reconcile their own data dir — choosing the canonical dev dir
/// whenever it exists would leave release files permanently stale on developer
/// machines.
pub fn reconcile_persona_team_dirs(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    // Single-dir on purpose: unlike reconcile_legacy_command_names and
    // reconcile_provider_mcp_commands, which patch both [current, canonical],
    // path rewrites are target-dependent — a dual pass through a dev
    // instance's symlinked store would write worktree-local paths into the
    // shared canonical file.
    let target_dir = reconcile_target_dir(&current_dir);
    let path = target_dir.join("agents/managed-agents.json");
    if !path.exists() {
        return;
    }
    reconcile_team_dirs_in_file(&path, &target_dir);
}

/// One-time migration from packs to teams.
///
/// Runs on app launch if `agents/packs/` exists or if any record in
/// `managed-agents.json` still uses the old `persona_pack_path` field name.
/// Steps (in order, each individually idempotent):
///
/// 1. Rename `agents/packs/` → `agents/teams/` on disk
/// 2. Rewrite `personas.json`: `source_pack` → `source_team`, `source_pack_persona_slug` → `source_team_persona_slug`
/// 3. Rewrite `managed-agents.json`: `persona_pack_path` → `persona_team_dir` (with `/packs/` → `/teams/` path fix), `persona_name_in_pack` → `persona_name_in_team`
pub fn migrate_packs_to_teams(app: &tauri::AppHandle) {
    use crate::managed_agents::MigrationReport;

    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let target_dir = reconcile_target_dir(&current_dir);

    let packs_dir = target_dir.join("agents/packs");
    let teams_dir = target_dir.join("agents/teams");
    let personas_path = target_dir.join("agents/personas.json");
    let agents_path = target_dir.join("agents/managed-agents.json");

    // Check if migration is needed: packs dir exists OR agents JSON has old field names
    let packs_dir_exists = packs_dir.exists() && !packs_dir.is_symlink();
    let has_old_fields = agents_path.exists()
        && std::fs::read_to_string(&agents_path)
            .map(|c| c.contains("persona_pack_path"))
            .unwrap_or(false);
    let personas_has_old_fields = personas_path.exists()
        && std::fs::read_to_string(&personas_path)
            .map(|c| c.contains("\"source_pack\""))
            .unwrap_or(false);

    if !packs_dir_exists && !has_old_fields && !personas_has_old_fields {
        return;
    }

    let mut report = MigrationReport {
        packs_migrated: 0,
        personas_updated: 0,
        agents_updated: 0,
        errors: Vec::new(),
    };

    // Step 1: Rename directory agents/packs/ → agents/teams/
    if packs_dir_exists {
        if teams_dir.exists() {
            // Merge: move contents from packs into teams, skip conflicts
            if let Ok(entries) = std::fs::read_dir(&packs_dir) {
                for entry in entries.flatten() {
                    let dest = teams_dir.join(entry.file_name());
                    if !dest.exists() {
                        if let Err(e) = std::fs::rename(entry.path(), &dest) {
                            report
                                .errors
                                .push(format!("failed to move {:?}: {e}", entry.file_name()));
                        } else {
                            report.packs_migrated += 1;
                        }
                    }
                }
            }
            // Remove packs dir only if empty (external tools like ai-rules
            // may have recreated symlinks here between migration runs)
            let _ = std::fs::remove_dir(&packs_dir);
        } else {
            // Simple rename
            if let Some(parent) = teams_dir.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::rename(&packs_dir, &teams_dir) {
                Ok(_) => {
                    if let Ok(entries) = std::fs::read_dir(&teams_dir) {
                        report.packs_migrated = entries.count();
                    }
                }
                Err(e) => {
                    report
                        .errors
                        .push(format!("failed to rename packs → teams: {e}"));
                    eprintln!("buzz-desktop: packs→teams migration: directory rename failed: {e}");
                    return;
                }
            }
        }
    }

    // Step 2: Rewrite personas.json field names
    if personas_path.exists() {
        patch_json_records(&personas_path, |obj| {
            let mut changed = false;
            if let Some(val) = obj.remove("source_pack") {
                obj.insert("source_team".to_string(), val);
                changed = true;
            }
            if let Some(val) = obj.remove("source_pack_persona_slug") {
                obj.insert("source_team_persona_slug".to_string(), val);
                changed = true;
            }
            if changed {
                report.personas_updated += 1;
            }
            changed
        });
    }

    // Step 3: Rewrite managed-agents.json field names and paths
    if agents_path.exists() {
        patch_json_records(&agents_path, |obj| {
            let mut changed = false;
            if let Some(val) = obj.remove("persona_pack_path") {
                // Also fix the path: replace /packs/ with /teams/
                let new_val = if let Some(s) = val.as_str() {
                    serde_json::Value::String(s.replace("/packs/", "/teams/"))
                } else {
                    val
                };
                obj.insert("persona_team_dir".to_string(), new_val);
                changed = true;
            }
            if let Some(val) = obj.remove("persona_name_in_pack") {
                obj.insert("persona_name_in_team".to_string(), val);
                changed = true;
            }
            if changed {
                report.agents_updated += 1;
            }
            changed
        });
    }

    if report.packs_migrated > 0 || report.personas_updated > 0 || report.agents_updated > 0 {
        eprintln!(
            "buzz-desktop: packs→teams migration complete: {} dirs, {} personas, {} agents{}",
            report.packs_migrated,
            report.personas_updated,
            report.agents_updated,
            if report.errors.is_empty() {
                String::new()
            } else {
                format!(" ({} errors)", report.errors.len())
            }
        );
    }
}

fn reconcile_mcp_commands_in_file(path: &Path) {
    patch_json_records(path, |obj| {
        let agent_command = match obj.get("agent_command").and_then(|v| v.as_str()) {
            Some(cmd) => cmd.to_string(),
            None => return false,
        };
        let Some(runtime) = crate::managed_agents::known_acp_runtime(&agent_command) else {
            return false;
        };
        let expected = runtime.mcp_command.unwrap_or("");
        let current = obj
            .get("mcp_command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if current == expected {
            return false;
        }
        // Only fix values that are clearly stale (empty or a removed binary).
        // Leave user-customized values untouched.
        if !current.is_empty() && current != "buzz-mcp-server" {
            return false;
        }
        eprintln!(
            "buzz-desktop: runtime-reconcile: {:?} ({:?}): mcp_command {:?} → {:?}",
            obj.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
            agent_command,
            current,
            expected,
        );
        obj.insert(
            "mcp_command".to_string(),
            serde_json::Value::String(expected.to_string()),
        );
        true
    });
}

fn replace_command_field(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    replacement: String,
) -> bool {
    let Some(current) = obj.get(field).and_then(|v| v.as_str()) else {
        return false;
    };
    if current == replacement {
        return false;
    }
    eprintln!(
        "buzz-desktop: command-rename-reconcile: {:?}: {field} {:?} → {:?}",
        obj.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
        current,
        replacement,
    );
    obj.insert(field.to_string(), serde_json::Value::String(replacement));
    true
}

fn reconcile_legacy_command_names_in_file(path: &Path) {
    patch_json_records(path, |obj| {
        let mut changed = false;

        if let Some(acp_command) = obj
            .get("acp_command")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        {
            if acp_command == "sprout-acp" {
                changed |= replace_command_field(obj, "acp_command", "buzz-acp".to_string());
            }
        }

        let mut agent_command = obj
            .get("agent_command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if agent_command == "sprout-agent" {
            agent_command = "buzz-agent".to_string();
            changed |= replace_command_field(obj, "agent_command", agent_command.clone());
        }

        if let Some(mcp_command) = obj
            .get("mcp_command")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        {
            match mcp_command.as_str() {
                "sprout-dev-mcp" => {
                    changed |=
                        replace_command_field(obj, "mcp_command", "buzz-dev-mcp".to_string());
                }
                "sprout-mcp" | "sprout-mcp-server" | "buzz-mcp-server" => {
                    let replacement = if agent_command == "buzz-agent" {
                        "buzz-dev-mcp"
                    } else {
                        ""
                    };
                    changed |= replace_command_field(obj, "mcp_command", replacement.to_string());
                }
                _ => {}
            }
        }

        changed
    });
}

fn reconcile_legacy_persona_runtimes_in_file(path: &Path) {
    patch_json_records(path, |obj| {
        let Some(runtime) = obj.get("runtime").and_then(|v| v.as_str()) else {
            return false;
        };
        if runtime != "sprout-agent" {
            return false;
        }
        eprintln!(
            "buzz-desktop: command-rename-reconcile: persona {:?}: runtime {:?} → {:?}",
            obj.get("display_name")
                .or_else(|| obj.get("displayName"))
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
            runtime,
            "buzz-agent",
        );
        obj.insert(
            "runtime".to_string(),
            serde_json::Value::String("buzz-agent".to_string()),
        );
        true
    });
}

fn rewrite_legacy_persona_md_runtime(content: &str) -> Option<String> {
    let (frontmatter, body) = buzz_persona_pkg::persona::split_frontmatter(content).ok()?;
    let mut value = serde_yaml::from_str::<serde_yaml::Value>(frontmatter).ok()?;
    let mapping = value.as_mapping_mut()?;
    let runtime = mapping.get_mut(serde_yaml::Value::String("runtime".to_string()))?;
    if runtime.as_str()? != "sprout-agent" {
        return None;
    }
    *runtime = serde_yaml::Value::String("buzz-agent".to_string());
    let frontmatter = serde_yaml::to_string(&value).ok()?;
    Some(format!("---\n{frontmatter}---\n{body}"))
}

fn reconcile_legacy_team_persona_runtime_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            reconcile_legacy_team_persona_runtime_files(&path);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".persona.md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(updated) = rewrite_legacy_persona_md_runtime(&content) else {
            continue;
        };
        if updated == content {
            continue;
        }
        match std::fs::write(&path, updated) {
            Ok(()) => {
                eprintln!(
                    "buzz-desktop: command-rename-reconcile: updated {}",
                    path.display()
                );
            }
            Err(error) => {
                eprintln!(
                    "buzz-desktop: command-rename-reconcile: failed to update {}: {error}",
                    path.display()
                );
            }
        }
    }
}

/// Reconcile exact built-in command values persisted before the Sprout→Buzz
/// rename. Custom commands and explicit paths are left untouched.
pub fn reconcile_legacy_command_names(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }
    for dir in dirs {
        let path = dir.join("agents/managed-agents.json");
        if path.exists() {
            reconcile_legacy_command_names_in_file(&path);
        }
        let personas_path = dir.join("agents/personas.json");
        if personas_path.exists() {
            reconcile_legacy_persona_runtimes_in_file(&personas_path);
        }
        let teams_dir = dir.join("agents/teams");
        if teams_dir.exists() && !teams_dir.is_symlink() {
            reconcile_legacy_team_persona_runtime_files(&teams_dir);
        }
    }
}

/// Reconcile `mcp_command` values in managed-agents.json against the
/// discovery table. Known runtimes get their canonical mcp_command;
/// unknown/custom agents are left untouched. Covers both the current
/// app data dir and the canonical dev data dir (for worktree instances).
pub fn reconcile_provider_mcp_commands(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }
    for dir in dirs {
        let path = dir.join("agents/managed-agents.json");
        if path.exists() {
            reconcile_mcp_commands_in_file(&path);
        }
    }
}

fn rename_provider_to_runtime_in_personas(path: &Path) {
    patch_json_records(path, |obj| {
        if obj.contains_key("runtime") {
            return false;
        }
        if let Some(value) = obj.remove("provider") {
            obj.insert("runtime".to_string(), value);
            true
        } else {
            false
        }
    });
}

pub fn migrate_persona_provider_to_runtime(app: &tauri::AppHandle) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let path = dir.join("agents/personas.json");
    if !path.exists() {
        return;
    }
    rename_provider_to_runtime_in_personas(&path);
}

#[cfg(test)]
#[path = "migration_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "migration_command_tests.rs"]
mod command_tests;

#[cfg(test)]
#[path = "migration_team_dir_tests.rs"]
mod team_dir_tests;
