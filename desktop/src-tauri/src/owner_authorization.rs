//! Foreground owner-proof preparation, not workspace activation or agent custody.
use crate::user_operation::UserOperationScope;
use crate::{active_user_signer::ActiveUserSigner, app_state::AppState};
use nostr::{Keys, ToBech32};

/// One captured owner capability and relay, retained across all proof awaits.
pub(crate) struct OwnerAuthorizationScope {
    pub(crate) signer: ActiveUserSigner,
    pub(crate) relay_base: String,
    pub(crate) operation: UserOperationScope,
    legacy_recovery: bool,
}

impl OwnerAuthorizationScope {
    pub(crate) fn capture(state: &AppState) -> Result<Self, String> {
        Self::capture_with_recovery(state, false)
    }

    /// Preserve the historical missing-tag repair's local recovery exception.
    pub(crate) fn capture_legacy_repair(state: &AppState) -> Result<Self, String> {
        Self::capture_with_recovery(state, true)
    }

    fn capture_with_recovery(state: &AppState, legacy_recovery: bool) -> Result<Self, String> {
        let generation = state
            .operation_generation
            .lock()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            signer: if legacy_recovery {
                state.legacy_local_signer()?
            } else {
                state.active_signer()?
            },
            relay_base: crate::relay::relay_api_base_url_with_override(state),
            operation: UserOperationScope::capture_locked(state, *generation)?,
            legacy_recovery,
        })
    }

    /// Point-in-time check for callers that do not commit synchronous side effects.
    /// Use `admit` and retain its guard for a write or process start.
    pub(crate) fn check_current(&self, state: &AppState) -> Result<(), String> {
        drop(self.admit(state)?);
        Ok(())
    }

    pub(crate) fn admit<'a>(
        &self,
        state: &'a AppState,
    ) -> Result<std::sync::MutexGuard<'a, u64>, String> {
        let guard = self.operation.admit(state)?;
        self.signer.check_valid()?;
        let current = if self.legacy_recovery {
            state.legacy_local_signer()?
        } else {
            state.active_signer()?
        };
        if current.generation() != self.signer.generation()
            || current.public_key() != self.signer.public_key()
            || crate::relay::relay_api_base_url_with_override(state) != self.relay_base
        {
            return Err("owner authorization scope changed; retry operation".into());
        }
        Ok(guard)
    }
}

/// Inert independent-agent material. Nothing is stored or published by minting.
/// Dropping this on any proof failure/cancellation leaves no keyring/record writes.
pub(crate) struct AuthorizedAgent {
    pub(crate) keys: Keys,
    pub(crate) private_key_nsec: String,
    pub(crate) pubkey: String,
    pub(crate) auth_tag: Option<String>,
}

/// Shared production mint boundary for create and both snapshot import callers.
/// Human custody comes only from `owner`; fresh keys belong only to the new agent.
pub(crate) async fn prepare_agent(owner: &ActiveUserSigner) -> Result<AuthorizedAgent, String> {
    owner.check_valid()?;
    let keys = Keys::generate();
    let auth_tag = Some(owner.authorize_agent(&keys.public_key(), "").await?);
    let private_key_nsec = keys.secret_key().to_bech32().map_err(|e| e.to_string())?;
    Ok(AuthorizedAgent {
        pubkey: keys.public_key().to_hex(),
        keys,
        private_key_nsec,
        auth_tag,
    })
}

#[cfg(test)]
#[path = "owner_authorization_tests.rs"]
mod tests;

/// Workspace-wide remote mutations remain gated until their complete workflows are integrated.
pub(crate) fn require_owned_workspace(state: &AppState) -> Result<(), String> {
    if state.is_remote_identity() {
        return Err("remote owner workspace mutations are unsupported until generation-owned workspace lifecycle".into());
    }
    Ok(())
}
