use super::auth::{self, Error};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::HeaderMap,
    response::Response,
};
use buzz_db::personal_read::{
    ChannelReadSummary, ReadIntent, ThreadsCursor, ThreadsPage, MAX_CHANNELS, MAX_COVERAGE_IDS,
    MAX_INTENTS, MAX_THREADS,
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
        .map_err(|error| {
            tracing::warn!(%error, "buzz_v1 sidebar projection failed");
            Error::unavailable()
        })?;
        let channels: Vec<_> = page.channels.iter().map(|c| c.channel_id).collect();
        release(&state, &headers, &principal, &channels).await?;
        auth::response(page)
    })
    .await
    .map_err(|_| Error::unavailable())?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThreadsQuery {
    limit: Option<usize>,
    cursor: Option<String>,
}

pub(super) async fn threads(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    query: Result<Query<ThreadsQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Error> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let principal = auth::authorize(&state, &headers, &uri, "GET", None).await?;
        let Query(query) = query.map_err(|_| Error::invalid())?;
        let limit = query.limit.unwrap_or(MAX_THREADS);
        let cursor = query
            .cursor
            .map(|c| ThreadsCursor::parse(&c).ok_or_else(Error::invalid))
            .transpose()?;
        if !(1..=MAX_THREADS).contains(&limit) {
            return Err(Error::invalid());
        }
        let page: ThreadsPage = state
            .db
            .personal_read_threads(
                principal.tenant.community(),
                &principal.actor,
                state.config.buzz_v1_retention_seconds,
                limit,
                cursor.as_ref(),
            )
            .await
            .map_err(|_| Error::unavailable())?;
        let channels: BTreeSet<_> = page.threads.iter().map(|t| t.channel_id).collect();
        let channels: Vec<_> = channels.into_iter().collect();
        release(&state, &headers, &principal, &channels).await?;
        auth::response(page)
    })
    .await
    .map_err(|_| Error::unavailable())?
}

/// Release projected rows only after admission and the actor's membership of
/// every row's channel are rechecked outside the projection snapshot.
async fn release(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    channels: &[Uuid],
) -> Result<(), Error> {
    auth::recheck(state, headers, principal)
        .await
        .inspect_err(|_| {
            tracing::warn!("buzz_v1 release: admission recheck failed");
        })?;
    let memberships = state
        .db
        .membership_pairs(
            principal.tenant.community(),
            channels,
            &[principal.actor.to_bytes().to_vec()],
        )
        .await
        .map_err(|_| Error::unavailable())?;
    if memberships.len() != channels.len() {
        tracing::warn!(
            rows = channels.len(),
            memberships = memberships.len(),
            "buzz_v1 release: membership changed since the snapshot"
        );
        return Err(Error::unavailable());
    }
    Ok(())
}

/// Updated rows for the channels a batch applied to. A channel the actor has
/// not joined has no row.
async fn refreshed(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    channels: &BTreeSet<Uuid>,
) -> Result<(Vec<ChannelReadSummary>, i64), Error> {
    let channels: Vec<_> = channels.iter().copied().collect();
    let mut rows = Vec::with_capacity(channels.len());
    let mut revision = 0;
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
        let ids: Vec<_> = page.channels.iter().map(|c| c.channel_id).collect();
        release(state, headers, principal, &ids).await?;
        revision = revision.max(page.revision);
        // A full follow list per row would exceed the response limit.
        rows.extend(page.channels.into_iter().map(|mut row| {
            row.followed_threads = None;
            row.followed_threads_truncated = None;
            row
        }));
    }
    Ok((rows, revision))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageRequest {
    message_ids: Vec<String>,
}

