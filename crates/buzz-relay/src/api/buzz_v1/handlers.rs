use super::auth::{self, Error};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::HeaderMap,
    response::Response,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buzz_db::{
    activity::{self, Cursor, Marker, Page, Row, RowKind, Select, DEFAULT_LIMIT, MAX_LIMIT},
    personal_read::{ReadIntent, ReadTarget, MAX_INTENTS},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivityQuery {
    /// Comma-separated conversation and thread kinds, the whole list when
    /// absent.
    kinds: Option<String>,
    /// Cursor of the next older page.
    before: Option<String>,
    /// Marker of an earlier read: only what changed.
    since: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MentionsQuery {
    /// Cursor of the next older page.
    before: Option<String>,
    limit: Option<usize>,
    /// Only the mentions not yet read.
    unread: Option<bool>,
}

/// Unique kinds in wire order; `None` when a name is unknown, repeated, or
/// not a kind of the list.
fn parse_kinds(value: Option<&str>) -> Option<Vec<RowKind>> {
    let Some(value) = value else {
        return Some(RowKind::LIST.to_vec());
    };
    let kinds = value
        .split(',')
        .map(RowKind::parse)
        .collect::<Option<BTreeSet<_>>>()?;
    (kinds.len() == value.split(',').count() && !kinds.contains(&RowKind::Mention))
        .then(|| kinds.into_iter().collect())
}

fn event_id(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 || value.bytes().any(|b| !b.is_ascii_hexdigit()) {
        return None;
    }
    hex::decode(value).ok()?.try_into().ok()
}

/// What a cursor is bound to: it continues only the listing that issued it.
fn filter_tag(kinds: &[RowKind]) -> String {
    RowKind::ALL
        .iter()
        .enumerate()
        .filter(|(_, kind)| kinds.contains(kind))
        .fold(0u8, |mask, (bit, _)| mask | (1 << bit))
        .to_string()
}

fn encode_cursor(cursor: &Cursor, filter: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "{}.{}.{}.{filter}",
        cursor.micros, cursor.rank, cursor.id
    ))
}

fn decode_cursor(value: &str, filter: &str) -> Option<Cursor> {
    let text = String::from_utf8(URL_SAFE_NO_PAD.decode(value).ok()?).ok()?;
    let mut parts = text.split('.');
    let cursor = Cursor {
        micros: parts.next()?.parse().ok()?,
        rank: parts.next()?.parse().ok()?,
        id: parts.next()?.to_owned(),
    };
    let valid_id = Uuid::parse_str(&cursor.id).is_ok() || event_id(&cursor.id).is_some();
    (parts.next()? == filter && parts.next().is_none() && cursor.rank <= 2 && valid_id)
        .then_some(cursor)
}

fn parse_page(before: Option<String>, limit: Option<usize>, filter: &str) -> Option<Paging> {
    let before = match before {
        Some(before) => Some(decode_cursor(&before, filter)?),
        None => None,
    };
    let limit = limit.map_or(Some(DEFAULT_LIMIT), |limit| {
        (1..=MAX_LIMIT).contains(&limit).then_some(limit)
    })?;
    Some((before, limit))
}

type Paging = (Option<Cursor>, usize);

fn parse_query(query: ActivityQuery) -> Option<(activity::Query, String)> {
    let kinds = parse_kinds(query.kinds.as_deref())?;
    let filter = filter_tag(&kinds);
    let select = match (query.since, query.before) {
        (Some(since), None) if query.limit.is_none() => Select::Since(Marker::parse(&since)?),
        (None, before) => {
            let (before, limit) = parse_page(before, query.limit, &filter)?;
            Select::Page { before, limit }
        }
        _ => return None,
    };
    Some((activity::Query { kinds, select }, filter))
}

fn parse_mentions(query: MentionsQuery) -> Option<(activity::Query, &'static str)> {
    let unread = query.unread.unwrap_or(false);
    // An unread feed and a full one are different listings.
    let filter = if unread { "mu" } else { "m" };
    let (before, limit) = parse_page(query.before, query.limit, filter)?;
    // A mention's cursor names an event.
    if before.as_ref().is_some_and(|c| event_id(&c.id).is_none()) {
        return None;
    }
    let select = Select::Mentions {
        before,
        limit,
        unread,
    };
    let kinds = vec![RowKind::Mention];
    Some((activity::Query { kinds, select }, filter))
}

