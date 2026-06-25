//! OS keyring access for desktop nsec private keys.
//!
//! All secrets are stored as a single JSON blob under one keychain entry
//! (service = the store's service name, username = `"secrets"`). This means
//! exactly one OS prompt per process lifetime regardless of how many keys are
//! stored — the same pattern used by Goose.
//!
//! The chosen backend is selected at compile time by the per-target feature in
//! `Cargo.toml`. On macOS the legacy `keyring` crate (SecKeychain API) is used
//! for the blob entry so that signed release builds and unsigned dev builds
//! share the same store. DPK (Data Protection Keychain) is used only by the
//! one-time migration path that reads old per-key entries written by #1264.
//! Windows and Linux use the `keyring` crate directly. The `system-keyring`
//! feature gates the whole store; when it is off, [`SecretStore`] is unusable
//! and callers fall back to their own `0o600` file storage.
//!
//! The store is deliberately NOT on any env-read path. `BUZZ_PRIVATE_KEY`
//! resolution for harnessed agents and CI is handled upstream (an env
//! short-circuit for the human key, child-process env injection for agents);
//! adding an env tier here would duplicate that precedence and create a
//! divergent-behavior trap.

use std::collections::HashMap;
use std::sync::Mutex;

/// Result of probing the keyring before a migration: distinguishes "reachable
/// but holds no entry" (safe to migrate into) from "unreachable this boot"
/// (must NOT migrate — re-importing from a leftover plaintext file could
/// resurrect a rotated/stale key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyringProbe {
    /// Keyring is reachable and an entry for the key already exists.
    Present,
    /// Keyring is reachable but has no entry for the key.
    ReachableButEmpty,
    /// Keyring backend is unavailable this boot (no Secret Service, dbus
    /// failure, etc.). Migration must be skipped.
    Unreachable,
}

/// Username used for the single blob keychain entry. All secrets are stored
/// as a JSON map under this name within the service.
const BLOB_KEY: &str = "secrets";

/// An OS keyring, addressed by service name. All secrets are stored in a
/// single JSON blob entry (one OS prompt per process lifetime).
pub struct SecretStore {
    service: String,
    /// In-memory cache of the deserialized blob. `None` means "not yet loaded".
    cache: Mutex<Option<HashMap<String, String>>>,
}

impl SecretStore {
    /// Keyring-backed store under `service`. The active platform backend
    /// (apple-native / windows-native / sync-secret-service) is chosen at
    /// compile time.
    pub fn keyring(service: impl Into<String>) -> Self {
        SecretStore {
            service: service.into(),
            cache: Mutex::new(None),
        }
    }
}

/// Whether a keyring error string indicates the backend itself is unavailable
/// (vs. a per-entry error like "not found"). Mirrors goose's discriminator
/// (`crates/goose/src/config/base.rs`): treat dbus / Secret Service / platform
/// secure-storage failures as "keyring unavailable, fall back to file".
#[cfg(feature = "system-keyring")]
fn is_keyring_availability_error(error_str: &str) -> bool {
    let lower = error_str.to_lowercase();
    lower.contains("keyring")
        || lower.contains("dbus")
        || lower.contains("org.freedesktop.secrets")
        || lower.contains("platform secure storage")
        || lower.contains("no secret service")
}

#[cfg(feature = "system-keyring")]
fn keyring_entry(service: &str, key: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(service, key)
}

// macOS-specific imports for the Data Protection Keychain backend.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
use security_framework::base::Error as SFError;
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
use security_framework::passwords::{
    delete_generic_password_options, generic_password, PasswordOptions,
};

/// Returns true when the security-framework error is "item not found" (-25300).
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
fn is_not_found(e: &SFError) -> bool {
    e.code() == -25300
}

/// Returns true when DPK is unavailable because the binary lacks the required
/// entitlement (`errSecMissingEntitlement`, -34018). This happens for unsigned
/// dev builds (`tauri dev` / `cargo run`). The caller should fall back to the
/// legacy `keyring` crate path, which uses the old-style keychain and does not
/// require hardened-runtime entitlements.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
fn is_dpk_unavailable(e: &SFError) -> bool {
    e.code() == -34018
}

/// Build a `PasswordOptions` for the Data Protection Keychain.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
fn dpk_opts(service: &str, key: &str) -> PasswordOptions {
    let mut opts = PasswordOptions::new_generic_password(service, key);
    opts.use_protected_keychain();
    opts
}

