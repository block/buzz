use super::auth::{self, Error};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::HeaderMap,
    response::Response,
};
use buzz_db::personal_read::{
    ChannelReadSummary, ReadIntent, SidebarPage, MAX_CHANNELS, MAX_INTENTS,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
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
        auth::response(release(&state, &headers, &principal, page).await?)
    })
    .await
    .map_err(|_| Error::unavailable())?
}

/// Release projected rows only after admission and the actor's membership of
/// every row are rechecked outside the projection snapshot.
async fn release(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    page: SidebarPage,
) -> Result<SidebarPage, Error> {
    auth::recheck(state, headers, principal).await?;
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
    Ok(page)
}

/// Updated rows for the channels a batch applied to. A channel the actor has
/// not joined has no row.
async fn refreshed(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    channels: &BTreeSet<Uuid>,
) -> Result<Vec<ChannelReadSummary>, Error> {
    let channels: Vec<_> = channels.iter().copied().collect();
    let mut rows = Vec::with_capacity(channels.len());
    for chunk in channels.chunks(MAX_CHANNELS) {
        let page = state
            .db
            .personal_read_sidebar_channels(
                principal.tenant.community(),
                &principal.actor,
                state.config.buzz_v1_retention_seconds,
                chunk,
            )
            .await
            .map_err(|_| Error::unavailable())?;
        rows.extend(release(state, headers, principal, page).await?.channels);
    }
    Ok(rows)
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
    let mut applied = BTreeSet::new();
    for item in batch.intents {
        let Ok(intent) = serde_json::from_value::<ReadIntent>(item) else {
            outcomes.push(json!({"status":"invalid"}));
            continue;
        };
        // A deadline/DB failure after commit is ambiguous, not a false failure.
        // Earlier acknowledged commits survive all later projection/item failures.
        let outcome = tokio::time::timeout_at(
            deadline,
            write_intent(&state, &headers, &principal, &intent),
        )
        .await
        .unwrap_or_else(|_| json!({"status":"unknown","retryable":true}));
        if outcome["status"] == "applied" {
            let ReadIntent::MarkThrough { target, .. } = &intent;
            applied.insert(target.channel_id);
        }
        outcomes.push(outcome);
    }
    let mut body = json!({ "outcomes": outcomes });
    // Committed outcomes stand even when their rows cannot be read in the
    // remaining budget: omit them.
    if let Ok(Ok(channels)) =
        tokio::time::timeout_at(deadline, refreshed(&state, &headers, &principal, &applied)).await
    {
        body["channels"] = json!(channels);
    }
    auth::response(body)
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

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_db::personal_read::{ThreadReadSummary, MAX_THREAD_SUMMARIES};

    /// A POST that applied to `MAX_INTENTS` channels, each with a full thread
    /// list, must still fit: an oversized body is a 503 that hides committed
    /// outcomes.
    #[test]
    fn largest_write_response_fits_the_response_limit() {
        let id = || Some("f".repeat(64));
        let channels: Vec<_> = (0..MAX_INTENTS)
            .map(|_| ChannelReadSummary {
                channel_id: Uuid::max(),
                unread: true,
                mentions: u32::MAX,
                read_through_id: id(),
                latest_id: id(),
                threads: (0..MAX_THREAD_SUMMARIES)
                    .map(|_| ThreadReadSummary {
                        root_id: "f".repeat(64),
                        unread: true,
                        mentions: u32::MAX,
                        read_through_id: id(),
                        latest_id: "f".repeat(64),
                    })
                    .collect(),
            })
            .collect();
        let outcomes = vec![json!({"status":"unknown","retryable":true}); MAX_INTENTS];
        let body = json!({ "outcomes": outcomes, "channels": channels });
        let bytes = serde_json::to_vec(&body).unwrap().len();
        assert!(bytes <= auth::MAX_RESPONSE_BYTES, "{bytes} bytes");
    }
}
