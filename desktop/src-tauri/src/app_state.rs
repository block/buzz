use std::{
    collections::HashMap,
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicU16},
        Arc, Mutex,
    },
};

use nostr::{Keys, ToBech32};
use tauri::{AppHandle, Manager};
#[cfg(feature = "mesh-llm")]
use tokio::sync::Mutex as AsyncMutex;

use crate::huddle::HuddleState;
use crate::managed_agents::config_bridge::SessionConfigCache;
use crate::managed_agents::{ManagedAgentPairRuntime, ManagedAgentRuntimeKey};
pub struct AppState {
    pub keys: Mutex<Keys>,
    pub http_client: reqwest::Client,
    /// A no-redirect client for authenticated relay media fetches (download,
    /// clipboard copy, snapshot, editor). Every caller pre-validates the URL
    /// origin, but the app-wide `http_client` follows redirects by default, so
    /// a relay `/media/` URL returning a 3xx to an off-origin or private host
    /// would forward the minted media Authorization header across origins —
    /// a redirect-hop SSRF. This client treats any 3xx as a non-success
    /// response (surfaced as an error) so the auth token never leaves the
    /// validated relay origin.
    pub media_fetch_client: reqwest::Client,
    /// Workspace-provided relay URL override. Set by `apply_workspace` on app
    /// init and takes priority over env vars and compile-time defaults.
    pub relay_url_override: Mutex<Option<String>>,
    /// Set during backend setup when managed agents are eligible for launch
    /// restore. `apply_workspace` consumes it after installing the workspace
    /// relay and identity, so agents never start against the fallback relay.
    pub managed_agent_restore_pending: AtomicBool,
    /// Whether desktop may repair managed-agent kind:0 profiles from its local
    /// records. Disabled by the agent-managed profiles experiment so an agent's
    /// own profile updates are not overwritten on start or restore.
    pub managed_agent_profile_reconcile_enabled: AtomicBool,
    /// Shared shutdown signal checked by launch-time agent restoration.
    pub shutdown_started: AtomicBool,
    /// Serializes every managed-runtime transition that changes the protected
    /// PID set: spawn/register, adoption, stop, shutdown, and sweep snapshots.
    /// Never perform network I/O while holding this lock.
    pub managed_agent_runtime_transition: Mutex<()>,
    pub managed_agents_store_lock: Mutex<()>,
    pub channel_templates_store_lock: Mutex<()>,
    pub managed_agent_processes: Mutex<HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>>,
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
    /// Set when identity resolution detected a "keyring-locked" state: the
    /// keyring is unreachable this boot but a migration marker shows the key
    /// lives there. An ephemeral key is generated so the app can open; all
    /// signing commands check this flag via [`AppState::signing_keys`] and
    /// return `Err` so no events are published under the inaccessible identity.
    /// Mutually exclusive with `identity_lost` (guaranteed by `RecoveryState`
    /// at the resolve boundary).
    ///
    /// Ordering: writers store with `Ordering::Release` after `state.keys` is
    /// updated, so a reader observing `false` with `Ordering::Acquire` is
    /// guaranteed to see the updated keys. Writers: `setup()` (initial
    /// resolution via `resolve_persisted_identity`) and `import_identity`
    /// (clears the flag when the user successfully imports a new key).
    pub keyring_locked: AtomicBool,
    /// Set when identity resolution detected a "lost" state: the migration
    /// marker was present but the keyring was empty and no plaintext fallback
    /// existed. An ephemeral key was generated to let the app boot; the
    /// frontend checks this flag via `get_identity` and routes to the nsec
    /// re-import step instead of the normal onboarding profile flow.
    ///
    /// Ordering: writers store with `Ordering::Release` after `state.keys` is
    /// updated, so a reader observing `false` with `Ordering::Acquire` is
    /// guaranteed to see the updated keys. Writers: `setup()` (initial
    /// resolution) and `import_identity`/`persist_current_identity`
    /// (user-initiated key import).
    pub identity_lost: AtomicBool,
    /// Serializes runtime identity mutations (`import_identity` and
    /// `persist_current_identity`) so a stale ephemeral key can never overwrite
    /// a newer imported key during concurrent calls. Deliberately separate from
    /// `keys` so readers (signing, get_identity, etc.) are not blocked during
    /// keyring I/O.
    pub identity_mutation: Mutex<()>,
    /// Set when the boot-time Phase 2 reset attempted a wipe but verification
    /// failed. The sentinel is preserved so the next relaunch retries. All
    /// identity-dependent setup is skipped; the frontend shows a reset-failed
    /// recovery screen via `get_identity`.
    ///
    /// Ordering: written once in `setup()` with `Ordering::Release`; read in
    /// `get_identity` with `Ordering::Acquire`.
    pub reset_failed: AtomicBool,
    /// Cached ACP session config from running agents, keyed by canonical
    /// `(agent pubkey, relay URL)` runtime identity.
    /// Populated when the harness emits `session_config_captured` observer events.
    pub session_config_cache: Mutex<HashMap<ManagedAgentRuntimeKey, SessionConfigCache>>,
    /// IOKit power assertion state — prevents idle sleep while agents run.
    pub prevent_sleep: Arc<Mutex<crate::prevent_sleep::PreventSleepState>>,
    /// In-process mesh-llm node started by Buzz Desktop.
    #[cfg(feature = "mesh-llm")]
    pub mesh_llm_runtime: AsyncMutex<Option<crate::mesh_llm::DesktopMeshRuntime>>,
    /// Runtime-owned shared-compute coordinator. It publishes member-signed
    /// discovery status and reconciles MeshLLM's admission roster; MeshLLM
    /// itself owns direct QUIC/iroh connection establishment.
    #[cfg(feature = "mesh-llm")]
    pub mesh_coordinator: AsyncMutex<Option<crate::mesh_llm::MeshCoordinator>>,
    /// `(creator_pubkey_hex, channel_id)` pairs for channels the *named*
    /// identity created via `create_channel` and has not yet observed its own
    /// kind:39002 membership entry for. The relay provisions that entry
    /// asynchronously (#1761), so without this overlay a freshly created
    /// channel's owner reads back as `is_member=false` until the snapshot
    /// propagates, disabling their own composer. Entries are bound to the
    /// creating identity so an in-process identity swap (`import_identity`,
    /// workspace apply) can never inherit another identity's stale
    /// membership. Populated only by this process's own `create_channel`
    /// calls — a relay can never write into it — so it carries no
    /// trust-boundary risk. `get_channels` clears an entry once the real
    /// kind:39002 is observed for the current identity, keeping the set
    /// bounded and letting a later leave correctly flip the channel back to
    /// `is_member=false`.
    pub pending_owned_channels: Mutex<std::collections::HashSet<(String, String)>>,
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

/// Build the no-redirect HTTP client used for authenticated relay media
/// fetches (download / copy).
///
/// This client is a security boundary, not a convenience: it carries a minted
/// media `Authorization` header, so it MUST NOT follow redirects. A relay 3xx
/// to an off-origin or private host would otherwise forward that header across
/// origins (a redirect-hop SSRF). `redirect::Policy::none()` returns the 3xx
/// verbatim so the caller can reject it.
///
/// Returned as a `Result` so the fail-closed invariant is testable — callers
/// must never substitute a redirect-following client on build failure. Shares
/// the localhost `resolve`/pool config with the app-wide `http_client`.
pub fn build_media_fetch_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .pool_idle_timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .redirect(reqwest::redirect::Policy::none())
        .build()
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
            .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
            .pool_idle_timeout(std::time::Duration::from_secs(10))
            .pool_max_idle_per_host(1)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        media_fetch_client: build_media_fetch_client().expect(
            "media_fetch_client must build with redirect::Policy::none(); a \
             redirect-following fallback would forward the minted media auth \
             header across origins (redirect-hop SSRF)",
        ),
        relay_url_override: Mutex::new(None),
        managed_agent_restore_pending: AtomicBool::new(false),
        managed_agent_profile_reconcile_enabled: AtomicBool::new(true),
        shutdown_started: AtomicBool::new(false),
        managed_agent_runtime_transition: Mutex::new(()),
        identity_mutation: Mutex::new(()),
        managed_agents_store_lock: Mutex::new(()),
        channel_templates_store_lock: Mutex::new(()),
        managed_agent_processes: Mutex::new(HashMap::new()),
        session_config_cache: Mutex::new(HashMap::new()),
        huddle_state: Mutex::new(HuddleState::default()),
        app_handle: Mutex::new(None),
        audio_output_device: Mutex::new(None),
        media_proxy_port: AtomicU16::new(0),
        prevent_sleep: Arc::new(Mutex::new(
            crate::prevent_sleep::PreventSleepState::default(),
        )),
        keyring_locked: AtomicBool::new(false),
        identity_lost: AtomicBool::new(false),
        reset_failed: AtomicBool::new(false),
        #[cfg(feature = "mesh-llm")]
        mesh_llm_runtime: AsyncMutex::new(None),
        #[cfg(feature = "mesh-llm")]
        mesh_coordinator: AsyncMutex::new(None),
        pending_owned_channels: Mutex::new(std::collections::HashSet::new()),
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