impl SecretStore {
    /// Read the blob from the keychain and return the deserialized map.
    ///
    /// Returns `Ok(None)` when no blob entry exists yet (first launch or
    /// fresh install). Returns `Err` when the backend is unavailable or the
    /// stored JSON is corrupt.
    ///
    /// On success the result is stored in `self.cache` so subsequent calls
    /// within the same process return immediately without a keychain round-trip.
    #[cfg(feature = "system-keyring")]
    fn load_blob(&self) -> Result<Option<HashMap<String, String>>, String> {
        {
            let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ref map) = *guard {
                return Ok(Some(map.clone()));
            }
        }

        let raw = self.read_blob_raw()?;
        let map = match raw {
            None => return Ok(None),
            Some(bytes) => {
                let json = String::from_utf8(bytes).map_err(|e| format!("blob utf8: {e}"))?;
                serde_json::from_str::<HashMap<String, String>>(&json)
                    .map_err(|e| format!("blob json: {e}"))?
            }
        };

        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(map.clone());
        Ok(Some(map))
    }

    /// Read the raw blob bytes from the keychain. `Ok(None)` = not found.
    ///
    /// Always uses the legacy keyring crate on macOS so that signed and
    /// unsigned (dev) builds share the same store. DPK is only used by
    /// `migrate_legacy_key` to read old per-key entries written by #1264.
    #[cfg(all(feature = "system-keyring", target_os = "macos"))]
    fn read_blob_raw(&self) -> Result<Option<Vec<u8>>, String> {
        self.read_blob_raw_keyring()
    }

    #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
    fn read_blob_raw(&self) -> Result<Option<Vec<u8>>, String> {
        self.read_blob_raw_keyring()
    }

    /// Read blob via the legacy `keyring` crate (Windows, Linux, or macOS dev
    /// builds that lack hardened-runtime entitlements).
    #[cfg(feature = "system-keyring")]
    fn read_blob_raw_keyring(&self) -> Result<Option<Vec<u8>>, String> {
        let entry =
            keyring_entry(&self.service, BLOB_KEY).map_err(|e| format!("keyring entry: {e}"))?;
        match entry.get_password() {
            Ok(s) => Ok(Some(s.into_bytes())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) if is_keyring_availability_error(&e.to_string()) => {
                Err(format!("keyring unavailable: {e}"))
            }
            Err(e) => Err(format!("keyring read: {e}")),
        }
    }

    /// Serialize `map` to JSON and write it as the single blob keychain entry.
    #[cfg(feature = "system-keyring")]
    fn save_blob(&self, map: &HashMap<String, String>) -> Result<(), String> {
        let json = serde_json::to_string(map).map_err(|e| format!("blob serialize: {e}"))?;
        self.write_blob_raw(json.as_bytes())?;
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(map.clone());
        Ok(())
    }

    /// Always uses the legacy keyring crate on macOS — see `read_blob_raw`.
    #[cfg(all(feature = "system-keyring", target_os = "macos"))]
    fn write_blob_raw(&self, bytes: &[u8]) -> Result<(), String> {
        self.write_blob_raw_keyring(bytes)
    }

    #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
    fn write_blob_raw(&self, bytes: &[u8]) -> Result<(), String> {
        self.write_blob_raw_keyring(bytes)
    }

    #[cfg(feature = "system-keyring")]
    fn write_blob_raw_keyring(&self, bytes: &[u8]) -> Result<(), String> {
        let value = std::str::from_utf8(bytes).map_err(|e| format!("blob utf8 encode: {e}"))?;
        let entry =
            keyring_entry(&self.service, BLOB_KEY).map_err(|e| format!("keyring entry: {e}"))?;
        entry
            .set_password(value)
            .map_err(|e| format!("keyring write: {e}"))
    }

    /// Probe whether `key` exists and whether the backend is reachable.
    pub fn probe(&self, key: &str) -> KeyringProbe {
        #[cfg(feature = "system-keyring")]
        {
            match self.load_blob() {
                Ok(Some(map)) => {
                    if map.contains_key(key) {
                        KeyringProbe::Present
                    } else {
                        KeyringProbe::ReachableButEmpty
                    }
                }
                // No blob yet — check old per-key entries so callers that
                // gate `load()` on `Present` still trigger migration.
                Ok(None) => self.probe_legacy_key(key),
                Err(e) if is_keyring_availability_error(&e) => KeyringProbe::Unreachable,
                Err(_) => KeyringProbe::ReachableButEmpty,
            }
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            KeyringProbe::Unreachable
        }
    }

    /// Check old per-key DPK/keyring entries for `key`. Used by `probe()` when
    /// the blob doesn't exist yet (first launch after upgrade).
    #[cfg(all(feature = "system-keyring", target_os = "macos"))]
    fn probe_legacy_key(&self, key: &str) -> KeyringProbe {
        match generic_password(dpk_opts(&self.service, key)) {
            Ok(_) => KeyringProbe::Present,
            Err(ref e) if is_not_found(e) => self.probe_legacy_key_keyring(key),
            Err(ref e) if is_dpk_unavailable(e) => self.probe_legacy_key_keyring(key),
            Err(ref e) if is_keyring_availability_error(&e.to_string()) => {
                KeyringProbe::Unreachable
            }
            Err(_) => KeyringProbe::ReachableButEmpty,
        }
    }

    #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
    fn probe_legacy_key(&self, key: &str) -> KeyringProbe {
        self.probe_legacy_key_keyring(key)
    }

    #[cfg(feature = "system-keyring")]
    fn probe_legacy_key_keyring(&self, key: &str) -> KeyringProbe {
        match keyring_entry(&self.service, key) {
            Ok(entry) => match entry.get_password() {
                Ok(_) => KeyringProbe::Present,
                Err(keyring::Error::NoEntry) => KeyringProbe::ReachableButEmpty,
                Err(e) if is_keyring_availability_error(&e.to_string()) => {
                    KeyringProbe::Unreachable
                }
                Err(_) => KeyringProbe::ReachableButEmpty,
            },
            Err(e) if is_keyring_availability_error(&e.to_string()) => KeyringProbe::Unreachable,
            Err(_) => KeyringProbe::Unreachable,
        }
    }

    /// Load the secret for `key`. `Ok(None)` when there is no entry; `Err` only
    /// when the backend errored in a way that is not "missing".
    ///
    /// On first launch after an upgrade from the per-key DPK format, the blob
    /// will not exist yet. In that case the macOS path falls back to reading the
    /// old per-key DPK entry for `key` specifically, writes it into a new blob,
    /// and deletes the old item — a one-time migration per key.
    pub fn load(&self, key: &str) -> Result<Option<String>, String> {
        #[cfg(feature = "system-keyring")]
        {
            match self.load_blob() {
                Ok(Some(map)) => Ok(map.get(key).cloned()),
                Ok(None) => {
                    // No blob yet — attempt one-time migration from old per-key
                    // DPK entry (macOS) or return Ok(None) (other platforms).
                    self.migrate_legacy_key(key)
                }
                Err(e) => Err(e),
            }
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// On first launch after upgrading from the per-key DPK format, read the
    /// old DPK entry for `key`, write it into a new blob, and delete the old
    /// item. Returns `Ok(None)` when no old entry exists.
    #[cfg(all(feature = "system-keyring", target_os = "macos"))]
    fn migrate_legacy_key(&self, key: &str) -> Result<Option<String>, String> {
        // Try the old per-key DPK entry.
        match generic_password(dpk_opts(&self.service, key)) {
            Ok(bytes) => {
                let value = String::from_utf8(bytes).map_err(|e| format!("keyring utf8: {e}"))?;
                // Write into blob (creates the blob if it doesn't exist).
                self.store(key, &value)?;
                // Best-effort cleanup of the old per-key entry.
                let _ = delete_generic_password_options(dpk_opts(&self.service, key));
                Ok(Some(value))
            }
            Err(ref e) if is_not_found(e) => {
                // Also check the old keyring-crate entry (pre-#1264 installs).
                self.migrate_legacy_key_keyring(key)
            }
            Err(ref e) if is_dpk_unavailable(e) => {
                // Unsigned dev build — check old keyring-crate entry only.
                self.migrate_legacy_key_keyring(key)
            }
            Err(e) => Err(format!("keyring get: {e}")),
        }
    }

    #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
    fn migrate_legacy_key(&self, key: &str) -> Result<Option<String>, String> {
        // Non-macOS: no DPK, just check the old keyring-crate per-key entry.
        self.migrate_legacy_key_keyring(key)
    }

    /// Check the old per-key `keyring` crate entry (pre-#1264 format) and
    /// migrate it into the blob if found.
    #[cfg(feature = "system-keyring")]
    fn migrate_legacy_key_keyring(&self, key: &str) -> Result<Option<String>, String> {
        let entry = keyring_entry(&self.service, key).map_err(|e| format!("keyring entry: {e}"))?;
        match entry.get_password() {
            Ok(value) => {
                self.store(key, &value)?;
                let _ = entry.delete_credential();
                Ok(Some(value))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("keyring get: {e}")),
        }
    }

    /// Store `value` for `key`. Reports `Err` on availability failures — callers
    /// decide whether to fall back to file storage.
    pub fn store(&self, key: &str, value: &str) -> Result<(), String> {
        #[cfg(feature = "system-keyring")]
        {
            let mut map = self.load_blob()?.unwrap_or_default();
            map.insert(key.to_string(), value.to_string());
            self.save_blob(&map)
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = (key, value);
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Delete the secret for `key`. A missing entry is not an error.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        #[cfg(feature = "system-keyring")]
        {
            match self.load_blob()? {
                Some(mut map) => {
                    map.remove(key);
                    self.save_blob(&map)
                }
                // No blob — nothing to delete.
                None => Ok(()),
            }
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            Err("system-keyring feature disabled".to_string())
        }
    }
}

#[cfg(all(test, feature = "system-keyring"))]
mod tests {
    use super::*;

    #[test]
    fn availability_error_discriminator() {
        assert!(is_keyring_availability_error("dbus connection failed"));
        assert!(is_keyring_availability_error(
            "org.freedesktop.secrets not provided"
        ));
        assert!(is_keyring_availability_error("No Secret Service"));
        assert!(is_keyring_availability_error(
            "Platform secure storage failure"
        ));
        // A plain "not found" is per-entry, not an availability failure.
        assert!(!is_keyring_availability_error("entry not found"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn dpk_error_discriminators() {
        // errSecMissingEntitlement = -34018 signals unsigned dev build.
        let e = SFError::from_code(-34018);
        assert!(is_dpk_unavailable(&e));
        assert!(!is_not_found(&e));
        // errSecItemNotFound = -25300 is not a DPK-unavailable error.
        let e = SFError::from_code(-25300);
        assert!(is_not_found(&e));
        assert!(!is_dpk_unavailable(&e));
    }

    // Integration tests that exercise the real OS keychain. Skipped in CI
    // (unsigned builds lack keychain entitlements); run locally with:
    //   cargo test -p buzz-desktop -- --ignored blob_
    //
    // Each test uses a unique service name to avoid cross-test pollution.

    #[ignore = "requires real OS keychain (run locally)"]
    #[test]
    fn blob_stores_and_retrieves_multiple_keys() {
        let store = SecretStore::keyring("buzz-test-blob-multi");
        store.store("key_a", "val_a").unwrap();
        store.store("key_b", "val_b").unwrap();
        assert_eq!(store.load("key_a").unwrap(), Some("val_a".to_string()));
        assert_eq!(store.load("key_b").unwrap(), Some("val_b".to_string()));
        assert_eq!(store.load("key_c").unwrap(), None);
        // Cleanup.
        let _ = store.delete("key_a");
        let _ = store.delete("key_b");
    }

    #[ignore = "requires real OS keychain (run locally)"]
    #[test]
    fn blob_probe_present_absent_unreachable() {
        let store = SecretStore::keyring("buzz-test-blob-probe");
        // No blob yet — key absent, backend reachable.
        assert_eq!(store.probe("identity"), KeyringProbe::ReachableButEmpty);
        store.store("identity", "nsec1test").unwrap();
        // Key now present.
        assert_eq!(store.probe("identity"), KeyringProbe::Present);
        // Different key — blob exists but key absent.
        assert_eq!(store.probe("other"), KeyringProbe::ReachableButEmpty);
        // Cleanup.
        let _ = store.delete("identity");
    }

    #[ignore = "requires real OS keychain (run locally)"]
    #[test]
    fn blob_delete_removes_key_not_others() {
        let store = SecretStore::keyring("buzz-test-blob-delete");
        store.store("keep", "keep_val").unwrap();
        store.store("remove", "remove_val").unwrap();
        store.delete("remove").unwrap();
        assert_eq!(store.load("keep").unwrap(), Some("keep_val".to_string()));
        assert_eq!(store.load("remove").unwrap(), None);
        // Cleanup.
        let _ = store.delete("keep");
    }

    #[ignore = "requires real OS keychain (run locally)"]
    #[test]
    fn blob_migration_from_per_key_entry() {
        let svc = "buzz-test-blob-migration";
        let key = "identity";
        let value = "nsec1migrationtest";

        // Seed a per-key entry (old format) — no blob exists.
        let entry = keyring_entry(svc, key).unwrap();
        entry.set_password(value).unwrap();

        // Fresh store — no blob in the keychain yet.
        let store = SecretStore::keyring(svc);

        // probe should find the legacy key.
        assert_eq!(store.probe(key), KeyringProbe::Present);

        // load should migrate it into the blob and return the value.
        assert_eq!(store.load(key).unwrap(), Some(value.to_string()));

        // Old per-key entry should be cleaned up.
        let entry = keyring_entry(svc, key).unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));

        // Key is now in the blob — probe confirms.
        let store2 = SecretStore::keyring(svc);
        assert_eq!(store2.probe(key), KeyringProbe::Present);
        assert_eq!(store2.load(key).unwrap(), Some(value.to_string()));

        // Cleanup.
        let _ = store2.delete(key);
    }
}
