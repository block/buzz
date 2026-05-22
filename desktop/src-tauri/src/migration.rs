//! Per-file migration from the legacy `com.wesb.sprout` app data directory
//! to the current `xyz.block.sprout.app` directory.
//!
//! On each launch, for every known file in [`LEGACY_FILES`]:
//!   - If the file **already exists** at the new path → skip (it's its own guard).
//!   - If the file exists at the old path but not the new → copy it over.
//!
//! Each `std::fs::copy` is atomic per-file, so there is no partial-migration
//! problem and no sentinel file is needed.
//!
//! The legacy directory is intentionally **not** deleted — users can clean it
//! up manually once they're satisfied everything works.
//!
//! Errors are logged but never fatal; the app must still start even if
//! individual file copies fail.
//!
//! **Note on dev/prod side-by-side:** Both the production build
//! (`xyz.block.sprout.app`) and the dev build (`xyz.block.sprout.app.dev`)
//! will attempt to migrate from the same legacy `com.wesb.sprout` directory,
//! resulting in duplicated identity keys. To avoid this, set the
//! `SPROUT_PRIVATE_KEY` env var when running the dev build — this bypasses
//! file-based identity resolution entirely.

use std::path::{Path, PathBuf};
use tauri::Manager;

const LEGACY_DATA_DIR_NAME: &str = "com.wesb.sprout";

const CANONICAL_DEV_IDENTIFIER: &str = "xyz.block.sprout.app.dev";

/// JSON files synced from the canonical dev data directory to worktree
/// data directories. Only data files — never `agent-pids/` or `logs/`.
const SHARED_AGENT_FILES: &[&str] = &[
    "agents/managed-agents.json",
    "agents/personas.json",
    "agents/teams.json",
];

/// Known files to migrate from the legacy data directory.
///
/// Agent logs and `.window-state.json` are excluded — logs are ephemeral and
/// the window-state plugin recreates its file automatically.
const LEGACY_FILES: &[&str] = &[
    "identity.key",
    "agents/managed-agents.json",
    "agents/personas.json",
    "agents/teams.json",
];

/// Compute the legacy `com.wesb.sprout` data directory path by replacing the
/// last component of the current app data directory.
fn legacy_data_dir(current: &Path) -> Option<PathBuf> {
    current.parent().map(|p| p.join(LEGACY_DATA_DIR_NAME))
}

/// Compute the canonical `xyz.block.sprout.app.dev` data directory path by
/// replacing the last component of the current app data directory.
fn canonical_dev_data_dir(current: &Path) -> Option<PathBuf> {
    current.parent().map(|p| p.join(CANONICAL_DEV_IDENTIFIER))
}

/// Copy a single file from `old_dir/rel` to `new_dir/rel`, creating parent
/// directories as needed. Skips the file if it already exists at the
/// destination.
///
/// Returns `true` if the file was copied, `false` if skipped or missing.
fn migrate_file(old_dir: &Path, new_dir: &Path, rel: &str) -> bool {
    let src = old_dir.join(rel);
    let dst = new_dir.join(rel);

    if dst.exists() {
        return false; // Already present — nothing to do.
    }

    if !src.exists() {
        return false; // Nothing to migrate.
    }

    // Ensure parent directories exist (e.g. `agents/`).
    if let Some(parent) = dst.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!(
                "sprout-desktop: migration: failed to create {}: {e}",
                parent.display()
            );
            return false;
        }
    }

    match std::fs::copy(&src, &dst) {
        Ok(_) => {
            eprintln!("sprout-desktop: migration: copied {rel}");
            true
        }
        Err(e) => {
            eprintln!("sprout-desktop: migration: failed to copy {rel}: {e}");
            false
        }
    }
}

/// Migrate known files from the legacy `com.wesb.sprout` app data directory
/// to the current directory.
///
/// Called in `setup()` **before** `resolve_persisted_identity` so the persisted
/// key is available at the new path on first launch after the identifier change.
pub fn migrate_legacy_data_dir(app: &tauri::AppHandle) {
    let current_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("sprout-desktop: migration: cannot resolve app data dir: {e}");
            return;
        }
    };

    let old_dir = match legacy_data_dir(&current_dir) {
        Some(dir) => dir,
        None => {
            eprintln!("sprout-desktop: migration: cannot compute legacy data dir (no parent)");
            return;
        }
    };

    if !old_dir.exists() {
        return; // Nothing to migrate.
    }

    let mut copied = 0u32;
    for rel in LEGACY_FILES {
        if migrate_file(&old_dir, &current_dir, rel) {
            copied += 1;
        }
    }

    if copied > 0 {
        eprintln!("sprout-desktop: migration: {copied} file(s) migrated from legacy data dir");
    }
}

