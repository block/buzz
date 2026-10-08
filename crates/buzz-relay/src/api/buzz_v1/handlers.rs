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

/// 1..=MAX_CHANNELS unique UUIDs, comma-separated; anything else is invalid.
fn parse_channel_ids(value: &str) -> Option<Vec<Uuid>> {
    let ids = value
        .split(',')
        .map(|id| Uuid::parse_str(id).ok())
        .collect::<Option<Vec<_>>>()?;
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    ((1..=MAX_CHANNELS).contains(&ids.len()) && unique.len() == ids.len()).then_some(ids)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SidebarQuery {
    limit: Option<usize>,
    cursor: Option<Uuid>,
    /// Comma-separated channel UUIDs to refresh; exclusive with paging.
    channel_ids: Option<String>,
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
        let community = principal.tenant.community();
        let retention = state.config.buzz_v1_retention_seconds;
        let page = match query.channel_ids {
            Some(ids) => {
                let ids = parse_channel_ids(&ids)
                    .filter(|_| query.limit.is_none() && query.cursor.is_none())
                    .ok_or_else(Error::invalid)?;
                state
                    .db
                    .personal_read_sidebar_channels(community, &principal.actor, retention, &ids)
                    .await
            }
            None => {
                state
                    .db
                    .personal_read_sidebar(
                        community,
                        &principal.actor,
                        retention,
                        query.limit.unwrap_or(MAX_CHANNELS),
                        query.cursor,
                    )
                    .await
            }
        }
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
        outcomes.push(
            tokio::time::timeout_at(
                deadline,
                write_intent(&state, &headers, &principal, &intent),
            )
            .await
            .unwrap_or_else(|_| json!({"status":"unknown","retryable":true})),
        );
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

// Preserve earlier committed outcomes while distinguishing a definite denial
// before the transaction from an ambiguous storage/timeout failure.
pub(super) async fn write_intent(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    intent: &ReadIntent,
) -> Value {
    if let Err(error) = auth::recheck(state, headers, principal).await {
        return if error.terminal_denial() {
            json!({"status":"blocked"})
        } else {
            json!({"status":"unknown","retryable":true})
        };
    }
    match state
        .db
        .apply_personal_read_intent(principal.tenant.community(), &principal.actor, intent)
        .await
    {
        Ok(outcome) => json!(outcome),
        Err(_) => json!({"status":"unknown","retryable":true}),
    }
}
