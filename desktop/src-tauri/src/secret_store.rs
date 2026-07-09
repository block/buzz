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
use std::path::PathBuf;
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

// ── Interprocess advisory lock ─────────────────────────────────────────────
//
// Two concurrent Buzz processes (e.g. the signed DMG build and an unsigned dev
// build via `just staging`) share the same OS keychain blob because the
// service name `"buzz-desktop"` is a constant — it does not key off the bundle
// identifier. Each process holds its own in-memory cache, so without an
// interprocess lock a warm-cache write in process A drops keys added by process
// B between A's last cache-warming read and A's write.
//
// The fix: `mutate_blob` acquires an exclusive advisory file lock, then always
// performs a fresh `read_blob_raw()` inside the lock, applies the mutation,
// writes back, and releases. The cache is still updated after a successful
// write, so same-process reads remain fast. The lock is file-based at a fixed
// per-user path `/tmp/buzz-keychain-<uid>-<service>.lock` on Unix — a path
// that is invariant to `$TMPDIR`/process environment, so both the GUI-launched
// signed DMG and a terminal-launched dev build always take the same lock.

/// Return the path of the advisory lockfile for `service`.
///
/// The path is `/tmp/buzz-keychain-<uid>-<service>.lock` on Unix — a
/// deterministic per-user path that is invariant to `$TMPDIR`/process
/// environment. Both a GUI-launched signed DMG (`launchd`, env-stripped) and a
/// terminal-launched dev build resolve `/tmp` to the same inode, so they
/// contend on the same lockfile and achieve mutual exclusion.
///
/// On Windows the same name used for the kernel mutex is derived from the
/// lockfile path, so the service-keyed uniqueness is preserved.
fn blob_lockfile_path(service: &str) -> PathBuf {
    #[cfg(unix)]
    {
        // Use the real UID so distinct users get distinct lockfiles.
        // SAFETY: getuid() is always safe on Unix — it never fails.
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/buzz-keychain-{uid}-{service}.lock"))
    }
    #[cfg(not(unix))]
    {
        // Windows: no lockfile used (named mutex instead); this path is only
        // used to derive the mutex name and for test assertions.
        std::env::temp_dir().join(format!("buzz-keychain-{service}.lock"))
    }
}

/// Acquire an exclusive advisory file lock for the blob identified by `service`.
///
/// Opens (or creates) the lockfile and blocks until the lock is acquired.
/// Returns the open `File`; the lock is released when the file is dropped.
///
/// On non-Unix/non-Windows platforms this is a no-op that returns a stub.
#[cfg(feature = "system-keyring")]
fn acquire_blob_lock(service: &str) -> Result<BlobLockGuard, String> {
    let path = blob_lockfile_path(service);
    BlobLockGuard::acquire(&path)
}

/// RAII guard that holds an exclusive advisory file lock.
///
/// On Unix, implemented via `flock(2)` on a lockfile in the system temp dir.
/// On Windows, implemented via a named kernel mutex (cross-process, no file I/O
/// needed). The Windows mutex handle is released on drop.
#[cfg(feature = "system-keyring")]
struct BlobLockGuard {
    /// The open lockfile. Never read — held purely for RAII: closing the fd
    /// releases the `flock(LOCK_EX)` on Unix.
    #[cfg(unix)]
    #[allow(dead_code)]
    file: std::fs::File,
    #[cfg(windows)]
    mutex_handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(feature = "system-keyring")]
