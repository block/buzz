//! NIP-45 COUNT handler — aggregate queries with channel access enforcement.

use std::sync::Arc;

use nostr::Filter;
use tracing::warn;

use crate::connection::{AuthState, ConnectionState};
use crate::handlers::req::is_author_only_event;
use crate::protocol::RelayMessage;
use crate::state::AppState;

/// Extract a channel UUID from a single filter's `#h` tag.
fn extract_channel_from_filter(filter: &Filter) -> Option<uuid::Uuid> {
    let h_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::H);
    filter.generic_tags.get(&h_tag).and_then(|vs| {
        if vs.len() == 1 {
            vs.iter().next()?.parse::<uuid::Uuid>().ok()
        } else {
            None
        }
    })
}

/// Handle a COUNT message: require auth, enforce channel access, execute filters,
/// return aggregate count.
pub async fn handle_count(
    sub_id: String,
    filters: Vec<Filter>,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
) {
    // Require auth
    let pubkey_bytes = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Authenticated(ctx) => ctx.pubkey.to_bytes().to_vec(),
            _ => {
                conn.send(RelayMessage::closed(
                    &sub_id,
                    "auth-required: not authenticated",
                ));
                return;
            }
        }
    };

    // P-gated kinds (gift wraps, member notifications, observer frames) require
    // the caller's own pubkey in the #p tag — same enforcement as WS REQ handler.
    let authed_pubkey_hex = hex::encode(&pubkey_bytes);
    if !super::req::p_gated_filters_authorized(&filters, &authed_pubkey_hex) {
        conn.send(RelayMessage::closed(
            &sub_id,
            "restricted: p-gated kinds require #p tag matching your pubkey",
        ));
        return;
    }
    if !super::req::engram_filters_authorized(&filters, &authed_pubkey_hex) {
        conn.send(RelayMessage::closed(
            &sub_id,
            "restricted: agent-engram reads require authors=[self] or #p=[self]",
        ));
        return;
    }
    if !super::req::author_only_filters_authorized(&filters, &authed_pubkey_hex) {
        conn.send(RelayMessage::closed(
            &sub_id,
            "restricted: author-only kinds require authors=[self]",
        ));
        return;
    }

    // Get channels this user can access — same enforcement as WS REQ handler.
    let accessible_channels = match state.get_accessible_channel_ids_cached(&pubkey_bytes).await {
        Ok(ids) => ids,
        Err(e) => {
            warn!(sub_id = %sub_id, "Failed to get accessible channels: {e}");
            conn.send(RelayMessage::closed(&sub_id, "error: database error"));
            return;
        }
    };

    // For each filter, count matching events with channel access enforcement.
    let mut total: u64 = 0;
    for filter in &filters {
        // Determine if this filter can match author-only kinds — if so, the
        // fast-path count_events() cannot be used because it doesn't do
        // per-event author filtering.
        let needs_author_only_filtering = super::req::filter_can_match_author_only_kinds(filter);

        if let Some(ch_id) = extract_channel_from_filter(filter) {
            // Filter targets a specific channel — verify access.
            if !accessible_channels.contains(&ch_id) {
                continue; // Skip filters targeting inaccessible channels.
            }
            // Channel is accessible — count with pushability check.
            let query =
                super::req::build_event_query_from_filter(filter, &pubkey_bytes, &state).await;
            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if super::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
            {
                match state.db.count_events(&query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        conn.send(RelayMessage::closed(&sub_id, &format!("error: {e}")));
                        return;
                    }
                }
            } else {
                // Fallback: query + post-filter for non-pushable constraints.
                let mut q = query;
                q.limit = Some(100_000);
                q.max_limit = Some(100_000);
                match state.db.query_events(&q).await {
                    Ok(stored_events) => {
                        for se in stored_events {
                            if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se)
                            {
                                continue;
                            }
                            if is_author_only_event(&se.event, &pubkey_bytes) {
                                continue;
                            }
                            total += 1;
                        }
                    }
                    Err(e) => {
                        conn.send(RelayMessage::closed(&sub_id, &format!("error: {e}")));
                        return;
                    }
                }
            }
        } else {
            // No channel filter — use SQL-level channel_ids pushdown to count
            // only events in accessible channels (+ global events).
            //
            // If the filter has generic tags beyond what SQL can push down
            // (#h, #p single, #d single, #e), we must fall back to
            // query + post-filter to avoid overcounting.
            let mut query =
                super::req::build_event_query_from_filter(filter, &pubkey_bytes, &state).await;
            query.channel_ids = Some(accessible_channels.to_vec());

            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if super::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
            {
                query.limit = None; // COUNT doesn't need a row limit
                match state.db.count_events(&query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        conn.send(RelayMessage::closed(&sub_id, &format!("error: {e}")));
                        return;
                    }
                }
            } else {
                // Fallback: query with high limit + post-filter for correctness.
                query.limit = Some(100_000);
                query.max_limit = Some(100_000);
                match state.db.query_events(&query).await {
                    Ok(stored_events) => {
                        for se in stored_events {
                            if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se)
                            {
                                continue;
                            }
                            if is_author_only_event(&se.event, &pubkey_bytes) {
                                continue;
                            }
                            total += 1;
                        }
                    }
                    Err(e) => {
                        conn.send(RelayMessage::closed(&sub_id, &format!("error: {e}")));
                        return;
                    }
                }
            }
        }
    }
    conn.send(RelayMessage::count(&sub_id, total));
}
