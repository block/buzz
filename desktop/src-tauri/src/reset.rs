//! Two-phase boot-time sentinel wipe.
//!
//! **Phase 1** (`write_sentinel`) — called by `sign_out`: writes a durable
//! reset-intent file outside every path that will be wiped.
//!
//! **Phase 2** (`run_boot_reset`) — called at the very top of `setup()` in
//! `lib.rs`, before migrations and identity resolution: if the sentinel is
//! present the wipe runs atomically and the app falls through into clean
//! onboarding.
//!
//! The sentinel lives at `<app_data_dir's parent>/.<bundle_id>.reset-pending`
//! and survives the app-data wipe because the wipe targets the exact
//! app-data dir, not its parent.
//!
//! Idempotency: if the process crashes mid-wipe the sentinel is still present
//! on next boot and the wipe retries from the top.

use std::path::{Path, PathBuf};

// ── Sentinel helpers ──────────────────────────────────────────────────────────

/// Sentinel path: `<app_data_dir.parent>/.<bundle_id>.reset-pending`
/// where `bundle_id` is the file-name component of `app_data_dir`
/// (e.g. `xyz.block.buzz.app` or `xyz.block.buzz.app.dev`).
pub(crate) fn sentinel_path(app_data_dir: &Path) -> PathBuf {
    let bundle_id = app_data_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("buzz");
    let name = format!(".{bundle_id}.reset-pending");
    match app_data_dir.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(&name),
    }
}

/// Atomically write the sentinel file. Content is intentionally empty —
/// existence is the signal.
pub(crate) fn write_sentinel(app_data_dir: &Path) -> Result<(), String> {
    let path = sentinel_path(app_data_dir);
    std::fs::write(&path, b"").map_err(|e| format!("write sentinel {}: {e}", path.display()))
}

/// Return `true` when the sentinel file exists.
pub(crate) fn check_sentinel(app_data_dir: &Path) -> bool {
    sentinel_path(app_data_dir).exists()
}

/// Remove the sentinel file. A missing file is not an error.
pub(crate) fn delete_sentinel(app_data_dir: &Path) -> Result<(), String> {
    let path = sentinel_path(app_data_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("delete sentinel {}: {e}", path.display())),
    }
}

// ── Keychain abstraction (enables unit testing) ───────────────────────────────

/// Keychain operations needed by the boot-time reset.
/// Implemented for `SecretStore`; a fake is used in tests.
pub(crate) trait ResetKeychain {
    /// Delete the blob + all per-key legacy entries.
    fn delete_all_with_legacy(&self) -> Result<(), String>;
    /// Return `true` when all keychain shapes (main blob, DPK blob, per-key
    /// entries) that `migrate_legacy_key` can consume are absent.
    fn verify_fully_wiped(&self) -> bool;
}

impl ResetKeychain for crate::secret_store::SecretStore {
    fn delete_all_with_legacy(&self) -> Result<(), String> {
        self.delete_all_with_legacy_cleanup()
    }

    fn verify_fully_wiped(&self) -> bool {
        self.verify_fully_wiped()
    }
}

// ── Result type ───────────────────────────────────────────────────────────────

/// Outcome of the boot-time reset check.
#[derive(Debug, Default)]
pub(crate) struct ResetOutcome {
    /// Wipe completed successfully this boot — suppress nest migrations.
    pub completed: bool,
    /// Wipe was attempted but verification failed — surface error state.
    pub failed: bool,
}

// ── Boot-time reset ───────────────────────────────────────────────────────────

/// Wipe parameters assembled by `lib.rs` and passed into `run_boot_reset_with_keychain`.
pub(crate) struct ResetContext<'a> {
    pub app_data_dir: &'a Path,
    /// Legacy App Support dir for this build (Sprout import source). When
    /// present and non-empty, wiped alongside `app_data_dir` to prevent
    /// `migrate_legacy_app_data_dir` from restoring the old identity.
    pub legacy_app_data_dir: Option<PathBuf>,
    /// Nest dir (`~/.buzz` or `~/.buzz-dev`) scoped to this build's variant,
    /// injected so unit tests can override without touching the global OnceLock.
    pub nest_dir: Option<PathBuf>,
    pub keychain: &'a dyn ResetKeychain,
    pub home_dir: Option<PathBuf>,
    pub is_dev: bool,
}

