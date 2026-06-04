//! REQ handler — subscribe, deliver historical events, then EOSE.

use std::collections::HashSet;
use std::sync::Arc;

use tracing::{debug, warn};

use hex;
use nostr::Filter;
use sprout_core::filter::filters_match;
use sprout_core::kind::{
    KIND_AGENT_ENGRAM, KIND_AGENT_OBSERVER_FRAME, KIND_DM_VISIBILITY, KIND_GIFT_WRAP,
    KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION,
};
use sprout_db::EventQuery;

use sprout_auth::Scope;

use crate::connection::{AuthState, ConnectionState};
use crate::protocol::RelayMessage;
use crate::state::AppState;

const MAX_HISTORICAL_LIMIT: i64 = 10_000;
const MAX_SUBSCRIPTIONS: usize = 1024;
const P_GATED_KINDS: [u32; 5] = [
    KIND_AGENT_OBSERVER_FRAME,
    KIND_MEMBER_ADDED_NOTIFICATION,
    KIND_MEMBER_REMOVED_NOTIFICATION,
    KIND_GIFT_WRAP,
    KIND_DM_VISIBILITY,
];

/// Handle a REQ message: register the subscription, deliver historical events, then send EOSE.
pub async fn handle_req(
    sub_id: String,
    filters: Vec<Filter>,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
) {
    let (conn_id, pubkey_bytes, token_channel_ids) = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Authenticated(ctx) => {
                if !ctx.scopes.is_empty() && !ctx.scopes.contains(&Scope::MessagesRead) {
                    conn.send(RelayMessage::notice("restricted: insufficient scope"));
                    conn.send(RelayMessage::closed(
                        &sub_id,
                        "restricted: insufficient scope",
                    ));
                    return;
                }

                let pk_bytes = ctx.pubkey.to_bytes().to_vec();

                let subs = conn.subscriptions.lock().await;
                if !subs.contains_key(&sub_id) && subs.len() >= MAX_SUBSCRIPTIONS {
                    conn.send(RelayMessage::closed(
                        &sub_id,
                        "error: too many subscriptions",
                    ));
                    return;
                }

                (conn.conn_id, pk_bytes, ctx.channel_ids.clone())
            }
            _ => {
                conn.send(RelayMessage::notice(
                    "auth-required: authenticate before subscribing",
                ));
                conn.send(RelayMessage::closed(
                    &sub_id,
                    "auth-required: not authenticated",
                ));
                return;
            }
        }
    };

    let mut accessible_channels = match state.get_accessible_channel_ids_cached(&pubkey_bytes).await
    {
        Ok(ids) => ids,
        Err(e) => {
            warn!(conn_id = %conn_id, "Failed to get accessible channels: {e}");
            conn.send(RelayMessage::closed(&sub_id, "error: database error"));
            return;
        }
    };
    if let Some(allowed) = token_channel_ids.as_deref() {
        accessible_channels.retain(|channel_id| allowed.contains(channel_id));
    }

    let channel_id = extract_channel_id_from_filters(&filters);

    // ── #p / engram gating for globally-stored sensitive kinds ───────────────
    // Applied BEFORE the NIP-50 search branch so that an authenticated member
    // cannot use `{"search":"...","kinds":[30174]}` (or similar for p-gated
    // kinds) to harvest indexed-but-globally-stored sensitive events. Search
    // hits are looked up by event id and returned without the per-filter
    // post-check the historical-delivery branch applies, so the gate must run
    // here, up front. Only applies to GLOBAL subscriptions (channel_id = None):
    // channel-scoped subs can never receive globally-stored events because of
    // the fan_out() invariant in subscription.rs.
    if channel_id.is_none() {
        let authed_pubkey_hex = hex::encode(&pubkey_bytes);
        if !p_gated_filters_authorized(&filters, &authed_pubkey_hex) {
            conn.send(RelayMessage::closed(
                &sub_id,
                "restricted: p-gated events require #p matching your pubkey",
            ));
            return;
        }
        if !engram_filters_authorized(&filters, &authed_pubkey_hex) {
            conn.send(RelayMessage::closed(
                &sub_id,
                "restricted: agent-engram reads require authors=[self] or #p=[self]",
            ));
            return;
        }
    }

    // ── NIP-50 search: one-shot, no persistent subscription ──────────────────
    // Search filters hit Typesense and return historical hits, then EOSE.
    // They are not registered for fan-out. The sensitive-kind gates above
    // already ran, so an authed member cannot use search to bypass author/#p
    // rules for kind:30174 or other globally-stored gated kinds.
    let has_search = filters.iter().any(|f| f.search.is_some());
    if has_search {
        if filters.iter().any(|f| f.search.is_none()) {
            conn.send(RelayMessage::closed(
                &sub_id,
                "error: mixed search and non-search filters not supported",
            ));
            return;
        }
        handle_search_req(
            &sub_id,
            &filters,
            &accessible_channels,
            token_channel_ids.is_none(),
            &hex::encode(&pubkey_bytes),
            &conn,
            &state,
        )
        .await;
        return;
    }

    // Check channel access BEFORE registering the subscription.
    if let Some(ch_id) = channel_id {
        if !accessible_channels.contains(&ch_id) {
            conn.send(RelayMessage::closed(
                &sub_id,
                "restricted: not a channel member",
            ));
            return;
        }
    }

    {
        let mut subs = conn.subscriptions.lock().await;
        subs.insert(sub_id.clone(), filters.clone());
    }

    state
        .sub_registry
        .register(conn_id, sub_id.clone(), filters.clone(), channel_id);

    debug!(conn_id = %conn_id, sub_id = %sub_id, "Subscription registered");

    // NIP-01 OR semantics: execute one DB query per filter and deduplicate results
    // by event ID. Collapsing all filters into a single query would merge their
    // time windows and limits, causing under-fetching when filters have different
    // per-filter limits or non-overlapping time windows.
    let mut seen_ids: HashSet<nostr::EventId> = HashSet::new();
    let mut total_sent: usize = 0;
    let viewer_hex = hex::encode(&pubkey_bytes);

    for filter in &filters {
        // Use per-filter #h channel scope when available, falling back to the
        // subscription-level channel_id. This prevents unrelated accessible-channel
        // rows from consuming the LIMIT when filters target specific channels but
        // the subscription is global (multiple distinct #h values across filters).
        let per_filter_channel = {
            let h = nostr::SingleLetterTag::lowercase(nostr::Alphabet::H);
            filter
                .generic_tags
                .get(&h)
                .and_then(|vs| {
                    if vs.len() == 1 {
                        vs.iter().next()?.parse::<uuid::Uuid>().ok()
                    } else {
                        None
                    }
                })
                .or(channel_id)
        };
        let params = filter_to_query_params(filter, per_filter_channel);

        let filter_events = state.db.query_events(&params).await;

        let events = match filter_events {
            Ok(evs) => evs,
            Err(e) => {
                warn!(conn_id = %conn_id, sub_id = %sub_id, "Historical query failed: {e}");
                conn.send(RelayMessage::eose(&sub_id));
                return;
            }
        };

        for stored in &events {
            // Per-filter NIP-01 matching — use the current filter only, not the
            // full filter set. OR semantics across filters are handled by the outer
            // loop (each filter gets its own DB query).
            if !filters_match(std::slice::from_ref(filter), stored) {
                continue;
            }

            if let Some(ch_id) = stored.channel_id {
                if !accessible_channels.contains(&ch_id) {
                    continue;
                }
            }

            // Result-level read auth: a viewer-private snapshot (kind:30622) is
            // delivered only to its owner, even if reached via a kindless
            // `ids:[…]` subscription that skips the filter-level `#p` gate.
            if !sprout_core::filter::reader_authorized_for_event(&stored.event, &viewer_hex) {
                continue;
            }

            // Dedup AFTER acceptance — an event that fails filter A's constraints
            // must remain eligible for filter B (NIP-01 OR semantics).
            if !seen_ids.insert(stored.event.id) {
                continue;
            }

            let msg = RelayMessage::event(&sub_id, &stored.event);
            if !conn.send(msg) {
                return;
            }
            total_sent += 1;
        }
    }

    conn.send(RelayMessage::eose(&sub_id));

    debug!(
        conn_id = %conn_id,
        sub_id = %sub_id,
        count = total_sent,
        "EOSE sent after historical delivery"
    );
}