/// The response body. A listing too large for the response limit is cut and
/// continues from `next`; a `since` read too large asks the caller to start
/// over.
fn body(mut page: Page, filter: &str, since: bool) -> Value {
    loop {
        let value = json!({
            "rows": page.rows,
            "removed": page.removed,
            "totals": page.totals,
            "next": page.next.as_ref().map(|cursor| encode_cursor(cursor, filter)),
            "marker": page.marker.map(Marker::encode),
            "reset": page.reset,
            "imported": page.imported,
        });
        let fits =
            serde_json::to_vec(&value).is_ok_and(|bytes| bytes.len() <= auth::MAX_RESPONSE_BYTES);
        if fits || page.rows.len() <= 1 {
            return value;
        }
        if since {
            page = Page {
                reset: true,
                ..Page::default()
            };
            continue;
        }
        page.rows.truncate(page.rows.len() / 2);
        page.next = page.rows.last().map(Row::cursor);
    }
}

pub(super) async fn activity(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    query: Result<Query<ActivityQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Error> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let principal = auth::authorize(&state, &slot, &headers, &uri, "GET", None).await?;
        let Query(query) = query.map_err(|_| Error::invalid())?;
        let (query, filter) = parse_query(query).ok_or_else(Error::invalid)?;
        let page = read(&state, &headers, &principal, &query).await?;
        let since = matches!(query.select, Select::Since(_));
        auth::response(body(page, &filter, since))
    })
    .await
    .map_err(|_| Error::unavailable())?
}

/// `GET /me/mentions`: the person's mentions, newest first.
pub(super) async fn mentions(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    query: Result<Query<MentionsQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Error> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let principal = auth::authorize(&state, &slot, &headers, &uri, "GET", None).await?;
        let Query(query) = query.map_err(|_| Error::invalid())?;
        let (query, filter) = parse_mentions(query).ok_or_else(Error::invalid)?;
        let page = read(&state, &headers, &principal, &query).await?;
        let mut body = body(page, filter, false);
        // The feed has only rows and where they continue.
        if let Some(fields) = body.as_object_mut() {
            fields.retain(|name, _| name == "rows" || name == "next");
        }
        auth::response(body)
    })
    .await
    .map_err(|_| Error::unavailable())?
}

