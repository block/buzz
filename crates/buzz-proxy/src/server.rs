//! External-facing NIP-01 WebSocket server for standard Nostr clients.
//!
//! Handles NIP-11 relay info, NIP-42 AUTH challenge/response (with
//! reactive-auth–compatible CLOSED/OK rejections for pre-auth messages),
//! guest and invite token authentication, and kind:40/41 interception
//! from the local [`ChannelMap`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        FromRequest, Query, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use nostr::prelude::*;
use serde::Deserialize;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::channel_map::ChannelMap;
use crate::guest_store::GuestStore;
use crate::invite_store::InviteStore;
use crate::translate::Translator;
use crate::upstream::UpstreamClient;

/// Shared state injected into every axum handler.
#[derive(Clone)]
pub struct ProxyState {
    /// Bidirectional UUID ↔ kind:40 event ID map (loaded at startup).
    pub channel_map: Arc<ChannelMap>,
    /// Pubkey-based guest registry (persistent access, no token needed).
    pub guest_store: Arc<GuestStore>,
    /// In-memory invite token registry (temporary access via bearer token).
    pub invite_store: Arc<InviteStore>,
    /// Event translator: NIP-28 ↔ Buzz internal format.
    pub translator: Arc<Translator>,
    /// Upstream relay client — used to send events, REQs, and CLOSEs.
    pub upstream: Arc<UpstreamClient>,
    /// Broadcast channel: raw NIP-01 JSON strings FROM the upstream relay.
    /// Each WebSocket connection subscribes its own receiver.
    pub upstream_events: tokio::sync::broadcast::Sender<String>,
    /// Optional shared secret for the admin endpoint.
    /// If `Some`, requests must include `Authorization: Bearer <secret>`.
    /// If `None`, the endpoint is unauthenticated (dev mode).
    pub admin_secret: Option<String>,
    /// This proxy's own WebSocket URL (e.g. "ws://0.0.0.0:4869").
    /// Used for NIP-42 relay tag validation.
    pub relay_url: String,
}

/// Query parameters accepted on the root WebSocket endpoint.
#[derive(Deserialize)]
pub struct WsParams {
    /// Invite token string (required for WebSocket connections).
    token: Option<String>,
}

/// Build the axum [`Router`] for the proxy server.
///
/// Routes:
/// - `GET /`               — NIP-11 JSON *or* WebSocket upgrade (content-negotiated)
/// - `POST /admin/invite`  — Create an invite token (temporary access)
/// - `POST /admin/guests`  — Register a guest pubkey (persistent access)
/// - `DELETE /admin/guests` — Revoke a guest pubkey
/// - `GET /admin/guests`   — List all registered guests
///
/// All `/admin/*` routes are protected by `BUZZ_PROXY_ADMIN_SECRET` if set.
pub fn router(state: ProxyState) -> Router {
    Router::new()
        .route("/", get(root_handler))
        .route("/admin/invite", axum::routing::post(create_invite))
        .route(
            "/admin/guests",
            axum::routing::post(register_guest)
                .delete(revoke_guest)
                .get(list_guests),
        )
        .with_state(state)
}

/// Content-negotiate between NIP-11 JSON and WebSocket upgrade.
///
/// Uses `axum::extract::Request` to manually attempt the WS upgrade so that
/// plain HTTP GET requests (NIP-11 clients, browser visits) are not rejected
/// by the extractor. Mirrors the pattern used in `buzz-relay/src/router.rs`.
async fn root_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Query(params): Query<WsParams>,
    req: axum::extract::Request,
) -> Response {
    // NIP-11: clients that send `Accept: application/nostr+json` want relay info.
    let wants_nip11 = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("application/nostr+json"))
        .unwrap_or(false);

    if wants_nip11 {
        return nip11_response().into_response();
    }

    // Try WebSocket upgrade; fall back to NIP-11 JSON for plain HTTP.
    match WebSocketUpgrade::from_request(req, &state).await {
        Ok(ws) => {
            let token = params.token.unwrap_or_default();
            ws.on_upgrade(move |socket| handle_ws(socket, state, token))
        }
        Err(_) => nip11_response().into_response(),
    }
}

fn nip11_response() -> impl IntoResponse {
    let nip11 = serde_json::json!({
        "name": "buzz-proxy",
        "description": "Buzz NIP-28 guest proxy for standard Nostr clients",
        "supported_nips": [1, 11, 28, 42],
        "software": "buzz-proxy",
        "version": env!("CARGO_PKG_VERSION"),
        "limitation": {
            "auth_required": true
        }
    });
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/nostr+json"),
            (axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        serde_json::to_string_pretty(&nip11).unwrap(),
    )
}

