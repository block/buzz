use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};

use nostr::Keys;

use super::AppState;
use crate::managed_agents::{config_bridge::SessionConfigCache, ManagedAgentRuntimeKey};

impl AppState {
    pub(crate) fn clear_managed_agent_reference_sync_ready(&self) -> Result<(), String> {
        self.managed_agent_reference_sync_ready
            .lock()
            .map_err(|error| error.to_string())?
            .clear();
        Ok(())
    }

    pub(crate) fn mark_managed_agent_reference_sync_ready(
        &self,
        scope: PathBuf,
    ) -> Result<(), String> {
        self.managed_agent_reference_sync_ready
            .lock()
            .map_err(|error| error.to_string())?
            .insert(scope);
        Ok(())
    }

    pub(crate) fn managed_agent_reference_sync_is_ready(
        &self,
        scope: &Path,
    ) -> Result<bool, String> {
        Ok(self
            .managed_agent_reference_sync_ready
            .lock()
            .map_err(|error| error.to_string())?
            .contains(scope))
    }

    pub(crate) fn reserve_managed_agent_persona_mint(
        &self,
        persona_id: String,
    ) -> Result<ManagedAgentPersonaMintReservation<'_>, String> {
        let mut active = self
            .managed_agent_persona_mints
            .lock()
            .map_err(|error| error.to_string())?;
        if !active.insert(persona_id.clone()) {
            return Err(format!(
                "agent identity creation is already in progress for persona {persona_id}"
            ));
        }
        Ok(ManagedAgentPersonaMintReservation {
            active: &self.managed_agent_persona_mints,
            persona_id,
        })
    }

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

pub(crate) struct ManagedAgentPersonaMintReservation<'a> {
    active: &'a Mutex<HashSet<String>>,
    persona_id: String,
}

impl Drop for ManagedAgentPersonaMintReservation<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.persona_id);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn persona_mint_reservation_serializes_the_pre_save_window() {
        let state = crate::app_state::build_app_state();
        let first = state
            .reserve_managed_agent_persona_mint("persona-1".to_string())
            .unwrap();

        let error = match state.reserve_managed_agent_persona_mint("persona-1".to_string()) {
            Ok(_) => panic!("a second reservation must not enter the mint window"),
            Err(error) => error,
        };
        assert!(error.contains("already in progress"));

        drop(first);
        assert!(state
            .reserve_managed_agent_persona_mint("persona-1".to_string())
            .is_ok());
    }
}
