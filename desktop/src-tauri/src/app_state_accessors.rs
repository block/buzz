//! Convenience accessors over [`AppState`]'s lock-guarded fields.
//!
//! Kept apart from `app_state.rs`, which owns the struct, its builder, and the
//! identity-key resolution that populates it.

use nostr::Keys;

use crate::app_state::AppState;
use crate::managed_agents::config_bridge::SessionConfigCache;
use crate::managed_agents::ManagedAgentRuntimeKey;

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