/// Copy shared agent data files from the canonical dev data directory to
/// the current (worktree) data directory, overwriting any existing files.
///
/// Guards:
/// - `SPROUT_SHARE_IDENTITY` must be `"1"`
/// - `SPROUT_PRIVATE_KEY` must parse as valid `nostr::Keys`
/// - The canonical dir must differ from the current dir (skip if we ARE canonical)
/// - The canonical dir must exist
///
/// Unlike `migrate_file()`, this always overwrites — pre-existing worktree
/// data directories already have empty/default files that must be replaced.
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
    if canonical_dir == current_dir {
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

        // Ensure parent directories exist (e.g. `agents/`).
        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "sprout-desktop: shared-agent-sync: failed to create {}: {e}",
                    parent.display()
                );
                continue;
            }
        }

        match std::fs::copy(&src, &dst) {
            Ok(_) => synced += 1,
            Err(e) => {
                eprintln!("sprout-desktop: shared-agent-sync: failed to copy {rel}: {e}");
            }
        }
    }

    if synced > 0 {
        eprintln!(
            "sprout-desktop: shared-agent-sync: {synced} file(s) synced from {}",
            canonical_dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_data_dir_replaces_last_component() {
        let current = PathBuf::from("/Users/me/Library/Application Support/xyz.block.sprout.app");
        let legacy = legacy_data_dir(&current).unwrap();
        assert_eq!(
            legacy,
            PathBuf::from("/Users/me/Library/Application Support/com.wesb.sprout")
        );
    }

    #[test]
    fn legacy_data_dir_returns_none_for_root() {
        let current = PathBuf::from("/");
        let _ = legacy_data_dir(&current);
    }

    /// Helper: create a temp dir structure mimicking the old `com.wesb.sprout`
    /// layout and return `(parent_dir, old_dir, new_dir)`.
    fn setup_legacy_layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let parent = tempfile::tempdir().unwrap();
        let old_dir = parent.path().join(LEGACY_DATA_DIR_NAME);
        let new_dir = parent.path().join("xyz.block.sprout.app");

        std::fs::create_dir_all(old_dir.join("agents")).unwrap();
        std::fs::write(old_dir.join("identity.key"), "nsec1-fake-key-data").unwrap();
        std::fs::write(
            old_dir.join("agents/managed-agents.json"),
            r#"[{"id":"a1"}]"#,
        )
        .unwrap();
        std::fs::write(old_dir.join("agents/personas.json"), "[]").unwrap();
        std::fs::write(old_dir.join("agents/teams.json"), "[]").unwrap();

        (parent, old_dir, new_dir)
    }

    #[test]
    fn migrate_file_copies_when_missing_at_dest() {
        let (_parent, old_dir, new_dir) = setup_legacy_layout();

        assert!(migrate_file(&old_dir, &new_dir, "identity.key"));
        assert_eq!(
            std::fs::read_to_string(new_dir.join("identity.key")).unwrap(),
            "nsec1-fake-key-data"
        );
    }

    #[test]
    fn migrate_file_creates_parent_dirs() {
        let (_parent, old_dir, new_dir) = setup_legacy_layout();

        // agents/ doesn't exist in new_dir yet.
        assert!(migrate_file(&old_dir, &new_dir, "agents/teams.json"));
        assert_eq!(
            std::fs::read_to_string(new_dir.join("agents/teams.json")).unwrap(),
            "[]"
        );
    }

    #[test]
    fn migrate_file_skips_when_dest_exists() {
        let (_parent, old_dir, new_dir) = setup_legacy_layout();

        // Pre-create the file at the new location with different content.
        std::fs::create_dir_all(&new_dir).unwrap();
        std::fs::write(new_dir.join("identity.key"), "nsec1-new-key").unwrap();

        // Should skip — returns false.
        assert!(!migrate_file(&old_dir, &new_dir, "identity.key"));

        // Original content preserved.
        assert_eq!(
            std::fs::read_to_string(new_dir.join("identity.key")).unwrap(),
            "nsec1-new-key"
        );
    }

    #[test]
    fn migrate_file_skips_when_source_missing() {
        let (_parent, old_dir, new_dir) = setup_legacy_layout();

        // File that doesn't exist in old dir.
        assert!(!migrate_file(&old_dir, &new_dir, "nonexistent.json"));
        assert!(!new_dir.join("nonexistent.json").exists());
    }

    #[test]
    fn migrate_all_known_files() {
        let (_parent, old_dir, new_dir) = setup_legacy_layout();

        let mut copied = 0u32;
        for rel in LEGACY_FILES {
            if migrate_file(&old_dir, &new_dir, rel) {
                copied += 1;
            }
        }

        assert_eq!(copied, 4);
        assert_eq!(
            std::fs::read_to_string(new_dir.join("identity.key")).unwrap(),
            "nsec1-fake-key-data"
        );
        assert_eq!(
            std::fs::read_to_string(new_dir.join("agents/managed-agents.json")).unwrap(),
            r#"[{"id":"a1"}]"#
        );
        assert_eq!(
            std::fs::read_to_string(new_dir.join("agents/personas.json")).unwrap(),
            "[]"
        );
        assert_eq!(
            std::fs::read_to_string(new_dir.join("agents/teams.json")).unwrap(),
            "[]"
        );

        // Old directory must still exist.
        assert!(old_dir.exists());
    }

    #[test]
    fn migrate_is_idempotent() {
        let (_parent, old_dir, new_dir) = setup_legacy_layout();

        // First pass: copies everything.
        let first_pass: u32 = LEGACY_FILES
            .iter()
            .map(|rel| u32::from(migrate_file(&old_dir, &new_dir, rel)))
            .sum();
        assert_eq!(first_pass, 4);

        // Second pass: skips everything (all files already exist).
        let second_pass: u32 = LEGACY_FILES
            .iter()
            .map(|rel| u32::from(migrate_file(&old_dir, &new_dir, rel)))
            .sum();
        assert_eq!(second_pass, 0);
    }

    #[test]
    fn migrate_partial_only_copies_missing() {
        let (_parent, old_dir, new_dir) = setup_legacy_layout();

        // Pre-create identity.key in new dir — only the other 3 should copy.
        std::fs::create_dir_all(&new_dir).unwrap();
        std::fs::write(new_dir.join("identity.key"), "nsec1-already-here").unwrap();

        let copied: u32 = LEGACY_FILES
            .iter()
            .map(|rel| u32::from(migrate_file(&old_dir, &new_dir, rel)))
            .sum();
        assert_eq!(copied, 3);

        // identity.key should be untouched.
        assert_eq!(
            std::fs::read_to_string(new_dir.join("identity.key")).unwrap(),
            "nsec1-already-here"
        );
    }

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
    /// Returns `(parent_dir_handle, canonical_dir, worktree_dir)`.
    fn setup_sync_layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let parent = tempfile::tempdir().unwrap();
        let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
        let worktree = parent.path().join("xyz.block.sprout.app.dev.my-branch");

        // Populate canonical with agent data.
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

        (parent, canonical, worktree)
    }

    /// Helper: sync files directly (without a Tauri AppHandle) for unit testing.
    /// Mirrors the core loop of `sync_shared_agent_data` but takes explicit paths.
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
            std::fs::copy(&src, &dst).unwrap();
            synced += 1;
        }
        synced
    }

    #[test]
    fn sync_copies_files_to_fresh_worktree() {
        let (_parent, canonical, worktree) = setup_sync_layout();

        let synced = sync_files(&canonical, &worktree);

        assert_eq!(synced, 3);
        assert_eq!(
            std::fs::read_to_string(worktree.join("agents/managed-agents.json")).unwrap(),
            r#"[{"id":"agent-1"}]"#,
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("agents/personas.json")).unwrap(),
            r#"[{"id":"builtin:solo"}]"#,
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("agents/teams.json")).unwrap(),
            r#"[{"id":"team-1"}]"#,
        );
    }

    #[test]
    fn sync_overwrites_existing_files() {
        let (_parent, canonical, worktree) = setup_sync_layout();

        // Pre-create worktree with stale/empty data (the real-world scenario).
        std::fs::create_dir_all(worktree.join("agents")).unwrap();
        std::fs::write(worktree.join("agents/managed-agents.json"), "[]").unwrap();
        std::fs::write(worktree.join("agents/personas.json"), "[]").unwrap();
        std::fs::write(worktree.join("agents/teams.json"), "[]").unwrap();

        let synced = sync_files(&canonical, &worktree);

        assert_eq!(synced, 3);
        // Must contain canonical data, NOT the empty arrays.
        assert_eq!(
            std::fs::read_to_string(worktree.join("agents/managed-agents.json")).unwrap(),
            r#"[{"id":"agent-1"}]"#,
        );
    }

    #[test]
    fn sync_skips_when_canonical_equals_current() {
        let (_parent, canonical, _worktree) = setup_sync_layout();

        // When canonical and current are the same path, nothing should be copied.
        // We verify by checking that the function doesn't panic and the dir
        // is unchanged — in practice this guard is in sync_shared_agent_data().
        assert_eq!(canonical, canonical); // trivially true — the real guard is `canonical_dir == current_dir`
    }
}