/// Handle a NIP-50 search REQ: query Typesense, fetch full events, deliver results, EOSE.
/// Search subscriptions are one-shot — no persistent subscription is registered.
/// Maximum Typesense pages to fetch per filter (prevents unbounded loops).
const MAX_SEARCH_PAGES: u32 = 10;

pub(crate) fn build_search_channel_scope_filter(
    accessible_channels: &[uuid::Uuid],
    include_global: bool,
) -> Option<String> {
    if accessible_channels.is_empty() {
        return if include_global {
            Some("channel_id:=__global__".to_string())
        } else {
            None
        };
    }

    let ids: Vec<String> = accessible_channels
        .iter()
        .map(|id| id.to_string())
        .collect();
    Some(if include_global {
        format!(
            "(channel_id:=[{}] || channel_id:=__global__)",
            ids.join(",")
        )
    } else {
        format!("channel_id:=[{}]", ids.join(","))
    })
}

async fn handle_search_req(
    sub_id: &str,
    filters: &[Filter],
    accessible_channels: &[uuid::Uuid],
    include_global: bool,
    reader_pubkey_hex: &str,
    conn: &ConnectionState,
    state: &AppState,
) {
    let all_channels_filter =
        match build_search_channel_scope_filter(accessible_channels, include_global) {
            Some(filter) => filter,
            None => {
                conn.send(RelayMessage::eose(sub_id));
                return;
            }
        };

    let mut seen_ids: HashSet<nostr::EventId> = HashSet::new();

    for filter in filters {
        let search_text = match &filter.search {
            Some(s) if !s.is_empty() => s.clone(),
            _ => continue,
        };

        let limit = filter
            .limit
            .map(|l| (l as u32).min(MAX_HISTORICAL_LIMIT as u32))
            .unwrap_or(MAX_HISTORICAL_LIMIT as u32);

        if limit == 0 {
            continue; // NIP-01: limit 0 means "no results from this filter"
        }

        // Push as many NIP-01 constraints into Typesense as possible so
        // post-filtering is a correction step, not the primary filter.
        //
        // If the filter has a #h tag, push the specific channel(s) into Typesense
        // instead of the full accessible set. This prevents cross-channel hits from
        // consuming pagination budget and causing under-fetch.
        // If the filter has #h, intersect with accessible channels. If all #h
        // values are invalid/inaccessible, skip the filter entirely (match nothing)
        // rather than broadening to all channels.
        let h_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::H);
        let channel_scope =
            if let Some(vs) = filter.generic_tags.get(&h_tag).filter(|vs| !vs.is_empty()) {
                let valid: Vec<String> = vs
                    .iter()
                    .filter_map(|v| v.parse::<uuid::Uuid>().ok())
                    .filter(|id| accessible_channels.contains(id))
                    .map(|id| id.to_string())
                    .collect();
                if valid.is_empty() {
                    continue; // all #h values invalid/inaccessible — skip filter
                }
                format!("channel_id:=[{}]", valid.join(","))
            } else {
                all_channels_filter.clone()
            };
        let mut filter_parts = vec![channel_scope];
        if let Some(ref kinds) = filter.kinds {
            if !kinds.is_empty() {
                let kind_vals: Vec<String> = kinds.iter().map(|k| k.as_u16().to_string()).collect();
                filter_parts.push(format!("kind:=[{}]", kind_vals.join(",")));
            }
        }
        if let Some(ref authors) = filter.authors {
            if !authors.is_empty() {
                let author_vals: Vec<String> = authors.iter().map(|a| a.to_hex()).collect();
                filter_parts.push(format!("pubkey:=[{}]", author_vals.join(",")));
            }
        }
        if let Some(since) = filter.since {
            filter_parts.push(format!("created_at:>={}", since.as_secs()));
        }
        if let Some(until) = filter.until {
            filter_parts.push(format!("created_at:<={}", until.as_secs()));
        }

        let filter_by = filter_parts.join(" && ");

        // Paginate: keep fetching pages until we've emitted `limit` results
        // or exhausted the search result set. This ensures post-filtering
        // doesn't silently reduce the result count below the requested limit.
        let mut emitted: u32 = 0;
        // Always fetch full pages (100) regardless of limit — post-filtering
        // may discard many hits, so we need headroom to fill the requested limit.
        let per_page: u32 = 100;

        for page in 1..=MAX_SEARCH_PAGES {
            if emitted >= limit {
                break;
            }

            let search_query = sprout_search::SearchQuery {
                q: search_text.clone(),
                filter_by: Some(filter_by.clone()),
                sort_by: None, // Typesense default = relevance (text_match score)
                page,
                per_page,
            };

            let search_result = match state.search.search(&search_query).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(sub_id = %sub_id, "NIP-50 search failed: {e}");
                    break;
                }
            };

            let page_empty = search_result.hits.is_empty();
            let exhausted = (page as u64) * (per_page as u64) >= search_result.found;

            let hit_ids: Vec<Vec<u8>> = search_result
                .hits
                .into_iter()
                .filter_map(|h| hex::decode(&h.event_id).ok())
                .filter(|bytes| bytes.len() == 32)
                .collect();

            if !hit_ids.is_empty() {
                let id_refs: Vec<&[u8]> = hit_ids.iter().map(|b| b.as_slice()).collect();
                let events = match state.db.get_events_by_ids(&id_refs).await {
                    Ok(evs) => evs,
                    Err(e) => {
                        warn!(sub_id = %sub_id, "NIP-50 batch fetch failed: {e}");
                        break;
                    }
                };

                let event_map: std::collections::HashMap<[u8; 32], &sprout_core::StoredEvent> =
                    events
                        .iter()
                        .map(|ev| (ev.event.id.to_bytes(), ev))
                        .collect();

                for hit_id in &hit_ids {
                    if emitted >= limit {
                        break;
                    }
                    let id_array: [u8; 32] = match hit_id.as_slice().try_into() {
                        Ok(a) => a,
                        Err(_) => continue,
                    };
                    let stored = match event_map.get(&id_array) {
                        Some(ev) => ev,
                        None => continue,
                    };
                    // NIP-01 post-filtering against THIS filter only (not OR of all filters).
                    if !filters_match(std::slice::from_ref(filter), stored) {
                        continue;
                    }
                    if let Some(ch_id) = stored.channel_id {
                        if !accessible_channels.contains(&ch_id) {
                            continue;
                        }
                    }
                    if !sprout_core::filter::reader_authorized_for_event(
                        &stored.event,
                        reader_pubkey_hex,
                    ) {
                        continue;
                    }
                    // Dedup AFTER acceptance — an event that fails filter A's constraints
                    // must remain eligible for filter B (NIP-01 OR semantics).
                    if !seen_ids.insert(stored.event.id) {
                        continue;
                    }
                    if !conn.send(RelayMessage::event(sub_id, &stored.event)) {
                        return;
                    }
                    emitted += 1;
                }
            }

            if page_empty || exhausted {
                break;
            }
        }
    }

    conn.send(RelayMessage::eose(sub_id));
}

