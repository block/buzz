//! Captured identity, relay and lifetime shared by media read transports.
use crate::app_state::AppState;
use crate::commands::media::{sign_blossom_get_auth_header, MEDIA_GET_AUTH_EXPIRY_SECS};

/// A media read never looks up replacement credentials after admission.
#[derive(Clone)]
pub(crate) struct MediaReadScope {
    operation: Option<crate::user_operation::UserOperationScope>,
    signer: Option<crate::active_user_signer::ActiveUserSigner>,
    pub(crate) base: String,
}

impl MediaReadScope {
    pub(crate) fn capture_renderer(
        state: &AppState,
        generation: Option<u64>,
    ) -> Result<Self, String> {
        let generation_guard = state
            .operation_generation
            .lock()
            .map_err(|e| e.to_string())?;
        state.media_read_scope(generation, *generation_guard)
    }

    pub(crate) fn new(
        signer: Option<crate::active_user_signer::ActiveUserSigner>,
        base: String,
        operation: Option<crate::user_operation::UserOperationScope>,
    ) -> Self {
        Self {
            signer,
            base,
            operation,
        }
    }

    pub(crate) fn requires_session(&self) -> bool {
        self.operation.is_some()
    }

    /// Command entrypoints capture before their first await. IPC admission is
    /// separate; this pins all downstream signing and HTTP to the same identity.
    pub(crate) fn capture_command(state: &AppState) -> Result<Self, String> {
        let generation = state.active_signer().ok().and_then(|s| s.generation());
        Self::capture_renderer(state, generation)
    }

    /// Only local recovery can read unsigned. Operational signing errors propagate.
    pub(crate) async fn authorization(&self) -> Result<Option<String>, String> {
        match &self.signer {
            Some(signer) => {
                match sign_blossom_get_auth_header(signer, &self.base, MEDIA_GET_AUTH_EXPIRY_SECS)
                    .await
                {
                    Ok(header) => Ok(Some(header)),
                    Err(error) if !self.requires_session() => {
                        eprintln!("buzz-desktop: media get auth signing failed (unsigned request): {error}");
                        Ok(None)
                    }
                    Err(error) => Err(error),
                }
            }
            None => Ok(None),
        }
    }

    /// Serialize a final native effect with session replacement. No await and no
    /// credential lookup is allowed inside this effect. Local behavior is unchanged.
    pub(crate) fn commit<T>(
        &self,
        state: &AppState,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let _admission = self
            .operation
            .as_ref()
            .map(|operation| operation.admit(state))
            .transpose()?;
        effect()
    }

    pub(crate) async fn run<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        match &self.signer {
            Some(signer) => signer.run(work).await,
            None => work.await,
        }
    }
}

impl<'de, R: tauri::Runtime> tauri::ipc::CommandArg<'de, R> for MediaReadScope {
    fn from_command(
        command: tauri::ipc::CommandItem<'de, R>,
    ) -> Result<Self, tauri::ipc::InvokeError> {
        let state = command
            .message
            .state_ref()
            .try_get::<AppState>()
            .ok_or_else(|| tauri::ipc::InvokeError::from("native app state unavailable"))?;
        let generation = crate::invocation_authority::invocation_generation(
            command.message.payload(),
            command.message.headers(),
        );
        Self::capture_renderer(&state, generation).map_err(Into::into)
    }
}
