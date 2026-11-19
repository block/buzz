//! Raw artifact filters must be parsed before nostr::Filter discards extensions.
use super::api_error;
use crate::state::AppState;
use axum::{http::StatusCode, Json};
use buzz_core::artifact::{route_filter, FilterRoute};
use buzz_core::TenantContext;
use serde_json::Value;
use std::sync::Arc;

/// Serve explicit artifact queries; `None` leaves the request to the generic
/// path (including existing extensions such as `#buzz-channel`). `routed`
/// runs once the request is known to be an artifact read, before any of its
/// work awaits, so a request dropped mid-read is still described.
pub(super) async fn query(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    caller: &super::bridge::ProvenCaller,
    filters: &[Value],
    count: bool,
    routed: impl FnOnce(),
) -> Option<Result<Json<Value>, (StatusCode, Json<Value>)>> {
    let mut artifact = Ok(false);
    for filter in filters {
        match route_filter(filter) {
            FilterRoute::Generic => {}
            FilterRoute::Artifact => artifact = Ok(true),
            FilterRoute::Rejected(reason) => {
                artifact = Err(reason);
                break;
            }
        }
    }
    match artifact {
        Ok(false) => None,
        Ok(true) => {
            routed();
            Some(execute(state, tenant, caller, filters, count).await)
        }
        Err(reason) => {
            routed();
            Some(Err(api_error(StatusCode::BAD_REQUEST, reason)))
        }
    }
}

async fn execute(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    caller: &super::bridge::ProvenCaller,
    filters: &[Value],
    count: bool,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let reader = &caller.pubkey;
    // One filter avoids ambiguous OR-union count/page semantics. AND/OR inside
    // exact predicates still supports cross-channel multi-value lists.
    if filters.len() != 1 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "artifact queries require exactly one filter",
        ));
    }
    if filters[0].get("artifact").and_then(Value::as_str) == Some("incremental") {
        if count {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "incremental requires /query",
            ));
        }
        return super::artifact_sync::query(state, tenant, reader, &filters[0]).await;
    }
    let query = buzz_core::artifact::parse_query(&filters[0])
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    if query.scope == buzz_core::artifact::ArtifactScope::Community {
        if count {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "community artifact count is unsupported",
            ));
        }
        if caller.signed_at.is_none() {
            return Err(api_error(
                StatusCode::UNAUTHORIZED,
                "community artifacts require signed HTTP authentication",
            ));
        }
        if !crate::handlers::community_artifact::enabled(state)
            .await
            .unwrap_or(false)
        {
            return Err(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "community artifacts unavailable",
            ));
        }
        let principal = buzz_db::community_artifact::CommunityArtifactPrincipal {
            owner_admission: true,
            pubkey: reader.to_bytes().to_vec(),
            verified_owner: caller
                .token_owner
                .or(caller.artifact_owner)
                .map(|p| p.to_bytes().to_vec()),
            require_membership: state.config.require_relay_membership,
            reject_required_nip_fi: false,
        };
        if query.view == buzz_core::artifact::ArtifactView::Policy {
            let artifact_type = buzz_core::artifact::exact_type(&query.tags)
                .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
            let policy = state
                .db
                .community_artifact_policy(tenant.community(), &principal, artifact_type)
                .await
                .map_err(community_error)?;
            return Ok(Json(serde_json::json!(policy)));
        }
        let events = state
            .db
            .query_community_artifacts(tenant.community(), &principal, &query)
            .await
            .map_err(community_error)?;
        return Ok(Json(Value::Array(
            events
                .into_iter()
                .map(|e| serde_json::json!(e.event))
                .collect(),
        )));
    }
    // The query's statement_timeout is the enforced execution budget.
    let (events, n) = state
        .db
        .query_artifacts(tenant.community(), reader.as_bytes(), &query, count)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "artifact query failed or execution budget exceeded",
            )
        })?;
    if count {
        return Ok(Json(serde_json::json!({"count":n})));
    }
    let events = events
        .into_iter()
        .map(|e| serde_json::to_value(e.event))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "artifact serialization failed",
            )
        })?;
    Ok(Json(Value::Array(events)))
}

fn community_error(error: buzz_db::DbError) -> (StatusCode, Json<Value>) {
    let status = if matches!(error, buzz_db::DbError::AccessDenied(_)) {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    api_error(status, "community artifact read denied or unavailable")
}