/// Compare two strings in constant time to prevent timing side-channel attacks.
/// Returns `true` only if both strings are identical.
///
/// Uses hash-then-compare to eliminate the length oracle: both inputs are hashed
/// to fixed 32-byte values before comparison, so string length is never leaked.
fn constant_time_eq(a: &str, b: &str) -> bool {
    use sha2::{Digest, Sha256};
    let hash_a: [u8; 32] = Sha256::digest(a.as_bytes()).into();
    let hash_b: [u8; 32] = Sha256::digest(b.as_bytes()).into();
    // Fixed-length comparison — no length oracle
    hash_a
        .iter()
        .zip(hash_b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Helper: serialize a [`RelayMessage`] and send it over the socket.
/// Returns `true` if the send succeeded.
async fn send_relay_msg(socket: &mut WebSocket, msg: RelayMessage<'_>) -> bool {
    let json = msg.as_json();
    socket.send(Message::Text(json.into())).await.is_ok()
}

async fn handle_ws(mut socket: WebSocket, state: ProxyState, token: String) {
    // Per-connection prefix for subscription ID namespacing.
    // All sub IDs sent upstream are prefixed with this to prevent collisions
    // across clients sharing the single upstream connection.
    let conn_prefix = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();

    // Token validation is deferred until after NIP-42 auth completes.
    // Registered guests (in GuestStore) don't need a token at all.
    let challenge = uuid::Uuid::new_v4().to_string();
    if !send_relay_msg(&mut socket, RelayMessage::auth(challenge.clone())).await {
        return;
    }

    // Returns `(pubkey, channels)` on successful auth, or drops the connection
    // on timeout / disconnect / invalid auth.
    let auth_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

    let (client_pubkey, allowed_channels): (PublicKey, Vec<Uuid>) = loop {
        let msg = tokio::select! {
            msg = socket.recv() => msg,
            _ = tokio::time::sleep_until(auth_deadline) => {
                let _ = send_relay_msg(
                    &mut socket,
                    RelayMessage::notice("auth-required: authentication timeout"),
                )
                .await;
                return;
            }
        };

        let text = match msg {
            Some(Ok(Message::Text(t))) => t.to_string(),
            Some(Ok(Message::Close(_))) | None => return,
            _ => continue,
        };

        match ClientMessage::from_json(&text) {
            Ok(ClientMessage::Auth(auth_event)) => {
                // Must be kind 22242
                if auth_event.kind != Kind::Authentication {
                    let _ = send_relay_msg(
                        &mut socket,
                        RelayMessage::ok(auth_event.id, false, "invalid: wrong kind for AUTH"),
                    )
                    .await;
                    continue;
                }

                // Challenge tag must match
                let has_challenge = auth_event.tags.iter().any(|t| {
                    let s = t.as_slice();
                    s.len() >= 2 && s[0] == "challenge" && s[1] == challenge
                });
                if !has_challenge {
                    let _ = send_relay_msg(
                        &mut socket,
                        RelayMessage::ok(auth_event.id, false, "invalid: wrong challenge"),
                    )
                    .await;
                    continue;
                }

                // FIX 4: Timestamp recency check — must be within 10 minutes of now.
                let time_diff = Timestamp::now()
                    .as_secs()
                    .abs_diff(auth_event.created_at.as_secs());
                if time_diff >= 600 {
                    let _ = send_relay_msg(
                        &mut socket,
                        RelayMessage::ok(
                            auth_event.id,
                            false,
                            "invalid: auth event timestamp too far from now",
                        ),
                    )
                    .await;
                    continue;
                }

                // FIX F: Validate relay tag (non-fatal — many clients omit it).
                let has_relay = auth_event.tags.iter().any(|t| {
                    let s = t.as_slice();
                    s.len() >= 2 && s[0] == "relay" && s[1] == state.relay_url
                });
                if !has_relay {
                    debug!("NIP-42 AUTH missing or mismatched relay tag (non-fatal)");
                }

                // Signature must be valid
                if auth_event.verify().is_err() {
                    let _ = send_relay_msg(
                        &mut socket,
                        RelayMessage::ok(auth_event.id, false, "invalid: bad signature"),
                    )
                    .await;
                    continue;
                }

                // Priority: GuestStore (pubkey-based) > invite token.
                let pubkey = auth_event.pubkey;
                let event_id = auth_event.id;

                let channels = if let Some(guest_channels) = state.guest_store.lookup(&pubkey) {
                    // Registered guest — no token needed.
                    info!(pubkey = %pubkey, channels = guest_channels.len(), "guest authenticated (pubkey-based)");
                    guest_channels
                } else if !token.is_empty() {
                    // Fall back to invite token.
                    match state.invite_store.validate_and_consume(&token) {
                        Ok(ch) => {
                            info!(pubkey = %pubkey, channels = ch.len(), "guest authenticated (invite token)");
                            ch
                        }
                        Err(e) => {
                            let _ = send_relay_msg(
                                &mut socket,
                                RelayMessage::notice(format!("error: token invalid: {e}")),
                            )
                            .await;
                            return;
                        }
                    }
                } else {
                    // No guest registration, no token → reject.
                    let _ = send_relay_msg(
                        &mut socket,
                        RelayMessage::ok(
                            event_id,
                            false,
                            "restricted: pubkey not registered and no invite token provided",
                        ),
                    )
                    .await;
                    return;
                };

                let _ = send_relay_msg(&mut socket, RelayMessage::ok(event_id, true, "")).await;
                break (pubkey, channels);
            }

            Ok(ClientMessage::Req {
                subscription_id, ..
            }) => {
                // NIP-42: reject pre-auth REQs with CLOSED so clients like nak
                // can detect the auth-required rejection, authenticate, and
                // re-send the REQ. Buffering silently would leave reactive-auth
                // clients stuck waiting forever.
                let _ = send_relay_msg(
                    &mut socket,
                    RelayMessage::closed(
                        subscription_id.into_owned(),
                        "auth-required: authenticate before subscribing",
                    ),
                )
                .await;
            }

            Ok(ClientMessage::Event(event)) => {
                // NIP-42: respond with OK false so clients like nak can detect
                // the auth-required rejection and retry after authenticating.
                let _ = send_relay_msg(
                    &mut socket,
                    RelayMessage::ok(
                        event.id,
                        false,
                        "auth-required: authenticate before sending events",
                    ),
                )
                .await;
            }

            _ => {
                // Ignore unknown / unparseable messages during pre-auth
            }
        }
    };

    // FIX 1: pending_oks maps upstream_event_id_hex → client_original_event_id
    // FIX 5: active_subs tracks prefixed sub IDs sent upstream for cleanup on disconnect
    let mut pending_oks: HashMap<String, EventId> = HashMap::new();
    let mut active_subs: HashSet<String> = HashSet::new();

    let mut upstream_rx = state.upstream_events.subscribe();

    loop {
        tokio::select! {
            // Inbound from client
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_client_message(
                            &mut socket,
                            &state,
                            &text.to_string(),
                            &allowed_channels,
                            &client_pubkey,
                            &conn_prefix,
                            &mut pending_oks,
                            &mut active_subs,
                        )
                        .await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }

            // Outbound from upstream relay — translate and filter per-client
            upstream = upstream_rx.recv() => {
                match upstream {
                    Ok(text) => {
                        match RelayMessage::from_json(&text) {
                            Ok(RelayMessage::Event { subscription_id, event }) => {
                                // Only process events for subscriptions owned by this connection.
                                let sub_str = subscription_id.to_string();
                                if !sub_str.starts_with(&conn_prefix) {
                                    continue; // Not ours — another client's subscription.
                                }
                                // Strip the connection prefix before sending to client.
                                let client_sub_id = SubscriptionId::new(&sub_str[conn_prefix.len() + 1..]);
                                // Translate outbound: kind:9 → kind:42, #h → #e
                                match state
                                    .translator
                                    .translate_outbound(&event, &allowed_channels)
                                    .await
                                {
                                    Ok(Some(translated)) => {
                                        let out = RelayMessage::event(client_sub_id, translated);
                                        if socket.send(Message::Text(out.as_json().into())).await.is_err() {
                                            break;
                                        }
                                    }
                                    Ok(None) => {
                                        // Not translatable or not a stream message — drop silently.
                                    }
                                    Err(e) => {
                                        // Permission denied or channel not found — skip silently.
                                        debug!(error = %e, "dropping upstream event (not in scope)");
                                    }
                                }
                            }
                            Ok(RelayMessage::EndOfStoredEvents(ref sub_id)) => {
                                let sub_str = sub_id.to_string();
                                if sub_str.starts_with(&conn_prefix) {
                                    let client_sub_id = SubscriptionId::new(&sub_str[conn_prefix.len() + 1..]);
                                    let out = RelayMessage::eose(client_sub_id);
                                    if socket.send(Message::Text(out.as_json().into())).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Ok(RelayMessage::Closed { ref subscription_id, ref message }) => {
                                let sub_str = subscription_id.to_string();
                                if sub_str.starts_with(&conn_prefix) {
                                    let client_sub_id = SubscriptionId::new(&sub_str[conn_prefix.len() + 1..]);
                                    let out = RelayMessage::closed(client_sub_id, message.clone());
                                    if socket.send(Message::Text(out.as_json().into())).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            // FIX 1: Route OK messages to the correct client using pending_oks map.
                            Ok(RelayMessage::Ok { event_id, status, message }) => {
                                let upstream_id_hex = event_id.to_hex();
                                if let Some(client_event_id) = pending_oks.remove(&upstream_id_hex) {
                                    // This OK is for an event we sent — rewrite with client's original ID.
                                    let out = RelayMessage::ok(client_event_id, status, message);
                                    if socket.send(Message::Text(out.as_json().into())).await.is_err() {
                                        break;
                                    }
                                }
                                // If not in pending_oks, this OK belongs to another client — skip it.
                            }
                            // FIX 1: NOTICE messages from upstream contain operational details.
                            // Log them but do NOT forward to clients.
                            Ok(RelayMessage::Notice(notice_msg)) => {
                                debug!(notice = %notice_msg, "upstream notice (not forwarded to client)");
                            }
                            Ok(_other) => {
                                // AUTH, COUNT, and other control-plane messages from
                                // upstream are internal to the proxy↔relay connection.
                                // Do NOT forward to clients — they leak relay internals.
                                debug!("dropping upstream control-plane message (not forwarded)");
                            }
                            Err(_) => {
                                // Unparseable upstream message — drop silently.
                                // Forwarding raw frames could leak relay internals.
                                debug!("dropping unparseable upstream message");
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "upstream broadcast lagged");
                        // Keep going — the client may have missed some events but
                        // we don't want to drop the connection.
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        error!("upstream broadcast channel closed");
                        break;
                    }
                }
            }
        }
    }

    // FIX 5: On disconnect, send CLOSE for all active upstream subscriptions.
    for prefixed_sub in active_subs {
        let sub_id = SubscriptionId::new(prefixed_sub);
        if let Err(e) = state.upstream.send_close(sub_id).await {
            warn!("upstream send_close on disconnect failed: {e}");
        }
    }

    debug!(pubkey = %client_pubkey, "client disconnected");
}

#[allow(clippy::too_many_arguments)]
async fn handle_client_message(
    socket: &mut WebSocket,
    state: &ProxyState,
    raw_msg: &str,
    allowed_channels: &[Uuid],
    client_pubkey: &PublicKey,
    conn_prefix: &str,
    pending_oks: &mut HashMap<String, EventId>,
    active_subs: &mut HashSet<String>,
) {
    let msg = match ClientMessage::from_json(raw_msg) {
        Ok(m) => m,
        Err(_) => {
            let _ = send_relay_msg(socket, RelayMessage::notice("error: invalid message")).await;
            return;
        }
    };

    match msg {
        ClientMessage::Req {
            subscription_id,
            filters,
        } => {
            handle_req(
                socket,
                state,
                subscription_id.into_owned(),
                filters.into_iter().map(|f| f.into_owned()).collect(),
                allowed_channels,
                conn_prefix,
                active_subs,
            )
            .await;
        }
        ClientMessage::Event(event) => {
            let event_id = event.id;

            // Verify the event is signed by the authenticated client.
            // Without this, an AUTHed connection could submit arbitrary events
            // that get re-signed under the client's shadow identity.
            if event.pubkey != *client_pubkey {
                let ok_msg = RelayMessage::ok(
                    event_id,
                    false,
                    "invalid: event pubkey does not match authenticated identity",
                );
                let _ = socket.send(Message::Text(ok_msg.as_json().into())).await;
                return;
            }
            if event.verify().is_err() {
                let ok_msg = RelayMessage::ok(event_id, false, "invalid: bad event signature");
                let _ = socket.send(Message::Text(ok_msg.as_json().into())).await;
                return;
            }

            // Translate inbound: kind:42 → kind:9, #e → #h, re-sign with shadow key.
            match state.translator.translate_inbound(
                &event,
                &client_pubkey.to_hex(),
                allowed_channels,
            ) {
                Ok(translated) => {
                    // FIX H: Cap pending_oks to prevent unbounded growth if upstream never ACKs.
                    if pending_oks.len() >= 1000 {
                        let ok_msg =
                            RelayMessage::ok(event_id, false, "error: too many pending events");
                        let _ = socket.send(Message::Text(ok_msg.as_json().into())).await;
                        return;
                    }
                    // FIX 1: Store mapping from upstream event ID → client original event ID
                    // so we can route the OK response back correctly.
                    // FIX C: Capture upstream_id before moving `translated` into send_event,
                    // so we can remove the correct key on failure.
                    let upstream_id = translated.id;
                    pending_oks.insert(upstream_id.to_hex(), event_id);
                    if let Err(e) = state.upstream.send_event(translated).await {
                        warn!("upstream send_event failed: {e}");
                        // FIX C: Remove by translated (upstream) ID, not client event ID.
                        pending_oks.remove(&upstream_id.to_hex());
                        let ok_msg = RelayMessage::ok(
                            event_id,
                            false,
                            "error: upstream unavailable".to_string(),
                        );
                        let _ = socket.send(Message::Text(ok_msg.as_json().into())).await;
                    }
                }
                Err(e) => {
                    let ok_msg = RelayMessage::ok(event_id, false, format!("error: {e}"));
                    let _ = socket.send(Message::Text(ok_msg.as_json().into())).await;
                }
            }
        }
        ClientMessage::Close(sub_id) => {
            let prefixed = format!("{conn_prefix}:{}", sub_id);
            // FIX 5: Remove from active_subs tracking.
            active_subs.remove(&prefixed);
            let prefixed_sub_id = SubscriptionId::new(prefixed);
            if let Err(e) = state.upstream.send_close(prefixed_sub_id).await {
                warn!("upstream send_close failed: {e}");
            }
        }
        // AUTH after initial handshake is silently ignored.
        ClientMessage::Auth(_) => {}
        _ => {}
    }
}

/// Split a list of NIP-28 filters into local (kind:40/41) and upstream groups.
///
/// **Routing rules:**
/// - kind:40 → local only (channel creation is synthesized from ChannelMap)
/// - kind:41 → BOTH local (synthesized metadata) AND upstream (edit events, kind:40003)
/// - kind:42 and others → upstream only
/// - no kinds → both local (with 40/41 injected) and upstream
///
/// Returns `(local_filters, upstream_filters)`.
fn split_filters(filters: &[Filter]) -> (Vec<Filter>, Vec<Filter>) {
    let mut local_filters: Vec<Filter> = Vec::new();
    let mut upstream_filters: Vec<Filter> = Vec::new();

    for filter in filters {
        let kinds: Vec<u16> = filter
            .kinds
            .as_ref()
            .map(|k| k.iter().map(|kind| kind.as_u16()).collect())
            .unwrap_or_default();

        // kind:40 and kind:41 are served locally (synthesized metadata).
        let has_local = kinds.iter().any(|k| *k == 40 || *k == 41);
        // kind:41 ALSO goes upstream (translates to kind:40003 for edit events).
        // kind:42 and everything else goes upstream.
        let has_upstream = kinds.iter().any(|k| *k != 40);

        if has_local {
            let local_kinds: Vec<Kind> = kinds
                .iter()
                .filter(|k| **k == 40 || **k == 41)
                .map(|k| Kind::Custom(*k))
                .collect();
            let mut local_f = filter.clone();
            if let Some(ref all_kinds) = filter.kinds {
                local_f = local_f.remove_kinds(all_kinds.iter().cloned());
            }
            local_f = local_f.kinds(local_kinds);
            local_filters.push(local_f);
        }
        if has_upstream {
            // Upstream gets everything except kind:40 (which is local-only).
            let upstream_kinds: Vec<Kind> = kinds
                .iter()
                .filter(|k| **k != 40)
                .map(|k| Kind::Custom(*k))
                .collect();
            let mut upstream_f = filter.clone();
            if let Some(ref all_kinds) = filter.kinds {
                upstream_f = upstream_f.remove_kinds(all_kinds.iter().cloned());
            }
            upstream_f = upstream_f.kinds(upstream_kinds);
            upstream_filters.push(upstream_f);
        }
        if !has_local && !has_upstream {
            // No kinds specified — "subscribe to everything".
            // Forward upstream AND serve local kind:40/41 metadata.
            upstream_filters.push(filter.clone());
            let mut local_f = filter.clone();
            local_f = local_f.kinds([Kind::ChannelCreation, Kind::ChannelMetadata]);
            local_filters.push(local_f);
        }
    }

    (local_filters, upstream_filters)
}

/// Collect locally-served events for kind:40/41 filters from the channel map.
///
/// Returns the events that match the filter constraints (kinds, #e, authors,
/// since, until, ids, limit). The caller is responsible for sending them.
fn collect_local_events(
    filter: &Filter,
    channel_map: &ChannelMap,
    allowed_channels: &[Uuid],
) -> Vec<Event> {
    let kinds: Vec<u16> = filter
        .kinds
        .as_ref()
        .map(|k| k.iter().map(|kind| kind.as_u16()).collect())
        .unwrap_or_default();

    let wants_40 = kinds.contains(&40);
    let wants_41 = kinds.contains(&41);

    // Apply `authors` filter — all synthesized events share the server pubkey.
    let server_pubkey = channel_map.server_keys().public_key();
    if let Some(ref authors) = filter.authors {
        if !authors.contains(&server_pubkey) {
            return Vec::new();
        }
    }

    let e_tag_key = nostr::SingleLetterTag::lowercase(nostr::Alphabet::E);
    let e_filter_values = filter.generic_tags.get(&e_tag_key);

    let channels = channel_map.all_channels();
    let limit: usize = filter.limit.unwrap_or(usize::MAX);
    let mut events: Vec<Event> = Vec::new();

    for ch in &channels {
        if events.len() >= limit {
            break;
        }
        if !allowed_channels.contains(&ch.uuid) {
            continue;
        }
        if let Some(e_values) = e_filter_values {
            if !e_values.contains(&ch.kind40_event_id) {
                continue;
            }
        }
        if let Some(since) = filter.since {
            if Timestamp::from(ch.created_at_unix) < since {
                continue;
            }
        }
        if let Some(until) = filter.until {
            if Timestamp::from(ch.created_at_unix) > until {
                continue;
            }
        }

        if wants_40 && events.len() < limit {
            let kind40 = channel_map.synthesize_kind40(&ch.uuid.to_string(), ch.created_at_unix);
            let id_ok = filter
                .ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&kind40.id));
            if id_ok {
                events.push(kind40);
            }
        }
        if wants_41 && events.len() < limit {
            let kind41 = channel_map.synthesize_kind41(ch);
            let id_ok = filter
                .ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&kind41.id));
            if id_ok {
                events.push(kind41);
            }
        }
    }

    events
}

async fn handle_req(
    socket: &mut WebSocket,
    state: &ProxyState,
    sub_id: SubscriptionId,
    filters: Vec<Filter>,
    allowed_channels: &[Uuid],
    conn_prefix: &str,
    active_subs: &mut HashSet<String>,
) {
    let (owned_local_filters, owned_upstream_filters) = split_filters(&filters);

    // Serve local filters from ChannelMap via the extracted pure function.
    for filter in &owned_local_filters {
        let events = collect_local_events(filter, &state.channel_map, allowed_channels);
        for event in events {
            let _ = send_relay_msg(socket, RelayMessage::event(sub_id.clone(), event)).await;
        }
    }

    if owned_upstream_filters.is_empty() {
        // Only local filters — send EOSE immediately after serving them.
        let _ = send_relay_msg(socket, RelayMessage::eose(sub_id.clone())).await;
        return;
    }

    // Forward upstream filters (translated) to the upstream relay.
    // The upstream EOSE will serve as the combined EOSE for mixed REQs.
    let translated_filters: Vec<Filter> = owned_upstream_filters
        .iter()
        .map(|f| {
            state
                .translator
                .translate_filter_inbound(f, allowed_channels)
        })
        .collect();

    let prefixed_sub_id_str = format!("{conn_prefix}:{}", sub_id);
    let prefixed_sub_id = SubscriptionId::new(prefixed_sub_id_str.clone());

    // FIX 5: Track this subscription for cleanup on disconnect.
    active_subs.insert(prefixed_sub_id_str);

    if let Err(e) = state
        .upstream
        .send_req(prefixed_sub_id, translated_filters)
        .await
    {
        warn!("upstream send_req failed: {e}");
    }
}

#[derive(Deserialize)]
struct CreateInviteRequest {
    /// Comma-separated channel UUIDs this token grants access to.
    channels: String,
    /// Hours until the token expires (default: 24).
    #[serde(default = "default_hours")]
    hours: u32,
    /// Maximum number of times the token may be used (default: 10).
    #[serde(default = "default_max_uses")]
    max_uses: u32,
}

fn default_hours() -> u32 {
    24
}

/// FIX 7: Default max_uses changed from 1 to 10.
fn default_max_uses() -> u32 {
    10
}

async fn create_invite(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<CreateInviteRequest>,
) -> impl IntoResponse {
    if let Some(err) = check_admin_secret(&state.admin_secret, &headers) {
        return err;
    }

    let channel_ids: Vec<Uuid> = req
        .channels
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if channel_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "at least one valid channel UUID required" })),
        )
            .into_response();
    }

    if req.hours == 0 || req.max_uses == 0 {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "hours and max_uses must be > 0" })),
        )
            .into_response();
    }

    let token_str = format!("buzz_invite_{}", Uuid::new_v4().simple());
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(req.hours as i64);

    let token = crate::InviteToken::new(&token_str, channel_ids.clone(), expires_at, req.max_uses);
    state.invite_store.insert(token);

    info!(
        token_prefix = %&token_str[..20],
        channels = channel_ids.len(),
        hours = req.hours,
        "invite token created"
    );

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "token": token_str,
            "channels": channel_ids,
            "expires_at": expires_at.to_rfc3339(),
            "max_uses": req.max_uses,
        })),
    )
        .into_response()
}

