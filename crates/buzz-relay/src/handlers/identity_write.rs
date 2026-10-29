//! Bound a user's multi-transaction or external action with the existing lease.
use super::ingest::{event_write_error, IngestError};
use crate::state::AppState;
use buzz_core::TenantContext;
use nostr::PublicKey;
use std::{future::Future, sync::Arc};

pub(super) fn error(error: anyhow::Error) -> IngestError {
    if matches!(
        error.downcast_ref::<buzz_db::DbError>(),
        Some(buzz_db::DbError::UserDeletionRestricted)
    ) {
        event_write_error(buzz_db::DbError::UserDeletionRestricted)
    } else {
        tracing::warn!(%error, "identity write lease unavailable");
        IngestError::Internal("error: identity write lease unavailable".into())
    }
}

pub(super) async fn run<T>(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: PublicKey,
    operation: &str,
    action: impl Future<Output = Result<T, IngestError>>,
) -> Result<T, IngestError> {
    let lease =
        buzz_deletion::acquire_actor_serving_write(&state.db, tenant.community(), operation, actor)
            .await
            .map_err(error)?;
    let result = match lease.protect_landed(action).await {
        Ok((Err(denial), _)) => Ok(Err(denial)),
        Ok((Ok(value), verified)) => verified.map(|()| Ok(value)).map_err(error),
        Err(cause) => Err(error(cause)),
    };
    let finished = lease.finish().await.map_err(error);
    match result {
        Ok(Ok(value)) => {
            finished?;
            Ok(value)
        }
        Ok(Err(error)) | Err(error) => {
            if let Err(cleanup) = finished {
                tracing::warn!(
                    ?cleanup,
                    "identity lease cleanup failed; expiry retains drain proof"
                );
            }
            Err(error)
        }
    }
}

/// Fence a separately authenticated HTTP mutation for its complete lifetime.
#[allow(clippy::result_large_err)] // Axum handlers return their denial response directly.
pub(crate) async fn run_http<T>(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: PublicKey,
    operation: &str,
    action: impl Future<Output = Result<T, axum::response::Response>>,
) -> Result<T, axum::response::Response> {
    use axum::response::IntoResponse;
    let denial = |error: anyhow::Error| {
        let restricted = matches!(
            error.downcast_ref::<buzz_db::DbError>(),
            Some(buzz_db::DbError::UserDeletionRestricted)
        );
        if restricted {
            (
                axum::http::StatusCode::FORBIDDEN,
                "restricted: identity is restricted for user deletion",
            )
                .into_response()
        } else {
            tracing::warn!(%error, "HTTP identity write lease unavailable");
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "identity write lease unavailable",
            )
                .into_response()
        }
    };
    let lease =
        buzz_deletion::acquire_actor_serving_write(&state.db, tenant.community(), operation, actor)
            .await
            .map_err(denial)?;
    let result = match lease.protect_landed(action).await {
        Ok((Err(response), _)) => Ok(Err(response)),
        Ok((Ok(value), verified)) => verified.map(|()| Ok(value)).map_err(denial),
        Err(cause) => Err(denial(cause)),
    };
    let finished = lease.finish().await.map_err(denial);
    match result {
        Ok(Ok(value)) => {
            finished?;
            Ok(value)
        }
        Ok(Err(error)) | Err(error) => {
            let _ = finished;
            Err(error)
        }
    }
}