/// Convert a single NIP-01 filter into an [`EventQuery`] for the database.
///
/// Public wrapper for use by the HTTP bridge and COUNT handler.
/// Resolves accessible channels for the given pubkey and builds the query.
pub async fn build_event_query_from_filter(
    filter: &Filter,
    _pubkey_bytes: &[u8],
    _state: &AppState,
) -> EventQuery {
    let channel_id = extract_channel_id_from_filter(filter);
    filter_to_query_params(filter, channel_id)
}

/// Returns `true` if all constraints in this filter can be fully represented
/// in SQL by `filter_to_query_params` — meaning `count_events()` will produce
/// an exact count without post-filtering.
///
/// Pushed constraints: kinds, authors (single or multi), ids, since, until,
/// channel_id (#h single), #p (single), #d (single, NIP-33-only kinds), #e (any),
/// channel_ids (injected by caller).
///
/// Anything else (multi-#p, #t, #a, search, multi-#h, #d on non-NIP-33)
/// requires post-filtering and cannot use the fast COUNT path.
pub fn filter_fully_pushable(filter: &Filter) -> bool {
    // Check if filter exclusively targets NIP-33 kinds (needed for #d pushability).
    let is_nip33_only = filter.kinds.as_ref().is_some_and(|ks| {
        !ks.is_empty()
            && ks
                .iter()
                .all(|k| sprout_core::kind::is_parameterized_replaceable(k.as_u16() as u32))
    });

    for (tag_key, tag_values) in filter.generic_tags.iter() {
        let key = tag_key.to_string();
        match key.as_str() {
            "h" => {
                // Single #h is pushed as channel_id; multi-#h is not.
                if tag_values.len() > 1 {
                    return false;
                }
            }
            "p" => {
                // Single #p is pushed via event_mentions join; multi is not.
                if tag_values.len() > 1 {
                    return false;
                }
            }
            "d" => {
                // #d is pushed (single or multi) ONLY for NIP-33-only kind filters.
                // Otherwise it's silently ignored by SQL → overcount.
                if !tag_values.is_empty() && !is_nip33_only {
                    return false;
                }
            }
            "e" => {
                // #e is fully pushed (any count) via JSONB containment.
            }
            _ => {
                // Any other generic tag (#t, #a, etc.) is not pushed.
                if !tag_values.is_empty() {
                    return false;
                }
            }
        }
    }
    // search field is not pushed by filter_to_query_params
    if filter.search.is_some() {
        return false;
    }
    true
}

