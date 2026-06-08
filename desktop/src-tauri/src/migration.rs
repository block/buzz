//! Worktree data sync and on-launch reconciliation for the Sprout desktop app.
//!
//! **Worktree sync** (`sync_shared_agent_data`): Per-launch symlink creation
//! from the current worktree data directory to the canonical dev data
//! directory (`xyz.block.sprout.app.dev`). Only runs when
//! `SPROUT_SHARE_IDENTITY=1` and `SPROUT_PRIVATE_KEY` is set. All dev
//! instances share the same physical files — edits in any worktree are
//! immediately visible to all others.

use std::path::{Path, PathBuf};
use tauri::Manager;

const CANONICAL_DEV_IDENTIFIER: &str = "xyz.block.sprout.app.dev";

/// JSON files symlinked from worktree data directories to the canonical
/// dev data directory. Only data files — never `agent-pids/` or `logs/`.
/// `identity.key` is deliberately excluded because worktree instances
/// receive their identity via the `SPROUT_PRIVATE_KEY` env var.
const SHARED_AGENT_FILES: &[&str] = &[
    "agents/managed-agents.json",
    "agents/personas.json",
    "agents/teams.json",
];

/// Directories symlinked from worktree data directories to the canonical
/// dev data directory. Each entry becomes a single directory symlink.
const SHARED_AGENT_DIRS: &[&str] = &["agents/teams"];

fn canonical_dev_data_dir(current: &Path) -> Option<PathBuf> {
    current.parent().map(|p| p.join(CANONICAL_DEV_IDENTIFIER))
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
            "sprout-desktop: patch-json-records: failed to parse {}",
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
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// Create symlinks for shared agent data files from the current (worktree)
/// data directory to the canonical dev data directory.
///
/// Guards:
/// - `SPROUT_SHARE_IDENTITY` must be `"1"`
/// - `SPROUT_PRIVATE_KEY` must parse as valid `nostr::Keys`
/// - The canonical dir must differ from the current dir (skip if we ARE canonical)
/// - The canonical dir must exist
pub fn sync_shared_agent_data(app: &tauri::AppHandle) {
    // Guard: only runs when sharing identity with a worktree.
    let is_shared = std::env::var("SPROUT_SHARE_IDENTITY")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !is_shared {
        return;
    }

    // Guard: SPROUT_PRIVATE_KEY must be a valid nostr key.
    let has_valid_key = std::env::var("SPROUT_PRIVATE_KEY")
        .ok()
        .and_then(|k| k.parse::<nostr::Keys>().ok())
        .is_some();
    if !has_valid_key {
        eprintln!(
            "sprout-desktop: shared-agent-sync: SPROUT_PRIVATE_KEY missing or invalid, skipping"
        );
        return;
    }

    let current_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("sprout-desktop: shared-agent-sync: cannot resolve app data dir: {e}");
            return;
        }
    };

    let canonical_dir = match canonical_dev_data_dir(&current_dir) {
        Some(dir) => dir,
        None => {
            eprintln!(
                "sprout-desktop: shared-agent-sync: cannot compute canonical dir (no parent)"
            );
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
            "sprout-desktop: shared-agent-sync: canonical dir does not exist: {}",
            canonical_dir.display()
        );
        return;
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
                    "sprout-desktop: shared-agent-sync: failed to create {}: {e}",
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

        match std::os::unix::fs::symlink(&src, &dst) {
            Ok(_) => synced += 1,
            Err(e) => {
                eprintln!("sprout-desktop: shared-agent-sync: failed to symlink {rel}: {e}");
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
                    "sprout-desktop: shared-agent-sync: failed to create {}: {e}",
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
                            let _ = std::os::unix::fs::symlink(&canonical_target, &sibling_dir);
                            eprintln!(
                                "sprout-desktop: shared-agent-sync: migrated {rel} from {}",
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
                    "sprout-desktop: shared-agent-sync: failed to create {}: {e}",
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

        match std::os::unix::fs::symlink(&src, &dst) {
            Ok(_) => synced += 1,
            Err(e) => {
                eprintln!("sprout-desktop: shared-agent-sync: failed to symlink {rel}: {e}");
            }
        }
    }

    if synced > 0 {
        eprintln!(
            "sprout-desktop: shared-agent-sync: {synced} item(s) linked to {}",
            canonical_dir.display()
        );
    }
}

fn reconcile_team_dirs_in_file(path: &Path, canonical_dir: &Path) {
    let canonical_teams = canonical_dir.join("agents/teams");
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
        let expected = canonical_teams.join(id);
        if team_path == expected {
            return false;
        }
        eprintln!(
            "sprout-desktop: team-dir-reconcile: {:?}: {:?} → {:?}",
            obj.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
            team_path,
            expected,
        );
        // Always write the canonical new field name
        obj.remove("persona_pack_path");
        obj.insert(
            "persona_team_dir".to_string(),
            serde_json::Value::String(expected.to_string_lossy().into_owned()),
        );
        true
    });
}