    pub fn get_session_cache(&self, key: &ManagedAgentRuntimeKey) -> Option<SessionConfigCache> {
        self.session_config_cache.lock().ok()?.get(key).cloned()
    }

    pub fn put_session_cache(&self, key: ManagedAgentRuntimeKey, cache: SessionConfigCache) {
        if let Ok(mut map) = self.session_config_cache.lock() {
            map.insert(key, cache);
        }
    }

    pub fn clear_agent_session_cache(&self, key: &ManagedAgentRuntimeKey) {
        if let Ok(mut map) = self.session_config_cache.lock() {
            map.remove(key);
        }
    }

    pub fn clear_agent_session_caches(&self, pubkey: &str) {
        if let Ok(mut map) = self.session_config_cache.lock() {
            map.retain(|key, _| key.pubkey != pubkey);
        }
    }

    /// Record that `channel_id` was just created by `creator_pubkey` and its
    /// kind:39002 owner membership has not yet been observed.
    pub fn mark_pending_owned_channel(&self, creator_pubkey: &str, channel_id: &str) {
        if let Ok(mut set) = self.pending_owned_channels.lock() {
            set.insert((creator_pubkey.to_string(), channel_id.to_string()));
        }
    }

    /// Whether `channel_id` is still awaiting `my_pubkey`'s kind:39002 entry.
    /// Bound to `my_pubkey` so an in-process identity swap never inherits
    /// another identity's pending-owner entry for the same channel id.
    pub fn is_pending_owned_channel(&self, my_pubkey: &str, channel_id: &str) -> bool {
        self.pending_owned_channels
            .lock()
            .map(|set| set.contains(&(my_pubkey.to_string(), channel_id.to_string())))
            .unwrap_or(false)
    }