/// Whether each message is counted unread, by the sidebar's own rules.
pub(super) async fn coverage(
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
        let request: CoverageRequest =
            serde_json::from_slice(&body).map_err(|_| Error::invalid())?;
        let unique: std::collections::HashSet<_> = request
            .message_ids
            .iter()
            .map(|id| id.to_ascii_lowercase())
            .collect();
        if !(1..=MAX_COVERAGE_IDS).contains(&request.message_ids.len())
            || unique.len() != request.message_ids.len()
        {
            return Err(Error::invalid());
        }
        let answer = state
            .db
            .personal_read_coverage(
                principal.tenant.community(),
                &principal.actor,
                state.config.buzz_v1_retention_seconds,
                &request.message_ids,
            )
            .await
            .map_err(|error| match error {
                buzz_db::DbError::InvalidData(_) => Error::invalid(),
                _ => Error::unavailable(),
            })?;
        release(&state, &headers, &principal, &answer.channels).await?;
        auth::response(json!({ "messages": answer.messages, "read_revision": answer.revision }))
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
    let mut applied = BTreeSet::new();
    let mut changed = super::ping::Changes::default();
    let mut applied_count = 0_i64;
    let mut last_revision: Option<i64> = None;
    for item in batch.intents {
        let Ok(intent) = serde_json::from_value::<ReadIntent>(item) else {
            outcomes.push(json!({"status":"invalid"}));
            continue;
        };
        // A deadline/DB failure after commit is ambiguous, not a false failure.
        // Earlier acknowledged commits survive all later projection/item failures.
        let (outcome, intent_revision) = tokio::time::timeout_at(
            deadline,
            write_intent_with_revision(&state, &headers, &principal, &intent),
        )
        .await
        .unwrap_or_else(|_| (json!({"status":"unknown","retryable":true}), None));
        if let Some(r) = intent_revision {
            applied_count += 1;
            last_revision = Some(last_revision.map_or(r, |l: i64| l.max(r)));
        }
        if outcome["status"] == "applied" {
            applied.insert(intent.target().channel_id);
        }
        if outcome["status"] == "applied" || outcome["status"] == "unknown" {
            let target = intent.target();
            changed.channels.insert(target.channel_id);
            if let Some(root) = &target.root_id {
                changed
                    .threads
                    .insert((target.channel_id, root.to_ascii_lowercase()));
            }
        }
        outcomes.push(outcome);
    }
    // This POST moved the revision from `from` to `last` when its applied
    // intents were contiguous. Another writer interleaving makes `from`
    // larger than the true start, which a client reads as a gap and
    // reconciles with a full load: conservative, never a hidden gap.
    let revision_range = last_revision.map(|last| (last - applied_count, last));
    // After every commit and before the response, so a client that sees the
    // response and then reconnects cannot miss a change it caused.
    let ping_id = super::ping::send(
        &state,
        &principal.tenant,
        &principal.actor,
        &changed,
        revision_range,
    )
    .await;
    let mut body = json!({ "outcomes": outcomes });
    if let Some(ping_id) = ping_id {
        body["ping_id"] = json!(ping_id);
    }
    // Committed outcomes stand even when their rows cannot be read in the
    // remaining budget: omit them.
    if let Ok(Ok((channels, _))) =
        tokio::time::timeout_at(deadline, refreshed(&state, &headers, &principal, &applied)).await
    {
        body["channels"] = json!(channels);
    }
    // Omitted when nothing applied: there is no change to place.
    if let Some((from, last)) = revision_range {
        body["read_revision_from"] = json!(from);
        body["read_revision"] = json!(last);
    }
    auth::response(body)
}

// Preserve earlier committed outcomes while distinguishing a definite denial
// before the transaction from an ambiguous storage/timeout failure.
#[cfg(test)]
pub(super) async fn write_intent(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    intent: &ReadIntent,
) -> Value {
    write_intent_with_revision(state, headers, principal, intent)
        .await
        .0
}

/// The intent's outcome and, when it applied, the revision it committed.
async fn write_intent_with_revision(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    intent: &ReadIntent,
) -> (Value, Option<i64>) {
    if let Err(error) = auth::recheck(state, headers, principal).await {
        return if error.terminal_denial() {
            (json!({"status":"blocked"}), None)
        } else {
            (json!({"status":"unknown","retryable":true}), None)
        };
    }
    match state
        .db
        .apply_personal_read_intent_with_revision(
            principal.tenant.community(),
            &principal.actor,
            intent,
        )
        .await
    {
        Ok((outcome, revision)) => (json!(outcome), revision),
        Err(_) => (json!({"status":"unknown","retryable":true}), None),
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
                threads_truncated: true,
                followed_threads: None,
                followed_threads_truncated: None,
            })
            .collect();
        let outcomes = vec![json!({"status":"unknown","retryable":true}); MAX_INTENTS];
        let body = json!({ "outcomes": outcomes, "channels": channels });
        let bytes = serde_json::to_vec(&body).unwrap().len();
        assert!(bytes <= auth::MAX_RESPONSE_BYTES, "{bytes} bytes");
    }
}
