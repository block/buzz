//! NIP-45 COUNT handler — aggregate queries with channel access enforcement.

use std::sync::Arc;

use nostr::Filter;
use tracing::warn;

use crate::connection::{AuthState, ConnectionState};
use crate::handlers::req::{
    event_visible_to_reader, filter_can_match_result_gated_kinds,
    filter_can_match_shared_gated_kinds, result_gated_count_safe_for_pushdown,
};
use crate::protocol::RelayMessage;
use crate::state::AppState;

/// Handle a COUNT message: require auth, enforce channel access, execute filters,
/// and return the aggregate count.
pub async fn handle_count(
    sub_id: String,
    filters: Vec<Filter>,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
) {
    // Require auth
    let (pubkey_bytes, token_channel_ids) = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Authenticated(ctx) => {
                (ctx.pubkey.to_bytes().to_vec(), ctx.channel_ids.clone())
            }
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

    let requested_channel_sets =
        match super::req::extract_channel_ids_from_filters_limited(&filters) {
            Ok(_) => filters
                .iter()
                .map(|filter| {
                    super::req::extract_channel_ids_from_filters(std::slice::from_ref(filter))
                })
                .collect::<Vec<_>>(),
            Err(()) => {
                conn.send(RelayMessage::closed(
                    &sub_id,
                    "restricted: too many explicit channels",
                ));
                return;
            }
        };

    // Get channels this user can access — same enforcement as WS REQ handler.
    let mut accessible_channels = match state
        .get_accessible_channel_ids_cached(conn.tenant.community(), &pubkey_bytes)
        .await
    {
        Ok(ids) => ids,
        Err(e) => {
            warn!(sub_id = %sub_id, "Failed to get accessible channels: {e}");
            conn.send(RelayMessage::closed(&sub_id, "error: database error"));
            return;
        }
    };
    // Narrow to the token's channel scope, mirroring the WS REQ handler. Without
    // this, a scoped token would COUNT events in channels outside its scope via
    // the no-channel-filter SQL pushdown below (which counts every accessible
    // channel). The per-filter targeted-channel repair is bounded by the same
    // scope through `resolve_request_local_access`'s `token_allows` argument.
    if let Some(allowed) = token_channel_ids.as_deref() {
        accessible_channels.retain(|channel_id| allowed.contains(channel_id));
    }

    // B2: acquire effect permit immediately before the first DB count query.
    // The permit is held through all count queries and the COUNT response.
    // Off-mode: proceed unconditionally.
    // [FI-TRACE-LEASE-BOUND, B2 seam: COUNT query]
    //
    // Test hook: fires immediately before acquire_effect.
    // [nip_fi_test_hooks::count_query_hook]
    #[cfg(test)]
    crate::nip_fi_test_hooks::before_count_query(conn.tenant.community()).await;
    let _count_permit = match conn.nip_fi_gate.acquire_effect().await {
        Ok(permit) => permit,
        Err(crate::nip_fi_gate::SessionExpired) => {
            conn.send(RelayMessage::closed(&sub_id, "restricted: session expired"));
            return;
        }
    };

    // For each filter, count matching events with channel access enforcement.
    let mut total: u64 = 0;
    for (filter, requested_channels) in filters.iter().zip(requested_channel_sets) {
        // Determine if this filter can match author-only kinds — if so, the
        // fast-path count_events() cannot be used because it doesn't do
        // per-event author filtering.
        let needs_author_only_filtering = super::req::filter_can_match_author_only_kinds(filter);
        // Determine if this filter can match a shared-gated kind (30175, 30178)
        // — if so, the fast path must be bypassed because it has no per-event
        // shared-tag check. A fast count over those kinds would include foreign
        // unshared events, leaking the existence of private agent activity.
        let needs_shared_gate_filtering = filter_can_match_shared_gated_kinds(filter);
        // Determine if this filter can match result-gated kinds (44200, 30622)
        // that require a per-event owner check. When the fast SQL path would
        // count matching rows without calling reader_authorized_for_event, a
        // non-owner learns the existence of events they are not allowed to see.
        // The only safe pushdown is when #p is pinned to the authenticated
        // reader's own pubkey.
        let needs_result_gated_filtering = filter_can_match_result_gated_kinds(filter)
            && !result_gated_count_safe_for_pushdown(filter, &authed_pubkey_hex);

        if let Some(requested_channels) = requested_channels {
            for &ch_id in &requested_channels {
                if accessible_channels.contains(&ch_id) {
                    continue;
                }
                let token_allows = token_channel_ids
                    .as_deref()
                    .is_none_or(|allowed| allowed.contains(&ch_id));
                let db_is_member = if token_allows {
                    match state
                        .db
                        .is_member(conn.tenant.community(), ch_id, &pubkey_bytes)
                        .await
                    {
                        Ok(member) => Some(member),
                        Err(e) => {
                            warn!(sub_id = %sub_id, "Channel membership confirmation failed: {e}");
                            conn.send(RelayMessage::closed(&sub_id, "error: database error"));
                            return;
                        }
                    }
                } else {
                    None
                };
                super::req::resolve_request_local_access(
                    &mut accessible_channels,
                    ch_id,
                    token_allows,
                    db_is_member,
                );
            }
            let authorized_requested: Vec<_> = requested_channels
                .iter()
                .copied()
                .filter(|channel_id| accessible_channels.contains(channel_id))
                .collect();
            if authorized_requested.is_empty() {
                continue;
            }
            // Preserve the original explicit multi-channel shape even when
            // authorization narrows it to one channel. The helper must write
            // that intersection into `channel_ids`; synthesizing `Some(A)` here
            // would leave a query built from multi-#h completely unscoped.
            let ch_id = (requested_channels.len() == 1).then_some(authorized_requested[0]);
            // Channel is accessible — count with pushability check.
            let mut query = super::req::build_event_query_from_filter(
                filter,
                &pubkey_bytes,
                &state,
                conn.tenant.community(),
            )
            .await;
            super::req::apply_channel_scope_to_query(
                &mut query,
                filter,
                ch_id,
                &accessible_channels,
            );
            // Shared-gated visibility pushdown: pre-filter the fallback
            // query_events candidate page before ORDER/LIMIT.
            if needs_shared_gate_filtering {
                query.shared_gated_reader = Some(pubkey_bytes.clone());
            }
            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if super::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
                && !needs_result_gated_filtering
                && !needs_shared_gate_filtering
            {
                match state.db.count_events_routed("count_req", &query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        conn.send(RelayMessage::closed(&sub_id, &format!("error: {e}")));
                        return;
                    }
                }
            } else {
                // Fallback: query + post-filter for non-pushable constraints.
                let mut q = query;
                super::req::apply_count_fallback_limit(&mut q);
                match state
                    .db
                    .query_events_routed_bounded("count_req_fallback", &q)
                    .await
                {
                    Ok(stored_events) => {
                        if super::req::count_fallback_exceeded(stored_events.len()) {
                            metrics::counter!("buzz_count_fallback_rejections_total").increment(1);
                            conn.send(RelayMessage::closed(
                                &sub_id,
                                "restricted: count filter requires narrower constraints",
                            ));
                            return;
                        }
                        for se in stored_events {
                            if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se)
                            {
                                continue;
                            }
                            if !event_visible_to_reader(&se.event, &pubkey_bytes) {
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
            let mut query = super::req::build_event_query_from_filter(
                filter,
                &pubkey_bytes,
                &state,
                conn.tenant.community(),
            )
            .await;
            query.channel_ids = Some(accessible_channels.to_vec());
            // Shared-gated visibility pushdown for the fallback query_events path.
            if needs_shared_gate_filtering {
                query.shared_gated_reader = Some(pubkey_bytes.clone());
            }

            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if super::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
                && !needs_result_gated_filtering
                && !needs_shared_gate_filtering
            {
                query.limit = None; // COUNT doesn't need a row limit
                match state.db.count_events_routed("count_req", &query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        conn.send(RelayMessage::closed(&sub_id, &format!("error: {e}")));
                        return;
                    }
                }
            } else {
                // Fallback: query a bounded candidate set + post-filter.
                super::req::apply_count_fallback_limit(&mut query);
                match state
                    .db
                    .query_events_routed_bounded("count_req_fallback", &query)
                    .await
                {
                    Ok(stored_events) => {
                        if super::req::count_fallback_exceeded(stored_events.len()) {
                            metrics::counter!("buzz_count_fallback_rejections_total").increment(1);
                            conn.send(RelayMessage::closed(
                                &sub_id,
                                "restricted: count filter requires narrower constraints",
                            ));
                            return;
                        }
                        for se in stored_events {
                            if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se)
                            {
                                continue;
                            }
                            if !event_visible_to_reader(&se.event, &pubkey_bytes) {
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── W4: B2 COUNT gate — barrier expiry mid-flight blocks count query ────────
    //
    // Arms `before_count_query` — the hook immediately before `acquire_effect()`
    // in the COUNT query path. Dispatches `handle_count` with a live (not-yet-
    // cancelled) gate, waits for the hook to signal the handler reached the permit
    // boundary, fires expiry (cancel), then releases the hook. The handler tries
    // `acquire_effect()` and gets `SessionExpired`, sends CLOSED without issuing
    // any DB query or modifying any state.
    //
    // Hook location: `handlers/count.rs`, immediately before `acquire_effect()`.
    //
    // Mutation evidence:
    //   A) Delete `#[cfg(test)] before_count_query(...)` from count.rs →
    //      hook never fires → `arrived_rx` times out → test panics.
    //   B) Remove `acquire_effect()` from count.rs → handler falls through to the
    //      DB path. With a lazy pool the query errors out, but the gate boundary is
    //      gone — the CLOSED message changes from "session expired" → assertion panics.
    //   C) Change gate to `off_mode` → `acquire_effect()` succeeds after cancel
    //      → handler proceeds, no CLOSED sent at all → `try_recv()` returns `Err`
    //      → assertion panics.

    #[tokio::test]
    async fn w4_b2_count_barrier_expiry_mid_flight_blocks_count_query() {
        use nostr::Keys;
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::{mpsc, RwLock};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        let keys = Keys::generate();
        let deadline = chrono::Utc::now() + chrono::Duration::hours(1);

        // Live gate — NOT pre-cancelled. acquire_effect succeeds unless we fire expiry.
        let cancel = CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());

        let (send_tx, mut send_rx) = mpsc::channel::<axum::extract::ws::Message>(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel::<axum::extract::ws::Message>(8);
        let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel::<axum::extract::ws::Message>(1);

        let conn = Arc::new(crate::connection::ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(community, "test.local".to_string()),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(crate::connection::AuthState::Authenticated(
                buzz_auth::AuthContext {
                    pubkey: keys.public_key(),
                    scopes: vec![],
                    channel_ids: None,
                    auth_method: buzz_auth::AuthMethod::Nip42,
                    agent_owner_pubkey: None,
                },
            )),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: None,
            session_deadline: Some(deadline),
            nip_fi_gate: gate,
            nip_fi_reason_tx: tokio::sync::watch::channel(None).0,
        });

        let state = crate::state::tests::test_state().await;
        let sub_id = "w4-barrier-test".to_string();
        // Kind:1 (TextNote) — not p-gated — so the filter clears all pre-gate
        // authorization checks and reaches the `before_count_query` hook.
        let filters = vec![nostr::Filter::new().kind(nostr::Kind::TextNote).limit(1)];

        // Arm the barrier: fires when handle_count reaches before_count_query.
        let (arrived_rx, release) = crate::nip_fi_test_hooks::count_query_hook::arm(community);

        let conn2 = Arc::clone(&conn);
        let state2 = Arc::clone(&state);
        let handle =
            tokio::spawn(async move { handle_count(sub_id, filters, conn2, state2).await });

        // Wait for the handler to reach the permit boundary.
        tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
            .await
            .expect("W4: handler must reach before_count_query within 5s")
            .expect("arrived channel closed");

        // Fire expiry: cancel so acquire_effect returns SessionExpired.
        cancel.cancel();

        // Release — handler resumes, calls acquire_effect(), gets SessionExpired.
        release.notify_one();

        tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("W4: handle_count must return within 5s after hook release")
            .expect("handle_count task must not panic");

        // A CLOSED frame must have been sent with the session-expired message —
        // no DB query was issued.
        let frame = send_rx
            .try_recv()
            .expect("W4: handler must send CLOSED on expired gate");
        match frame {
            axum::extract::ws::Message::Text(t) => {
                assert!(
                    t.contains("session expired"),
                    "W4: CLOSED message must contain 'session expired'; got: {t}"
                );
            }
            other => panic!("W4: expected Text CLOSED frame, got {other:?}"),
        }
    }
}