    /// Drop the `(my_pubkey, channel_id)` entry from the pending-owner
    /// overlay once that identity's real kind:39002 membership has been
    /// observed.
    pub fn clear_pending_owned_channel(&self, my_pubkey: &str, channel_id: &str) {
        if let Ok(mut set) = self.pending_owned_channels.lock() {
            set.remove(&(my_pubkey.to_string(), channel_id.to_string()));
        }
    }

    /// Return the active identity keys if they are in a signable state.
    ///
    /// Returns `Err` when the identity is in a lost state (`identity_lost`
    /// — ephemeral key, user must re-import their nsec) or when the keyring
    /// is locked (`keyring_locked` — key is held in a keyring that is
    /// unavailable this boot). All signing and publish commands must call
    /// this instead of locking `state.keys` directly, so that recovery mode
    /// blocks publishing under an invalid or inaccessible identity.
    pub fn signing_keys(&self) -> Result<Keys, String> {
        if self
            .identity_lost
            .load(std::sync::atomic::Ordering::Acquire)
            || self
                .keyring_locked
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("identity is in recovery mode; event signing is disabled \
                 until the identity is restored and Buzz is relaunched"
                .to_string());
        }
        self.keys
            .lock()
            .map_err(|e| e.to_string())
            .map(|k| k.clone())
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

/// Resolve the user's identity key from the app data directory and wire
/// the resulting [`RecoveryState`] into `AppState`.
///
/// Priority: `BUZZ_PRIVATE_KEY` env var (already handled in `build_app_state`)
/// → keyring → `{app_data_dir}/identity.key` file → generate + save.
///
/// On success, writes the resolved keys into `state.keys` (with the mutex)
/// before storing the recovery flags (Release), so any thread that reads
/// either flag as `false` with Acquire is guaranteed to see the updated keys.
///
/// Sets `state.identity_lost` on `RecoveryState::Lost` (keyring empty after
/// migration — key gone externally) and `state.keyring_locked` on
/// `RecoveryState::KeyringLocked` (keyring unreachable — key still in keyring
/// but inaccessible this boot). Both states boot with an ephemeral key; the
/// frontend shows different recovery screens for each.
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

