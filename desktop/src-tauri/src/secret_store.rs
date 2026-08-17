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

/// Resolve the directory whose inode is the cross-process *transaction* lock
/// target for the store physically held by `store_file`
/// (`managed-agents.json`).
///
/// The transaction lock is deliberately NOT keyed like the per-operation blob
/// lock. That lock is a service-keyed lockfile under `/tmp`; this one is the
/// store's own **directory inode**. Two properties fall out of that choice,
/// and both are load-bearing:
///
/// - **Stable shared identity, independent of keyring service.**
///   `managed-agents.json` is symlinked *per file* across dev worktrees while
///   its parent directory is not (see `migration::SHARED_AGENT_FILES`), so
///   resolving the file through its symlink and taking the real parent yields
///   the one canonical directory every process sharing the store contends on —
///   even when `keyring_service()` hands those processes different scoped
///   services. Keying by service would let two processes share one JSON inode
///   while taking different locks; keying by the resolved directory cannot.
///
/// - **Immunity to the unlink/recreate split.** A `/tmp` lockfile can be
///   unlinked by a temp cleaner while a process holds its `flock`; a second
///   process then recreates the pathname, locks a fresh inode, and both
///   "hold" the lock. A directory that holds the store files is non-empty and
///   lives in the owner's app-data tree: no tmp-cleaner unlinks it and
///   `rmdir` refuses a non-empty directory, so the inode a held lock refers to
///   cannot be swapped out underneath a second participant.
///
/// Falls back to the file's own parent when the file does not exist yet (first
/// boot, before anything is written or shared) — there is no committed record
/// to lose in that window.
pub fn store_txn_lock_dir(store_file: &std::path::Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(store_file) {
        if let Some(parent) = real.parent() {
            return parent.to_path_buf();
        }
    }
    store_file
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| store_file.to_path_buf())
}

/// Upper bound on lockfile re-acquisition attempts when a tmp cleaner keeps
/// unlinking the blob lockfile out from under us. Reaching it means the file is
/// being churned faster than we can lock a live inode — fail loudly rather than
/// spin forever.
#[cfg(all(unix, feature = "system-keyring"))]
const MAX_BLOB_LOCK_REACQUIRE: u32 = 100;

