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

pub fn build_app_state() -> AppState {
    // Env var takes precedence (dev/CI). If absent, resolve_persisted_identity()
    // in setup() will replace the ephemeral placeholder with a persisted key.
    let (keys, source) = match std::env::var("BUZZ_PRIVATE_KEY") {
        Ok(nsec) => match Keys::parse(nsec.trim()) {
            Ok(keys) => (keys, "configured"),
            Err(error) => {
                eprintln!("buzz-desktop: invalid BUZZ_PRIVATE_KEY: {error}");
                (Keys::generate(), "ephemeral")
            }
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("buzz-desktop: BUZZ_PRIVATE_KEY contains invalid UTF-8");
            (Keys::generate(), "ephemeral")
        }
        Err(std::env::VarError::NotPresent) => (Keys::generate(), "ephemeral"),
    };

    if source == "configured" {
        eprintln!(
            "buzz-desktop: configured identity pubkey {}",
            keys.public_key().to_hex()
        );
    }

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
    if let Ok(nsec) = std::env::var("BUZZ_PRIVATE_KEY") {
        if Keys::parse(nsec.trim()).is_ok() {
            return Ok(());
        }
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;
    let key_path = data_dir.join("identity.key");

    // Try to load an existing key.
    if key_path.exists() {
        match load_key_file(&key_path) {
            Ok(keys) => {
                eprintln!(
                    "buzz-desktop: persisted identity pubkey {}",
                    keys.public_key().to_hex()
                );
                *state.keys.lock().map_err(|e| e.to_string())? = keys;
                return Ok(());
            }
            Err(error) => {
                // Corrupted — quarantine with a timestamp so prior backups
                // are never overwritten.
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let bad_name = format!("identity.key.bad.{ts}");
                eprintln!(
                    "buzz-desktop: corrupt identity.key ({error}), quarantining to {bad_name}"
                );
                let bad_path = data_dir.join(bad_name);
                if std::fs::rename(&key_path, &bad_path).is_err() {
                    let _ = std::fs::remove_file(&key_path);
                }
            }
        }
    }

    // First run (or recovery from corruption): generate and save.
    let keys = Keys::generate();
    save_key_file(&key_path, &keys)?;

    eprintln!(
        "buzz-desktop: generated and saved identity pubkey {}",
        keys.public_key().to_hex()
    );
    *state.keys.lock().map_err(|e| e.to_string())? = keys;
    Ok(())
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
}
