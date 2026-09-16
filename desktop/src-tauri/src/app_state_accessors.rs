//! Convenience accessors over [`AppState`]'s lock-guarded fields.
//!
//! Kept apart from `app_state.rs`, which owns the struct, its builder, and the
//! identity-key resolution that populates it.

use nostr::Keys;

use crate::app_state::AppState;
use crate::managed_agents::config_bridge::SessionConfigCache;
use crate::managed_agents::ManagedAgentRuntimeKey;

impl AppState {
    /// Capture the authority supplied by this renderer realm.
    pub(crate) fn renderer_signer(
        &self,
        generation: Option<u64>,
    ) -> Result<crate::active_user_signer::ActiveUserSigner, String> {
        self.renderer_signer_at(generation, &crate::relay::relay_ws_url_with_override(self))
    }

    /// Remote authority is limited to its enrolled workspace and immutable generation.
    pub(crate) fn renderer_signer_at(
        &self,
        generation: Option<u64>,
        relay: &str,
    ) -> Result<crate::active_user_signer::ActiveUserSigner, String> {
        if self.is_remote_identity() {
            let configured = crate::relay::relay_ws_url_with_override(self);
            if crate::relay::relay_http_base_url(relay)
                != crate::relay::relay_http_base_url(&configured)
            {
                return Err("remote operation requires the configured workspace relay".into());
            }
            self.native_auth.workspace_signer(generation, &configured)
        } else {
            self.active_signer()
        }
    }

    /// Local recovery may read unsigned; remote media always requires captured authority.
    pub(crate) fn media_read_scope(
        &self,
        generation: Option<u64>,
        operation_generation: u64,
    ) -> Result<crate::media_read::MediaReadScope, String> {
        let relay = crate::relay::relay_ws_url_with_override(self);
        let (signer, operation) = if self.is_remote_identity() {
            let signer = self.renderer_signer_at(generation, &relay)?;
            let operation = crate::user_operation::UserOperationScope::capture_locked(
                self,
                operation_generation,
            )?;
            (Some(signer), Some(operation))
        } else {
            (self.active_signer().ok(), None)
        };
        Ok(crate::media_read::MediaReadScope::new(
            signer,
            crate::relay::relay_http_base_url(&relay),
            operation,
        ))
    }

    /// Whether this native state has remote (keyless) user custody.
    pub(crate) fn is_remote_identity(&self) -> bool {
        self.signer_mode.is_remote()
    }

    /// Reject raw human-key capabilities before parsing, loading or persisting keys.
    pub(crate) fn require_local_identity(&self) -> Result<(), String> {
        if self.is_remote_identity() {
            Err("local user keys are unsupported in remote signer mode".into())
        } else {
            Ok(())
        }
    }

    /// Compatibility local-key access, preserving historical recovery exceptions.
    /// New signing paths must use active_signer/signing_keys instead.
    pub(crate) fn local_identity_keys(&self) -> Result<Keys, String> {
        self.require_local_identity()?;
        self.local_keys
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or_else(|| "local identity is unavailable".into())
    }

