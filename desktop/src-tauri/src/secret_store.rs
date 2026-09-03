//! OS keyring access for desktop nsec private keys.
//!
//! Each secret is stored as its **own** OS credential (service = the store's
//! service name, username = the secret's key name, e.g. `"identity"` or
//! `"agent:<pubkey>"`). A separate chunked *index* (`"secrets-index"`,
//! `"secrets-index-1"`, …) records which key names exist so the whole set can
//! be enumerated without a platform-specific credential-enumeration API.
//!
//! Secrets used to live in a single JSON blob under one entry (username =
//! `"secrets"`). That blob is a hard cap, not a style choice: the `keyring`
//! crate rejects any write over Windows' `CRED_MAX_CREDENTIAL_BLOB_SIZE`
//! (2560 bytes) *before* calling `CredWriteW`, so once the blob held an
//! identity plus 8 agents (~2380 bytes) every subsequent agent write failed
//! atomically and fell back to inline plaintext storage. One credential per
//! secret removes the shared budget — each agent key is ~140 bytes on its own.
//! [`SecretStore::ensure_migrated`] performs the one-time, verified, all-or-
//! nothing move out of the legacy blob on first access.
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

/// Username of the **legacy** single-blob keychain entry. No longer written;
/// read once by [`SecretStore::ensure_migrated`] and deleted after every secret
/// it held has been rewritten as its own credential and read back verified.
const BLOB_KEY: &str = "secrets";

/// Username of index chunk 0. Chunk `i > 0` lives at `"secrets-index-<i>"`.
///
/// The index holds key *names* only — never secret values — so a corrupt or
/// partially-written index can never lose a secret: `load()` addresses each
/// credential by name and never consults the index.
#[cfg(feature = "system-keyring")]
const INDEX_KEY: &str = "secrets-index";

/// Character budget for one index chunk. Windows caps a credential blob at
/// `CRED_MAX_CREDENTIAL_BLOB_SIZE` = 2560 bytes and the backend measures the
/// UTF-16 encoding, so 1000 chars = 2000 bytes leaves ample headroom. Without
/// chunking the index would simply reintroduce the blob's cliff at ~17 agents
/// (an `"agent:<pubkey>"` name is ~70 chars).
#[cfg(feature = "system-keyring")]
const INDEX_CHUNK_CHARS: usize = 1000;

/// Hard ceiling on index chunks, so a corrupt chunk count can never spin the
/// reader over unbounded credential lookups. 512 chunks ≈ 6500 secrets.
#[cfg(feature = "system-keyring")]
const MAX_INDEX_CHUNKS: usize = 512;

/// Username of index chunk `i`.
#[cfg(feature = "system-keyring")]
fn index_chunk_key(i: usize) -> String {
    if i == 0 {
        INDEX_KEY.to_string()
    } else {
        format!("{INDEX_KEY}-{i}")
    }
}

/// One index chunk as stored. `chunks` is present only on chunk 0, where it
/// declares how many chunks the reader must visit.
#[cfg(feature = "system-keyring")]
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct IndexChunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chunks: Option<usize>,
    #[serde(default)]
    names: Vec<String>,
}

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

/// An OS keyring, addressed by service name. Each secret is one credential
/// under that service, keyed by the secret's name.
pub struct SecretStore {
    service: String,
    /// In-memory cache of key→value pairs already read from (or written to)
    /// the OS this process. **Partial**: a missing entry means "not loaded",
    /// never "not stored", so a cache miss always falls through to the OS.
    cache: Mutex<Option<HashMap<String, String>>>,
    /// `true` once the legacy blob has been migrated away (or was never
    /// present). Left `false` on failure so the next access retries — the
    /// migration is idempotent.
    migrated: Mutex<bool>,
    /// Test-only: route all credential I/O to an in-process fake backend that
    /// enforces the same Windows blob-size cap the real backend does.
    #[cfg(test)]
    #[allow(dead_code)]
    fake: bool,
}

impl SecretStore {
    /// Keyring-backed store under `service`. The active platform backend
    /// (apple-native / windows-native / sync-secret-service) is chosen at
    /// compile time.
    pub fn keyring(service: impl Into<String>) -> Self {
        SecretStore {
            service: service.into(),
            cache: Mutex::new(None),
            migrated: Mutex::new(false),
            #[cfg(test)]
            fake: false,
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
    // ── Credential I/O ────────────────────────────────────────────────────
    //
    // Every credential read/write/delete in this module funnels through these
    // three functions, so the test fake has exactly one injection point and a
    // fake-backed store can never reach the real OS keychain.

    /// Read one credential under this service. `Ok(None)` = no such entry.
    ///
    /// Always uses the `keyring` crate — on macOS that is the legacy
    /// SecKeychain API, so signed release builds and unsigned dev builds share
    /// one store. DPK is used only by the `migrate_legacy_key` read paths.
    #[cfg(feature = "system-keyring")]
    fn entry_get(&self, key: &str) -> Result<Option<String>, String> {
        #[cfg(test)]
        {
            if self.fake {
                return fake_backend::get(&self.service, key);
            }
        }
        let entry = keyring_entry(&self.service, key).map_err(|e| format!("keyring entry: {e}"))?;
        match entry.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) if is_keyring_availability_error(&e.to_string()) => {
                Err(format!("keyring unavailable: {e}"))
            }
            Err(e) => Err(format!("keyring read: {e}")),
        }
    }

    /// Write one credential under this service.
    #[cfg(feature = "system-keyring")]
    fn entry_set(&self, key: &str, value: &str) -> Result<(), String> {
        #[cfg(test)]
        {
            if self.fake {
                return fake_backend::set(&self.service, key, value);
            }
        }
        let entry = keyring_entry(&self.service, key).map_err(|e| format!("keyring entry: {e}"))?;
        entry
            .set_password(value)
            .map_err(|e| format!("keyring write: {e}"))
    }

    /// Delete one credential under this service. A missing entry is not an error.
    #[cfg(feature = "system-keyring")]
    fn entry_delete(&self, key: &str) -> Result<(), String> {
        #[cfg(test)]
        {
            if self.fake {
                return fake_backend::delete(&self.service, key);
            }
        }
        let entry = keyring_entry(&self.service, key).map_err(|e| format!("keyring entry: {e}"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) if is_keyring_availability_error(&e.to_string()) => {
                Err(format!("keyring unavailable: {e}"))
            }
            Err(e) => Err(format!("keyring delete: {e}")),
        }
    }

