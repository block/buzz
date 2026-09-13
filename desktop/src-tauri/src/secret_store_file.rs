//! Debug-only file backend for [`super::SecretStore`].
//!
//! Stores the same JSON blob the keyring backend uses in a `0o600`
//! `secrets.<service>.json` in the app-data dir. See the `secret_store`
//! module docs for why (unsigned dev binaries invalidate the keychain ACL
//! on every rebuild) and for the deliberate no-migration policy.

use std::path::PathBuf;

/// Where a [`super::SecretStore`]'s blob physically lives. The `File` variant
/// is debug-only so release binaries are keyring-only by construction.
pub(super) enum SecretBackend {
    Keyring,
    /// Blob at this exact path (`secrets.<service>.json` in the app-data dir).
    #[cfg(debug_assertions)]
    File(PathBuf),
}

/// App-data dir for the debug file backend, recorded once at boot.
#[cfg(debug_assertions)]
static FILE_BACKEND_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Record the app-data dir the debug file backend stores secrets under.
/// Must run before the first [`super::SecretStore::shared`] call — `lib.rs`
/// setup does, ahead of `run_boot_reset`. Later calls are no-ops.
#[cfg(debug_assertions)]
pub fn init_file_backend_dir(dir: &std::path::Path) {
    let _ = FILE_BACKEND_DIR.set(dir.to_path_buf());
}

/// Pure backend decision for debug builds: keyring when the user opted back
/// in via `BUZZ_DEV_USE_KEYCHAIN=1` or the file dir was never initialized,
/// otherwise the per-service secrets file (namespaced because `just dev` and
/// a main-checkout standalone share one app-data dir with different services).
#[cfg(debug_assertions)]
fn select_backend(
    use_keychain_env: Option<&str>,
    file_dir: Option<&std::path::Path>,
    service: &str,
) -> SecretBackend {
    if use_keychain_env == Some("1") {
        return SecretBackend::Keyring;
    }
    match file_dir {
        Some(dir) => SecretBackend::File(dir.join(format!("secrets.{service}.json"))),
        None => SecretBackend::Keyring,
    }
}

#[cfg(debug_assertions)]
pub(super) fn backend_for(service: &str) -> SecretBackend {
    let env = std::env::var("BUZZ_DEV_USE_KEYCHAIN").ok();
    let backend = select_backend(
        env.as_deref(),
        FILE_BACKEND_DIR.get().map(|p| p.as_path()),
        service,
    );
    if matches!(backend, SecretBackend::Keyring) && env.as_deref() != Some("1") {
        eprintln!(
            "buzz-desktop: file backend dir not initialized; \
             using OS keychain for service {service}"
        );
    }
    backend
}

#[cfg(not(debug_assertions))]
pub(super) fn backend_for(_service: &str) -> SecretBackend {
    SecretBackend::Keyring
}

/// Read the file backend's blob. `Ok(None)` = no file yet (fresh store —
/// deliberately no fallback to the old dev keychain item).
#[cfg(all(debug_assertions, feature = "system-keyring"))]
pub(super) fn read_blob_raw_file(path: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("secrets file read {}: {e}", path.display())),
    }
}