/// Verify the admin secret from the Authorization header. Returns an error
/// response if the secret is required but missing/wrong, or `None` if OK.
fn check_admin_secret(admin_secret: &Option<String>, headers: &HeaderMap) -> Option<Response> {
    if let Some(ref secret) = admin_secret {
        let provided = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        match provided {
            Some(token) if constant_time_eq(token, secret) => None,
            _ => Some(
                (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({
                        "error": "unauthorized: missing or invalid Authorization header"
                    })),
                )
                    .into_response(),
            ),
        }
    } else {
        None // No secret configured — dev mode, allow all.
    }
}

#[derive(Deserialize)]
struct RegisterGuestRequest {
    /// Hex-encoded Nostr public key (64 chars).
    pubkey: String,
    /// Comma-separated channel UUIDs this guest can access.
    channels: String,
}

async fn register_guest(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<RegisterGuestRequest>,
) -> impl IntoResponse {
    if let Some(err) = check_admin_secret(&state.admin_secret, &headers) {
        return err;
    }

    let pubkey = match PublicKey::from_hex(&req.pubkey) {
        Ok(pk) => pk,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": format!("invalid pubkey: {e}") })),
            )
                .into_response();
        }
    };

    let channel_ids: Vec<Uuid> = req
        .channels
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if channel_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "at least one valid channel UUID required" })),
        )
            .into_response();
    }

    state.guest_store.register(pubkey, channel_ids.clone());
    info!(pubkey = %pubkey, channels = channel_ids.len(), "guest registered");

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "pubkey": req.pubkey,
            "channels": channel_ids,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct RevokeGuestRequest {
    /// Hex-encoded Nostr public key to revoke.
    pubkey: String,
}

