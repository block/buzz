//! NIP-CW thread-mode bridge adapter. Legacy thread and channel-window code stays separate.

use std::{collections::HashSet, time::Duration};

use axum::{http::StatusCode, Json};
use buzz_core::{thread_window::Request, TenantContext};
use buzz_db::thread_window::{AuxBudget, AuxQuery};
use serde_json::{json, Value};

use super::{event_in_accessible_channel, WINDOW_AUX_DELETE_KINDS, WINDOW_AUX_KINDS};
use crate::{
    api::{api_error, internal_error},
    state::AppState,
};

type Error = (StatusCode, Json<Value>);
/// Shared deadline across all thread-window filters in a /query request.
pub(super) const DEADLINE: Duration = Duration::from_secs(8);
const MAX_BYTES: usize = 8 * 1024 * 1024;
/// One ledger shared by all opted-in filters, including replica fallback.
#[derive(Default)]
pub(super) struct Budget {
    bytes: usize,
    aux: AuxBudget,
}

/// Validate before search/presence/other extension dispatch can swallow the
/// opt-in. Absent/false preserves legacy handling; any other value is invalid.
pub(super) fn parse(filters: &[Value]) -> Result<Vec<Option<Request>>, Error> {
    let mut count = 0;
    filters
        .iter()
        .map(|raw| match raw.get("thread_window") {
            None | Some(Value::Bool(false)) => Ok(None),
            Some(_) => {
                count += 1;
                if count > 4 {
                    return Err(api_error(
                        StatusCode::BAD_REQUEST,
                        "at most four thread windows per query",
                    ));
                }
                Request::parse(raw)
                    .map(Some)
                    .map_err(|e| api_error(StatusCode::BAD_REQUEST, &e))
            }
        })
        .collect()
}

fn unavailable(message: &str) -> Error {
    api_error(StatusCode::SERVICE_UNAVAILABLE, message)
}

fn database_error(context: &str, error: buzz_db::DbError) -> Error {
    if let buzz_db::DbError::ThreadWindowBudgetExceeded(_) = &error {
        return unavailable(&format!("{error}; reduce window work before retrying"));
    }
    // Pool acquisition and PostgreSQL's statement/lock budgets may expire
    // before the outer HTTP deadline. They are retryable, not internal faults.
    let timed_out = match &error {
        buzz_db::DbError::Sqlx(sqlx::Error::PoolTimedOut) => true,
        buzz_db::DbError::Sqlx(sqlx::Error::Database(error)) => {
            matches!(error.code().as_deref(), Some("57014" | "55P03"))
        }
        _ => false,
    };
    if timed_out {
        tracing::warn!(%error, context, "thread window database timeout");
        unavailable("thread database timeout; retry window")
    } else {
        internal_error(&format!("thread {context}: {error}"))
    }
}

fn append(events: &mut Vec<Value>, budget: &mut Budget, event: &nostr::Event) -> Result<(), Error> {
    let value = serde_json::to_value(event)
        .map_err(|e| internal_error(&format!("thread serialize: {e}")))?;
    budget.bytes = budget.bytes.saturating_add(value.to_string().len() + 1);
    if budget.bytes > MAX_BYTES {
        return Err(unavailable("thread window exceeds response byte budget"));
    }
    events.push(value);
    Ok(())
}

/// Authorize the entire batch against one writer access set, then refresh it
/// once before releasing any output. A later window must never suppress only
/// its own rows while releasing an earlier window built before revocation.
pub(super) async fn query_batch<'a>(
    state: &AppState,
    tenant: &TenantContext,
    reader: &nostr::PublicKey,
    requests: impl IntoIterator<Item = &'a Request>,
) -> Result<Vec<Value>, Error> {
    let accessible = state
        .db
        .get_accessible_channel_ids(tenant.community(), &reader.to_bytes())
        .await
        .map_err(|e| database_error("access", e))?;
    let mut budget = Budget::default();
    let mut events = Vec::new();
    for request in requests {
        if accessible.contains(&request.channel) {
            events.extend(query(state, tenant, reader, request, &accessible, &mut budget).await?);
        }
    }
    let current = state
        .db
        .get_accessible_channel_ids(tenant.community(), &reader.to_bytes())
        .await
        .map_err(|e| database_error("final access", e))?;
    // Grants can expose auxiliary events omitted from the original closure;
    // revocations can invalidate earlier windows or their cross-channel aux.
    if accessible.iter().collect::<HashSet<_>>() != current.iter().collect::<HashSet<_>>() {
        return Err(unavailable("thread authorization changed; retry query"));
    }
    Ok(events)
}