/// Read rows and release them only after admission and the actor's
/// membership of every row's channel are rechecked outside the read's
/// snapshot.
async fn read(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    query: &activity::Query,
) -> Result<Page, Error> {
    let page = state
        .db
        .activity(
            principal.tenant.community(),
            &principal.actor,
            state.config.buzz_v1_retention_seconds,
            query,
        )
        .await
        .map_err(|error| match error {
            buzz_db::DbError::InvalidData(_) | buzz_db::DbError::InvalidTimestamp(_) => {
                Error::invalid()
            }
            _ => Error::unavailable(),
        })?;
    auth::recheck(state, headers, principal).await?;
    let channels: Vec<Uuid> = page
        .rows
        .iter()
        .map(|row| row.channel_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
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

/// Updated rows for the contexts a batch applied to. A context in a channel
/// the actor has not joined has no row.
async fn refreshed(
    state: &AppState,
    headers: &HeaderMap,
    principal: &auth::Principal,
    applied: &[ReadTarget],
) -> Result<Vec<Row>, Error> {
    let mut channels = BTreeSet::new();
    let mut roots = BTreeSet::new();
    for target in applied {
        match target.root_id.as_deref().map(event_id) {
            None => channels.insert(target.channel_id),
            Some(Some(root)) => roots.insert(root),
            Some(None) => false,
        };
    }
    if channels.is_empty() && roots.is_empty() {
        return Ok(Vec::new());
    }
    let query = activity::Query {
        kinds: vec![
            RowKind::Channel,
            RowKind::Dm,
            RowKind::Solo,
            RowKind::Thread,
        ],
        select: Select::Ids {
            channels: channels.into_iter().collect(),
            roots: roots.into_iter().collect(),
        },
    };
    Ok(read(state, headers, principal, &query).await?.rows)
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
    let mut applied = Vec::new();
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
        if let Some(target) = intent.target().filter(|_| outcome["status"] == "applied") {
            if !applied.contains(target) {
                applied.push(target.clone());
            }
        }
        outcomes.push(outcome);
    }
    let mut body = json!({ "outcomes": outcomes });
    // Committed outcomes stand even when their rows cannot be read in the
    // remaining budget: omit them. The person's other devices learn of the
    // change from their next `since` read.
    if let Ok(Ok(rows)) =
        tokio::time::timeout_at(deadline, refreshed(&state, &headers, &principal, &applied)).await
    {
        body["rows"] = json!(rows);
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

    fn query(kinds: Option<&str>, before: Option<String>) -> ActivityQuery {
        ActivityQuery {
            kinds: kinds.map(str::to_owned),
            before,
            since: None,
            limit: None,
        }
    }

    fn feed(before: Option<String>, unread: bool) -> MentionsQuery {
        MentionsQuery {
            before,
            limit: None,
            unread: unread.then_some(true),
        }
    }

    #[test]
    fn a_cursor_continues_only_the_listing_that_issued_it() {
        let cursor = Cursor {
            micros: 1_700_000_000_123_456,
            rank: 1,
            id: "ab".repeat(32),
        };
        let (_, filter) = parse_query(query(Some("thread,dm"), None)).unwrap();
        let wire = encode_cursor(&cursor, &filter);
        let (parsed, _) = parse_query(query(Some("dm,thread"), Some(wire.clone()))).unwrap();
        assert!(matches!(
            parsed.select,
            Select::Page { before: Some(before), limit: DEFAULT_LIMIT } if before == cursor
        ));
        assert!(parse_query(query(Some("thread"), Some(wire.clone()))).is_none());
        assert!(parse_query(query(None, Some(wire.clone()))).is_none());
        assert!(parse_mentions(feed(Some(wire), false)).is_none());
    }

    #[test]
    fn mentions_are_a_feed_of_their_own() {
        // The list is conversations and threads; a mention is not one of its kinds.
        let (list, _) = parse_query(query(None, None)).unwrap();
        assert_eq!(list.kinds, RowKind::LIST);
        assert!(parse_query(query(Some("mention"), None)).is_none());
        assert!(parse_query(query(Some("thread,mention"), None)).is_none());

        let (page, all) = parse_mentions(MentionsQuery {
            limit: Some(50),
            ..feed(None, false)
        })
        .unwrap();
        assert!(matches!(
            page.select,
            Select::Mentions {
                before: None,
                limit: 50,
                unread: false
            }
        ));
        let (page, unread) = parse_mentions(feed(None, true)).unwrap();
        assert!(matches!(page.select, Select::Mentions { unread: true, .. }));
        // A cursor of the full feed does not continue the unread one, and a
        // mention's cursor names an event.
        let cursor = Cursor {
            micros: 1_700_000_000_123_456,
            rank: 2,
            id: "ab".repeat(32),
        };
        let wire = encode_cursor(&cursor, all);
        assert!(parse_mentions(feed(Some(wire.clone()), false)).is_some());
        assert!(parse_mentions(feed(Some(wire), true)).is_none());
        let channel = Cursor {
            id: Uuid::nil().to_string(),
            ..cursor
        };
        assert!(parse_mentions(feed(Some(encode_cursor(&channel, unread)), true)).is_none());
    }

    #[test]
    fn selectors_are_exclusive_and_bounded() {
        let with = |edit: fn(&mut ActivityQuery)| {
            let mut q = query(None, None);
            edit(&mut q);
            parse_query(q).is_some()
        };
        assert!(with(|q| q.since = Some("3.1700000000000000".into())));
        assert!(!with(|q| q.since = Some("x".into())));
        assert!(!with(|q| {
            q.since = Some("3.1700000000000000".into());
            q.limit = Some(5);
        }));
        assert!(!with(|q| {
            q.since = Some("3.1700000000000000".into());
            q.before = Some("x".into());
        }));
        assert!(with(|q| q.limit = Some(MAX_LIMIT)));
        assert!(!with(|q| q.limit = Some(MAX_LIMIT + 1)));
        assert!(!with(|q| q.limit = Some(0)));
        assert!(!with(|q| q.kinds = Some("channel,channel".into())));
        assert!(!with(|q| q.kinds = Some("sidebar".into())));
    }
}
