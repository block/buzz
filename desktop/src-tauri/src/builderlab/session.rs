//! Single memory-only authority shared by native auth IPC and AppState.
use crate::active_user_signer::ActiveUserSigner;
use chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

const STALE: &str = "native authentication canceled or expired";

/// Captured generation validity. Never resolves a replacement credential/identity.
#[derive(Clone, Debug)]
pub(crate) struct SessionValidity {
    pub(crate) generation: u64,
    pub(crate) cancel: CancellationToken,
    pub(crate) expires: DateTime<Utc>,
}
impl SessionValidity {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.cancel.is_cancelled() || Utc::now() >= self.expires {
            self.cancel.cancel();
            Err(STALE.into())
        } else {
            Ok(())
        }
    }
    pub(crate) async fn canceled(&self) {
        let delay = (self.expires - Utc::now()).to_std().unwrap_or_default();
        tokio::select! { _ = self.cancel.cancelled() => {}, _ = tokio::time::sleep(delay) => { self.cancel.cancel(); } }
    }
}

impl crate::active_user_signer::CapabilityLifetime for SessionValidity {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn check(&self) -> Result<(), String> {
        SessionValidity::check(self)
    }

    fn canceled(&self) -> nostr::util::BoxedFuture<'_, ()> {
        Box::pin(SessionValidity::canceled(self))
    }
}

#[derive(Clone, Default)]
pub(crate) struct BuilderlabSession {
    state: Arc<Mutex<AuthState>>,
    pub(crate) operation_generation: Arc<Mutex<u64>>,
}
#[derive(Default)]
struct AuthState {
    generation: u64,
    pending: Option<SessionValidity>,
    session: Option<StoredSession>,
    workspace: Option<(u64, String)>,
}
#[derive(Clone)]
pub(super) struct StoredSession {
    pub(super) credential: String,
    pub(super) info: super::BuilderlabAuthInfo,
    pub(super) validity: SessionValidity,
    pub(super) signer: Option<ActiveUserSigner>,
}
impl AuthState {
    fn invalidate(&mut self, operation_generation: &mut u64) {
        *operation_generation = operation_generation.wrapping_add(1);
        self.workspace = None;
        if let Some(p) = self.pending.take() {
            p.cancel.cancel();
        }
        if let Some(s) = self.session.take() {
            s.validity.cancel.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
    }
}
impl BuilderlabSession {
    /// Erase expired credentials when no synchronous effect owns admission.
    /// Reads inside an admitted effect must not recursively acquire that lock;
    /// they still reject expiry via SessionValidity, and the next read reaps it.
    fn reap_expired(&self) -> Result<(), String> {
        let mut operation = match self.operation_generation.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(()),
            Err(_) => return Err(STALE.into()),
        };
        let mut state = self.state.lock().map_err(|_| STALE)?;
        if state
            .session
            .as_ref()
            .is_some_and(|s| s.validity.check().is_err())
            || state.pending.as_ref().is_some_and(|p| p.check().is_err())
        {
            state.invalidate(&mut operation);
        }
        Ok(())
    }

    /// Caller retaining operation admission can validate without reacquiring it.
    pub(crate) fn check_signer(&self, signer: &ActiveUserSigner) -> Result<(), String> {
        let state = self.state.lock().map_err(|_| STALE)?;
        signer.check_valid()?;
        if signer.generation() != Some(state.generation)
            || state
                .session
                .as_ref()
                .and_then(|s| s.signer.as_ref())
                .is_none_or(|current| current.public_key() != signer.public_key())
        {
            return Err(STALE.into());
        }
        Ok(())
    }
    /// Publish activation only while the captured authenticated generation is current.
    pub(crate) fn activate_workspace(
        &self,
        signer: &ActiveUserSigner,
        relay: &str,
    ) -> Result<(), String> {
        let mut operation = self.operation_generation.lock().map_err(|_| STALE)?;
        let mut state = self.state.lock().map_err(|_| STALE)?;
        signer.check_valid()?;
        if signer.generation() != Some(state.generation) || state.session.is_none() {
            return Err(STALE.into());
        }
        if state.workspace.as_ref() != Some(&(state.generation, relay.to_owned())) {
            *operation = operation.wrapping_add(1);
            state.workspace = Some((state.generation, relay.to_owned()));
        }
        Ok(())
    }

    /// No HTTP, task join, or SQLite work is performed under the auth mutex.
    pub(crate) fn workspace_signer(
        &self,
        generation: Option<u64>,
        relay: &str,
    ) -> Result<ActiveUserSigner, String> {
        let state = self.state.lock().map_err(|_| STALE)?;
        let signer = state
            .session
            .as_ref()
            .and_then(|session| session.signer.clone())
            .ok_or("remote identity is signed out")?;
        if generation != Some(state.generation) {
            return Err(
                "renderer operation requires the current native identity generation".into(),
            );
        }
        if state.workspace.as_ref() != Some(&(state.generation, relay.to_owned())) {
            return Err("remote workspace is not active for this generation and relay".into());
        }
        signer.check_valid()?;
        Ok(signer)
    }