impl BlobLockGuard {
    fn acquire(path: &std::path::Path) -> Result<Self, String> {
        #[cfg(unix)]
        {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path)
                .map_err(|e| format!("blob lock open {}: {e}", path.display()))?;
            use std::os::unix::io::AsRawFd;
            // LOCK_EX blocks until the lock is acquired (no LOCK_NB).
            let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("blob lock flock: {err}"));
            }
            return Ok(BlobLockGuard { file });
        }

        #[cfg(windows)]
        {
            // Named kernel mutexes are cross-process on Windows — no lockfile
            // needed. Derive a unique mutex name from the lockfile path so
            // distinct services get distinct mutexes.
            let name_str = format!(
                "Local\\BuzzKeychain-{}",
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("default")
            );
            // Encode as null-terminated UTF-16.
            let name_wide: Vec<u16> = name_str
                .encode_utf16()
                .chain(std::iter::once(0u16))
                .collect();
            use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
            use windows_sys::Win32::System::Threading::{
                CreateMutexW, WaitForSingleObject, INFINITE,
            };
            // CreateMutexW: lpMutexAttributes = null (default security),
            // bInitialOwner = FALSE (0), lpName = our mutex name.
            let handle = unsafe {
                CreateMutexW(
                    std::ptr::null::<SECURITY_ATTRIBUTES>(),
                    0,
                    name_wide.as_ptr(),
                )
            };
            // HANDLE = *mut c_void; null means creation failed.
            if handle.is_null() {
                let err = std::io::Error::last_os_error();
                return Err(format!("blob lock CreateMutexW: {err}"));
            }
            let wait_result = unsafe { WaitForSingleObject(handle, INFINITE) };
            if wait_result != WAIT_OBJECT_0 {
                // Also accept WAIT_ABANDONED (0x80) — previous holder crashed;
                // the mutex is still acquired and we own it.
                if wait_result != windows_sys::Win32::Foundation::WAIT_ABANDONED {
                    let err = std::io::Error::last_os_error();
                    unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
                    return Err(format!(
                        "blob lock WaitForSingleObject: {wait_result} / {err}"
                    ));
                }
            }
            return Ok(BlobLockGuard {
                mutex_handle: handle,
            });
        }

        // Fallback for exotic platforms: no-op lock (only Unix/Windows ship).
        #[allow(unreachable_code)]
        Err("blob lock: unsupported platform".to_string())
    }
}

#[cfg(feature = "system-keyring")]
impl Drop for BlobLockGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // Dropping `self.file` closes the fd, which releases flock on Unix.
            // Nothing explicit needed.
        }
        #[cfg(windows)]
        {
            unsafe {
                windows_sys::Win32::System::Threading::ReleaseMutex(self.mutex_handle);
                windows_sys::Win32::Foundation::CloseHandle(self.mutex_handle);
            }
        }
    }
}