async fn query(
    state: &AppState,
    tenant: &TenantContext,
    reader: &nostr::PublicKey,
    request: &Request,
    accessible: &[uuid::Uuid],
    budget: &mut Budget,
) -> Result<Vec<Value>, Error> {
    let (window, mut session) = state
        .db
        .get_thread_window_with_session(tenant.community(), request)
        .await
        .map_err(|e| database_error("window", e))?;
    // An unsupported, missing or out-of-scope root is not a served window.
    // In particular, never sign false exhaustion for an unsupported root kind.
    if !window.root_in_channel {
        return Ok(vec![]);
    }
    let reader_bytes = reader.to_bytes();
    let visible = |se: &buzz_core::StoredEvent| {
        event_in_accessible_channel(se, accessible)
            && crate::handlers::req::event_visible_to_reader(&se.event, &reader_bytes)
    };
    let mut events = Vec::new();
    let page_start_bytes = budget.bytes;
    budget.bytes += 2;
    let mut targets = vec![request.root.clone()];
    for row in &window.rows {
        if !visible(row) {
            // This would contradict the SQL's channel and row-kind predicates.
            // Do not issue authoritative bounds for a mismatched selection.
            return Err(unavailable(
                "thread row authorization changed during selection",
            ));
        }
        targets.push(row.event.id.to_hex());
        append(&mut events, budget, &row.event)?;
    }
    if request.include_aux {
        let row_count = events.len();
        let original_targets = targets;
        // A writer retry at an older aux cursor alone would miss newer writer
        // edits. Restart both closure hops once after permanent degradation;
        // preserve the request ledger/deadline, never reset work allowances.
        'closure: loop {
            let mut targets = original_targets.clone();
            let mut seen = HashSet::new();
            for kinds in [&WINDOW_AUX_KINDS[..], &WINDOW_AUX_DELETE_KINDS[..]] {
                let mut next_targets = HashSet::new();
                // Bound SQL expression size, including deletion-of-aux fanout.
                for batch in targets.chunks(200) {
                    let mut query = AuxQuery {
                        community: tenant.community(),
                        targets: batch,
                        kinds,
                        accessible,
                        cursor: None,
                    };
                    loop {
                        let was_replica = session.is_replica();
                        let page = session
                            .thread_window_aux(&query, &mut budget.aux)
                            .await
                            .map_err(|e| database_error("auxiliary closure", e))?;
                        if was_replica && !session.is_replica() {
                            events.truncate(row_count);
                            continue 'closure;
                        }
                        next_targets.extend(page.target_ids);
                        for event in page.events {
                            if visible(&event) && seen.insert(event.event.id) {
                                append(&mut events, budget, &event.event)?;
                            }
                        }
                        let Some(cursor) = page.next_cursor else {
                            break;
                        };
                        if query.cursor.as_ref().is_some_and(|old| {
                            cursor.created_at > old.created_at
                                || (cursor.created_at == old.created_at && cursor.id <= old.id)
                        }) {
                            return Err(unavailable("thread auxiliary scan did not advance"));
                        }
                        query.cursor = Some(cursor);
                    }
                }
                targets = next_targets.into_iter().collect();
                targets.sort_unstable();
                if targets.is_empty() {
                    break;
                }
            }
            break;
        }
    }
    let tags = [
        [
            "d".to_string(),
            request.binding(tenant.host(), &reader.to_hex()),
        ],
        ["h".to_string(), request.channel.to_string()],
        ["e".to_string(), request.root.clone()],
    ]
    .into_iter()
    .map(nostr::Tag::parse)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| internal_error(&format!("thread bounds tags: {e}")))?;
    let bounds = nostr::EventBuilder::new(
        nostr::Kind::Custom(buzz_core::kind::KIND_THREAD_WINDOW_BOUNDS as u16),
        json!({"version":1,"direction":"older","has_more":window.has_more,
            "next_cursor":window.next_cursor})
        .to_string(),
    )
    .tags(tags)
    .sign_with_keys(&state.relay_keypair)
    .map_err(|e| internal_error(&format!("thread bounds sign: {e}")))?;
    append(&mut events, budget, &bounds)?;
    metrics::histogram!("buzz_thread_window_response_bytes")
        .record((budget.bytes - page_start_bytes) as f64);
    Ok(events)
}

#[cfg(test)]
mod postgres_tests;
