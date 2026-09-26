use super::auth::{self, Error};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::HeaderMap,
    response::Response,
};
use buzz_db::personal_read::{ReadIntent, MAX_CHANNELS, MAX_INTENTS};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SidebarQuery {
    limit: Option<usize>,
    cursor: Option<Uuid>,
}

pub(super) async fn sidebar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    query: Result<Query<SidebarQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Error> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let principal = auth::authorize(&state, &headers, &uri, "GET", None).await?;
        let Query(query) = query.map_err(|_| Error::invalid())?;
        if query
            .limit
            .is_some_and(|limit| !(1..=MAX_CHANNELS).contains(&limit))
        {
            return Err(Error::invalid());
        }
        let page = state
            .db
            .personal_read_sidebar(
                principal.tenant.community(),
                &principal.actor,
                state.config.buzz_v1_retention_seconds,
                query.limit.unwrap_or(MAX_CHANNELS),
                query.cursor,
            )
            .await
            .map_err(|_| Error::unavailable())?;
        auth::recheck(&state, &headers, &principal).await?;
        let channels: Vec<_> = page.channels.iter().map(|c| c.channel_id).collect();
        let memberships = state
            .db
            .membership_pairs(
                principal.tenant.community(),
                &channels,
                &[principal.actor.to_bytes().to_vec()],
            )
            .await
            .map_err(|_| Error::unavailable())?;
        if memberships.len() != channels.len() {
            return Err(Error::unavailable());
        }
        auth::response(page)
    })
    .await
    .map_err(|_| Error::unavailable())?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    intents: Vec<Value>,
}

pub(super) async fn write(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response, Error> {
    if body.len() > 64 * 1024 {
        return Err(Error::invalid());
    }
    let principal = auth::authorize(&state, &headers, &uri, "POST", Some(&body)).await?;
    let batch: Batch = serde_json::from_slice(&body).map_err(|_| Error::invalid())?;
    if batch.intents.is_empty() || batch.intents.len() > MAX_INTENTS {
        return Err(Error::invalid());
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut outcomes = Vec::with_capacity(batch.intents.len());
    for item in batch.intents {
        let Ok(intent) = serde_json::from_value::<ReadIntent>(item) else {
            outcomes.push(json!({"status":"invalid"}));
            continue;
        };
        // A deadline/DB failure after commit is ambiguous, not a false failure.
        // Earlier acknowledged commits survive all later projection/item failures.
        let result = tokio::time::timeout_at(deadline, async {
            auth::recheck(&state, &headers, &principal).await?;
            state
                .db
                .apply_personal_read_intent(principal.tenant.community(), &principal.actor, &intent)
                .await
                .map_err(|_| Error::unavailable())
        })
        .await;
        outcomes.push(match result {
            Ok(Ok(outcome)) => serde_json::to_value(outcome).map_err(|_| Error::unavailable())?,
            _ => json!({"status":"unknown","retryable":true}),
        });
    }
    auth::response(json!({"outcomes":outcomes,"projection_status":"not_requested"}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContextQuery {
    // JSON array carried as one URL-encoded, signed query parameter.
    targets: String,
}

pub(super) async fn contexts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    query: Result<Query<ContextQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Error> {
    use buzz_db::personal_read::{
        ContextQuery as Target, ContextState, MAX_CONTEXTS, MAX_CONTEXT_MESSAGES,
    };
    tokio::time::timeout(Duration::from_secs(8), async {
        if uri.to_string().len() > 16 * 1024 {
            return Err(Error::invalid());
        }
        let principal = auth::authorize(&state, &headers, &uri, "GET", None).await?;
        let Query(query) = query.map_err(|_| Error::invalid())?;
        let targets: Vec<Target> =
            serde_json::from_str(&query.targets).map_err(|_| Error::invalid())?;
        let valid_id = |id: &str| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit());
        if targets.is_empty()
            || targets.len() > MAX_CONTEXTS
            || targets.iter().map(|t| t.message_ids.len()).sum::<usize>() > MAX_CONTEXT_MESSAGES
            || targets.iter().any(|t| {
                t.message_ids.iter().any(|id| !valid_id(id))
                    || t.target.root_id.as_deref().is_some_and(|id| !valid_id(id))
            })
        {
            return Err(Error::invalid());
        }
        let mut page = state
            .db
            .personal_read_contexts(
                principal.tenant.community(),
                &principal.actor,
                state.config.buzz_v1_retention_seconds,
                &targets,
            )
            .await
            .map_err(|_| Error::unavailable())?;
        auth::recheck(&state, &headers, &principal).await?;
        let channels: Vec<_> = targets.iter().map(|t| t.target.channel_id).collect();
        let allowed = state
            .db
            .personal_read_accessible_contexts(
                principal.tenant.community(),
                &principal.actor,
                &channels,
            )
            .await
            .map_err(|_| Error::unavailable())?;
        for (target, result) in targets.iter().zip(&mut page.contexts) {
            if !allowed.contains(&target.target.channel_id) {
                *result = ContextState::Unavailable;
            }
        }
        auth::response(page)
    })
    .await
    .map_err(|_| Error::unavailable())?
}
