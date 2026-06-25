//! OS keyring access for desktop nsec private keys.
//!
//! Backed by the `keyring` crate (macOS Keychain / Windows Credential Manager /
//! Linux Secret Service via D-Bus). The chosen backend is selected at compile
//! time by the per-target feature in `Cargo.toml`. The `system-keyring`
//! feature gates the whole store; when it is off, [`SecretStore`] is unusable
//! and callers fall back to their own `0o600` file storage.
//!
//! The store is deliberately NOT on any env-read path. `BUZZ_PRIVATE_KEY`
//! resolution for harnessed agents and CI is handled upstream (an env
//! short-circuit for the human key, child-process env injection for agents);
//! adding an env tier here would duplicate that precedence and create a
//! divergent-behavior trap.

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

/// An OS keyring, addressed by service name. Each logical secret is a distinct
/// key within the service (passed to each operation as the keyring "username").
pub struct SecretStore {
    service: String,
}

impl SecretStore {
    /// Keyring-backed store under `service`. The active platform backend
    /// (apple-native / windows-native / sync-secret-service) is chosen at
    /// compile time.
    pub fn keyring(service: impl Into<String>) -> Self {
        SecretStore {
            service: service.into(),
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
    delete_generic_password_options, generic_password, set_generic_password_options,
    PasswordOptions,
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
    /// Probe whether `key` exists and whether the backend is reachable.
    pub fn probe(&self, key: &str) -> KeyringProbe {
        // macOS: probe the Data Protection Keychain first. If DPK is
        // unavailable (unsigned dev build), fall back to the legacy keyring
        // crate path. Items still in the old keychain will be migrated on the
        // first `load` call.
        #[cfg(all(feature = "system-keyring", target_os = "macos"))]
        {
            match generic_password(dpk_opts(&self.service, key)) {
                Ok(_) => KeyringProbe::Present,
                Err(ref e) if is_not_found(e) => KeyringProbe::ReachableButEmpty,
                Err(ref e) if is_dpk_unavailable(e) => {
                    // DPK unavailable (unsigned build) — fall back to keyring.
                    match keyring_entry(&self.service, key) {
                        Ok(entry) => match entry.get_password() {
                            Ok(_) => KeyringProbe::Present,
                            Err(keyring::Error::NoEntry) => KeyringProbe::ReachableButEmpty,
                            Err(e) if is_keyring_availability_error(&e.to_string()) => {
                                KeyringProbe::Unreachable
                            }
                            Err(_) => KeyringProbe::ReachableButEmpty,
                        },
                        Err(e) if is_keyring_availability_error(&e.to_string()) => {
                            KeyringProbe::Unreachable
                        }
                        Err(_) => KeyringProbe::Unreachable,
                    }
                }
                Err(ref e) if is_keyring_availability_error(&e.to_string()) => {
                    KeyringProbe::Unreachable
                }
                Err(_) => KeyringProbe::ReachableButEmpty,
            }
        }
        // Non-macOS system-keyring path (Windows, Linux).
        #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
        {
            match keyring_entry(&self.service, key) {
                Ok(entry) => match entry.get_password() {
                    Ok(_) => KeyringProbe::Present,
                    Err(keyring::Error::NoEntry) => KeyringProbe::ReachableButEmpty,
                    Err(e) if is_keyring_availability_error(&e.to_string()) => {
                        KeyringProbe::Unreachable
                    }
                    Err(_) => KeyringProbe::ReachableButEmpty,
                },
                Err(e) if is_keyring_availability_error(&e.to_string()) => {
                    KeyringProbe::Unreachable
                }
                Err(_) => KeyringProbe::Unreachable,
            }
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            KeyringProbe::Unreachable
        }
    }

    /// Load the secret for `key`. `Ok(None)` when there is no entry; `Err` only
    /// when the backend errored in a way that is not "missing".
    pub fn load(&self, key: &str) -> Result<Option<String>, String> {
        // macOS: try Data Protection Keychain first; fall back to old keychain
        // and migrate on a miss (one-time per item). If DPK is unavailable
        // (unsigned dev build, errSecMissingEntitlement), use the legacy
        // keyring crate path directly — no migration needed in that case.
        #[cfg(all(feature = "system-keyring", target_os = "macos"))]
        {
            match generic_password(dpk_opts(&self.service, key)) {
                Ok(bytes) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|e| format!("keyring utf8: {e}")),
                Err(ref e) if is_not_found(e) => {
                    // Not in DPK — check old keychain and migrate if found.
                    let entry = keyring_entry(&self.service, key)
                        .map_err(|e| format!("keyring entry: {e}"))?;
                    match entry.get_password() {
                        Ok(old_val) => {
                            // Migrate to DPK.
                            self.store(key, &old_val)?;
                            // Best-effort cleanup from old keychain.
                            let _ = entry.delete_credential();
                            Ok(Some(old_val))
                        }
                        Err(keyring::Error::NoEntry) => Ok(None),
                        Err(e) => Err(format!("keyring get: {e}")),
                    }
                }
                Err(ref e) if is_dpk_unavailable(e) => {
                    // DPK unavailable (unsigned build) — use keyring directly.
                    let entry = keyring_entry(&self.service, key)
                        .map_err(|e| format!("keyring entry: {e}"))?;
                    match entry.get_password() {
                        Ok(secret) => Ok(Some(secret)),
                        Err(keyring::Error::NoEntry) => Ok(None),
                        Err(e) => Err(format!("keyring get: {e}")),
                    }
                }
                Err(e) => Err(format!("keyring get: {e}")),
            }
        }
        // Non-macOS system-keyring path (Windows, Linux).
        #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
        {
            let entry =
                keyring_entry(&self.service, key).map_err(|e| format!("keyring entry: {e}"))?;
            match entry.get_password() {
                Ok(secret) => Ok(Some(secret)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(format!("keyring get: {e}")),
            }
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Store `value` for `key`. Reports `Err` on availability failures — callers
    /// decide whether to fall back to file storage.
    pub fn store(&self, key: &str, value: &str) -> Result<(), String> {
        // macOS: write directly to the Data Protection Keychain. If DPK is
        // unavailable (unsigned dev build), fall back to the legacy keyring
        // crate path.
        #[cfg(all(feature = "system-keyring", target_os = "macos"))]
        {
            match set_generic_password_options(value.as_bytes(), dpk_opts(&self.service, key)) {
                Ok(()) => Ok(()),
                Err(ref e) if is_dpk_unavailable(e) => {
                    // DPK unavailable (unsigned build) — use keyring directly.
                    let entry = keyring_entry(&self.service, key)
                        .map_err(|e| format!("keyring entry: {e}"))?;
                    entry
                        .set_password(value)
                        .map_err(|e| format!("keyring set: {e}"))
                }
                Err(e) => Err(format!("keyring set: {e}")),
            }
        }
        // Non-macOS system-keyring path (Windows, Linux).
        #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
        {
            let entry =
                keyring_entry(&self.service, key).map_err(|e| format!("keyring entry: {e}"))?;
            entry
                .set_password(value)
                .map_err(|e| format!("keyring set: {e}"))
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = (key, value);
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Delete the secret for `key`. A missing entry is not an error.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        // macOS: delete from both DPK and old keychain (best-effort on old).
        // If DPK is unavailable (unsigned dev build), fall back to the legacy
        // keyring crate path.
        #[cfg(all(feature = "system-keyring", target_os = "macos"))]
        {
            // Delete from Data Protection Keychain; missing is fine.
            match delete_generic_password_options(dpk_opts(&self.service, key)) {
                Ok(()) => {}
                Err(ref e) if is_not_found(e) => {}
                Err(ref e) if is_dpk_unavailable(e) => {
                    // DPK unavailable (unsigned build) — use keyring directly.
                    let entry = keyring_entry(&self.service, key)
                        .map_err(|e| format!("keyring entry: {e}"))?;
                    return match entry.delete_credential() {
                        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                        Err(e) => Err(format!("keyring delete: {e}")),
                    };
                }
                Err(e) => return Err(format!("keyring delete: {e}")),
            }
            // Best-effort cleanup from old keychain.
            if let Ok(entry) = keyring_entry(&self.service, key) {
                match entry.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => {}
                    Err(e) => {
                        eprintln!("buzz-desktop: old-keychain delete for {key}: {e}");
                    }
                }
            }
            Ok(())
        }
        // Non-macOS system-keyring path (Windows, Linux).
        #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
        {
            let entry =
                keyring_entry(&self.service, key).map_err(|e| format!("keyring entry: {e}"))?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(format!("keyring delete: {e}")),
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
    fn dpk_unavailable_discriminator() {
        // errSecMissingEntitlement = -34018 signals unsigned dev build.
        let e = SFError::from_code(-34018);
        assert!(is_dpk_unavailable(&e));
        // errSecItemNotFound = -25300 is not a DPK-unavailable error.
        let e = SFError::from_code(-25300);
        assert!(!is_dpk_unavailable(&e));
        // errSecDuplicateItem = -25299 is not a DPK-unavailable error.
        let e = SFError::from_code(-25299);
        assert!(!is_dpk_unavailable(&e));
    }
}
