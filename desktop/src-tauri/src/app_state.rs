use std::{
    collections::HashMap,
    io::Write,
    sync::{atomic::AtomicU16, Arc, Mutex},
};

use nostr::{Keys, ToBech32};
use tauri::{AppHandle, Manager};
#[cfg(feature = "mesh-llm")]
use tokio::sync::Mutex as AsyncMutex;

use crate::huddle::HuddleState;
use crate::managed_agents::ManagedAgentProcess;

pub struct AppState {
    pub keys: Mutex<Keys>,
    pub http_client: reqwest::Client,
    /// Workspace-provided relay URL override. Set by `apply_workspace` on app
    /// init and takes priority over env vars and compile-time defaults.
    pub relay_url_override: Mutex<Option<String>>,
    pub managed_agents_store_lock: Mutex<()>,
    pub channel_templates_store_lock: Mutex<()>,
    pub managed_agent_processes: Mutex<HashMap<String, ManagedAgentProcess>>,
    pub huddle_state: Mutex<HuddleState>,
    /// Tauri app handle — stored after setup so huddle commands can emit
    /// `huddle-state-changed` events without needing the handle threaded
    /// through every call site.
    ///
    /// Set once during `setup()` in `lib.rs`; never cleared.
    pub app_handle: Mutex<Option<AppHandle>>,
    /// Selected audio output device name. `None` = system default.
    /// Used by `connect_audio_relay` and TTS pipeline when opening sinks.
    pub audio_output_device: Mutex<Option<String>>,
    /// Port of the localhost media streaming proxy (set during setup).
    pub media_proxy_port: AtomicU16,
    /// IOKit power assertion state — prevents idle sleep while agents run.
    pub prevent_sleep: Arc<Mutex<crate::prevent_sleep::PreventSleepState>>,
    /// In-process mesh-llm node started by Buzz Desktop.
    #[cfg(feature = "mesh-llm")]
    pub mesh_llm_runtime: AsyncMutex<Option<crate::mesh_llm::DesktopMeshRuntime>>,
    /// Runtime-owned relay-mesh control plane (call-me-now listener + connect
    /// request publish/retry). Installed once at identity-set time so the
    /// listener is up before any restore/create can request a connection.
    #[cfg(feature = "mesh-llm")]
    pub mesh_coordinator: AsyncMutex<Option<crate::mesh_llm::MeshCoordinator>>,
}

/// Parse the `BUZZ_PRIVATE_KEY` env var into identity keys. `Some` means the
/// env var was present and valid and MUST win over any persisted/keyring key
/// (the dev/CI/harness override). `None` means absent or malformed — callers
/// fall through to persisted resolution. A malformed value is logged and
/// treated as absent rather than left on an ephemeral identity.
fn identity_from_env() -> Option<Keys> {
    match std::env::var("BUZZ_PRIVATE_KEY") {
        Ok(nsec) => match Keys::parse(nsec.trim()) {
            Ok(keys) => Some(keys),
            Err(error) => {
                eprintln!("buzz-desktop: invalid BUZZ_PRIVATE_KEY: {error}");
                None
            }
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("buzz-desktop: BUZZ_PRIVATE_KEY contains invalid UTF-8");
            None
        }
        Err(std::env::VarError::NotPresent) => None,
    }
}

pub fn build_app_state() -> AppState {
    // Env var takes precedence (dev/CI). If absent, resolve_persisted_identity()
    // in setup() will replace the ephemeral placeholder with a persisted key.
    let keys = match identity_from_env() {
        Some(keys) => {
            eprintln!(
                "buzz-desktop: configured identity pubkey {}",
                keys.public_key().to_hex()
            );
            keys
        }
        None => Keys::generate(),
    };

    AppState {
        keys: Mutex::new(keys),
        http_client: reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(10))
            .pool_max_idle_per_host(1)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        relay_url_override: Mutex::new(None),
        managed_agents_store_lock: Mutex::new(()),
        channel_templates_store_lock: Mutex::new(()),
        managed_agent_processes: Mutex::new(HashMap::new()),
        huddle_state: Mutex::new(HuddleState::default()),
        app_handle: Mutex::new(None),
        audio_output_device: Mutex::new(None),
        media_proxy_port: AtomicU16::new(0),
        prevent_sleep: Arc::new(Mutex::new(
            crate::prevent_sleep::PreventSleepState::default(),
        )),
        #[cfg(feature = "mesh-llm")]
        mesh_llm_runtime: AsyncMutex::new(None),
        #[cfg(feature = "mesh-llm")]
        mesh_coordinator: AsyncMutex::new(None),
    }
}