    let resolved = load_or_create_identity(&data_dir)?;
    // Write keys before setting the recovery flags (Release) so any thread
    // that reads a flag as false with Acquire is guaranteed to see the keys.
    *state.keys.lock().map_err(|e| e.to_string())? = resolved.keys;
    state.identity_lost.store(
        resolved.recovery == RecoveryState::Lost,
        std::sync::atomic::Ordering::Release,
    );
    state.keyring_locked.store(
        resolved.recovery == RecoveryState::KeyringLocked,
        std::sync::atomic::Ordering::Release,
    );
    Ok(())
}

#[path = "app_state_keyring.rs"]
mod keyring_config;
pub(crate) use keyring_config::keyring_service;

/// Keyring key name for the human identity nsec.
const IDENTITY_KEY_NAME: &str = "identity";

/// Filename of the marker written once a successful keyring migration deletes
/// the legacy `identity.key`. Its presence is the only durable signal that a
/// key once lived in the keyring — used to tell a genuine first-ever launch
/// (no key anywhere, generating is correct) from a post-migration boot whose
/// keyring is merely unreachable (the key IS in the keyring, must NOT generate).
const MIGRATION_MARKER_NAME: &str = "identity.migrated";

/// Recovery state produced by identity resolution. `None` means the app has
/// a real, usable identity. `Lost` means the keyring was reachable-but-empty
/// despite a prior successful migration — the key vanished externally. `KeyringLocked`
/// means the keyring is unreachable this boot but was used in the past
/// (marker present, no file) — the key still exists but is temporarily
/// inaccessible. Both non-`None` variants boot with an ephemeral key; the
/// frontend shows a different recovery screen for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryState {
    None,
    Lost,
    KeyringLocked,
}

/// The output of identity resolution.
struct ResolvedIdentity {
    keys: Keys,
    recovery: RecoveryState,
}

