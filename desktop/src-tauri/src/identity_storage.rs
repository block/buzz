use nostr::Keys;

use crate::app_state::AppState;

/// Durable location of the active human identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum IdentityStorage {
    Ephemeral = 0,
    SystemKeyring = 1,
    LocalFile = 2,
    Environment = 3,
}

impl IdentityStorage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ephemeral => "ephemeral",
            Self::SystemKeyring => "system-keyring",
            Self::LocalFile => "local-file",
            Self::Environment => "environment",
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::SystemKeyring,
            2 => Self::LocalFile,
            3 => Self::Environment,
            _ => Self::Ephemeral,
        }
    }
}

impl AppState {
    pub(crate) fn identity_storage(&self) -> IdentityStorage {
        IdentityStorage::from_u8(
            self.identity_storage
                .load(std::sync::atomic::Ordering::Acquire),
        )
    }

    pub(crate) fn set_identity_storage(&self, storage: IdentityStorage) {
        self.identity_storage
            .store(storage as u8, std::sync::atomic::Ordering::Release);
    }
}

/// Recovery state produced by identity resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryState {
    None,
    Lost,
    KeyringLocked,
}

/// Identity and persistence metadata produced by startup resolution.
pub(crate) struct ResolvedIdentity {
    pub(crate) keys: Keys,
    pub(crate) recovery: RecoveryState,
    pub(crate) storage: IdentityStorage,
}

/// Active workspace agent scope management.
///
/// Kept in this file to manage app_state.rs line count (the ratchet).
/// These methods operate on the `active_agent_scope` field added by the
/// workspace-scoped agent definition store feature.
impl AppState {
    /// Capture a snapshot of the active workspace agent scope.
    /// Returns `None` when no workspace has been applied — fail closed.
    /// Callers crossing `.await` must capture at entry and validate generation.
    pub fn capture_active_scope(
        &self,
    ) -> Option<crate::managed_agents::scope::WorkspaceAgentScope> {
        self.active_agent_scope.lock().ok().and_then(|g| g.clone())
    }

    /// Clear the active scope and bump the generation.
    /// Called by live identity import and prepare-rollback.
    pub(crate) fn clear_active_scope(&self) {
        if let Ok(mut g) = self.active_agent_scope.lock() {
            *g = None;
        }
        crate::managed_agents::scope::next_scope_generation();
    }

    /// Commit the active scope directly. Used by tests that need to set up a
    /// live workspace without running the full `apply_workspace` pipeline.
    #[cfg(test)]
    pub(crate) fn commit_active_scope(
        &self,
        scope: crate::managed_agents::scope::WorkspaceAgentScope,
    ) {
        if let Ok(mut g) = self.active_agent_scope.lock() {
            *g = Some(scope);
        }
    }
}

/// Pending-owned-channel overlay — moved here to keep `app_state.rs` within
/// the line-count ratchet. These track channels whose kind:39002 owner entry
/// has not yet been observed after `create_channel`.
impl AppState {
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

    /// Drop the `(my_pubkey, channel_id)` entry once the real kind:39002 has
    /// been observed.
    pub fn clear_pending_owned_channel(&self, my_pubkey: &str, channel_id: &str) {
        if let Ok(mut set) = self.pending_owned_channels.lock() {
            set.remove(&(my_pubkey.to_string(), channel_id.to_string()));
        }
    }
}