impl AppState {
    /// Lock the huddle state mutex, converting a poisoned-lock error to a String.
    ///
    /// Convenience wrapper — replaces 15+ instances of
    /// `state.huddle_state.lock().map_err(|e| e.to_string())?` throughout the
    /// huddle module.
    pub fn huddle(&self) -> Result<std::sync::MutexGuard<'_, crate::huddle::HuddleState>, String> {
        self.huddle_state.lock().map_err(|e| e.to_string())
    }

    /// Emit the current huddle state to the frontend via Tauri event.
    ///
    /// Acquires both locks (app_handle + huddle_state), clones a snapshot,
    /// releases both, then emits. Best-effort — no-op if either lock is
    /// poisoned or the app_handle hasn't been set yet.
    pub fn emit_huddle_state_changed(&self) {
        let app = match self.app_handle.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => return,
        };
        let Some(app) = app else { return };
        let snapshot = match self.huddle_state.lock() {
            Ok(hs) => hs.clone(),
            Err(_) => return,
        };
        crate::huddle::state::emit_huddle_state(&app, &snapshot);
    }
}

/// Resolve the user's identity key from the app data directory.
///
/// Priority: `BUZZ_PRIVATE_KEY` env var (already handled in `build_app_state`)
/// → `{app_data_dir}/identity.key` file → generate + save.
///
/// Writes use `atomic-write-file` which handles temp file creation, fsync,
/// atomic rename, and directory sync — no partial or corrupt files on disk.
pub fn resolve_persisted_identity(app: &AppHandle, state: &AppState) -> Result<(), String> {
    // Only skip file-based resolution if the env var was present AND parsed
    // successfully. A malformed env var should fall through to the persisted
    // key rather than leaving the app on an ephemeral identity.
    if identity_from_env().is_some() {
        return Ok(());
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;

    let keys = load_or_create_identity(&data_dir)?;
    *state.keys.lock().map_err(|e| e.to_string())? = keys;
    Ok(())
}

/// Service name for the desktop OS keyring. Shared by the human identity key
/// and managed-agent keys (each addressed by a distinct key name within it).
pub(crate) const KEYRING_SERVICE: &str = "buzz-desktop";

/// Keyring key name for the human identity nsec.
const IDENTITY_KEY_NAME: &str = "identity";

/// The keyring operations the identity resolution flow needs. Abstracted so the
/// corrupt-keyring recovery decision ([`recover_from_keyring`]) can be
/// unit-tested against a fake without touching the live OS keyring.
trait IdentityKeyStore {
    fn probe(&self, name: &str) -> crate::secret_store::KeyringProbe;
    fn load(&self, name: &str) -> Result<Option<String>, String>;
    fn store(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

impl IdentityKeyStore for crate::secret_store::SecretStore {
    fn probe(&self, name: &str) -> crate::secret_store::KeyringProbe {
        crate::secret_store::SecretStore::probe(self, name)
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        crate::secret_store::SecretStore::load(self, name)
    }
    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        crate::secret_store::SecretStore::store(self, name, value)
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        crate::secret_store::SecretStore::delete(self, name)
    }
}

/// Resolve the human identity key: migrate a legacy `identity.key` into the
/// keyring when safe, otherwise load from whichever backend holds it, else
/// generate-and-save.
///
/// Migration rule (prevents stale-key resurrection): only import the plaintext
/// file when the keyring is REACHABLE-but-empty. If the keyring is UNREACHABLE
/// this boot, fall back to reading the file directly and do NOT migrate — a
/// later import from a leftover (possibly rotated) file could resurrect an old
/// key.
fn load_or_create_identity(data_dir: &std::path::Path) -> Result<Keys, String> {
    let legacy_path = data_dir.join("identity.key");

    // No keyring available in this build: the `0o600` file is the only store.
    if !cfg!(feature = "system-keyring") {
        return load_file_or_generate(&legacy_path, data_dir);
    }

    let store = crate::secret_store::SecretStore::keyring(KEYRING_SERVICE);
    resolve_identity_with_store(&store, &legacy_path, data_dir)
}

/// Identity resolution over an [`IdentityKeyStore`] seam. Split from
/// [`load_or_create_identity`] so the probe/recover branches are testable
/// without the live OS keyring.
fn resolve_identity_with_store(
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<Keys, String> {
    use crate::secret_store::KeyringProbe;

    match store.probe(IDENTITY_KEY_NAME) {
        KeyringProbe::Present => {
            if let Some(nsec) = store.load(IDENTITY_KEY_NAME)? {
                match Keys::parse(nsec.trim()) {
                    Ok(keys) => {
                        eprintln!(
                            "buzz-desktop: persisted identity pubkey {}",
                            keys.public_key().to_hex()
                        );
                        // The key is authoritative in the keyring. A leftover
                        // `identity.key` means a prior migration's `remove_file`
                        // failed (transient AV lock, read-only mount, EPERM) and
                        // never retried — clean it up now so plaintext does not
                        // linger on disk.
                        cleanup_leftover_identity_file(legacy_path);
                        return Ok(keys);
                    }
                    // The corruption is in the KEYRING, not the file. Clear the
                    // bad keyring value and recover from the file (or generate
                    // fresh) — do NOT quarantine a valid leftover `identity.key`
                    // that holds the user's only good key.
                    Err(error) => {
                        return recover_from_keyring(store, legacy_path, &error.to_string());
                    }
                }
            }
            // Probe said Present but load found nothing — treat as empty.
        }
        KeyringProbe::ReachableButEmpty => {
            // One-time migration: import the legacy plaintext file, read-back
            // verify, THEN delete it.
            if legacy_path.exists() {
                if let Some(keys) = migrate_identity_file(store, legacy_path)? {
                    return Ok(keys);
                }
            }
        }
        KeyringProbe::Unreachable => {
            // Keyring down this boot — read the file directly, do NOT migrate.
            return load_file_or_generate(legacy_path, data_dir);
        }
    }

    generate_and_persist(store, legacy_path)
}

/// Recover from a corrupt nsec in the keyring (parse failed). Clear the bad
/// keyring value, then migrate a valid leftover `identity.key` if one exists,
/// generating fresh only as a last resort. The keyring delete is best-effort:
/// a delete failure logs and continues — it must never block startup.
fn recover_from_keyring(
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
    error: &str,
) -> Result<Keys, String> {
    eprintln!("buzz-desktop: corrupt nsec in keyring ({error}), clearing and recovering from file");
    if let Err(e) = store.delete(IDENTITY_KEY_NAME) {
        eprintln!("buzz-desktop: failed to clear corrupt keyring value: {e}");
    }
    if legacy_path.exists() {
        if let Some(keys) = migrate_identity_file(store, legacy_path)? {
            return Ok(keys);
        }
    }
    generate_and_persist(store, legacy_path)
}

/// Load the `0o600` identity file, quarantining corruption, else generate and
/// save a fresh key to the file. Used when no keyring is available.
fn load_file_or_generate(
    legacy_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<Keys, String> {
    if legacy_path.exists() {
        match load_key_file(legacy_path) {
            Ok(keys) => {
                eprintln!(
                    "buzz-desktop: persisted identity pubkey {}",
                    keys.public_key().to_hex()
                );
                return Ok(keys);
            }
            Err(error) => quarantine_corrupt_key(legacy_path, data_dir, &error),
        }
    }
    let keys = Keys::generate();
    save_key_file(legacy_path, &keys)?;
    eprintln!(
        "buzz-desktop: generated and saved identity pubkey {}",
        keys.public_key().to_hex()
    );
    Ok(keys)
}

/// Import the plaintext `identity.key` into the store, verify the round-trip,
/// then delete the file. Returns `Ok(None)` if the file was corrupt (caller
/// continues to generate-and-save).
fn migrate_identity_file(
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
) -> Result<Option<Keys>, String> {
    let keys = match load_key_file(legacy_path) {
        Ok(keys) => keys,
        Err(error) => {
            eprintln!("buzz-desktop: corrupt identity.key during migration ({error}), skipping");
            return Ok(None);
        }
    };
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("encode nsec: {e}"))?;

    store.store(IDENTITY_KEY_NAME, &nsec)?;
    // Read-back verify before deleting the plaintext file.
    match store.load(IDENTITY_KEY_NAME)? {
        Some(stored) if stored == nsec => {
            if let Err(e) = std::fs::remove_file(legacy_path) {
                eprintln!("buzz-desktop: keyring import ok but failed to delete identity.key: {e}");
            } else {
                eprintln!("buzz-desktop: migrated identity key into OS keyring");
            }
            Ok(Some(keys))
        }
        _ => Err("keyring read-back verify failed for identity key".to_string()),
    }
}

/// Generate a fresh identity, persist it through the store, return it.
fn generate_and_persist(
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
) -> Result<Keys, String> {
    let keys = Keys::generate();
    persist_identity(store, &keys, legacy_path)?;
    eprintln!(
        "buzz-desktop: generated and saved identity pubkey {}",
        keys.public_key().to_hex()
    );
    Ok(keys)
}

/// Persist `keys` through the store, falling back to the `0o600` file when the
/// keyring write fails on an availability error.
fn persist_identity(
    store: &impl IdentityKeyStore,
    keys: &Keys,
    legacy_path: &std::path::Path,
) -> Result<(), String> {
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("encode nsec: {e}"))?;
    match store.store(IDENTITY_KEY_NAME, &nsec) {
        Ok(()) => Ok(()),
        Err(keyring_err) => {
            eprintln!("buzz-desktop: keyring write failed ({keyring_err}), using file fallback");
            save_key_file(legacy_path, keys)
        }
    }
}

/// Best-effort removal of a leftover `identity.key` once the keyring is the
/// authoritative store. Idempotent: a missing file is success. Logs but does
/// not error on failure — a delete failure must never block startup.
fn cleanup_leftover_identity_file(legacy_path: &std::path::Path) {
    if !legacy_path.exists() {
        return;
    }
    match std::fs::remove_file(legacy_path) {
        Ok(()) => eprintln!("buzz-desktop: removed leftover identity.key (key is in keyring)"),
        Err(e) => eprintln!("buzz-desktop: failed to remove leftover identity.key: {e}"),
    }
}

/// Quarantine a corrupt `identity.key` with a timestamp so prior backups are
/// never overwritten.
fn quarantine_corrupt_key(key_path: &std::path::Path, data_dir: &std::path::Path, error: &str) {
    if !key_path.exists() {
        return;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bad_name = format!("identity.key.bad.{ts}");
    eprintln!("buzz-desktop: corrupt identity.key ({error}), quarantining to {bad_name}");
    let bad_path = data_dir.join(bad_name);
    if std::fs::rename(key_path, &bad_path).is_err() {
        let _ = std::fs::remove_file(key_path);
    }
}

fn load_key_file(path: &std::path::Path) -> Result<Keys, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read identity.key: {e}"))?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("empty identity.key".to_string());
    }
    Keys::parse(trimmed).map_err(|e| format!("parse identity.key: {e}"))
}

/// Atomically write the key to disk. Uses `atomic-write-file` which:
/// 1. Writes to a temp file in the same directory
/// 2. Calls fsync on the file
/// 3. Renames temp → target (atomic on POSIX, best-effort on Windows)
/// 4. Calls fsync on the parent directory
///
/// On Unix, the file is created with mode 0600 (owner read/write only).
/// On Windows, default ACLs apply — the app data directory is already
/// per-user, so the key is not world-readable in practice.
pub(crate) fn save_key_file(path: &std::path::Path, keys: &Keys) -> Result<(), String> {
    use atomic_write_file::AtomicWriteFile;

    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("encode nsec: {e}"))?;

    let mut file = AtomicWriteFile::open(path)
        .map_err(|e| format!("open identity.key for atomic write: {e}"))?;

    // Set owner-only permissions before writing the secret.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set identity.key permissions: {e}"))?;
    }

    file.write_all(nsec.as_bytes())
        .map_err(|e| format!("write identity.key: {e}"))?;
    file.commit()
        .map_err(|e| format!("commit identity.key: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_key_eq(a: &Keys, b: &Keys) {
        assert_eq!(a.public_key().to_hex(), b.public_key().to_hex());
    }

    /// `BUZZ_PRIVATE_KEY` is process-global; serialize the env-mutating tests
    /// so they don't race each other under the parallel test runner.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `body` with `BUZZ_PRIVATE_KEY` set to `value` (or unset when `None`),
    /// restoring the prior value afterward.
    fn with_env_key<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var("BUZZ_PRIVATE_KEY").ok();
        match value {
            Some(v) => std::env::set_var("BUZZ_PRIVATE_KEY", v),
            None => std::env::remove_var("BUZZ_PRIVATE_KEY"),
        }
        let out = body();
        match prior {
            Some(v) => std::env::set_var("BUZZ_PRIVATE_KEY", v),
            None => std::env::remove_var("BUZZ_PRIVATE_KEY"),
        }
        out
    }

    #[test]
    fn identity_from_env_wins_when_valid() {
        let configured = Keys::generate();
        let nsec = configured.secret_key().to_bech32().unwrap();

        let resolved =
            with_env_key(Some(&nsec), identity_from_env).expect("valid env key must resolve");

        assert_key_eq(&configured, &resolved);
    }

    #[test]
    fn identity_from_env_none_when_absent() {
        assert!(with_env_key(None, identity_from_env).is_none());
    }

    #[test]
    fn identity_from_env_none_when_malformed() {
        // A malformed env var falls through to persisted resolution rather than
        // winning — otherwise a typo'd key would silently shadow the real one.
        assert!(with_env_key(Some("not-a-valid-nsec"), identity_from_env).is_none());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        let keys = Keys::generate();

        save_key_file(&path, &keys).unwrap();
        let loaded = load_key_file(&path).unwrap();
        assert_key_eq(&keys, &loaded);
    }

    #[test]
    fn load_rejects_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        std::fs::write(&path, "").unwrap();

        assert!(load_key_file(&path).is_err());
    }

    #[test]
    fn load_rejects_corrupt_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        std::fs::write(&path, "not-a-valid-nsec").unwrap();

        assert!(load_key_file(&path).is_err());
    }

    #[test]
    fn load_missing_file_is_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.key");

        assert!(load_key_file(&path).is_err());
    }

    #[test]
    fn cleanup_removes_leftover_identity_file() {
        // Item 1: a leftover identity.key (from a migration whose remove_file
        // failed) is deleted once the keyring is authoritative, so plaintext
        // does not linger on disk.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        save_key_file(&path, &Keys::generate()).unwrap();
        assert!(path.exists());

        cleanup_leftover_identity_file(&path);

        assert!(!path.exists());
    }

    #[test]
    fn cleanup_is_noop_when_no_leftover_file() {
        // Idempotent: the cleanup runs on every keyring-Present boot, so a
        // missing file must be a silent success, not an error or panic.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        assert!(!path.exists());

        cleanup_leftover_identity_file(&path);

        assert!(!path.exists());
    }

    #[test]
    fn save_creates_file_with_valid_nsec() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        let keys = Keys::generate();

        save_key_file(&path, &keys).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("nsec1"));
    }

    #[cfg(unix)]
    #[test]
    fn save_creates_file_with_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        let keys = Keys::generate();

        save_key_file(&path, &keys).unwrap();

        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[test]
    fn save_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");

        let keys1 = Keys::generate();
        save_key_file(&path, &keys1).unwrap();

        let keys2 = Keys::generate();
        save_key_file(&path, &keys2).unwrap();

        let loaded = load_key_file(&path).unwrap();
        assert_key_eq(&keys2, &loaded);
    }

    use std::cell::RefCell;
    use std::collections::HashMap;

    use crate::secret_store::KeyringProbe;

    /// In-memory [`IdentityKeyStore`] for testing identity recovery without the
    /// OS keyring. Seeded with an initial value and a probe outcome; records
    /// every `delete`/`store` so tests can assert the keyring was cleared and
    /// rewritten. `write_and_verify` succeeds (store then load reflects it).
    struct FakeIdentityStore {
        probe: KeyringProbe,
        slot: RefCell<HashMap<String, String>>,
        deleted: RefCell<Vec<String>>,
    }

    impl FakeIdentityStore {
        fn present_with(value: &str) -> Self {
            let mut slot = HashMap::new();
            slot.insert(IDENTITY_KEY_NAME.to_string(), value.to_string());
            Self {
                probe: KeyringProbe::Present,
                slot: RefCell::new(slot),
                deleted: RefCell::new(Vec::new()),
            }
        }
    }

    impl IdentityKeyStore for FakeIdentityStore {
        fn probe(&self, _name: &str) -> KeyringProbe {
            self.probe
        }
        fn load(&self, name: &str) -> Result<Option<String>, String> {
            Ok(self.slot.borrow().get(name).cloned())
        }
        fn store(&self, name: &str, value: &str) -> Result<(), String> {
            self.slot
                .borrow_mut()
                .insert(name.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.deleted.borrow_mut().push(name.to_string());
            self.slot.borrow_mut().remove(name);
            Ok(())
        }
    }

    #[test]
    fn corrupt_keyring_recovers_valid_file_without_rotating() {
        // The load-bearing regression guard. When the keyring holds a corrupt
        // nsec (Present) AND a valid `identity.key` is on disk (leftover from a
        // failed prior migration), recovery must RECOVER THE FILE'S identity —
        // not quarantine the file and rotate to a fresh key (the original
        // hazard). The corrupt keyring value must be cleared and replaced by the
        // file's key (migrated in).
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("identity.key");
        let file_keys = Keys::generate();
        save_key_file(&legacy_path, &file_keys).unwrap();

        let store = FakeIdentityStore::present_with("not-a-valid-nsec");
        let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

        // The FILE's identity is recovered — NOT a freshly generated one.
        assert_key_eq(&file_keys, &resolved);
        // The corrupt keyring value was cleared.
        assert_eq!(store.deleted.borrow().as_slice(), [IDENTITY_KEY_NAME]);
        // The keyring now holds the file's key (migrated in, read-back verified).
        let file_nsec = file_keys.secret_key().to_bech32().unwrap();
        assert_eq!(
            store
                .slot
                .borrow()
                .get(IDENTITY_KEY_NAME)
                .map(String::as_str),
            Some(file_nsec.as_str())
        );
        // The valid file was migrated (deleted), not quarantined to .bad.*.
        assert!(!legacy_path.exists());
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".bad.")));
    }

    #[test]
    fn corrupt_keyring_generates_fresh_only_when_no_file() {
        // With a corrupt keyring value and NO file on disk, generate-fresh is
        // the correct last resort — and the corrupt keyring value is cleared
        // first.
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("identity.key");
        assert!(!legacy_path.exists());

        let store = FakeIdentityStore::present_with("not-a-valid-nsec");
        let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

        assert_eq!(store.deleted.borrow().as_slice(), [IDENTITY_KEY_NAME]);
        // A fresh, valid key was persisted to the keyring (replacing the cleared
        // corrupt value).
        let stored = store.slot.borrow().get(IDENTITY_KEY_NAME).cloned();
        assert_eq!(
            stored.as_deref(),
            Some(resolved.secret_key().to_bech32().unwrap().as_str())
        );
    }

    #[test]
    fn valid_keyring_is_used_and_leftover_file_cleaned_up() {
        // The happy path is unchanged: a valid keyring value is used as-is, and
        // a leftover plaintext file is cleaned up (keyring is authoritative).
        let keyring_keys = Keys::generate();
        let nsec = keyring_keys.secret_key().to_bech32().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("identity.key");
        save_key_file(&legacy_path, &Keys::generate()).unwrap();

        let store = FakeIdentityStore::present_with(&nsec);
        let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

        assert_key_eq(&keyring_keys, &resolved);
        assert!(store.deleted.borrow().is_empty());
        assert!(!legacy_path.exists());
    }
}