    /// Read the raw **legacy** blob bytes. `Ok(None)` = not found (the normal
    /// state after migration).
    #[cfg(feature = "system-keyring")]
    fn read_blob_raw(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.entry_get(BLOB_KEY)?.map(String::into_bytes))
    }

    /// Read the credential holding the secret named `key`, bypassing the cache.
    #[cfg(feature = "system-keyring")]
    fn read_secret_raw(&self, key: &str) -> Result<Option<String>, String> {
        self.entry_get(key)
    }

    // ── Name index ────────────────────────────────────────────────────────

    /// Read every indexed key name plus the chunk count currently on disk.
    ///
    /// A missing or corrupt chunk is skipped rather than fatal: the index is
    /// only an enumeration aid, and `load()` addresses credentials by name.
    #[cfg(feature = "system-keyring")]
    fn read_index_raw(&self) -> Result<(Vec<String>, usize), String> {
        let Some(head_raw) = self.entry_get(INDEX_KEY)? else {
            return Ok((Vec::new(), 0));
        };
        let head: IndexChunk =
            serde_json::from_str(&head_raw).map_err(|e| format!("index json: {e}"))?;
        let count = head.chunks.unwrap_or(1).clamp(1, MAX_INDEX_CHUNKS);
        let mut names = head.names;
        for i in 1..count {
            if let Some(raw) = self.entry_get(&index_chunk_key(i))? {
                if let Ok(chunk) = serde_json::from_str::<IndexChunk>(&raw) {
                    names.extend(chunk.names);
                }
            }
        }
        names.sort();
        names.dedup();
        Ok((names, count))
    }

    /// Replace the index with `names`, splitting it across as many chunks as
    /// the per-credential size cap requires.
    ///
    /// Tail chunks are written before chunk 0 so the count chunk 0 declares is
    /// never larger than the data actually present. `prev_chunks` is the count
    /// read alongside the names, used to drop chunks a shrinking index leaves
    /// behind.
    #[cfg(feature = "system-keyring")]
    fn write_index(&self, names: &[String], prev_chunks: usize) -> Result<(), String> {
        let mut sorted: Vec<String> = names.to_vec();
        sorted.sort();
        sorted.dedup();

        let mut chunks: Vec<Vec<String>> = vec![Vec::new()];
        let mut used = 0usize;
        for name in sorted {
            // +4 covers the two quotes, the comma and JSON escaping slack.
            let cost = name.chars().count() + 4;
            if used + cost > INDEX_CHUNK_CHARS
                && !chunks.last().map(Vec::is_empty).unwrap_or(true)
                && chunks.len() < MAX_INDEX_CHUNKS
            {
                chunks.push(Vec::new());
                used = 0;
            }
            used += cost;
            if let Some(last) = chunks.last_mut() {
                last.push(name);
            }
        }
        let n_chunks = chunks.len();

        for i in (1..n_chunks).rev() {
            let body = serde_json::to_string(&IndexChunk {
                chunks: None,
                names: chunks[i].clone(),
            })
            .map_err(|e| format!("index serialize: {e}"))?;
            self.entry_set(&index_chunk_key(i), &body)?;
        }
        let head = serde_json::to_string(&IndexChunk {
            chunks: Some(n_chunks),
            names: chunks[0].clone(),
        })
        .map_err(|e| format!("index serialize: {e}"))?;
        self.entry_set(INDEX_KEY, &head)?;

        for i in n_chunks..prev_chunks.min(MAX_INDEX_CHUNKS) {
            let _ = self.entry_delete(&index_chunk_key(i));
        }
        Ok(())
    }

    /// Add `keys` to the index if any are missing. No write when all present.
    #[cfg(feature = "system-keyring")]
    fn index_insert(&self, keys: &[String]) -> Result<(), String> {
        let (mut names, prev) = self.read_index_raw()?;
        let mut changed = false;
        for key in keys {
            if !names.iter().any(|n| n == key) {
                names.push(key.clone());
                changed = true;
            }
        }
        if changed {
            self.write_index(&names, prev)?;
        }
        Ok(())
    }

    /// Drop `key` from the index. No write when it was not listed.
    #[cfg(feature = "system-keyring")]
    fn index_remove(&self, key: &str) -> Result<(), String> {
        let (mut names, prev) = self.read_index_raw()?;
        let before = names.len();
        names.retain(|n| n != key);
        if names.len() != before {
            self.write_index(&names, prev)?;
        }
        Ok(())
    }

    // ── One-time blob → per-credential migration ──────────────────────────

    /// Move every secret out of the legacy single-blob entry into its own
    /// credential, exactly once, without ever producing a partial state.
    ///
    /// Sequence, under the interprocess advisory lock:
    /// 1. Read the blob. Absent → nothing to do, mark migrated.
    /// 2. For each secret: write its own credential, then **read it back from
    ///    the OS and verify** the value matches.
    /// 3. Add every name to the index.
    /// 4. Only then delete the blob entry.
    ///
    /// If any single write, read-back, verify or index write fails, every
    /// credential this attempt touched is restored to its prior value (deleted
    /// when there was none), the blob is left **intact**, and `migrated` stays
    /// `false` so the next access retries. Reads prefer the per-credential
    /// value and fall back to the blob, so an aborted attempt is invisible.
    #[cfg(feature = "system-keyring")]
    fn ensure_migrated(&self) -> Result<(), String> {
        {
            let guard = self.migrated.lock().unwrap_or_else(|e| e.into_inner());
            if *guard {
                return Ok(());
            }
        }

        // Cross-process exclusion: another Buzz process may be migrating the
        // same service right now. Note this must not be held by our caller —
        // `flock(2)` on a second fd in the same process would self-deadlock.
        let _lock = acquire_blob_lock(&self.service)?;
        {
            let guard = self.migrated.lock().unwrap_or_else(|e| e.into_inner());
            if *guard {
                return Ok(());
            }
        }

        let Some(bytes) = self.read_blob_raw()? else {
            // No legacy blob: fresh install, or another process already
            // finished the migration.
            *self.migrated.lock().unwrap_or_else(|e| e.into_inner()) = true;
            return Ok(());
        };
        let json = String::from_utf8(bytes).map_err(|e| format!("blob utf8: {e}"))?;
        let map = serde_json::from_str::<HashMap<String, String>>(&json)
            .map_err(|e| format!("blob json: {e}"))?;

        // (name, value before this attempt touched it) — the rollback journal.
        let mut touched: Vec<(String, Option<String>)> = Vec::new();
        for (key, value) in &map {
            let prior = match self.read_secret_raw(key) {
                Ok(p) => p,
                Err(e) => {
                    self.rollback_migration(&touched);
                    return Err(format!("blob migration: read {key}: {e}"));
                }
            };
            if prior.as_deref() == Some(value.as_str()) {
                continue; // already migrated by an earlier attempt
            }
            if let Err(e) = self.entry_set(key, value) {
                self.rollback_migration(&touched);
                return Err(format!("blob migration: write {key}: {e}"));
            }
            touched.push((key.clone(), prior));
            // Read back from the OS — proves the round-trip, not just that an
            // in-process buffer was updated.
            match self.read_secret_raw(key) {
                Ok(Some(got)) if got == *value => {}
                other => {
                    self.rollback_migration(&touched);
                    return Err(format!(
                        "blob migration: read-back verify failed for {key}: {other:?}"
                    ));
                }
            }
        }

        let names: Vec<String> = map.keys().cloned().collect();
        if let Err(e) = self.index_insert(&names) {
            self.rollback_migration(&touched);
            return Err(format!("blob migration: index: {e}"));
        }

        // Every secret is durable and verified in its own credential; the blob
        // is now redundant. A failure here is not data loss — the blob is
        // simply retried on the next process start.
        if let Err(e) = self.entry_delete(BLOB_KEY) {
            eprintln!("buzz-desktop: secret_store: blob migrated but not deleted: {e}");
        }

        {
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let cached = guard.get_or_insert_with(HashMap::new);
            for (key, value) in map {
                cached.insert(key, value);
            }
        }
        *self.migrated.lock().unwrap_or_else(|e| e.into_inner()) = true;
        Ok(())
    }

    /// Undo the credential writes journalled in `touched`, newest first.
    /// Best-effort by construction: the blob is still intact, so anything that
    /// cannot be restored here is re-derived on the next migration attempt.
    #[cfg(feature = "system-keyring")]
    fn rollback_migration(&self, touched: &[(String, Option<String>)]) {
        for (key, prior) in touched.iter().rev() {
            let _ = match prior {
                Some(old) => self.entry_set(key, old),
                None => self.entry_delete(key),
            };
        }
    }

    /// Run [`Self::ensure_migrated`] but let a failure fall through to the
    /// caller's own OS access, which produces the more precise error.
    #[cfg(feature = "system-keyring")]
    fn try_migrate(&self) {
        if let Err(e) = self.ensure_migrated() {
            eprintln!("buzz-desktop: secret_store: blob migration deferred: {e}");
        }
    }

    /// Write `entries` as one credential each, then record their names in the
    /// index, then advance the cache. Replaces the old `mutate_blob`.
    ///
    /// **Cross-process safety**: acquires the same exclusive advisory lock
    /// (`flock(2)` on Unix, a named kernel mutex on Windows) the blob path used,
    /// keyed by service name, so a concurrent process cannot interleave an
    /// index read-modify-write with ours. The index is always re-read fresh
    /// inside the lock, never from cache, so another process's names are never
    /// dropped. Per-secret values no longer share a record at all, so a write
    /// here cannot clobber a secret it does not name.
    ///
    /// **Idempotent**: a credential whose stored value already equals the new
    /// one is not rewritten. On macOS the legacy `SecKeychain` API treats a
    /// write as a distinct ACL operation from the "Always Allow"-ed read, so
    /// skipping no-op writes still avoids the prompt that would otherwise fire
    /// when saving an agent whose model changed but whose key did not.
    ///
    /// **Cache honesty**: only keys that reached the OS are added to the cache,
    /// so a failed write is never visible to a later `load()`.
    ///
    /// Deadlock-free: `ensure_migrated` takes the same lock, so it must run to
    /// completion *before* the lock is acquired here, not inside it.
    #[cfg(feature = "system-keyring")]
    fn store_secrets(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        self.ensure_migrated()?;
        let _lock = acquire_blob_lock(&self.service)?;

        let mut written: Vec<String> = Vec::new();
        let mut first_err: Option<String> = None;
        for (key, value) in entries {
            match self.read_secret_raw(key) {
                // Already durable with this exact value — no write, no prompt.
                Ok(Some(cur)) if cur == *value => written.push(key.clone()),
                Ok(_) => match self.entry_set(key, value) {
                    Ok(()) => written.push(key.clone()),
                    Err(e) => {
                        first_err.get_or_insert(e);
                    }
                },
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }

        if !written.is_empty() {
            if let Err(e) = self.index_insert(&written) {
                first_err.get_or_insert(e);
            }
        }

        {
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let map = guard.get_or_insert_with(HashMap::new);
            for key in &written {
                if let Some(value) = entries.get(key) {
                    map.insert(key.clone(), value.clone());
                }
            }
        }

        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Value for `key` if this process has already read or written it. Never
    /// authoritative for absence — a miss must fall through to the OS.
    #[cfg(feature = "system-keyring")]
    fn cached(&self, key: &str) -> Option<String> {
        let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().and_then(|m| m.get(key).cloned())
    }

    /// Record `key`→`value` as read from the OS.
    #[cfg(feature = "system-keyring")]
    fn cache_put(&self, key: &str, value: &str) {
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get_or_insert_with(HashMap::new)
            .insert(key.to_string(), value.to_string());
    }

    /// Probe whether `key` exists and whether the backend is reachable.
    pub fn probe(&self, key: &str) -> KeyringProbe {
        #[cfg(feature = "system-keyring")]
        {
            self.try_migrate();
            if self.cached(key).is_some() {
                return KeyringProbe::Present;
            }
            match self.read_secret_raw(key) {
                Ok(Some(_)) => KeyringProbe::Present,
                // No credential of its own — check the shapes an unfinished
                // migration can leave behind (legacy blob, then old DPK items)
                // so callers that gate `load()` on `Present` still fire.
                Ok(None) => match self.blob_value(key) {
                    Ok(Some(_)) => KeyringProbe::Present,
                    Ok(None) => self.probe_legacy_key(key),
                    Err(_) => KeyringProbe::Unreachable, // corrupt blob — fail closed
                },
                Err(e) if is_keyring_availability_error(&e) => KeyringProbe::Unreachable,
                Err(_) => KeyringProbe::Unreachable,
            }
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            KeyringProbe::Unreachable
        }
    }

    /// Value for `key` still sitting in the legacy blob, if the blob survives
    /// (i.e. a migration attempt has not yet succeeded). This is what keeps an
    /// aborted migration invisible to readers.
    #[cfg(feature = "system-keyring")]
    fn blob_value(&self, key: &str) -> Result<Option<String>, String> {
        let Some(bytes) = self.read_blob_raw()? else {
            return Ok(None);
        };
        let json = String::from_utf8(bytes).map_err(|e| format!("blob utf8: {e}"))?;
        let map = serde_json::from_str::<HashMap<String, String>>(&json)
            .map_err(|e| format!("blob json: {e}"))?;
        Ok(map.get(key).cloned())
    }

    /// Check old per-key DPK entries for `key`. Used by `probe()` once the
    /// key's own credential and the legacy blob have both come up empty.
    #[cfg(all(feature = "system-keyring", target_os = "macos"))]
    fn probe_legacy_key(&self, key: &str) -> KeyringProbe {
        match generic_password(dpk_opts(&self.service, key)) {
            Ok(_) => KeyringProbe::Present,
            Err(ref e) if is_not_found(e) => KeyringProbe::ReachableButEmpty,
            Err(ref e) if is_dpk_unavailable(e) => KeyringProbe::ReachableButEmpty,
            Err(ref e) if is_keyring_availability_error(&e.to_string()) => {
                KeyringProbe::Unreachable
            }
            Err(_) => KeyringProbe::ReachableButEmpty,
        }
    }

    #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
    fn probe_legacy_key(&self, _key: &str) -> KeyringProbe {
        // No DPK off macOS: the key's own credential is the only shape, and
        // `probe` already found it absent.
        KeyringProbe::ReachableButEmpty
    }

    /// Load the secret for `key`. `Ok(None)` when there is no entry; `Err` only
    /// when the backend errored in a way that is not "missing".
    ///
    /// Resolution order: in-process cache → the key's own credential → the
    /// legacy blob (only reachable while a migration attempt is outstanding) →
    /// the old per-key DPK item on macOS, which is migrated in place and
    /// deleted. Pre-blob installs are handled for free: their per-key `keyring`
    /// entries live at exactly the address this format uses.
    pub fn load(&self, key: &str) -> Result<Option<String>, String> {
        #[cfg(feature = "system-keyring")]
        {
            self.try_migrate();
            if let Some(value) = self.cached(key) {
                return Ok(Some(value));
            }
            if let Some(value) = self.read_secret_raw(key)? {
                self.cache_put(key, &value);
                return Ok(Some(value));
            }
            // A migration that aborted leaves the blob authoritative.
            if let Some(value) = self.blob_value(key)? {
                return Ok(Some(value));
            }
            self.migrate_legacy_key(key)
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Read every stored secret without any legacy-migration side effects.
    ///
    /// Returns the full key→value map, `Ok(None)` when nothing has ever been
    /// stored for this service, and `Err` only when the backend is unavailable.
    /// Never calls `migrate_legacy_key`.
    ///
    /// The key set comes from the index, unioned with any names still in an
    /// unmigrated blob and with `"identity"` (which predates the index and can
    /// exist as a bare credential on a pre-blob install).
    pub fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String> {
        #[cfg(feature = "system-keyring")]
        {
            self.try_migrate();

            let (mut names, _) = self.read_index_raw()?;
            let indexed = !names.is_empty();

            // A blob only survives here when a migration attempt aborted; its
            // contents are still the authoritative copy for those keys.
            let blob: HashMap<String, String> = match self.read_blob_raw()? {
                Some(bytes) => {
                    let json = String::from_utf8(bytes).map_err(|e| format!("blob utf8: {e}"))?;
                    serde_json::from_str(&json).map_err(|e| format!("blob json: {e}"))?
                }
                None => HashMap::new(),
            };
            for name in blob.keys() {
                if !names.iter().any(|n| n == name) {
                    names.push(name.clone());
                }
            }
            if !names.iter().any(|n| n == "identity") {
                names.push("identity".to_string());
            }

            let mut map = HashMap::new();
            for name in names {
                match self.read_secret_raw(&name)? {
                    Some(value) => {
                        map.insert(name, value);
                    }
                    // Indexed but no credential: an index entry outliving its
                    // secret is a benign inconsistency, not a read failure.
                    None => {
                        if let Some(value) = blob.get(&name) {
                            map.insert(name, value.clone());
                        }
                    }
                }
            }

            if map.is_empty() && !indexed {
                return Ok(None); // nothing has ever been stored
            }
            {
                let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                let cached = guard.get_or_insert_with(HashMap::new);
                for (k, v) in &map {
                    cached.insert(k.clone(), v.clone());
                }
            }
            Ok(Some(map))
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Store every entry in `entries`, one credential each.
    ///
    /// Entries that already exist are overwritten; keys not present in
    /// `entries` are left untouched. A credential whose stored value already
    /// equals the new one is not rewritten, so a no-op save costs no write.
    pub fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        #[cfg(feature = "system-keyring")]
        {
            self.store_secrets(entries)
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = entries;
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// On first launch after upgrading from the per-key DPK format, read the
    /// old DPK entry for `key`, write it into the key's own credential, and
    /// delete the old item. Returns `Ok(None)` when no old entry exists.
    ///
    /// Also handles a one-time migration from the DPK blob format written by
    /// #1267 (before the dev/release split was fixed). Anyone who ran main
    /// while #1267 was present has a DPK blob instead of per-key entries; this
    /// reads it, lifts every key it holds into its own credential, and deletes
    /// the DPK blob.
    ///
    /// The pre-#1264 `keyring`-crate per-key entries need no handling: they sit
    /// at exactly the address the current format uses, so `load` already found
    /// them before reaching here.
    #[cfg(all(feature = "system-keyring", target_os = "macos"))]
    fn migrate_legacy_key(&self, key: &str) -> Result<Option<String>, String> {
        // One-time migration: check for a DPK blob (key = BLOB_KEY = "secrets")
        // written by #1267 before the dev/release split was fixed.
        match generic_password(dpk_opts(&self.service, BLOB_KEY)) {
            Ok(bytes) => {
                let json = String::from_utf8(bytes).map_err(|e| format!("dpk blob utf8: {e}"))?;
                let dpk_map = serde_json::from_str::<HashMap<String, String>>(&json)
                    .map_err(|e| format!("dpk blob json: {e}"))?;
                // Lift each key into its own credential, never overwriting one
                // that already exists (the existing copy is the newer one).
                let mut fresh = HashMap::new();
                for (k, v) in &dpk_map {
                    if self.read_secret_raw(k)?.is_none() {
                        fresh.insert(k.clone(), v.clone());
                    }
                }
                if !fresh.is_empty() {
                    self.store_secrets(&fresh)?;
                }
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
                // Write into the key's own credential.
                self.store(key, &value)?;
                // Best-effort cleanup of the old per-key entry.
                let _ = delete_generic_password_options(dpk_opts(&self.service, key));
                Ok(Some(value))
            }
            Err(ref e) if is_not_found(e) => Ok(None),
            Err(ref e) if is_dpk_unavailable(e) => Ok(None),
            Err(e) => Err(format!("keyring get: {e}")),
        }
    }

    #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
    fn migrate_legacy_key(&self, _key: &str) -> Result<Option<String>, String> {
        // No DPK off macOS, and the pre-#1264 per-key `keyring` entries share
        // the current format's address — `load` has already checked there.
        Ok(None)
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
            if self.read_secret_raw(key)?.as_deref() == Some(expected) {
                return Ok(true);
            }
            // A key still awaiting migration is durable in the blob.
            Ok(self.blob_value(key)?.as_deref() == Some(expected))
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
            let mut one = HashMap::with_capacity(1);
            one.insert(key.to_string(), value.to_string());
            self.store_secrets(&one)
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = (key, value);
            Err("system-keyring feature disabled".to_string())
        }
    }

    /// Delete every secret credential for this service, plus every legacy shape
    /// that could resurrect an identity on next boot.
    ///
    /// Order of operations:
    /// 1. Collect every key name: the index, plus any names left in an
    ///    unmigrated blob, plus `"identity"` unconditionally.
    /// 2. Delete legacy per-key DPK entries for every key + the DPK blob itself.
    /// 3. Delete each key's own credential.
    /// 4. Delete the legacy blob entry and every index chunk.
    /// 5. Clear the in-memory cache.
    ///
    /// This is the correct wipe path for sign-out: the old `delete_all` skipped
    /// steps 1–3 so stale per-key entries could be re-imported on the next
    /// launch via `migrate_legacy_key`. This method prevents that resurrection.
    pub fn delete_all_with_legacy_cleanup(&self) -> Result<(), String> {
        #[cfg(feature = "system-keyring")]
        {
            let _lock = acquire_blob_lock(&self.service)?;

            // Step 1: every name we know about (best-effort; errors = empty set).
            let (mut all_keys, index_chunks) = self.read_index_raw().unwrap_or_default();
            if let Ok(Some(bytes)) = self.read_blob_raw() {
                let json = String::from_utf8(bytes).unwrap_or_default();
                if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&json) {
                    for key in map.into_keys() {
                        if !all_keys.contains(&key) {
                            all_keys.push(key);
                        }
                    }
                }
            }
            // Always include "identity" even when nothing is indexed — it may
            // exist only as a bare credential from a pre-blob install.
            if !all_keys.contains(&"identity".to_string()) {
                all_keys.push("identity".to_string());
            }

            // Steps 2 & 3: delete the DPK entry and the credential for every key.
            for key in &all_keys {
                #[cfg(target_os = "macos")]
                {
                    match delete_generic_password_options(dpk_opts(&self.service, key)) {
                        Ok(()) => {}
                        Err(ref e) if is_not_found(e) => {}
                        Err(ref e) if is_dpk_unavailable(e) => {}
                        Err(e) => return Err(format!("dpk per-key delete {key}: {e}")),
                    }
                }
                self.entry_delete(key)
                    .map_err(|e| format!("keyring per-key delete {key}: {e}"))?;
            }
            // Step 2 (cont.): also delete the legacy DPK blob written by #1267.
            #[cfg(target_os = "macos")]
            {
                match delete_generic_password_options(dpk_opts(&self.service, BLOB_KEY)) {
                    Ok(()) => {}
                    Err(ref e) if is_not_found(e) => {}
                    Err(ref e) if is_dpk_unavailable(e) => {}
                    Err(e) => return Err(format!("dpk blob delete: {e}")),
                }
            }

            // Step 4: delete the legacy blob entry and every index chunk.
            self.entry_delete(BLOB_KEY)
                .map_err(|e| format!("keyring blob delete: {e}"))?;
            for i in 0..index_chunks.clamp(1, MAX_INDEX_CHUNKS) {
                self.entry_delete(&index_chunk_key(i))
                    .map_err(|e| format!("keyring index delete {i}: {e}"))?;
            }

            // Step 5: clear the in-memory cache. The blob is gone, so there is
            // nothing left to migrate either.
            *self.migrated.lock().unwrap_or_else(|e| e.into_inner()) = true;
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            *guard = None;
            Ok(())
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            Ok(()) // No-op: no keyring, nothing to delete.
        }
    }

    /// Verify no identity-bearing keychain entry survives in any shape
    /// that `load("identity")` → `migrate_legacy_key` can consume:
    /// main blob, DPK blob (`BLOB_KEY`), and per-key `"identity"`.
    ///
    /// Returns `true` when all three shapes are absent (or inaccessible in an
    /// expected way), `false` when any entry is found or the keychain is
    /// unavailable (fail-closed).
    pub fn verify_fully_wiped(&self) -> bool {
        #[cfg(feature = "system-keyring")]
        {
            // 1. Main blob must be absent.
            match self.read_blob_raw() {
                Ok(None) => {}
                Ok(Some(_)) => return false,
                Err(_) => return false,
            }
            // 2. The "identity" credential itself must be absent. Only an
            //    explicit "no entry" is proof of absence; any error fails closed.
            match self.entry_get("identity") {
                Ok(None) => {}
                Ok(Some(_)) => return false,
                Err(_) => return false,
            }
            // 3. DPK blob (macOS only).
            #[cfg(target_os = "macos")]
            {
                match generic_password(dpk_opts(&self.service, BLOB_KEY)) {
                    Err(ref e) if is_not_found(e) => {}
                    // dpk-unavailable is symmetric with load(): if load() can't
                    // consume DPK in this state, a surviving entry is harmless.
                    Err(ref e) if is_dpk_unavailable(e) => {}
                    Ok(_) => return false,
                    // Any other error → fail closed (not proof of absence).
                    Err(_) => return false,
                }
                // 4. Per-key DPK "identity" (macOS only).
                match generic_password(dpk_opts(&self.service, "identity")) {
                    Err(ref e) if is_not_found(e) => {}
                    // dpk-unavailable: symmetric with load() — if load() can't
                    // read DPK, a surviving entry can't resurrect identity.
                    Err(ref e) if is_dpk_unavailable(e) => {}
                    Ok(_) => return false,
                    // Any other error → fail closed.
                    Err(_) => return false,
                }
            }
            true
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            true // No keyring = nothing to verify.
        }
    }

    /// Delete the secret for `key`. A missing entry is not an error.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        #[cfg(feature = "system-keyring")]
        {
            // Migrate first: a key still sitting in the blob would otherwise
            // survive the delete and be resurrected on the next load.
            self.try_migrate();
            let _lock = acquire_blob_lock(&self.service)?;

            self.entry_delete(key)?;
            self.index_remove(key)?;
            {
                let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(map) = guard.as_mut() {
                    map.remove(key);
                }
            }
            // Best-effort: also delete any old DPK entry for this key to
            // prevent resurrection on the next probe/load (migration path).
            #[cfg(target_os = "macos")]
            let _ = delete_generic_password_options(dpk_opts(&self.service, key));
            Ok(())
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = key;
            Err("system-keyring feature disabled".to_string())
        }
    }
}

/// In-process stand-in for the OS credential store, used by the unit tests so
/// they never touch (or depend on) a real keychain.
///
/// It reproduces the constraint that motivated per-credential storage: a write
/// whose value exceeds Windows' `CRED_MAX_CREDENTIAL_BLOB_SIZE` is rejected
/// before anything is stored, exactly as `keyring`'s Windows backend rejects it
/// ahead of `CredWriteW`. Sizes are measured over the UTF-16 encoding the
/// backend actually writes.
#[cfg(all(test, feature = "system-keyring"))]
mod fake_backend {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Mutex, OnceLock};

    /// `CRED_MAX_CREDENTIAL_BLOB_SIZE` from `wincred.h`.
    pub const MAX_BLOB_BYTES: usize = 2560;

    type Creds = HashMap<(String, String), String>;

    fn creds() -> &'static Mutex<Creds> {
        static CREDS: OnceLock<Mutex<Creds>> = OnceLock::new();
        CREDS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn failures() -> &'static Mutex<HashSet<(String, String)>> {
        static FAILURES: OnceLock<Mutex<HashSet<(String, String)>>> = OnceLock::new();
        FAILURES.get_or_init(|| Mutex::new(HashSet::new()))
    }

    /// UTF-16 byte length, i.e. what the Windows backend measures.
    pub fn blob_bytes(value: &str) -> usize {
        value.encode_utf16().count() * 2
    }

    pub fn get(service: &str, key: &str) -> Result<Option<String>, String> {
        let guard = creds().lock().unwrap_or_else(|e| e.into_inner());
        Ok(guard.get(&(service.to_string(), key.to_string())).cloned())
    }

    pub fn set(service: &str, key: &str, value: &str) -> Result<(), String> {
        let id = (service.to_string(), key.to_string());
        if failures()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&id)
        {
            return Err("keyring write: injected backend failure".to_string());
        }
        if blob_bytes(value) > MAX_BLOB_BYTES {
            return Err(format!(
                "keyring write: credential blob too long ({} bytes > {MAX_BLOB_BYTES})",
                blob_bytes(value)
            ));
        }
        creds()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, value.to_string());
        Ok(())
    }

    pub fn delete(service: &str, key: &str) -> Result<(), String> {
        creds()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(service.to_string(), key.to_string()));
        Ok(())
    }

    // ── helpers for tests ─────────────────────────────────────────────────

    /// Write a credential directly, bypassing the failure injection.
    pub fn seed(service: &str, key: &str, value: &str) {
        creds()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((service.to_string(), key.to_string()), value.to_string());
    }

    /// Read a credential directly, bypassing `SecretStore` entirely.
    pub fn raw(service: &str, key: &str) -> Option<String> {
        creds()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(service.to_string(), key.to_string()))
            .cloned()
    }

    /// Every credential name that exists under `service`, sorted.
    pub fn names_for(service: &str) -> Vec<String> {
        let guard = creds().lock().unwrap_or_else(|e| e.into_inner());
        let mut names: Vec<String> = guard
            .keys()
            .filter(|(s, _)| s == service)
            .map(|(_, k)| k.clone())
            .collect();
        names.sort();
        names
    }

    /// Make every write to `service`/`key` fail, simulating a denied prompt or
    /// a transient backend outage part-way through a migration.
    pub fn fail_writes_for(service: &str, key: &str) {
        failures()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((service.to_string(), key.to_string()));
    }

    pub fn clear_failures() {
        failures().lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Drop all state for `service`.
    pub fn reset(service: &str) {
        creds()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(s, _), _| s != service);
        failures()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(s, _)| s != service);
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
                // A pre-seeded cache stands in for "already read from the OS",
                // so there is nothing left to migrate.
                migrated: Mutex::new(true),
                fake: true,
            }
        }

        /// Store backed by the in-process fake credential backend.
        fn with_fake(service: &str) -> Self {
            SecretStore {
                service: service.to_string(),
                cache: Mutex::new(None),
                migrated: Mutex::new(false),
                fake: true,
            }
        }
    }

    /// Serialize `entries` the way the legacy blob stored them.
    fn blob_json(entries: &HashMap<String, String>) -> String {
        serde_json::to_string(entries).unwrap()
    }

    /// A realistically-sized agent key name/value pair (`agent:<npub>` →
    /// `nsec…`), the shape that made the blob overflow.
    fn agent_pair(i: usize) -> (String, String) {
        (format!("agent:npub1{:0>58}", i), format!("nsec1{:0>58}", i))
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

    #[ignore = "requires real OS keychain (run locally)"]
    #[test]
    fn delete_all_with_legacy_cleanup_removes_per_key_identity() {
        let svc = "buzz-test-delete-all-legacy";
        let key = "identity";
        let value = "nsec1legacytest";

        // Seed a legacy per-key entry (old format, pre-blob migration).
        let entry = keyring_entry(svc, key).unwrap();
        entry.set_password(value).unwrap();

        // Also seed a blob with a different key to exercise the full path.
        let store = SecretStore::keyring(svc);
        store.store("agent:abc123", "nsec1agent").unwrap();

        // Legacy per-key identity should be discoverable via probe.
        let store2 = SecretStore::keyring(svc);
        assert_eq!(store2.probe(key), KeyringProbe::Present);

        // Wipe everything via the sign-out path.
        store2.delete_all_with_legacy_cleanup().unwrap();

        // Fresh store — neither the blob nor the per-key entry should remain.
        let store3 = SecretStore::keyring(svc);
        assert_eq!(
            store3.probe(key),
            KeyringProbe::ReachableButEmpty,
            "per-key identity must not survive delete_all_with_legacy_cleanup"
        );
        assert_eq!(
            store3.load(key).unwrap(),
            None,
            "load must not resurrect the legacy per-key identity"
        );
        // Agent key should also be gone.
        assert_eq!(store3.load("agent:abc123").unwrap(), None);
    }

    // ── Blob → per-credential migration (fake backend, no OS keychain) ─────

    #[test]
    fn migration_moves_every_blob_secret_into_its_own_credential() {
        let svc = "buzz-test-fake-migrate-basic";
        fake_backend::reset(svc);

        let mut blob = HashMap::new();
        blob.insert("identity".to_string(), "nsec1identityvalue".to_string());
        blob.insert("agent:aaa".to_string(), "nsec1aaa".to_string());
        blob.insert("agent:bbb".to_string(), "nsec1bbb".to_string());
        fake_backend::seed(svc, BLOB_KEY, &blob_json(&blob));

        // First access triggers the migration.
        let store = SecretStore::with_fake(svc);
        assert_eq!(
            store.load("identity").unwrap(),
            Some("nsec1identityvalue".to_string())
        );

        for (key, value) in &blob {
            assert_eq!(
                fake_backend::raw(svc, key).as_deref(),
                Some(value.as_str()),
                "{key} must now live in its own credential"
            );
        }
        assert_eq!(
            fake_backend::raw(svc, BLOB_KEY),
            None,
            "the legacy blob must be deleted once every secret is written and verified"
        );
        assert!(
            fake_backend::raw(svc, INDEX_KEY).is_some(),
            "the index must list the migrated names"
        );

        // A cold store sees the full set, and migrating again is a no-op.
        let reader = SecretStore::with_fake(svc);
        let all = reader.load_all_readonly().unwrap().unwrap();
        assert_eq!(all, blob, "every secret must survive the migration");
        reader.ensure_migrated().unwrap();
        assert_eq!(reader.load("agent:bbb").unwrap(), Some("nsec1bbb".into()));

        fake_backend::reset(svc);
    }

    #[test]
    fn migration_rolls_back_and_leaves_blob_intact_when_one_write_fails() {
        let svc = "buzz-test-fake-migrate-rollback";
        fake_backend::reset(svc);

        let mut blob = HashMap::new();
        blob.insert("identity".to_string(), "nsec1identityvalue".to_string());
        for i in 0..6 {
            let (key, value) = agent_pair(i);
            blob.insert(key, value);
        }
        let original = blob_json(&blob);
        fake_backend::seed(svc, BLOB_KEY, &original);

        // One secret's write fails part-way through (denied prompt / outage).
        let (doomed, _) = agent_pair(3);
        fake_backend::fail_writes_for(svc, &doomed);

        let store = SecretStore::with_fake(svc);
        let err = store.ensure_migrated().unwrap_err();
        assert!(
            err.contains(&doomed),
            "the error must name the secret that failed: {err}"
        );

        assert_eq!(
            fake_backend::raw(svc, BLOB_KEY).as_deref(),
            Some(original.as_str()),
            "the blob must be left byte-for-byte intact so the migration can retry"
        );
        assert_eq!(
            fake_backend::names_for(svc),
            vec![BLOB_KEY.to_string()],
            "rollback must leave no partially-migrated credentials and no index"
        );

        // Reads still resolve, out of the surviving blob — the abort is invisible.
        assert_eq!(
            store.load("identity").unwrap(),
            Some("nsec1identityvalue".to_string())
        );
        let (key5, value5) = agent_pair(5);
        assert_eq!(store.load(&key5).unwrap(), Some(value5));
        assert_eq!(store.probe(&doomed), KeyringProbe::Present);

        // Once the backend recovers, the retry completes.
        fake_backend::clear_failures();
        store.ensure_migrated().unwrap();
        assert_eq!(fake_backend::raw(svc, BLOB_KEY), None);
        let (key3, value3) = agent_pair(3);
        assert_eq!(
            fake_backend::raw(svc, &key3).as_deref(),
            Some(value3.as_str())
        );
        assert_eq!(
            SecretStore::with_fake(svc)
                .load_all_readonly()
                .unwrap()
                .unwrap(),
            blob
        );

        fake_backend::reset(svc);
    }

    #[test]
    fn migration_rollback_restores_prior_credential_values() {
        // The previous test aborts at whatever point `HashMap` iteration puts
        // the doomed key. This one fails the *index* write, which runs only
        // after every secret has been written and verified, so the rollback
        // journal is guaranteed full — including one credential that already
        // existed and must be put back rather than deleted.
        let svc = "buzz-test-fake-migrate-rollback-restore";
        fake_backend::reset(svc);

        let mut blob = HashMap::new();
        blob.insert("identity".to_string(), "nsec1new-identity".to_string());
        for i in 0..4 {
            let (key, value) = agent_pair(i);
            blob.insert(key, value);
        }
        let original = blob_json(&blob);
        fake_backend::seed(svc, BLOB_KEY, &original);
        fake_backend::seed(svc, "identity", "nsec1stale-identity");
        fake_backend::fail_writes_for(svc, INDEX_KEY);

        let store = SecretStore::with_fake(svc);
        let err = store.ensure_migrated().unwrap_err();
        assert!(
            err.contains("index"),
            "the index write must be the failure: {err}"
        );

        assert_eq!(
            fake_backend::raw(svc, BLOB_KEY).as_deref(),
            Some(original.as_str()),
            "the blob must survive a failure at the index step too"
        );
        assert_eq!(
            fake_backend::raw(svc, "identity").as_deref(),
            Some("nsec1stale-identity"),
            "a credential that existed before the migration must be restored to \
             its prior value, not deleted"
        );
        for i in 0..4 {
            let (key, _) = agent_pair(i);
            assert_eq!(
                fake_backend::raw(svc, &key),
                None,
                "{key} was created by this attempt and must be rolled back"
            );
        }
        assert_eq!(
            fake_backend::names_for(svc),
            vec!["identity".to_string(), BLOB_KEY.to_string()],
            "nothing else may be left behind"
        );

        // The retry then completes, and the blob's copy wins over the stale one.
        fake_backend::clear_failures();
        store.ensure_migrated().unwrap();
        assert_eq!(
            fake_backend::raw(svc, "identity").as_deref(),
            Some("nsec1new-identity")
        );
        assert_eq!(fake_backend::raw(svc, BLOB_KEY), None);
        assert_eq!(
            SecretStore::with_fake(svc)
                .load_all_readonly()
                .unwrap()
                .unwrap(),
            blob
        );

        fake_backend::reset(svc);
    }

    #[test]
    fn sixteen_agents_plus_identity_all_reach_the_credential_store() {
        // The regression this whole change exists for: as one JSON blob,
        // identity + 8 agents already filled ~2380 of the 2560 bytes Windows
        // allows in a credential, so agent 9 onward could never be written at
        // all and silently fell back to inline plaintext. One credential per
        // secret removes the shared budget.
        let svc = "buzz-test-fake-sixteen-agents";
        fake_backend::reset(svc);

        let store = SecretStore::with_fake(svc);
        let identity = format!("nsec1{:0>58}", 999);
        store.store("identity", &identity).unwrap();

        let mut expected = HashMap::new();
        expected.insert("identity".to_string(), identity);
        for i in 0..16 {
            let (key, value) = agent_pair(i);
            store
                .store(&key, &value)
                .unwrap_or_else(|e| panic!("agent {i} must reach the credential store: {e}"));
            expected.insert(key, value);
        }

        // Cold store: every secret round-trips out of the OS, not the cache.
        let reader = SecretStore::with_fake(svc);
        for (key, value) in &expected {
            assert_eq!(
                reader.load(key).unwrap().as_deref(),
                Some(value.as_str()),
                "{key} must load back"
            );
        }
        assert_eq!(
            reader.load_all_readonly().unwrap().unwrap(),
            expected,
            "identity + 16 agents must all be enumerable"
        );

        // Proof the test is not vacuous: the old format put this whole set in
        // one credential, and that write is still rejected by the backend.
        let as_one_blob = blob_json(&expected);
        assert!(
            fake_backend::blob_bytes(&as_one_blob) > fake_backend::MAX_BLOB_BYTES,
            "regression guard: the fixture must exceed the single-credential cap"
        );
        assert!(
            fake_backend::set(svc, "would-be-blob", &as_one_blob).is_err(),
            "regression guard: the single-blob write must still fail — that is the bug being fixed"
        );
        // The index is over one credential's worth of names too, so it chunks.
        assert!(
            fake_backend::raw(svc, &index_chunk_key(1)).is_some(),
            "the name index must spill into a second chunk rather than overflow"
        );
        // And nothing written anywhere is over the cap.
        for name in fake_backend::names_for(svc) {
            let value = fake_backend::raw(svc, &name).unwrap();
            assert!(
                fake_backend::blob_bytes(&value) <= fake_backend::MAX_BLOB_BYTES,
                "credential {name} is {} bytes, over the OS cap",
                fake_backend::blob_bytes(&value)
            );
        }

        // Deleting one agent leaves the other fifteen and the identity alone.
        let (gone, _) = agent_pair(7);
        store.delete(&gone).unwrap();
        let after = SecretStore::with_fake(svc)
            .load_all_readonly()
            .unwrap()
            .unwrap();
        assert_eq!(after.len(), 16);
        assert!(!after.contains_key(&gone));
        assert!(after.contains_key("identity"));

        fake_backend::reset(svc);
    }

    #[test]
    fn identity_survives_blob_migration_on_both_service_names() {
        // The human identity shares the blob with the agent keys, on both the
        // release and the dev service name. Neither may lose it.
        for svc in ["buzz-desktop", "buzz-desktop-dev"] {
            fake_backend::reset(svc);

            let expected = format!("nsec1identity-{svc}");
            let mut blob = HashMap::new();
            blob.insert("identity".to_string(), expected.clone());
            blob.insert("agent:aaa".to_string(), "nsec1aaa".to_string());
            fake_backend::seed(svc, BLOB_KEY, &blob_json(&blob));

            let store = SecretStore::with_fake(svc);
            assert_eq!(
                store.load("identity").unwrap(),
                Some(expected.clone()),
                "{svc}: identity must survive the migration"
            );
            assert_eq!(store.probe("identity"), KeyringProbe::Present, "{svc}");
            assert!(
                store.verify_stored_raw("identity", &expected).unwrap(),
                "{svc}: identity must verify against the OS, not the cache"
            );
            assert_eq!(
                fake_backend::raw(svc, "identity").as_deref(),
                Some(expected.as_str()),
                "{svc}: identity must have its own credential"
            );
            assert_eq!(fake_backend::raw(svc, BLOB_KEY), None, "{svc}: blob gone");

            // Sign-out still wipes it everywhere.
            store.delete_all_with_legacy_cleanup().unwrap();
            assert!(store.verify_fully_wiped(), "{svc}: wipe must verify");
            assert_eq!(
                fake_backend::names_for(svc),
                Vec::<String>::new(),
                "{svc}: no credential may survive sign-out"
            );

            fake_backend::reset(svc);
        }
    }

    // The point of this guard is to spell out each signature verbatim, so
    // factoring one out into a type alias would defeat it.
    #[allow(clippy::type_complexity)]
    #[test]
    fn public_api_surface_is_unchanged() {
        // Amputation guard: every public entry point callers depend on must
        // keep its exact signature. `storage.rs`, `app_state.rs`, `reset.rs`,
        // `identity.rs` and `pairing.rs` all bind to these.
        let _: fn(String) -> SecretStore = SecretStore::keyring;
        let _: fn(&'static str) -> SecretStore = SecretStore::keyring;
        let _: fn(&'static str) -> &'static SecretStore = SecretStore::shared;
        let _: fn(&SecretStore, &str) -> KeyringProbe = SecretStore::probe;
        let _: fn(&SecretStore, &str) -> Result<Option<String>, String> = SecretStore::load;
        let _: fn(&SecretStore) -> Result<Option<HashMap<String, String>>, String> =
            SecretStore::load_all_readonly;
        let _: fn(&SecretStore, &str, &str) -> Result<(), String> = SecretStore::store;
        let _: fn(&SecretStore, &HashMap<String, String>) -> Result<(), String> =
            SecretStore::store_all;
        let _: fn(&SecretStore, &str, &str) -> Result<bool, String> =
            SecretStore::verify_stored_raw;
        let _: fn(&SecretStore, &str) -> Result<(), String> = SecretStore::delete;
        let _: fn(&SecretStore) -> Result<(), String> = SecretStore::delete_all_with_legacy_cleanup;
        let _: fn(&SecretStore) -> bool = SecretStore::verify_fully_wiped;

        // KeyringProbe's variants are matched on by callers.
        let _ = KeyringProbe::Present;
        let _ = KeyringProbe::ReachableButEmpty;
        let _ = KeyringProbe::Unreachable;
    }
}
