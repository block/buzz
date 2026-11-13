use super::auth::{self, Error};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::HeaderMap,
    response::Response,
};
use buzz_db::channel_heads::{ChannelHeads, MAX_CHANNEL_IDS};
use buzz_db::personal_read::{
    ChannelReadSummary, ReadIntent, SidebarPage, MAX_CHANNELS, MAX_INTENTS,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use uuid::Uuid;

/// 1..=MAX_CHANNELS unique UUIDs, comma-separated; anything else is invalid.
fn parse_channel_ids(value: &str) -> Option<Vec<Uuid>> {
    parse_ids(value, MAX_CHANNELS)
}

/// 1..=`max` unique UUIDs, comma-separated; anything else is invalid.
fn parse_ids(value: &str, max: usize) -> Option<Vec<Uuid>> {
    let ids = value
        .split(',')
        .map(|id| Uuid::parse_str(id).ok())
        .collect::<Option<Vec<_>>>()?;
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    ((1..=max).contains(&ids.len()) && unique.len() == ids.len()).then_some(ids)
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
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    query: Result<Query<SidebarQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Error> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let principal = auth::authorize(&state, &slot, &headers, &uri, "GET", None).await?;
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
pub(super) struct ChannelHeadsQuery {
    /// Return only rows changed at or after this `as_of`, Unix seconds.
    since: Option<i64>,
    /// Comma-separated channel UUIDs to read; exclusive with `since`.
    channel_ids: Option<String>,
}

/// Leaves headroom under [`auth::MAX_RESPONSE_BYTES`] for the envelope.
const CHANNEL_HEADS_BYTES: usize = auth::MAX_RESPONSE_BYTES - 1024;

/// Keep the most recent rows that fit the response limit. Dropped rows are
/// the oldest, and `truncated` tells the caller to ask for them by id.
fn fit_channel_heads(mut page: ChannelHeads) -> Result<ChannelHeads, Error> {
    let mut bytes = 0;
    for (index, row) in page.channels.iter().enumerate() {
        bytes += serde_json::to_vec(row)
            .map_err(|_| Error::unavailable())?
            .len()
            + 1;
        if bytes > CHANNEL_HEADS_BYTES {
            page.channels.truncate(index);
            page.truncated = true;
            break;
        }
    }
    Ok(page)
}

/// Every joined channel's stored row in one read: last activity and the
/// newest top-level message. Display state only; unread is not included.
pub(super) async fn channel_heads(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    query: Result<Query<ChannelHeadsQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Error> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let principal = auth::authorize(&state, &slot, &headers, &uri, "GET", None).await?;
        let Query(query) = query.map_err(|_| Error::invalid())?;
        let ids = match query.channel_ids {
            Some(ids) => Some(
                parse_ids(&ids, MAX_CHANNEL_IDS)
                    .filter(|_| query.since.is_none())
                    .ok_or_else(Error::invalid)?,
            ),
            None => None,
        };
        // Unix seconds through year 9999; anything else is not a timestamp.
        if query
            .since
            .is_some_and(|since| !(0..=253_402_300_799).contains(&since))
        {
            return Err(Error::invalid());
        }
        let page = state
            .db
            .channel_heads(
                principal.tenant.community(),
                &principal.actor,
                query.since,
                ids.as_deref(),
            )
            .await
            .map_err(|_| Error::unavailable())?;
        let page = fit_channel_heads(page)?;
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
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response, Error> {
    if body.len() > 64 * 1024 {
        return Err(Error::invalid());
    }
    let principal = auth::authorize(&state, &slot, &headers, &uri, "POST", Some(&body)).await?;
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

    /// Head text is bounded in characters, not bytes: a full list of worst
    /// case rows is cut to the newest that fit instead of becoming a 503.
    #[test]
    fn largest_channel_heads_response_is_cut_to_fit() {
        use buzz_db::channel_heads::{ChannelHead, Head, HEAD_TEXT_CHARS, MAX_CHANNELS};
        let row = |text: String| ChannelHead {
            channel_id: Uuid::max(),
            activity_at: Some(i64::MAX),
            activity_pubkey: Some("f".repeat(64)),
            head: Some(Head {
                id: "f".repeat(64),
                pubkey: "f".repeat(64),
                created_at: i64::MAX,
                text,
                edited: true,
            }),
        };
        let page = |text: &str| ChannelHeads {
            as_of: i64::MAX,
            channels: vec![row(text.repeat(HEAD_TEXT_CHARS)); MAX_CHANNELS],
            truncated: false,
        };
        // A control character is six bytes of JSON.
        let cut = fit_channel_heads(page("\u{1}")).ok().unwrap();
        assert!(cut.truncated && !cut.channels.is_empty() && cut.channels.len() < MAX_CHANNELS);
        assert!(serde_json::to_vec(&cut).unwrap().len() <= auth::MAX_RESPONSE_BYTES);
        let whole = fit_channel_heads(page("a")).ok().unwrap();
        assert!(!whole.truncated && whole.channels.len() == MAX_CHANNELS);
    }
}