/// Atomically replace the file backend's blob: write a `0o600` sibling tmp
/// file, fsync, rename over the final path. A crash mid-write can never leave
/// a truncated secrets file.
#[cfg(all(debug_assertions, feature = "system-keyring"))]
pub(super) fn write_blob_raw_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("secrets dir create {}: {e}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("secrets.json");
    let tmp = path.with_file_name(format!("{file_name}.tmp"));

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(&tmp)
        .map_err(|e| format!("secrets tmp open {}: {e}", tmp.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("secrets tmp write: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("secrets tmp fsync: {e}"))?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| format!("secrets file rename: {e}"))
}

#[cfg(all(test, debug_assertions, feature = "system-keyring"))]
mod tests {
    use super::super::{KeyringProbe, SecretStore};
    use super::*;
    use std::sync::Mutex;

    // Test-only constructor: file backend at an explicit path, bypassing
    // the process-global FILE_BACKEND_DIR.
    impl SecretStore {
        fn file_at(service: &str, path: std::path::PathBuf) -> Self {
            SecretStore {
                service: service.to_string(),
                backend: SecretBackend::File(path),
                cache: Mutex::new(None),
            }
        }
    }

    fn tmp_store(service: &str) -> (tempfile::TempDir, SecretStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("secrets.{service}.json"));
        let store = SecretStore::file_at(service, path);
        (dir, store)
    }

    #[test]
    fn select_backend_uses_service_namespaced_file_when_dir_set() {
        let dir = std::path::Path::new("/tmp/buzz-test-data");
        match select_backend(None, Some(dir), "buzz-desktop-dev.slug") {
            SecretBackend::File(p) => {
                assert_eq!(p, dir.join("secrets.buzz-desktop-dev.slug.json"))
            }
            SecretBackend::Keyring => panic!("expected file backend"),
        }
    }

    #[test]
    fn select_backend_env_escape_hatch_forces_keyring() {
        let dir = std::path::Path::new("/tmp/buzz-test-data");
        assert!(matches!(
            select_backend(Some("1"), Some(dir), "buzz-desktop-dev"),
            SecretBackend::Keyring
        ));
        // Any value other than "1" does not opt back into the keychain.
        assert!(matches!(
            select_backend(Some("0"), Some(dir), "buzz-desktop-dev"),
            SecretBackend::File(_)
        ));
    }

    #[test]
    fn select_backend_without_dir_falls_back_to_keyring() {
        assert!(matches!(
            select_backend(None, None, "buzz-desktop-dev"),
            SecretBackend::Keyring
        ));
    }

    #[test]
    fn is_file_backed_reflects_backend() {
        let (_dir, store) = tmp_store("buzz-test-file-backed");
        assert!(store.is_file_backed());
        assert!(!SecretStore::keyring("buzz-test-file-backed").is_file_backed());
    }

    #[test]
    fn file_roundtrip_store_load_delete() {
        let (_dir, store) = tmp_store("buzz-test-file-roundtrip");
        store.store("identity", "nsec1aaa").unwrap();
        store.store("agent:abc", "nsec1bbb").unwrap();
        assert_eq!(
            store.load("identity").unwrap(),
            Some("nsec1aaa".to_string())
        );
        assert_eq!(
            store.load("agent:abc").unwrap(),
            Some("nsec1bbb".to_string())
        );
        store.delete("agent:abc").unwrap();
        assert_eq!(store.load("agent:abc").unwrap(), None);
        assert_eq!(
            store.load("identity").unwrap(),
            Some("nsec1aaa".to_string())
        );
    }

    #[test]
    fn missing_key_probe_and_load_never_consult_legacy_keychain() {
        // A fresh (empty) file store must report reachable-but-empty and
        // Ok(None) without falling through to the legacy keychain
        // migration paths — file mode never touches the OS keychain.
        let (_dir, store) = tmp_store("buzz-test-file-no-legacy");
        assert_eq!(store.probe("identity"), KeyringProbe::ReachableButEmpty);
        assert_eq!(store.load("identity").unwrap(), None);
        // Same when a blob exists but the key is absent.
        store.store("other", "v").unwrap();
        assert_eq!(store.probe("identity"), KeyringProbe::ReachableButEmpty);
        assert_eq!(store.load("identity").unwrap(), None);
        assert_eq!(store.probe("other"), KeyringProbe::Present);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_created_0o600() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, store) = tmp_store("buzz-test-file-perms");
        store.store("identity", "nsec1aaa").unwrap();
        let path = dir.path().join("secrets.buzz-test-file-perms.json");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "secrets file must be 0o600");
    }

    #[test]
    fn write_is_atomic_leaves_no_tmp_residue() {
        let (dir, store) = tmp_store("buzz-test-file-atomic");
        store.store("identity", "nsec1aaa").unwrap();
        store.store("agent:abc", "nsec1bbb").unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["secrets.buzz-test-file-atomic.json".to_string()],
            "only the final secrets file may remain: {names:?}"
        );
    }

    #[test]
    fn corrupt_file_fails_closed_and_is_preserved() {
        let (dir, store) = tmp_store("buzz-test-file-corrupt");
        let path = dir.path().join("secrets.buzz-test-file-corrupt.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(store.load("identity").is_err());
        assert_eq!(store.probe("identity"), KeyringProbe::Unreachable);
        // A store() must not clobber the corrupt file — the fresh read
        // inside mutate_blob fails first.
        assert!(store.store("identity", "nsec1aaa").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not json");
    }

    #[test]
    fn two_stores_same_path_observe_each_others_writes() {
        // CI-runnable port of the cross-process stale-cache race test:
        // two instances (= two processes with separate caches) on one
        // file must never drop each other's keys.
        let dir = tempfile::tempdir().unwrap();
        let svc = "buzz-test-file-race";
        let path = dir.path().join(format!("secrets.{svc}.json"));
        let store_a = SecretStore::file_at(svc, path.clone());
        store_a.store("k1", "v1").unwrap(); // warms A's cache
        let store_b = SecretStore::file_at(svc, path.clone());
        store_b.store("k2", "v2").unwrap();
        store_a.store("k3", "v3").unwrap(); // must re-read, not drop k2
        let reader = SecretStore::file_at(svc, path);
        for (k, v) in [("k1", "v1"), ("k2", "v2"), ("k3", "v3")] {
            assert_eq!(
                reader.load(k).unwrap(),
                Some(v.to_string()),
                "{k} must survive"
            );
        }
    }

    #[test]
    fn delete_all_removes_file_and_verifies_wiped() {
        let (dir, store) = tmp_store("buzz-test-file-wipe");
        store.store("identity", "nsec1aaa").unwrap();
        assert!(!store.verify_fully_wiped());
        store.delete_all_with_legacy_cleanup().unwrap();
        let path = dir.path().join("secrets.buzz-test-file-wipe.json");
        assert!(!path.exists(), "secrets file must be deleted");
        assert!(store.verify_fully_wiped());
        // Idempotent on an already-absent file.
        store.delete_all_with_legacy_cleanup().unwrap();
        assert_eq!(store.load("identity").unwrap(), None);
    }
}
