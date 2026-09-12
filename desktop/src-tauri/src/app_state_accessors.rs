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

    /// Local-secret capability: return the active identity keys in a signable state.
    ///
    /// Backup, pairing, encryption and agent provisioning require this capability;
    /// generic event consumers should capture `event_signer` instead.
    ///
    /// Returns `Err` when the identity is in a lost state (`identity_lost`
    /// — ephemeral key, user must re-import their nsec) or when the keyring
    /// is locked (`keyring_locked` — key is held in a keyring that is
    /// unavailable this boot). All signing and publish commands must call
    /// this instead of locking `state.keys` directly, so that recovery mode
    /// blocks publishing under an invalid or inaccessible identity.
    pub fn signing_keys(&self) -> Result<Keys, String> {
        if crate::enterprise_identity::enabled() {
            return Err(
                "This feature requires a local private key and is unavailable in corporate builds"
                    .into(),
            );
        }
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
            .and_then(|k| k.clone().ok_or_else(|| "Local identity unavailable".into()))
    }

    /// Capture the active event signer after the same recovery checks as local keys.
    /// Secret export, encryption, pairing and provisioning must use `signing_keys`.
    pub fn event_signer(
        &self,
    ) -> Result<std::sync::Arc<dyn buzz_ws_client_pkg::event_signer::EventSigner>, String> {
        if crate::enterprise_identity::enabled() {
            return self.enterprise.event_signer();
        }
        Ok(std::sync::Arc::new(
            buzz_ws_client_pkg::event_signer::LocalEventSigner::new(self.signing_keys()?),
        ))
    }

    /// Public identity, including recovery-mode local identity display.
    pub(crate) fn public_key(&self) -> Result<nostr::PublicKey, String> {
        if crate::enterprise_identity::enabled() {
            use buzz_ws_client_pkg::event_signer::EventSigner;
            return Ok(self.enterprise.event_signer()?.public_key());
        }
        self.keys
            .lock()
            .map_err(|e| e.to_string())?
            .as_ref()
            .map(Keys::public_key)
            .ok_or_else(|| "Local identity unavailable".into())
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