/// Extract a channel UUID from a single filter's `#h` tag.
fn extract_channel_id_from_filter(filter: &Filter) -> Option<uuid::Uuid> {
    for (tag_key, tag_values) in filter.generic_tags.iter() {
        let key = tag_key.to_string();
        if key == "h" {
            for val in tag_values {
                if let Ok(id) = val.parse::<uuid::Uuid>() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// Convert a single NIP-01 filter into an [`EventQuery`] for the database.
///
/// Each filter is queried independently so that per-filter `limit` and time
/// windows are respected. Results are deduplicated by event ID in the caller.
fn filter_to_query_params(filter: &Filter, channel_id: Option<uuid::Uuid>) -> EventQuery {
    let kinds: Option<Vec<i32>> = filter.kinds.as_ref().map(|ks| {
        if ks.is_empty() {
            // kinds:[] means "match no kinds" — skip this filter entirely by
            // returning a sentinel that the DB query will produce zero rows for.
            // We use Some(vec![]) which the DB layer treats as "no matching kinds".
            vec![]
        } else {
            // Cast to i32 for Postgres INT column; safe because all Sprout kinds fit in i32.
            ks.iter().map(|k| k.as_u16() as i32).collect()
        }
    });

    let since = filter
        .since
        .and_then(|s| chrono::DateTime::from_timestamp(s.as_secs() as i64, 0));
    let until = filter
        .until
        .and_then(|u| chrono::DateTime::from_timestamp(u.as_secs() as i64, 0));
    let limit = filter
        .limit
        .map(|l| (l as i64).min(MAX_HISTORICAL_LIMIT))
        .unwrap_or(MAX_HISTORICAL_LIMIT);

    // Push author filter into SQL. Single-author uses the indexed `pubkey` column;
    // multi-author uses the `authors` IN-list pushdown added in the pure-nostr PR.
    let (pubkey, authors) = match filter.authors.as_ref() {
        Some(a) if a.len() == 1 => (a.iter().next().map(|pk| pk.to_bytes().to_vec()), None),
        Some(a) if !a.is_empty() => (
            None,
            Some(
                a.iter()
                    .map(|pk| pk.to_bytes().to_vec())
                    .collect::<Vec<_>>(),
            ),
        ),
        _ => (None, None),
    };

    // Push event IDs into SQL via the `ids` IN-list pushdown.
    let ids = filter.ids.as_ref().and_then(|id_set| {
        if id_set.is_empty() {
            None
        } else {
            Some(
                id_set
                    .iter()
                    .map(|id| id.to_bytes().to_vec())
                    .collect::<Vec<_>>(),
            )
        }
    });

    // Push #e tag filter into SQL via JSONB containment.
    let e_tag_key = nostr::SingleLetterTag::lowercase(nostr::Alphabet::E);
    let e_tags = filter.generic_tags.get(&e_tag_key).and_then(|values| {
        if values.is_empty() {
            None
        } else {
            Some(values.iter().map(|v| v.to_string()).collect::<Vec<_>>())
        }
    });

    // Push single-value #p tag into SQL via event_mentions join.
    // This is critical for gift-wrap (kind:1059) and membership notification
    // queries where >500 events for other recipients would otherwise push
    // the caller's events past the LIMIT before post-filtering.
    let p_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::P);
    let p_tag_hex = filter.generic_tags.get(&p_tag).and_then(|values| {
        if values.len() == 1 {
            values.iter().next().map(|v| v.to_string())
        } else {
            None
        }
    });

    // Push single-value #d tag into SQL via the d_tag column (NIP-33).
    // Critical for parameterized replaceable lookups (authors + kinds + #d)
    // where many events from the same author would push the target past LIMIT.
    //
    // Only push when the filter exclusively targets NIP-33 kinds (30000–39999),
    // because `d_tag` is only populated for those kinds. Non-NIP-33 events have
    // `d_tag = NULL`, so pushing `AND d_tag = $N` for a mixed-kind or kindless
    // filter would silently exclude non-NIP-33 rows that match via their tags.
    let filter_is_nip33_only = kinds.as_ref().is_some_and(|ks| {
        !ks.is_empty()
            && ks
                .iter()
                .all(|&k| sprout_core::kind::is_parameterized_replaceable(k as u32))
    });
    let d_tag_key = nostr::SingleLetterTag::lowercase(nostr::Alphabet::D);
    let (d_tag, d_tags) = if filter_is_nip33_only {
        let values = filter.generic_tags.get(&d_tag_key);
        match values.map(|v| v.len()) {
            Some(1) => (
                values.and_then(|vs| vs.iter().next().map(|v| v.to_string())),
                None,
            ),
            Some(n) if n > 1 => (
                None,
                values.map(|vs| vs.iter().map(|v| v.to_string()).collect::<Vec<_>>()),
            ),
            _ => (None, None),
        }
    } else {
        (None, None)
    };

    EventQuery {
        channel_id,
        kinds,
        pubkey,
        since,
        until,
        limit: Some(limit),
        p_tag_hex,
        d_tag,
        d_tags,
        authors,
        ids,
        e_tags,
        ..Default::default()
    }
}

/// Extract a single channel UUID from filter generic tags, or `None` if the
/// subscription is logically global.
///
/// Checks the `"h"` tag key — channel-scoped subscriptions use `#h = <uuid>`.
///
/// Returns `None` when:
/// - Any filter has no channel tag (that filter matches all channels → global sub), or
/// - Multiple distinct channel UUIDs appear across filters (can't index under one channel).
///
/// Callers that receive `None` treat the subscription as global (slow-path fan-out).
fn extract_channel_id_from_filters(filters: &[Filter]) -> Option<uuid::Uuid> {
    let mut found_id: Option<uuid::Uuid> = None;
    for f in filters {
        let mut filter_has_channel = false;
        for (tag_key, tag_values) in f.generic_tags.iter() {
            let key = tag_key.to_string();
            if key == "h" {
                for val in tag_values {
                    if let Ok(id) = val.parse::<uuid::Uuid>() {
                        filter_has_channel = true;
                        match found_id {
                            Some(existing) if existing != id => {
                                // Multiple distinct channel IDs — fall back to global.
                                return None;
                            }
                            _ => found_id = Some(id),
                        }
                    }
                }
            }
        }
        if !filter_has_channel {
            // This filter has no channel constraint — the subscription is global.
            return None;
        }
    }
    found_id
}

pub(crate) fn p_gated_filters_authorized(filters: &[Filter], authed_pubkey_hex: &str) -> bool {
    let p_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::P);
    filters.iter().all(|filter| {
        let can_match_p_gated = filter.kinds.as_ref().is_none_or(|ks| {
            ks.iter()
                .any(|kind| P_GATED_KINDS.contains(&(kind.as_u16() as u32)))
        });
        if !can_match_p_gated {
            return true;
        }

        // The `ids` exemption ("knowing the id implies authorization") is only
        // safe for kinds whose id is author-bound or whose content is encrypted.
        // KIND_DM_VISIBILITY is relay-signed (id not author-bound) and exposes
        // plaintext private hide choices, so its `#p` owner check MUST hold even
        // when `ids` is present. Only filters that explicitly name the kind lose
        // the exemption — a kindless `ids` lookup is unaffected.
        let explicitly_dm_visibility = filter.kinds.as_ref().is_some_and(|ks| {
            ks.iter()
                .any(|kind| kind.as_u16() as u32 == KIND_DM_VISIBILITY)
        });
        if !explicitly_dm_visibility && filter.ids.as_ref().is_some_and(|ids| !ids.is_empty()) {
            return true;
        }

        filter.generic_tags.get(&p_tag).is_some_and(|values| {
            !values.is_empty() && values.iter().all(|value| value == authed_pubkey_hex)
        })
    })
}

/// Authorize read access for filters that can match KIND_AGENT_ENGRAM events.
///
/// NIP-AE engrams are global (no channel scope) and have encrypted content,
/// but their public `#p` (owner) and timestamps still leak who-pairs-with-whom
/// plus write-activity patterns. Only the agent (the event's author) or the
/// owner (the `#p` value) should be able to enumerate them.
///
/// A filter is authorized when at least one of:
///   - `authors` is non-empty and every entry equals the authed pubkey
///     (the agent reading its own engrams), OR
///   - `#p` is non-empty and every entry equals the authed pubkey
///     (the owner reading engrams addressed to them).
///
/// Filters with explicit `ids` are exempt — knowing the event id already
/// implies authorization (the engram event id is itself derived from the
/// signed envelope, which only the agent could have produced).
///
/// Mixed-kind filters (e.g. `{kinds:[30174, 9]}`) are evaluated under this
/// gate when KIND_AGENT_ENGRAM is present; matching events of other kinds in
/// the same filter is also restricted, but that is the conservative choice
/// — clients should query engrams in a dedicated filter.
pub(crate) fn engram_filters_authorized(filters: &[Filter], authed_pubkey_hex: &str) -> bool {
    let p_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::P);
    filters.iter().all(|filter| {
        // Specific-event lookups don't fish.
        if filter.ids.as_ref().is_some_and(|ids| !ids.is_empty()) {
            return true;
        }

        let can_match_engram = filter
            .kinds
            .as_ref()
            .is_none_or(|ks| ks.iter().any(|k| k.as_u16() as u32 == KIND_AGENT_ENGRAM));
        if !can_match_engram {
            return true;
        }

        let authors_ok = filter.authors.as_ref().is_some_and(|authors| {
            !authors.is_empty()
                && authors
                    .iter()
                    .all(|a| a.to_hex().eq_ignore_ascii_case(authed_pubkey_hex))
        });
        if authors_ok {
            return true;
        }

        filter.generic_tags.get(&p_tag).is_some_and(|values| {
            !values.is_empty() && values.iter().all(|v| v == authed_pubkey_hex)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Alphabet, Filter, SingleLetterTag};

    fn filter_with_channel(channel_id: uuid::Uuid) -> Filter {
        Filter::new().custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            channel_id.to_string(),
        )
    }

    #[test]
    fn test_extract_channel_id_single_channel() {
        let channel_id = uuid::Uuid::new_v4();
        let filters = vec![filter_with_channel(channel_id)];
        assert_eq!(extract_channel_id_from_filters(&filters), Some(channel_id));
    }

    #[test]
    fn test_extract_channel_id_mixed_channels_returns_none() {
        let channel_a = uuid::Uuid::new_v4();
        let channel_b = uuid::Uuid::new_v4();
        let filters = vec![
            filter_with_channel(channel_a),
            filter_with_channel(channel_b),
        ];
        assert_eq!(extract_channel_id_from_filters(&filters), None);
    }

    #[test]
    fn test_extract_channel_id_no_channel_tag_returns_none() {
        let filters = vec![Filter::new()];
        assert_eq!(extract_channel_id_from_filters(&filters), None);
    }

    #[test]
    fn test_extract_channel_id_one_filter_missing_channel_returns_none() {
        // Even if one filter has a channel, a second filter without one makes it global.
        let channel_id = uuid::Uuid::new_v4();
        let filters = vec![filter_with_channel(channel_id), Filter::new()];
        assert_eq!(extract_channel_id_from_filters(&filters), None);
    }

    #[test]
    fn test_extract_channel_id_same_channel_multiple_filters() {
        let channel_id = uuid::Uuid::new_v4();
        let filters = vec![
            filter_with_channel(channel_id),
            filter_with_channel(channel_id),
        ];
        assert_eq!(extract_channel_id_from_filters(&filters), Some(channel_id));
    }

    #[test]
    fn test_search_filter_detection() {
        let search_filter = Filter::new().search("hello world");
        let filters = [search_filter];
        assert!(filters.iter().any(|f| f.search.is_some()));
    }

    #[test]
    fn dm_visibility_requires_p_tag_even_with_ids() {
        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let authed = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let snapshot_id = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let dm_vis = nostr::Kind::Custom(sprout_core::kind::KIND_DM_VISIBILITY as u16);

        // Knowing another viewer's snapshot id must NOT authorize reading it:
        // ids alone, or ids + someone else's #p, are both rejected.
        let ids_only = Filter::new()
            .kind(dm_vis)
            .id(nostr::EventId::from_hex(snapshot_id).unwrap());
        assert!(!p_gated_filters_authorized(&[ids_only], authed));

        let ids_wrong_p = Filter::new()
            .kind(dm_vis)
            .id(nostr::EventId::from_hex(snapshot_id).unwrap())
            .custom_tags(p_tag, [other]);
        assert!(!p_gated_filters_authorized(&[ids_wrong_p], authed));

        // The owner querying their own snapshot (by #p) is allowed, ids or not.
        let owner = Filter::new().kind(dm_vis).custom_tags(p_tag, [authed]);
        assert!(p_gated_filters_authorized(&[owner], authed));

        // The ids exemption still applies to other p-gated kinds (member notifs).
        let member_notif_ids = Filter::new()
            .kind(nostr::Kind::Custom(
                sprout_core::kind::KIND_MEMBER_ADDED_NOTIFICATION as u16,
            ))
            .id(nostr::EventId::from_hex(snapshot_id).unwrap());
        assert!(p_gated_filters_authorized(&[member_notif_ids], authed));
    }

    #[test]
    fn test_mixed_search_and_non_search_detection() {
        let search_filter = Filter::new().search("hello");
        let plain_filter = Filter::new();
        let filters = [search_filter, plain_filter];
        let has_search = filters.iter().any(|f| f.search.is_some());
        let has_non_search = filters.iter().any(|f| f.search.is_none());
        assert!(has_search && has_non_search, "should detect mixed filters");
    }

    #[test]
    fn test_all_search_filters_not_mixed() {
        let f1 = Filter::new().search("hello");
        let f2 = Filter::new().search("world");
        let filters = [f1, f2];
        let has_search = filters.iter().any(|f| f.search.is_some());
        let has_non_search = filters.iter().any(|f| f.search.is_none());
        assert!(has_search);
        assert!(!has_non_search, "all-search filters should not be mixed");
    }

    #[test]
    fn agent_observer_subscription_requires_matching_p_tag() {
        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let authed = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let missing_p = Filter::new().kind(nostr::Kind::Custom(
            sprout_core::kind::KIND_AGENT_OBSERVER_FRAME as u16,
        ));
        assert!(!p_gated_filters_authorized(&[missing_p], authed));

        let wrong_p = Filter::new()
            .kind(nostr::Kind::Custom(
                sprout_core::kind::KIND_AGENT_OBSERVER_FRAME as u16,
            ))
            .custom_tags(p_tag, [other]);
        assert!(!p_gated_filters_authorized(&[wrong_p], authed));

        let matching_p = Filter::new()
            .kind(nostr::Kind::Custom(
                sprout_core::kind::KIND_AGENT_OBSERVER_FRAME as u16,
            ))
            .custom_tags(p_tag, [authed]);
        assert!(p_gated_filters_authorized(&[matching_p], authed));
    }

    #[test]
    fn d_tag_pushdown_only_for_nip33_kinds() {
        let d_tag = SingleLetterTag::lowercase(Alphabet::D);

        // NIP-33 kind with #d → pushdown active
        let nip33_filter = Filter::new()
            .kind(nostr::Kind::Custom(30023))
            .custom_tags(d_tag, ["my-slug"]);
        let q = filter_to_query_params(&nip33_filter, None);
        assert_eq!(q.d_tag, Some("my-slug".to_string()));

        // Non-NIP-33 kind with #d → pushdown NOT active (would miss rows with d_tag=NULL)
        let non_nip33_filter = Filter::new()
            .kind(nostr::Kind::Custom(1))
            .custom_tags(d_tag, ["some-value"]);
        let q2 = filter_to_query_params(&non_nip33_filter, None);
        assert_eq!(q2.d_tag, None);

        // Mixed kinds (one NIP-33, one not) → pushdown NOT active
        let mixed_filter = Filter::new()
            .kinds([nostr::Kind::Custom(30023), nostr::Kind::Custom(1)])
            .custom_tags(d_tag, ["slug"]);
        let q3 = filter_to_query_params(&mixed_filter, None);
        assert_eq!(q3.d_tag, None);

        // No kinds specified → pushdown NOT active
        let no_kinds_filter = Filter::new().custom_tags(d_tag, ["slug"]);
        let q4 = filter_to_query_params(&no_kinds_filter, None);
        assert_eq!(q4.d_tag, None);

        // Multi-value #d → pushdown NOT active (can't push OR into single column match)
        let multi_d_filter = Filter::new()
            .kind(nostr::Kind::Custom(30023))
            .custom_tags(d_tag, ["slug-a", "slug-b"]);
        let q5 = filter_to_query_params(&multi_d_filter, None);
        assert_eq!(q5.d_tag, None);
    }

    #[test]
    fn restricted_search_scope_excludes_global_results() {
        let channel_id = uuid::Uuid::new_v4();

        let scope = build_search_channel_scope_filter(&[channel_id], false)
            .expect("restricted tokens with channel access should still search that channel");

        assert_eq!(scope, format!("channel_id:=[{channel_id}]"));
    }

    #[test]
    fn restricted_search_scope_without_accessible_channels_matches_nothing() {
        assert!(
            build_search_channel_scope_filter(&[], false).is_none(),
            "restricted tokens must not fall back to global search results"
        );
    }

    // ── NIP-AE engram read gating ────────────────────────────────────────

    /// Three real x-only pubkeys (valid for `PublicKey::from_hex`). Distinct,
    /// so we can label them clearly in tests.
    fn three_pubkeys() -> (String, String, String) {
        let agent = nostr::Keys::generate().public_key().to_hex();
        let owner = nostr::Keys::generate().public_key().to_hex();
        let attacker = nostr::Keys::generate().public_key().to_hex();
        (agent, owner, attacker)
    }

    #[test]
    fn engram_gate_allows_agent_querying_own() {
        let (agent, owner, _) = three_pubkeys();
        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let f = Filter::new()
            .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
            .author(nostr::PublicKey::from_hex(&agent).unwrap())
            .custom_tags(p_tag, [&owner]);
        assert!(engram_filters_authorized(&[f], &agent));
    }

    #[test]
    fn engram_gate_allows_owner_querying() {
        let (agent, owner, _) = three_pubkeys();
        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        // Owner-side read: knows the agent's pubkey, queries with #p=self.
        let f = Filter::new()
            .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
            .author(nostr::PublicKey::from_hex(&agent).unwrap())
            .custom_tags(p_tag, [&owner]);
        assert!(engram_filters_authorized(&[f], &owner));
    }

    #[test]
    fn engram_gate_allows_owner_with_no_authors_filter() {
        // Owner doesn't necessarily know the agent's pubkey ahead of time.
        let (_, owner, _) = three_pubkeys();
        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let f = Filter::new()
            .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
            .custom_tags(p_tag, [&owner]);
        assert!(engram_filters_authorized(&[f], &owner));
    }

    #[test]
    fn engram_gate_rejects_unrelated_reader() {
        let (agent, owner, attacker) = three_pubkeys();
        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        // Attacker tries to fish for engrams between agent and owner.
        let f = Filter::new()
            .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
            .author(nostr::PublicKey::from_hex(&agent).unwrap())
            .custom_tags(p_tag, [&owner]);
        assert!(!engram_filters_authorized(&[f], &attacker));
    }

    #[test]
    fn engram_gate_rejects_bare_kind_filter() {
        // {kinds:[30174]} with no authors and no #p — open fishing.
        let (agent, _, _) = three_pubkeys();
        let f = Filter::new().kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16));
        assert!(!engram_filters_authorized(&[f], &agent));
    }

    #[test]
    fn engram_gate_rejects_wildcard_kind_filter() {
        // Filter with no kinds field at all — matches everything including
        // engrams; must still be gated.
        let (agent, _, _) = three_pubkeys();
        let f = Filter::new();
        assert!(!engram_filters_authorized(&[f], &agent));
    }

    #[test]
    fn engram_gate_skips_non_engram_kinds() {
        // Filter not targeting engrams — pass through; this gate is silent.
        let (agent, _, _) = three_pubkeys();
        let f = Filter::new().kind(nostr::Kind::Custom(9));
        assert!(engram_filters_authorized(&[f], &agent));
    }

    #[test]
    fn engram_gate_allows_ids_lookup() {
        // Specific event ids — knowing the id implies prior authorization.
        let (agent, _, _) = three_pubkeys();
        let id = nostr::EventId::from_hex(
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        )
        .unwrap();
        let f = Filter::new()
            .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
            .id(id);
        assert!(engram_filters_authorized(&[f], &agent));
    }

    #[test]
    fn engram_gate_rejects_mixed_authors_with_unauthed() {
        // {authors:[self, attacker]} — must reject; an author-list with any
        // non-self entry could let an attacker piggy-back on the agent's
        // legitimate query path.
        let (agent, other, _) = three_pubkeys();
        let f = Filter::new()
            .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
            .authors([
                nostr::PublicKey::from_hex(&agent).unwrap(),
                nostr::PublicKey::from_hex(&other).unwrap(),
            ]);
        assert!(!engram_filters_authorized(&[f], &agent));
    }

    // ── NIP-50 search bypass regressions ─────────────────────────────────
    // These filters are the shape an authenticated relay member would send
    // to try to harvest indexed engram envelopes via the search path. The
    // gate must reject them regardless of the presence of `search`.

    #[test]
    fn engram_gate_rejects_bare_kind_search_filter() {
        // {"search":"*", "kinds":[30174]} — exactly the bypass codex found.
        let (agent, _, _) = three_pubkeys();
        let f = Filter::new()
            .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
            .search("*");
        assert!(!engram_filters_authorized(&[f], &agent));
    }

    #[test]
    fn engram_gate_rejects_wildcard_kind_search_filter() {
        // {"search":"foo"} — no `kinds` field at all matches engrams too.
        let (agent, _, _) = three_pubkeys();
        let f = Filter::new().search("foo");
        assert!(!engram_filters_authorized(&[f], &agent));
    }

    #[test]
    fn engram_gate_allows_authored_engram_search() {
        // Agent searching their own engrams by content keyword is legitimate.
        let (agent, _, _) = three_pubkeys();
        let f = Filter::new()
            .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
            .author(nostr::PublicKey::from_hex(&agent).unwrap())
            .search("foo");
        assert!(engram_filters_authorized(&[f], &agent));
    }

    #[test]
    fn p_gate_rejects_bare_kind_search_filter_for_gift_wrap() {
        // P-gated kinds (observer frames, member notifications) are indexed
        // too. Same bypass shape: {"search":"x","kinds":[<p-gated kind>]}.
        // Use KIND_AGENT_OBSERVER_FRAME — globally stored, p-gated, indexed.
        let (agent, _, _) = three_pubkeys();
        let f = Filter::new()
            .kind(nostr::Kind::Custom(
                sprout_core::kind::KIND_AGENT_OBSERVER_FRAME as u16,
            ))
            .search("x");
        assert!(!p_gated_filters_authorized(&[f], &agent));
    }
}