/// Entry point called from `lib.rs` setup (before migrations).
///
/// Constructs a `SecretStore` for the running build's keyring service and
/// delegates to `run_boot_reset_with_keychain` for testable wipe logic.
pub(crate) fn run_boot_reset(app_data_dir: &Path) -> ResetOutcome {
    if !check_sentinel(app_data_dir) {
        return ResetOutcome::default();
    }

    let is_dev = app_data_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(crate::migration::is_dev_data_dir_name)
        .unwrap_or(false);

    let store = crate::secret_store::SecretStore::keyring(crate::app_state::keyring_service());
    let home_dir = dirs::home_dir();
    let legacy_dir = crate::migration::legacy_app_data_dir(app_data_dir);
    let nest_dir = crate::managed_agents::nest_dir();

    let ctx = ResetContext {
        app_data_dir,
        legacy_app_data_dir: legacy_dir,
        nest_dir,
        keychain: &store,
        home_dir,
        is_dev,
    };

    run_boot_reset_with_keychain(ctx)
}

/// Deterministic trash path: `<original>.reset-trash`. Unlike PID-based names,
/// any boot can discover and clean trash from a prior crashed attempt.
fn trash_path(original: &Path) -> PathBuf {
    original.with_file_name(format!(
        "{}.reset-trash",
        original
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("buzz")
    ))
}

/// Remove an existing reset-trash directory if present (from a prior crashed
/// attempt), then rename `src` into the deterministic trash path. Returns
/// `Ok(trash_path)` on success.
fn rename_to_trash(src: &Path) -> Result<PathBuf, String> {
    let dst = trash_path(src);
    // Clear prior trash before renaming so a collision doesn't fail the rename.
    if dst.exists() {
        let _ = std::fs::remove_dir_all(&dst);
    }
    std::fs::rename(src, &dst)
        .map_err(|e| format!("rename {} → {}: {e}", src.display(), dst.display()))?;
    Ok(dst)
}