/// Reconcile `persona_team_dir` (and legacy `persona_pack_path`) values in
/// managed-agents.json to point to the canonical dev data directory's
/// `agents/teams/` prefix. Fixes stale paths left when agents were created
/// from worktree instances whose data directories don't have local team copies.
pub fn reconcile_persona_team_dirs(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let canonical_dir = match canonical_dev_data_dir(&current_dir) {
        Some(dir) if dir.exists() => dir,
        _ => current_dir,
    };
    let path = canonical_dir.join("agents/managed-agents.json");
    if !path.exists() {
        return;
    }
    reconcile_team_dirs_in_file(&path, &canonical_dir);
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
    let canonical_dir = match canonical_dev_data_dir(&current_dir) {
        Some(dir) if dir.exists() => dir,
        _ => current_dir,
    };

    let packs_dir = canonical_dir.join("agents/packs");
    let teams_dir = canonical_dir.join("agents/teams");
    let personas_path = canonical_dir.join("agents/personas.json");
    let agents_path = canonical_dir.join("agents/managed-agents.json");

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
            // Remove the now-empty packs dir
            let _ = std::fs::remove_dir_all(&packs_dir);
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
                    eprintln!(
                        "sprout-desktop: packs→teams migration: directory rename failed: {e}"
                    );
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
            "sprout-desktop: packs→teams migration complete: {} dirs, {} personas, {} agents{}",
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
        if !current.is_empty() && current != "sprout-mcp-server" {
            return false;
        }
        eprintln!(
            "sprout-desktop: runtime-reconcile: {:?} ({:?}): mcp_command {:?} → {:?}",
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
mod tests {
    use super::*;

    #[test]
    fn canonical_dev_data_dir_replaces_last_component() {
        let current = PathBuf::from(
            "/Users/me/Library/Application Support/xyz.block.sprout.app.dev.my-branch",
        );
        let canonical = canonical_dev_data_dir(&current).unwrap();
        assert_eq!(
            canonical,
            PathBuf::from("/Users/me/Library/Application Support/xyz.block.sprout.app.dev")
        );
    }

    #[test]
    fn canonical_dev_data_dir_returns_none_for_root() {
        // A root path has no parent — should return None.
        assert!(canonical_dev_data_dir(Path::new("/")).is_none());
    }

    /// Helper: create a temp dir structure mimicking canonical + worktree layout.
    /// Packs live in a `.main` sibling (not canonical) to match real-world state.
    /// Returns `(parent_dir_handle, canonical_dir, worktree_dir)`.
    fn setup_sync_layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        let worktree = parent.path().join("xyz.block.sprout.app.dev.my-branch");
        let main_instance = parent.path().join("xyz.block.sprout.app.dev.main");

        std::fs::create_dir_all(canonical.join("agents")).unwrap();
        std::fs::write(
            canonical.join("agents/managed-agents.json"),
            r#"[{"id":"agent-1"}]"#,
        )
        .unwrap();
        std::fs::write(
            canonical.join("agents/personas.json"),
            r#"[{"id":"builtin:solo"}]"#,
        )
        .unwrap();
        std::fs::write(canonical.join("agents/teams.json"), r#"[{"id":"team-1"}]"#).unwrap();

        // Teams installed from `.main` — canonical has no teams dir.
        let team_dir = main_instance.join("agents/teams/com.example.test-pack");
        std::fs::create_dir_all(&team_dir).unwrap();
        std::fs::write(team_dir.join("instructions.md"), "# Test pack").unwrap();
        std::fs::write(team_dir.join("solo.persona.md"), "# Solo").unwrap();

        (parent, canonical, worktree)
    }

    /// Helper: sync files directly (without a Tauri AppHandle) for unit testing.
    /// Mirrors the symlink loop of `sync_shared_agent_data` but takes explicit
    /// paths. `sync_shared_agent_data` requires a live Tauri AppHandle and
    /// cannot be unit-tested directly.
    fn sync_files(canonical: &Path, worktree: &Path) -> u32 {
        let mut synced = 0u32;
        for rel in SHARED_AGENT_FILES {
            let src = canonical.join(rel);
            let dst = worktree.join(rel);
            if !src.exists() {
                continue;
            }
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            if dst.is_symlink() {
                if let Ok(target) = std::fs::read_link(&dst) {
                    if target == src {
                        continue;
                    }
                }
            }
            if dst.exists() || dst.is_symlink() {
                let _ = std::fs::remove_file(&dst);
            }
            std::os::unix::fs::symlink(&src, &dst).unwrap();
            synced += 1;
        }
        // Migrate packs from siblings to canonical (mirrors production logic).
        for rel in SHARED_AGENT_DIRS {
            let canonical_target = canonical.join(rel);
            if !canonical_target.exists() {
                std::fs::create_dir_all(&canonical_target).unwrap();
                if let Some(parent) = canonical.parent() {
                    if let Ok(entries) = std::fs::read_dir(parent) {
                        for entry in entries.flatten() {
                            let sibling = entry.path();
                            if sibling == canonical {
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
                                let _ = std::fs::remove_dir_all(&sibling_dir);
                                let _ = std::os::unix::fs::symlink(&canonical_target, &sibling_dir);
                                break;
                            }
                        }
                    }
                }
            }
        }

        for rel in SHARED_AGENT_DIRS {
            let src = canonical.join(rel);
            let dst = worktree.join(rel);
            if !src.exists() {
                continue;
            }
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).unwrap();
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
            std::os::unix::fs::symlink(&src, &dst).unwrap();
            synced += 1;
        }
        synced
    }

    #[test]
    fn sync_creates_symlinks_to_fresh_worktree() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        let synced = sync_files(&canonical, &worktree);
        assert_eq!(synced, 4);
        for rel in SHARED_AGENT_FILES {
            let dst = worktree.join(rel);
            assert!(dst.is_symlink(), "{rel} should be a symlink");
            assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
        }
        for rel in SHARED_AGENT_DIRS {
            let dst = worktree.join(rel);
            assert!(dst.is_symlink(), "{rel} should be a symlink");
            assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
        }
        assert_eq!(
            std::fs::read_to_string(worktree.join("agents/managed-agents.json")).unwrap(),
            r#"[{"id":"agent-1"}]"#,
        );
    }

    #[test]
    fn sync_replaces_existing_files_with_symlinks() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        std::fs::create_dir_all(worktree.join("agents")).unwrap();
        std::fs::write(worktree.join("agents/managed-agents.json"), "[]").unwrap();
        std::fs::write(worktree.join("agents/personas.json"), "[]").unwrap();
        std::fs::write(worktree.join("agents/teams.json"), "[]").unwrap();

        let synced = sync_files(&canonical, &worktree);

        assert_eq!(synced, 4);
        for rel in SHARED_AGENT_FILES {
            let dst = worktree.join(rel);
            assert!(
                dst.is_symlink(),
                "{rel} should be a symlink after replacing regular file"
            );
            assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
        }
        assert_eq!(
            std::fs::read_to_string(worktree.join("agents/managed-agents.json")).unwrap(),
            r#"[{"id":"agent-1"}]"#,
        );
    }

    #[test]
    fn sync_preserves_correct_symlinks() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        assert_eq!(sync_files(&canonical, &worktree), 4);
        assert_eq!(sync_files(&canonical, &worktree), 0);
        for rel in SHARED_AGENT_FILES {
            let dst = worktree.join(rel);
            assert!(dst.is_symlink());
            assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
        }
    }

    #[test]
    fn sync_replaces_wrong_symlinks() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        let wrong_target = PathBuf::from("/nonexistent/wrong-target.json");
        std::fs::create_dir_all(worktree.join("agents")).unwrap();
        for rel in SHARED_AGENT_FILES {
            std::os::unix::fs::symlink(&wrong_target, worktree.join(rel)).unwrap();
        }
        let synced = sync_files(&canonical, &worktree);
        assert_eq!(synced, 4);
        for rel in SHARED_AGENT_FILES {
            assert_eq!(
                std::fs::read_link(worktree.join(rel)).unwrap(),
                canonical.join(rel)
            );
        }
    }

    #[test]
    fn sync_handles_broken_symlinks() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        std::fs::create_dir_all(worktree.join("agents")).unwrap();
        let broken_target = PathBuf::from("/this/does/not/exist.json");
        for rel in SHARED_AGENT_FILES {
            std::os::unix::fs::symlink(&broken_target, worktree.join(rel)).unwrap();
        }
        let synced = sync_files(&canonical, &worktree);
        assert_eq!(synced, 4);
        for rel in SHARED_AGENT_FILES {
            let dst = worktree.join(rel);
            assert!(dst.is_symlink());
            assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
            // Content should be readable through the fixed symlink.
            assert!(std::fs::read_to_string(&dst).is_ok());
        }
    }

    #[test]
    fn writes_through_symlink_reach_canonical() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        sync_files(&canonical, &worktree);

        let worktree_path = worktree.join("agents/personas.json");
        let canonical_path = canonical.join("agents/personas.json");

        // Write through the symlink using the same pattern as atomic_write_json.
        let new_content = r#"[{"id":"builtin:solo","updated":true}]"#;
        let resolved = std::fs::canonicalize(&worktree_path).unwrap();
        let tmp = resolved.with_extension("json.tmp");
        std::fs::write(&tmp, new_content.as_bytes()).unwrap();
        std::fs::rename(&tmp, &resolved).unwrap();

        // The canonical file should have the new content.
        assert_eq!(
            std::fs::read_to_string(&canonical_path).unwrap(),
            new_content
        );
        // The worktree path should still be a symlink.
        assert!(worktree_path.is_symlink());
        // Reading through the symlink should return the new content.
        assert_eq!(
            std::fs::read_to_string(&worktree_path).unwrap(),
            new_content
        );
    }

    #[test]
    fn canonical_dev_data_dir_returns_self_for_canonical_instance() {
        // When the current app data dir IS the canonical dev identifier,
        // canonical_dev_data_dir returns the exact same path — the caller
        // (sync_shared_agent_data) uses this equality to skip the sync.
        // The env-var guards (SPROUT_SHARE_IDENTITY, SPROUT_PRIVATE_KEY)
        // require a live Tauri AppHandle and are covered by integration
        // testing only.
        let current =
            PathBuf::from("/Users/me/Library/Application Support/xyz.block.sprout.app.dev");
        assert_eq!(canonical_dev_data_dir(&current).unwrap(), current);

        // Also verify with a temp dir on the real filesystem.
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        assert_eq!(canonical_dev_data_dir(&canonical).unwrap(), canonical);
    }

    fn write_agents_json(dir: &Path, records: &serde_json::Value) {
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(
            dir.join("agents/managed-agents.json"),
            serde_json::to_vec_pretty(records).unwrap(),
        )
        .unwrap();
    }

    fn read_agents_json(dir: &Path) -> Vec<serde_json::Value> {
        let content = std::fs::read_to_string(dir.join("agents/managed-agents.json")).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    #[test]
    fn sync_creates_teams_directory_symlink() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        sync_files(&canonical, &worktree);

        let teams_link = worktree.join("agents/teams");
        assert!(teams_link.is_symlink());
        assert_eq!(
            std::fs::read_link(&teams_link).unwrap(),
            canonical.join("agents/teams")
        );
        assert_eq!(
            std::fs::read_to_string(
                worktree.join("agents/teams/com.example.test-pack/instructions.md")
            )
            .unwrap(),
            "# Test pack"
        );
    }

    #[test]
    fn sync_migrates_teams_from_sibling_to_canonical() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        let main_instance = canonical
            .parent()
            .unwrap()
            .join("xyz.block.sprout.app.dev.main");

        // Before sync: canonical has no teams, .main has the real team dir.
        assert!(!canonical.join("agents/teams").exists());
        assert!(main_instance
            .join("agents/teams/com.example.test-pack")
            .is_dir());

        sync_files(&canonical, &worktree);

        // After sync: canonical has the team, .main is now a symlink.
        assert!(canonical
            .join("agents/teams/com.example.test-pack/instructions.md")
            .exists());
        assert!(main_instance.join("agents/teams").is_symlink());
        assert_eq!(
            std::fs::read_link(main_instance.join("agents/teams")).unwrap(),
            canonical.join("agents/teams")
        );
    }

    #[test]
    fn sync_replaces_real_teams_dir_with_symlink() {
        let (_parent, canonical, worktree) = setup_sync_layout();
        let real_teams = worktree.join("agents/teams");
        std::fs::create_dir_all(&real_teams).unwrap();
        std::fs::write(real_teams.join("stale-file.txt"), "stale").unwrap();

        sync_files(&canonical, &worktree);

        assert!(worktree.join("agents/teams").is_symlink());
        assert_eq!(
            std::fs::read_link(worktree.join("agents/teams")).unwrap(),
            canonical.join("agents/teams")
        );
    }

    #[test]
    fn team_dir_reconcile_rewrites_worktree_path() {
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        std::fs::create_dir_all(canonical.join("agents")).unwrap();

        let worktree_pack_path = format!(
            "{}/agents/packs/com.wpfleger.sietch-tabr",
            parent
                .path()
                .join("xyz.block.sprout.app.dev.worktree-my-branch")
                .display()
        );
        let expected_path = format!(
            "{}/agents/teams/com.wpfleger.sietch-tabr",
            canonical.display()
        );

        write_agents_json(
            &canonical,
            &serde_json::json!([{
                "name": "Paul",
                "persona_pack_path": worktree_pack_path
            }]),
        );

        reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);

        let records = read_agents_json(&canonical);
        assert_eq!(records[0]["persona_team_dir"], expected_path);
        // Old field name should be removed
        assert!(records[0].get("persona_pack_path").is_none());
    }

    #[test]
    fn team_dir_reconcile_rewrites_new_field_name() {
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        std::fs::create_dir_all(canonical.join("agents")).unwrap();

        let worktree_team_path = format!(
            "{}/agents/teams/com.wpfleger.sietch-tabr",
            parent
                .path()
                .join("xyz.block.sprout.app.dev.worktree-my-branch")
                .display()
        );
        let expected_path = format!(
            "{}/agents/teams/com.wpfleger.sietch-tabr",
            canonical.display()
        );

        write_agents_json(
            &canonical,
            &serde_json::json!([{
                "name": "Paul",
                "persona_team_dir": worktree_team_path
            }]),
        );

        reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);

        let records = read_agents_json(&canonical);
        assert_eq!(records[0]["persona_team_dir"], expected_path);
    }

    #[test]
    fn team_dir_reconcile_leaves_canonical_path_unchanged() {
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        std::fs::create_dir_all(canonical.join("agents")).unwrap();

        let canonical_path = format!(
            "{}/agents/teams/com.wpfleger.sietch-tabr",
            canonical.display()
        );

        write_agents_json(
            &canonical,
            &serde_json::json!([{
                "name": "Duncan",
                "persona_team_dir": canonical_path
            }]),
        );

        let before = std::fs::read_to_string(canonical.join("agents/managed-agents.json")).unwrap();
        reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);
        let after = std::fs::read_to_string(canonical.join("agents/managed-agents.json")).unwrap();

        assert_eq!(before, after);
    }

    #[test]
    fn team_dir_reconcile_skips_records_without_team_dir() {
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        std::fs::create_dir_all(canonical.join("agents")).unwrap();

        write_agents_json(
            &canonical,
            &serde_json::json!([{
                "name": "Test Agent",
                "agent_command": "sprout-agent"
            }]),
        );

        let before = std::fs::read_to_string(canonical.join("agents/managed-agents.json")).unwrap();
        reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);
        let after = std::fs::read_to_string(canonical.join("agents/managed-agents.json")).unwrap();

        assert_eq!(before, after);
    }

    #[test]
    fn team_dir_reconcile_is_idempotent() {
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        std::fs::create_dir_all(canonical.join("agents")).unwrap();

        let worktree_pack_path = format!(
            "{}/agents/packs/com.wpfleger.sietch-tabr",
            parent
                .path()
                .join("xyz.block.sprout.app.dev.worktree-my-branch")
                .display()
        );

        write_agents_json(
            &canonical,
            &serde_json::json!([{
                "name": "Paul",
                "persona_pack_path": worktree_pack_path
            }]),
        );

        let path = canonical.join("agents/managed-agents.json");
        reconcile_team_dirs_in_file(&path, &canonical);
        let after_first = std::fs::read_to_string(&path).unwrap();
        reconcile_team_dirs_in_file(&path, &canonical);
        let after_second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(after_first, after_second);
    }

    // ── Packs → Teams migration tests ───────────────────────────────────

    #[test]
    fn migrate_packs_to_teams_renames_directory() {
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        let packs_dir = canonical.join("agents/packs/com.example.test-pack");
        std::fs::create_dir_all(&packs_dir).unwrap();
        std::fs::write(packs_dir.join("plugin.json"), "{}").unwrap();

        // No personas or agents JSON needed for directory rename
        std::fs::create_dir_all(canonical.join("agents")).unwrap();

        // Simulate calling the migration steps directly (no AppHandle needed)
        let packs = canonical.join("agents/packs");
        let teams = canonical.join("agents/teams");
        std::fs::rename(&packs, &teams).unwrap();

        assert!(!packs.exists());
        assert!(teams.join("com.example.test-pack/plugin.json").exists());
    }

    #[test]
    fn migrate_packs_to_teams_rewrites_personas_json() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([{
                "id": "persona-1",
                "display_name": "Test",
                "source_pack": "com.example.my-pack",
                "source_pack_persona_slug": "agent-one"
            }]),
        );

        let path = dir.path().join("agents/personas.json");
        patch_json_records(&path, |obj| {
            let mut changed = false;
            if let Some(val) = obj.remove("source_pack") {
                obj.insert("source_team".to_string(), val);
                changed = true;
            }
            if let Some(val) = obj.remove("source_pack_persona_slug") {
                obj.insert("source_team_persona_slug".to_string(), val);
                changed = true;
            }
            changed
        });

        let records = read_personas_json(dir.path());
        assert_eq!(records[0]["source_team"], "com.example.my-pack");
        assert_eq!(records[0]["source_team_persona_slug"], "agent-one");
        assert!(records[0].get("source_pack").is_none());
        assert!(records[0].get("source_pack_persona_slug").is_none());
    }

    #[test]
    fn migrate_packs_to_teams_rewrites_agents_json() {
        let dir = tempfile::tempdir().unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([{
                "name": "Paul",
                "persona_pack_path": "/data/agents/packs/com.example.my-pack",
                "persona_name_in_pack": "agent-one"
            }]),
        );

        let path = dir.path().join("agents/managed-agents.json");
        patch_json_records(&path, |obj| {
            let mut changed = false;
            if let Some(val) = obj.remove("persona_pack_path") {
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
            changed
        });

        let records = read_agents_json(dir.path());
        assert_eq!(
            records[0]["persona_team_dir"],
            "/data/agents/teams/com.example.my-pack"
        );
        assert_eq!(records[0]["persona_name_in_team"], "agent-one");
        assert!(records[0].get("persona_pack_path").is_none());
        assert!(records[0].get("persona_name_in_pack").is_none());
    }

    fn write_personas_json(dir: &Path, records: &serde_json::Value) {
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(
            dir.join("agents/personas.json"),
            serde_json::to_vec_pretty(records).unwrap(),
        )
        .unwrap();
    }

    fn read_personas_json(dir: &Path) -> Vec<serde_json::Value> {
        let content = std::fs::read_to_string(dir.join("agents/personas.json")).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    #[test]
    fn rename_provider_to_runtime_migrates_field() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([{
                "id": "persona-1",
                "displayName": "Alice",
                "provider": "goose"
            }]),
        );
        rename_provider_to_runtime_in_personas(&dir.path().join("agents/personas.json"));
        let records = read_personas_json(dir.path());
        assert_eq!(records[0]["runtime"], "goose");
        assert!(records[0].get("provider").is_none());
    }

    #[test]
    fn rename_provider_to_runtime_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([{
                "id": "persona-1",
                "displayName": "Alice",
                "runtime": "goose"
            }]),
        );
        let before = std::fs::read_to_string(dir.path().join("agents/personas.json")).unwrap();
        rename_provider_to_runtime_in_personas(&dir.path().join("agents/personas.json"));
        let after = std::fs::read_to_string(dir.path().join("agents/personas.json")).unwrap();
        assert_eq!(
            before, after,
            "file should not be rewritten when already migrated"
        );
    }

    #[test]
    fn rename_provider_to_runtime_skips_record_without_either_key() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([{
                "id": "persona-1",
                "displayName": "Alice"
            }]),
        );
        let before = std::fs::read_to_string(dir.path().join("agents/personas.json")).unwrap();
        rename_provider_to_runtime_in_personas(&dir.path().join("agents/personas.json"));
        let after = std::fs::read_to_string(dir.path().join("agents/personas.json")).unwrap();
        assert_eq!(
            before, after,
            "file should not be rewritten when no provider key exists"
        );
    }

    #[test]
    fn rename_provider_to_runtime_preserves_existing_runtime_over_provider() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([{
                "id": "persona-1",
                "displayName": "Alice",
                "provider": "old-value",
                "runtime": "correct-value"
            }]),
        );
        rename_provider_to_runtime_in_personas(&dir.path().join("agents/personas.json"));
        let records = read_personas_json(dir.path());
        assert_eq!(records[0]["runtime"], "correct-value");
        // provider key should still be there since the closure returns false when runtime exists
        assert_eq!(records[0]["provider"], "old-value");
    }

    #[test]
    fn reconcile_mcp_commands_clears_stale_sprout_mcp_server() {
        let dir = tempfile::tempdir().unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([{
                "name": "Solo",
                "agent_command": "goose",
                "mcp_command": "sprout-mcp-server"
            }]),
        );
        reconcile_mcp_commands_in_file(&dir.path().join("agents/managed-agents.json"));
        let records = read_agents_json(dir.path());
        assert_eq!(records[0]["mcp_command"], "");
    }

    #[test]
    fn reconcile_mcp_commands_sets_canonical_for_sprout_agent() {
        let dir = tempfile::tempdir().unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([{
                "name": "Stilgar",
                "agent_command": "sprout-agent",
                "mcp_command": "sprout-mcp-server"
            }]),
        );
        reconcile_mcp_commands_in_file(&dir.path().join("agents/managed-agents.json"));
        let records = read_agents_json(dir.path());
        assert_eq!(records[0]["mcp_command"], "sprout-dev-mcp");
    }

    #[test]
    fn reconcile_mcp_commands_leaves_custom_value_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!([{
            "name": "Solo",
            "agent_command": "goose",
            "mcp_command": "my-custom-mcp"
        }]);
        write_agents_json(dir.path(), &json);
        let path = dir.path().join("agents/managed-agents.json");
        let before = std::fs::read_to_string(&path).unwrap();
        reconcile_mcp_commands_in_file(&path);
        assert_eq!(before, std::fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn reconcile_mcp_commands_leaves_unknown_runtime_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!([{
            "name": "Custom",
            "agent_command": "my-custom-agent",
            "mcp_command": "sprout-mcp-server"
        }]);
        write_agents_json(dir.path(), &json);
        let path = dir.path().join("agents/managed-agents.json");
        let before = std::fs::read_to_string(&path).unwrap();
        reconcile_mcp_commands_in_file(&path);
        assert_eq!(before, std::fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn reconcile_mcp_commands_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([{
                "name": "Solo",
                "agent_command": "goose",
                "mcp_command": "sprout-mcp-server"
            }]),
        );
        let path = dir.path().join("agents/managed-agents.json");
        reconcile_mcp_commands_in_file(&path);
        let after_first = std::fs::read_to_string(&path).unwrap();
        reconcile_mcp_commands_in_file(&path);
        assert_eq!(after_first, std::fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn reconcile_mcp_commands_handles_mixed_agents() {
        let dir = tempfile::tempdir().unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([
                {"name": "Stale Goose", "agent_command": "goose", "mcp_command": "sprout-mcp-server"},
                {"name": "Clean Goose", "agent_command": "goose", "mcp_command": ""},
                {"name": "Custom Agent", "agent_command": "goose", "mcp_command": "my-custom-mcp"},
                {"name": "Stale Sprout", "agent_command": "sprout-agent", "mcp_command": "sprout-mcp-server"}
            ]),
        );
        reconcile_mcp_commands_in_file(&dir.path().join("agents/managed-agents.json"));
        let records = read_agents_json(dir.path());
        assert_eq!(records[0]["mcp_command"], "");
        assert_eq!(records[1]["mcp_command"], "");
        assert_eq!(records[2]["mcp_command"], "my-custom-mcp");
        assert_eq!(records[3]["mcp_command"], "sprout-dev-mcp");
    }

    #[test]
    fn reconcile_mcp_commands_skips_record_without_agent_command() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!([{
            "name": "No Command",
            "mcp_command": "sprout-mcp-server"
        }]);
        write_agents_json(dir.path(), &json);
        let path = dir.path().join("agents/managed-agents.json");
        let before = std::fs::read_to_string(&path).unwrap();
        reconcile_mcp_commands_in_file(&path);
        assert_eq!(before, std::fs::read_to_string(&path).unwrap());
    }
}