/// True iff `file` (an open, `flock`-held fd) is locked on the same inode the
/// pathname currently resolves to.
///
/// Called after the lock is granted to detect a tmp-cleaner unlink/recreate: a
/// mismatch means our fd holds a lock on a now-nameless dead inode while the
/// live pathname is a *different* inode another process can lock in parallel —
/// two processes each "holding" the lock over different inodes, mutual
/// exclusion lost. A missing pathname (`stat` fails) counts as not-live so the
/// caller re-creates and re-locks the live inode.
#[cfg(all(unix, feature = "system-keyring"))]
fn locked_inode_is_live(file: &std::fs::File, path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(locked) = file.metadata() else {
        return false;
    };
    // Compare (dev, ino): inode numbers are only unique within a device, and
    // the open fd pins its inode number so a recreate under the same pathname
    // is guaranteed a different one.
    matches!(
        std::fs::metadata(path),
        Ok(live) if live.dev() == locked.dev() && live.ino() == locked.ino()
    )
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
            use std::os::unix::io::AsRawFd;
            // Loop to survive a tmp cleaner unlinking the lockfile out from
            // under us. The classic `/tmp` lock split: we `flock` an inode, the
            // pathname is unlinked and recreated as a fresh inode, and a second
            // process locks that fresh inode — both "hold" the lock over
            // different inodes. Defeat it by rechecking, after the lock is
            // granted, that the pathname still resolves to the inode we locked.
            // If it does not, our lock is on a dead inode: drop it and retry so
            // we contend on the live one. LOCK_EX blocks until granted, so a
            // stable inode converges in one pass.
            for _ in 0..MAX_BLOB_LOCK_REACQUIRE {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(path)
                    .map_err(|e| format!("blob lock open {}: {e}", path.display()))?;
                // LOCK_EX blocks until the lock is acquired (no LOCK_NB).
                let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
                if ret != 0 {
                    let err = std::io::Error::last_os_error();
                    return Err(format!("blob lock flock: {err}"));
                }
                if locked_inode_is_live(&file, path) {
                    return Ok(BlobLockGuard { file });
                }
                // Stale inode: the pathname was unlinked/recreated while we
                // blocked. Drop this fd (releasing the dead-inode lock) and
                // retry against the live pathname.
            }
            return Err(format!(
                "blob lock: pathname {} churned {MAX_BLOB_LOCK_REACQUIRE} times without a stable inode",
                path.display()
            ));
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

/// Acquire the cross-process secret **transaction** lock on the store
/// directory identified by `store_dir` (the resolved canonical directory from
/// [`store_txn_lock_dir`]) and hold it for the returned guard's lifetime.
///
/// This is the coarse lock every multi-step secret transaction must hold so
/// two Desktop processes sharing the store cannot interleave: a save holds it
/// from the other-half read through generation writes to the atomic JSON
/// commit; GC holds it from the live-ref read through the blob `remove_batch`.
/// Without it, process B's GC could delete a generation that process A wrote
/// but has not yet committed to JSON, leaving A's committed ref dangling; or A
/// could commit a stale pre-lock snapshot over B's committed record.
///
/// The lock target is the store **directory inode**, not a `/tmp` lockfile and
/// not the keyring service — see [`store_txn_lock_dir`] for why (stable shared
/// identity across worktrees + immunity to the unlink/recreate split).
///
/// Orthogonal to the per-operation `mutate_blob` lock, which flocks a separate
/// `/tmp` lockfile ([`blob_lockfile_path`]): a `mutate_blob` inside a
/// transaction takes a lock on a *different* object, so the two never
/// self-deadlock. Transaction callers must not nest this lock within
/// themselves (a second acquire in the same process blocks on its own held
/// exclusive lock); the save/GC/global-config entry points are all leaf-level.
///
/// The guard is `#[must_use]` — dropping it early releases the lock. On a
/// build without the keyring feature this is a no-op guard.
#[cfg(feature = "system-keyring")]
#[must_use = "the transaction lock is released when the guard is dropped"]
pub fn transaction_lock_at(store_dir: &std::path::Path) -> Result<SecretTxnGuard, String> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // Open the directory read-only (never create/truncate — the store dir
        // already exists, created by `managed_agents_base_dir`). `flock` on the
        // directory inode blocks until the lock is acquired; no file inside can
        // split it because a non-empty directory cannot be `rmdir`'d and its
        // inode is fixed for the directory's lifetime.
        let dir = std::fs::File::open(store_dir)
            .map_err(|e| format!("txn lock open dir {}: {e}", store_dir.display()))?;
        let ret = unsafe { libc::flock(dir.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            return Err(format!("txn lock flock {}: {err}", store_dir.display()));
        }
        Ok(SecretTxnGuard { _dir: dir })
    }
    #[cfg(windows)]
    {
        // Windows cannot `flock` a directory handle the same way, so use a
        // named kernel mutex whose name is a deterministic hash of the
        // resolved directory path — the same stable-identity property as the
        // Unix directory inode, and cross-build stable so a signed build and a
        // dev build sharing the store contend on one mutex.
        let name_str = format!("Local\\BuzzSecretTxn-{:016x}", fnv1a64(store_dir));
        let name_wide: Vec<u16> = name_str
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .collect();
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject, INFINITE};
        let handle = unsafe {
            CreateMutexW(
                std::ptr::null::<SECURITY_ATTRIBUTES>(),
                0,
                name_wide.as_ptr(),
            )
        };
        if handle.is_null() {
            let err = std::io::Error::last_os_error();
            return Err(format!("txn lock CreateMutexW: {err}"));
        }
        let wait_result = unsafe { WaitForSingleObject(handle, INFINITE) };
        if wait_result != WAIT_OBJECT_0
            && wait_result != windows_sys::Win32::Foundation::WAIT_ABANDONED
        {
            let err = std::io::Error::last_os_error();
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
            return Err(format!(
                "txn lock WaitForSingleObject: {wait_result} / {err}"
            ));
        }
        Ok(SecretTxnGuard {
            mutex_handle: handle,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = store_dir;
        Err("txn lock: unsupported platform".to_string())
    }
}

/// No-op transaction lock when the keyring feature is disabled: there are no
/// generation writes to serialize, so the guard holds nothing.
#[cfg(not(feature = "system-keyring"))]
#[must_use = "the transaction lock is released when the guard is dropped"]
pub fn transaction_lock_at(_store_dir: &std::path::Path) -> Result<SecretTxnGuard, String> {
    Ok(SecretTxnGuard {})
}

/// Deterministic 64-bit FNV-1a over a path's bytes, used only to name the
/// Windows transaction mutex. Stable across builds (no random seed), so two
/// Desktop builds sharing a store derive the same mutex name.
#[cfg(all(feature = "system-keyring", windows))]
fn fnv1a64(path: &std::path::Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// RAII guard for the cross-process secret transaction lock. Held for the full
/// duration of a save (other-half read → generation writes → JSON commit) or a
/// GC pass (live-ref read → blob remove). See [`transaction_lock_at`].
#[must_use = "the transaction lock is released when the guard is dropped"]
pub struct SecretTxnGuard {
    /// The open directory fd. Never read — held purely for RAII: closing it
    /// releases the `flock(LOCK_EX)` on the directory inode.
    #[cfg(all(feature = "system-keyring", unix))]
    #[allow(dead_code)]
    _dir: std::fs::File,
    #[cfg(all(feature = "system-keyring", windows))]
    mutex_handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(all(feature = "system-keyring", windows))]
impl Drop for SecretTxnGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.mutex_handle);
            windows_sys::Win32::Foundation::CloseHandle(self.mutex_handle);
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
    /// **Cross-process safety**: acquires an exclusive cross-process lock
    /// (`flock(2)` on a service-keyed lockfile in the system temp directory on
    /// Unix, a named kernel mutex via `CreateMutexW` on Windows — see
    /// [`BlobLockGuard`]) before reading, mutating, and writing. The Unix
    /// lockfile is reachable from both the signed DMG build and unsigned dev
    /// builds. Inside the lock a fresh `read_blob_raw()` is always performed
    /// (even when the cache is warm) so a concurrent process's write is never
    /// silently dropped.
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

    /// Delete the entire keychain blob for this service, plus all legacy per-key
    /// entries that could resurrect an identity on next boot.
    ///
    /// Order of operations:
    /// 1. Read the blob to collect every key name (e.g. `identity`, agent keys).
    /// 2. Delete legacy per-key DPK entries for every key + the DPK blob itself.
    /// 3. Delete legacy per-key keyring entries for every key.
    /// 4. Delete the blob entry.
    /// 5. Clear the in-memory cache.
    ///
    /// This is the correct wipe path for sign-out: the old `delete_all` skipped
    /// step 1–3 so stale per-key entries could be re-imported on the next launch
    /// via `migrate_legacy_key`. This method prevents that resurrection.
    pub fn delete_all_with_legacy_cleanup(&self) -> Result<(), String> {
        #[cfg(feature = "system-keyring")]
        {
            let _lock = acquire_blob_lock(&self.service)?;

            // Step 1: read current blob keys (best-effort; no entry = empty set).
            let blob_keys: Vec<String> = match self.read_blob_raw() {
                Ok(Some(bytes)) => {
                    let json = String::from_utf8(bytes).unwrap_or_default();
                    serde_json::from_str::<std::collections::HashMap<String, String>>(&json)
                        .map(|m| m.into_keys().collect())
                        .unwrap_or_default()
                }
                _ => vec![],
            };

            // Always include "identity" even if the blob is empty or absent —
            // it may exist only as a legacy per-key entry.
            let mut all_keys = blob_keys;
            if !all_keys.contains(&"identity".to_string()) {
                all_keys.push("identity".to_string());
            }

            // Steps 2 & 3: delete legacy per-key entries for every key.
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
                {
                    let entry = keyring_entry(&self.service, key)
                        .map_err(|e| format!("keyring entry constructor {key}: {e}"))?;
                    match entry.delete_credential() {
                        Ok(()) | Err(keyring::Error::NoEntry) => {}
                        Err(e) if is_keyring_availability_error(&e.to_string()) => {
                            return Err(format!("keyring unavailable deleting {key}: {e}"));
                        }
                        Err(e) => {
                            return Err(format!("keyring per-key delete {key}: {e}"));
                        }
                    }
                }
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

            // Step 4: delete the main blob entry.
            {
                let entry = keyring_entry(&self.service, BLOB_KEY)
                    .map_err(|e| format!("keyring entry constructor blob: {e}"))?;
                match entry.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => {}
                    Err(e) if is_keyring_availability_error(&e.to_string()) => {
                        return Err(format!("keyring unavailable: {e}"));
                    }
                    Err(e) => {
                        return Err(format!("keyring blob delete: {e}"));
                    }
                }
            }

            // Step 5: clear the in-memory cache.
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
            // 2. Per-key "identity" via legacy keyring must be absent.
            match keyring_entry(&self.service, "identity") {
                Ok(entry) => match entry.get_password() {
                    Err(keyring::Error::NoEntry) => {}
                    Ok(_) => return false,
                    // Any other error (availability, unknown, transient) → fail closed.
                    // Only explicit NoEntry is proof of absence.
                    Err(_) => return false,
                },
                // Constructor failure → cannot verify → fail closed.
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
#[path = "secret_store_tests.rs"]
mod tests;