/// Core wipe logic — separated for testing.
pub(crate) fn run_boot_reset_with_keychain(ctx: ResetContext<'_>) -> ResetOutcome {
    let app_data_dir = ctx.app_data_dir;

    // ── Step 1: rename app-data dir (atomic — sentinel survives the parent) ──
    let trash_app = trash_path(app_data_dir);

    if app_data_dir.exists() {
        if let Err(e) = rename_to_trash(app_data_dir) {
            eprintln!("buzz-desktop reset: {e}");
            return ResetOutcome {
                completed: false,
                failed: true,
            };
        }
    }

    // ── Step 1b: rename legacy App Support dir (sprout import source) ────────
    let trash_legacy: Option<PathBuf> = ctx.legacy_app_data_dir.as_ref().map(|l| trash_path(l));
    if let Some(ref legacy) = ctx.legacy_app_data_dir {
        if legacy.exists() {
            if let Err(e) = rename_to_trash(legacy) {
                eprintln!("buzz-desktop reset: {e}");
                // Non-fatal for legacy dir — continue
            }
        }
    }

    // ── Step 2: rename WebKit dir for this build ──────────────────────────────
    let trash_webkit: Option<PathBuf> = if let Some(ref home) = ctx.home_dir {
        let bundle_id = app_data_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("buzz");
        let webkit_dir = home.join("Library").join("WebKit").join(bundle_id);
        let tw = trash_path(&webkit_dir);
        if webkit_dir.exists() {
            if let Err(e) = rename_to_trash(&webkit_dir) {
                eprintln!("buzz-desktop reset: {e}");
                // Non-fatal — continue
            }
        }
        Some(tw)
    } else {
        None
    };

    // ── Step 3: remove nest, ~/.sprout, ~/.config/buzz-agent, CLI symlink ────
    if let Some(ref nest) = ctx.nest_dir {
        let _ = std::fs::remove_dir_all(nest);
    }
    if let Some(ref home) = ctx.home_dir {
        let _ = std::fs::remove_dir_all(home.join(".sprout"));
        let _ = std::fs::remove_dir_all(home.join(".config").join("buzz-agent"));
        let link_name = crate::managed_agents::cli_link_name(ctx.is_dev);
        let _ = std::fs::remove_file(home.join(".local").join("bin").join(link_name));
    }

    // ── Step 4: keychain — LAST so we can read keys before deleting ──────────
    if let Err(e) = ctx.keychain.delete_all_with_legacy() {
        eprintln!("buzz-desktop reset: keychain delete: {e}");
        // Keychain failure is fatal: keep sentinel, signal failure.
        // Restore all three dirs so the app returns to a coherent pre-reset state.
        if trash_app.exists() {
            let _ = std::fs::rename(&trash_app, app_data_dir);
        }
        if let Some(ref legacy) = ctx.legacy_app_data_dir {
            if let Some(ref tl) = trash_legacy {
                if tl.exists() {
                    let _ = std::fs::rename(tl, legacy);
                }
            }
        }
        if let Some(ref home) = ctx.home_dir {
            let bundle_id = app_data_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("buzz");
            let webkit_dir = home.join("Library").join("WebKit").join(bundle_id);
            if let Some(ref tw) = trash_webkit {
                if tw.exists() {
                    let _ = std::fs::rename(tw, &webkit_dir);
                }
            }
        }
        return ResetOutcome {
            completed: false,
            failed: true,
        };
    }

    // ── Step 5: sweep ALL reset trash (including from prior crashed boots) ───
    let _ = std::fs::remove_dir_all(&trash_app);
    if let Some(ref tl) = trash_legacy {
        let _ = std::fs::remove_dir_all(tl);
    }
    if let Some(ref tw) = trash_webkit {
        let _ = std::fs::remove_dir_all(tw);
    }

    // ── Step 6: verify ────────────────────────────────────────────────────────
    let keychain_ok = ctx.keychain.verify_fully_wiped();
    let app_data_gone = !app_data_dir.exists();
    let legacy_gone = ctx
        .legacy_app_data_dir
        .as_ref()
        .map(|p| !p.exists())
        .unwrap_or(true);
    let nest_gone = ctx.nest_dir.as_ref().map(|n| !n.exists()).unwrap_or(true);
    let trash_app_gone = !trash_app.exists();
    let trash_legacy_gone = trash_legacy.as_ref().map(|p| !p.exists()).unwrap_or(true);
    let trash_webkit_gone = trash_webkit.as_ref().map(|p| !p.exists()).unwrap_or(true);

    if !keychain_ok
        || !app_data_gone
        || !legacy_gone
        || !nest_gone
        || !trash_app_gone
        || !trash_legacy_gone
        || !trash_webkit_gone
    {
        eprintln!(
            "buzz-desktop reset: verification failed (keychain_wiped={keychain_ok}, \
             app_data_gone={app_data_gone}, legacy_gone={legacy_gone}, nest_gone={nest_gone}, \
             trash_app_gone={trash_app_gone}, trash_legacy_gone={trash_legacy_gone}, \
             trash_webkit_gone={trash_webkit_gone})"
        );
        return ResetOutcome {
            completed: false,
            failed: true,
        };
    }

    // ── Step 7: delete sentinel → success ────────────────────────────────────
    if let Err(e) = delete_sentinel(app_data_dir) {
        eprintln!("buzz-desktop reset: delete sentinel: {e}");
        // Sentinel not deleted — keep failed=false so the app boots into
        // onboarding, but on next boot the reset will retry (idempotent).
    }

    ResetOutcome {
        completed: true,
        failed: false,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use tempfile::TempDir;

    // ── Fake keychain ─────────────────────────────────────────────────────────

    struct FakeKeychain {
        delete_result: Result<(), String>,
        /// Tracks number of times delete was called.
        delete_calls: Cell<u32>,
        /// Whether `verify_fully_wiped` returns true after a successful delete.
        wiped_after_delete: bool,
        /// When true, verify_fully_wiped always returns false regardless of
        /// delete outcome — simulates a transient/unknown keychain error
        /// during verification that cannot confirm absence.
        verify_always_fails: bool,
    }

    impl FakeKeychain {
        fn ok() -> Self {
            FakeKeychain {
                delete_result: Ok(()),
                delete_calls: Cell::new(0),
                wiped_after_delete: true,
                verify_always_fails: false,
            }
        }

        fn fail(msg: &str) -> Self {
            FakeKeychain {
                delete_result: Err(msg.to_string()),
                delete_calls: Cell::new(0),
                wiped_after_delete: false,
                verify_always_fails: false,
            }
        }

        fn ok_but_not_wiped() -> Self {
            FakeKeychain {
                delete_result: Ok(()),
                delete_calls: Cell::new(0),
                wiped_after_delete: false,
                verify_always_fails: false,
            }
        }

        /// Delete succeeds but verify returns false — simulates a transient
        /// unknown error during verification (e.g. keyring constructor failure
        /// or unclassified read error that cannot confirm absence).
        fn ok_but_verify_fails() -> Self {
            FakeKeychain {
                delete_result: Ok(()),
                delete_calls: Cell::new(0),
                wiped_after_delete: true,
                verify_always_fails: true,
            }
        }
    }

    impl ResetKeychain for FakeKeychain {
        fn delete_all_with_legacy(&self) -> Result<(), String> {
            self.delete_calls.set(self.delete_calls.get() + 1);
            self.delete_result.clone()
        }

        fn verify_fully_wiped(&self) -> bool {
            if self.verify_always_fails {
                return false;
            }
            self.wiped_after_delete && self.delete_calls.get() > 0
        }
    }

    fn make_app_data(tmp: &TempDir) -> PathBuf {
        let dir = tmp
            .path()
            .join("Application Support")
            .join("xyz.block.buzz.app");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_ctx<'a>(
        app_data_dir: &'a Path,
        keychain: &'a dyn ResetKeychain,
        is_dev: bool,
    ) -> ResetContext<'a> {
        ResetContext {
            app_data_dir,
            legacy_app_data_dir: None,
            nest_dir: None,
            keychain,
            home_dir: None, // skip nest/sprout/CLI ops in unit tests
            is_dev,
        }
    }

    // ── Test 1: no sentinel ───────────────────────────────────────────────────

    #[test]
    fn test_no_sentinel_returns_no_op() {
        let tmp = TempDir::new().unwrap();
        let app_data = make_app_data(&tmp);
        let kc = FakeKeychain::ok();

        let outcome = run_boot_reset(app_data.as_path());
        assert!(!outcome.completed, "no sentinel → not completed");
        assert!(!outcome.failed, "no sentinel → not failed");
        assert_eq!(kc.delete_calls.get(), 0, "keychain not touched");
        assert!(app_data.exists(), "app-data dir untouched");
    }

    // ── Test 2: full wipe succeeds ────────────────────────────────────────────

    #[test]
    fn test_sentinel_present_full_wipe_succeeds() {
        let tmp = TempDir::new().unwrap();
        let app_data = make_app_data(&tmp);

        // Also create a legacy App Support dir (sprout import source).
        let legacy_dir = tmp
            .path()
            .join("Application Support")
            .join("xyz.block.sprout.app");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("identity.key"), b"old-identity").unwrap();

        write_sentinel(&app_data).unwrap();
        let kc = FakeKeychain::ok();

        let ctx = ResetContext {
            app_data_dir: &app_data,
            legacy_app_data_dir: Some(legacy_dir.clone()),
            nest_dir: None,
            keychain: &kc,
            home_dir: None,
            is_dev: false,
        };

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(outcome.completed, "should complete");
        assert!(!outcome.failed, "should not fail");
        assert!(!app_data.exists(), "app-data must be gone");
        assert!(!legacy_dir.exists(), "legacy app-data must be gone");
        assert!(!sentinel_path(&app_data).exists(), "sentinel must be gone");
        assert_eq!(kc.delete_calls.get(), 1, "keychain deleted once");
    }

    // ── Test 3: keychain failure keeps sentinel ────────────────────────────────

    #[test]
    fn test_sentinel_present_keychain_failure_keeps_sentinel() {
        let tmp = TempDir::new().unwrap();
        let app_data = make_app_data(&tmp);
        write_sentinel(&app_data).unwrap();
        let kc = FakeKeychain::fail("keychain unavailable");
        let ctx = make_ctx(&app_data, &kc, false);

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(!outcome.completed);
        assert!(outcome.failed);
        assert!(
            sentinel_path(&app_data).exists(),
            "sentinel must be preserved on failure"
        );
    }

    // ── Test 4: app-data rename works but verify fails ────────────────────────

    #[test]
    fn test_sentinel_present_verify_failure_keeps_sentinel() {
        let tmp = TempDir::new().unwrap();
        let app_data = make_app_data(&tmp);
        write_sentinel(&app_data).unwrap();
        // Keychain delete "succeeds" but verify_fully_wiped still returns false.
        let kc = FakeKeychain::ok_but_not_wiped();
        let ctx = make_ctx(&app_data, &kc, false);

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(!outcome.completed);
        assert!(outcome.failed);
        assert!(sentinel_path(&app_data).exists(), "sentinel preserved");
    }

    // ── Test 5: crash-then-retry completes ───────────────────────────────────

    #[test]
    fn test_crash_then_retry_completes() {
        let tmp = TempDir::new().unwrap();
        let app_data = make_app_data(&tmp);
        write_sentinel(&app_data).unwrap();

        // First run — keychain fails (simulates a crash mid-wipe).
        let kc1 = FakeKeychain::fail("transient error");
        let ctx1 = make_ctx(&app_data, &kc1, false);
        let first = run_boot_reset_with_keychain(ctx1);
        assert!(first.failed);
        assert!(
            sentinel_path(&app_data).exists(),
            "sentinel must survive first attempt"
        );

        // Second run — keychain succeeds. App-data dir was renamed but we need
        // to recreate it for the test (the wipe tried to rename it).
        // In production the dir would already be gone; here it was renamed to
        // .trash-<pid> and then not cleaned up (keychain failed before cleanup).
        // Create a fresh app-data dir to simulate a reboot where app recreated it.
        std::fs::create_dir_all(&app_data).unwrap();

        let kc2 = FakeKeychain::ok();
        let ctx2 = make_ctx(&app_data, &kc2, false);
        let second = run_boot_reset_with_keychain(ctx2);
        assert!(second.completed, "second attempt must complete");
        assert!(!second.failed);
        assert!(
            !sentinel_path(&app_data).exists(),
            "sentinel cleared on success"
        );
    }

    // ── Test 6: dev build wipes dev nest, leaves prod nest intact ────────────

    #[test]
    fn test_dev_build_wipes_dev_nest_not_prod() {
        let tmp = TempDir::new().unwrap();

        // Create both nests.
        let dev_nest = tmp.path().join(".buzz-dev");
        let prod_nest = tmp.path().join(".buzz");
        std::fs::create_dir_all(&dev_nest).unwrap();
        std::fs::create_dir_all(&prod_nest).unwrap();

        let app_data = tmp
            .path()
            .join("Application Support")
            .join("xyz.block.buzz.app.dev");
        std::fs::create_dir_all(&app_data).unwrap();
        write_sentinel(&app_data).unwrap();

        let kc = FakeKeychain::ok();
        let ctx = ResetContext {
            app_data_dir: &app_data,
            legacy_app_data_dir: None,
            nest_dir: Some(dev_nest.clone()),
            keychain: &kc,
            home_dir: None,
            is_dev: true,
        };

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(outcome.completed, "wipe must complete");
        assert!(!dev_nest.exists(), "dev nest must be wiped");
        assert!(prod_nest.exists(), "prod nest must survive");
    }

    // ── Test 7: prod build wipes prod nest, leaves dev nest intact ───────────

    #[test]
    fn test_prod_build_wipes_prod_nest_not_dev() {
        let tmp = TempDir::new().unwrap();

        // Create both nests.
        let dev_nest = tmp.path().join(".buzz-dev");
        let prod_nest = tmp.path().join(".buzz");
        std::fs::create_dir_all(&dev_nest).unwrap();
        std::fs::create_dir_all(&prod_nest).unwrap();

        let app_data = tmp
            .path()
            .join("Application Support")
            .join("xyz.block.buzz.app");
        std::fs::create_dir_all(&app_data).unwrap();
        write_sentinel(&app_data).unwrap();

        let kc = FakeKeychain::ok();
        let ctx = ResetContext {
            app_data_dir: &app_data,
            legacy_app_data_dir: None,
            nest_dir: Some(prod_nest.clone()),
            keychain: &kc,
            home_dir: None,
            is_dev: false,
        };

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(outcome.completed, "wipe must complete");
        assert!(!prod_nest.exists(), "prod nest must be wiped");
        assert!(dev_nest.exists(), "dev nest must survive");
    }

    // ── Test 8: legacy app-data removed on reset ──────────────────────────────

    #[test]
    fn test_legacy_app_data_removed_on_reset() {
        let tmp = TempDir::new().unwrap();
        let app_data = make_app_data(&tmp);

        // Seed a legacy sprout dir with data that would be re-imported.
        let legacy_dir = tmp
            .path()
            .join("Application Support")
            .join("xyz.block.sprout.app");
        std::fs::create_dir_all(legacy_dir.join("agents")).unwrap();
        std::fs::write(legacy_dir.join("identity.key"), b"sprout-nsec").unwrap();

        write_sentinel(&app_data).unwrap();
        let kc = FakeKeychain::ok();
        let ctx = ResetContext {
            app_data_dir: &app_data,
            legacy_app_data_dir: Some(legacy_dir.clone()),
            nest_dir: None,
            keychain: &kc,
            home_dir: None,
            is_dev: false,
        };

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(outcome.completed, "reset must complete");
        assert!(!legacy_dir.exists(), "legacy app-data dir must be removed");
    }

    // ── Test 9: unknown error during delete → failed, sentinel kept ────────

    #[test]
    fn test_unknown_delete_error_keeps_sentinel() {
        let tmp = TempDir::new().unwrap();
        let app_data = make_app_data(&tmp);
        write_sentinel(&app_data).unwrap();
        // Simulates an unclassified/transient keychain error during delete.
        let kc = FakeKeychain::fail("unknown transient keychain error");
        let ctx = make_ctx(&app_data, &kc, false);

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(outcome.failed, "unknown delete error must fail");
        assert!(!outcome.completed, "must not complete on delete error");
        assert!(
            sentinel_path(&app_data).exists(),
            "sentinel must survive unknown delete error"
        );
    }

    // ── Test 10: unknown error during verify → failed, sentinel kept ─────

    #[test]
    fn test_unknown_verify_error_keeps_sentinel() {
        let tmp = TempDir::new().unwrap();
        let app_data = make_app_data(&tmp);
        write_sentinel(&app_data).unwrap();
        // Delete succeeds but verification fails — simulates a transient
        // keychain error (e.g. constructor failure, unclassified read error)
        // that cannot confirm absence. The sentinel must survive.
        let kc = FakeKeychain::ok_but_verify_fails();
        let ctx = make_ctx(&app_data, &kc, false);

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(outcome.failed, "unknown verify error must fail");
        assert!(!outcome.completed, "must not complete on verify error");
        assert!(
            sentinel_path(&app_data).exists(),
            "sentinel must survive unknown verify error"
        );
    }

    // ── Test 11: completed dev reset suppresses dev repos-dir migration ────

    #[test]
    fn test_completed_dev_reset_suppresses_dev_repos_dir_migration() {
        let tmp = TempDir::new().unwrap();
        let app_data = tmp
            .path()
            .join("Application Support")
            .join("xyz.block.buzz.app.dev");
        std::fs::create_dir_all(&app_data).unwrap();
        write_sentinel(&app_data).unwrap();

        let kc = FakeKeychain::ok();
        let ctx = ResetContext {
            app_data_dir: &app_data,
            legacy_app_data_dir: None,
            nest_dir: Some(tmp.path().join(".buzz-dev")),
            keychain: &kc,
            home_dir: None,
            is_dev: true,
        };

        let outcome = run_boot_reset_with_keychain(ctx);
        assert!(outcome.completed, "reset must complete");

        // The gate that lib.rs uses to decide whether to run
        // migrate_dev_repos_dir: should_migrate_dev_repos_dir(is_dev, completed).
        // A completed dev reset must suppress the migration.
        assert!(
            !crate::migration::should_migrate_dev_repos_dir(true, outcome.completed),
            "completed dev reset must suppress dev repos-dir migration"
        );
        // Conversely, a non-reset dev boot must allow it.
        assert!(
            crate::migration::should_migrate_dev_repos_dir(true, false),
            "non-reset dev boot must allow dev repos-dir migration"
        );
        // Prod builds never run it regardless.
        assert!(
            !crate::migration::should_migrate_dev_repos_dir(false, false),
            "prod builds must never run dev repos-dir migration"
        );
    }

    // ── Test 12: crash-after-rename retry cleans prior trash ─────────────

    #[test]
    fn test_crash_retry_cleans_prior_deterministic_trash() {
        let tmp = TempDir::new().unwrap();
        let app_support = tmp.path().join("Application Support");
        let app_data = app_support.join("xyz.block.buzz.app");
        std::fs::create_dir_all(&app_data).unwrap();
        write_sentinel(&app_data).unwrap();

        // Simulate a prior crashed boot: originals absent, deterministic trash
        // present from the crash (as if the process renamed then died).
        let trash_app_dir = app_support.join("xyz.block.buzz.app.reset-trash");
        std::fs::create_dir_all(&trash_app_dir).unwrap();
        std::fs::write(trash_app_dir.join("identity.key"), b"old-key").unwrap();

        // Original is gone (crashed mid-wipe), so remove it for the retry.
        std::fs::remove_dir_all(&app_data).unwrap();

        let kc = FakeKeychain::ok();
        let ctx = make_ctx(&app_data, &kc, false);

        let outcome = run_boot_reset_with_keychain(ctx);
        assert!(outcome.completed, "retry must complete");
        assert!(
            !trash_app_dir.exists(),
            "prior crash trash must be cleaned by retry"
        );
        assert!(
            !sentinel_path(&app_data).exists(),
            "sentinel must be cleared"
        );
    }

    // ── Test 13: keychain-fail restores all dirs, retry cleans trash ──────

    #[test]
    fn test_keychain_fail_restores_all_then_retry_cleans() {
        let tmp = TempDir::new().unwrap();
        let app_support = tmp.path().join("Application Support");
        let app_data = app_support.join("xyz.block.buzz.app");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::write(app_data.join("config.json"), b"{}").unwrap();

        let legacy = app_support.join("xyz.block.sprout.app");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("identity.key"), b"sprout-key").unwrap();

        write_sentinel(&app_data).unwrap();

        // First attempt: keychain fails → dirs restored.
        let kc1 = FakeKeychain::fail("keychain locked");
        let ctx1 = ResetContext {
            app_data_dir: &app_data,
            legacy_app_data_dir: Some(legacy.clone()),
            nest_dir: None,
            keychain: &kc1,
            home_dir: Some(tmp.path().to_path_buf()),
            is_dev: false,
        };
        let first = run_boot_reset_with_keychain(ctx1);
        assert!(first.failed, "first attempt must fail");
        assert!(
            app_data.exists(),
            "app-data must be restored after keychain fail"
        );
        assert!(
            legacy.exists(),
            "legacy dir must be restored after keychain fail"
        );
        assert!(sentinel_path(&app_data).exists(), "sentinel survives");

        // Second attempt: keychain succeeds → everything cleaned including
        // any residual trash from prior attempts.
        let kc2 = FakeKeychain::ok();
        let ctx2 = ResetContext {
            app_data_dir: &app_data,
            legacy_app_data_dir: Some(legacy.clone()),
            nest_dir: None,
            keychain: &kc2,
            home_dir: Some(tmp.path().to_path_buf()),
            is_dev: false,
        };
        let second = run_boot_reset_with_keychain(ctx2);
        assert!(second.completed, "second attempt must complete");
        assert!(!app_data.exists(), "app-data must be gone");
        assert!(!legacy.exists(), "legacy must be gone");
        // No trash directories should remain.
        let trash_app = app_support.join("xyz.block.buzz.app.reset-trash");
        let trash_legacy = app_support.join("xyz.block.sprout.app.reset-trash");
        assert!(!trash_app.exists(), "app trash must be cleaned");
        assert!(!trash_legacy.exists(), "legacy trash must be cleaned");
    }
}