    pub(crate) fn workspace_active(&self) -> Result<bool, String> {
        self.reap_expired()?;
        let state = self.state.lock().map_err(|_| STALE)?;
        Ok(state.workspace.is_some()
            && state
                .session
                .as_ref()
                .is_some_and(|s| s.validity.check().is_ok()))
    }

    /// Reserve before listener/browser work. Replacement invalidates first.
    pub(super) fn begin(&self) -> Result<LoginAttempt, String> {
        let mut operation = self.operation_generation.lock().map_err(|_| STALE)?;
        let mut state = self.state.lock().map_err(|_| STALE)?;
        state.invalidate(&mut operation);
        let validity = SessionValidity {
            generation: state.generation,
            cancel: CancellationToken::new(),
            expires: Utc::now() + chrono::Duration::minutes(10),
        };
        state.pending = Some(validity.clone());
        Ok(LoginAttempt {
            owner: self.clone(),
            validity,
            committed: false,
        })
    }
    pub(crate) fn clear(&self) -> Result<(), String> {
        let mut operation = self.operation_generation.lock().map_err(|_| STALE)?;
        self.state
            .lock()
            .map_err(|_| STALE)?
            .invalidate(&mut operation);
        Ok(())
    }
    pub(super) fn cancel_login(&self) -> Result<(), String> {
        let mut operation = self.operation_generation.lock().map_err(|_| STALE)?;
        let mut state = self.state.lock().map_err(|_| STALE)?;
        if state.pending.is_some() {
            state.invalidate(&mut operation);
        }
        Ok(())
    }
    pub(super) fn clear_if_current(&self, generation: u64) -> Result<(), String> {
        let mut operation = self.operation_generation.lock().map_err(|_| STALE)?;
        let mut state = self.state.lock().map_err(|_| STALE)?;
        if state.generation == generation {
            state.invalidate(&mut operation);
        }
        Ok(())
    }
    pub(super) fn snapshot(&self) -> Result<Option<StoredSession>, String> {
        self.reap_expired()?;
        let state = self.state.lock().map_err(|_| STALE)?;
        Ok(state
            .session
            .as_ref()
            .filter(|s| s.validity.check().is_ok())
            .cloned())
    }
    pub(super) fn check(&self, validity: &SessionValidity) -> Result<(), String> {
        validity.check()?;
        let state = self.state.lock().map_err(|_| STALE)?;
        if state.generation != validity.generation {
            Err(STALE.into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn active_signer(&self) -> Result<ActiveUserSigner, String> {
        self.snapshot()?
            .and_then(|s| s.signer)
            .ok_or_else(|| "remote identity is signed out".into())
    }
    pub(crate) fn status(&self) -> Result<(&'static str, Option<String>, u64), String> {
        self.reap_expired()?;
        let state = self.state.lock().map_err(|_| STALE)?;
        let session = state
            .session
            .as_ref()
            .filter(|s| s.validity.check().is_ok());
        Ok((
            if session.is_some() {
                "authenticated"
            } else if state.pending.as_ref().is_some_and(|p| p.check().is_ok()) {
                "logging-in"
            } else {
                "signed-out"
            },
            session
                .and_then(|s| s.signer.as_ref())
                .map(|s| s.public_key().to_hex()),
            state.generation,
        ))
    }
}

/// Drop cancels abandoned IPC futures as well as failed login phases.
pub(super) struct LoginAttempt {
    owner: BuilderlabSession,
    pub(super) validity: SessionValidity,
    committed: bool,
}
impl LoginAttempt {
    pub(super) async fn run<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        self.owner.check(&self.validity)?;
        let result = tokio::select! { biased; _ = self.validity.canceled() => Err(STALE.into()), result = work => result };
        self.owner.check(&self.validity)?;
        result
    }
    pub(super) fn commit(
        mut self,
        session: StoredSession,
    ) -> Result<super::BuilderlabAuthInfo, String> {
        self.validity.check()?;
        session.validity.check()?;
        let _operation = self.owner.operation_generation.lock().map_err(|_| STALE)?;
        let mut state = self.owner.state.lock().map_err(|_| STALE)?;
        if state.generation != self.validity.generation {
            return Err(STALE.into());
        }
        let info = session.info.clone();
        state.pending = None;
        state.session = Some(session);
        self.committed = true;
        Ok(info)
    }
}
impl Drop for LoginAttempt {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.owner.clear_if_current(self.validity.generation);
        }
    }
}
