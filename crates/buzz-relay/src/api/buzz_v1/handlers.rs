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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleStateRequest {
    channel_ids: Vec<String>,
}

/// 1..=MAX_STATE_CHANNELS unique ids in canonical lowercase form.
fn parse_lifecycle_ids(raw: &[String]) -> Option<Vec<Uuid>> {
    let ids = raw
        .iter()
        .map(|id| Uuid::parse_str(id).ok().filter(|u| u.to_string() == *id))
        .collect::<Option<Vec<_>>>()?;
    let unique: BTreeSet<_> = ids.iter().collect();
    ((1..=buzz_db::lifecycle::MAX_STATE_CHANNELS).contains(&ids.len()) && unique.len() == ids.len())
        .then_some(ids)
}

/// Current lifecycle state of the requested channels the actor created or
/// is an active member of, in one statement. An omitted id is unknown to
/// the actor. Body POST, so a large id list never becomes a long URI.
pub(super) async fn lifecycle_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response, Error> {
    if body.len() > 64 * 1024 {
        return Err(Error::invalid());
    }
    tokio::time::timeout(Duration::from_secs(8), async {
        let principal = auth::authorize(&state, &headers, &uri, "POST", Some(&body)).await?;
        let request: LifecycleStateRequest =
            serde_json::from_slice(&body).map_err(|_| Error::invalid())?;
        let ids = parse_lifecycle_ids(&request.channel_ids).ok_or_else(Error::invalid)?;
        let channels = state
            .db
            .lifecycle_state(
                principal.tenant.community(),
                &principal.actor.to_bytes(),
                &ids,
            )
            .await
            .map_err(|_| Error::unavailable())?;
        // Release rows only after admission and bans are rechecked.
        auth::recheck(&state, &headers, &principal).await?;
        auth::response(json!({ "channels": channels }))
    })
    .await
    .map_err(|_| Error::unavailable())?
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

    #[test]
    fn lifecycle_ids_are_canonical_unique_and_bounded() {
        let id = |n: u128| Uuid::from_u128(n).to_string();
        assert_eq!(
            parse_lifecycle_ids(&[id(1)]),
            Some(vec![Uuid::from_u128(1)])
        );
        assert_eq!(parse_lifecycle_ids(&[]), None);
        assert_eq!(parse_lifecycle_ids(&[id(1), id(1)]), None);
        assert_eq!(parse_lifecycle_ids(&[id(10).to_uppercase()]), None);
        assert_eq!(parse_lifecycle_ids(&[id(1).replace('-', "")]), None);
        assert_eq!(parse_lifecycle_ids(&[format!("{{{}}}", id(1))]), None);
        let max = buzz_db::lifecycle::MAX_STATE_CHANNELS as u128;
        let full: Vec<_> = (0..max).map(id).collect();
        assert_eq!(
            parse_lifecycle_ids(&full).map(|ids| ids.len()),
            Some(full.len())
        );
        let over: Vec<_> = (0..=max).map(id).collect();
        assert_eq!(parse_lifecycle_ids(&over), None);
    }

    /// The largest state response fits the response limit.
    #[test]
    fn largest_lifecycle_state_response_fits_the_response_limit() {
        let channels: Vec<_> = (0..buzz_db::lifecycle::MAX_STATE_CHANNELS)
            .map(|_| buzz_db::lifecycle::ChannelLifecycleState {
                id: Uuid::max(),
                archived_at: Some(i64::MAX),
                deleted: true,
                lifecycle_token: Some(Uuid::max()),
            })
            .collect();
        let bytes = serde_json::to_vec(&json!({ "channels": channels }))
            .unwrap()
            .len();
        assert!(bytes <= auth::MAX_RESPONSE_BYTES, "{bytes} bytes");
    }
}
