//! Exact, authorized governance-head reads, including authenticated absence.
use crate::{api::api_error, state::AppState};
use axum::{http::StatusCode, Json};
use buzz_core::{governance::DocumentId, TenantContext};
use serde_json::{json, Value};

pub(super) fn requested(filters: &[Value]) -> bool {
    filters.iter().any(|f| f.get("governance_head").is_some())
}

pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    actor: &nostr::PublicKey,
    grant: Option<&buzz_core::governance::GitGrant>,
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let invalid = || {
        api_error(
            StatusCode::BAD_REQUEST,
            "governance_head requires one exact version-1 document filter",
        )
    };
    let id: DocumentId = filters
        .first()
        .and_then(|v| v.get("document"))
        .and_then(Value::as_str)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    if filters != [json!({"governance_head": 1, "document": id.to_string()})] {
        return Err(invalid());
    }
    let head = state
        .db
        .governance_head(tenant.community(), &id, actor.as_bytes(), grant)
        .await
        .map_err(|error| match error {
            buzz_db::DbError::AccessDenied(_) => {
                api_error(StatusCode::FORBIDDEN, "restricted: governance read denied")
            }
            error => {
                tracing::error!(%error, "governance read failed");
                api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "governance head unavailable",
                )
            }
        })?;
    if let Some(repo) = id.repo() {
        record_read(state, tenant, repo, actor).await?;
    }
    Ok(Json(
        json!({"governance_head": 1, "community_id": tenant.community().as_uuid(), "document": id.to_string(), "head": head}),
    ))
}

/// Record a state read of a governed repository before it is shown
/// ([`crate::api::git::reads`]); no record, no read.
pub(super) async fn record_read(
    state: &AppState,
    tenant: &TenantContext,
    repo: &buzz_core::governance::RepoCoordinate,
    actor: &nostr::PublicKey,
) -> Result<(), (StatusCode, Json<Value>)> {
    use crate::api::git::reads;
    reads::record(
        &state.db,
        tenant.community(),
        repo,
        actor.as_bytes(),
        reads::Door::State,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "governed read record failed");
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "repository read unavailable",
        )
    })
}