// ── End interprocess advisory lock ────────────────────────────────────────

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

    /// Return a process-global `SecretStore` for `service`. All callers with
    /// the same service name share one instance — and therefore one in-memory
    /// cache and one mutex — so concurrent blob read-modify-write operations
    /// see each other's writes and the last-writer-wins race is closed.
    ///
    /// Only one service name (`"buzz-desktop"`) is used in practice. If a
    /// second service name is ever needed, this can be extended to a registry.
    pub fn shared(service: &'static str) -> &'static SecretStore {
        use std::sync::OnceLock;
        static INSTANCE: OnceLock<SecretStore> = OnceLock::new();
        INSTANCE.get_or_init(|| SecretStore::keyring(service))
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

        // Only populate the cache if it is still empty — a concurrent
        // mutate_blob() may have written a newer value while we were reading.
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(map.clone());
        }
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

    /// Atomically load the blob, apply `f` to a candidate map, write back if
    /// changed, and only then advance the cache.
    ///
    /// **Cross-process safety**: acquires an exclusive advisory file lock
    /// (`flock(2)` on Unix, `LockFileEx` on Windows) before reading, mutating,
    /// and writing. The lock is keyed by service name and stored in the system
    /// temp directory, making it reachable from both the signed DMG build and
    /// unsigned dev builds. Inside the lock a fresh `read_blob_raw()` is always
    /// performed (even when the cache is warm) so a concurrent process's write
    /// is never silently dropped.
    ///
    /// **Idempotent**: when `f` leaves the candidate equal to the freshly-read
    /// map, `write_blob_raw` is skipped entirely. On macOS the legacy
    /// `SecKeychain` API treats a write as a distinct ACL operation from the
    /// "Always Allow"-ed read, so skipping no-op writes eliminates the keychain
    /// prompt that fires when saving an agent whose model changed but whose key
    /// did not.
    ///
    /// **Copy-on-write**: the candidate `next` is a separate allocation from
    /// `current`. The cache is only replaced with `next` after `write_blob_raw`
    /// succeeds. On write failure the cache is cleared to `None` so the next
    /// caller re-reads from the keychain rather than building on a stale state.
    ///
    /// Deadlock-free: `read_blob_raw` and `write_blob_raw` do not acquire the
    /// cache mutex. `load_blob` does acquire it, but `mutate_blob` does not call
    /// `load_blob` — it reads from the keyring directly inside the file lock.
    #[cfg(feature = "system-keyring")]
    fn mutate_blob<F>(&self, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut HashMap<String, String>),
    {
        // Acquire the interprocess advisory lock first. All Buzz processes
        // using the same service name contend on the same lockfile at
        // /tmp/buzz-keychain-<uid>-<service>.lock (a deterministic per-user
        // path invariant to $TMPDIR), so only one process performs a
        // read-modify-write at a time.
        let _lock = acquire_blob_lock(&self.service)?;

        // Always do a fresh read from the keychain while holding the lock —
        // this is the critical correction over the prior warm-cache path. A
        // stale warm cache would make us build our candidate on an outdated
        // baseline and drop keys written by another process.
        let raw = self.read_blob_raw()?;
        let current: HashMap<String, String> = match raw {
            None => HashMap::new(),
            Some(bytes) => {
                let json = String::from_utf8(bytes).map_err(|e| format!("blob utf8: {e}"))?;
                serde_json::from_str::<HashMap<String, String>>(&json)
                    .map_err(|e| format!("blob json: {e}"))?
            }
        };

        // Build the candidate state in a separate allocation so that a write
        // failure below cannot leave the cache ahead of durable storage.
        let mut next = current.clone();
        f(&mut next);

        // Skip the keychain write when the candidate equals the freshly-read
        // durable state — no I/O needed and no keychain ACL prompt on macOS.
        if next == current {
            // Update the cache to the fresh read even on no-op so subsequent
            // reads in this process see any keys another process may have added.
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(current);
            return Ok(());
        }

        // Write to keyring while still holding the file lock.
        let json = serde_json::to_string(&next).map_err(|e| format!("blob serialize: {e}"))?;
        match self.write_blob_raw(json.as_bytes()) {
            Ok(()) => {
                // Advance the cache to `next` only after the durable write succeeds.
                let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                *guard = Some(next);
                Ok(())
            }
            Err(e) => {
                // On write failure, clear the cache so the next caller re-reads
                // from the keychain rather than building on a stale state.
                let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                *guard = None;
                Err(e)
            }
        }
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
                        // Blob exists but key absent — still check old per-key
                        // entries so a partial migration (e.g. identity migrated
                        // first) doesn't silently drop agent keys.
                        self.probe_legacy_key(key)
                    }
                }
                // No blob yet — check old per-key entries so callers that
                // gate `load()` on `Present` still trigger migration.
                Ok(None) => self.probe_legacy_key(key),
                Err(e) if is_keyring_availability_error(&e) => KeyringProbe::Unreachable,
                Err(_) => KeyringProbe::Unreachable, // corrupt blob — fail closed
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
    /// and deletes the old item — a one-time migration per key. The same
    /// migration fires when the blob exists but the key is absent, covering
    /// partial-migration scenarios (e.g. identity migrated first, agents not yet).
    pub fn load(&self, key: &str) -> Result<Option<String>, String> {
        #[cfg(feature = "system-keyring")]
        {
            match self.load_blob() {
                Ok(Some(map)) => {
                    if let Some(value) = map.get(key) {
                        Ok(Some(value.clone()))
                    } else {
                        // Blob exists but key absent — attempt migration from old
                        // per-key entry. migrate_legacy_key writes the result into
                        // the blob if found, so subsequent loads hit the cache.
                        self.migrate_legacy_key(key)
                    }
                }
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

    /// Read the secret for `key` without any legacy-migration side effects.
    ///
    /// Unlike [`load`](Self::load), this method never calls
    /// `migrate_legacy_key` and therefore never writes to or deletes from the
    /// keyring. Use this when the caller must guarantee the store is not
    /// mutated — for example, when reading from a foreign service (prod) to
    /// copy values into a dev service.
    ///
    /// Returns `Ok(Some(value))` when the key is present in the blob,
    /// `Ok(None)` when the blob is absent or the key is not in it, and `Err`
    /// only when the backend is unavailable.
    pub fn load_readonly(&self, key: &str) -> Result<Option<String>, String> {
        #[cfg(feature = "system-keyring")]
        {
            match self.load_blob()? {
                Some(map) => Ok(map.get(key).cloned()),
                None => Ok(None),
            }
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Read the entire blob without any legacy-migration side effects.
    ///
    /// Returns the full key→value map when a blob exists, `Ok(None)` when no
    /// blob has been written yet, and `Err` only when the backend is
    /// unavailable. Never calls `migrate_legacy_key`.
    pub fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String> {
        #[cfg(feature = "system-keyring")]
        {
            self.load_blob()
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Insert all entries from `entries` into the blob in a single mutation.
    ///
    /// Entries that already exist in the blob are overwritten; entries not
    /// present in `entries` are left unchanged. If the resulting blob is
    /// identical to what is already stored, no keychain write occurs.
    pub fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        #[cfg(feature = "system-keyring")]
        {
            self.mutate_blob(|map| {
                for (k, v) in entries {
                    map.insert(k.clone(), v.clone());
                }
            })
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = entries;
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// On first launch after upgrading from the per-key DPK format, read the
    /// old DPK entry for `key`, write it into a new blob, and delete the old
    /// item. Returns `Ok(None)` when no old entry exists.
    ///
    /// Also handles a one-time migration from the DPK blob format written by
    /// #1267 (before the dev/release split was fixed). Anyone who ran main
    /// while #1267 was present has a DPK blob instead of per-key entries; this
    /// reads it, merges all keys into the legacy blob, and deletes the DPK blob.
    #[cfg(all(feature = "system-keyring", target_os = "macos"))]
    fn migrate_legacy_key(&self, key: &str) -> Result<Option<String>, String> {
        // One-time migration: check for a DPK blob (key = BLOB_KEY = "secrets")
        // written by #1267 before the dev/release split was fixed.
        match generic_password(dpk_opts(&self.service, BLOB_KEY)) {
            Ok(bytes) => {
                let json = String::from_utf8(bytes).map_err(|e| format!("dpk blob utf8: {e}"))?;
                let dpk_map = serde_json::from_str::<HashMap<String, String>>(&json)
                    .map_err(|e| format!("dpk blob json: {e}"))?;
                // Merge all keys from the DPK blob into the legacy blob.
                self.mutate_blob(|map| {
                    for (k, v) in &dpk_map {
                        map.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                })?;
                // Best-effort delete the DPK blob.
                let _ = delete_generic_password_options(dpk_opts(&self.service, BLOB_KEY));
                return Ok(dpk_map.get(key).cloned());
            }
            Err(ref e) if is_not_found(e) => {
                // No DPK blob — fall through to per-key migration.
            }
            Err(ref e) if is_dpk_unavailable(e) => {
                // Unsigned dev build — DPK inaccessible, fall through.
            }
            Err(e) => return Err(format!("dpk blob read: {e}")),
        }

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

    /// Verify that `key` holds `expected` by reading directly from the OS
    /// backend, bypassing the in-process cache. This is the key innovation for
    /// read-back verification: it proves the OS keyring round-trip, not just
    /// that the in-process cache was updated.
    ///
    /// Returns `Ok(true)` when the stored value matches `expected`, `Ok(false)`
    /// when the entry is absent or holds a different value, and `Err` when the
    /// backend is unavailable.
    pub fn verify_stored_raw(&self, key: &str, expected: &str) -> Result<bool, String> {
        #[cfg(feature = "system-keyring")]
        {
            let raw = self.read_blob_raw()?;
            match raw {
                None => Ok(false),
                Some(bytes) => {
                    let json = String::from_utf8(bytes).map_err(|e| format!("blob utf8: {e}"))?;
                    let map =
                        serde_json::from_str::<std::collections::HashMap<String, String>>(&json)
                            .map_err(|e| format!("blob json: {e}"))?;
                    Ok(map.get(key).is_some_and(|v| v == expected))
                }
            }
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = (key, expected);
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Store `value` for `key`. Reports `Err` on availability failures — callers
    /// decide whether to fall back to file storage.
    pub fn store(&self, key: &str, value: &str) -> Result<(), String> {
        #[cfg(feature = "system-keyring")]
        {
            self.mutate_blob(|map| {
                map.insert(key.to_string(), value.to_string());
            })
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
            self.mutate_blob(|map| {
                map.remove(key);
            })?;
            // Best-effort: also delete any old per-key entry for this key to
            // prevent resurrection on the next probe/load (migration path).
            #[cfg(target_os = "macos")]
            let _ = delete_generic_password_options(dpk_opts(&self.service, key));
            if let Ok(entry) = keyring_entry(&self.service, key) {
                let _ = entry.delete_credential();
            }
            Ok(())
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

    // Test-only constructor: pre-seed the cache without touching the OS keychain.
    impl SecretStore {
        fn with_cache(service: &str, cache: Option<HashMap<String, String>>) -> Self {
            SecretStore {
                service: service.to_string(),
                cache: Mutex::new(cache),
            }
        }
    }

    #[test]
    fn probe_returns_present_when_key_in_cache() {
        let mut map = HashMap::new();
        map.insert("identity".to_string(), "nsec1test".to_string());
        let store = SecretStore::with_cache("buzz-test-cache-hit", Some(map));
        // Cache is warm and contains "identity" — probe must return Present
        // without touching the keychain.
        assert_eq!(store.probe("identity"), KeyringProbe::Present);
    }

    #[test]
    fn load_returns_value_when_key_in_cache() {
        let mut map = HashMap::new();
        map.insert("identity".to_string(), "nsec1test".to_string());
        let store = SecretStore::with_cache("buzz-test-load-cache-hit", Some(map));
        // Cache is warm and contains "identity" — load must return the value
        // without touching the keychain.
        assert_eq!(
            store.load("identity").unwrap(),
            Some("nsec1test".to_string())
        );
    }

    // ── Cross-process race tests (require real OS keychain) ────────────────

    #[ignore = "requires real OS keychain (run locally)"]
    #[test]
    fn test_stale_warm_cache_add_observes_prior_write() {
        // Simulates the cross-process race that stranded Will's agent keys.
        //
        // Setup: two SecretStore instances for the same service (= two
        // "processes" with separate caches). Process A warms its cache to
        // {k1}. Process B then writes {k1, k2}. Without the fix, A's next
        // mutate_blob would build from its stale {k1} cache and write
        // {k1, k3}, silently dropping k2. With the fix, A always re-reads
        // from the keychain inside the lock, so the result is {k1, k2, k3}.
        let svc = "buzz-test-race-stale-cache";

        // Clean state.
        let setup = SecretStore::keyring(svc);
        let _ = setup.delete("k1");
        let _ = setup.delete("k2");
        let _ = setup.delete("k3");

        // Process A: write k1, warming its cache.
        let store_a = SecretStore::keyring(svc);
        store_a.store("k1", "v1").unwrap();

        // Process B: write k2 (separate instance = separate cache).
        let store_b = SecretStore::keyring(svc);
        store_b.store("k2", "v2").unwrap();

        // Process A: write k3. With the fix, A re-reads inside the lock and
        // sees {k1, k2} before appending k3 — result must be {k1, k2, k3}.
        store_a.store("k3", "v3").unwrap();

        // Verify via a third reader (clean cache).
        let reader = SecretStore::keyring(svc);
        assert_eq!(
            reader.load("k1").unwrap(),
            Some("v1".to_string()),
            "k1 must survive"
        );
        assert_eq!(
            reader.load("k2").unwrap(),
            Some("v2".to_string()),
            "k2 must not be dropped"
        );
        assert_eq!(
            reader.load("k3").unwrap(),
            Some("v3".to_string()),
            "k3 must be written"
        );

        // Cleanup.
        let _ = reader.delete("k1");
        let _ = reader.delete("k2");
        let _ = reader.delete("k3");
    }

    #[ignore = "requires real OS keychain (run locally)"]
    #[test]
    fn test_concurrent_adds_neither_key_dropped() {
        // Two sequential stores from distinct instances (simulating two
        // processes each adding one key) must both be durably visible.
        let svc = "buzz-test-race-concurrent-add";

        let setup = SecretStore::keyring(svc);
        let _ = setup.delete("agent_a");
        let _ = setup.delete("agent_b");

        let store1 = SecretStore::keyring(svc);
        store1.store("agent_a", "nsec1aaa").unwrap();

        let store2 = SecretStore::keyring(svc);
        store2.store("agent_b", "nsec1bbb").unwrap();

        let reader = SecretStore::keyring(svc);
        assert_eq!(
            reader.load("agent_a").unwrap(),
            Some("nsec1aaa".to_string()),
            "agent_a must not be dropped"
        );
        assert_eq!(
            reader.load("agent_b").unwrap(),
            Some("nsec1bbb".to_string()),
            "agent_b must not be dropped"
        );

        // Cleanup.
        let _ = reader.delete("agent_a");
        let _ = reader.delete("agent_b");
    }

    #[test]
    fn test_blob_lockfile_path_is_in_tmp_with_uid() {
        // The lockfile must be at a deterministic per-user path under /tmp —
        // invariant to $TMPDIR — so both a GUI-launched DMG (env-stripped by
        // launchd) and a terminal-launched dev build resolve the same inode and
        // achieve mutual exclusion.
        let path = blob_lockfile_path("buzz-desktop");
        #[cfg(unix)]
        {
            let uid = unsafe { libc::getuid() };
            assert!(
                path.starts_with("/tmp"),
                "lockfile {path:?} must start with /tmp (not $TMPDIR)"
            );
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            assert!(
                name.contains(&uid.to_string()),
                "lockfile {path:?} must contain uid {uid}"
            );
            assert!(
                name.contains("buzz-keychain"),
                "lockfile name must contain 'buzz-keychain'"
            );
        }
        #[cfg(not(unix))]
        {
            assert!(
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains("buzz-keychain")),
                "lockfile name must contain 'buzz-keychain'"
            );
        }
    }

    #[test]
    fn test_blob_lock_acquire_and_release() {
        // Verify the advisory lock can be acquired and released without errors.
        // This exercises the real flock/mutex path on the current platform.
        let guard = acquire_blob_lock("buzz-test-lock-smoke");
        assert!(
            guard.is_ok(),
            "advisory lock acquire must succeed: {:?}",
            guard.err()
        );
        // Drop the guard — lock is released. A second acquire must succeed.
        drop(guard);
        let guard2 = acquire_blob_lock("buzz-test-lock-smoke");
        assert!(
            guard2.is_ok(),
            "advisory lock re-acquire after release must succeed: {:?}",
            guard2.err()
        );
    }

    #[ignore = "requires real OS keychain (run locally)"]
    #[test]
    fn mutate_blob_does_not_advance_cache_on_write_failure() {
        // Copy-on-write safety: if `write_blob_raw` fails (denied prompt,
        // transient outage, ACL rejection), the cache must stay at the last
        // known durable state. A subsequent `store()` for the same key/value
        // must NOT be skipped as a no-op — the equality check must compare
        // against the durable cache, not an unpersisted candidate.
        //
        // This is a real-keychain integration test. Run locally with:
        //   cargo test -p buzz-desktop -- --ignored mutate_blob_does_not_advance
        //
        // On a machine with a reachable keychain the `store()` call succeeds
        // (result.is_ok()) and the write-failure branch is skipped — the test
        // still passes. On a machine where the write is denied (e.g., user
        // clicks Deny in the macOS prompt) result.is_err() and the assertions
        // below verify the cache invariant. We verify that after an error:
        //   1. The cache is not advanced (the previously cached key is intact).
        //   2. The failed key is not present (the dirty candidate was discarded).
        let mut map = HashMap::new();
        map.insert("existing".to_string(), "durable_val".to_string());
        let store = SecretStore::with_cache("buzz-test-cow-write-fail", Some(map));

        // Attempt to add a new key — this calls write_blob_raw against the
        // real keychain; with copy-on-write the cache must remain at {existing}
        // if the write fails.
        let result = store.store("new_key", "new_val");

        if result.is_err() {
            // Write failed (e.g., user denied the keychain prompt): confirm
            // cache was not advanced — the existing key is still intact and
            // the new key was never committed to the in-memory state.
            assert_eq!(
                store.load("existing").unwrap(),
                Some("durable_val".to_string()),
                "cache must remain at last durable state after write failure"
            );
            // load("new_key") goes through the unchanged cache (no entry),
            // then attempts migrate_legacy_key which also fails on a denied
            // keychain, returning either Ok(None) or Err — either is correct
            // since the key was never durably stored.
            let after = store.load("new_key");
            assert!(
                matches!(after, Ok(None) | Err(_)),
                "a key whose write failed must not be visible via load: {after:?}"
            );
        }
        // If result.is_ok() the write succeeded — the cache-integrity invariant
        // does not apply to the success path; no assertion needed here.
    }

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