/// The keyring operations the identity resolution flow needs. Abstracted so the
/// corrupt-keyring recovery decision ([`recover_from_keyring`]) can be
/// unit-tested against a fake without touching the live OS keyring.
trait IdentityKeyStore {
    fn probe(&self, name: &str) -> crate::secret_store::KeyringProbe;
    fn load(&self, name: &str) -> Result<Option<String>, String>;
    fn store(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
    /// Verify that `key` holds `expected` by reading directly from the OS
    /// backend — bypassing any in-process cache. Returns `Ok(true)` when the
    /// stored value matches, `Ok(false)` when it does not or is absent, and
    /// `Err` when the backend is unavailable.
    fn verify_stored(&self, key: &str, expected: &str) -> Result<bool, String>;
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
    fn verify_stored(&self, key: &str, expected: &str) -> Result<bool, String> {
        crate::secret_store::SecretStore::verify_stored_raw(self, key, expected)
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
fn load_or_create_identity(data_dir: &std::path::Path) -> Result<ResolvedIdentity, String> {
    let legacy_path = data_dir.join("identity.key");

    // No keyring available in this build: the `0o600` file is the only store.
    if !cfg!(feature = "system-keyring") {
        let keys = load_file_or_generate(&legacy_path, data_dir)?;
        return Ok(ResolvedIdentity {
            keys,
            recovery: RecoveryState::None,
        });
    }

    let store = crate::secret_store::SecretStore::shared(keyring_service());
    resolve_identity_with_store(store, &legacy_path, data_dir)
}

/// Identity resolution over an [`IdentityKeyStore`] seam. Split from
/// [`load_or_create_identity`] so the probe/recover branches are testable
/// without the live OS keyring.
fn resolve_identity_with_store(
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<ResolvedIdentity, String> {
    use crate::secret_store::KeyringProbe;

    match store.probe(IDENTITY_KEY_NAME) {
        KeyringProbe::Present => {
            if let Some(nsec) = store.load(IDENTITY_KEY_NAME)? {
                match Keys::parse(nsec.trim()) {
                    Ok(keyring_keys) => {
                        eprintln!(
                            "buzz-desktop: persisted identity pubkey {}",
                            keyring_keys.public_key().to_hex()
                        );
                        // Check for a leftover identity.key. If it holds a
                        // DIFFERENT pubkey, the user imported that key after
                        // the last boot (pre-fix, import only wrote the file).
                        // Adopt it into the keyring so the user's intent sticks.
                        // If the pubkeys match it is a stale leftover from a
                        // prior migration whose remove_file failed — clean it up.
                        if legacy_path.exists() {
                            match load_key_file(legacy_path) {
                                Ok(file_keys)
                                    if file_keys.public_key() != keyring_keys.public_key() =>
                                {
                                    eprintln!(
                                        "buzz-desktop: identity.key differs from keyring; \
                                         adopting imported key {}",
                                        file_keys.public_key().to_hex()
                                    );
                                    // Delegate the store→read-back-verify→marker→delete
                                    // sequence to `persist_identity_to_keyring`, which owns
                                    // the marker-before-delete invariant and the fallback
                                    // logic that keeps identity.key when the marker write
                                    // fails. A transient keyring failure must not abort
                                    // boot — the file key is safe and adoption retries next
                                    // boot when the keyring is reachable again.
                                    if let Err(e) = persist_identity_to_keyring(
                                        store,
                                        &file_keys,
                                        legacy_path,
                                        data_dir,
                                    ) {
                                        eprintln!(
                                            "buzz-desktop: keyring adoption of identity.key \
                                             failed ({e}); using file key, will retry next boot"
                                        );
                                    }
                                    return Ok(ResolvedIdentity {
                                        keys: file_keys,
                                        recovery: RecoveryState::None,
                                    });
                                }
                                // Corrupt file — keyring is authoritative. Log before
                                // cleanup so there is a diagnostic for the lost data.
                                Err(e) => {
                                    eprintln!(
                                        "buzz-desktop: leftover identity.key is corrupt ({e}); \
                                         keyring is authoritative, removing"
                                    );
                                    ensure_marker_then_cleanup(data_dir, legacy_path);
                                }
                                // Same pubkey (stale leftover from a completed migration
                                // whose remove_file previously failed) — keyring is
                                // authoritative. Ensure the marker exists (crash-safe
                                // ordering: marker before delete), then clean up.
                                Ok(_) => {
                                    ensure_marker_then_cleanup(data_dir, legacy_path);
                                }
                            }
                        }
                        // Self-heal: if the identity.key is gone and the migration
                        // marker is absent (e.g. a stranded keyring-only install from
                        // a pre-fix path that stored to the keyring but could not write
                        // the marker or fallback file), write the marker now so a later
                        // keyring-Unreachable boot does not treat this as a fresh install
                        // and silently rotate the identity. Failure is non-fatal — boot
                        // must never be blocked here.
                        if !legacy_path.exists() && !migration_marker_path(data_dir).exists() {
                            if let Err(e) = write_migration_marker(&migration_marker_path(data_dir))
                            {
                                eprintln!(
                                    "buzz-desktop: keyring present but marker missing; \
                                     self-heal marker write failed ({e}), continuing"
                                );
                            }
                        }
                        return Ok(ResolvedIdentity {
                            keys: keyring_keys,
                            recovery: RecoveryState::None,
                        });
                    }
                    // The corruption is in the KEYRING, not the file. Clear the
                    // bad keyring value and recover from the file (or generate
                    // fresh) — do NOT quarantine a valid leftover `identity.key`
                    // that holds the user's only good key.
                    Err(error) => {
                        return recover_from_keyring(
                            store,
                            legacy_path,
                            data_dir,
                            &error.to_string(),
                        );
                    }
                }
            } else {
                // Probe said Present but load found nothing — treat as empty.
                // Falls through to generate_and_persist below.
            }
        }
        KeyringProbe::ReachableButEmpty => {
            // One-time migration: import the legacy plaintext file, read-back
            // verify, THEN delete it.
            if legacy_path.exists() {
                if let Some(keys) = migrate_identity_file(store, legacy_path, data_dir)? {
                    return Ok(ResolvedIdentity {
                        keys,
                        recovery: RecoveryState::None,
                    });
                }
            } else if migration_marker_path(data_dir).exists() {
                // Marker present, keyring empty, no file — the key was previously
                // durably stored in the keyring but is now gone (keyring cleared,
                // new login session, or the entry was externally deleted). There
                // is no plaintext fallback to recover from.
                //
                // Generate an ephemeral in-memory key so the app can boot, but
                // surface a "lost" flag so the frontend prompts re-import rather
                // than silently starting a fresh identity.
                let ephemeral = Keys::generate();
                eprintln!(
                    "buzz-desktop: identity lost — keyring was empty despite migration marker; \
                     using ephemeral key {}, awaiting user re-import",
                    ephemeral.public_key().to_hex()
                );
                return Ok(ResolvedIdentity {
                    keys: ephemeral,
                    recovery: RecoveryState::Lost,
                });
            }
        }
        KeyringProbe::Unreachable => {
            // Keyring down this boot. If a recoverable file is present, use it
            // (and do NOT migrate — re-importing later could resurrect a
            // rotated key). With NO file, the marker disambiguates two states
            // that are otherwise byte-identical (Unreachable + no file):
            //   - marker present → the key was migrated into the keyring and the
            //     file deleted. The real key is unreachable this boot but still
            //     exists in the keyring. Boot keyring-locked recovery (ephemeral
            //     key, all signing disabled) so the app can at least open; the
            //     frontend shows a "unlock the keyring and relaunch" screen.
            //     Fail-closed semantics are preserved: nothing is ever persisted
            //     under the ephemeral key, so no silent identity rotation occurs.
            //   - no marker → genuine first-ever launch with nothing to protect.
            //     Generate to the `0o600` file (legitimate first-run).
            if !legacy_path.exists() && migration_marker_path(data_dir).exists() {
                let ephemeral = Keys::generate();
                eprintln!(
                    "buzz-desktop: keyring unreachable but migration marker present; \
                     booting keyring-locked recovery with ephemeral key {} — \
                     unlock the keyring and relaunch",
                    ephemeral.public_key().to_hex()
                );
                return Ok(ResolvedIdentity {
                    keys: ephemeral,
                    recovery: RecoveryState::KeyringLocked,
                });
            }
            let keys = load_file_or_generate(legacy_path, data_dir)?;
            return Ok(ResolvedIdentity {
                keys,
                recovery: RecoveryState::None,
            });
        }
    }

    let keys = generate_and_persist(store, legacy_path, data_dir)?;
    Ok(ResolvedIdentity {
        keys,
        recovery: RecoveryState::None,
    })
}

/// Recover from a corrupt nsec in the keyring (parse failed). Clear the bad
/// keyring value, then migrate a valid leftover `identity.key` if one exists.
/// If the migration marker is present but no valid file exists, the prior
/// identity is unrecoverable — return `Lost` recovery rather than silently
/// generating a new identity. Generating fresh is only correct when no prior
/// identity ever existed (no marker). The keyring delete is best-effort: a
/// delete failure logs and continues — it must never block startup.
fn recover_from_keyring(
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
    data_dir: &std::path::Path,
    error: &str,
) -> Result<ResolvedIdentity, String> {
    eprintln!("buzz-desktop: corrupt nsec in keyring ({error}), clearing and recovering from file");
    if let Err(e) = store.delete(IDENTITY_KEY_NAME) {
        eprintln!("buzz-desktop: failed to clear corrupt keyring value: {e}");
    }
    if legacy_path.exists() {
        if let Some(keys) = migrate_identity_file(store, legacy_path, data_dir)? {
            return Ok(ResolvedIdentity {
                keys,
                recovery: RecoveryState::None,
            });
        }
    }
    // No valid file to recover from. If the migration marker exists, a prior
    // identity was stored in the keyring and is now corrupt AND gone — the key
    // is unrecoverable. Enter Lost recovery instead of silently rotating.
    if migration_marker_path(data_dir).exists() {
        let ephemeral = Keys::generate();
        eprintln!(
            "buzz-desktop: identity lost — keyring had corrupt data and no valid identity.key \
             backup; prior identity (migration marker present) is unrecoverable; \
             using ephemeral key {}, awaiting user re-import",
            ephemeral.public_key().to_hex()
        );
        return Ok(ResolvedIdentity {
            keys: ephemeral,
            recovery: RecoveryState::Lost,
        });
    }
    // No marker: genuine first launch with a corrupt keyring. Generate fresh.
    let keys = generate_and_persist(store, legacy_path, data_dir)?;
    Ok(ResolvedIdentity {
        keys,
        recovery: RecoveryState::None,
    })
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
    data_dir: &std::path::Path,
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
    // Read-back verify before deleting the plaintext file. Uses verify_stored()
    // which bypasses the in-process cache and reads directly from the OS
    // backend — proving the OS keyring round-trip, not just the cache.
    let verify_ok = match store.verify_stored(IDENTITY_KEY_NAME, &nsec) {
        Ok(b) => b,
        Err(e) => return Err(format!("keyring read-back verify failed: {e}")),
    };
    if !verify_ok {
        return Err("keyring read-back verify failed for identity key".to_string());
    }
    // Crash-safe ordering: record that the key now lives in the keyring
    // (marker write + fsync) BEFORE deleting the file. A crash between
    // the two must never leave "file gone, no marker" — that state is
    // indistinguishable from a fresh install and would silently rotate
    // the identity on the next keyring-unreachable boot. If the marker
    // cannot be written, keep the file so the key is never stranded.
    let marker_path = migration_marker_path(data_dir);
    if let Err(e) = write_migration_marker(&marker_path) {
        eprintln!(
            "buzz-desktop: keyring import ok but failed to write migration marker ({e}); \
             keeping identity.key so the key is not stranded"
        );
        return Ok(Some(keys));
    }
    if let Err(e) = std::fs::remove_file(legacy_path) {
        eprintln!("buzz-desktop: keyring import ok but failed to delete identity.key: {e}");
    } else {
        eprintln!("buzz-desktop: migrated identity key into OS keyring");
    }
    Ok(Some(keys))
}

/// Persist `keys` into the keyring with read-back verification, write the
/// migration marker, and delete any leftover `identity.key`. Returns `Ok` on
/// success. Returns `Err` when the keyring write fails (availability error) —
/// the caller must fall back to `save_key_file` so the key survives the boot.
///
/// This is the shared kernel used by both one-time file migration and the
/// `import_identity` command. Crash-safe ordering: marker is written BEFORE
/// deleting the file.
fn persist_identity_to_keyring(
    store: &impl IdentityKeyStore,
    keys: &Keys,
    legacy_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<(), String> {
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("encode nsec: {e}"))?;

    // Will error if the keyring is unavailable — caller falls back to the file.
    store.store(IDENTITY_KEY_NAME, &nsec)?;

    // Read-back verify before touching durable state. Uses verify_stored()
    // which bypasses the in-process cache and reads directly from the OS
    // backend — proving the OS keyring round-trip, not just the cache.
    match store.verify_stored(IDENTITY_KEY_NAME, &nsec) {
        Ok(true) => {}
        Ok(false) => return Err("keyring read-back verify failed".to_string()),
        Err(e) => return Err(format!("keyring read-back verify failed: {e}")),
    }

    // Write marker before deleting the file (crash-safe ordering).
    let marker_path = migration_marker_path(data_dir);
    if let Err(e) = write_migration_marker(&marker_path) {
        // Keyring holds the key but no marker exists. Preserve the invariant
        // "keyring-only implies marker exists" by ensuring identity.key is
        // present as a fallback: write it if absent, leave it if already there.
        // This prevents a later keyring-unreachable + no-marker boot from
        // treating this as a fresh install and silently rotating identity.
        if !legacy_path.exists() {
            if let Err(write_err) = save_key_file(legacy_path, keys) {
                eprintln!(
                    "buzz-desktop: keyring ok but marker write failed ({e}) and \
                     identity.key write also failed ({write_err}); key may be unrecoverable"
                );
                return Err(format!(
                    "keyring ok but neither migration marker nor identity.key fallback \
                     could be written (marker: {e}; file: {write_err}); \
                     identity must not be treated as durably persisted — retry the import"
                ));
            } else {
                eprintln!(
                    "buzz-desktop: keyring ok but marker write failed ({e}); \
                     wrote identity.key as fallback so the key is not stranded"
                );
            }
        } else {
            eprintln!(
                "buzz-desktop: keyring ok but marker write failed ({e}); \
                 keeping existing identity.key so the key is not stranded"
            );
        }
        return Ok(());
    }

    if legacy_path.exists() {
        if let Err(e) = std::fs::remove_file(legacy_path) {
            eprintln!("buzz-desktop: keyring write ok but failed to delete identity.key: {e}");
        }
    }

    Ok(())
}

/// Core implementation of imported-identity persistence. Tries the OS keyring
/// first via [`persist_identity_to_keyring`]; if the keyring is unavailable,
/// falls back to the `0o600` identity.key file. Returns `Err` only when both
/// the keyring write and the file fallback fail.
fn persist_imported_identity_impl(
    store: &impl IdentityKeyStore,
    keys: &Keys,
    legacy_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<(), String> {
    match persist_identity_to_keyring(store, keys, legacy_path, data_dir) {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!(
                "buzz-desktop: keyring write failed during import ({e}), \
                 falling back to identity.key"
            );
            save_key_file(legacy_path, keys)
        }
    }
}

/// Public entry point binding [`persist_imported_identity_impl`] to the shared
/// [`crate::secret_store::SecretStore`]. See the impl for the persistence policy.
pub(crate) fn persist_imported_identity(
    store: &crate::secret_store::SecretStore,
    keys: &Keys,
    legacy_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<(), String> {
    persist_imported_identity_impl(store, keys, legacy_path, data_dir)
}

/// Path of the migration-completed marker within `data_dir`.
fn migration_marker_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(keyring_config::migration_marker_name(
        keyring_service(),
        MIGRATION_MARKER_NAME,
    ))
}

/// Atomically write (and fsync) the migration-completed marker. The content is
/// irrelevant — only the file's durable existence is the signal — so a single
/// byte keeps it minimal. Atomicity + fsync guarantee that once this returns
/// `Ok`, the marker survives a crash, which is what makes deleting the legacy
/// file afterward safe.
fn write_migration_marker(marker_path: &std::path::Path) -> Result<(), String> {
    use atomic_write_file::AtomicWriteFile;

    let mut file = AtomicWriteFile::open(marker_path)
        .map_err(|e| format!("open migration marker for atomic write: {e}"))?;
    file.write_all(b"1")
        .map_err(|e| format!("write migration marker: {e}"))?;
    file.commit()
        .map_err(|e| format!("commit migration marker: {e}"))
}

/// Which backend [`store_key_preferring_keyring`] wrote to. The caller writes
/// the migration marker only after a keyring success — on the file-fallback arm
/// the key is on disk and a marker would wrongly trip the next Unreachable boot
/// into failing closed.
enum PersistBackend {
    Keyring,
    File,
}

/// Generate a fresh identity, persist it through the store, return it.
///
/// On a keyring-backed persist no file is written, so a later
/// keyring-Unreachable boot would see "no file, no marker" (identical to a
/// fresh install) and silently rotate the identity. Writing the marker here
/// makes that boot fail closed. If the marker write fails, fall back to the
/// `0o600` file so the key is never keyring-only-without-marker.
fn generate_and_persist(
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<Keys, String> {
    let keys = Keys::generate();
    if let PersistBackend::Keyring = store_key_preferring_keyring(store, &keys, legacy_path)? {
        let marker_path = migration_marker_path(data_dir);
        if let Err(e) = write_migration_marker(&marker_path) {
            eprintln!(
                "buzz-desktop: stored identity in keyring but failed to write migration marker \
                 ({e}); saving identity.key fallback so the key is not stranded"
            );
            save_key_file(legacy_path, &keys)?;
        }
    }
    eprintln!(
        "buzz-desktop: generated and saved identity pubkey {}",
        keys.public_key().to_hex()
    );
    Ok(keys)
}

/// Persist `keys` through the store, silently falling back to the `0o600` file
/// when the keyring write fails on an availability error. Reports which backend
/// held the key (no verify/marker/delete — those belong to callers that own the
/// full migration contract) so the caller can write the migration marker only on
/// keyring success.
fn store_key_preferring_keyring(
    store: &impl IdentityKeyStore,
    keys: &Keys,
    legacy_path: &std::path::Path,
) -> Result<PersistBackend, String> {
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("encode nsec: {e}"))?;
    match store.store(IDENTITY_KEY_NAME, &nsec) {
        Ok(()) => Ok(PersistBackend::Keyring),
        Err(keyring_err) => {
            eprintln!("buzz-desktop: keyring write failed ({keyring_err}), using file fallback");
            save_key_file(legacy_path, keys)?;
            Ok(PersistBackend::File)
        }
    }
}

/// Ensure the migration marker exists (writing it if absent), then remove the
/// leftover `identity.key`. Crash-safe ordering: the marker is written and
/// fsync-committed before the file is deleted, so a crash between the two
/// leaves the marker on disk and the file intact — the invariant "keyring-only
/// implies marker exists" is preserved. If the marker write fails, the file is
/// kept so a later keyring-unreachable boot can use it as a fallback.
fn ensure_marker_then_cleanup(data_dir: &std::path::Path, legacy_path: &std::path::Path) {
    let marker_path = migration_marker_path(data_dir);
    let marker_ok = marker_path.exists()
        || write_migration_marker(&marker_path)
            .map_err(|e| {
                eprintln!(
                    "buzz-desktop: keyring present but marker missing; \
                     failed to write marker ({e}), keeping identity.key"
                );
            })
            .is_ok();
    if marker_ok {
        cleanup_leftover_identity_file(legacy_path);
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
#[path = "app_state_tests.rs"]
mod tests;
