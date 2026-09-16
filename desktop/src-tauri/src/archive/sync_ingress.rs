//! Captured sync operation; no remote activation or renderer history contract.
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Runtime};
use tokio_util::sync::CancellationToken;

use crate::archive::{prepare, ArchiveBatchResult, ArchiveCandidate};
use crate::{active_user_signer::ActiveUserSigner, app_state::AppState};

/// Only ciphertext/data crosses a retry or generation boundary.
pub(crate) struct SyncOutcome {
    pub(crate) committed: ArchiveBatchResult,
    pub(crate) retry: Vec<ArchiveCandidate>,
}

impl SyncOutcome {
    pub(crate) fn retry(candidates: Vec<ArchiveCandidate>) -> Self {
        Self {
            committed: ArchiveBatchResult {
                persisted: 0,
                persisted_agent_metrics: 0,
                dropped: 0,
            },
            retry: candidates,
        }
    }
}

/// Per-run commit authority. Invalidation takes this same mutex before cancel;
/// a commit either finishes first or is rejected. Never held during HTTP/join.
#[derive(Default)]
pub(crate) struct SyncAdmission(Mutex<bool>);
impl SyncAdmission {
    pub(crate) fn revoke(&self) {
        if let Ok(mut revoked) = self.0.lock() {
            *revoked = true;
        }
    }
}

pub(crate) struct SyncScope {
    pub(crate) signer: ActiveUserSigner,
    pub(crate) relay: String,
    relay_api: String,
    pub(crate) admission: Arc<SyncAdmission>,
    pub(crate) cancel: CancellationToken,
}
impl SyncScope {
    pub(crate) fn capture(
        state: &AppState,
        cancel: CancellationToken,
        admission: Arc<SyncAdmission>,
    ) -> Result<Self, String> {
        // Match the PR1 local archive recovery exception, while remote mode
        // still requires its captured valid session (no local fallback).
        let _operation = state
            .operation_generation
            .lock()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            signer: state.legacy_local_signer()?,
            relay: crate::relay::relay_ws_url_with_override(state),
            relay_api: crate::relay::relay_api_base_url_with_override(state),
            admission,
            cancel,
        })
    }
    pub(crate) fn data_scope(&self) -> (String, String) {
        (self.signer.public_key().to_hex(), self.relay.clone())
    }
    fn check(&self) -> Result<(), String> {
        self.signer.check_valid()?;
        if self.cancel.is_cancelled() {
            return Err("archive sync canceled".into());
        }
        Ok(())
    }

    pub(crate) fn notify_current(&self, state: &AppState, notify: impl FnOnce()) {
        if let Ok(_permit) = self.admit(state) {
            notify();
        }
    }
    fn admit<'a>(&'a self, state: &'a AppState) -> Result<impl Sized + 'a, String> {
        let workspace = state
            .workspace_apply_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "workspace is changing; retry archive")?;
        let operation = state
            .operation_generation
            .lock()
            .map_err(|_| "operation lock poisoned")?;
        let relay = state
            .relay_url_override
            .lock()
            .map_err(|_| "archive relay lock poisoned")?;
        if relay.clone().unwrap_or_else(crate::relay::relay_ws_url) != self.relay {
            return Err("archive relay changed".into());
        }
        if state.is_remote_identity() {
            state.native_auth.check_signer(&self.signer)?;
        }
        let local = if state.is_remote_identity() {
            None
        } else {
            Some(state.archive_local_admission(self.signer.public_key())?)
        };
        let admission = self
            .admission
            .0
            .lock()
            .map_err(|_| "archive admission poisoned")?;
        if *admission {
            return Err("archive sync revoked".into());
        }
        self.check()?;
        Ok((workspace, operation, relay, local, admission))
    }

    /// Lock order: SQLite IMMEDIATE -> workspace TRY -> operation -> relay -> native auth
    /// (or local key) -> run admission. Existing workspace apply holds workspace
    /// over other work, so we MUST NOT wait for it while holding SQLite. None
    /// of the short authority locks are held during SQLite busy acquisition.
    /// Native begin/clear/clear_if_current/expiry all take the same AuthState
    /// mutex. Local replacement takes the key mutex; relay replacement takes
    /// the override mutex. Do not call active_signer/relay helpers while held.
    pub(crate) fn commit(
        &self,
        state: &AppState,
        batch: &prepare::PreparedBatch,
        conn: &rusqlite::Connection,
    ) -> Result<ArchiveBatchResult, String> {
        let owner = self.signer.public_key().to_hex();
        prepare::commit_ready_guarded(
            batch,
            &owner,
            &self.relay,
            crate::archive::now_secs(),
            conn,
            || self.admit(state),
            || self.check(),
        )
    }
}

/// Real sync ingress returns committed counters even on partial failure. The
/// compatibility IPC Result remains local-only and cannot enable remote reads.
/// Started blocking DB work is always awaited, never canceled by dropping its
/// future. Only HTTP preparation is cancel-selected, with original inputs held.
pub(crate) async fn archive_scoped<R: Runtime>(
    app: &AppHandle<R>,
    scope: Arc<SyncScope>,
    candidates: Vec<ArchiveCandidate>,
) -> SyncOutcome {
    let state = app.state::<AppState>();
    if scope.check().is_err() {
        return SyncOutcome::retry(candidates);
    }
    let originals = candidates.clone();
    let (owner, relay) = scope.data_scope();
    let plan = match state
        .archive_db
        .with_conn(move |conn| crate::archive::plan_archive(candidates, &owner, &relay, conn))
        .await
    {
        Ok(plan) => plan,
        Err(_) => return SyncOutcome::retry(originals),
    };
    let prepare = async {
        let buckets =
            crate::archive::query_buckets(plan.buckets, &state, &scope.relay_api, &scope.signer)
                .await;
        prepare::prepare_archive(buckets, plan.ephemeral, plan.pre_dropped, &scope.signer).await
    };
    let prepared = tokio::select! { biased;
        _ = scope.cancel.cancelled() => return SyncOutcome::retry(originals),
        result = scope.signer.run(async { Ok(prepare.await) }) => match result {
            Ok(batch) => batch,
            Err(_) => return SyncOutcome::retry(originals),
        }
    };
    let app = app.clone();
    let result = state
        .archive_db
        .with_conn(move |conn| {
            let committed = scope.commit(&app.state::<AppState>(), &prepared, conn)?;
            Ok(SyncOutcome {
                committed,
                retry: prepared.retry,
            })
        })
        .await;
    result.unwrap_or_else(|_| SyncOutcome::retry(originals))
}
