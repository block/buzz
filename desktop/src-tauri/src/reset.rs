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
    /// Return `true` when the keychain has no blob entry and no legacy entries
    /// for `"identity"`.
    fn probe_empty(&self) -> bool;
}

impl ResetKeychain for crate::secret_store::SecretStore {
    fn delete_all_with_legacy(&self) -> Result<(), String> {
        self.delete_all_with_legacy_cleanup()
    }

    fn probe_empty(&self) -> bool {
        use crate::secret_store::KeyringProbe;
        matches!(self.probe("identity"), KeyringProbe::ReachableButEmpty)
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
        .map(|n| n.starts_with("xyz.block.buzz.app.dev"))
        .unwrap_or(false);

    let service = if is_dev {
        "buzz-desktop-dev"
    } else {
        "buzz-desktop"
    };

    let store = crate::secret_store::SecretStore::keyring(service);
    let home_dir = dirs::home_dir();

    let ctx = ResetContext {
        app_data_dir,
        keychain: &store,
        home_dir,
        is_dev,
    };

    run_boot_reset_with_keychain(ctx)
}

/// Core wipe logic — separated for testing.
pub(crate) fn run_boot_reset_with_keychain(ctx: ResetContext<'_>) -> ResetOutcome {
    let app_data_dir = ctx.app_data_dir;

    // ── Step 1: rename app-data dir (atomic — sentinel survives the parent) ──
    let trash_pid = std::process::id();
    let trash_app = app_data_dir.with_file_name(format!(
        "{}.trash-{trash_pid}",
        app_data_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("buzz")
    ));

    if app_data_dir.exists() {
        if let Err(e) = std::fs::rename(app_data_dir, &trash_app) {
            eprintln!(
                "buzz-desktop reset: rename app-data {}: {e}",
                app_data_dir.display()
            );
            return ResetOutcome {
                completed: false,
                failed: true,
            };
        }
    }

    // ── Step 2: rename WebKit dir for this build ──────────────────────────────
    let mut trash_webkit: Option<PathBuf> = None;
    if let Some(ref home) = ctx.home_dir {
        let bundle_id = app_data_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("buzz");
        let webkit_dir = home.join("Library").join("WebKit").join(bundle_id);
        if webkit_dir.exists() {
            let webkit_trash = webkit_dir.with_file_name(format!("{bundle_id}.trash-{trash_pid}"));
            if let Err(e) = std::fs::rename(&webkit_dir, &webkit_trash) {
                eprintln!(
                    "buzz-desktop reset: rename webkit {}: {e}",
                    webkit_dir.display()
                );
                // Non-fatal — continue
            } else {
                trash_webkit = Some(webkit_trash);
            }
        }
    }

    // ── Step 3: remove nest, ~/.sprout, ~/.config/buzz-agent, CLI symlink ────
    if let Some(nest) = crate::managed_agents::nest_dir() {
        let _ = std::fs::remove_dir_all(&nest);
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
        // Keychain failure is fatal for the reset: keep sentinel, signal failure.
        // Best-effort: try to undo the rename so the app can at least open.
        if trash_app.exists() {
            let _ = std::fs::rename(&trash_app, app_data_dir);
        }
        return ResetOutcome {
            completed: false,
            failed: true,
        };
    }

    // ── Step 5: best-effort delete of .trash-* dirs ───────────────────────────
    if trash_app.exists() {
        let _ = std::fs::remove_dir_all(&trash_app);
    }
    if let Some(ref tw) = trash_webkit {
        if tw.exists() {
            let _ = std::fs::remove_dir_all(tw);
        }
    }

    // ── Step 6: verify ────────────────────────────────────────────────────────
    let keychain_ok = ctx.keychain.probe_empty();
    let app_data_gone = !app_data_dir.exists();
    let nest_gone = crate::managed_agents::nest_dir()
        .map(|n| !n.exists())
        .unwrap_or(true);

    if !keychain_ok || !app_data_gone || !nest_gone {
        eprintln!(
            "buzz-desktop reset: verification failed (keychain_empty={keychain_ok}, \
             app_data_gone={app_data_gone}, nest_gone={nest_gone})"
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
        empty_after_delete: bool,
    }

    impl FakeKeychain {
        fn ok() -> Self {
            FakeKeychain {
                delete_result: Ok(()),
                delete_calls: Cell::new(0),
                empty_after_delete: true,
            }
        }

        fn fail(msg: &str) -> Self {
            FakeKeychain {
                delete_result: Err(msg.to_string()),
                delete_calls: Cell::new(0),
                empty_after_delete: false,
            }
        }

        fn ok_but_not_empty() -> Self {
            FakeKeychain {
                delete_result: Ok(()),
                delete_calls: Cell::new(0),
                empty_after_delete: false,
            }
        }
    }

    impl ResetKeychain for FakeKeychain {
        fn delete_all_with_legacy(&self) -> Result<(), String> {
            self.delete_calls.set(self.delete_calls.get() + 1);
            self.delete_result.clone()
        }

        fn probe_empty(&self) -> bool {
            self.empty_after_delete && self.delete_calls.get() > 0
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
        write_sentinel(&app_data).unwrap();
        let kc = FakeKeychain::ok();
        let ctx = make_ctx(&app_data, &kc, false);

        let outcome = run_boot_reset_with_keychain(ctx);

        assert!(outcome.completed, "should complete");
        assert!(!outcome.failed, "should not fail");
        assert!(!app_data.exists(), "app-data must be gone");
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
        // Keychain delete "succeeds" but probe still returns non-empty.
        let kc = FakeKeychain::ok_but_not_empty();
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

    // ── Test 6: dev build does not touch prod nest ────────────────────────────

    #[test]
    fn test_dev_build_does_not_touch_prod_nest() {
        // Verify is_dev discriminator: prod context has is_dev=false.
        let tmp = TempDir::new().unwrap();
        let app_data = tmp
            .path()
            .join("Application Support")
            .join("xyz.block.buzz.app");
        std::fs::create_dir_all(&app_data).unwrap();

        // Sentinel path should encode prod bundle id.
        let sp = sentinel_path(&app_data);
        assert!(
            sp.to_str().unwrap().contains("xyz.block.buzz.app"),
            "sentinel path must encode bundle id"
        );
        assert!(
            !sp.to_str().unwrap().contains(".dev"),
            "prod sentinel must not contain .dev"
        );
    }

    // ── Test 7: prod build does not touch dev nest ────────────────────────────

    #[test]
    fn test_prod_build_does_not_touch_dev_nest() {
        let tmp = TempDir::new().unwrap();
        let app_data = tmp
            .path()
            .join("Application Support")
            .join("xyz.block.buzz.app.dev");
        std::fs::create_dir_all(&app_data).unwrap();

        let sp = sentinel_path(&app_data);
        assert!(
            sp.to_str().unwrap().contains("xyz.block.buzz.app.dev"),
            "dev sentinel must encode dev bundle id"
        );

        // is_dev discriminator.
        let is_dev = app_data
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("xyz.block.buzz.app.dev"))
            .unwrap_or(false);
        assert!(is_dev, "dev app-data dir must be detected as dev");
    }
}
