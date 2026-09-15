//! Captured foreground admission, shared by command workflows.
use crate::app_state::AppState;

/// Captured foreground user-operation scope. This does not own running agents:
/// independent agent runtimes may outlive the human session that started them.
#[derive(Clone)]
pub(crate) struct UserOperationScope {
    pub(crate) owner_pubkey: nostr::PublicKey,
    pub(crate) relay_url: String,
    generation: u64,
    signer: crate::active_user_signer::ActiveUserSigner,
}

impl UserOperationScope {
    pub(crate) fn capture(state: &AppState) -> Result<Self, String> {
        let generation = state
            .operation_generation
            .lock()
            .map_err(|e| e.to_string())?;
        Self::capture_locked(state, *generation)
    }

    pub(crate) fn capture_locked(state: &AppState, generation: u64) -> Result<Self, String> {
        Ok(Self {
            // Preserve list/start's historical local recovery behavior. This is
            // public identity, not permission to mint a new owner proof.
            owner_pubkey: state.identity_public_key()?,
            relay_url: crate::relay::relay_ws_url_with_override(state),
            generation,
            signer: state.legacy_local_signer()?,
        })
    }

    /// Hold only around synchronous side effects, AFTER waiting for store locks.
    /// Identity/workspace replacement uses this same lock, so a successful check
    /// cannot race the write or process registration it admits. Never await here.
    pub(crate) fn admit<'a>(
        &self,
        state: &'a AppState,
    ) -> Result<std::sync::MutexGuard<'a, u64>, String> {
        let generation = state
            .operation_generation
            .lock()
            .map_err(|e| e.to_string())?;
        self.signer.check_valid()?;
        if *generation != self.generation
            || state.identity_public_key()? != self.owner_pubkey
            || crate::relay::relay_ws_url_with_override(state) != self.relay_url
        {
            return Err("owner authorization scope changed; retry operation".into());
        }
        Ok(generation)
    }
}

impl<'de, R: tauri::Runtime> tauri::ipc::CommandArg<'de, R> for UserOperationScope {
    fn from_command(
        command: tauri::ipc::CommandItem<'de, R>,
    ) -> Result<Self, tauri::ipc::InvokeError> {
        let state = command
            .message
            .state_ref()
            .try_get::<AppState>()
            .ok_or_else(|| tauri::ipc::InvokeError::from("native app state unavailable"))?;
        let operation = Self::capture(&state)?;
        if operation.signer.generation().is_some() {
            let generation = crate::invocation_authority::invocation_generation(
                command.message.payload(),
                command.message.headers(),
            );
            let signer = state.renderer_signer(generation)?;
            if signer.generation() != operation.signer.generation() {
                return Err(
                    "renderer operation requires the current native identity generation".into(),
                );
            }
        }
        Ok(operation)
    }
}