    /// Hold local identity replacement out of a short archive commit.
    pub(crate) fn archive_local_admission(
        &self,
        owner: nostr::PublicKey,
    ) -> Result<impl Sized + '_, String> {
        self.require_local_identity()?;
        let guard = self.local_keys.lock().map_err(|e| e.to_string())?;
        if guard.as_ref().is_none_or(|keys| keys.public_key() != owner) {
            return Err("archive owner changed".into());
        }
        Ok(guard)
    }

    /// Test-only replacement; production installs identity storage or a workspace.
    #[cfg(test)]
    pub(crate) fn replace_local_identity_keys(&self, keys: Keys) -> Result<(), String> {
        self.require_local_identity()?;
        let mut generation = self
            .operation_generation
            .lock()
            .map_err(|e| e.to_string())?;
        *self.local_keys.lock().map_err(|e| e.to_string())? = Some(keys);
        *generation = generation.wrapping_add(1);
        Ok(())
    }

    /// Atomically replace local keys and their storage metadata after persistence.
    pub(crate) fn install_local_identity(
        &self,
        keys: Keys,
        storage: crate::identity_storage::IdentityStorage,
    ) -> Result<(), String> {
        self.require_local_identity()?;
        let mut generation = self
            .operation_generation
            .lock()
            .map_err(|e| e.to_string())?;
        let mut guard = self.local_keys.lock().map_err(|e| e.to_string())?;
        *guard = Some(keys);
        self.set_identity_storage(storage);
        *generation = generation.wrapping_add(1);
        Ok(())
    }

    /// Replace the workspace identity and relay as one foreground-operation boundary.
    /// Identical local owner/relay reapply preserves PR1 operation lifetime.
    pub(crate) fn install_local_workspace(
        &self,
        relay_url: String,
        keys: Option<Keys>,
    ) -> Result<(), String> {
        self.require_local_identity()?;
        let mut generation = self
            .operation_generation
            .lock()
            .map_err(|e| e.to_string())?;
        let mut relay = self.relay_url_override.lock().map_err(|e| e.to_string())?;
        let mut current_keys = self.local_keys.lock().map_err(|e| e.to_string())?;
        let current_relay = relay.clone().unwrap_or_else(crate::relay::relay_ws_url);
        let changed = current_relay != relay_url
            || keys.as_ref().is_some_and(|keys| {
                current_keys
                    .as_ref()
                    .is_none_or(|current| current.public_key() != keys.public_key())
            });
        *relay = Some(relay_url);
        if let Some(keys) = keys {
            *current_keys = Some(keys);
        }
        if changed {
            *generation = generation.wrapping_add(1);
        }
        Ok(())
    }

    /// Preserve the local identity IPC's key/metadata snapshot under the key lock.
    pub(crate) fn local_identity_snapshot(
        &self,
    ) -> Result<crate::identity_storage::LocalIdentitySnapshot, String> {
        use std::sync::atomic::Ordering::Acquire;
        self.require_local_identity()?;
        let keys = self.local_keys.lock().map_err(|e| e.to_string())?;
        let keys = keys.as_ref().ok_or("local identity is unavailable")?;
        Ok(crate::identity_storage::LocalIdentitySnapshot {
            pubkey: keys.public_key(),
            storage: self.identity_storage(),
            lost: self.identity_lost.load(Acquire),
            locked: self.keyring_locked.load(Acquire),
            reset_failed: self.reset_failed.load(Acquire),
        })
    }

    /// Public identity for reads that historically allowed local recovery mode.
    pub(crate) fn identity_public_key(&self) -> Result<nostr::PublicKey, String> {
        if self.is_remote_identity() {
            return self.active_signer().map(|signer| signer.public_key());
        }
        self.local_identity_keys().map(|keys| keys.public_key())
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

    /// Capture the active user's signing capability, preserving the recovery guard.
    pub(crate) fn active_signer(
        &self,
    ) -> Result<crate::active_user_signer::ActiveUserSigner, String> {
        if self.is_remote_identity() {
            return self.native_auth.active_signer();
        }
        #[cfg(test)]
        {
            self.signing_keys()?;
            if let Some(signer) = self.test_signer.lock().map_err(|e| e.to_string())?.clone() {
                return Ok(signer);
            }
        }
        self.signing_keys()
            .map(crate::active_user_signer::ActiveUserSigner::local)
    }

    /// Capture local signing for entrypoints that historically allowed recovery mode.
    ///
    /// Compatibility only: HTTP reads, renderer binding, and human huddle audio/STT
    /// used the raw key lock before signer extraction. Do not use this for new
    /// entrypoints or replace an existing `active_signer` / `signing_keys` guard.
    pub(crate) fn legacy_local_signer(
        &self,
    ) -> Result<crate::active_user_signer::ActiveUserSigner, String> {
        if self.is_remote_identity() {
            return self.active_signer();
        }
        self.local_identity_keys()
            .map(crate::active_user_signer::ActiveUserSigner::local)
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
        self.require_local_identity()?;
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
        self.local_identity_keys()
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
