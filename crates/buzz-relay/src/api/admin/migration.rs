use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{access::AccessSlot, admission::migration, state::AppState};

use super::{auth, error::ApiError};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WriteLimit {
    events_per_minute: u32,
}

pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    uri: Uri,
    method: Method,
    headers: HeaderMap,
) -> Result<Json<WriteLimit>, ApiError> {
    auth::authorize_read(&state, &slot, &headers, &method, &uri)
        .await?
        .operator(&slot)?;

    let events_per_minute = migration::read_limit(&state.redis_pool)
        .await
        .map_err(storage_unavailable)?;
    Ok(Json(WriteLimit { events_per_minute }))
}

pub(super) async fn put(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WriteLimit>, ApiError> {
    let operator = auth::authorize_write(&state, &slot, &headers, &method, &uri, Some(&body))
        .await?
        .operator(&slot)?;

    let limit: WriteLimit = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid_body", "invalid migration write limit body"))?;
    if limit.events_per_minute > migration::MAX_EVENTS_PER_MINUTE {
        return Err(ApiError::bad_request(
            "invalid_limit",
            &format!(
                "eventsPerMinute must be between 0 and {}",
                migration::MAX_EVENTS_PER_MINUTE
            ),
        ));
    }

    migration::set_limit(&state.redis_pool, limit.events_per_minute)
        .await
        .map_err(storage_unavailable)?;
    tracing::info!(
        operator_pubkey = %hex::encode(operator.pubkey),
        events_per_minute = limit.events_per_minute,
        "migration write limit updated",
    );
    Ok(Json(limit))
}

fn storage_unavailable(error: redis::RedisError) -> ApiError {
    tracing::warn!(%error, "migration write limit storage unavailable");
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "migration_write_limit_unavailable",
        message: "migration write limit storage unavailable".to_owned(),
    }
}