async fn revoke_guest(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<RevokeGuestRequest>,
) -> impl IntoResponse {
    if let Some(err) = check_admin_secret(&state.admin_secret, &headers) {
        return err;
    }

    let pubkey = match PublicKey::from_hex(&req.pubkey) {
        Ok(pk) => pk,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": format!("invalid pubkey: {e}") })),
            )
                .into_response();
        }
    };

    let removed = state.guest_store.remove(&pubkey);
    if removed {
        info!(pubkey = %pubkey, "guest revoked");
        (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "revoked": true })),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "error": "pubkey not registered" })),
        )
            .into_response()
    }
}

async fn list_guests(State(state): State<ProxyState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(err) = check_admin_secret(&state.admin_secret, &headers) {
        return err;
    }

    let guests: Vec<serde_json::Value> = state
        .guest_store
        .all()
        .into_iter()
        .map(|(pk, channels)| {
            serde_json::json!({
                "pubkey": pk.to_hex(),
                "channels": channels,
            })
        })
        .collect();

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "guests": guests })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn make_state() -> ProxyState {
        let keys = Keys::generate();
        let channel_map = Arc::new(crate::channel_map::ChannelMap::new(keys.clone()));
        let guest_store = Arc::new(GuestStore::new());
        let invite_store = Arc::new(InviteStore::new());
        let (upstream_events, _) = broadcast::channel(16);
        let shadow_keys = Arc::new(
            crate::shadow_keys::ShadowKeyManager::new(b"test-salt-server-tests")
                .expect("shadow key manager"),
        );
        let translator = Arc::new(crate::translate::Translator::new(
            shadow_keys,
            channel_map.clone(),
            "http://localhost:3000",
            "buzz_test",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        ));
        let upstream = Arc::new(UpstreamClient::new("ws://localhost:3000", "buzz_test"));
        ProxyState {
            channel_map,
            guest_store,
            invite_store,
            translator,
            upstream,
            upstream_events,
            admin_secret: None,
            relay_url: "ws://127.0.0.1:4869".to_string(),
        }
    }

    #[test]
    fn router_builds() {
        let state = make_state();
        let _r = router(state);
    }

    #[test]
    fn nip11_json_is_valid() {
        // Just ensure the NIP-11 JSON serializes without panic
        let response = nip11_response();
        let _ = response.into_response();
    }

    #[test]
    fn default_hours_and_max_uses() {
        assert_eq!(default_hours(), 24);
        // FIX 7: default max_uses is now 10
        assert_eq!(default_max_uses(), 10);
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq("hello", "hello"));
        assert!(!constant_time_eq("hello", "world"));
        assert!(!constant_time_eq("hello", "hell"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
        // Ensure different-length strings with same prefix don't match
        assert!(!constant_time_eq("abc", "abcd"));
    }

    #[test]
    fn split_filters_pure_local() {
        // kind:40 is the only pure-local kind (channel creation is synthesized).
        let f = Filter::new().kind(Kind::ChannelCreation);
        let (local, upstream) = split_filters(&[f]);
        assert_eq!(local.len(), 1);
        assert!(upstream.is_empty(), "kind:40 is local-only");
    }

    #[test]
    fn split_filters_kind41_goes_both() {
        // kind:41 goes to BOTH local (synthesized metadata) and upstream (edits).
        let f = Filter::new().kind(Kind::ChannelMetadata);
        let (local, upstream) = split_filters(&[f]);
        assert_eq!(local.len(), 1, "kind:41 must produce a local filter");
        assert_eq!(
            upstream.len(),
            1,
            "kind:41 must also produce an upstream filter"
        );
    }

    #[test]
    fn split_filters_pure_upstream() {
        let f = Filter::new().kind(Kind::Custom(42));
        let (local, upstream) = split_filters(&[f]);
        assert!(local.is_empty());
        assert_eq!(upstream.len(), 1);
    }

    #[test]
    fn split_filters_mixed_kind() {
        let f = Filter::new().kinds([Kind::ChannelCreation, Kind::Custom(42)]);
        let (local, upstream) = split_filters(&[f]);
        assert_eq!(local.len(), 1, "mixed filter must produce a local portion");
        assert_eq!(
            upstream.len(),
            1,
            "mixed filter must produce an upstream portion"
        );
        let local_k: Vec<u16> = local[0]
            .kinds
            .as_ref()
            .unwrap()
            .iter()
            .map(|k| k.as_u16())
            .collect();
        assert!(local_k.contains(&40));
        assert!(!local_k.contains(&42));
        let up_k: Vec<u16> = upstream[0]
            .kinds
            .as_ref()
            .unwrap()
            .iter()
            .map(|k| k.as_u16())
            .collect();
        assert!(up_k.contains(&42));
        assert!(!up_k.contains(&40));
    }

    #[test]
    fn split_filters_no_kind_duplicates() {
        let f = Filter::new();
        let (local, upstream) = split_filters(&[f]);
        assert_eq!(local.len(), 1);
        assert_eq!(upstream.len(), 1);
        let local_k: Vec<u16> = local[0]
            .kinds
            .as_ref()
            .unwrap()
            .iter()
            .map(|k| k.as_u16())
            .collect();
        assert!(local_k.contains(&40));
        assert!(local_k.contains(&41));
        assert!(upstream[0].kinds.is_none());
    }

    fn make_channel_map_with_channel() -> (Arc<ChannelMap>, Uuid) {
        let keys = Keys::generate();
        let map = ChannelMap::new(keys);
        let dto = crate::channel_map::ChannelDto {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "test-channel".to_string(),
            created_at: "2026-01-15T12:00:00Z".to_string(),
            visibility: "open".to_string(),
            description: "A test channel".to_string(),
            created_by: "0101010101010101010101010101010101010101010101010101010101010101"
                .to_string(),
        };
        map.register(&dto).expect("register must succeed");
        let uuid: Uuid = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        (Arc::new(map), uuid)
    }

    #[test]
    fn collect_local_kind40_basic() {
        let (map, uuid) = make_channel_map_with_channel();
        let filter = Filter::new().kind(Kind::ChannelCreation);
        let events = collect_local_events(&filter, &map, &[uuid]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind.as_u16(), 40);
    }

    #[test]
    fn collect_local_kind41_basic() {
        let (map, uuid) = make_channel_map_with_channel();
        let filter = Filter::new().kind(Kind::ChannelMetadata);
        let events = collect_local_events(&filter, &map, &[uuid]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind.as_u16(), 41);
    }

    #[test]
    fn collect_local_both_kinds() {
        let (map, uuid) = make_channel_map_with_channel();
        let filter = Filter::new().kinds([Kind::ChannelCreation, Kind::ChannelMetadata]);
        let events = collect_local_events(&filter, &map, &[uuid]);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn collect_local_authors_filter_matches() {
        let (map, uuid) = make_channel_map_with_channel();
        let server_pk = map.server_keys().public_key();
        let filter = Filter::new().kind(Kind::ChannelCreation).author(server_pk);
        let events = collect_local_events(&filter, &map, &[uuid]);
        assert_eq!(events.len(), 1, "server pubkey must match");
    }

    #[test]
    fn collect_local_authors_filter_rejects() {
        let (map, uuid) = make_channel_map_with_channel();
        let random_pk = Keys::generate().public_key();
        let filter = Filter::new().kind(Kind::ChannelCreation).author(random_pk);
        let events = collect_local_events(&filter, &map, &[uuid]);
        assert!(events.is_empty(), "random pubkey must not match");
    }

    #[test]
    fn collect_local_channel_not_in_scope() {
        let (map, _uuid) = make_channel_map_with_channel();
        let other: Uuid = "00000000-0000-0000-0000-000000000001".parse().unwrap();
        let filter = Filter::new().kind(Kind::ChannelCreation);
        let events = collect_local_events(&filter, &map, &[other]);
        assert!(events.is_empty());
    }

    #[test]
    fn collect_local_limit_respected() {
        let (map, uuid) = make_channel_map_with_channel();
        let filter = Filter::new()
            .kinds([Kind::ChannelCreation, Kind::ChannelMetadata])
            .limit(1);
        let events = collect_local_events(&filter, &map, &[uuid]);
        assert_eq!(events.len(), 1, "limit:1 must cap at 1 event");
    }

    #[test]
    fn collect_local_since_excludes() {
        let (map, uuid) = make_channel_map_with_channel();
        // Channel created 2026-01-15T12:00:00Z. Since after that → empty.
        let filter = Filter::new()
            .kind(Kind::ChannelCreation)
            .since(Timestamp::from(1768478401u64));
        let events = collect_local_events(&filter, &map, &[uuid]);
        assert!(events.is_empty());
    }

    #[test]
    fn collect_local_until_excludes() {
        let (map, uuid) = make_channel_map_with_channel();
        let filter = Filter::new()
            .kind(Kind::ChannelCreation)
            .until(Timestamp::from(1000000000u64));
        let events = collect_local_events(&filter, &map, &[uuid]);
        assert!(events.is_empty());
    }
}
